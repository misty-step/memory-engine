//! Production HTTP API boundary for the mobile study app.
//!
//! This crate owns transport-facing account/session behavior. It intentionally
//! stays outside `memory-engine-core`, which remains pure learning semantics.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use memory_engine_persistence::BetaPersistenceStore;
use memory_engine_persistence_postgres::{AccountScope, AccountStudyStore, PostgresStudyStore};
use memory_engine_study::{
    BetaStudyCurrent, BetaStudyDraftRow, BetaStudyOptions, BetaStudySession, BetaStudySourceInput,
    BetaStudySummary, BetaStudyView,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default)]
pub struct ApiState {
    accounts: AccountRegistry,
}

impl ApiState {
    #[must_use]
    pub fn new(accounts: AccountRegistry) -> Self {
        Self { accounts }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccountRegistry {
    inner: Arc<Mutex<AccountRegistryData>>,
}

impl AccountRegistry {
    #[must_use]
    pub fn with_store_root(store_root: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AccountRegistryData {
                accounts: BTreeMap::new(),
                storage: StudyStorageBackend::File {
                    store_root: store_root.into(),
                },
            })),
        }
    }

    #[must_use]
    pub fn with_postgres_url(database_url: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AccountRegistryData {
                accounts: BTreeMap::new(),
                storage: StudyStorageBackend::Postgres {
                    database_url: database_url.into(),
                },
            })),
        }
    }
}

#[derive(Debug, Default)]
struct AccountRegistryData {
    accounts: BTreeMap<String, AccountRecord>,
    storage: StudyStorageBackend,
}

#[derive(Clone, Debug)]
struct AccountRecord {
    session_token: String,
    store_path: PathBuf,
    sources: BTreeMap<String, SourceRecord>,
    submitted_reviews: BTreeMap<String, StudyViewResponse>,
}

#[derive(Clone, Debug)]
enum StudyStorageBackend {
    File { store_root: PathBuf },
    Postgres { database_url: String },
}

impl Default for StudyStorageBackend {
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

impl AccountRegistry {
    /// Create a local account record for the production shell.
    ///
    /// The first slice keeps this registry in-memory while the Postgres adapter
    /// is shaped behind the same account-scoped route contract.
    fn create_account(&self, email: &str) -> Result<AccountCreated, ApiFailure> {
        let account_id = account_id_for(email);
        if self.account_exists(&account_id)? {
            return Err(ApiFailure::conflict("Account already exists."));
        }
        let account = AccountCreated {
            account_id: account_id.clone(),
            session_token: new_session_token(),
        };
        let storage = self.storage();
        storage.save_account_session(&account_id, &account.session_token)?;
        let mut data = self.inner.lock().expect("account registry lock");
        let record = data
            .accounts
            .entry(account.account_id.clone())
            .or_insert_with(|| AccountRecord {
                session_token: String::new(),
                store_path: storage.account_store_path(&account_id),
                sources: BTreeMap::new(),
                submitted_reviews: BTreeMap::new(),
            });
        record.session_token.clone_from(&account.session_token);
        drop(data);

        Ok(account)
    }

    fn save_account(
        &self,
        source_account_id: &str,
        source_session_token: &str,
        email: &str,
    ) -> Result<AccountCreated, ApiFailure> {
        let target_account_id = account_id_for(email);
        let target = AccountCreated {
            account_id: target_account_id.clone(),
            session_token: new_session_token(),
        };
        let source = self.require_account(source_account_id, source_session_token)?;
        let storage = self.storage();
        if target_account_id != source_account_id && self.account_exists(&target_account_id)? {
            return Err(ApiFailure::conflict("Account already exists."));
        }
        storage.save_account_session(&target_account_id, &target.session_token)?;
        let target_store_path = storage.account_store_path(&target_account_id);
        storage.copy_account(source_account_id, &target_account_id, &source.store_path)?;

        let mut data = self.inner.lock().expect("account registry lock");
        let record = data
            .accounts
            .entry(target.account_id.clone())
            .or_insert_with(|| AccountRecord {
                session_token: String::new(),
                store_path: target_store_path,
                sources: source.sources.clone(),
                submitted_reviews: BTreeMap::new(),
            });
        record.session_token.clone_from(&target.session_token);

        Ok(target)
    }

    fn save_source(
        &self,
        account_id: &str,
        session_token: &str,
        request: &CreateSourceRequest,
    ) -> Result<SourceRecord, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let title = normalize_required_text(&request.title, "Source title")?;
        let body = normalize_required_text(&request.body, "Source body")?;
        let source = SourceRecord {
            source_id: source_id_for(account_id, &title, &body),
            title,
            body,
        };

        let storage = self.storage();
        storage.save_source(account_id, &account.store_path, &source)?;
        let mut data = self.inner.lock().expect("account registry lock");
        let record = data
            .accounts
            .entry(account_id.to_owned())
            .or_insert_with(|| account.clone());
        record
            .sources
            .entry(source.source_id.clone())
            .or_insert_with(|| source.clone());

        Ok(source)
    }

    fn list_sources(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let storage = self.storage();

        storage.list_sources(account_id, &account.store_path)
    }

    fn generate_source(
        &self,
        account_id: &str,
        session_token: &str,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .generate_source(account_id, &account.store_path, source_id)
    }

    fn approve_draft(
        &self,
        account_id: &str,
        session_token: &str,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .approve_draft(account_id, &account.store_path, draft_id)
    }

    fn next_review(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage().next_review(account_id, &account.store_path)
    }

    fn study_view(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage().study_view(account_id, &account.store_path)
    }

    fn reveal_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        self.storage()
            .reveal_review(account_id, &account.store_path, review_unit_id)
    }

    fn submit_review(
        &self,
        account_id: &str,
        session_token: &str,
        review_unit_id: &str,
        request: &SubmitReviewRequest,
    ) -> Result<StudyViewResponse, ApiFailure> {
        let account = self.require_account(account_id, session_token)?;
        let idempotency_key = normalize_required_text(&request.idempotency_key, "Idempotency key")?;
        if let Some(response) = account.submitted_reviews.get(&idempotency_key) {
            return Ok(response.clone());
        }
        let answer = normalize_required_text(&request.answer, "Review answer")?;
        if request.response_time_ms == 0 {
            return Err(ApiFailure::bad_request(
                "Review response time must be a positive integer.",
            ));
        }
        let response = self.storage().submit_review(
            account_id,
            &account.store_path,
            review_unit_id,
            answer,
            request.response_time_ms,
            idempotency_key.clone(),
        )?;
        let mut data = self.inner.lock().expect("account registry lock");
        let record = data
            .accounts
            .entry(account_id.to_owned())
            .or_insert_with(|| account.clone());
        require_account_session(record, session_token)?;
        record
            .submitted_reviews
            .insert(idempotency_key, response.clone());

        Ok(response)
    }

    fn storage(&self) -> StudyStorageBackend {
        let data = self.inner.lock().expect("account registry lock");
        data.storage.clone()
    }

    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        let storage = self.storage();
        {
            let data = self.inner.lock().expect("account registry lock");
            if data.accounts.contains_key(account_id) {
                return Ok(true);
            }
        }

        storage.account_exists(account_id)
    }

    fn require_account(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<AccountRecord, ApiFailure> {
        let storage = self.storage();
        {
            let data = self.inner.lock().expect("account registry lock");
            if let Some(account) = data.accounts.get(account_id) {
                require_account_session(account, session_token)?;

                return Ok(account.clone());
            }
        }

        if storage.account_session_matches(account_id, session_token)? {
            return Ok(AccountRecord {
                session_token: session_token.to_owned(),
                store_path: storage.account_store_path(account_id),
                sources: BTreeMap::new(),
                submitted_reviews: BTreeMap::new(),
            });
        }

        Err(ApiFailure::unknown_account())
    }
}

