use std::{
    fmt, fs, io,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use memory_engine_study::{BetaStudySession, BetaStudySourceInput};

use crate::{
    account_session_path, account_store_path, auth_challenge_consumed_path, auth_challenge_path,
    browser_session_path, persisted_source_exists, persisted_sources, postgres_failure,
    rate_limit_path, require_current_review, require_current_review_postgres,
    run_source_generation, secret_hash, study_failure, with_postgres_account, with_postgres_store,
    with_postgres_study, write_atomic, ApiFailure, BrowserSessionRecord, SourceRecord,
    StudyViewResponse,
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
pub(crate) struct StudyStorage {
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

    #[cfg(test)]
    pub(crate) fn file(store_root: impl Into<PathBuf>, now: fn() -> i64) -> Self {
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

    pub(crate) fn save_auth_challenge(
        &self,
        challenge_hash: &str,
        email: &str,
        expires_at_ms: i64,
    ) -> Result<(), ApiFailure> {
        self.inner
            .save_auth_challenge(challenge_hash, email, expires_at_ms)
    }

    pub(crate) fn consume_auth_challenge(
        &self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, ApiFailure> {
        self.inner.consume_auth_challenge(challenge_hash, now_ms)
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
    fn submit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure>;
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
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ApiFailure::internal(error.to_string()))?;
            }
            write_atomic(&path, &format!("{window_start_ms}\n{attempts}\n"))?;
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
                .map(|source| SourceRecord {
                    source_id: source.id,
                    title: source.title,
                    body: source.body.unwrap_or_default(),
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
                .any(|source| source.id == source_id)
            {
                return Err(ApiFailure::not_found("Source not found."));
            }
            let mut study = BetaStudySession::from_store(account, self.now);
            let view = run_source_generation(&mut study, source_id)?;

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
}
