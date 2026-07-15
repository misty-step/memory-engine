use std::{
    fmt, fs, io,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use memory_engine_service::{record_content_feedback, RecordContentFeedbackCommand};
use memory_engine_study::{BetaStudySession, BetaStudySourceInput};

use crate::{
    account_session_path, account_store_path, auth_challenge_consumed_path, auth_challenge_path,
    browser_session_path, persisted_project_deck_exists, persisted_source_exists,
    persisted_sources, postgres_failure, rate_limit_path, require_current_review,
    require_current_review_postgres, run_bridge_generation, run_reference_generation,
    run_source_generation, secret_hash, study_failure, with_postgres_account, with_postgres_store,
    with_postgres_study, write_atomic, ApiFailure, BrowserSessionRecord, ReturnNotificationClaim,
    ReturnNotificationClaimRequest, ReturnNotificationPreference, SourceRecord, StudyViewResponse,
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

    pub(crate) fn storage(&self, now: fn() -> i64) -> StudyStorage {
        match self {
            Self::File { store_root } => StudyStorage::new(FileStudyStorage {
                store_root: store_root.clone(),
                now,
            }),
            Self::Postgres { database_url } => StudyStorage::new(PostgresStudyStorage {
                database_url: database_url.clone(),
                now,
            }),
        }
    }
}

#[derive(Clone)]
pub struct StudyStorage {
    inner: Arc<dyn StudyStorageAdapter>,
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

    pub(crate) fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        self.inner
            .account_session_matches(account_id, session_token)
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

    pub(crate) fn revoke_browser_session(
        &self,
        session_id: &str,
        now_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner.revoke_browser_session(session_id, now_ms)
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
    ) -> Result<(), ApiFailure> {
        self.inner.release_return_notification(account_id, claim_id)
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
        self.inner.list_sources(account_id, store_path)
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

    pub(crate) fn approve_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.approve_draft(account_id, store_path, draft_id)
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

    pub(crate) fn delete_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner
            .delete_review(account_id, store_path, review_unit_id)
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
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure> {
        self.inner.submit_review(
            account_id,
            store_path,
            review_unit_id,
            answer,
            response_time_ms,
            idempotency_key,
        )
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
}

struct FileReturnNotificationLock {
    path: PathBuf,
}

const RETURN_NOTIFICATION_LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

impl FileReturnNotificationLock {
    fn acquire(path: PathBuf) -> Result<Self, ApiFailure> {
        for _ in 0..5_000 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > RETURN_NOTIFICATION_LOCK_STALE_AFTER)
                    {
                        let _ = fs::remove_file(&path);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(ApiFailure::internal(error.to_string())),
            }
        }
        Err(ApiFailure::internal(
            "timed out acquiring return notification claim lock".to_owned(),
        ))
    }
}

impl Drop for FileReturnNotificationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
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
    fn save_browser_session(
        &self,
        session_id: &str,
        session: &BrowserSessionRecord,
    ) -> Result<(), ApiFailure>;
    fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure>;
    fn revoke_browser_session(&self, session_id: &str, now_ms: i64) -> Result<(), ApiFailure>;
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
    ) -> Result<(), ApiFailure>;
    fn record_rate_limit_attempts(
        &self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: u32,
    ) -> Result<bool, ApiFailure>;
    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure>;
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
    fn generate_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure>;
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
    fn approve_draft(
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
    fn delete_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
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
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure>;
    fn record_content_feedback(
        &self,
        account_id: &str,
        store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure>;
}

#[derive(Debug)]
struct FileStudyStorage {
    store_root: PathBuf,
    now: fn() -> i64,
}

impl FileStudyStorage {
    fn now_ms(&self) -> i64 {
        (self.now)()
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
        let path = account_session_path(&self.store_root, account_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
        }
        fs::write(path, session_token).map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        let path = account_session_path(&self.store_root, account_id);
        let Ok(saved) = fs::read_to_string(path) else {
            return Ok(false);
        };

        Ok(saved.trim() == session_token)
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
                "{}\n{}\n{}\n{}\n",
                session.account_id,
                session.session_token,
                session.csrf_token_hash,
                session.expires_at_ms
            ),
        )
        .map_err(|error| ApiFailure::internal(error.to_string()))
    }

    fn load_browser_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserSessionRecord>, ApiFailure> {
        let path = browser_session_path(&self.store_root, session_id);
        let Ok(saved) = fs::read_to_string(path) else {
            return Ok(None);
        };
        let mut lines = saved.lines();
        let Some(account_id) = lines.next() else {
            return Ok(None);
        };
        let Some(session_token) = lines.next() else {
            return Ok(None);
        };
        let Some(csrf_token_hash) = lines.next() else {
            return Ok(None);
        };
        let Some(expires_at_ms) = lines.next().and_then(|value| value.parse::<i64>().ok()) else {
            return Ok(None);
        };
        if expires_at_ms <= self.now_ms() {
            return Ok(None);
        }

        Ok(Some(BrowserSessionRecord {
            account_id: account_id.to_owned(),
            session_token: session_token.to_owned(),
            csrf_token_hash: csrf_token_hash.to_owned(),
            expires_at_ms,
        }))
    }

    fn revoke_browser_session(&self, session_id: &str, _now_ms: i64) -> Result<(), ApiFailure> {
        let path = browser_session_path(&self.store_root, session_id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ApiFailure::internal(error.to_string())),
        }
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
            FileReturnNotificationLock::acquire(account_dir.join("return-notifications.lock"))?;
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
            FileReturnNotificationLock::acquire(account_dir.join("return-notifications.lock"))?;
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
            FileReturnNotificationLock::acquire(account_dir.join("return-notifications.lock"))?;
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(None);
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let interval_elapsed = preference
            .last_sent_at_ms
            .is_none_or(|sent| request.now_ms.saturating_sub(sent) >= request.interval_ms);
        let eligible = preference.pending_delivery_key.is_some()
            || (interval_elapsed && (request.force_confirmation || request.due_count > 0));
        if !preference.enabled
            || !eligible
            || preference
                .claim_expires_at_ms
                .is_some_and(|expires| expires > request.now_ms)
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
            .unwrap_or(request.unsubscribe_expires_at_ms);
        preference.claim_id = Some(request.claim_id.clone());
        preference.claim_expires_at_ms = Some(request.claim_expires_at_ms);
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
        sent_at_ms: i64,
    ) -> Result<bool, ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            FileReturnNotificationLock::acquire(account_dir.join("return-notifications.lock"))?;
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
        let bytes = serde_json::to_vec(&preference)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        write_atomic(&path, &bytes).map_err(|error| ApiFailure::internal(error.to_string()))?;
        Ok(true)
    }

    fn release_return_notification(
        &self,
        account_id: &str,
        claim_id: &str,
    ) -> Result<(), ApiFailure> {
        let account_dir = self.store_root.join(account_id);
        fs::create_dir_all(&account_dir)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        let _lock =
            FileReturnNotificationLock::acquire(account_dir.join("return-notifications.lock"))?;
        let path = account_dir.join("return-notifications.json");
        let Ok(bytes) = fs::read(&path) else {
            return Ok(());
        };
        let mut preference: ReturnNotificationPreference = serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(error.to_string()))?;
        if preference.claim_id.as_deref() == Some(claim_id) {
            preference.claim_id = None;
            preference.claim_expires_at_ms = None;
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
        Ok(account_session_path(&self.store_root, account_id).exists()
            || account_store_path(&self.store_root, account_id).exists())
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
            fs::copy(source_store_path, target_store_path)
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
        let mut study = crate::open_study_session(store_path, self.now)?;
        study
            .add_source(BetaStudySourceInput {
                id: source.source_id.clone(),
                title: source.title.clone(),
                body: source.body.clone(),
                project_key: source.project_key.clone(),
                ttl_expires_at: source.ttl_expires_at,
            })
            .map_err(study_failure)?;
        Ok(())
    }

    fn list_sources(
        &self,
        _account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        persisted_sources(store_path)
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
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = run_source_generation(&mut study, source_id)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn archive_source(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<(StudyViewResponse, usize), ApiFailure> {
        if !persisted_source_exists(store_path, source_id)? {
            return Err(ApiFailure::not_found("Source not found."));
        }
        let mut study = crate::open_study_session(store_path, self.now)?;
        let (view, archived_count) = study.archive_source(source_id).map_err(study_failure)?;

        Ok((StudyViewResponse::from_view(view), archived_count))
    }

    fn invalidate_project_deck(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        deck_id: &str,
        invalidated_at: i64,
    ) -> Result<StudyViewResponse, ApiFailure> {
        if !persisted_project_deck_exists(store_path, deck_id)? {
            return Err(ApiFailure::not_found("Project deck not found."));
        }
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = study
            .invalidate_project_deck(deck_id, invalidated_at)
            .map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn approve_draft(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        let view = study.approve_draft(draft_id).map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
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
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = run_reference_generation(&mut study)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn skip_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study.skip_current().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn snooze_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study.snooze_current().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn delete_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study.archive_current().map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn bridge_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = run_bridge_generation(&mut study)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn submit_review(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let mut study = crate::open_study_session(store_path, self.now)?;
        require_current_review(&mut study, review_unit_id)?;
        let view = study
            .submit_answer_with_idempotency_key(answer, response_time_ms, Some(idempotency_key))
            .map_err(study_failure)?;

        Ok(StudyViewResponse::from_view(view))
    }

    fn record_content_feedback(
        &self,
        _account_id: &str,
        store_path: &FsPath,
        command: RecordContentFeedbackCommand,
    ) -> Result<memory_engine_service::ContentFeedback, ApiFailure> {
        let mut store = crate::open_persistence_store(store_path)?;
        record_content_feedback(&mut store, command)
            .map_err(|error| ApiFailure::internal(error.to_string()))
    }
}

#[derive(Debug)]
struct PostgresStudyStorage {
    database_url: String,
    now: fn() -> i64,
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
                    .save_api_session(session_token, self.now_ms())
                    .map_err(postgres_failure)
            },
        )
    }

    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .api_session_matches(account_id, session_token)
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

    fn revoke_browser_session(&self, session_id: &str, now_ms: i64) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .revoke_browser_session(&secret_hash(session_id), now_ms)
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
                        claim_id: None,
                        claim_expires_at_ms: None,
                        pending_delivery_key: None,
                        pending_due_count: None,
                        pending_unsubscribe_expires_at_ms: None,
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
    ) -> Result<(), ApiFailure> {
        with_postgres_store(&self.database_url, |store| {
            store
                .release_return_notification(account_id, claim_id)
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
                })
                .map(drop)
                .map_err(study_failure)
        })
    }

    fn list_sources(
        &self,
        account_id: &str,
        _store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
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
                    project_key: source.project_key,
                    ttl_expires_at: source.ttl_expires_at,
                })
                .collect())
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
            let view = run_source_generation(&mut study, source_id)?;

            Ok(StudyViewResponse::from_view(view))
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

    fn approve_draft(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            let view = study.approve_draft(draft_id).map_err(study_failure)?;

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
        _store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            let view = study.view().map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
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
            let view = run_reference_generation(study)?;

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

    fn bridge_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_study(&self.database_url, account_id, self.now, |study| {
            require_current_review_postgres(study, review_unit_id)?;
            let view = run_bridge_generation(study)?;

            Ok(StudyViewResponse::from_view(view))
        })
    }

    fn submit_review(
        &self,
        account_id: &str,
        _store_path: &FsPath,
        review_unit_id: &str,
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure> {
        with_postgres_account(&self.database_url, account_id, self.now_ms(), |account| {
            if account
                .applied_review_idempotency_key_exists(&idempotency_key)
                .map_err(postgres_failure)?
            {
                let study = BetaStudySession::from_store(account, self.now);
                let view = study.view().map_err(study_failure)?;

                return Ok(StudyViewResponse::from_view(view));
            }

            let mut study = BetaStudySession::from_store(account, self.now);
            require_current_review_postgres(&mut study, review_unit_id)?;
            let view = study
                .submit_answer_with_idempotency_key(answer, response_time_ms, Some(idempotency_key))
                .map_err(study_failure)?;

            Ok(StudyViewResponse::from_view(view))
        })
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
                    .map_err(|error| ApiFailure::internal(error.to_string()))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    fn test_now() -> i64 {
        1_700_000_000_000
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
        reenable.join().expect("reenable worker").expect("reenable");
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