impl StudyStorageBackend {
    fn account_store_path(&self, account_id: &str) -> PathBuf {
        match self {
            Self::File { store_root } => account_store_path(store_root, account_id),
            Self::Postgres { .. } => PathBuf::new(),
        }
    }

    fn save_account_session(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<(), ApiFailure> {
        match self {
            Self::File { store_root } => {
                let path = account_session_path(store_root, account_id);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| ApiFailure::internal(error.to_string()))?;
                }
                fs::write(path, session_token)
                    .map_err(|error| ApiFailure::internal(error.to_string()))
            }
            Self::Postgres { database_url } => {
                with_postgres_account(database_url, account_id, |mut account| {
                    account
                        .save_api_session(session_token, api_study_now())
                        .map_err(postgres_failure)
                })
            }
        }
    }

    fn account_session_matches(
        &self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, ApiFailure> {
        match self {
            Self::File { store_root } => {
                let path = account_session_path(store_root, account_id);
                let Ok(saved) = fs::read_to_string(path) else {
                    return Ok(false);
                };

                Ok(saved.trim() == session_token)
            }
            Self::Postgres { database_url } => with_postgres_store(database_url, |store| {
                store
                    .api_session_matches(account_id, session_token)
                    .map_err(postgres_failure)
            }),
        }
    }

    fn account_exists(&self, account_id: &str) -> Result<bool, ApiFailure> {
        match self {
            Self::File { store_root } => Ok(account_session_path(store_root, account_id).exists()
                || account_store_path(store_root, account_id).exists()),
            Self::Postgres { database_url } => with_postgres_store(database_url, |store| {
                store.account_exists(account_id).map_err(postgres_failure)
            }),
        }
    }

    fn copy_account(
        &self,
        source_account_id: &str,
        target_account_id: &str,
        source_store_path: &FsPath,
    ) -> Result<(), ApiFailure> {
        match self {
            Self::File { store_root } => {
                let target_store_path = account_store_path(store_root, target_account_id);
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
            Self::Postgres { database_url } => {
                let snapshot = with_postgres_account(database_url, source_account_id, |account| {
                    account.snapshot().map_err(postgres_failure)
                })?;
                with_postgres_account(database_url, target_account_id, |mut account| {
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
                                api_study_now(),
                            )
                            .map_err(postgres_failure)?;
                    }
                    Ok(())
                })
            }
        }
    }

    fn save_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source: &SourceRecord,
    ) -> Result<(), ApiFailure> {
        match self {
            Self::File { .. } => {
                let mut study = open_study_session(store_path)?;
                study
                    .add_source(BetaStudySourceInput {
                        id: source.source_id.clone(),
                        title: source.title.clone(),
                        body: source.body.clone(),
                    })
                    .map_err(study_failure)?;
                Ok(())
            }
            Self::Postgres { database_url } => {
                with_postgres_study(database_url, account_id, |study| {
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
        }
    }

    fn list_sources(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<Vec<SourceRecord>, ApiFailure> {
        match self {
            Self::File { .. } => persisted_sources(store_path),
            Self::Postgres { database_url } => {
                with_postgres_account(database_url, account_id, |account| {
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
        }
    }

    fn generate_source(
        &self,
        account_id: &str,
        store_path: &FsPath,
        source_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                if !persisted_source_exists(store_path, source_id)? {
                    return Err(ApiFailure::not_found("Source not found."));
                }
                let mut study = open_study_session(store_path)?;
                let view = study
                    .generate(Some(vec![source_id.to_owned()]))
                    .map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_account(database_url, account_id, |account| {
                    if !account
                        .snapshot()
                        .map_err(postgres_failure)?
                        .source_documents
                        .iter()
                        .any(|source| source.id == source_id)
                    {
                        return Err(ApiFailure::not_found("Source not found."));
                    }
                    let mut study = BetaStudySession::from_store(account, api_study_now);
                    let view = study
                        .generate(Some(vec![source_id.to_owned()]))
                        .map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }

    fn approve_draft(
        &self,
        account_id: &str,
        store_path: &FsPath,
        draft_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                let mut study = open_study_session(store_path)?;
                let view = study.approve_draft(draft_id).map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_study(database_url, account_id, |study| {
                    let view = study.approve_draft(draft_id).map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }

    fn next_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                let mut study = open_study_session(store_path)?;
                let view = study.start().map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_study(database_url, account_id, |study| {
                    let view = study.start().map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }

    fn study_view(
        &self,
        account_id: &str,
        store_path: &FsPath,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                let study = open_study_session(store_path)?;
                let view = study.view().map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_study(database_url, account_id, |study| {
                    let view = study.view().map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }

    fn reveal_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                let mut study = open_study_session(store_path)?;
                require_current_review(&mut study, review_unit_id)?;
                let view = study.reveal().map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_study(database_url, account_id, |study| {
                    require_current_review_postgres(study, review_unit_id)?;
                    let view = study.reveal().map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }

    fn submit_review(
        &self,
        account_id: &str,
        store_path: &FsPath,
        review_unit_id: &str,
        answer: String,
        response_time_ms: u32,
        idempotency_key: String,
    ) -> Result<StudyViewResponse, ApiFailure> {
        match self {
            Self::File { .. } => {
                let mut study = open_study_session(store_path)?;
                require_current_review(&mut study, review_unit_id)?;
                let view = study
                    .submit_answer_with_idempotency_key(
                        answer,
                        response_time_ms,
                        Some(idempotency_key),
                    )
                    .map_err(study_failure)?;

                Ok(StudyViewResponse::from_view(view))
            }
            Self::Postgres { database_url } => {
                with_postgres_account(database_url, account_id, |account| {
                    if account
                        .applied_review_idempotency_key_exists(&idempotency_key)
                        .map_err(postgres_failure)?
                    {
                        let study = BetaStudySession::from_store(account, api_study_now);
                        let view = study.view().map_err(study_failure)?;

                        return Ok(StudyViewResponse::from_view(view));
                    }

                    let mut study = BetaStudySession::from_store(account, api_study_now);
                    require_current_review_postgres(&mut study, review_unit_id)?;
                    let view = study
                        .submit_answer_with_idempotency_key(
                            answer,
                            response_time_ms,
                            Some(idempotency_key),
                        )
                        .map_err(study_failure)?;

                    Ok(StudyViewResponse::from_view(view))
                })
            }
        }
    }
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyViewResponse {
    pub drafts: Vec<BetaStudyDraftRow>,
    pub current: Option<BetaStudyCurrent>,
    pub summary: BetaStudySummary,
}

impl StudyViewResponse {
    fn from_view(view: BetaStudyView) -> Self {
        Self {
            drafts: view.drafts,
            current: view.current,
            summary: view.summary,
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

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(app_home))
        .route("/accounts", post(create_account))
        .route("/app/start", post(start_app_study))
        .route("/app/account", post(create_app_account))
        .route("/app/save-account", post(save_app_account))
        .route("/app/source", post(create_app_source))
        .route("/app/generate", post(generate_app_source))
        .route("/app/approve", post(approve_app_draft))
        .route("/app/next", post(next_app_review))
        .route("/app/reveal", post(reveal_app_review))
        .route("/app/submit", post(submit_app_review))
        .route(
            "/accounts/{account_id}/sources",
            get(list_sources).post(create_source),
        )
        .route(
            "/accounts/{account_id}/sources/{source_id}/generate",
            post(generate_source),
        )
        .route(
            "/accounts/{account_id}/drafts/{draft_id}/approve",
            post(approve_draft),
        )
        .route("/accounts/{account_id}/review/next", get(next_review))
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/reveal",
            post(reveal_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/submit",
            post(submit_review),
        )
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "memory-engine-api",
    })
}

async fn app_home() -> Html<String> {
    Html(render_app_shell(None, &[], None, None))
}

async fn create_account(
    State(state): State<ApiState>,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountCreated>), ApiFailure> {
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))?;
    let account = state.accounts.create_account(&email)?;

    Ok((StatusCode::CREATED, Json(account)))
}

async fn create_source(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<SourceRecord>), ApiFailure> {
    let session_token = read_session_token(&headers)?;
    let source = state
        .accounts
        .save_source(&account_id, session_token, &request)?;

    Ok((StatusCode::CREATED, Json(source)))
}

async fn list_sources(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SourceList>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(SourceList {
        sources: state.accounts.list_sources(&account_id, session_token)?,
    }))
}

async fn generate_source(
    State(state): State<ApiState>,
    Path((account_id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.accounts.generate_source(
        &account_id,
        session_token,
        &source_id,
    )?))
}

async fn approve_draft(
    State(state): State<ApiState>,
    Path((account_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.accounts.approve_draft(
        &account_id,
        session_token,
        &draft_id,
    )?))
}

async fn next_review(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(
        state.accounts.next_review(&account_id, session_token)?,
    ))
}

async fn reveal_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.accounts.reveal_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn submit_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<SubmitReviewRequest>,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.accounts.submit_review(
        &account_id,
        session_token,
        &review_unit_id,
        &request,
    )?))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppAccountForm {
    email: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppStartForm {
    title: String,
    body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSourceForm {
    account_id: String,
    session_token: String,
    title: String,
    body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSaveAccountForm {
    account_id: String,
    session_token: String,
    email: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppAccountActionForm {
    account_id: String,
    session_token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSourceActionForm {
    account_id: String,
    session_token: String,
    source_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppDraftActionForm {
    account_id: String,
    session_token: String,
    draft_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewActionForm {
    account_id: String,
    session_token: String,
    review_unit_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewSubmitForm {
    account_id: String,
    session_token: String,
    review_unit_id: String,
    answer: String,
    response_time_ms: u32,
    idempotency_key: String,
}

async fn create_app_account(
    State(state): State<ApiState>,
    Form(form): Form<AppAccountForm>,
) -> Html<String> {
    let result = normalize_email(&form.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))
        .and_then(|email| state.accounts.create_account(&email));

    match result {
        Ok(account) => {
            let account = AppAccount::from(account);
            let view = state
                .accounts
                .study_view(&account.account_id, &account.session_token)
                .ok();
            Html(render_account_page(&state, &account, view.as_ref(), None))
        }
        Err(error) => Html(render_app_shell(None, &[], None, Some(&error.message))),
    }
}

async fn save_app_account(
    State(state): State<ApiState>,
    Form(form): Form<AppSaveAccountForm>,
) -> Html<String> {
    let source_account = form.account();
    let source_view = state
        .accounts
        .study_view(&form.account_id, &form.session_token)
        .ok();
    let result = normalize_email(&form.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))
        .and_then(|email| {
            state
                .accounts
                .save_account(&form.account_id, &form.session_token, &email)
        });

    match result {
        Ok(account) => {
            let account = AppAccount::from(account);
            let view = state
                .accounts
                .study_view(&account.account_id, &account.session_token)
                .ok()
                .or(source_view);
            Html(render_account_page(&state, &account, view.as_ref(), None))
        }
        Err(error) => Html(render_account_page(
            &state,
            &source_account,
            source_view.as_ref(),
            Some(&error.message),
        )),
    }
}

async fn start_app_study(
    State(state): State<ApiState>,
    Form(form): Form<AppStartForm>,
) -> Html<String> {
    let email = format!("guest-{:032x}@memory-engine.local", rand::random::<u128>());
    let account = match state.accounts.create_account(&email) {
        Ok(account) => account,
        Err(error) => return Html(render_app_shell(None, &[], None, Some(&error.message))),
    };
    let account = AppAccount::from(account);
    let source = state.accounts.save_source(
        &account.account_id,
        &account.session_token,
        &CreateSourceRequest {
            title: form.title,
            body: form.body,
        },
    );
    let result = source.and_then(|source| {
        state.accounts.generate_source(
            &account.account_id,
            &account.session_token,
            &source.source_id,
        )
    });

    render_action_result(&state, &account, result)
}

async fn create_app_source(
    State(state): State<ApiState>,
    Form(form): Form<AppSourceForm>,
) -> Html<String> {
    let account = form.account();
    let result = state
        .accounts
        .save_source(
            &form.account_id,
            &form.session_token,
            &CreateSourceRequest {
                title: form.title,
                body: form.body,
            },
        )
        .and_then(|_| {
            state
                .accounts
                .list_sources(&form.account_id, &form.session_token)
        });

    match result {
        Ok(sources) => Html(render_app_shell(Some(&account), &sources, None, None)),
        Err(error) => Html(render_account_page(
            &state,
            &account,
            None,
            Some(&error.message),
        )),
    }
}

async fn generate_app_source(
    State(state): State<ApiState>,
    Form(form): Form<AppSourceActionForm>,
) -> Html<String> {
    let account = form.account();
    let result =
        state
            .accounts
            .generate_source(&form.account_id, &form.session_token, &form.source_id);

    render_action_result(&state, &account, result)
}

async fn approve_app_draft(
    State(state): State<ApiState>,
    Form(form): Form<AppDraftActionForm>,
) -> Html<String> {
    let account = form.account();
    let result =
        state
            .accounts
            .approve_draft(&form.account_id, &form.session_token, &form.draft_id);

    render_action_result(&state, &account, result)
}

async fn next_app_review(
    State(state): State<ApiState>,
    Form(form): Form<AppAccountActionForm>,
) -> Html<String> {
    let account = form.account();
    let result = state
        .accounts
        .next_review(&form.account_id, &form.session_token);

    render_action_result(&state, &account, result)
}

async fn reveal_app_review(
    State(state): State<ApiState>,
    Form(form): Form<AppReviewActionForm>,
) -> Html<String> {
    let account = form.account();
    let result =
        state
            .accounts
            .reveal_review(&form.account_id, &form.session_token, &form.review_unit_id);

    render_action_result(&state, &account, result)
}

async fn submit_app_review(
    State(state): State<ApiState>,
    Form(form): Form<AppReviewSubmitForm>,
) -> Html<String> {
    let account = form.account();
    let result = state.accounts.submit_review(
        &form.account_id,
        &form.session_token,
        &form.review_unit_id,
        &SubmitReviewRequest {
            answer: form.answer,
            response_time_ms: form.response_time_ms,
            idempotency_key: form.idempotency_key,
        },
    );

    render_action_result(&state, &account, result)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AppAccount {
    account_id: String,
    session_token: String,
}

impl From<AccountCreated> for AppAccount {
    fn from(account: AccountCreated) -> Self {
        Self {
            account_id: account.account_id,
            session_token: account.session_token,
        }
    }
}

impl AppSourceForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppSaveAccountForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppAccountActionForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppSourceActionForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppDraftActionForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppReviewActionForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

impl AppReviewSubmitForm {
    fn account(&self) -> AppAccount {
        AppAccount {
            account_id: self.account_id.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

fn render_action_result(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
) -> Html<String> {
    match result {
        Ok(view) => Html(render_account_page(state, account, Some(&view), None)),
        Err(error) => Html(render_account_page(
            state,
            account,
            None,
            Some(&error.message),
        )),
    }
}

fn render_account_page(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    error: Option<&str>,
) -> String {
    let sources = state
        .accounts
        .list_sources(&account.account_id, &account.session_token)
        .unwrap_or_default();
    render_app_shell(Some(account), &sources, view, error)
}

fn render_app_shell(
    account: Option<&AppAccount>,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    error: Option<&str>,
) -> String {
    let account_panel = account.map_or_else(render_account_form, |account| {
        [
            render_account_status(account),
            render_source_form(account),
            render_sources(account, sources),
        ]
        .join("")
    });
    let study_panel = account.map_or_else(String::new, |account| {
        view.map_or_else(String::new, |view| render_study(account, view))
    });
    let error_panel = error.map_or_else(String::new, |message| {
        format!(
            r#"<section class="notice" role="alert">{}</section>"#,
            escape_html(message)
        )
    });

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Memory Engine Study</title>
  <style>{APP_CSS}</style>
</head>
<body>
  <main>
    <header>
      <p class="eyebrow">Memory Engine</p>
      <h1>Study from source material</h1>
    </header>
    {error_panel}
    {account_panel}
    {study_panel}
  </main>
</body>
</html>"#
    )
}

fn render_account_form() -> String {
    format!(
        r#"{}<section>
  <h2>Create account</h2>
  <form action="/app/account" method="post">
    <label>Email <input name="email" type="email" autocomplete="email" required></label>
    <button type="submit">Continue</button>
  </form>
</section>"#,
        render_start_form()
    )
}

fn render_start_form() -> String {
    format!(
        r#"<section>
  <h2>Add source</h2>
  <form action="/app/start" method="post">
    <label>Title <input name="title" required value="NATO practice notes"></label>
    <label>Source text <textarea name="body" rows="11" required>{}</textarea></label>
    <button type="submit">Generate study material</button>
  </form>
</section>"#,
        escape_html(DEFAULT_SOURCE_BODY)
    )
}

fn render_account_status(account: &AppAccount) -> String {
    format!(
        r#"<section class="compact">
  <h2>Account</h2>
  <p class="muted">Session ready for <code>{}</code>.</p>
  <form action="/app/save-account" method="post">
    {}
    <label>Email <input name="email" type="email" autocomplete="email" required></label>
    <button type="submit">Save account email</button>
  </form>
</section>"#,
        escape_html(&account.account_id),
        hidden_account_inputs(account)
    )
}

fn render_source_form(account: &AppAccount) -> String {
    format!(
        r#"<section>
  <h2>Add source</h2>
  <form action="/app/source" method="post">
    {}
    <label>Title <input name="title" required value="NATO practice notes"></label>
    <label>Source text <textarea name="body" rows="11" required>{}</textarea></label>
    <button type="submit">Save source</button>
  </form>
</section>"#,
        hidden_account_inputs(account),
        escape_html(DEFAULT_SOURCE_BODY)
    )
}

fn render_sources(account: &AppAccount, sources: &[SourceRecord]) -> String {
    if sources.is_empty() {
        return r#"<section class="compact"><h2>Sources</h2><p class="muted">No sources yet.</p></section>"#
            .to_owned();
    }

    let mut rows = String::new();
    for source in sources {
        write!(
            rows,
            r#"<article class="item">
  <h3>{}</h3>
  <p>{}</p>
  <form action="/app/generate" method="post">
    {}
    <input type="hidden" name="sourceId" value="{}">
    <button type="submit">Generate study material</button>
  </form>
</article>"#,
            escape_html(&source.title),
            escape_html(&source.body),
            hidden_account_inputs(account),
            escape_html(&source.source_id)
        )
        .expect("write source html");
    }

    format!(r"<section><h2>Sources</h2>{rows}</section>")
}

fn render_study(account: &AppAccount, view: &StudyViewResponse) -> String {
    [
        render_summary(&view.summary),
        render_drafts(account, &view.drafts),
        render_current_review(account, view.current.as_ref()),
    ]
    .join("")
}

fn render_summary(summary: &BetaStudySummary) -> String {
    format!(
        r#"<section class="compact">
  <h2>Progress</h2>
  <div class="metrics">
    <span><strong>{}</strong> sources</span>
    <span><strong>{}</strong> drafts</span>
    <span><strong>{}</strong> reviews</span>
    <span><strong>{}</strong> attempts</span>
  </div>
</section>"#,
        summary.source_count,
        summary.accepted_draft_count,
        summary.approved_review_unit_count,
        summary.attempt_count
    )
}

fn render_drafts(account: &AppAccount, drafts: &[BetaStudyDraftRow]) -> String {
    if drafts.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for draft in drafts {
        let action = if draft.validation_status
            == memory_engine_persistence::GeneratedPromptValidationStatus::Accepted
        {
            format!(
                r#"<form action="/app/approve" method="post">
  {}
  <input type="hidden" name="draftId" value="{}">
  <button type="submit">Keep for review</button>
</form>"#,
                hidden_account_inputs(account),
                escape_html(&draft.id)
            )
        } else {
            String::new()
        };
        write!(
            rows,
            r#"<article class="item">
  <h3>{}</h3>
  <p>{}</p>
  <p class="muted">{}</p>
  {}
</article>"#,
            escape_html(&draft.activity_stage),
            escape_html(&draft.prompt),
            escape_html(&draft.validation_reasons.join(", ")),
            action
        )
        .expect("write draft html");
    }

    format!(r"<section><h2>Generated material</h2>{rows}</section>")
}

fn render_current_review(account: &AppAccount, current: Option<&BetaStudyCurrent>) -> String {
    let Some(current) = current else {
        return String::new();
    };
    let expected = current
        .expected_answer
        .as_ref()
        .map_or_else(String::new, |answer| {
            format!(
                r#"<div class="answer"><span>Answer</span><strong>{}</strong></div>"#,
                escape_html(answer)
            )
        });
    let grade = current.grade.as_ref().map_or_else(String::new, |grade| {
        format!(
            r#"<p class="muted">Last result: {:?}</p>
    <form action="/app/next" method="post">
      {}
      <button type="submit">Next review</button>
    </form>"#,
            grade.verdict,
            hidden_account_inputs(account)
        )
    });

    format!(
        r#"<section>
  <h2>Review</h2>
  <article class="item focus">
    <h3>{}</h3>
    <p>{}</p>
    {}
    {}
    <form action="/app/reveal" method="post">
      {}
      <input type="hidden" name="reviewUnitId" value="{}">
      <button type="submit">Reveal answer</button>
    </form>
    <form action="/app/submit" method="post">
      {}
      <input type="hidden" name="reviewUnitId" value="{}">
      <input type="hidden" name="responseTimeMs" value="1800">
      <input type="hidden" name="idempotencyKey" value="review-{}">
      <label>Your answer <input name="answer" required autocomplete="off"></label>
      <button type="submit">Submit review</button>
    </form>
  </article>
</section>"#,
        escape_html(&current.activity_stage),
        escape_html(&current.prompt),
        expected,
        grade,
        hidden_account_inputs(account),
        escape_html(&current.review_unit_id.to_string()),
        hidden_account_inputs(account),
        escape_html(&current.review_unit_id.to_string()),
        escape_html(&current.review_unit_id.to_string())
    )
}

fn hidden_account_inputs(account: &AppAccount) -> String {
    format!(
        r#"<input type="hidden" name="accountId" value="{}">
<input type="hidden" name="sessionToken" value="{}">"#,
        escape_html(&account.account_id),
        escape_html(&account.session_token)
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const DEFAULT_SOURCE_BODY: &str = "\
Concept: NATO letter A
Activity: quiz
Stage: recognition-3
Question: What is the NATO phonetic alphabet word for A?
Answer: ALFA
Distractors: BRAVO, CHARLIE
Reference: The NATO phonetic alphabet word for A is ALFA.

Concept: NATO CAT composition
Activity: exercise
Stage: composition
Question: Spell CAT over the phone using the NATO phonetic alphabet.
Answer: CHARLIE ALFA TANGO
Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.
Reference: C is CHARLIE. A is ALFA. T is TANGO.";

const APP_CSS: &str = r"
:root {
  color-scheme: light;
  font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  background: #f6f7f8;
  color: #172026;
}
* { box-sizing: border-box; }
body { margin: 0; }
main {
  width: min(100%, 720px);
  margin: 0 auto;
  padding: 20px 14px 40px;
}
header { padding: 8px 0 14px; }
.eyebrow {
  margin: 0 0 6px;
  font-size: 0.78rem;
  text-transform: uppercase;
  color: #56616a;
}
h1, h2, h3, p { overflow-wrap: anywhere; }
h1 { margin: 0; font-size: 1.85rem; line-height: 1.08; }
h2 { margin: 0 0 12px; font-size: 1.08rem; }
h3 { margin: 0 0 8px; font-size: 1rem; }
section {
  margin: 12px 0;
  padding: 14px;
  background: #ffffff;
  border: 1px solid #d8dde2;
  border-radius: 8px;
}
.compact { padding: 12px 14px; }
.notice {
  border-color: #ad3f32;
  background: #fff1ef;
  color: #7f241a;
}
.item {
  margin: 10px 0;
  padding: 12px;
  border: 1px solid #d8dde2;
  border-radius: 8px;
  background: #fbfcfc;
}
.focus { border-color: #2f6f73; }
.muted { color: #56616a; font-size: 0.92rem; }
.metrics {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}
.metrics span {
  padding: 8px;
  background: #edf2f2;
  border-radius: 6px;
}
form { display: grid; gap: 10px; margin: 10px 0 0; }
label { display: grid; gap: 6px; font-weight: 650; }
input, textarea {
  width: 100%;
  min-width: 0;
  padding: 11px 12px;
  border: 1px solid #bac3c9;
  border-radius: 6px;
  font: inherit;
  background: #ffffff;
}
textarea { resize: vertical; }
button {
  width: 100%;
  min-height: 44px;
  border: 0;
  border-radius: 6px;
  background: #275d61;
  color: #ffffff;
  font: inherit;
  font-weight: 750;
}
code {
  font-size: 0.82rem;
  white-space: normal;
}
.answer {
  display: grid;
  gap: 4px;
  margin: 10px 0;
  padding: 10px;
  border-radius: 6px;
  background: #e9f3ec;
}
.answer span {
  color: #46614d;
  font-size: 0.78rem;
  text-transform: uppercase;
}
";

#[derive(Debug)]
struct ApiFailure {
    status: StatusCode,
    message: String,
}

impl ApiFailure {
    fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_owned(),
        }
    }

    fn unknown_account() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "Account not found.".to_owned(),
        }
    }

    fn not_found(message: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.to_owned(),
        }
    }

    fn conflict(message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.to_owned(),
        }
    }

    fn missing_session() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "Session token is required.".to_owned(),
        }
    }

    fn forbidden_account() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "Session token does not match account.".to_owned(),
        }
    }

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
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

fn normalize_email(email: &str) -> Option<String> {
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

fn read_session_token(headers: &HeaderMap) -> Result<&str, ApiFailure> {
    headers
        .get("x-session-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(ApiFailure::missing_session)
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

fn source_id_for(account_id: &str, title: &str, body: &str) -> String {
    let stable = [account_id, title, body]
        .into_iter()
        .flat_map(str::bytes)
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });

    format!("src_{stable:016x}")
}

fn account_store_path(store_root: &FsPath, account_id: &str) -> PathBuf {
    store_root.join(account_id).join("study.json")
}

fn account_session_path(store_root: &FsPath, account_id: &str) -> PathBuf {
    store_root.join(account_id).join("session.token")
}

fn open_study_session(path: &FsPath) -> Result<BetaStudySession, ApiFailure> {
    BetaStudySession::open(BetaStudyOptions::new(path)).map_err(study_failure)
}

fn open_persistence_store(path: &FsPath) -> Result<BetaPersistenceStore, ApiFailure> {
    BetaPersistenceStore::open(path).map_err(|error| ApiFailure::internal(error.to_string()))
}

fn api_study_now() -> i64 {
    memory_engine_study::DEFAULT_BETA_STUDY_NOW
}

fn with_postgres_account<R>(
    database_url: &str,
    account_id: &str,
    operation: impl FnOnce(AccountStudyStore<'_>) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    let run = || {
        let mut store = PostgresStudyStore::connect(database_url).map_err(postgres_failure)?;
        store.migrate().map_err(postgres_failure)?;
        let scope = AccountScope::new(account_id.to_owned()).map_err(postgres_failure)?;
        let mut account = store.for_account(scope);
        account
            .ensure_account(api_study_now())
            .map_err(postgres_failure)?;

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
        let mut store = PostgresStudyStore::connect(database_url).map_err(postgres_failure)?;
        store.migrate().map_err(postgres_failure)?;

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
    operation: impl FnOnce(&mut BetaStudySession<AccountStudyStore<'_>>) -> Result<R, ApiFailure>,
) -> Result<R, ApiFailure> {
    with_postgres_account(database_url, account_id, |account| {
        let mut study = BetaStudySession::from_store(account, api_study_now);
        operation(&mut study)
    })
}

fn postgres_failure(error: memory_engine_persistence_postgres::PostgresStoreError) -> ApiFailure {
    let message = error.to_string();
    drop(error);
    ApiFailure::internal(message)
}

fn persisted_sources(path: &FsPath) -> Result<Vec<SourceRecord>, ApiFailure> {
    let store = open_persistence_store(path)?;
    Ok(store
        .snapshot()
        .source_documents
        .into_iter()
        .map(|source| SourceRecord {
            source_id: source.id,
            title: source.title,
            body: source.body.unwrap_or_default(),
        })
        .collect())
}

fn persisted_source_exists(path: &FsPath, source_id: &str) -> Result<bool, ApiFailure> {
    let store = open_persistence_store(path)?;
    Ok(store
        .snapshot()
        .source_documents
        .iter()
        .any(|source| source.id == source_id))
}

fn study_failure<E: std::fmt::Display>(
    error: memory_engine_study::BetaStudyError<E>,
) -> ApiFailure {
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
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use postgres::{Client, NoTls};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::{router, AccountRegistry, ApiState};

    #[tokio::test]
    async fn healthz_exposes_production_api_boundary() {
        let response = router(ApiState::default())
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["status"], json!("ok"));
        assert_eq!(body["service"], json!("memory-engine-api"));
    }

    #[tokio::test]
    async fn mobile_home_prioritizes_source_capture_before_account_creation() {
        let response = router(ApiState::default())
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains(r#"<form action="/app/start" method="post">"#));
        assert!(body.contains("Generate study material"));
        assert!(body.contains(r#"<form action="/app/account" method="post">"#));
        assert!(
            body.find("/app/start").expect("start form")
                < body.find("/app/account").expect("account form")
        );
    }

    #[tokio::test]
    async fn mobile_form_flow_generates_keeps_reveals_and_submits_review() {
        let app = router(ApiState::default());
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("title", "NATO practice notes"), ("body", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let started = response_text(started).await;
        assert!(started.contains("Generated material"));
        assert!(started.contains("What is the NATO phonetic alphabet word for A?"));
        assert!(started.contains("Keep for review"));

        let account_id = html_value(&started, "accountId");
        let session_token = html_value(&started, "sessionToken");
        let draft_id = html_value(&started, "draftId");

        let approved = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/approve",
                &[
                    ("accountId", &account_id),
                    ("sessionToken", &session_token),
                    ("draftId", &draft_id),
                ],
            ))
            .await
            .expect("approve");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        assert!(approved.contains("Review"));
        assert!(approved.contains("Reveal answer"));
        let review_unit_id = html_value(&approved, "reviewUnitId");

        let revealed = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/reveal",
                &[
                    ("accountId", &account_id),
                    ("sessionToken", &session_token),
                    ("reviewUnitId", &review_unit_id),
                ],
            ))
            .await
            .expect("reveal");
        assert_eq!(revealed.status(), StatusCode::OK);
        let revealed = response_text(revealed).await;
        assert!(revealed.contains("ALFA"));

        let submitted = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/submit",
                &[
                    ("accountId", &account_id),
                    ("sessionToken", &session_token),
                    ("reviewUnitId", &review_unit_id),
                    ("answer", "ALFA"),
                    ("responseTimeMs", "1800"),
                    ("idempotencyKey", "mobile-nato-a"),
                ],
            ))
            .await
            .expect("submit");
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_text(submitted).await;
        assert!(submitted.contains("Last result: Correct"));
        assert!(submitted.contains("Next review"));
        assert!(submitted.contains("<strong>1</strong> attempts"));

        let next = app
            .oneshot(form_request(
                "POST",
                "/app/next",
                &[("accountId", &account_id), ("sessionToken", &session_token)],
            ))
            .await
            .expect("next");
        assert_eq!(next.status(), StatusCode::OK);
        let next = response_text(next).await;
        assert!(next.contains("Progress"));
    }

    #[tokio::test]
    async fn mobile_saved_account_session_resumes_sources_after_restart() {
        let store_root = temp_store_root("mobile-save-resume");
        let app = router(ApiState::new(super::AccountRegistry::with_store_root(
            &store_root,
        )));
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("title", "NATO practice notes"), ("body", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let started = response_text(started).await;
        let guest_account_id = html_value(&started, "accountId");
        let guest_session_token = html_value(&started, "sessionToken");

        let saved = app
            .oneshot(form_request(
                "POST",
                "/app/save-account",
                &[
                    ("accountId", &guest_account_id),
                    ("sessionToken", &guest_session_token),
                    ("email", " Learner@Example.COM "),
                ],
            ))
            .await
            .expect("save account");
        assert_eq!(saved.status(), StatusCode::OK);
        let saved = response_text(saved).await;
        assert!(saved.contains("acct_fc9e1ff15d47bd67"));
        assert!(saved.contains("NATO practice notes"));
        assert!(saved.contains("Keep for review"));
        let account_id = html_value(&saved, "accountId");
        let session_token = html_value(&saved, "sessionToken");
        let source_id = html_value(&saved, "sourceId");

        let restarted_app = router(ApiState::new(super::AccountRegistry::with_store_root(
            &store_root,
        )));
        let replay = restarted_app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", "learner@example.com")],
            ))
            .await
            .expect("email replay");
        assert_eq!(replay.status(), StatusCode::OK);
        let replay = response_text(replay).await;
        assert!(replay.contains("Account already exists."));

        let generated = restarted_app
            .oneshot(form_request(
                "POST",
                "/app/generate",
                &[
                    ("accountId", &account_id),
                    ("sessionToken", &session_token),
                    ("sourceId", &source_id),
                ],
            ))
            .await
            .expect("generate after resume");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_text(generated).await;
        assert!(generated.contains("Generated material"));
    }

    #[tokio::test]
    async fn create_account_returns_stable_account_id() {
        let request = Request::builder()
            .method("POST")
            .uri("/accounts")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"email":" Learner@Example.COM "}"#))
            .expect("request");

        let response = router(ApiState::default())
            .oneshot(request)
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_json(response).await;
        assert_eq!(body["accountId"], json!("acct_fc9e1ff15d47bd67"));
        assert!(body["sessionToken"]
            .as_str()
            .expect("session token")
            .starts_with("sess_"));
    }

    #[tokio::test]
    async fn create_account_rejects_malformed_email_without_account_id() {
        let request = Request::builder()
            .method("POST")
            .uri("/accounts")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"email":"not-an-email"}"#))
            .expect("request");

        let response = router(ApiState::default())
            .oneshot(request)
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(
            body["error"],
            json!("Account email must contain one @ and a domain.")
        );
        assert!(body.get("accountId").is_none());
    }

    #[tokio::test]
    async fn source_routes_are_scoped_to_the_account() {
        let app = router(ApiState::default());
        let first = create_account(&app, "first@example.com").await;
        let second = create_account(&app, "second@example.com").await;

        let first_saved = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", first.account_id),
                &first.session_token,
                &json!({
                    "title": "NATO notes",
                    "body": "ALFA is the NATO code word for A."
                }),
            ))
            .await
            .expect("save source");

        assert_eq!(first_saved.status(), StatusCode::CREATED);
        let first_saved = response_json(first_saved).await;
        assert_eq!(first_saved["title"], json!("NATO notes"));
        assert_eq!(
            first_saved["body"],
            json!("ALFA is the NATO code word for A.")
        );

        let second_saved = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", second.account_id),
                &second.session_token,
                &json!({
                    "title": "Latin notes",
                    "body": "Poena means punishment."
                }),
            ))
            .await
            .expect("save second source");

        assert_eq!(second_saved.status(), StatusCode::CREATED);
        let second_saved = response_json(second_saved).await;
        assert_eq!(second_saved["title"], json!("Latin notes"));

        let first_sources = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", first.account_id),
                &first.session_token,
            ))
            .await
            .expect("first sources");
        let first_sources = response_json(first_sources).await;
        let first_sources = first_sources["sources"].as_array().expect("sources");
        assert_eq!(first_sources.len(), 1);
        assert_eq!(first_sources[0]["title"], json!("NATO notes"));
        assert_ne!(first_sources[0]["sourceId"], second_saved["sourceId"]);

        let second_sources = app
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", second.account_id),
                &second.session_token,
            ))
            .await
            .expect("second sources");
        let second_sources = response_json(second_sources).await;
        let second_sources = second_sources["sources"]
            .as_array()
            .expect("second sources");
        assert_eq!(second_sources.len(), 1);
        assert_eq!(second_sources[0]["title"], json!("Latin notes"));
        assert_ne!(second_sources[0]["sourceId"], first_saved["sourceId"]);
    }

    #[tokio::test]
    async fn source_routes_reject_unknown_accounts_without_mutating_state() {
        let app = router(ApiState::default());
        let response = app
            .oneshot(json_request(
                "POST",
                "/accounts/acct_missing/sources",
                "sess_missing",
                &json!({
                    "title": "NATO notes",
                    "body": "ALFA is the NATO code word for A."
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        assert_eq!(body["error"], json!("Account not found."));
    }

    #[tokio::test]
    async fn source_routes_reject_cross_account_session_tokens() {
        let app = router(ApiState::default());
        let first = create_account(&app, "first@example.com").await;
        let second = create_account(&app, "second@example.com").await;
        assert_ne!(first.session_token, second.session_token);

        let write = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", second.account_id),
                &first.session_token,
                &json!({
                    "title": "NATO notes",
                    "body": "ALFA is the NATO code word for A."
                }),
            ))
            .await
            .expect("cross write");

        assert_eq!(write.status(), StatusCode::FORBIDDEN);
        let body = response_json(write).await;
        assert_eq!(
            body["error"],
            json!("Session token does not match account.")
        );

        let read = app
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", second.account_id),
                &first.session_token,
            ))
            .await
            .expect("cross read");

        assert_eq!(read.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn recreating_account_rejects_email_replay_without_session() {
        let app = router(ApiState::default());
        let first = create_account(&app, "learner@example.com").await;
        let saved = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", first.account_id),
                &first.session_token,
                &json!({
                    "title": "NATO notes",
                    "body": "ALFA is the NATO code word for A."
                }),
            ))
            .await
            .expect("save source");
        assert_eq!(saved.status(), StatusCode::CREATED);

        let replay = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":" LEARNER@example.com "}"#))
                    .expect("request"),
            )
            .await
            .expect("email replay");
        assert_eq!(replay.status(), StatusCode::CONFLICT);

        let current_session = app
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", first.account_id),
                &first.session_token,
            ))
            .await
            .expect("current session");
        assert_eq!(current_session.status(), StatusCode::OK);
        let current_session = response_json(current_session).await;
        assert_eq!(
            current_session["sources"]
                .as_array()
                .expect("sources")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn configured_store_root_resumes_sources_after_api_restart() {
        let store_root = temp_store_root("restart-resume");
        let first_app = router(ApiState::new(super::AccountRegistry::with_store_root(
            &store_root,
        )));
        let account = create_account(&first_app, "learner@example.com").await;
        let source = save_source(&first_app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id").to_owned();

        let restarted_app = router(ApiState::new(super::AccountRegistry::with_store_root(
            &store_root,
        )));
        let replay = restarted_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":" learner@example.com "}"#))
                    .expect("request"),
            )
            .await
            .expect("email replay after restart");
        assert_eq!(replay.status(), StatusCode::CONFLICT);

        let sources = restarted_app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("resumed sources");
        assert_eq!(sources.status(), StatusCode::OK);
        let sources = response_json(sources).await;
        assert_eq!(sources["sources"][0]["sourceId"], json!(source_id));
        assert_eq!(sources["sources"][0]["body"], json!(source_body()));

        let generated = restarted_app
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate after restart");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_json(generated).await;
        assert_eq!(
            generated["drafts"][0]["prompt"],
            json!("What is the NATO phonetic alphabet word for A?")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn postgres_backend_routes_drive_source_to_review() {
        let Some(database) = PostgresTestDatabase::new("routes") else {
            return;
        };
        let app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let account = create_account(&app, "learner@example.com").await;
        let routed_app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let source =
            save_source(&routed_app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id").to_owned();

        let generated = routed_app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_json(generated).await;
        let draft_id = generated["drafts"][0]
            .as_object()
            .and_then(|draft| draft.get("id"))
            .and_then(Value::as_str)
            .expect("draft id");

        let approved = routed_app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/accounts/{}/drafts/{draft_id}/approve", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("approve");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_json(approved).await;
        let review_unit_id = approved["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id");

        let revealed = routed_app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/reveal",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("reveal");
        assert_eq!(revealed.status(), StatusCode::OK);
        let revealed = response_json(revealed).await;
        assert_eq!(revealed["current"]["expectedAnswer"], json!("ALFA"));

        let submitted = routed_app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": "postgres-api-submit-nato-a"
                }),
            ))
            .await
            .expect("submit");
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_json(submitted).await;
        assert_eq!(submitted["summary"]["attemptCount"], json!(1));
        assert_eq!(submitted["current"]["grade"]["verdict"], json!("correct"));

        assert_postgres_restart_resume_and_duplicate_submit(
            &database.scoped_url,
            &account,
            &source_id,
            review_unit_id,
        )
        .await;
    }

    #[tokio::test]
    async fn source_routes_reject_blank_source_material() {
        let app = router(ApiState::default());
        let account = create_account(&app, "learner@example.com").await;
        let response = app
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", account.account_id),
                &account.session_token,
                &json!({
                    "title": " ",
                    "body": "ALFA is the NATO code word for A."
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"], json!("Source title must not be blank."));
    }

    #[tokio::test]
    async fn source_generation_approval_and_review_are_account_scoped() {
        let app = router(ApiState::default());
        let first = create_account(&app, "first@example.com").await;
        let second = create_account(&app, "second@example.com").await;
        let source = save_source(&app, &first, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id");

        let generated = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    first.account_id
                ),
                &first.session_token,
            ))
            .await
            .expect("generate");

        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_json(generated).await;
        let drafts = generated["drafts"].as_array().expect("drafts");
        assert_eq!(drafts.len(), 2);
        assert_eq!(
            drafts[0]["prompt"],
            json!("What is the NATO phonetic alphabet word for A?")
        );
        assert_eq!(drafts[0]["validationStatus"], json!("accepted"));
        let draft_id = drafts[0]["id"].as_str().expect("draft id");

        let cross_approve = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/accounts/{}/drafts/{draft_id}/approve", first.account_id),
                &second.session_token,
            ))
            .await
            .expect("cross approve");
        assert_eq!(cross_approve.status(), StatusCode::FORBIDDEN);

        let approved = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/accounts/{}/drafts/{draft_id}/approve", first.account_id),
                &first.session_token,
            ))
            .await
            .expect("approve");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_json(approved).await;
        assert_eq!(approved["summary"]["approvedReviewUnitCount"], json!(1));
        let review_unit_id = approved["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id");
        assert_eq!(approved["current"]["expectedAnswer"], json!(null));

        let revealed = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/reveal",
                    first.account_id
                ),
                &first.session_token,
            ))
            .await
            .expect("reveal");
        assert_eq!(revealed.status(), StatusCode::OK);
        let revealed = response_json(revealed).await;
        assert_eq!(revealed["current"]["expectedAnswer"], json!("ALFA"));
        assert_eq!(revealed["summary"]["attemptCount"], json!(0));

        let submitted = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    first.account_id
                ),
                &first.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": "submit-first-nato-a"
                }),
            ))
            .await
            .expect("submit");
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_json(submitted).await;
        assert_eq!(submitted["summary"]["attemptCount"], json!(1));
        assert_eq!(submitted["current"]["grade"]["verdict"], json!("correct"));

        let cross_next = app
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/review/next", first.account_id),
                &second.session_token,
            ))
            .await
            .expect("cross next");
        assert_eq!(cross_next.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn duplicate_review_submit_does_not_double_count_attempts() {
        let app = router(ApiState::default());
        let account = create_account(&app, "learner@example.com").await;
        let source = save_source(&app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id");
        let generated = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate");
        let generated = response_json(generated).await;
        let draft_id = generated["drafts"][0]["id"].as_str().expect("draft id");
        let approved = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/accounts/{}/drafts/{draft_id}/approve", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("approve");
        let approved = response_json(approved).await;
        let review_unit_id = approved["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id");

        let first = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": "submit-nato-a-once"
                }),
            ))
            .await
            .expect("first submit");
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        assert_eq!(first["summary"]["attemptCount"], json!(1));

        let duplicate = app
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": "submit-nato-a-once"
                }),
            ))
            .await
            .expect("duplicate submit");

        assert_eq!(duplicate.status(), StatusCode::OK);
        let duplicate = response_json(duplicate).await;
        assert_eq!(duplicate["summary"]["attemptCount"], json!(1));
        assert_eq!(
            duplicate["current"]["reviewState"],
            first["current"]["reviewState"]
        );
    }

    #[tokio::test]
    async fn review_submit_requires_an_idempotency_key() {
        let app = router(ApiState::default());
        let account = create_account(&app, "learner@example.com").await;
        let source = save_source(&app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id");
        let generated = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate");
        let generated = response_json(generated).await;
        let draft_id = generated["drafts"][0]["id"].as_str().expect("draft id");
        let approved = app
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/accounts/{}/drafts/{draft_id}/approve", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("approve");
        let approved = response_json(approved).await;
        let review_unit_id = approved["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id");

        let rejected = app
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": " "
                }),
            ))
            .await
            .expect("submit");

        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let rejected = response_json(rejected).await;
        assert_eq!(
            rejected["error"],
            json!("Idempotency key must not be blank.")
        );
    }

    struct TestAccount {
        account_id: String,
        session_token: String,
    }

    async fn create_account(app: &axum::Router, email: &str) -> TestAccount {
        let response = app
            .clone()
            .oneshot(account_request(email))
            .await
            .expect("account response");
        assert_eq!(response.status(), StatusCode::CREATED);

        let body = response_json(response).await;
        TestAccount {
            account_id: body["accountId"].as_str().expect("account id").to_owned(),
            session_token: body["sessionToken"]
                .as_str()
                .expect("session token")
                .to_owned(),
        }
    }

    fn account_request(email: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/accounts")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "email": email }).to_string()))
            .expect("request")
    }

    fn json_request(method: &str, uri: &str, session_token: &str, body: &Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("x-session-token", session_token)
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    fn empty_request(method: &str, uri: &str, session_token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("x-session-token", session_token)
            .body(Body::empty())
            .expect("request")
    }

    fn form_request(method: &str, uri: &str, fields: &[(&str, &str)]) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form_body(fields)))
            .expect("request")
    }

    fn form_body(fields: &[(&str, &str)]) -> String {
        fields
            .iter()
            .map(|(name, value)| format!("{}={}", form_escape(name), form_escape(value)))
            .collect::<Vec<_>>()
            .join("&")
    }

    fn form_escape(value: &str) -> String {
        value
            .bytes()
            .flat_map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    vec![char::from(byte)]
                }
                b' ' => vec!['+'],
                _ => {
                    let encoded = format!("%{byte:02X}");
                    encoded.chars().collect()
                }
            })
            .collect()
    }

    fn html_value(html: &str, name: &str) -> String {
        let marker = format!(r#"name="{name}" value=""#);
        let start = html.find(&marker).expect("field marker") + marker.len();
        let end = html[start..].find('"').expect("field end") + start;
        html[start..end].to_owned()
    }

    async fn save_source(
        app: &axum::Router,
        account: &TestAccount,
        title: &str,
        body: &str,
    ) -> Value {
        let response = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", account.account_id),
                &account.session_token,
                &json!({
                    "title": title,
                    "body": body
                }),
            ))
            .await
            .expect("source response");
        assert_eq!(response.status(), StatusCode::CREATED);

        response_json(response).await
    }

    async fn assert_postgres_restart_resume_and_duplicate_submit(
        database_url: &str,
        original: &TestAccount,
        source_id: &str,
        review_unit_id: &str,
    ) {
        let restarted_app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database_url.to_owned(),
        )));
        let replay = restarted_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":" LEARNER@example.com "}"#))
                    .expect("request"),
            )
            .await
            .expect("email replay after postgres restart");
        assert_eq!(replay.status(), StatusCode::CONFLICT);

        let duplicate = restarted_app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!(
                    "/accounts/{}/review/{review_unit_id}/submit",
                    original.account_id
                ),
                &original.session_token,
                &json!({
                    "answer": "ALFA",
                    "responseTimeMs": 1800,
                    "idempotencyKey": "postgres-api-submit-nato-a"
                }),
            ))
            .await
            .expect("duplicate submit after restart");
        assert_eq!(duplicate.status(), StatusCode::OK);
        let duplicate = response_json(duplicate).await;
        assert_eq!(duplicate["summary"]["attemptCount"], json!(1));
        assert_eq!(duplicate["summary"]["lastOutcome"], json!("correct"));

        let sources = restarted_app
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", original.account_id),
                &original.session_token,
            ))
            .await
            .expect("resumed sources");
        assert_eq!(sources.status(), StatusCode::OK);
        let sources = response_json(sources).await;
        assert_eq!(sources["sources"][0]["sourceId"], json!(source_id));
        assert_eq!(sources["sources"][0]["body"], json!(source_body()));
    }

    fn source_body() -> String {
        [
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for A?",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
            "",
            "Concept: NATO CAT composition",
            "Activity: exercise",
            "Stage: composition",
            "Question: Spell CAT over the phone using the NATO phonetic alphabet.",
            "Answer: CHARLIE ALFA TANGO",
            "Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.",
            "Reference: C is CHARLIE. A is ALFA. T is TANGO.",
        ]
        .join("\n")
    }

    fn temp_store_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "memory-engine-api-{name}-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    struct PostgresTestDatabase {
        admin_url: String,
        schema: String,
        scoped_url: String,
    }

    impl PostgresTestDatabase {
        fn new(name: &str) -> Option<Self> {
            let admin_url = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok()?;
            let schema = format!(
                "memory_engine_api_{name}_{}_{}",
                std::process::id(),
                rand::random::<u64>()
            );
            tokio::task::block_in_place(|| {
                let mut client = Client::connect(&admin_url, NoTls).expect("postgres test connect");
                client
                    .batch_execute(&format!("CREATE SCHEMA {schema}"))
                    .expect("create postgres test schema");
            });
            let separator = if admin_url.contains('?') { '&' } else { '?' };
            let scoped_url = format!("{admin_url}{separator}options=-csearch_path%3D{schema}");

            Some(Self {
                admin_url,
                schema,
                scoped_url,
            })
        }
    }

    impl Drop for PostgresTestDatabase {
        fn drop(&mut self) {
            let drop_schema = || {
                if let Ok(mut client) = Client::connect(&self.admin_url, NoTls) {
                    let _ = client
                        .batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema));
                }
            };
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::task::block_in_place(drop_schema);
            } else {
                drop_schema();
            }
        }
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    async fn response_text(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }
}
