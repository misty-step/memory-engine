#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]

//! State, auth, storage, and jobs for the Memory Engine HTTP API.
//!
//! This crate is still a boundary crate. It intentionally stays outside
//! `memory-engine-core`, which remains pure learning semantics.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path as FsPath, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};

use axum::{
    http::{
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Response},
    Json,
};
use hmac::Hmac;
#[cfg(test)]
use memory_engine_generation::DraftProvider;
use memory_engine_generation::FallbackProvider;
pub use memory_engine_openrouter::OpenRouterConfig;
use memory_engine_openrouter::OpenRouterProvider;
pub use memory_engine_persistence::SourcePermission;
use memory_engine_persistence::{BetaPersistenceStore, BetaStoreError};
use memory_engine_persistence_postgres::{
    AccountScope, AccountStudyStore, PostgresStoreError, PostgresStudyStore,
};
use memory_engine_service::{ContentFeedback, ContentFeedbackError, ContentFeedbackVerdict};
use memory_engine_study::{
    BetaStudyConceptProgress, BetaStudyCurrent, BetaStudyDraftRow, BetaStudyOptions,
    BetaStudySession, BetaStudySummary, BetaStudyView,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type UnsubscribeHmac = Hmac<Sha256>;

mod file_lock;
mod jobs;
mod registry;
mod storage;
mod waitlist;

pub use jobs::{EnqueueOutcome, GenerationJob, JobBroadcast, JobQueue, JobStatus};
pub use storage::StudyStorage;
pub use waitlist::WaitlistEntry;

use storage::StudyStorageConfig;

#[derive(Clone)]
pub struct ApiState {
    accounts: AccountRegistry,
    jobs: JobQueue,
    scheduler: Arc<SchedulerRuntime>,
}

struct SchedulerRuntime {
    enabled: AtomicBool,
    running: AtomicBool,
    last_run_at_ms: AtomicI64,
    last_success_at_ms: AtomicI64,
    failure_count: AtomicU64,
}

/// Owns the scheduler task and joins it after signalling shutdown.
pub struct SchedulerHandle {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SchedulerHandle {
    fn disabled() -> Self {
        Self {
            shutdown: None,
            task: None,
        }
    }

    /// Stop the scheduler and wait for any in-flight blocking sweep to finish.
    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Default for SchedulerRuntime {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            running: AtomicBool::new(false),
            last_run_at_ms: AtomicI64::new(0),
            last_success_at_ms: AtomicI64::new(0),
            failure_count: AtomicU64::new(0),
        }
    }
}

impl ApiState {
    #[must_use]
    pub fn new(accounts: AccountRegistry) -> Self {
        // The file-backed host mirrors job history to disk; production uses the
        // Postgres ledger so queued/running/retry state survives process loss.
        let jobs = match (accounts.job_history_path(), accounts.postgres_url()) {
            (Some(path), _) => JobQueue::with_persistence(accounts.clone(), path),
            (None, Some(database_url)) => JobQueue::with_postgres(accounts.clone(), database_url),
            (None, None) => JobQueue::new(accounts.clone()),
        };
        Self {
            accounts,
            jobs,
            scheduler: Arc::new(SchedulerRuntime::default()),
        }
    }

    /// Start the background generation worker. Call once, from inside the Tokio
    /// runtime (e.g. in `main`), before serving requests.
    pub fn start_worker(&self) {
        self.jobs.spawn_worker();
    }

    /// Create an account through the API state boundary.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth or persistence rejects the account.
    pub fn create_account(&self, email: &str) -> Result<AccountCreated, ApiFailure> {
        self.accounts.create_account(email)
    }

    /// Join the invite-beta waitlist. Idempotent on normalized email and
    /// silent about allowlist/account state: a repeat join or a join by an
    /// address that already has access looks identical to a brand-new one.
    /// Persists to Postgres in production and to the file store locally.
    ///
    /// # Errors
    ///
    /// Returns bad request on a malformed email, too-many-requests when the
    /// per-email or per-IP limit is spent, and service-unavailable when
    /// storage rejects the write.
    pub fn join_waitlist(
        &self,
        email: &str,
        source: &str,
        client_rate_limit_key: &str,
    ) -> Result<(), ApiFailure> {
        self.accounts
            .join_waitlist(email, source, client_rate_limit_key)
    }

    /// List every waitlist entry for the operator, gated by the admin token.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the admin token is unconfigured or mismatched,
    /// and service-unavailable when storage rejects the read.
    pub fn list_waitlist(&self, admin_token: &str) -> Result<Vec<WaitlistEntry>, ApiFailure> {
        self.accounts.list_waitlist(admin_token)
    }

    /// Mark one waitlist entry invited for the operator, gated by the admin
    /// token. Idempotent: marking an already-invited entry again leaves its
    /// `invitedAtMs` unchanged.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the admin token is unconfigured or mismatched,
    /// not-found when no entry matches the normalized email, and
    /// service-unavailable when storage rejects the read or write.
    pub fn mark_waitlist_invited(
        &self,
        admin_token: &str,
        email: &str,
    ) -> Result<WaitlistEntry, ApiFailure> {
        self.accounts.mark_waitlist_invited(admin_token, email)
    }

    /// Delete one waitlist entry for the operator, gated by the admin token.
    /// The append-only audit trail keeps a record of what happened to the
    /// address; only the operational row is removed.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the admin token is unconfigured or mismatched,
    /// not-found when no entry matches the normalized email, and
    /// service-unavailable when storage rejects the write.
    pub fn delete_waitlist_entry(&self, admin_token: &str, email: &str) -> Result<(), ApiFailure> {
        self.accounts.delete_waitlist_entry(admin_token, email)
    }

    /// Issue (or rotate) the service-session credential for an allowlisted
    /// account, gated by the operator admin token. Reissuing revokes the
    /// prior credential immediately.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the admin token is not configured or does not
    /// match, or when the email is outside the allowlist.
    pub fn issue_service_session(
        &self,
        admin_token: &str,
        email: &str,
    ) -> Result<AccountCreated, ApiFailure> {
        self.accounts.issue_service_session(admin_token, email)
    }

    /// Check a caller-supplied token against the configured operator admin
    /// token, without touching any request body.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the admin token is unconfigured, empty, or
    /// mismatched.
    pub fn verify_admin_token(&self, admin_token: &str) -> Result<(), ApiFailure> {
        self.accounts.verify_admin_token(admin_token)
    }

    /// Request an auth magic link.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the email is invalid, rate-limited, or link
    /// delivery fails.
    pub fn request_magic_link(
        &self,
        email: &str,
        client_rate_limit_key: &str,
    ) -> Result<MagicLinkRequest, ApiFailure> {
        self.accounts
            .request_magic_link(email, client_rate_limit_key)
    }

    /// Verify an auth magic link and return a browser session.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the link is missing, expired, replayed, or
    /// invalid.
    pub fn verify_magic_link(&self, token: &str) -> Result<AppAccount, ApiFailure> {
        self.accounts.verify_magic_link(token)
    }

    /// Create a browser session for an already-created account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when session persistence fails.
    pub fn create_browser_session(
        &self,
        account: &AccountCreated,
    ) -> Result<AppAccount, ApiFailure> {
        self.accounts.create_browser_session(account)
    }

    /// Require a valid browser session and CSRF token.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the browser session or CSRF token is invalid.
    pub fn require_browser_session(
        &self,
        headers: &HeaderMap,
        csrf_token: &str,
    ) -> Result<AppAccount, ApiFailure> {
        self.accounts.require_browser_session(headers, csrf_token)
    }
    /// Require a valid browser session while accounting for every Postgres
    /// boundary traversed by the request.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the browser session or CSRF token is invalid.
    pub fn require_browser_session_with_timings(
        &self,
        headers: &HeaderMap,
        csrf_token: &str,
        timings: &mut SubmitReviewTimings,
    ) -> Result<AppAccount, ApiFailure> {
        self.accounts
            .require_browser_session_with_timings(headers, csrf_token, timings)
    }

    /// Require a valid browser session for a read-only request.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the browser session is invalid.
    pub fn require_browser_session_readonly(
        &self,
        headers: &HeaderMap,
    ) -> Result<AppAccount, ApiFailure> {
        self.accounts.require_browser_session_readonly(headers)
    }

    /// Revoke a browser session after CSRF validation.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the browser session or CSRF token is invalid.
    pub fn revoke_browser_session(
        &self,
        headers: &HeaderMap,
        csrf_token: &str,
    ) -> Result<(), ApiFailure> {
        self.accounts.revoke_browser_session(headers, csrf_token)
    }

    /// Save material through the API state boundary.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, validation, or persistence rejects the source.
    pub fn save_source(
        &self,
        account_id: &str,
        session_token: &str,
        request: &CreateSourceRequest,
    ) -> Result<SourceRecord, ApiFailure> {
        self.accounts
            .save_source(account_id, session_token, request)
    }

    /// Save material for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when validation or persistence rejects the source.
    pub fn save_app_source(
        &self,
        account: &AppAccount,
        request: &CreateSourceRequest,
    ) -> Result<SourceRecord, ApiFailure> {
        self.accounts
            .save_source(account.account_id(), account.session_token(), request)
    }

    /// Create a project-scoped volatile deck through the API state boundary.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, validation, or persistence rejects the deck.
    pub fn create_project_deck(
        &self,
        account_id: &str,
        session_token: &str,
        request: &CreateProjectDeckRequest,
    ) -> Result<ProjectDeckRecord, ApiFailure> {
        self.accounts
            .create_project_deck(account_id, session_token, request)
    }

    /// Retire cards generated from a project deck after an external event.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, deck lookup, or persistence rejects the event.
    pub fn invalidate_project_deck(
        &self,
        account_id: &str,
        session_token: &str,
        deck_id: &str,
        request: &InvalidateProjectDeckRequest,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .invalidate_project_deck(account_id, session_token, deck_id, request)
    }

    /// List saved material through the API state boundary.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth or persistence rejects the read.
    pub fn list_sources(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.accounts.list_sources(account_id, session_token)
    }

    /// Update an active source's model-sharing permission for an authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the account, source, or persistence boundary rejects it.
    pub fn update_source_permission(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure> {
        self.accounts
            .update_source_permission(account_id, session_token, source_id, permission)
    }

    /// List saved material for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when persistence rejects the read.
    pub fn list_app_sources(&self, account: &AppAccount) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.accounts
            .list_sources(account.account_id(), account.session_token())
    }
    /// List saved material while accounting for every Postgres boundary
    /// traversed by a timed browser request.
    ///
    /// # Errors
    ///
    /// Returns an API failure when persistence rejects the read.
    pub fn list_app_sources_with_timings(
        &self,
        account: &AppAccount,
        timings: &mut SubmitReviewTimings,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.accounts.list_sources_with_timings(
            account.account_id(),
            account.session_token(),
            Some(timings),
        )
    }

    /// Update an active source's model-sharing permission for a browser account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the source is unknown, archived, or cannot be persisted.
    pub fn update_app_source_permission(
        &self,
        account: &AppAccount,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure> {
        self.accounts.update_source_permission(
            account.account_id(),
            account.session_token(),
            source_id,
            permission,
        )
    }

    /// Save an email-backed account over an existing browser session.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, validation, or persistence rejects the save.
    pub fn save_account(
        &self,
        source_account: &AppAccount,
        email: &str,
    ) -> Result<AccountCreated, ApiFailure> {
        self.accounts.save_account(
            source_account.account_id(),
            source_account.session_token(),
            email,
        )
    }

    /// Generate review material from a saved source.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, source lookup, generation, or persistence fails.
    pub fn generate_source(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .generate_source(account_id, session_token, source_id)
    }

    /// Archive saved material.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, source lookup, or persistence fails.
    pub fn archive_source(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        self.accounts
            .archive_source(account_id, session_token, source_id)
    }

    /// Archive saved material for a browser-authenticated account. Returns
    /// the view plus the count of cards actually retired (across every
    /// generation run for the source) so the caller can report it rather
    /// than a generic notice (memory-engine-088).
    ///
    /// # Errors
    ///
    /// Returns an API failure when source lookup or persistence fails.
    pub fn archive_app_source(
        &self,
        account: &AppAccount,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        self.accounts
            .archive_source(account.account_id(), account.session_token(), source_id)
    }

    /// Approve a generated draft.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, draft lookup, or persistence fails.
    pub fn approve_draft(
        &self,
        account_id: &str,
        session_token: &str,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .approve_draft(account_id, session_token, draft_id)
    }

    /// Fetch the next due review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth or study state rejects the read.
    pub fn next_review(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.next_review(account_id, session_token)
    }

    /// Fetch the next due review for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when study state rejects the read.
    pub fn next_app_review(&self, account: &AppAccount) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .next_review(account.account_id(), account.session_token())
    }

    /// Render the current study view.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth or study state rejects the read.
    pub fn study_view(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.study_view(account_id, session_token)
    }

    /// Render the current study view for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when study state rejects the read.
    pub fn app_study_view(&self, account: &AppAccount) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .study_view(account.account_id(), account.session_token())
    }
    /// Render the current study view while accounting for every Postgres
    /// boundary traversed by a timed browser request.
    ///
    /// # Errors
    ///
    /// Returns an API failure when study state rejects the read.
    pub fn app_study_view_with_timings(
        &self,
        account: &AppAccount,
        timings: &mut SubmitReviewTimings,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.study_view_with_timings(
            account.account_id(),
            account.session_token(),
            Some(timings),
        )
    }

    /// Persist the learner's explicit due-count return-channel choice.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the account preference cannot be stored.
    pub fn set_return_notification(
        &self,
        account: &AppAccount,
        email: Option<&str>,
        enabled: bool,
    ) -> Result<(), ApiFailure> {
        self.accounts.set_return_notification(
            account.account_id(),
            account.session_token(),
            email,
            enabled,
        )
    }

    /// Send the due-count message when the deterministic daily policy allows it.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the configured mail boundary fails.
    pub fn maybe_send_due_count_notification(
        &self,
        account: &AppAccount,
        due_count: usize,
        force_confirmation: bool,
    ) -> Result<bool, ApiFailure> {
        self.accounts.maybe_send_due_count_notification(
            account.account_id(),
            account.session_token(),
            due_count,
            force_confirmation,
        )
    }

    /// Run one bounded, request-independent reminder sweep. This is the
    /// scheduler's explicit execution surface; it is safe to call from a
    /// cron/manual trigger and uses the durable account claim before mail.
    ///
    /// # Errors
    ///
    /// Returns an API failure when enumeration itself cannot be completed.
    pub fn run_scheduled_return_notifications(
        &self,
    ) -> Result<ScheduledReturnNotificationReport, ApiFailure> {
        self.run_scheduled_return_notifications_with_config(
            ReturnNotificationSchedulerConfig::default(),
        )
    }

    /// Run one scheduler sweep with an explicit bound. This is public for the
    /// safe manual/backfill trigger and deterministic boundary tests.
    ///
    /// # Errors
    ///
    /// Returns an API failure when enumeration itself cannot be completed.
    pub fn run_scheduled_return_notifications_with_config(
        &self,
        config: ReturnNotificationSchedulerConfig,
    ) -> Result<ScheduledReturnNotificationReport, ApiFailure> {
        let result = self.accounts.run_scheduled_return_notifications(config);
        match &result {
            Ok(report) => {
                self.scheduler
                    .last_run_at_ms
                    .store(report.finished_at_ms, Ordering::Relaxed);
                if report.failed == 0 {
                    self.scheduler
                        .last_success_at_ms
                        .store(report.finished_at_ms, Ordering::Relaxed);
                }
                self.scheduler
                    .failure_count
                    .fetch_add(report.failed as u64, Ordering::Relaxed);
            }
            Err(_) => {
                self.scheduler.failure_count.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }

    /// Start the production scheduled trigger. Multiple API instances are
    /// safe because durable storage owns the per-account claim/fence.
    #[must_use]
    pub fn start_return_notification_scheduler(&self) -> SchedulerHandle {
        let interval_ms = scheduler_interval_ms();
        let config = ReturnNotificationSchedulerConfig::from_env();
        if !scheduler_enabled() {
            return SchedulerHandle::disabled();
        }
        self.start_return_notification_scheduler_with_config(interval_ms, config)
    }

    /// Start a scheduler with explicit timing and batch controls.
    ///
    /// This is also the lifecycle seam used by deterministic boundary tests;
    /// production hosts should use [`Self::start_return_notification_scheduler`].
    #[must_use]
    pub fn start_return_notification_scheduler_with_interval(
        &self,
        interval: Duration,
        config: ReturnNotificationSchedulerConfig,
    ) -> SchedulerHandle {
        let interval_ms =
            u64::try_from(interval.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX);
        self.start_return_notification_scheduler_with_config(interval_ms, config)
    }

    fn start_return_notification_scheduler_with_config(
        &self,
        interval_ms: u64,
        config: ReturnNotificationSchedulerConfig,
    ) -> SchedulerHandle {
        self.scheduler.enabled.store(true, Ordering::Relaxed);
        let state = self.clone();
        let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                state.scheduler.running.store(true, Ordering::Relaxed);
                let run_state = state.clone();
                let mut run = std::pin::pin!(tokio::task::spawn_blocking(move || {
                    run_state.run_scheduled_return_notifications_with_config(config)
                }));
                let result = tokio::select! {
                    result = &mut run => Some(result),
                    _ = &mut shutdown_rx => {
                        let _ = (&mut run).await;
                        None
                    }
                };
                state.scheduler.running.store(false, Ordering::Relaxed);
                let Some(result) = result else {
                    break;
                };
                match result {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        eprintln!("return notification scheduler enumeration failed: {error:?}");
                    }
                    Err(error) => {
                        state
                            .scheduler
                            .failure_count
                            .fetch_add(1, Ordering::Relaxed);
                        eprintln!("return notification scheduler worker failed: {error}");
                    }
                }
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
                    _ = &mut shutdown_rx => break,
                }
            }
            state.scheduler.enabled.store(false, Ordering::Relaxed);
            state.scheduler.running.store(false, Ordering::Relaxed);
        });
        SchedulerHandle {
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    /// Run the bounded scheduler through the operator-only manual token.
    ///
    /// # Errors
    ///
    /// Returns forbidden when the manual trigger is not configured or the
    /// supplied token does not match.
    pub fn run_manual_return_notification_scheduler(
        &self,
        token: &str,
    ) -> Result<ScheduledReturnNotificationReport, ApiFailure> {
        let configured = self
            .accounts
            .lock_data()
            .auth_config
            .scheduler_manual_token
            .clone();
        if configured.as_deref().is_none_or(|value| value != token) {
            return Err(ApiFailure::forbidden(
                "Scheduled reminder trigger is not authorized.",
            ));
        }
        self.run_scheduled_return_notifications()
    }

    #[must_use]
    pub fn scheduler_health(&self) -> SchedulerHealth {
        SchedulerHealth {
            enabled: self.scheduler.enabled.load(Ordering::Relaxed),
            running: self.scheduler.running.load(Ordering::Relaxed),
            last_run_at_ms: nonzero_timestamp(
                self.scheduler.last_run_at_ms.load(Ordering::Relaxed),
            ),
            last_success_at_ms: nonzero_timestamp(
                self.scheduler.last_success_at_ms.load(Ordering::Relaxed),
            ),
            failure_count: self.scheduler.failure_count.load(Ordering::Relaxed),
        }
    }

    /// Validate an email unsubscribe link without changing preference state.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the signed token is invalid, expired, or no
    /// longer matches the account-scoped preference.
    pub fn validate_return_notification_token(&self, token: &str) -> Result<(), ApiFailure> {
        self.accounts.validate_return_notification_token(token)
    }

    /// Disable reminders using an account-scoped email bearer token. This is a
    /// POST-only mutation; the token intentionally does not require a browser
    /// session because it is delivered to the opted-in mailbox.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the signed token is invalid, expired, or no
    /// longer matches the account-scoped preference.
    pub fn disable_return_notification(&self, token: &str) -> Result<(), ApiFailure> {
        self.accounts.disable_return_notification(token)
    }

    /// Reveal a review answer.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, or persistence fails.
    pub fn reveal_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .reveal_review(account_id, session_token, review_unit_id)
    }

    /// Reveal a browser-authenticated review answer.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup or persistence fails.
    pub fn reveal_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.reveal_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Fetch reference material for a review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, generation, or persistence fails.
    pub fn learn_more_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .learn_more_review(account_id, session_token, review_unit_id)
    }

    /// Fetch reference material for a browser-authenticated review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup, generation, or persistence fails.
    pub fn learn_more_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.learn_more_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Skip a review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, or persistence fails.
    pub fn skip_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .skip_review(account_id, session_token, review_unit_id)
    }

    /// Skip a browser-authenticated review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup or persistence fails.
    pub fn skip_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.skip_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Delete a browser-authenticated review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup or persistence fails.
    pub fn delete_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.delete_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Edit a browser-authenticated review without changing its schedule or
    /// attempt history.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup, validation, or persistence
    /// rejects the edit.
    pub fn edit_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.edit_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
            prompt,
            expected_answer,
        )
    }

    /// Snooze a review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, or persistence fails.
    pub fn snooze_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .snooze_review(account_id, session_token, review_unit_id)
    }

    /// Snooze a browser-authenticated review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup or persistence fails.
    pub fn snooze_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.snooze_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Snooze every review card under the active card's persisted concept key.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, or persistence fails.
    pub fn snooze_concept_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .snooze_concept_review(account_id, session_token, review_unit_id)
    }

    /// Snooze every review card under the browser-authenticated card's
    /// persisted concept key.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup or persistence fails.
    pub fn snooze_concept_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.snooze_concept_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Generate bridge material for a review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, generation, or persistence fails.
    pub fn bridge_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .bridge_review(account_id, session_token, review_unit_id)
    }

    /// Generate bridge material for a browser-authenticated review.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup, generation, or persistence fails.
    pub fn bridge_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.bridge_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Submit a review answer.
    ///
    /// # Errors
    ///
    /// Returns an API failure when auth, review lookup, grading, or persistence fails.
    pub fn submit_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        request: &SubmitReviewRequest,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts
            .submit_review(account_id, session_token, review_unit_id, request)
    }

    /// Submit a browser-authenticated review answer.
    ///
    /// # Errors
    ///
    /// Returns an API failure when review lookup, grading, or persistence fails.
    pub fn submit_app_review(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
        request: &SubmitReviewRequest,
        timings: &mut SubmitReviewTimings,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.submit_review_with_timings(
            account.account_id(),
            account.session_token(),
            review_unit_id,
            request,
            Some(timings),
        )
    }

    /// Record a learner's binary content-quality judgment for a review unit.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the account session, feedback command, or
    /// account-scoped persistence rejects the record.
    pub fn record_content_feedback(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        request: &ContentFeedbackRequest,
    ) -> Result<ContentFeedback, ApiFailure> {
        self.accounts
            .record_content_feedback(account_id, session_token, review_unit_id, request)
    }

    /// Record feedback for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the account session, feedback command, or
    /// account-scoped persistence rejects the record.
    pub fn record_app_content_feedback(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
        request: &ContentFeedbackRequest,
    ) -> Result<ContentFeedback, ApiFailure> {
        self.accounts.record_content_feedback(
            account.account_id(),
            account.session_token(),
            review_unit_id,
            request,
        )
    }

    /// Read the latest persisted content-feedback revision for a review unit.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the browser session or account-scoped
    /// persistence read fails.
    pub fn app_content_feedback_head(
        &self,
        account: &AppAccount,
        review_unit_id: &str,
    ) -> Result<Option<String>, ApiFailure> {
        self.accounts.content_feedback_head(
            account.account_id(),
            account.session_token(),
            review_unit_id,
        )
    }

    /// Enqueue a background generation job, coalescing onto an existing
    /// queued/running job for the same account+source (082) instead of
    /// starting a duplicate.
    #[must_use]
    pub fn enqueue_generation_job(
        &self,
        account: &AppAccount,
        source: &SourceRecord,
    ) -> EnqueueOutcome {
        self.jobs
            .enqueue_or_coalesce(account.account_id(), &source.source_id, &source.title)
    }

    /// Enqueue a background generation job by source id and title, coalescing
    /// onto an existing queued/running job for the same account+source (082)
    /// instead of starting a duplicate.
    #[must_use]
    pub fn enqueue_generation_job_by_source(
        &self,
        account: &AppAccount,
        source_id: &str,
        title: &str,
    ) -> EnqueueOutcome {
        self.jobs
            .enqueue_or_coalesce(account.account_id(), source_id, title)
    }

    /// Enqueue durable generation for an API-authenticated account and owned source.
    ///
    /// Returns the account-scoped job and whether this request coalesced onto an
    /// existing in-flight job.
    ///
    /// # Errors
    ///
    /// Returns an API failure when authentication, source lookup, or enqueue
    /// fails.
    pub fn enqueue_generation_job_for_session(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<(GenerationJob, bool), ApiFailure> {
        let source = self
            .accounts
            .list_sources(account_id, session_token)?
            .into_iter()
            .find(|source| source.source_id == source_id)
            .ok_or_else(|| ApiFailure::not_found("Source not found."))?;
        match self
            .jobs
            .enqueue_or_coalesce(account_id, &source.source_id, &source.title)
        {
            EnqueueOutcome::Started(job) => Ok((job, false)),
            EnqueueOutcome::AlreadyInFlight(job) => Ok((job, true)),
            EnqueueOutcome::Rejected(reason) => Err(ApiFailure::conflict_message(reason)),
            EnqueueOutcome::Unavailable(reason) => Err(ApiFailure::service_unavailable(reason)),
        }
    }

    /// Return one durable generation job to its API-authenticated owner.
    ///
    /// # Errors
    ///
    /// Returns an API failure when authentication fails or the account does not
    /// own the requested job.
    pub fn generation_job_for_session(
        &self,
        account_id: &str,
        session_token: &str,
        job_id: &str,
    ) -> Result<GenerationJob, ApiFailure> {
        self.accounts
            .authenticate_account(account_id, session_token)?;
        self.jobs
            .job_for_account(account_id, job_id)
            .map_err(ApiFailure::service_unavailable)?
            .ok_or_else(|| ApiFailure::not_found("Generation job not found."))
    }

    /// Retry a background generation job.
    #[must_use]
    pub fn retry_generation_job(&self, account: &AppAccount, job_id: &str) -> bool {
        self.jobs.retry(account.account_id(), job_id)
    }

    /// Return rendered job history for a browser-authenticated account.
    #[must_use]
    pub fn jobs_for_app_account(&self, account: &AppAccount) -> Vec<GenerationJob> {
        self.jobs.jobs_for(account.account_id())
    }
    /// Return rendered job history while accounting for every Postgres
    /// boundary traversed by a timed browser request.
    #[must_use]
    pub fn jobs_for_app_account_with_timings(
        &self,
        account: &AppAccount,
        timings: &mut SubmitReviewTimings,
    ) -> Vec<GenerationJob> {
        self.jobs
            .jobs_for_with_timings(account.account_id(), timings)
    }

    /// Return rendered job history by account id. Test helper for route coverage.
    #[doc(hidden)]
    #[must_use]
    pub fn jobs_for_account_id(&self, account_id: &str) -> Vec<GenerationJob> {
        self.jobs.jobs_for(account_id)
    }

    /// Subscribe to generation job broadcasts.
    #[must_use]
    pub fn subscribe_jobs(&self) -> tokio::sync::broadcast::Receiver<JobBroadcast> {
        self.jobs.subscribe()
    }

    /// Run pending background jobs synchronously. Test helper for route coverage.
    #[doc(hidden)]
    pub fn run_pending_jobs_blocking(&self) {
        self.jobs.run_pending_blocking();
    }

    /// Enqueue a generation job by account id, coalescing like production
    /// enqueue. Test helper for production-shaped (queued) route coverage.
    #[doc(hidden)]
    #[must_use]
    pub fn enqueue_generation_job_for_account_id(
        &self,
        account_id: &str,
        source_id: &str,
        title: &str,
    ) -> EnqueueOutcome {
        self.jobs.enqueue_or_coalesce(account_id, source_id, title)
    }

    /// Read durable reminder state for boundary tests and operator receipts.
    #[doc(hidden)]
    pub fn load_return_notification_preference_for_test(
        &self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, ApiFailure> {
        self.accounts
            .storage()
            .load_return_notification_preference(account_id)
    }

    /// Return one job by id. Test helper for route coverage.
    #[doc(hidden)]
    #[must_use]
    pub fn job(&self, job_id: &str) -> Option<GenerationJob> {
        self.jobs.job(job_id)
    }

    /// Readiness is separate from `/healthz`: it requires the production
    /// dependency and the worker loop, so a live but non-serving process is not
    /// advertised as ready.
    #[must_use]
    pub fn readiness(&self) -> ReadinessResponse {
        let worker_started = self.jobs.worker_ready();
        let postgres = self.accounts.postgres_ready();
        ReadinessResponse {
            status: if worker_started && postgres {
                "ready"
            } else {
                "not_ready"
            },
            service: "memory-engine-api",
            worker_started,
            postgres,
        }
    }
}

impl Default for ApiState {
    fn default() -> Self {
        Self::new(AccountRegistry::default())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConfig {
    allowed_emails: Option<BTreeSet<String>>,
    expose_debug_links: bool,
    link_delivery: AuthLinkDelivery,
    unsubscribe_secret: String,
    scheduler_manual_token: Option<String>,
    admin_token: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            allowed_emails: None,
            expose_debug_links: false,
            link_delivery: AuthLinkDelivery::None,
            unsubscribe_secret: format!("unsubscribe_{:032x}", rand::random::<u128>()),
            scheduler_manual_token: None,
            admin_token: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnNotificationPreference {
    pub email: String,
    pub enabled: bool,
    pub last_sent_at_ms: Option<i64>,
    #[serde(default)]
    pub unsubscribe_nonce: String,
    #[serde(default)]
    pub claim_id: Option<String>,
    #[serde(default)]
    pub claim_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub pending_delivery_key: Option<String>,
    #[serde(default)]
    pub pending_due_count: Option<usize>,
    #[serde(default)]
    pub pending_unsubscribe_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub next_retry_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReturnNotificationClaim {
    pub email: String,
    pub due_count: usize,
    pub delivery_key: String,
    pub unsubscribe_nonce: String,
    pub unsubscribe_expires_at_ms: i64,
    pub claim_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReturnNotificationClaimRequest {
    pub account_id: String,
    pub now_ms: i64,
    pub due_count: usize,
    pub force_confirmation: bool,
    pub interval_ms: i64,
    pub claim_id: String,
    pub delivery_key: String,
    pub claim_expires_at_ms: i64,
    pub unsubscribe_nonce: String,
    pub unsubscribe_expires_at_ms: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledReturnNotificationReport {
    pub examined: usize,
    pub due: usize,
    pub sent: usize,
    pub skipped: usize,
    pub failed: usize,
    pub truncated: bool,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReturnNotificationSchedulerConfig {
    pub batch_size: usize,
}

impl Default for ReturnNotificationSchedulerConfig {
    fn default() -> Self {
        Self { batch_size: 100 }
    }
}

impl ReturnNotificationSchedulerConfig {
    #[must_use]
    pub fn from_env() -> Self {
        let batch_size = std::env::var("MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .map_or(100, |value| value.min(1_000));
        Self { batch_size }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerHealth {
    pub enabled: bool,
    pub running: bool,
    pub last_run_at_ms: Option<i64>,
    pub last_success_at_ms: Option<i64>,
    pub failure_count: u64,
}

fn nonzero_timestamp(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

fn scheduler_enabled() -> bool {
    std::env::var("MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED")
        .map(|value| value.trim() != "false")
        .unwrap_or(true)
}

fn scheduler_interval_ms() -> u64 {
    std::env::var("MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map_or(900, |value| value.min(86_400))
        .saturating_mul(1_000)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AuthLinkDelivery {
    #[default]
    None,
    OutboxFile(PathBuf),
    Command(String),
}

impl AuthConfig {
    #[must_use]
    pub fn allow_emails(emails: impl IntoIterator<Item = String>) -> Self {
        let allowed_emails = emails
            .into_iter()
            .filter_map(|email| normalize_email(&email))
            .collect::<BTreeSet<_>>();

        Self {
            allowed_emails: Some(allowed_emails),
            expose_debug_links: false,
            link_delivery: AuthLinkDelivery::None,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_debug_links(mut self, expose_debug_links: bool) -> Self {
        self.expose_debug_links = expose_debug_links;
        self
    }

    #[must_use]
    pub fn with_link_outbox(mut self, path: impl Into<PathBuf>) -> Self {
        self.link_delivery = AuthLinkDelivery::OutboxFile(path.into());
        self
    }

    #[must_use]
    pub fn with_mailer_command(mut self, command: impl Into<String>) -> Self {
        self.link_delivery = AuthLinkDelivery::Command(command.into());
        self
    }

    /// Set the stable secret used to sign account-scoped unsubscribe links.
    /// Production hosts should source this from a secret manager and keep it
    /// stable across restarts so already-delivered links remain usable.
    #[must_use]
    pub fn with_unsubscribe_secret(mut self, secret: impl Into<String>) -> Self {
        self.unsubscribe_secret = secret.into();
        self
    }

    #[must_use]
    pub fn with_scheduler_manual_token(mut self, token: impl Into<String>) -> Self {
        self.scheduler_manual_token = Some(token.into());
        self
    }

    /// Set the operator admin token that gates service-session issuance.
    /// Production hosts should source this from a secret manager; leaving it
    /// unset disables the service-session surface entirely.
    #[must_use]
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Some(token.into());
        self
    }

    fn email_allowed(&self, email: &str) -> bool {
        self.allowed_emails
            .as_ref()
            .is_none_or(|allowed| allowed.contains(email))
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccountRegistry {
    inner: Arc<Mutex<AccountRegistryData>>,
    /// Per-account locks that serialize study-store read-modify-write, so
    /// concurrent generation jobs for one account can't clobber each other's
    /// cards (059). Shared across clones — the worker runs on clones — and keyed
    /// by account id, so different accounts never contend.
    store_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl AccountRegistry {
    #[must_use]
    pub fn with_store_root(store_root: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AccountRegistryData {
                storage: StudyStorageConfig::file(store_root),
                ..AccountRegistryData::default()
            })),
            store_locks: Arc::default(),
        }
    }

    #[must_use]
    pub fn with_postgres_url(database_url: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AccountRegistryData {
                storage: StudyStorageConfig::postgres(database_url),
                ..AccountRegistryData::default()
            })),
            store_locks: Arc::default(),
        }
    }

    /// Apply browser-auth configuration to this registry.
    ///
    #[must_use]
    pub fn with_auth_config(self, auth_config: AuthConfig) -> Self {
        let mut data = self.lock_data();
        data.auth_config = auth_config;
        drop(data);
        self
    }

    /// Inject the model-generation config used by the study routes.
    ///
    /// Production should set this from the environment; tests can pass an
    /// explicit config to exercise the exact route-selection code path.
    #[must_use]
    pub fn with_generation_provider_config(
        self,
        generation_provider_config: Option<OpenRouterConfig>,
    ) -> Self {
        let mut data = self.lock_data();
        data.generation_provider_config = generation_provider_config;
        drop(data);
        self
    }

    /// Replace the wall-clock time source, for tests that control time.
    ///
    /// Production constructors default to wall-clock milliseconds; every
    /// schedule, auth-challenge, and session-expiry decision flows through
    /// this clock.
    ///
    #[must_use]
    pub fn with_clock(self, now_fn: fn() -> i64) -> Self {
        let mut data = self.lock_data();
        data.now_fn = now_fn;
        drop(data);
        self
    }

    #[must_use]
    pub fn clock(&self) -> fn() -> i64 {
        self.lock_data().now_fn
    }

    pub(crate) fn now(&self) -> i64 {
        (self.clock())()
    }

    /// The lock guarding `account_id`'s study store. One `Mutex` per account,
    /// created on first use; held across a whole generation run so concurrent
    /// captures for the same account serialize their read-modify-write instead
    /// of clobbering each other. The map only grows by distinct account, so it
    /// stays small for a beta host.
    pub(crate) fn store_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        self.store_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(account_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Where the job queue should mirror its history, or `None` when there is no
    /// local file store (the postgres host, which keeps history in memory). The
    /// `_jobs.json` name sits beside the other store-root sidecars
    /// (`_rate_limits`), distinct from the per-account `study.json` subdirs.
    pub(crate) fn job_history_path(&self) -> Option<PathBuf> {
        match &self.lock_data().storage {
            StudyStorageConfig::File { store_root } => Some(store_root.join("_jobs.json")),
            StudyStorageConfig::Postgres { .. } => None,
        }
    }

    /// Where the waitlist should persist entries, or `None` when there is no
    /// local file store. `_waitlist.json` sits beside the other store-root
    /// sidecars (`_jobs.json`, `_rate_limits`).
    pub(crate) fn waitlist_store_path(&self) -> Option<PathBuf> {
        match &self.lock_data().storage {
            StudyStorageConfig::File { store_root } => Some(store_root.join("_waitlist.json")),
            StudyStorageConfig::Postgres { .. } => None,
        }
    }

    pub(crate) fn postgres_url(&self) -> Option<String> {
        match &self.lock_data().storage {
            StudyStorageConfig::Postgres { database_url } => Some(database_url.clone()),
            StudyStorageConfig::File { .. } => None,
        }
    }

    fn postgres_ready(&self) -> bool {
        let Some(database_url) = self.postgres_url() else {
            return true;
        };
        with_postgres_store(&database_url, |store| {
            store.ping().map_err(postgres_failure)
        })
        .is_ok()
    }

    pub(crate) fn generation_cost_for_run(
        &self,
        account_id: &str,
        run_id: &str,
    ) -> Result<i64, ApiFailure> {
        let Some(database_url) = self.postgres_url() else {
            return Ok(0);
        };
        with_postgres_store(&database_url, |store| {
            store
                .generation_cost_for_run(account_id, run_id)
                .map_err(postgres_failure)
        })
    }

    fn lock_data(&self) -> MutexGuard<'_, AccountRegistryData> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct AccountRegistryData {
    accounts: BTreeMap<String, AccountRecord>,
    browser_sessions: BTreeMap<String, BrowserSessionRecord>,
    auth_config: AuthConfig,
    generation_provider_config: Option<OpenRouterConfig>,
    storage: StudyStorageConfig,
    now_fn: fn() -> i64,
}

impl Default for AccountRegistryData {
    fn default() -> Self {
        Self {
            accounts: BTreeMap::new(),
            browser_sessions: BTreeMap::new(),
            auth_config: AuthConfig::default(),
            generation_provider_config: None,
            storage: StudyStorageConfig::default(),
            now_fn: wall_clock_ms,
        }
    }
}

#[derive(Clone, Debug)]
struct AccountRecord {
    session_token: String,
    store_path: PathBuf,
    sources: BTreeMap<String, SourceRecord>,
    submitted_reviews: BTreeMap<String, StudyViewResponse>,
}

#[derive(Clone, Debug)]
struct BrowserSessionRecord {
    account_id: String,
    session_token: String,
    csrf_token_hash: String,
    expires_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MagicLinkRequest {
    pub debug_link: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountRequest {
    pub email: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreated {
    pub account_id: String,
    pub session_token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSourceRequest {
    pub title: String,
    pub body: String,
    #[serde(default = "default_source_permission")]
    pub permission: SourcePermission,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub source_id: String,
    pub title: String,
    pub body: String,
    pub permission: SourcePermission,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_expires_at: Option<i64>,
}

fn default_source_permission() -> SourcePermission {
    SourcePermission::ModelEligible
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectDeckRequest {
    pub project_key: String,
    pub title: String,
    pub body: String,
    pub ttl_expires_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InvalidateProjectDeckRequest {
    pub event: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeckRecord {
    pub deck_id: String,
    pub project_key: String,
    pub source: SourceRecord,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceList {
    pub sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubmitReviewRequest {
    pub answer: String,
    pub response_time_ms: u32,
    pub idempotency_key: String,
}

/// Blocking Postgres phases observed for one browser review submission.
///
/// `None` means the phase did not run. Callers must not coerce an absent
/// phase to zero because auth, validation, and connection failures stop at
/// different boundaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubmitReviewTimings {
    connect_ms: Option<u64>,
    operation_ms: Option<u64>,
    statement_count: Option<u64>,
}

impl SubmitReviewTimings {
    #[must_use]
    pub const fn postgres_connect_ms(self) -> Option<u64> {
        self.connect_ms
    }

    #[must_use]
    pub const fn postgres_operation_ms(self) -> Option<u64> {
        self.operation_ms
    }

    #[must_use]
    pub const fn postgres_statement_count(self) -> Option<u64> {
        self.statement_count
    }

    pub(crate) fn record_postgres_connect(&mut self, duration_ms: u64) {
        self.connect_ms = Some(
            self.connect_ms
                .unwrap_or_default()
                .saturating_add(duration_ms),
        );
    }

    pub(crate) fn record_postgres_operation(&mut self, duration_ms: u64) {
        self.operation_ms = Some(
            self.operation_ms
                .unwrap_or_default()
                .saturating_add(duration_ms),
        );
    }

    pub(crate) fn record_postgres_statement_count(&mut self, count: u64) {
        self.statement_count = Some(
            self.statement_count
                .unwrap_or_default()
                .saturating_add(count),
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitPerformanceOutcome {
    Succeeded,
    ClientRejected,
    ServerFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitViewport {
    Mobile,
    Tablet,
    Desktop,
}

/// One browser-reported completion for a single `/app/submit` round trip:
/// the five raw phase durations from tap to graded-visible, joined to the
/// server request/trace ids, plus the viewport class. Groups what would
/// otherwise be an eight-argument call into one coherent value so
/// [`report_submit_browser_performance`] stays within clippy's
/// `too_many_arguments` limit.
#[derive(Clone, Copy, Debug)]
pub struct BrowserSubmitReceipt<'a> {
    pub request_id: &'a str,
    pub trace_id: &'a str,
    pub tap_to_ack_ms: u64,
    pub request_to_response_ms: u64,
    pub transfer_ms: u64,
    pub navigation_ms: u64,
    pub graded_visible_ms: u64,
    pub viewport: SubmitViewport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentFeedbackRequest {
    pub verdict: ContentFeedbackVerdict,
    pub rationale: Option<String>,
    pub idempotency_key: String,
    pub supersedes_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyViewResponse {
    pub drafts: Vec<BetaStudyDraftRow>,
    pub current: Option<BetaStudyCurrent>,
    pub concept_progress: Vec<BetaStudyConceptProgress>,
    pub summary: BetaStudySummary,
    pub due_count: usize,
    #[serde(default)]
    pub generation_notices: Vec<String>,
}

impl StudyViewResponse {
    fn from_view(view: BetaStudyView) -> Self {
        Self {
            drafts: view.drafts,
            current: view.current,
            concept_progress: view.concept_progress,
            summary: view.summary,
            due_count: view.due_count,
            generation_notices: view.generation_notices,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub return_notification_scheduler: SchedulerHealth,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub worker_started: bool,
    pub postgres: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppAccount {
    browser_session_id: String,
    account_id: String,
    session_token: String,
    csrf_token: String,
}

impl AppAccount {
    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    #[must_use]
    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    #[must_use]
    pub fn csrf_token(&self) -> &str {
        &self.csrf_token
    }
}

#[derive(Debug)]
pub struct ApiFailure {
    status: StatusCode,
    pub message: String,
}

impl ApiFailure {
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    #[must_use]
    pub fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn unknown_account() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "Account not found.".to_owned(),
        }
    }

    #[must_use]
    pub fn not_found(message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn conflict(message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn conflict_message(message: String) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message,
        }
    }

    #[must_use]
    pub fn missing_session() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "Session token is required.".to_owned(),
        }
    }

    #[must_use]
    pub fn forbidden_account() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "Session token does not match account.".to_owned(),
        }
    }

    #[must_use]
    pub fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn too_many_requests(message: &'static str) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn service_unavailable(message: String) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message,
        }
    }

    #[must_use]
    pub fn internal(message: String) -> Self {
        report_internal_error(&message);
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }

    #[must_use]
    pub fn payload_too_large(message: &'static str) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn is_session_expired(&self) -> bool {
        self.status == StatusCode::UNAUTHORIZED
    }

    #[must_use]
    pub fn is_magic_link_recovery(&self) -> bool {
        self.status == StatusCode::FORBIDDEN && self.message == "Magic link is invalid or expired."
    }
}

/// Process-wide Canary reporter, installed once by the binary entry point.
/// Unset (tests, local dev without credentials) means reporting is a no-op.
static CANARY: std::sync::OnceLock<Option<memory_engine_canary::CanaryReporter>> =
    std::sync::OnceLock::new();

/// Install the Canary reporter from the environment. Call once at startup;
/// later calls are ignored.
pub fn init_error_reporting() {
    let _ = CANARY.set(
        memory_engine_canary::CanaryConfig::from_env().map(|mut config| {
            if let Ok(environment) = std::env::var("MEMORY_ENGINE_ENVIRONMENT") {
                config.environment = environment;
            }
            memory_engine_canary::CanaryReporter::new(config)
        }),
    );
}

/// Drain and stop the process-wide Canary worker during graceful shutdown.
///
/// Returns `false` only when an installed reporter misses the deadline.
pub fn shutdown_error_reporting(deadline: std::time::Duration) -> bool {
    CANARY
        .get()
        .and_then(Option::as_ref)
        .is_none_or(|reporter| reporter.shutdown(deadline))
}

pub fn report_health_check_in() {
    if let Some(reporter) = CANARY.get().and_then(Option::as_ref) {
        reporter.check_in(&memory_engine_canary::CheckInEvent {
            monitor: "memory-engine-api".to_owned(),
            status: memory_engine_canary::CheckInStatus::Alive,
            summary: "memory-engine-api heartbeat".to_owned(),
            ttl_ms: 120_000,
            context: Some(serde_json::json!({
                "source": "memory-engine-api",
            })),
        });
    }
}

pub fn report_submit_server_performance(duration_ms: u64, outcome: SubmitPerformanceOutcome) {
    let outcome = match outcome {
        SubmitPerformanceOutcome::Succeeded => memory_engine_performance::Outcome::Succeeded,
        SubmitPerformanceOutcome::ClientRejected => {
            memory_engine_performance::Outcome::ClientRejected
        }
        SubmitPerformanceOutcome::ServerFailed => memory_engine_performance::Outcome::ServerFailed,
    };
    let marker = memory_engine_performance::CompletionMarker::server(
        memory_engine_performance::Action::Review(memory_engine_performance::ReviewAction::Submit),
        memory_engine_performance::CompletionPhase::ImmediateAck,
        outcome,
    );
    report_performance_observation(marker, duration_ms);
}

/// Emits the two Canary-aggregated completion phases plus a queryable,
/// content-free receipt of the full five-duration decomposition.
///
/// `memory_engine_performance::CompletionPhase` is a frozen v1 contract
/// (`cardinality_payload_and_rate_budgets_are_calculated_and_bounded` pins its
/// series count; growing it requires a new schema version). Only
/// `ImmediateAck` and `VisibleAfterTwoAnimationFrames` have a matching phase
/// there, so those two remain the Canary-aggregated observations exactly as
/// before. `request_to_response_ms`, `transfer_ms`, and `navigation_ms` are
/// the intermediate breakdown of `graded_visible_ms` and have no phase to
/// attach to under v1; [`report_browser_submit_durations_receipt`] keeps them
/// queryable through the same production-log path the Canary batch export
/// already relies on, without bumping the frozen series cardinality.
pub fn report_submit_browser_performance(receipt: BrowserSubmitReceipt<'_>) {
    let performance_viewport = match receipt.viewport {
        SubmitViewport::Mobile => memory_engine_performance::Viewport::Mobile,
        SubmitViewport::Tablet => memory_engine_performance::Viewport::Tablet,
        SubmitViewport::Desktop => memory_engine_performance::Viewport::Desktop,
    };
    for (phase, duration_ms) in [
        (
            memory_engine_performance::CompletionPhase::ImmediateAck,
            receipt.tap_to_ack_ms,
        ),
        (
            memory_engine_performance::CompletionPhase::VisibleAfterTwoAnimationFrames,
            receipt.graded_visible_ms,
        ),
    ] {
        let marker = memory_engine_performance::CompletionMarker::browser(
            memory_engine_performance::Action::Review(
                memory_engine_performance::ReviewAction::Submit,
            ),
            phase,
            memory_engine_performance::Outcome::Succeeded,
            memory_engine_performance::Navigation::FullPage,
            performance_viewport,
        );
        report_performance_observation(marker, duration_ms);
    }
    println!("{}", report_browser_submit_durations_receipt(receipt));
}

/// Build (without printing) the queryable five-duration receipt for one
/// browser submit completion. Pure and side-effect free so the full
/// decomposition can be asserted on directly in tests; see
/// [`report_submit_browser_performance`] for why this exists alongside the
/// two Canary observations.
fn report_browser_submit_durations_receipt(receipt: BrowserSubmitReceipt<'_>) -> serde_json::Value {
    let viewport = match receipt.viewport {
        SubmitViewport::Mobile => "mobile",
        SubmitViewport::Tablet => "tablet",
        SubmitViewport::Desktop => "desktop",
    };
    serde_json::json!({
        "schema": "memory_engine.browser_submit_durations.v1",
        "request_id": receipt.request_id,
        "trace_id": receipt.trace_id,
        "viewport": viewport,
        "tap_to_ack_ms": receipt.tap_to_ack_ms,
        "request_to_response_ms": receipt.request_to_response_ms,
        "transfer_ms": receipt.transfer_ms,
        "navigation_ms": receipt.navigation_ms,
        "graded_visible_ms": receipt.graded_visible_ms,
    })
}

fn report_performance_observation(
    marker: Result<
        memory_engine_performance::CompletionMarker,
        memory_engine_performance::MarkerError,
    >,
    duration_ms: u64,
) {
    let Some(reporter) = CANARY.get().and_then(Option::as_ref) else {
        return;
    };
    let Ok(marker) = marker else {
        return;
    };
    if let Ok(observation) = marker.observation(duration_ms) {
        let _ = reporter.report_performance(observation);
    }
}

pub fn start_health_reporting_loop() {
    if CANARY.get().and_then(Option::as_ref).is_none() {
        return;
    }

    report_health_check_in();
    std::thread::Builder::new()
        .name("canary-health".to_owned())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            report_health_check_in();
        })
        .ok();
}

fn report_internal_error(message: &str) {
    if let Some(reporter) = CANARY.get().and_then(Option::as_ref) {
        reporter.report(&memory_engine_canary::ErrorEvent {
            error_class: "ApiFailure::internal".to_owned(),
            message: message.to_owned(),
            severity: memory_engine_canary::Severity::Error,
            context: None,
            fingerprint: Vec::new(),
        });
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiError {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[must_use]
pub fn normalize_email(email: &str) -> Option<String> {
    let trimmed = email.trim().to_ascii_lowercase();
    let (local, domain) = trimmed.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !domain.split('.').all(|part| !part.is_empty())
        || !domain.contains('.')
    {
        return None;
    }

    Some(trimmed)
}

fn normalize_required_text(text: &str, label: &'static str) -> Result<String, ApiFailure> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ApiFailure::bad_request(match label {
            "Source title" => "Source title must not be blank.",
            "Source body" => "Source body must not be blank.",
            "Review answer" => "Review answer must not be blank.",
            "Review unit prompt" => "Review unit prompt must not be blank.",
            "Review unit expected answer" => "Review unit expected answer must not be blank.",
            "Idempotency key" => "Idempotency key must not be blank.",
            _ => "Value must not be blank.",
        }));
    }

    if label.contains("body") && trimmed.len() > MAX_SOURCE_BODY_BYTES {
        return Err(ApiFailure::payload_too_large(
            "Source body exceeds the 256 KiB generation limit.",
        ));
    }
    if label.contains("title") && trimmed.len() > MAX_SOURCE_TITLE_BYTES {
        return Err(ApiFailure::payload_too_large(
            "Source title exceeds the 512 byte limit.",
        ));
    }

    Ok(trimmed.to_owned())
}

/// Reads the API session token from request headers.
///
/// # Errors
///
/// Returns an API failure when neither the explicit session header nor the
/// bearer authorization header contains a usable token.
pub fn read_session_token(headers: &HeaderMap) -> Result<&str, ApiFailure> {
    headers
        .get("x-session-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| read_bearer_session_token(headers))
        .ok_or_else(ApiFailure::missing_session)
}

fn read_bearer_session_token(headers: &HeaderMap) -> Option<&str> {
    let authorization = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(char::is_whitespace)?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token);
        }
    }

    None
}

fn read_browser_session_id(headers: &HeaderMap) -> Result<&str, ApiFailure> {
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == APP_SESSION_COOKIE_NAME && !value.trim().is_empty()).then_some(value)
            })
        })
        .ok_or_else(ApiFailure::missing_session)
}

#[must_use]
pub fn client_rate_limit_key(headers: &HeaderMap) -> String {
    ["do-connecting-ip", "x-real-ip", "x-forwarded-for"]
        .into_iter()
        .find_map(|header| {
            headers
                .get(header)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(',').next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn csrf_token(value: Option<&String>) -> &str {
    value.map(String::as_str).map(str::trim).unwrap_or_default()
}

pub fn html_with_browser_session(account: &AppAccount, html: String) -> Response {
    let mut response = Html(html).into_response();
    if let Ok(value) = HeaderValue::from_str(&session_cookie_header(&account.browser_session_id)) {
        response.headers_mut().insert(SET_COOKIE, value);
    } else {
        report_internal_error("failed to build browser session cookie header");
    }
    response
}

pub fn html_with_cleared_browser_session(html: String) -> Response {
    let mut response = Html(html).into_response();
    if let Ok(value) = HeaderValue::from_str(&clear_session_cookie_header()) {
        response.headers_mut().insert(SET_COOKIE, value);
    } else {
        report_internal_error("failed to build clear-session cookie header");
    }
    response
}

fn session_cookie_header(session_id: &str) -> String {
    format!(
        "{APP_SESSION_COOKIE_NAME}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={APP_SESSION_MAX_AGE_SECONDS}",
        cookie_value(session_id)
    )
}

fn clear_session_cookie_header() -> String {
    format!("{APP_SESSION_COOKIE_NAME}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

fn cookie_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        .collect()
}

fn secret_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn require_account_session(account: &AccountRecord, session_token: &str) -> Result<(), ApiFailure> {
    if account.session_token == session_token {
        return Ok(());
    }

    Err(ApiFailure::forbidden_account())
}

fn account_id_for(email: &str) -> String {
    let stable = email.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });

    format!("acct_{stable:016x}")
}

fn new_session_token() -> String {
    format!("sess_{:032x}", rand::random::<u128>())
}

fn new_browser_session_id() -> String {
    format!("browser_{:032x}", rand::random::<u128>())
}

/// Derive a session's CSRF token from its server-side session secret.
///
/// The token is a pure function of `session_token`, which never leaves the
/// server — the browser cookie carries only the opaque session id. So any
/// cookie-authenticated render, including a plain `GET /` of the home, can emit
/// valid CSRF-protected forms without the token being stored or threaded
/// through. An attacker mounting a cross-site request knows neither the session
/// secret nor this one-way derivation of it, so they cannot forge the token; and
/// because the digest is one-way, exposing a form's CSRF token never reveals the
/// session secret. Validation stays hash-based: the session record holds
/// `secret_hash(session_csrf_token(session_token))`, unchanged.
fn session_csrf_token(session_token: &str) -> String {
    format!("csrf_{}", secret_hash(&format!("csrf:{session_token}")))
}

fn new_magic_link_token() -> String {
    format!("magic_{:032x}", rand::random::<u128>())
}

pub const APP_SESSION_COOKIE_NAME: &str = "__Host-memory_engine_session";
const APP_SESSION_MAX_AGE_SECONDS: u64 = 60 * 60 * 24 * 14;
pub const APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS: u32 = 5;
pub const MAX_SOURCE_BODY_BYTES: usize = 256 * 1024;
pub const MAX_SOURCE_TITLE_BYTES: usize = 512;
const APP_ACCOUNT_RATE_LIMIT_WINDOW_MS: i64 = 15 * 60 * 1_000;
/// Same policy shape as the magic-link request limiter: five attempts per
/// window, keyed by normalized email and by client IP.
pub const WAITLIST_RATE_LIMIT_MAX_ATTEMPTS: u32 = 5;
const WAITLIST_RATE_LIMIT_WINDOW_MS: i64 = 15 * 60 * 1_000;
// 30 minutes: links travel through email, where spam checks and device
// switches routinely burn ten minutes. Found in dogfood: a link expired
// before the operator could click it.
pub const AUTH_CHALLENGE_TTL_MS: i64 = 30 * 60 * 1_000;
/// At most one due-count reminder per account per day, apart from the
/// one-time confirmation sent immediately after an explicit opt-in.
pub const RETURN_NOTIFICATION_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000;
pub const RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

fn source_id_for(account_id: &str, title: &str, body: &str) -> String {
    let stable = [account_id, title, body]
        .into_iter()
        .flat_map(str::bytes)
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });

    format!("src_{stable:016x}")
}

fn project_deck_id_for(account_id: &str, project_key: &str, title: &str, body: &str) -> String {
    let stable = [account_id, project_key, title, body]
        .into_iter()
        .flat_map(str::bytes)
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });

    format!("deck_{stable:016x}")
}

fn account_store_path(store_root: &FsPath, account_id: &str) -> PathBuf {
    store_root.join(account_id).join("study.json")
}

fn account_session_path(store_root: &FsPath, account_id: &str) -> PathBuf {
    store_root.join(account_id).join("session.token")
}

fn browser_session_path(store_root: &FsPath, session_id: &str) -> PathBuf {
    store_root
        .join("_browser_sessions")
        .join(secret_hash(session_id))
        .join("session")
}

fn auth_challenge_path(store_root: &FsPath, challenge_hash: &str) -> PathBuf {
    store_root
        .join("_auth_challenges")
        .join(cookie_value(challenge_hash))
        .join("challenge")
}

fn auth_challenge_consumed_path(store_root: &FsPath, challenge_hash: &str) -> PathBuf {
    store_root
        .join("_auth_challenges")
        .join(cookie_value(challenge_hash))
        .join("consumed")
}

fn rate_limit_path(store_root: &FsPath, key: &str) -> PathBuf {
    store_root.join("_rate_limits").join(secret_hash(key))
}

/// Write `bytes` to `path` atomically and crash-durably: write a uniquely-named
/// sibling temp file, fsync it, rename it over the target, then fsync the parent
/// directory so the rename itself survives a power loss. The rename is atomic on
/// POSIX, so a crash mid-write leaves either the old file or the new — never a
/// truncated or half-written one. The randomized temp name lets concurrent
/// writers to *different* targets share this helper safely.
pub(crate) fn write_atomic(path: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension(format!("tmp-{:032x}", rand::random::<u128>()));
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temp_path, path)?;
    if let Some(parent) = path.parent() {
        // Best-effort: a crash before this lands could lose the rename, and not
        // every platform supports directory fsync — neither is worth failing on.
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Generate drafts for one source using the configured provider.
///
/// When `OPENROUTER_API_KEY` is set, `ModelEligible` arbitrary prose routes to
/// the model via a [`FallbackProvider`] whose primary is the deterministic
/// structured-block parser. `LocalOnly` sources always take the structured
/// parser path, so they remain usable without model or network access.
fn run_source_generation<S>(
    study: &mut BetaStudySession<S>,
    source_id: &str,
    generation_provider_config: Option<OpenRouterConfig>,
) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    let run_id = format!("study-run-{:032x}", rand::random::<u128>());
    run_source_generation_with_run_id(study, source_id, &run_id, generation_provider_config)
}

pub(crate) fn run_source_generation_with_run_id<S>(
    study: &mut BetaStudySession<S>,
    source_id: &str,
    run_id: &str,
    generation_provider_config: Option<OpenRouterConfig>,
) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    let ids = Some(vec![source_id.to_owned()]);
    let local_only = study
        .view()
        .map_err(study_failure)?
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .is_some_and(|source| source.permission == SourcePermission::LocalOnly);
    if local_only {
        return study
            .generate_with_run_id(ids, run_id)
            .map_err(study_failure);
    }
    match generation_provider_config {
        Some(config) => {
            let model = OpenRouterProvider::new(config);
            let provider = FallbackProvider::new(&model);
            study.generate_with_provider_and_run_id(ids, &provider, run_id)
        }
        None => study.generate_with_run_id(ids, run_id),
    }
    .map_err(study_failure)
}

#[cfg(test)]
pub(crate) fn run_source_generation_with_provider<S>(
    study: &mut BetaStudySession<S>,
    source_id: &str,
    provider: Option<&dyn DraftProvider>,
) -> Result<
    BetaStudyView,
    memory_engine_study::BetaStudyError<<S as memory_engine_service::MemoryServiceStore>::Error>,
>
where
    S: memory_engine_study::BetaStudyStore,
{
    let local_only = study
        .view()?
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .is_some_and(|source| source.permission == SourcePermission::LocalOnly);
    let ids = Some(vec![source_id.to_owned()]);
    if local_only {
        study.generate(ids)
    } else if let Some(provider) = provider {
        study.generate_with_provider(ids, provider)
    } else {
        study.generate(ids)
    }
}

fn run_reference_generation<S>(
    study: &mut BetaStudySession<S>,
    generation_provider_config: Option<OpenRouterConfig>,
) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    let authorization = study
        .current_source_authorization()
        .map_err(study_failure)?;
    if authorization.local_only_source_id().is_some() {
        return study.learn_more().map_err(study_failure);
    }
    match generation_provider_config {
        Some(config) => {
            let model = OpenRouterProvider::new(config);
            study.learn_more_with_provider(&model)
        }
        None => study.learn_more(),
    }
    .map_err(study_failure)
}

fn run_bridge_generation<S>(
    study: &mut BetaStudySession<S>,
    generation_provider_config: Option<OpenRouterConfig>,
) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    let authorization = study
        .current_source_authorization()
        .map_err(study_failure)?;
    if authorization.local_only_source_id().is_some() {
        return study.generate_bridge_material().map_err(study_failure);
    }
    match generation_provider_config {
        Some(config) => {
            let model = OpenRouterProvider::new(config);
            study.generate_bridge_material_with_provider(&model)
        }
        None => study.generate_bridge_material(),
    }
    .map_err(study_failure)
}

fn open_study_session(path: &FsPath, now: fn() -> i64) -> Result<BetaStudySession, ApiFailure> {
    BetaStudySession::open(BetaStudyOptions::new(path).with_clock(now)).map_err(study_failure)
}

fn open_persistence_store(path: &FsPath) -> Result<BetaPersistenceStore, ApiFailure> {
    BetaPersistenceStore::open(path).map_err(|error| ApiFailure::internal(error.to_string()))
}

/// Wall-clock milliseconds since the Unix epoch: the production time source.
fn wall_clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[must_use]
pub fn app_session_max_age_ms() -> i64 {
    i64::try_from(APP_SESSION_MAX_AGE_SECONDS)
        .unwrap_or(i64::MAX)
        .saturating_mul(1_000)
}

fn with_postgres_account<R>(
    database_url: &str,
    account_id: &str,
    now_ms: i64,
    operation: impl FnOnce(AccountStudyStore<'_>) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let run = || {
        let mut store = connect_postgres_migrated(database_url)?;
        let scope = AccountScope::new(account_id.to_owned()).map_err(postgres_failure)?;
        let mut account = store.for_account(scope);
        account.ensure_account(now_ms).map_err(postgres_failure)?;

        operation(account)
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(run)
    } else {
        run()
    }
}

fn with_postgres_account_timed<R>(
    database_url: &str,
    account_id: &str,
    now_ms: i64,
    timings: Option<&mut SubmitReviewTimings>,
    operation: impl FnOnce(AccountStudyStore<'_>) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let Some(timings) = timings else {
        return with_postgres_account(database_url, account_id, now_ms, operation);
    };
    let run = || {
        let connect_started = std::time::Instant::now();
        let connected = PostgresStudyStore::connect(database_url).map_err(postgres_failure);
        timings.record_postgres_connect(bounded_elapsed_ms(connect_started));
        let mut store = connected?;

        let operation_started = std::time::Instant::now();
        let result = (|| {
            migrate_postgres_store(database_url, &mut store)?;
            let scope = AccountScope::new(account_id.to_owned()).map_err(postgres_failure)?;
            let mut account = store.for_account(scope);
            account.ensure_account(now_ms).map_err(postgres_failure)?;
            operation(account)
        })();
        timings.record_postgres_operation(bounded_elapsed_ms(operation_started));
        timings.record_postgres_statement_count(store.statement_count());
        result
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(run)
    } else {
        run()
    }
}

fn bounded_elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .clamp(1, memory_engine_performance::REQUEST_UI_MAX_DURATION_MS)
}

fn with_postgres_store<R>(
    database_url: &str,
    operation: impl FnOnce(&mut PostgresStudyStore) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let run = || {
        let mut store = connect_postgres_migrated(database_url)?;

        operation(&mut store)
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(run)
    } else {
        run()
    }
}
fn with_postgres_store_timed<R>(
    database_url: &str,
    timings: Option<&mut SubmitReviewTimings>,
    operation: impl FnOnce(&mut PostgresStudyStore) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let Some(timings) = timings else {
        return with_postgres_store(database_url, operation);
    };
    let run = || {
        let connect_started = std::time::Instant::now();
        let connected = PostgresStudyStore::connect(database_url).map_err(postgres_failure);
        timings.record_postgres_connect(bounded_elapsed_ms(connect_started));
        let mut store = connected?;

        let operation_started = std::time::Instant::now();
        let result = (|| {
            migrate_postgres_store(database_url, &mut store)?;
            operation(&mut store)
        })();
        timings.record_postgres_operation(bounded_elapsed_ms(operation_started));
        timings.record_postgres_statement_count(store.statement_count());
        result
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(run)
    } else {
        run()
    }
}

fn with_postgres_study<R>(
    database_url: &str,
    account_id: &str,
    now: fn() -> i64,
    operation: impl FnOnce(&mut BetaStudySession<AccountStudyStore<'_>>) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    with_postgres_account(database_url, account_id, now(), |account| {
        let mut study = BetaStudySession::from_store(account, now);
        operation(&mut study)
    })
}

/// Connect to Postgres, running migrations only the first time this process
/// sees a given database URL. Previously every request re-ran the DDL
/// migration batch; at daily-use traffic that is pure overhead and DDL lock
/// pressure. The set is keyed by URL so tests pointing at scratch databases
/// still migrate each one.
fn connect_postgres_migrated(database_url: &str) -> Result<PostgresStudyStore, ApiFailure> {
    let mut store = PostgresStudyStore::connect(database_url).map_err(postgres_failure)?;
    migrate_postgres_store(database_url, &mut store)?;
    Ok(store)
}

fn migrate_postgres_store(
    database_url: &str,
    store: &mut PostgresStudyStore,
) -> Result<(), ApiFailure> {
    static MIGRATED_URLS: std::sync::LazyLock<
        std::sync::Mutex<std::collections::BTreeSet<String>>,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));

    let migrated = &*MIGRATED_URLS;
    // A panic while migrating must not poison every later request into a
    // panic; the set is a plain string collection, safe to keep using.
    let mut migrated = migrated
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !migrated.contains(database_url) {
        store.migrate().map_err(postgres_failure)?;
        migrated.insert(database_url.to_owned());
    }
    Ok(())
}

fn postgres_failure(error: memory_engine_persistence_postgres::PostgresStoreError) -> ApiFailure {
    let message = error.to_string();
    drop(error);
    ApiFailure::internal(message)
}

fn file_content_feedback_failure(error: ContentFeedbackError<BetaStoreError>) -> ApiFailure {
    match error {
        ContentFeedbackError::BlankFeedbackId | ContentFeedbackError::BlankAccountId => {
            ApiFailure::bad_request("Content feedback request is invalid.")
        }
        ContentFeedbackError::Store(BetaStoreError::UnknownReviewUnit(_)) => {
            ApiFailure::not_found("Review unit not found.")
        }
        ContentFeedbackError::Store(
            BetaStoreError::FeedbackSupersedesUnknown(_)
            | BetaStoreError::FeedbackSupersedesOtherReviewUnit(_)
            | BetaStoreError::FeedbackSupersedesOtherAccount(_),
        ) => ApiFailure::bad_request("Feedback supersedes an invalid revision."),
        ContentFeedbackError::Store(
            BetaStoreError::DuplicateContentFeedback(_)
            | BetaStoreError::FeedbackSupersedesStale { .. },
        ) => ApiFailure::conflict("Feedback conflicts with the current revision."),
        ContentFeedbackError::Store(error) => ApiFailure::internal(error.to_string()),
    }
}

fn postgres_content_feedback_failure(
    error: ContentFeedbackError<PostgresStoreError>,
) -> ApiFailure {
    match error {
        ContentFeedbackError::BlankFeedbackId | ContentFeedbackError::BlankAccountId => {
            ApiFailure::bad_request("Content feedback request is invalid.")
        }
        ContentFeedbackError::Store(PostgresStoreError::UnknownReviewUnit(_)) => {
            ApiFailure::not_found("Review unit not found.")
        }
        ContentFeedbackError::Store(
            PostgresStoreError::FeedbackSupersedesUnknown(_)
            | PostgresStoreError::FeedbackSupersedesOtherReviewUnit(_)
            | PostgresStoreError::FeedbackSupersedesOtherAccount(_)
            | PostgresStoreError::FeedbackAccountMismatch,
        ) => ApiFailure::bad_request("Feedback supersedes an invalid revision."),
        ContentFeedbackError::Store(
            PostgresStoreError::DuplicateContentFeedback(_)
            | PostgresStoreError::FeedbackSupersedesStale { .. },
        ) => ApiFailure::conflict("Feedback conflicts with the current revision."),
        ContentFeedbackError::Store(error) => ApiFailure::internal(error.to_string()),
    }
}

fn persisted_sources(path: &FsPath) -> Result<Vec<SourceRecord>, ApiFailure> {
    let store = open_persistence_store(path)?;
    Ok(store
        .snapshot()
        .source_documents
        .into_iter()
        .filter(|source| source.archived_at.is_none())
        .map(|source| SourceRecord {
            source_id: source.id,
            title: source.title,
            body: source.body.unwrap_or_default(),
            permission: source.permission,
            project_key: source.project_key,
            ttl_expires_at: source.ttl_expires_at,
        })
        .collect())
}

fn persisted_source_exists(path: &FsPath, source_id: &str) -> Result<bool, ApiFailure> {
    let store = open_persistence_store(path)?;
    Ok(store
        .snapshot()
        .source_documents
        .iter()
        .any(|source| source.id == source_id && source.archived_at.is_none()))
}

fn persisted_project_deck_exists(path: &FsPath, deck_id: &str) -> Result<bool, ApiFailure> {
    let store = open_persistence_store(path)?;
    Ok(store.snapshot().source_documents.iter().any(|source| {
        source.id == deck_id && source.archived_at.is_none() && source.project_key.is_some()
    }))
}

fn study_failure<E: std::fmt::Display>(
    error: memory_engine_study::BetaStudyError<E>,
) -> ApiFailure {
    if matches!(&error, memory_engine_study::BetaStudyError::NoConceptKey) {
        return ApiFailure::bad_request("The active review unit must have a nonblank concept key.");
    }
    let message = error.to_string();
    drop(error);
    ApiFailure::internal(message)
}

fn require_current_review(
    study: &mut BetaStudySession,
    review_unit_id: &str,
) -> Result<(), ApiFailure> {
    let view = study.start().map_err(study_failure)?;
    let Some(current) = view.current else {
        return Err(ApiFailure::not_found("Review unit not found."));
    };
    if current.review_unit_id.to_string() == review_unit_id {
        return Ok(());
    }

    Err(ApiFailure::not_found("Review unit not found."))
}

fn require_current_review_postgres(
    study: &mut BetaStudySession<AccountStudyStore<'_>>,
    review_unit_id: &str,
) -> Result<(), ApiFailure> {
    let view = study.start().map_err(study_failure)?;
    let Some(current) = view.current else {
        return Err(ApiFailure::not_found("Review unit not found."));
    };
    if current.review_unit_id.to_string() == review_unit_id {
        return Ok(());
    }

    Err(ApiFailure::not_found("Review unit not found."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CountingConfiguredProvider {
        calls: std::cell::Cell<usize>,
    }

    impl memory_engine_generation::DraftProvider for CountingConfiguredProvider {
        fn model(&self) -> memory_engine_persistence::GeneratedPromptModel {
            memory_engine_persistence::GeneratedPromptModel {
                provider: "counting".to_owned(),
                name: "configured".to_owned(),
                version: "test".to_owned(),
            }
        }

        fn generate_drafts(
            &self,
            _source: &memory_engine_persistence::SourceDocument,
        ) -> Result<
            memory_engine_generation::ProviderDrafts,
            memory_engine_generation::ProviderFailure,
        > {
            self.calls.set(self.calls.get() + 1);
            Ok(memory_engine_generation::ProviderDrafts {
                model: self.model(),
                learning_intent: None,
                candidates: Vec::new(),
                failures: Vec::new(),
                usage: None,
            })
        }
    }

    #[test]
    fn client_rate_limit_key_prefers_digitalocean_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("do-connecting-ip", HeaderValue::from_static("203.0.113.10"));
        headers.insert("x-real-ip", HeaderValue::from_static("10.0.0.1"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("10.0.0.2, 10.0.0.3"),
        );

        assert_eq!(client_rate_limit_key(&headers), "203.0.113.10");
    }

    #[tokio::test]
    async fn scheduler_handle_shutdown_stops_and_joins_the_owned_task() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-scheduler-lifecycle-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        fs::create_dir_all(&root).expect("scheduler store root");
        let state = ApiState::new(AccountRegistry::with_store_root(&root));
        let handle = state.start_return_notification_scheduler_with_interval(
            Duration::from_millis(1),
            ReturnNotificationSchedulerConfig { batch_size: 1 },
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.shutdown().await;
        let health = state.scheduler_health();
        assert!(!health.enabled);
        assert!(!health.running);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_validation_rejects_oversized_generation_input() {
        let body = "x".repeat(MAX_SOURCE_BODY_BYTES + 1);
        let error = normalize_required_text(&body, "Source body").expect_err("body is bounded");
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);

        let title = "x".repeat(MAX_SOURCE_TITLE_BYTES + 1);
        let error = normalize_required_text(&title, "Source title").expect_err("title is bounded");
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn readiness_requires_the_worker_even_when_file_dependencies_are_local() {
        let readiness = ApiState::default().readiness();
        assert_eq!(readiness.status, "not_ready");
        assert!(!readiness.worker_started);
        assert!(readiness.postgres);
    }

    #[test]
    fn local_only_generation_uses_structured_path_when_external_provider_is_configured() {
        let directory = std::env::temp_dir().join(format!(
            "memory-engine-api-state-local-only-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("study.json");
        let mut study = BetaStudySession::open(BetaStudyOptions::new(&path)).expect("study");
        study
            .add_source(memory_engine_study::BetaStudySourceInput {
                id: "local-source".to_owned(),
                title: "Local source".to_owned(),
                body: "Concept: NATO letter A\nQuestion: What word cues A?\nAnswer: ALFA"
                    .to_owned(),
                project_key: None,
                ttl_expires_at: None,
                permission: SourcePermission::LocalOnly,
            })
            .expect("source");
        let provider = CountingConfiguredProvider::default();

        let view = run_source_generation_with_provider(&mut study, "local-source", Some(&provider))
            .expect("local generation");

        assert!(!view.drafts.is_empty(), "local source should produce cards");
        assert_eq!(
            provider.calls.get(),
            0,
            "configured model must not be called"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn local_only_reference_and_bridge_ignore_the_runtime_generation_provider_config() {
        let directory = std::env::temp_dir().join(format!(
            "memory-engine-api-state-local-only-reference-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("study.json");
        let mut study = BetaStudySession::open(BetaStudyOptions::new(&path)).expect("study");
        study
            .add_source(memory_engine_study::BetaStudySourceInput {
                id: "local-source".to_owned(),
                title: "Local source".to_owned(),
                body: "Concept: NATO letter A\nQuestion: What word cues A?\nAnswer: ALFA"
                    .to_owned(),
                project_key: None,
                ttl_expires_at: None,
                permission: SourcePermission::LocalOnly,
            })
            .expect("source");

        let config = OpenRouterConfig {
            api_key: "test-key".to_owned(),
            model: "test-model".to_owned(),
            base_url: "http://127.0.0.1:9".to_owned(),
            timeout: std::time::Duration::from_millis(1),
            prompt: memory_engine_openrouter::PromptVariant::Principled,
            max_drafts: 1,
            proxy_socket: None,
        };

        let generated = study.generate(None).expect("generate");
        let draft_id = generated.drafts.first().expect("draft").id.clone();
        study.approve_draft(&draft_id).expect("approve");
        study.start().expect("start reference session");

        let reference = run_reference_generation(&mut study, Some(config.clone()))
            .expect("local reference generation");
        assert!(
            reference.current.is_some(),
            "local reference should still render"
        );

        study.start().expect("start bridge session");
        let bridge =
            run_bridge_generation(&mut study, Some(config)).expect("local bridge generation");
        assert!(bridge.current.is_some(), "local bridge should still render");

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn browser_submit_durations_receipt_retains_all_five_durations() {
        // memory-engine-109 review finding: report_submit_browser_performance
        // used to accept and forward only tap_to_ack_ms/graded_visible_ms,
        // silently discarding request_to_response_ms/transfer_ms/navigation_ms
        // at ingestion. The receipt must carry every one of the five raw
        // values through unchanged, joined to the same request/trace ids.
        let receipt = report_browser_submit_durations_receipt(BrowserSubmitReceipt {
            request_id: "req_0123456789abcdef0123456789abcdef",
            trace_id: "trace_0123456789abcdef0123456789abcdef",
            tap_to_ack_ms: 11,
            request_to_response_ms: 22,
            transfer_ms: 33,
            navigation_ms: 44,
            graded_visible_ms: 99,
            viewport: SubmitViewport::Mobile,
        });
        assert_eq!(
            receipt,
            serde_json::json!({
                "schema": "memory_engine.browser_submit_durations.v1",
                "request_id": "req_0123456789abcdef0123456789abcdef",
                "trace_id": "trace_0123456789abcdef0123456789abcdef",
                "viewport": "mobile",
                "tap_to_ack_ms": 11,
                "request_to_response_ms": 22,
                "transfer_ms": 33,
                "navigation_ms": 44,
                "graded_visible_ms": 99,
            })
        );
    }
}
