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
    sync::{Arc, Mutex, MutexGuard},
};

#[cfg(unix)]
use std::{fs::OpenOptions, os::fd::AsRawFd};

use axum::{
    http::{
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Response},
    Json,
};
use hmac::Hmac;
use memory_engine_generation::{FallbackProvider, StructuredBlockProvider};
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider};
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

mod jobs;
mod registry;
mod storage;

pub use jobs::{EnqueueOutcome, GenerationJob, JobBroadcast, JobQueue, JobStatus};
pub use storage::StudyStorage;

use storage::StudyStorageConfig;

#[derive(Clone)]
pub struct ApiState {
    accounts: AccountRegistry,
    jobs: JobQueue,
}

impl ApiState {
    #[must_use]
    pub fn new(accounts: AccountRegistry) -> Self {
        // The file-backed host mirrors job history to disk so the activity log
        // survives a restart; the postgres host keeps it in memory for now.
        let jobs = match accounts.job_history_path() {
            Some(path) => JobQueue::with_persistence(accounts.clone(), path),
            None => JobQueue::new(accounts.clone()),
        };
        Self { accounts, jobs }
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

    /// List saved material for a browser-authenticated account.
    ///
    /// # Errors
    ///
    /// Returns an API failure when persistence rejects the read.
    pub fn list_app_sources(&self, account: &AppAccount) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.accounts
            .list_sources(account.account_id(), account.session_token())
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
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.accounts.submit_review(
            account.account_id(),
            account.session_token(),
            review_unit_id,
            request,
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

    /// Return one job by id. Test helper for route coverage.
    #[doc(hidden)]
    #[must_use]
    pub fn job(&self, job_id: &str) -> Option<GenerationJob> {
        self.jobs.job(job_id)
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
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            allowed_emails: None,
            expose_debug_links: false,
            link_delivery: AuthLinkDelivery::None,
            unsubscribe_secret: format!("unsubscribe_{:032x}", rand::random::<u128>()),
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
    storage: StudyStorageConfig,
    now_fn: fn() -> i64,
}

impl Default for AccountRegistryData {
    fn default() -> Self {
        Self {
            accounts: BTreeMap::new(),
            browser_sessions: BTreeMap::new(),
            auth_config: AuthConfig::default(),
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub source_id: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_expires_at: Option<i64>,
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
    pub fn internal(message: String) -> Self {
        report_internal_error(&message);
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
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
            "Idempotency key" => "Idempotency key must not be blank.",
            _ => "Value must not be blank.",
        }));
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
const APP_ACCOUNT_RATE_LIMIT_WINDOW_MS: i64 = 15 * 60 * 1_000;
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

#[cfg(unix)]
struct FileAccountLock {
    _file: fs::File,
}

#[cfg(unix)]
fn acquire_file_account_lock(store_path: &FsPath) -> Result<FileAccountLock, ApiFailure> {
    let lock_path = store_path.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| ApiFailure::internal(error.to_string()))?;
    // The descriptor owns the lock. It is never reclaimed from metadata and
    // the lockfile is never deleted, so a delayed/drop path cannot affect a
    // replacement owner. Contention is deliberately a prompt 409: callers
    // are async route workers and must not sleep on a Tokio worker.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(FileAccountLock { _file: file });
    }
    let error = std::io::Error::last_os_error();
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
    {
        return Err(ApiFailure::conflict(
            "The account study store is busy; try again shortly.",
        ));
    }
    Err(ApiFailure::internal(error.to_string()))
}

#[cfg(not(unix))]
fn acquire_file_account_lock(_store_path: &FsPath) -> Result<(), ApiFailure> {
    Err(ApiFailure::internal(
        "File account locking is unsupported on this platform.",
    ))
}

pub(crate) fn with_file_account_lock<R>(
    store_path: &FsPath,
    operation: impl FnOnce() -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let _lock = acquire_file_account_lock(store_path)?;
    operation()
}

/// Generate drafts for one source using the configured provider.
///
/// When `OPENROUTER_API_KEY` is set, arbitrary prose routes to the model via
/// a [`FallbackProvider`] whose primary is the deterministic structured-block
/// parser, so hand-written `Concept:/Question:/Answer:` blocks keep their free
/// path. Without a key the structured parser runs alone, which is what CI and
/// key-less deployments use.
fn run_source_generation<S>(
    study: &mut BetaStudySession<S>,
    source_id: &str,
) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    let ids = Some(vec![source_id.to_owned()]);
    match OpenRouterConfig::from_env() {
        Ok(config) => {
            let structured = StructuredBlockProvider;
            let model = OpenRouterProvider::new(config);
            let provider = FallbackProvider::new(&structured, &model);
            study.generate_with_provider(ids, &provider)
        }
        Err(_) => study.generate(ids),
    }
    .map_err(study_failure)
}

fn run_reference_generation<S>(study: &mut BetaStudySession<S>) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    #[cfg(test)]
    {
        study.learn_more().map_err(study_failure)
    }

    #[cfg(not(test))]
    {
        match OpenRouterConfig::from_env() {
            Ok(config) => {
                let model = OpenRouterProvider::new(config);
                study.learn_more_with_provider(&model)
            }
            Err(_) => study.learn_more(),
        }
        .map_err(study_failure)
    }
}

fn run_bridge_generation<S>(study: &mut BetaStudySession<S>) -> Result<BetaStudyView, ApiFailure>
where
    S: memory_engine_study::BetaStudyStore,
    <S as memory_engine_service::MemoryServiceStore>::Error: std::fmt::Display,
{
    #[cfg(test)]
    {
        study.generate_bridge_material().map_err(study_failure)
    }

    #[cfg(not(test))]
    {
        match OpenRouterConfig::from_env() {
            Ok(config) => {
                let model = OpenRouterProvider::new(config);
                study.generate_bridge_material_with_provider(&model)
            }
            Err(_) => study.generate_bridge_material(),
        }
        .map_err(study_failure)
    }
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
    static MIGRATED_URLS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeSet<String>>,
    > = std::sync::OnceLock::new();

    let mut store = PostgresStudyStore::connect(database_url).map_err(postgres_failure)?;
    let migrated =
        MIGRATED_URLS.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));
    // A panic while migrating must not poison every later request into a
    // panic; the set is a plain string collection, safe to keep using.
    let mut migrated = migrated
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !migrated.contains(database_url) {
        store.migrate().map_err(postgres_failure)?;
        migrated.insert(database_url.to_owned());
    }

    Ok(store)
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

    #[test]
    fn independent_file_store_owners_cannot_steal_a_live_descriptor_lock() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-file-lock-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store_path = root.join("study.json");
        let first = acquire_file_account_lock(&store_path).expect("first descriptor lock");
        let second = acquire_file_account_lock(&store_path);
        assert!(matches!(second, Err(error) if error.status() == StatusCode::CONFLICT));
        drop(first);
        let replacement = acquire_file_account_lock(&store_path).expect("released lock");
        drop(replacement);
        let _ = fs::remove_dir_all(root);
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
}
