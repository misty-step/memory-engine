use std::{
    collections::HashSet,
    fmt, fs, io,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use memory_engine_persistence::SourcePermission;
use memory_engine_service::{
    record_content_feedback, ContentFeedback, RecordContentFeedbackCommand,
};
use memory_engine_study::{BetaStudySession, BetaStudySourceInput};

use crate::{
    account_store_path, app_session_max_age_ms, auth_challenge_consumed_path, auth_challenge_path,
    browser_session_path, file_content_feedback_failure, file_study_failure, is_secret_hash,
    persisted_project_deck_exists, persisted_source_exists, persisted_sources,
    postgres_content_feedback_failure, postgres_failure, postgres_study_failure, rate_limit_path,
    require_current_review, require_current_review_postgres, run_bridge_generation,
    run_reference_generation, run_source_generation, run_source_generation_with_run_id,
    secret_hash, session_csrf_token, study_failure, with_postgres_account,
    with_postgres_account_timed, with_postgres_store, with_postgres_store_timed,
    with_postgres_study, write_atomic, ApiFailure, BrowserSessionRecord, ReturnNotificationClaim,
    ReturnNotificationClaimRequest, ReturnNotificationPreference, SourceRecord, StudyViewResponse,
    SubmitReviewRequest, SubmitReviewTimings,
};

#[derive(Clone, Debug)]
pub(crate) enum StudyStorageConfig {
    File { store_root: PathBuf },
    Postgres { database_url: String },
}

impl Default for StudyStorageConfig {
    fn default() -> Self {
        Self::File {
            store_root: std::env::temp_dir().join(format!(
                "memory-engine-api-{}-{}",
                std::process::id(),
                rand::random::<u128>()
            )),
        }
    }
}

impl StudyStorageConfig {
    pub(crate) fn file(store_root: impl Into<PathBuf>) -> Self {
        Self::File {
            store_root: store_root.into(),
        }
    }

    pub(crate) fn postgres(database_url: impl Into<String>) -> Self {
        Self::Postgres {
            database_url: database_url.into(),
        }
    }

    pub(crate) fn storage(
        &self,
        now: fn() -> i64,
        generation_provider_config: Option<memory_engine_openrouter::OpenRouterConfig>,
    ) -> StudyStorage {
        match self {
            Self::File { store_root } => StudyStorage::new(FileStudyStorage {
                store_root: store_root.clone(),
                now,
                generation_provider_config: generation_provider_config.clone(),
            }),
            Self::Postgres { database_url } => StudyStorage::new(PostgresStudyStorage {
                database_url: database_url.clone(),
                now,
                generation_provider_config,
            }),
        }
    }
}

#[derive(Clone)]
pub struct StudyStorage {
    inner: Arc<dyn StudyStorageAdapter>,
}

fn order_content_feedback_for_copy(
    feedback: Vec<ContentFeedback>,
) -> Result<Vec<ContentFeedback>, String> {
    let mut remaining = feedback;
    let mut copied_ids = HashSet::with_capacity(remaining.len());
    let mut ordered = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let next_index = remaining
            .iter()
            .enumerate()
            .filter(|(_, feedback)| {
                feedback
                    .supersedes_id
                    .as_ref()
                    .is_none_or(|parent_id| copied_ids.contains(parent_id))
            })
            .min_by(|(_, left), (_, right)| {
                left.review_unit_id
                    .as_str()
                    .cmp(right.review_unit_id.as_str())
                    .then(left.occurred_at.cmp(&right.occurred_at))
                    .then(left.id.cmp(&right.id))
            })
            .map(|(index, _)| index);

        let Some(next_index) = next_index else {
            return Err("content feedback ancestry is not a deterministic DAG".to_owned());
        };
        let next = remaining.swap_remove(next_index);
        copied_ids.insert(next.id.clone());
        ordered.push(next);
    }

    Ok(ordered)
}

fn current_content_feedback_head(
    feedback: &[ContentFeedback],
    account_id: &str,
    review_unit_id: &str,
) -> Option<String> {
    feedback
        .iter()
        .filter(|candidate| {
            candidate.account_id == account_id
                && candidate.review_unit_id.as_str() == review_unit_id
        })
        .filter(|candidate| {
            !feedback.iter().any(|child| {
                child.account_id == account_id
                    && child.review_unit_id.as_str() == review_unit_id
                    && child.supersedes_id.as_deref() == Some(candidate.id.as_str())
            })
        })
        .max_by_key(|candidate| (candidate.occurred_at, candidate.id.as_str()))
        .map(|candidate| candidate.id.clone())
}

impl fmt::Debug for StudyStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudyStorage")
            .field("inner", &self.inner)
            .finish()
    }
}

impl StudyStorage {
    fn new(storage: impl StudyStorageAdapter + 'static) -> Self {
        Self {
            inner: Arc::new(storage),
        }
    }

    #[must_use]
    pub fn file(store_root: impl Into<PathBuf>, now: fn() -> i64) -> Self {
        Self::new(FileStudyStorage {
            store_root: store_root.into(),
            now,
            generation_provider_config: None,
        })
    }

    pub(crate) fn account_store_path(&self, account_id: &str) -> PathBuf {
        self.inner.account_store_path(account_id)
    }

    pub(crate) fn save_account_session(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<(), ApiFailure> {
        self.inner.save_account_session(account_id, session_token)
    }

    pub(crate) fn account_session_matches_with_timings(
        &self,
        account_id: &str,
        session_token: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        self.inner
            .account_session_matches_with_timings(account_id, session_token, timings)
    }

    pub(crate) fn revoke_account_session(
        &self,
        account_id: &str,
        session_token: &str,
        now_ms: i64,
    ) -> Result<bool, ApiFailure> {
        self.inner
            .revoke_account_session(account_id, session_token, now_ms)
    }

    /// Revokes every standalone account/API session for `account_id`.
    /// Browser sessions are an independent scope: any session currently
    /// backing a signed-in browser is preserved, not revoked. Use
    /// [`Self::revoke_browser_sessions_for_account`] to sign out browsers.
    pub(crate) fn revoke_account_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner
            .revoke_account_sessions_for_account(account_id, now_ms)
    }

    pub(crate) fn save_browser_session(
        &self,
        session_id: &str,
        session: &BrowserSessionRecord,
    ) -> Result<(), ApiFailure> {
        self.inner.save_browser_session(session_id, session)
    }

    pub(crate) fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        self.inner.load_browser_session(session_id)
    }
    pub(crate) fn load_browser_session_with_timings(
        &self,
        session_id: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        self.inner
            .load_browser_session_with_timings(session_id, timings)
    }

    pub(crate) fn revoke_browser_session(
        &self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner.revoke_browser_session(session_id, now_ms)
    }

    pub(crate) fn revoke_browser_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner
            .revoke_browser_sessions_for_account(account_id, now_ms)
    }

    /// Persists a magic-link auth challenge.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the storage adapter cannot persist the challenge.
    pub fn save_auth_challenge(
        &self,
        challenge_hash: &str,
        email: &str,
        expires_at_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner
            .save_auth_challenge(challenge_hash, email, expires_at_ms)
    }

    /// Consumes a magic-link auth challenge at most once.
    ///
    /// # Errors
    ///
    /// Returns an API failure when the storage adapter cannot read or mark the challenge.
    pub fn consume_auth_challenge(
        &self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, ApiFailure> {
        self.inner.consume_auth_challenge(challenge_hash, now_ms)
    }

    pub(crate) fn save_return_notification_preference(
        &self,
        account_id: &str,
        email: &str,
        enabled: bool,
        last_sent_at_ms: Option<i64>,
        unsubscribe_nonce: &str,
    ) -> Result<(), ApiFailure> {
        self.inner.save_return_notification_preference(
            account_id,
            email,
            enabled,
            last_sent_at_ms,
            unsubscribe_nonce,
        )
    }

    pub(crate) fn load_return_notification_preference(
        &self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, ApiFailure> {
        self.inner.load_return_notification_preference(account_id)
    }

    pub(crate) fn disable_return_notification(
        &self,
        account_id: &str,
        email: &str,
        current_nonce: &str,
        next_nonce: &str,
        updated_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        self.inner.disable_return_notification(
            account_id,
            email,
            current_nonce,
            next_nonce,
            updated_at_ms,
        )
    }

    pub(crate) fn claim_return_notification(
        &self,
        request: &ReturnNotificationClaimRequest,
    ) -> Result<Option<ReturnNotificationClaim>, ApiFailure> {
        self.inner.claim_return_notification(request)
    }

    pub(crate) fn complete_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        sent_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        self.inner
            .complete_return_notification(account_id, claim_id, sent_at_ms)
    }

    pub(crate) fn release_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner
            .release_return_notification(account_id, claim_id, now_ms)
    }

    pub(crate) fn enabled_return_notification_accounts(
        &self,
        limit: usize,
        now_ms: i64,
        interval_ms: i64,
    ) -> Result<Vec<String>, ApiFailure> {
        self.inner
            .enabled_return_notification_accounts(limit, now_ms, interval_ms)
    }

    pub(crate) fn record_rate_limit_attempts(
        &self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: u32,
    ) -> Result<bool, ApiFailure> {
        self.inner
            .record_rate_limit_attempts(keys, now_ms, window_ms, max_attempts)
    }

    pub(crate) fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        self.inner.account_exists(account_id)
    }
    pub(crate) fn account_exists_with_timings(
        &self,
        account_id: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        self.inner.account_exists_with_timings(account_id, timings)
    }

    pub(crate) fn copy_account(
        &self,
        source_account_id: &str,
        target_account_id: &str,
        source_store_path: &FsPath,
    ) -> Result<(), ApiFailure> {
        self.inner
            .copy_account(source_account_id, target_account_id, source_store_path)
    }

    pub(crate) fn save_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source: &SourceRecord,
    ) -> Result<(), ApiFailure> {
        self.inner.save_source(account_id, store_path, source)
    }

    pub(crate) fn list_sources(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.list_sources_with_timings(account_id, store_path, None)
    }

    pub(crate) fn list_sources_with_timings(
        &self,
        account_id: &str,
        store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.inner
            .list_sources_with_timings(account_id, store_path, timings)
    }

    pub(crate) fn update_source_permission(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure> {
        self.inner
            .update_source_permission(account_id, store_path, source_id, permission)
    }

    pub(crate) fn generate_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .generate_source(account_id, store_path, source_id)
    }

    pub(crate) fn generate_source_with_run_id(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.generate_source_with_run_id(
            account_id,
            store_path,
            source_id,
            run_id,
            generation_attempt,
            lease_token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finalize_generation_run(
        &self,
        account_id: &str,
        store_path: &FsPath,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
        now_ms: i64,
        lease_valid: bool,
    ) -> Result<bool, ApiFailure> {
        self.inner.finalize_generation_run(
            account_id,
            store_path,
            run_id,
            generation_attempt,
            lease_token,
            now_ms,
            lease_valid,
        )
    }

    /// Archive a source and every review unit generated from it. Returns the
    /// view plus the count of cards actually retired by this call (across
    /// every generation run for the source) — the caller reports this count
    /// to the learner rather than a generic notice (memory-engine-088).
    pub(crate) fn archive_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        self.inner.archive_source(account_id, store_path, source_id)
    }

    pub(crate) fn invalidate_project_deck(
        &self,
        account_id: &str,
        store_path: &FsPath,
        deck_id: &str,
        invalidated_at: i64,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .invalidate_project_deck(account_id, store_path, deck_id, invalidated_at)
    }

    pub(crate) fn keep_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.keep_draft(account_id, store_path, draft_id)
    }

    pub(crate) fn edit_pending_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .edit_pending_draft(account_id, store_path, draft_id, prompt, expected_answer)
    }

    pub(crate) fn reject_pending_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .reject_pending_draft(account_id, store_path, draft_id)
    }

    pub(crate) fn next_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.next_review(account_id, store_path)
    }

    pub(crate) fn study_view(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.study_view(account_id, store_path)
    }
    pub(crate) fn study_view_with_timings(
        &self,
        account_id: &str,
        store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .study_view_with_timings(account_id, store_path, timings)
    }

    pub(crate) fn reveal_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .reveal_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn learn_more_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .learn_more_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn skip_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .skip_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn snooze_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .snooze_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn snooze_concept_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .snooze_concept_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn delete_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .delete_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn edit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.edit_review(
            account_id,
            store_path,
            review_unit_id,
            prompt,
            expected_answer,
        )
    }

    pub(crate) fn bridge_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .bridge_review(account_id, store_path, review_unit_id)
    }

    pub(crate) fn submit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        request: SubmitReviewRequest,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .submit_review(account_id, store_path, review_unit_id, request, timings)
    }

    pub(crate) fn record_content_feedback(
        &self,
        account_id: &str,
        store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure> {
        self.inner
            .record_content_feedback(account_id, store_path, command)
    }

    pub(crate) fn content_feedback_head(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<Option<String>, ApiFailure> {
        self.inner
            .content_feedback_head(account_id, store_path, review_unit_id)
    }
}

trait StudyStorageAdapter: fmt::Debug + Send + Sync {
    fn account_store_path(&self, account_id: &str) -> PathBuf;
    fn save_account_session(&self, account_id: &str, session_token: &str)
        -> Result<(), ApiFailure>;
    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure>;
    fn revoke_account_session(
        &self,
        account_id: &str,
        session_token: &str,
        now_ms: i64,
    ) -> Result<bool, ApiFailure>;
    /// Browser sessions are an independent scope and must be preserved; see
    /// the [`StudyStorage::revoke_account_sessions_for_account`] facade doc.
    fn revoke_account_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure>;
    fn account_session_matches_with_timings(
        &self,
        account_id: &str,
        session_token: &str,
        _timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        self.account_session_matches(account_id, session_token)
    }
    fn save_browser_session(
        &self,
        session_id: &str,
        session: &BrowserSessionRecord,
    ) -> Result<(), ApiFailure>;
    fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure>;
    fn load_browser_session_with_timings(
        &self,
        session_id: &str,
        _timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        self.load_browser_session(session_id)
    }
    fn revoke_browser_session(&self, session_id: &str, now_ms: i64) -> Result<(), ApiFailure>;
    fn revoke_browser_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure>;
    fn save_auth_challenge(
        &self,
        challenge_hash: &str,
        email: &str,
        expires_at_ms: i64,
    ) -> Result<(), ApiFailure>;
    fn consume_auth_challenge(
        &self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, ApiFailure>;
    fn save_return_notification_preference(
        &self,
        account_id: &str,
        email: &str,
        enabled: bool,
        last_sent_at_ms: Option<i64>,
        unsubscribe_nonce: &str,
    ) -> Result<(), ApiFailure>;
    fn load_return_notification_preference(
        &self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, ApiFailure>;
    fn disable_return_notification(
        &self,
        account_id: &str,
        email: &str,
        current_nonce: &str,
        next_nonce: &str,
        updated_at_ms: i64,
    ) -> Result<bool, ApiFailure>;
    fn claim_return_notification(
        &self,
        request: &ReturnNotificationClaimRequest,
    ) -> Result<Option<ReturnNotificationClaim>, ApiFailure>;
    fn complete_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        sent_at_ms: i64,
    ) -> Result<bool, ApiFailure>;
    fn release_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure>;
    fn enabled_return_notification_accounts(
        &self,
        limit: usize,
        now_ms: i64,
        interval_ms: i64,
    ) -> Result<Vec<String>, ApiFailure>;
    fn record_rate_limit_attempts(
        &self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: u32,
    ) -> Result<bool, ApiFailure>;
    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure>;
    fn account_exists_with_timings(
        &self,
        account_id: &str,
        _timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        self.account_exists(account_id)
    }
    fn copy_account(
        &self,
        source_account_id: &str,
        target_account_id: &str,
        source_store_path: &FsPath,
    ) -> Result<(), ApiFailure>;
    fn save_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source: &SourceRecord,
    ) -> Result<(), ApiFailure>;
    fn list_sources(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure>;
    fn list_sources_with_timings(
        &self,
        account_id: &str,
        store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        let _ = timings;
        self.list_sources(account_id, store_path)
    }
    fn update_source_permission(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure>;
    fn generate_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn generate_source_with_run_id(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        run_id: &str,
        _generation_attempt: i32,
        _lease_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let _ = run_id;
        self.generate_source(account_id, store_path, source_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_generation_run(
        &self,
        _account_id: &str,
        _store_path: &FsPath,
        _run_id: &str,
        _generation_attempt: i32,
        _lease_token: &str,
        _now_ms: i64,
        _lease_valid: bool,
    ) -> Result<bool, ApiFailure> {
        Ok(true)
    }
    fn archive_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure>;
    fn invalidate_project_deck(
        &self,
        account_id: &str,
        store_path: &FsPath,
        deck_id: &str,
        invalidated_at: i64,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn keep_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn edit_pending_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn reject_pending_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn next_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn study_view(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn study_view_with_timings(
        &self,
        account_id: &str,
        store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let _ = timings;
        self.study_view(account_id, store_path)
    }
    fn reveal_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn learn_more_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn skip_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn snooze_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn snooze_concept_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn delete_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn edit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn bridge_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn submit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        request: SubmitReviewRequest,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn record_content_feedback(
        &self,
        account_id: &str,
        store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure>;
    fn content_feedback_head(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<Option<String>, ApiFailure>;
}

#[derive(Debug)]
struct FileStudyStorage {
    store_root: PathBuf,
    now: fn() -> i64,
    generation_provider_config: Option<memory_engine_openrouter::OpenRouterConfig>,
}

impl FileStudyStorage {
    fn now_ms(&self) -> i64 {
        (self.now)()
    }

    #[cfg(test)]
    fn generate_source_with_provider(
        &self,
        store_path: &FsPath,
        source_id: &str,
        provider: &dyn memory_engine_generation::DraftProvider,
    ) -> Result<StudyViewResponse, ApiFailure> {
        if !persisted_source_exists(store_path, source_id)? {
            return Err(ApiFailure::not_found("Source not found."));
        }
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view =
            crate::run_source_generation_with_provider(&mut study, source_id, Some(provider))
                .map_err(study_failure)?;
        Ok(StudyViewResponse::from_view(view))
    }

    fn with_locked_study<R>(
        &self,
        store_path: &FsPath,
        operation: impl FnOnce(&mut BetaStudySession) -> Result<R, ApiFailure>,
    ) -> Result<R, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        operation(&mut study)
    }

    fn migrate_legacy_account_session(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        let legacy_path = self.store_root.join(account_id).join("session.token");
        let Ok(raw) = fs::read_to_string(&legacy_path) else {
            return Ok(());
        };
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(());
        }
        let token_hash = secret_hash(raw);
        let expires_at_ms = now_ms.saturating_add(app_session_max_age_ms());
        let path = self
            .store_root
            .join("_api_sessions")
            .join(&token_hash)
            .join("session");
        write_atomic(
            &path,
            format!("{account_id}\n{now_ms}\n{expires_at_ms}\n\n").as_bytes(),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let persisted =
            fs::read_to_string(&path).map_err(|error| ApiFailure::internal(error.to_string()))?;
        let valid = persisted.lines().next() == Some(account_id)
            && persisted
                .lines()
                .nth(1)
                .and_then(|value| value.parse::<i64>().ok())
                == Some(now_ms)
            && persisted
                .lines()
                .nth(2)
                .and_then(|value| value.parse::<i64>().ok())
                == Some(expires_at_ms);
        if !valid {
            return Err(ApiFailure::internal(
                "legacy session migration verification failed; account remains locked".to_owned(),
            ));
        }
        if let Err(error) = fs::remove_file(&legacy_path) {
            // Never leave a usable hashed session beside a raw legacy token.
            // Mark the replacement revoked and fail closed; an operator can
            // restore the account from the documented store snapshot.
            let lock_record = format!("{account_id}\n{now_ms}\n{expires_at_ms}\n{now_ms}\n");
            let _ = write_atomic(&path, lock_record.as_bytes());
            return Err(ApiFailure::internal(format!(
                "legacy session cleanup failed; account remains locked: {error}"
            )));
        }
        Ok(())
    }

    fn migrate_legacy_browser_session_token(
        &self,
        path: &FsPath,
        raw_token: &str,
    ) -> Result<String, ApiFailure> {
        let _lock =
            crate::file_lock::acquire_blocking(&self.store_root.join("_browser_sessions.lock"))?;
        let Ok(saved) = fs::read_to_string(path) else {
            return Ok(secret_hash(raw_token));
        };
        let mut lines = saved.lines().map(str::to_owned).collect::<Vec<_>>();
        let Some(current_token) = lines.get(1).map(String::as_str) else {
            return Ok(secret_hash(raw_token));
        };
        if is_secret_hash(current_token) {
            return Ok(current_token.to_owned());
        }
        if current_token != raw_token {
            return Ok(secret_hash(current_token));
        }
        let token_hash = secret_hash(raw_token);
        lines[1].clone_from(&token_hash);
        if let Some(csrf_hash) = lines.get_mut(2) {
            *csrf_hash = secret_hash(&session_csrf_token(&token_hash));
        }
        write_atomic(path, format!("{}\n", lines.join("\n")).as_bytes())
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        Ok(token_hash)
    }

    /// Hashes of the account/API session credentials that currently back a
    /// live (non-revoked) browser session for `account_id`. Browser and
    /// machine session scopes are independent, so revoking every standalone
    /// API/service session must preserve whichever of these are backing a
    /// signed-in browser.
    fn browser_backed_session_hashes_for_account(
        &self,
        account_id: &str,
    ) -> Result<HashSet<String>, ApiFailure> {
        let root = self.store_root.join("_browser_sessions");
        let Ok(entries) = fs::read_dir(root) else {
            return Ok(HashSet::new());
        };
        let mut hashes = HashSet::new();
        for entry in entries {
            let entry = entry.map_err(|error| ApiFailure::internal(error.to_string()))?;
            let path = entry.path().join("session");
            let Ok(saved) = fs::read_to_string(&path) else {
                continue;
            };
            let mut lines = saved.lines();
            if lines.next() != Some(account_id) {
                continue;
            }
            let Some(session_token_hash) = lines.next() else {
                continue;
            };
            let revoked_at_ms = lines.nth(3).and_then(|value| value.parse::<i64>().ok());
            if revoked_at_ms.is_none() {
                hashes.insert(session_token_hash.to_owned());
            }
        }
        Ok(hashes)
    }
}

impl StudyStorageAdapter for FileStudyStorage {
    fn account_store_path(&self, account_id: &str) -> PathBuf {
        account_store_path(&self.store_root, account_id)
    }

    fn save_account_session(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<(), ApiFailure> {
        let _lock =
            crate::file_lock::acquire_blocking(&self.store_root.join("_api_sessions.lock"))?;
        let token_hash = secret_hash(session_token);
        let now_ms = self.now_ms();
        let expires_at_ms = now_ms.saturating_add(app_session_max_age_ms());
        let path = self
            .store_root
            .join("_api_sessions")
            .join(&token_hash)
            .join("session");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        write_atomic(
            &path,
            format!("{account_id}\n{now_ms}\n{expires_at_ms}\n\n").as_bytes(),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let marker = self.store_root.join(account_id).join("account.marker");
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        write_atomic(&marker, format!("{now_ms}\n").as_bytes())
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        let token_hash = if is_secret_hash(session_token) {
            session_token.to_owned()
        } else {
            secret_hash(session_token)
        };
        let path = self
            .store_root
            .join("_api_sessions")
            .join(&token_hash)
            .join("session");
        if !path.exists() {
            let _lock = crate::file_lock::acquire(&self.store_root.join("_api_sessions.lock"))?;
            if !path.exists() {
                self.migrate_legacy_account_session(account_id, self.now_ms())?;
            }
        }
        let Ok(saved) = fs::read_to_string(path) else {
            return Ok(false);
        };
        let mut lines = saved.lines();
        let Some(saved_account_id) = lines.next() else {
            return Ok(false);
        };
        let _created_at_ms = lines.next().and_then(|value| value.parse::<i64>().ok());
        let Some(expires_at_ms) = lines.next().and_then(|value| value.parse::<i64>().ok()) else {
            return Ok(false);
        };
        let revoked_at_ms = lines.next().and_then(|value| value.parse::<i64>().ok());
        Ok(saved_account_id == account_id
            && revoked_at_ms.is_none()
            && expires_at_ms > self.now_ms())
    }

    fn revoke_account_session(
        &self,
        account_id: &str,
        session_token: &str,
        now_ms: i64,
    ) -> Result<bool, ApiFailure> {
        let _lock =
            crate::file_lock::acquire_blocking(&self.store_root.join("_api_sessions.lock"))?;
        let token_hash = secret_hash(session_token);
        let path = self
            .store_root
            .join("_api_sessions")
            .join(token_hash)
            .join("session");
        let Ok(saved) = fs::read_to_string(&path) else {
            return Ok(false);
        };
        let mut lines = saved.lines().map(str::to_owned).collect::<Vec<_>>();
        if lines.first().map(String::as_str) != Some(account_id)
            || lines.get(3).is_some_and(|value| !value.trim().is_empty())
        {
            return Ok(false);
        }
        while lines.len() < 4 {
            lines.push(String::new());
        }
        lines[3] = now_ms.to_string();
        write_atomic(
            &path,
            format!(
                "{}
",
                lines.join(
                    "
"
                )
            )
            .as_bytes(),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))?;
        Ok(true)
    }

    /// Revokes every standalone account/API session for `account_id`,
    /// preserving whichever ones currently back a live browser session (see
    /// [`Self::browser_backed_session_hashes_for_account`]). Browser and
    /// machine session scopes are independent: this is the "revoke all
    /// API/service sessions" operation and must not silently sign out the
    /// account's signed-in browsers.
    fn revoke_account_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        let preserved = self.browser_backed_session_hashes_for_account(account_id)?;
        let _lock =
            crate::file_lock::acquire_blocking(&self.store_root.join("_api_sessions.lock"))?;
        let root = self.store_root.join("_api_sessions");
        let Ok(entries) = fs::read_dir(root) else {
            return Ok(());
        };
        for entry in entries {
            let entry = entry.map_err(|error| ApiFailure::internal(error.to_string()))?;
            if preserved.contains(&entry.file_name().to_string_lossy().into_owned()) {
                continue;
            }
            let path = entry.path().join("session");
            let Ok(saved) = fs::read_to_string(&path) else {
                continue;
            };
            let mut lines = saved.lines().map(str::to_owned).collect::<Vec<_>>();
            if lines.first().map(String::as_str) != Some(account_id)
                || lines.get(3).is_some_and(|value| !value.trim().is_empty())
            {
                continue;
            }
            while lines.len() < 4 {
                lines.push(String::new());
            }
            lines[3] = now_ms.to_string();
            write_atomic(&path, format!("{}\n", lines.join("\n")).as_bytes())
                .map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        Ok(())
    }

    fn save_browser_session(
        &self,
        session_id: &str,
        session: &BrowserSessionRecord,
    ) -> Result<(), ApiFailure> {
        let path = browser_session_path(&self.store_root, session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        fs::write(
            path,
            format!(
                "{}\n{}\n{}\n{}\n{}\n\n",
                session.account_id,
                session.session_token,
                session.csrf_token_hash,
                session.expires_at_ms,
                self.now_ms()
            ),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        let path = browser_session_path(&self.store_root, session_id);
        let Ok(saved) = fs::read_to_string(&path) else {
            return Ok(None);
        };
        let mut lines = saved.lines();
        let Some(account_id) = lines.next() else {
            return Ok(None);
        };
        let Some(session_token) = lines.next() else {
            return Ok(None);
        };
        let legacy_raw_token = (!is_secret_hash(session_token)).then(|| session_token.to_owned());
        let session_token = if let Some(raw_token) = legacy_raw_token.as_deref() {
            self.migrate_legacy_browser_session_token(&path, raw_token)?
        } else {
            session_token.to_owned()
        };
        let Some(persisted_csrf_token_hash) = lines.next() else {
            return Ok(None);
        };
        let Some(expires_at_ms) = lines.next().and_then(|value| value.parse::<i64>().ok()) else {
            return Ok(None);
        };
        let _created_at_ms = lines.next().and_then(|value| value.parse::<i64>().ok());
        let revoked_at_ms = lines.next().and_then(|value| value.parse::<i64>().ok());
        if expires_at_ms <= self.now_ms() || revoked_at_ms.is_some() {
            return Ok(None);
        }
        let csrf_token_hash = if legacy_raw_token.is_some() {
            secret_hash(&session_csrf_token(&session_token))
        } else {
            persisted_csrf_token_hash.to_owned()
        };
        Ok(Some(BrowserSessionRecord {
            account_id: account_id.to_owned(),
            session_token,
            csrf_token_hash,
            expires_at_ms,
        }))
    }

    fn revoke_browser_session(&self, session_id: &str, now_ms: i64) -> Result<(), ApiFailure> {
        let path = browser_session_path(&self.store_root, session_id);
        let Ok(saved) = fs::read_to_string(&path) else {
            return Ok(());
        };
        let mut lines = saved.lines().map(str::to_owned).collect::<Vec<_>>();
        while lines.len() < 5 {
            lines.push(String::new());
        }
        if lines.len() == 5 {
            lines.push(now_ms.to_string());
        } else {
            lines[5] = now_ms.to_string();
        }
        write_atomic(&path, format!("{}\n", lines.join("\n")).as_bytes())
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn revoke_browser_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        let root = self.store_root.join("_browser_sessions");
        let Ok(entries) = fs::read_dir(root) else {
            return Ok(());
        };
        for entry in entries {
            let entry = entry.map_err(|error| ApiFailure::internal(error.to_string()))?;
            let path = entry.path().join("session");
            let Ok(saved) = fs::read_to_string(&path) else {
                continue;
            };
            if saved.lines().next() == Some(account_id) {
                let session_id_hash = entry.file_name().to_string_lossy().into_owned();
                let session_path = self
                    .store_root
                    .join("_browser_sessions")
                    .join(session_id_hash)
                    .join("session");
                let mut lines = saved.lines().map(str::to_owned).collect::<Vec<_>>();
                while lines.len() < 5 {
                    lines.push(String::new());
                }
                if lines.len() == 5 {
                    lines.push(now_ms.to_string());
                } else {
                    lines[5] = now_ms.to_string();
                }
                write_atomic(&session_path, format!("{}\n", lines.join("\n")).as_bytes())
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
            }
        }
        Ok(())
    }

    fn save_auth_challenge(
        &self,
        challenge_hash: &str,
        email: &str,
        expires_at_ms: i64,
    ) -> Result<(), ApiFailure> {
        let path = auth_challenge_path(&self.store_root, challenge_hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        fs::write(path, format!("{email}\n{expires_at_ms}\n\n"))
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn consume_auth_challenge(
        &self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, ApiFailure> {
        let path = auth_challenge_path(&self.store_root, challenge_hash);
        let consumed_path = auth_challenge_consumed_path(&self.store_root, challenge_hash);
        match fs::rename(&path, &consumed_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ApiFailure::internal(error.to_string())),
        }
        let saved = fs::read_to_string(&consumed_path)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let mut lines = saved.lines();
        let Some(email) = lines.next() else {
            return Ok(None);
        };
        let Some(expires_at_ms) = lines.next().and_then(|value| value.parse::<i64>().ok()) else {
            return Ok(None);
        };
        let consumed_at_ms = lines
            .next()
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| value.parse::<i64>().ok());
        if consumed_at_ms.is_some() || expires_at_ms <= now_ms {
            return Ok(None);
        }
        fs::write(
            consumed_path,
            format!("{email}\n{expires_at_ms}\n{now_ms}\n"),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))?;

        Ok(Some(email.to_owned()))
    }

    fn save_return_notification_preference(
        &self,
        account_id: &str,
        email: &str,
        enabled: bool,
        last_sent_at_ms: Option<i64>,
        unsubscribe_nonce: &str,
    ) -> Result<(), ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            crate::file_lock::acquire_blocking(&account_dir.join("return-notifications.lock"))?;
        let path = account_dir.join("return-notifications.json");
        let existing = match fs::read(&path) {
            Ok(bytes) => Some(
                serde_json::from_slice::<ReturnNotificationPreference>(&bytes)
                    .map_err(|error| ApiFailure::internal(error.to_string()))?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ApiFailure::internal(error.to_string())),
        };
        let preserve_pending = existing.as_ref().is_some_and(|preference| {
            preference.enabled
                && enabled
                && preference.email == email
                && preference.pending_delivery_key.is_some()
        });
        let preference = ReturnNotificationPreference {
            email: email.to_owned(),
            enabled,
            last_sent_at_ms,
            unsubscribe_nonce: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .map(|preference| preference.unsubscribe_nonce.clone())
                })
                .flatten()
                .unwrap_or_else(|| unsubscribe_nonce.to_owned()),
            claim_id: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.claim_id.clone())
                })
                .flatten(),
            claim_expires_at_ms: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.claim_expires_at_ms)
                })
                .flatten(),
            pending_delivery_key: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.pending_delivery_key.clone())
                })
                .flatten(),
            pending_due_count: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.pending_due_count)
                })
                .flatten(),
            pending_unsubscribe_expires_at_ms: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.pending_unsubscribe_expires_at_ms)
                })
                .flatten(),
            retry_attempts: if preserve_pending {
                existing
                    .as_ref()
                    .map_or(0, |preference| preference.retry_attempts)
            } else {
                0
            },
            next_retry_at_ms: preserve_pending
                .then(|| {
                    existing
                        .as_ref()
                        .and_then(|preference| preference.next_retry_at_ms)
                })
                .flatten(),
        };
        let bytes = serde_json::to_vec(&preference)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        write_atomic(&path, &bytes).map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn load_return_notification_preference(
        &self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, ApiFailure> {
        let path = self
            .store_root
            .join(account_id)
            .join("return-notifications.json");
        let Ok(bytes) = fs::read(path) else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn disable_return_notification(
        &self,
        account_id: &str,
        email: &str,
        current_nonce: &str,
        next_nonce: &str,
        _updated_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            crate::file_lock::acquire_blocking(&account_dir.join("return-notifications.lock"))?;
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(false);
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        if !preference.enabled
            || preference.email != email
            || preference.unsubscribe_nonce != current_nonce
        {
            return Ok(false);
        }
        preference.enabled = false;
        next_nonce.clone_into(&mut preference.unsubscribe_nonce);
        preference.claim_id = None;
        preference.claim_expires_at_ms = None;
        preference.pending_delivery_key = None;
        preference.pending_due_count = None;
        preference.pending_unsubscribe_expires_at_ms = None;
        preference.retry_attempts = 0;
        preference.next_retry_at_ms = None;
        let bytes = serde_json::to_vec(&preference)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        write_atomic(&path, &bytes)
            .map(|()| true)
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn claim_return_notification(
        &self,
        request: &ReturnNotificationClaimRequest,
    ) -> Result<Option<ReturnNotificationClaim>, ApiFailure> {
        let account_dir = self.store_root.join(&request.account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            crate::file_lock::acquire_blocking(&account_dir.join("return-notifications.lock"))?;
        let now_ms = request.now_ms;
        let granted_at_ms = (self.now)();
        let claim_ttl_ms = request
            .claim_expires_at_ms
            .saturating_sub(request.now_ms)
            .max(0);
        let unsubscribe_ttl_ms = request
            .unsubscribe_expires_at_ms
            .saturating_sub(request.now_ms)
            .max(0);
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(None);
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let interval_elapsed = preference
            .last_sent_at_ms
            .is_none_or(|sent| now_ms.saturating_sub(sent) >= request.interval_ms);
        let eligible = (preference.pending_delivery_key.is_some()
            && preference
                .next_retry_at_ms
                .is_none_or(|retry_at| retry_at <= now_ms))
            || (preference.pending_delivery_key.is_none()
                && interval_elapsed
                && (request.force_confirmation || request.due_count > 0));
        if !preference.enabled
            || !eligible
            || preference
                .claim_expires_at_ms
                .is_some_and(|expires| expires > now_ms)
        {
            return Ok(None);
        }
        let delivery_key = preference
            .pending_delivery_key
            .clone()
            .unwrap_or_else(|| request.delivery_key.clone());
        let unsubscribe_nonce = if preference.unsubscribe_nonce.is_empty() {
            request.unsubscribe_nonce.clone()
        } else {
            preference.unsubscribe_nonce.clone()
        };
        let due_count = preference.pending_due_count.unwrap_or(request.due_count);
        let unsubscribe_expires_at_ms = preference
            .pending_unsubscribe_expires_at_ms
            .unwrap_or_else(|| granted_at_ms.saturating_add(unsubscribe_ttl_ms));
        preference.claim_id = Some(request.claim_id.clone());
        preference.claim_expires_at_ms = Some(granted_at_ms.saturating_add(claim_ttl_ms));
        preference.pending_delivery_key = Some(delivery_key.clone());
        preference.pending_due_count = Some(due_count);
        preference.pending_unsubscribe_expires_at_ms = Some(unsubscribe_expires_at_ms);
        preference.unsubscribe_nonce.clone_from(&unsubscribe_nonce);
        let bytes = serde_json::to_vec(&preference)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        write_atomic(&path, &bytes).map_err(|error| ApiFailure::internal(error.to_string()))?;
        Ok(Some(ReturnNotificationClaim {
            email: preference.email,
            due_count,
            delivery_key,
            unsubscribe_nonce,
            unsubscribe_expires_at_ms,
            claim_id: request.claim_id.clone(),
        }))
    }

    fn complete_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        _sent_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            crate::file_lock::acquire_blocking(&account_dir.join("return-notifications.lock"))?;
        let sent_at_ms = (self.now)();
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(false);
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        if preference.claim_id.as_deref() != Some(claim_id) {
            return Ok(false);
        }
        preference.last_sent_at_ms = Some(sent_at_ms);
        preference.claim_id = None;
        preference.claim_expires_at_ms = None;
        preference.pending_delivery_key = None;
        preference.pending_due_count = None;
        preference.pending_unsubscribe_expires_at_ms = None;
        preference.retry_attempts = 0;
        preference.next_retry_at_ms = None;
        let bytes = serde_json::to_vec(&preference)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        write_atomic(&path, &bytes).map_err(|error| ApiFailure::internal(error.to_string()))?;
        Ok(true)
    }

    fn release_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        _now_ms: i64,
    ) -> Result<(), ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            crate::file_lock::acquire_blocking(&account_dir.join("return-notifications.lock"))?;
        let now_ms = (self.now)();
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(());
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        if preference.claim_id.as_deref() == Some(claim_id) {
            preference.claim_id = None;
            preference.claim_expires_at_ms = None;
            let delay_ms = 60_000_i64.saturating_mul(
                1_i64
                    .checked_shl(preference.retry_attempts.min(9))
                    .unwrap_or(512)
                    .min(360),
            );
            preference.retry_attempts = preference.retry_attempts.saturating_add(1);
            preference.next_retry_at_ms =
                Some(now_ms.saturating_add(delay_ms.min(6 * 60 * 60 * 1_000)));
            let bytes = serde_json::to_vec(&preference)
                .map_err(|error| ApiFailure::internal(error.to_string()))?;
            write_atomic(&path, &bytes).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        Ok(())
    }

    fn record_rate_limit_attempts(
        &self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: u32,
    ) -> Result<bool, ApiFailure> {
        let _lock = crate::file_lock::acquire(&self.store_root.join("_rate_limits.lock"))?;
        let mut attempts_by_key = Vec::with_capacity(keys.len());
        for key in keys {
            let path = rate_limit_path(&self.store_root, key);
            let (window_start_ms, attempts) = fs::read_to_string(&path)
                .ok()
                .and_then(|saved| {
                    let mut lines = saved.lines();
                    let window_start_ms = lines.next()?.parse::<i64>().ok()?;
                    let attempts = lines.next()?.parse::<u32>().ok()?;
                    Some((window_start_ms, attempts))
                })
                .filter(|(window_start_ms, _)| now_ms.saturating_sub(*window_start_ms) < window_ms)
                .unwrap_or((now_ms, 0));
            if attempts >= max_attempts {
                return Ok(false);
            }
            attempts_by_key.push((path, window_start_ms, attempts.saturating_add(1)));
        }
        for (path, window_start_ms, attempts) in attempts_by_key {
            // `write_atomic` creates the parent directory itself.
            write_atomic(&path, format!("{window_start_ms}\n{attempts}\n").as_bytes())
                .map_err(|error| ApiFailure::internal(error.to_string()))?;
        }

        Ok(true)
    }

    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        Ok(self
            .store_root
            .join(account_id)
            .join("account.marker")
            .exists()
            || account_store_path(&self.store_root, account_id).exists())
    }

    fn enabled_return_notification_accounts(
        &self,
        limit: usize,
        now_ms: i64,
        interval_ms: i64,
    ) -> Result<Vec<String>, ApiFailure> {
        let mut account_ids = Vec::new();
        for entry in fs::read_dir(&self.store_root)
            .map_err(|error| ApiFailure::internal(error.to_string()))?
        {
            let entry = entry.map_err(|error| ApiFailure::internal(error.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| ApiFailure::internal(error.to_string()))?;
            if !file_type.is_dir() {
                continue;
            }
            let account_id = entry.file_name().to_string_lossy().into_owned();
            if account_id.starts_with('_') {
                continue;
            }
            let path = entry.path().join("return-notifications.json");
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(ApiFailure::internal(format!(
                        "failed to read return notification preference for {account_id}: {error}"
                    )))
                }
            };
            let preference = serde_json::from_slice::<ReturnNotificationPreference>(&bytes)
                .map_err(|error| {
                    ApiFailure::internal(format!(
                        "failed to parse return notification preference for {account_id}: {error}"
                    ))
                })?;
            let pending_ready = preference.pending_delivery_key.is_some()
                && preference
                    .next_retry_at_ms
                    .is_none_or(|retry_at_ms| retry_at_ms <= now_ms);
            let cadence_ready = preference.last_sent_at_ms.is_none_or(|last_sent_at_ms| {
                last_sent_at_ms <= now_ms.saturating_sub(interval_ms)
            });
            let claim_ready = preference
                .claim_expires_at_ms
                .is_none_or(|claim_expires_at_ms| claim_expires_at_ms <= now_ms);
            if preference.enabled
                && claim_ready
                && (pending_ready || (preference.pending_delivery_key.is_none() && cadence_ready))
            {
                account_ids.push(account_id);
            }
        }
        account_ids.sort();
        account_ids.truncate(limit);
        Ok(account_ids)
    }

    fn copy_account(
        &self,
        _source_account_id: &str,
        target_account_id: &str,
        source_store_path: &FsPath,
    ) -> Result<(), ApiFailure> {
        let target_store_path = account_store_path(&self.store_root, target_account_id);
        if source_store_path != target_store_path
            && source_store_path.exists()
            && !target_store_path.exists()
        {
            if let Some(parent) = target_store_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
            }
            let source_store =
                memory_engine_persistence::BetaPersistenceStore::open(source_store_path)
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
            source_store
                .copy_for_account(target_store_path, target_account_id)
                .map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        Ok(())
    }

    fn save_source(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source: &SourceRecord,
    ) -> Result<(), ApiFailure> {
        self.with_locked_study(store_path, |study| {
            study
                .add_source(BetaStudySourceInput {
                    id: source.source_id.clone(),
                    title: source.title.clone(),
                    body: source.body.clone(),
                    project_key: source.project_key.clone(),
                    ttl_expires_at: source.ttl_expires_at,
                    permission: source.permission.clone(),
                })
                .map_err(study_failure)?;
            Ok(())
        })
    }

    fn list_sources(
        &self,
        _account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        persisted_sources(store_path)
    }

    fn update_source_permission(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        study
            .update_source_permission(source_id, permission)
            .map(drop)
            .map_err(study_failure)
    }

    fn generate_source(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        if !persisted_source_exists(store_path, source_id)? {
            return Err(ApiFailure::not_found("Source not found."));
        }
        // Provider/model work must stay outside the descriptor lock. The
        // persistence store owns the atomic commit path; holding the account
        // lock here would turn normal foreground keep or invalidation on
        // the same account into a spurious 409 for the duration of generation.
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = run_source_generation(
            &mut study,
            source_id,
            self.generation_provider_config.clone(),
        )?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn generate_source_with_run_id(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source_id: &str,
        run_id: &str,
        _generation_attempt: i32,
        _lease_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        if !persisted_source_exists(store_path, source_id)? {
            return Err(ApiFailure::not_found("Source not found."));
        }
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = run_source_generation_with_run_id(
            &mut study,
            source_id,
            run_id,
            self.generation_provider_config.clone(),
        )?;
        Ok(StudyViewResponse::from_view(view))
    }

    fn finalize_generation_run(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
        now_ms: i64,
        lease_valid: bool,
    ) -> Result<bool, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            study
                .finalize_generation_run(
                    run_id,
                    generation_attempt,
                    lease_token,
                    now_ms,
                    lease_valid,
                )
                .map_err(study_failure)
        })
    }

    fn archive_source(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        self.with_locked_study(store_path, |study| {
            if !persisted_source_exists(store_path, source_id)? {
                return Err(ApiFailure::not_found("Source not found."));
            }
            let (view, archived_count) = study.archive_source(source_id).map_err(study_failure)?;

            Ok((StudyViewResponse::from_view(view), archived_count))
        })
    }

    fn invalidate_project_deck(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        deck_id: &str,
        invalidated_at: i64,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            if !persisted_project_deck_exists(store_path, deck_id)? {
                return Err(ApiFailure::not_found("Project deck not found."));
            }
            let view = study
                .invalidate_project_deck(deck_id, invalidated_at)
                .map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn keep_draft(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            let view = study.keep_draft(draft_id).map_err(file_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn edit_pending_draft(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            let view = study
                .edit_and_keep_draft(draft_id, prompt, expected_answer)
                .map_err(file_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn reject_pending_draft(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            let view = study.reject_draft(draft_id).map_err(file_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn next_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = study.start().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn study_view(
        &self,
        _account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let study = crate::open_study_session(store_path, self.now)?;
        let view = study.view().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn reveal_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study.reveal().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn learn_more_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let provider_config = self.generation_provider_config.clone();
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = run_reference_generation(study, provider_config)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn skip_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = study.skip_current().map_err(study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn snooze_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = study.snooze_current().map_err(study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn snooze_concept_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            let view = study.start().map_err(study_failure)?;
            let Some(current) = view.current else {
                return Err(ApiFailure::not_found("Review unit not found."));
            };
            if current.review_unit_id.to_string() != review_unit_id {
                return Err(ApiFailure::not_found("Review unit not found."));
            }
            let view = study.snooze_current_concept().map_err(study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn delete_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = study.archive_current().map_err(study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn edit_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study
            .edit_current_prompt(prompt, expected_answer)
            .map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn bridge_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let provider_config = self.generation_provider_config.clone();
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = run_bridge_generation(study, provider_config)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn submit_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        request: SubmitReviewRequest,
        _timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.with_locked_study(store_path, |study| {
            require_current_review(study, review_unit_id)?;
            let view = study
                .submit_answer_with_idempotency_key(
                    request.answer,
                    request.response_time_ms,
                    Some(request.idempotency_key),
                )
                .map_err(study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn record_content_feedback(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure> {
        let mut store = crate::open_persistence_store(store_path)?;
        record_content_feedback(&mut store, command).map_err(file_content_feedback_failure)
    }

    fn content_feedback_head(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<Option<String>, ApiFailure> {
        let store = crate::open_persistence_store(store_path)?;
        Ok(current_content_feedback_head(
            &store.snapshot().content_feedback,
            account_id,
            review_unit_id,
        ))
    }
}

#[derive(Debug)]
struct PostgresStudyStorage {
    database_url: String,
    now: fn() -> i64,
    generation_provider_config: Option<memory_engine_openrouter::OpenRouterConfig>,
}

impl PostgresStudyStorage {
    fn now_ms(&self) -> i64 {
        (self.now)()
    }
}

impl StudyStorageAdapter for PostgresStudyStorage {
    fn account_store_path(&self, _account_id: &str) -> PathBuf {
        PathBuf::new()
    }

    fn save_account_session(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<(), ApiFailure> {
        with_postgres_account(
            &self.database_url,
            account_id,
            self.now_ms(),
            |mut account| {
                account
                    .save_api_session(
                        &secret_hash(session_token),
                        self.now_ms(),
                        self.now_ms().saturating_add(app_session_max_age_ms()),
                    )
                    .map_err(postgres_failure)
            },
        )
    }

    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        let session_token_hash = if is_secret_hash(session_token) {
            session_token.to_owned()
        } else {
            secret_hash(session_token)
        };
        with_postgres_store(&self.database_url, |store| {
            store
                .api_session_matches(account_id, &session_token_hash, self.now_ms())
                .map_err(postgres_failure)
        })
    }
    fn account_session_matches_with_timings(
        &self,
        account_id: &str,
        session_token: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        let session_token_hash = if is_secret_hash(session_token) {
            session_token.to_owned()
        } else {
            secret_hash(session_token)
        };
        with_postgres_store_timed(&self.database_url, timings, |store| {
            store
                .api_session_matches(account_id, &session_token_hash, self.now_ms())
                .map_err(postgres_failure)
        })
    }

    fn revoke_account_session(
        &self,
        account_id: &str,
        session_token: &str,
        now_ms: i64,
    ) -> Result<bool, ApiFailure> {
        let token_hash = secret_hash(session_token);
        with_postgres_account(&self.database_url, account_id, now_ms, |mut account| {
            account
                .revoke_api_session(&token_hash, now_ms)
                .map_err(postgres_failure)
        })
    }

    fn revoke_account_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        with_postgres_account(&self.database_url, account_id, now_ms, |mut account| {
            account
                .revoke_api_sessions(now_ms)
                .map(|_| ())
                .map_err(postgres_failure)
        })
    }

    fn save_browser_session(
        &self,
        session_id: &str,
        session: &BrowserSessionRecord,
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .save_browser_session(
                    &secret_hash(session_id),
                    &session.account_id,
                    &session.session_token,
                    &session.csrf_token_hash,
                    self.now_ms(),
                    session.expires_at_ms,
                )
                .map_err(postgres_failure)
        })
    }

    fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .browser_session(&secret_hash(session_id), self.now_ms())
                .map(|session| {
                    session.map(|session| BrowserSessionRecord {
                        account_id: session.account_id,
                        session_token: session.session_token,
                        csrf_token_hash: session.csrf_token_hash,
                        expires_at_ms: session.expires_at_ms,
                    })
                })
                .map_err(postgres_failure)
        })
    }
    fn load_browser_session_with_timings(
        &self,
        session_id: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        with_postgres_store_timed(&self.database_url, timings, |store| {
            store
                .browser_session(&secret_hash(session_id), self.now_ms())
                .map(|session| {
                    session.map(|session| BrowserSessionRecord {
                        account_id: session.account_id,
                        session_token: session.session_token,
                        csrf_token_hash: session.csrf_token_hash,
                        expires_at_ms: session.expires_at_ms,
                    })
                })
                .map_err(postgres_failure)
        })
    }

    fn revoke_browser_session(&self, session_id: &str, now_ms: i64) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .revoke_browser_session(&secret_hash(session_id), now_ms)
                .map_err(postgres_failure)
        })
    }

    fn revoke_browser_sessions_for_account(
        &self,
        account_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .revoke_browser_sessions_for_account(account_id, now_ms)
                .map_err(postgres_failure)
        })
    }

    fn save_auth_challenge(
        &self,
        challenge_hash: &str,
        email: &str,
        expires_at_ms: i64,
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .save_auth_challenge(challenge_hash, email, expires_at_ms)
                .map_err(postgres_failure)
        })
    }

    fn consume_auth_challenge(
        &self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .consume_auth_challenge(challenge_hash, now_ms)
                .map_err(postgres_failure)
        })
    }

    fn save_return_notification_preference(
        &self,
        account_id: &str,
        email: &str,
        enabled: bool,
        last_sent_at_ms: Option<i64>,
        unsubscribe_nonce: &str,
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .save_return_notification_preference(
                    account_id,
                    email,
                    enabled,
                    last_sent_at_ms,
                    self.now_ms(),
                    unsubscribe_nonce,
                )
                .map_err(postgres_failure)
        })
    }

    fn load_return_notification_preference(
        &self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .return_notification_preference(account_id)
                .map(|preference| {
                    preference.map(|preference| ReturnNotificationPreference {
                        email: preference.email,
                        enabled: preference.enabled,
                        last_sent_at_ms: preference.last_sent_at_ms,
                        unsubscribe_nonce: preference.unsubscribe_nonce,
                        claim_id: preference.claim_id,
                        claim_expires_at_ms: preference.claim_expires_at_ms,
                        pending_delivery_key: preference.pending_delivery_key,
                        pending_due_count: preference
                            .pending_due_count
                            .and_then(|value| usize::try_from(value).ok()),
                        pending_unsubscribe_expires_at_ms: preference
                            .pending_unsubscribe_expires_at_ms,
                        retry_attempts: u32::try_from(preference.retry_attempts)
                            .map_or(u32::MAX, |value| value),
                        next_retry_at_ms: preference.next_retry_at_ms,
                    })
                })
                .map_err(postgres_failure)
        })
    }

    fn disable_return_notification(
        &self,
        account_id: &str,
        email: &str,
        current_nonce: &str,
        next_nonce: &str,
        updated_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .disable_return_notification(
                    account_id,
                    email,
                    current_nonce,
                    next_nonce,
                    updated_at_ms,
                )
                .map_err(postgres_failure)
        })
    }

    fn claim_return_notification(
        &self,
        request: &ReturnNotificationClaimRequest,
    ) -> Result<Option<ReturnNotificationClaim>, ApiFailure> {
        let due_count = i64::try_from(request.due_count)
            .map_err(|_| ApiFailure::internal("due count exceeds postgres range".to_owned()))?;
        with_postgres_store(&self.database_url, |store| {
            let request = memory_engine_persistence_postgres::ReturnNotificationClaimRequest {
                account_id: request.account_id.clone(),
                now_ms: request.now_ms,
                due_count,
                force_confirmation: request.force_confirmation,
                interval_ms: request.interval_ms,
                claim_id: request.claim_id.clone(),
                delivery_key: request.delivery_key.clone(),
                claim_expires_at_ms: request.claim_expires_at_ms,
                unsubscribe_nonce: request.unsubscribe_nonce.clone(),
                unsubscribe_expires_at_ms: request.unsubscribe_expires_at_ms,
            };
            store
                .claim_return_notification(&request)
                .map(|claim| {
                    claim.map(|claim| ReturnNotificationClaim {
                        email: claim.email,
                        due_count: usize::try_from(claim.due_count).unwrap_or(usize::MAX),
                        delivery_key: claim.delivery_key,
                        unsubscribe_nonce: claim.unsubscribe_nonce,
                        unsubscribe_expires_at_ms: claim.unsubscribe_expires_at_ms,
                        claim_id: claim.claim_id,
                    })
                })
                .map_err(postgres_failure)
        })
    }

    fn complete_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        sent_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .complete_return_notification(account_id, claim_id, sent_at_ms)
                .map_err(postgres_failure)
        })
    }

    fn release_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .release_return_notification(account_id, claim_id, now_ms)
                .map_err(postgres_failure)
        })
    }

    fn enabled_return_notification_accounts(
        &self,
        limit: usize,
        now_ms: i64,
        interval_ms: i64,
    ) -> Result<Vec<String>, ApiFailure> {
        let limit = i64::try_from(limit).map_err(|_| {
            ApiFailure::internal("scheduler batch exceeds postgres range".to_owned())
        })?;
        with_postgres_store(&self.database_url, |store| {
            store
                .enabled_return_notification_accounts(limit, now_ms, interval_ms)
                .map(|accounts| {
                    accounts
                        .into_iter()
                        .map(|account| account.account_id)
                        .collect()
                })
                .map_err(postgres_failure)
        })
    }

    fn record_rate_limit_attempts(
        &self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: u32,
    ) -> Result<bool, ApiFailure> {
        let max_attempts = i32::try_from(max_attempts).map_err(|_| {
            ApiFailure::internal("rate limit max attempts exceeds postgres range".to_owned())
        })?;
        with_postgres_store(&self.database_url, |store| {
            store
                .record_rate_limit_attempts(keys, now_ms, window_ms, max_attempts)
                .map_err(postgres_failure)
        })
    }

    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store.account_exists(account_id).map_err(postgres_failure)
        })
    }
    fn account_exists_with_timings(
        &self,
        account_id: &str,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<bool, ApiFailure> {
        with_postgres_store_timed(&self.database_url, timings, |store| {
            store.account_exists(account_id).map_err(postgres_failure)
        })
    }

    fn copy_account(
        &self,
        source_account_id: &str,
        target_account_id: &str,
        _source_store_path: &FsPath,
    ) -> Result<(), ApiFailure> {
        let snapshot = with_postgres_account(
            &self.database_url,
            source_account_id,
            self.now_ms(),
            |account| account.snapshot().map_err(postgres_failure),
        )?;
        with_postgres_account(
            &self.database_url,
            target_account_id,
            self.now_ms(),
            |mut account| {
                for document in snapshot.source_documents {
                    account
                        .save_source_document(&document)
                        .map_err(postgres_failure)?;
                }
                for reference in snapshot.reference_spans {
                    account
                        .save_reference_span(&reference)
                        .map_err(postgres_failure)?;
                }
                for note in snapshot.concept_reference_notes {
                    account
                        .save_concept_reference_note(&note)
                        .map_err(postgres_failure)?;
                }
                for run in snapshot.generation_runs {
                    account
                        .save_generation_run(&run)
                        .map_err(postgres_failure)?;
                }
                for draft in snapshot.generated_prompt_drafts {
                    account
                        .save_generated_prompt_draft(&draft)
                        .map_err(postgres_failure)?;
                }
                for review_unit in snapshot.review_units {
                    account
                        .save_review_unit(&review_unit)
                        .map_err(postgres_failure)?;
                }
                let feedback = order_content_feedback_for_copy(snapshot.content_feedback)
                    .map_err(ApiFailure::internal)?;
                for mut feedback in feedback {
                    target_account_id.clone_into(&mut feedback.account_id);
                    account
                        .record_content_feedback(&feedback)
                        .map_err(postgres_failure)?;
                }
                for schedule in snapshot.schedules {
                    account
                        .set_schedule_state(
                            &schedule.review_unit_id,
                            Some(&schedule.state),
                            self.now_ms(),
                        )
                        .map_err(postgres_failure)?;
                }
                Ok(())
            },
        )
    }

    fn save_source(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        source: &SourceRecord,
    ) -> Result<(), ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            study
                .add_source(BetaStudySourceInput {
                    id: source.source_id.clone(),
                    title: source.title.clone(),
                    body: source.body.clone(),
                    project_key: source.project_key.clone(),
                    ttl_expires_at: source.ttl_expires_at,
                    permission: source.permission.clone(),
                })
                .map(drop)
                .map_err(study_failure)
        })
    }

    fn list_sources(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        self.list_sources_with_timings(account_id, store_path, None)
    }

    fn list_sources_with_timings(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        with_postgres_account_timed(
            &self.database_url,
            account_id,
            self.now_ms(),
            timings,
            |account| {
                Ok(account
                    .snapshot()
                    .map_err(postgres_failure)?
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
            },
        )
    }

    fn update_source_permission(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        source_id: &str,
        permission: SourcePermission,
    ) -> Result<(), ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            study
                .update_source_permission(source_id, permission)
                .map(drop)
                .map_err(study_failure)
        })
    }

    fn generate_source(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            if !account
                .snapshot()
                .map_err(postgres_failure)?
                .source_documents
                .iter()
                .any(|source| source.id == source_id && source.archived_at.is_none())
            {
                return Err(ApiFailure::not_found("Source not found."));
            }
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = run_source_generation(
                &mut study,
                source_id,
                self.generation_provider_config.clone(),
            )?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn generate_source_with_run_id(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        source_id: &str,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(
            &self.database_url,
            account_id,
            self.now_ms(),
            |mut account| {
                account.set_generation_lease_fence(run_id, generation_attempt, lease_token);
                if !account
                    .snapshot()
                    .map_err(postgres_failure)?
                    .source_documents
                    .iter()
                    .any(|source| source.id == source_id && source.archived_at.is_none())
                {
                    return Err(ApiFailure::not_found("Source not found."));
                }
                let mut study = BetaStudySession::from_store(account, self.now);
                let view = run_source_generation_with_run_id(
                    &mut study,
                    source_id,
                    run_id,
                    self.generation_provider_config.clone(),
                )?;
                Ok(StudyViewResponse::from_view(view))
            },
        )
    }

    fn finalize_generation_run(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
        now_ms: i64,
        lease_valid: bool,
    ) -> Result<bool, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, now_ms, |mut account| {
            account
                .finalize_generation_run(
                    run_id,
                    generation_attempt,
                    lease_token,
                    now_ms,
                    lease_valid,
                )
                .map_err(postgres_failure)
        })
    }

    fn archive_source(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            if !account
                .snapshot()
                .map_err(postgres_failure)?
                .source_documents
                .iter()
                .any(|source| source.id == source_id && source.archived_at.is_none())
            {
                return Err(ApiFailure::not_found("Source not found."));
            }
            let mut study = BetaStudySession::from_store(account, self.now);
            let (view, archived_count) = study.archive_source(source_id).map_err(study_failure)?;

            Ok((StudyViewResponse::from_view(view), archived_count))
        })
    }

    fn invalidate_project_deck(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        deck_id: &str,
        invalidated_at: i64,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            if !account
                .snapshot()
                .map_err(postgres_failure)?
                .source_documents
                .iter()
                .any(|source| {
                    source.id == deck_id
                        && source.archived_at.is_none()
                        && source.project_key.is_some()
                })
            {
                return Err(ApiFailure::not_found("Project deck not found."));
            }
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = study
                .invalidate_project_deck(deck_id, invalidated_at)
                .map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn keep_draft(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = study.keep_draft(draft_id).map_err(postgres_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn edit_pending_draft(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        draft_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = study
                .edit_and_keep_draft(draft_id, prompt, expected_answer)
                .map_err(postgres_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn reject_pending_draft(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = study
                .reject_draft(draft_id)
                .map_err(postgres_study_failure)?;
            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn next_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            let view = study.start().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn study_view(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.study_view_with_timings(account_id, store_path, None)
    }

    fn study_view_with_timings(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account_timed(
            &self.database_url,
            account_id,
            self.now_ms(),
            timings,
            |account| {
                let study = BetaStudySession::from_store(account, self.now);
                let view = study.view().map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            },
        )
    }

    fn reveal_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = study.reveal().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn learn_more_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = run_reference_generation(study, self.generation_provider_config.clone())?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn skip_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = study.skip_current().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn snooze_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = study.snooze_current().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn snooze_concept_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(
            &self.database_url,
            account_id,
            self.now_ms(),
            |mut account| {
                account
                    .snooze_current_review_unit_concept_until(
                        review_unit_id,
                        self.now_ms(),
                        self.now_ms() + memory_engine_study::DEFAULT_SNOOZE_DEFER_MS,
                    )
                    .map_err(|error| {
                        match error {
                    memory_engine_persistence_postgres::PostgresStoreError::NoConceptKey => {
                        ApiFailure::bad_request(
                            "The active review unit must have a nonblank concept key.",
                        )
                    }
                    memory_engine_persistence_postgres::PostgresStoreError::UnknownReviewUnit(
                        _,
                    ) => ApiFailure::not_found("Review unit not found."),
                    error => postgres_failure(error),
                }
                    })?;
                let mut study = BetaStudySession::from_store(account, self.now);
                let view = study.start().map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            },
        )
    }

    fn delete_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = study.archive_current().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn edit_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
        prompt: &str,
        expected_answer: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = study
                .edit_current_prompt(prompt, expected_answer)
                .map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn bridge_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = run_bridge_generation(study, self.generation_provider_config.clone())?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn submit_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
        request: SubmitReviewRequest,
        timings: Option<&mut SubmitReviewTimings>,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account_timed(
            &self.database_url,
            account_id,
            self.now_ms(),
            timings,
            |account| {
                if account
                    .applied_review_idempotency_key_exists(&request.idempotency_key)
                    .map_err(postgres_failure)?
                {
                    let study = BetaStudySession::from_store(account, self.now);
                    let view = study.view().map_err(study_failure)?;

                    return Ok(StudyViewResponse::from_view(view));
                }

                let mut study = BetaStudySession::from_store(account, self.now);
                require_current_review_postgres(&mut study, review_unit_id)?;
                let view = study
                    .submit_answer_with_idempotency_key(
                        request.answer,
                        request.response_time_ms,
                        Some(request.idempotency_key),
                    )
                    .map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            },
        )
    }

    fn record_content_feedback(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure> {
        with_postgres_account(
            &self.database_url,
            account_id,
            self.now_ms(),
            |mut account| {
                record_content_feedback(&mut account, command)
                    .map_err(postgres_content_feedback_failure)
            },
        )
    }

    fn content_feedback_head(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<Option<String>, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            let snapshot = account.snapshot().map_err(postgres_failure)?;
            Ok(current_content_feedback_head(
                &snapshot.content_feedback,
                account_id,
                review_unit_id,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory_engine_generation::{
        DraftProvider, ProviderDrafts, ProviderFailure, StructuredBlockProvider,
    };
    use memory_engine_persistence::GeneratedPromptModel;
    use memory_engine_service::MemoryServiceStore;
    use std::{
        sync::{
            atomic::{AtomicI64, Ordering},
            mpsc, Arc, Barrier,
        },
        thread,
        time::Duration,
    };

    fn test_now() -> i64 {
        1_700_000_000_000
    }

    static NOTIFICATION_CLOCK: AtomicI64 = AtomicI64::new(1_700_000_000_000);

    fn notification_now() -> i64 {
        NOTIFICATION_CLOCK.load(Ordering::SeqCst)
    }

    struct BlockingDraftProvider {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
        delegate: StructuredBlockProvider,
    }

    impl DraftProvider for BlockingDraftProvider {
        fn model(&self) -> GeneratedPromptModel {
            self.delegate.model()
        }

        fn generate_drafts(
            &self,
            source: &memory_engine_persistence::SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            self.entered.wait();
            self.release.wait();
            self.delegate.generate_drafts(source)
        }
    }

    #[test]
    fn legacy_account_session_migration_hashes_and_removes_raw_token() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-legacy-session-migration-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let account_id = "acct_legacy_file";
        let legacy_path = root.join(account_id).join("session.token");
        fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("legacy parent");
        fs::write(&legacy_path, "legacy-wire-token\n").expect("legacy token");
        let storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        storage
            .migrate_legacy_account_session(account_id, test_now())
            .expect("migrate legacy account session");
        assert!(
            !legacy_path.exists(),
            "raw token must be removed after verified rewrite"
        );
        let hash_path = root
            .join("_api_sessions")
            .join(secret_hash("legacy-wire-token"))
            .join("session");
        let persisted = fs::read_to_string(hash_path).expect("hashed session");
        assert!(persisted.starts_with(account_id));
        assert!(!persisted.contains("legacy-wire-token"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_notification_claim_waits_and_rebases_ttls_after_lock_contention() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-notification-lock-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        NOTIFICATION_CLOCK.store(test_now(), Ordering::SeqCst);
        let storage = FileStudyStorage {
            store_root: root.clone(),
            now: notification_now,
            generation_provider_config: None,
        };
        storage
            .save_return_notification_preference(
                "acct",
                "learner@example.com",
                true,
                None,
                "unsubscribe-nonce",
            )
            .expect("notification preference");
        let held = crate::file_lock::acquire(&root.join("acct").join("return-notifications.lock"))
            .expect("hold notification lock");
        let request = ReturnNotificationClaimRequest {
            account_id: "acct".to_owned(),
            now_ms: test_now(),
            due_count: 1,
            force_confirmation: true,
            interval_ms: 86_400_000,
            claim_id: "claim".to_owned(),
            delivery_key: "delivery".to_owned(),
            claim_expires_at_ms: test_now() + 1,
            unsubscribe_nonce: "next-unsubscribe-nonce".to_owned(),
            unsubscribe_expires_at_ms: test_now() + 604_800_000,
        };
        let contender = FileStudyStorage {
            store_root: root.clone(),
            now: notification_now,
            generation_provider_config: None,
        };
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let claim = thread::spawn(move || {
            started_tx.send(()).expect("signal claim start");
            result_tx
                .send(contender.claim_return_notification(&request))
                .expect("send claim result");
        });
        started_rx.recv().expect("claim started");
        let early = result_rx.recv_timeout(Duration::from_millis(100));
        assert!(
            early.is_err(),
            "claim must wait while the notification lock is held"
        );
        NOTIFICATION_CLOCK.store(test_now() + 10, Ordering::SeqCst);
        drop(held);
        let result = early.unwrap_or_else(|_| result_rx.recv().expect("claim after lock release"));

        assert!(
            result.expect("claim result").is_some(),
            "short lock contention must not be reported as an ineligible notification"
        );
        claim.join().expect("claim thread");
        let preference = storage
            .load_return_notification_preference("acct")
            .expect("load notification preference")
            .expect("saved notification preference");
        assert_eq!(
            preference.claim_expires_at_ms,
            Some(test_now() + 11),
            "claim TTL must start after the blocking lock wait"
        );
        assert_eq!(
            preference.pending_unsubscribe_expires_at_ms,
            Some(test_now() + 10 + 604_800_000),
            "unsubscribe TTL must start after the blocking lock wait"
        );
        assert!(
            storage
                .claim_return_notification(&ReturnNotificationClaimRequest {
                    account_id: "acct".to_owned(),
                    now_ms: test_now() + 10,
                    due_count: 1,
                    force_confirmation: true,
                    interval_ms: 86_400_000,
                    claim_id: "premature-reclaim".to_owned(),
                    delivery_key: "premature-delivery".to_owned(),
                    claim_expires_at_ms: test_now() + 11,
                    unsubscribe_nonce: "premature-nonce".to_owned(),
                    unsubscribe_expires_at_ms: test_now() + 10 + 604_800_000,
                })
                .expect("premature reclaim check")
                .is_none(),
            "lock wait beyond the original TTL must not expose the fresh claim"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_notification_completion_waits_and_samples_time_after_lock_contention() {
        static COMPLETION_CLOCK: AtomicI64 = AtomicI64::new(1_700_000_000_000);
        fn completion_now() -> i64 {
            COMPLETION_CLOCK.load(Ordering::SeqCst)
        }

        let root = std::env::temp_dir().join(format!(
            "memory-engine-notification-completion-lock-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        COMPLETION_CLOCK.store(test_now(), Ordering::SeqCst);
        let storage = FileStudyStorage {
            store_root: root.clone(),
            now: completion_now,
            generation_provider_config: None,
        };
        storage
            .save_return_notification_preference(
                "acct",
                "learner@example.com",
                true,
                None,
                "unsubscribe-nonce",
            )
            .expect("notification preference");
        storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "acct".to_owned(),
                now_ms: test_now(),
                due_count: 1,
                force_confirmation: true,
                interval_ms: 86_400_000,
                claim_id: "claim".to_owned(),
                delivery_key: "delivery".to_owned(),
                claim_expires_at_ms: test_now() + 300_000,
                unsubscribe_nonce: "next-unsubscribe-nonce".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_000,
            })
            .expect("claim result")
            .expect("claim");

        let held = crate::file_lock::acquire(&root.join("acct").join("return-notifications.lock"))
            .expect("hold notification lock before completion");
        let contender = FileStudyStorage {
            store_root: root.clone(),
            now: completion_now,
            generation_provider_config: None,
        };
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let completion = thread::spawn(move || {
            started_tx.send(()).expect("signal completion start");
            result_tx
                .send(contender.complete_return_notification("acct", "claim", test_now()))
                .expect("send completion result");
        });
        started_rx.recv().expect("completion started");
        let early = result_rx.recv_timeout(Duration::from_millis(100));
        assert!(
            early.is_err(),
            "completion must wait while the notification lock is held"
        );
        COMPLETION_CLOCK.store(test_now() + 10, Ordering::SeqCst);
        drop(held);
        let result =
            early.unwrap_or_else(|_| result_rx.recv().expect("completion after lock release"));

        assert!(
            result.expect("completion result"),
            "short lock contention must not discard the matching completion"
        );
        completion.join().expect("completion thread");
        assert_eq!(
            storage
                .load_return_notification_preference("acct")
                .expect("load completed notification")
                .expect("completed notification")
                .last_sent_at_ms,
            Some(test_now() + 10),
            "completion timestamp must be sampled after the blocking lock wait"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_generation_releases_descriptor_before_provider_and_foreground_commit() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-generation-foreground-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let store_path = root.join("acct").join("study.json");
        let storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        storage
            .save_source(
                "acct",
                &store_path,
                &crate::SourceRecord {
                    source_id: "source".to_owned(),
                    title: "NATO notes".to_owned(),
                    body: "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.".to_owned(),
                    project_key: None,
                    ttl_expires_at: None,
                    permission: SourcePermission::ModelEligible,
                },
            )
            .expect("save source");
        let first = storage
            .generate_source("acct", &store_path, "source")
            .expect("seed draft");
        let draft_id = first.drafts.first().expect("generated draft").id.clone();

        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let provider = BlockingDraftProvider {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            delegate: StructuredBlockProvider,
        };
        let generation_storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        let generation_store_path = store_path.clone();
        let generation = thread::spawn(move || {
            generation_storage.generate_source_with_provider(
                &generation_store_path,
                "source",
                &provider,
            )
        });

        entered.wait();
        // This is the foreground operation that used to receive a spurious
        // 409 while generation held the descriptor across provider work.
        storage
            .keep_draft("acct", &root.join("acct").join("study.json"), &draft_id)
            .expect("foreground keep must commit while provider is running");
        storage
            .save_source(
                "acct",
                &store_path,
                &crate::SourceRecord {
                    source_id: "source-2".to_owned(),
                    title: "Second source".to_owned(),
                    body: "Concept: Unrelated\nActivity: quiz\nStage: recognition-1\nQuestion: What is unrelated?\nAnswer: UNRELATED".to_owned(),
                    project_key: None,
                    ttl_expires_at: None,
                    permission: SourcePermission::ModelEligible,
                },
            )
            .expect("foreground source mutation must commit while provider is running");
        release.wait();
        generation
            .join()
            .expect("generation thread")
            .expect("generation after foreground commit");

        let reloaded = memory_engine_persistence::BetaPersistenceStore::open(&store_path)
            .expect("reload persisted store");
        let snapshot = reloaded.snapshot();
        assert_eq!(snapshot.source_documents.len(), 2);
        assert!(
            snapshot.generation_runs.len() >= 2,
            "foreground and background generation runs must both persist"
        );
        assert!(snapshot
            .generated_prompt_drafts
            .iter()
            .any(|draft| draft.id == draft_id));
        assert!(
            snapshot.review_units.iter().any(|unit| {
                unit.generated_prompt_draft_id.as_deref() == Some(draft_id.as_str())
            }),
            "foreground keep must survive the later generation commit"
        );
        assert!(!reloaded.list_queue_candidates().expect("queue").is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn file_generation_keeps_foreground_commit_responsive_while_provider_waits() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-generation-responsive-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let store_path = root.join("acct").join("study.json");
        let storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        storage
            .save_source(
                "acct",
                &store_path,
                &crate::SourceRecord {
                    source_id: "source".to_owned(),
                    title: "NATO notes".to_owned(),
                    body: "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.".to_owned(),
                    project_key: None,
                    ttl_expires_at: None,
                    permission: SourcePermission::ModelEligible,
                },
            )
            .expect("save source");
        let first = storage
            .generate_source("acct", &store_path, "source")
            .expect("seed draft");
        let draft_id = first.drafts.first().expect("generated draft").id.clone();

        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let provider = BlockingDraftProvider {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            delegate: StructuredBlockProvider,
        };
        let generation_storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        let generation_store_path = store_path.clone();
        let generation = thread::spawn(move || {
            generation_storage.generate_source_with_provider(
                &generation_store_path,
                "source",
                &provider,
            )
        });

        entered.wait();
        let approval_storage = FileStudyStorage {
            store_root: root.clone(),
            now: test_now,
            generation_provider_config: None,
        };
        let approval_store_path = store_path.clone();
        let approval = tokio::task::spawn_blocking(move || {
            approval_storage.keep_draft("acct", &approval_store_path, &draft_id)
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), approval)
            .await
            .expect("foreground keep must not wait for provider to release")
            .expect("foreground keep task")
            .expect("foreground keep");
        release.wait();
        generation
            .join()
            .expect("generation thread")
            .expect("generation after foreground commit");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_return_notification_lock_is_nonblocking_and_descriptor_owned() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-lock-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        fs::create_dir_all(&root).expect("lock directory");
        let path = root.join("return-notifications.lock");
        fs::write(&path, b"existing-owner-marker").expect("existing lock path");

        let first = crate::file_lock::try_acquire(&path)
            .expect("first lock attempt")
            .expect("first owner");
        let started = std::time::Instant::now();
        assert!(
            crate::file_lock::try_acquire(&path)
                .expect("contended lock attempt")
                .is_none(),
            "a contended owner must not be acquired"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "contended lock acquisition must not sleep"
        );
        drop(first);

        assert!(path.exists(), "dropping an owner must not unlink the path");
        assert_eq!(
            fs::read(&path).expect("lock marker"),
            b"existing-owner-marker"
        );
        let second = crate::file_lock::try_acquire(&path)
            .expect("second lock attempt")
            .expect("ownership after first drop");
        drop(second);
        assert!(path.exists(), "the shared lock path remains durable");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_scheduler_filters_future_retries_before_batch_limit() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-fairness-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let storage = StudyStorage::file(&root, test_now);
        storage
            .save_return_notification_preference(
                "account-a-future-retry",
                "future@example.com",
                true,
                None,
                "future-nonce",
            )
            .expect("future preference");
        storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-a-future-retry".to_owned(),
                now_ms: test_now(),
                due_count: 1,
                force_confirmation: true,
                interval_ms: 86_400_000,
                claim_id: "future-claim".to_owned(),
                delivery_key: "future-delivery".to_owned(),
                claim_expires_at_ms: test_now() + 100,
                unsubscribe_nonce: "future-nonce".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_000,
            })
            .expect("future claim")
            .expect("future claim available");
        storage
            .release_return_notification("account-a-future-retry", "future-claim", test_now())
            .expect("future retry release");
        storage
            .save_return_notification_preference(
                "account-b-ready",
                "ready@example.com",
                true,
                None,
                "ready-nonce",
            )
            .expect("ready preference");
        storage
            .save_return_notification_preference(
                "account-c-active-claim",
                "active@example.com",
                true,
                None,
                "active-nonce",
            )
            .expect("active preference");
        storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-c-active-claim".to_owned(),
                now_ms: test_now(),
                due_count: 1,
                force_confirmation: true,
                interval_ms: 86_400_000,
                claim_id: "active-claim".to_owned(),
                delivery_key: "active-delivery".to_owned(),
                claim_expires_at_ms: test_now() + 60_000,
                unsubscribe_nonce: "active-nonce".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_000,
            })
            .expect("active claim")
            .expect("active claim available");

        assert_eq!(
            storage
                .enabled_return_notification_accounts(1, test_now(), 86_400_000)
                .expect("eligible accounts"),
            vec!["account-b-ready"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_scheduler_surfaces_malformed_notification_preferences() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-malformed-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let account_dir = root.join("account-malformed");
        fs::create_dir_all(&account_dir).expect("account directory");
        fs::write(account_dir.join("return-notifications.json"), b"not-json")
            .expect("malformed preference");
        let storage = StudyStorage::file(&root, test_now);
        let error = storage
            .enabled_return_notification_accounts(10, test_now(), 86_400_000)
            .expect_err("malformed preference must fail enumeration");
        assert!(error.message.contains("account-malformed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_retry_backoff_reaches_and_holds_at_six_hours() {
        static BACKOFF_CLOCK: AtomicI64 = AtomicI64::new(1_700_000_000_000);
        fn backoff_now() -> i64 {
            BACKOFF_CLOCK.load(Ordering::SeqCst)
        }

        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-backoff-cap-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        BACKOFF_CLOCK.store(test_now(), Ordering::SeqCst);
        let storage = StudyStorage::file(&root, backoff_now);
        storage
            .save_return_notification_preference(
                "account-backoff-cap",
                "backoff@example.com",
                true,
                None,
                "backoff-nonce",
            )
            .expect("backoff preference");
        let mut now_ms = test_now();
        for (attempt, expected_minutes) in [1_i64, 2, 4, 8, 16, 32, 64, 128, 256, 360, 360]
            .into_iter()
            .enumerate()
        {
            BACKOFF_CLOCK.store(now_ms, Ordering::SeqCst);
            let claim_id = format!("backoff-claim-{attempt}");
            storage
                .claim_return_notification(&ReturnNotificationClaimRequest {
                    account_id: "account-backoff-cap".to_owned(),
                    now_ms,
                    due_count: 1,
                    force_confirmation: true,
                    interval_ms: 86_400_000,
                    claim_id: claim_id.clone(),
                    delivery_key: "backoff-delivery".to_owned(),
                    claim_expires_at_ms: now_ms + 100,
                    unsubscribe_nonce: "backoff-nonce".to_owned(),
                    unsubscribe_expires_at_ms: now_ms + 604_800_000,
                })
                .expect("backoff claim")
                .expect("backoff claim available");
            storage
                .release_return_notification("account-backoff-cap", &claim_id, now_ms)
                .expect("backoff release");
            let preference = storage
                .load_return_notification_preference("account-backoff-cap")
                .expect("backoff load")
                .expect("backoff preference");
            assert_eq!(
                preference.next_retry_at_ms,
                Some(now_ms + expected_minutes * 60_000)
            );
            now_ms = preference.next_retry_at_ms.expect("retry timestamp");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_unsubscribe_race_cannot_leave_stale_token_disabled_after_reenable() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-race-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let storage = StudyStorage::file(&root, test_now);
        storage
            .save_return_notification_preference(
                "account-a",
                "a@example.com",
                true,
                None,
                "nonce-before",
            )
            .expect("initial preference");

        let barrier = Arc::new(Barrier::new(2));
        let stale_storage = storage.clone();
        let stale_barrier = Arc::clone(&barrier);
        let stale = thread::spawn(move || {
            stale_barrier.wait();
            stale_storage.disable_return_notification(
                "account-a",
                "a@example.com",
                "nonce-before",
                "nonce-stale",
                test_now(),
            )
        });
        let reenable_storage = storage.clone();
        let reenable_barrier = Arc::clone(&barrier);
        let reenable = thread::spawn(move || {
            reenable_barrier.wait();
            reenable_storage.save_return_notification_preference(
                "account-a",
                "a@example.com",
                true,
                None,
                "nonce-reenabled",
            )
        });

        let stale_result = stale.join().expect("stale worker");
        reenable
            .join()
            .expect("reenable worker")
            .expect("reenable after contention");
        let preference = storage
            .load_return_notification_preference("account-a")
            .expect("load preference")
            .expect("preference");
        assert!(preference.enabled, "stale result: {stale_result:?}");
        assert_eq!(preference.unsubscribe_nonce, "nonce-reenabled");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_same_enabled_email_preserves_retry_envelope_but_policy_changes_clear_it() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-save-policy-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let storage = StudyStorage::file(&root, test_now);
        storage
            .save_return_notification_preference(
                "account-policy",
                "same@example.com",
                true,
                None,
                "nonce-same",
            )
            .expect("initial preference");
        let first = storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-policy".to_owned(),
                now_ms: test_now(),
                due_count: 2,
                force_confirmation: true,
                interval_ms: 86_400_000,
                claim_id: "claim-policy".to_owned(),
                delivery_key: "envelope-policy".to_owned(),
                claim_expires_at_ms: test_now() + 100,
                unsubscribe_nonce: "nonce-request".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_000,
            })
            .expect("claim policy envelope")
            .expect("claim available");
        storage
            .save_return_notification_preference(
                "account-policy",
                "same@example.com",
                true,
                None,
                "nonce-new",
            )
            .expect("same enabled save");
        let preserved = storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-policy".to_owned(),
                now_ms: test_now() + 200,
                due_count: 1,
                force_confirmation: false,
                interval_ms: 86_400_000,
                claim_id: "claim-policy-retry".to_owned(),
                delivery_key: "envelope-new".to_owned(),
                claim_expires_at_ms: test_now() + 300,
                unsubscribe_nonce: "nonce-request-2".to_owned(),
                unsubscribe_expires_at_ms: test_now() + 604_800_200,
            })
            .expect("retry claim")
            .expect("retry available");
        assert_eq!(preserved.delivery_key, first.delivery_key);
        assert_eq!(preserved.unsubscribe_nonce, first.unsubscribe_nonce);
        assert_eq!(
            preserved.unsubscribe_expires_at_ms,
            first.unsubscribe_expires_at_ms
        );
        storage
            .save_return_notification_preference(
                "account-policy",
                "same@example.com",
                false,
                None,
                "nonce-disabled",
            )
            .expect("disable");
        storage
            .save_return_notification_preference(
                "account-policy",
                "changed@example.com",
                true,
                None,
                "nonce-changed",
            )
            .expect("email change and re-enable");
        let changed = storage
            .load_return_notification_preference("account-policy")
            .expect("load changed")
            .expect("changed preference");
        assert!(changed.claim_id.is_none());
        assert!(changed.pending_delivery_key.is_none());
        assert!(changed.pending_unsubscribe_expires_at_ms.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_claim_backfills_unsubscribe_expiry_for_old_schema_rows_and_fences_stale_claims() {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-return-old-schema-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let account_dir = root.join("account-old");
        fs::create_dir_all(&account_dir).expect("account directory");
        fs::write(
            account_dir.join("return-notifications.json"),
            r#"{"email":"old@example.com","enabled":true,"lastSentAtMs":null,"unsubscribeNonce":"nonce-old","pendingDeliveryKey":"delivery-old","pendingDueCount":2}"#,
        )
        .expect("old schema preference");
        let storage = StudyStorage::file(&root, test_now);
        let first_expiry = test_now() + 604_800_000;
        let first = storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-old".to_owned(),
                now_ms: test_now(),
                due_count: 9,
                force_confirmation: false,
                interval_ms: 86_400_000,
                claim_id: "claim-old".to_owned(),
                delivery_key: "delivery-new".to_owned(),
                claim_expires_at_ms: test_now() + 100,
                unsubscribe_nonce: "nonce-request".to_owned(),
                unsubscribe_expires_at_ms: first_expiry,
            })
            .expect("old schema claim")
            .expect("claim available");
        assert_eq!(first.delivery_key, "delivery-old");
        assert_eq!(first.unsubscribe_expires_at_ms, first_expiry);
        let second = storage
            .claim_return_notification(&ReturnNotificationClaimRequest {
                account_id: "account-old".to_owned(),
                now_ms: test_now() + 200,
                due_count: 1,
                force_confirmation: false,
                interval_ms: 86_400_000,
                claim_id: "claim-stale-retry".to_owned(),
                delivery_key: "delivery-retry".to_owned(),
                claim_expires_at_ms: test_now() + 300,
                unsubscribe_nonce: "nonce-request-2".to_owned(),
                unsubscribe_expires_at_ms: first_expiry + 200,
            })
            .expect("stale retry claim")
            .expect("retry available");
        assert_eq!(second.delivery_key, first.delivery_key);
        assert_eq!(second.unsubscribe_expires_at_ms, first_expiry);
        assert!(!storage
            .complete_return_notification("account-old", "claim-old", test_now() + 201)
            .expect("stale completion"));
        assert!(storage
            .complete_return_notification("account-old", "claim-stale-retry", test_now() + 202,)
            .expect("retry completion"));
        let _ = fs::remove_dir_all(root);
    }
}
