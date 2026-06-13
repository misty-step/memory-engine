//! Production HTTP API boundary for the mobile study app.
//!
//! This crate owns transport-facing account/session behavior. It intentionally
//! stays outside `memory-engine-core`, which remains pure learning semantics.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use axum::{
    http::{
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Response},
    Json,
};
use memory_engine_generation::{FallbackProvider, StructuredBlockProvider};
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider};
use memory_engine_persistence::BetaPersistenceStore;
use memory_engine_persistence_postgres::{AccountScope, AccountStudyStore, PostgresStudyStore};
use memory_engine_study::{
    BetaStudyConceptProgress, BetaStudyCurrent, BetaStudyDraftRow, BetaStudyOptions,
    BetaStudySession, BetaStudySummary, BetaStudyView,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod registry;
mod render;
mod routes;
mod storage;

#[cfg(test)]
use render::render_generation_notices;
use render::{
    render_account_page, render_action_result_html, render_app_shell, render_login_requested,
};
pub use routes::router;
use storage::{StudyStorage, StudyStorageConfig};

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthConfig {
    allowed_emails: Option<BTreeSet<String>>,
    expose_debug_links: bool,
    link_delivery: AuthLinkDelivery,
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

    fn email_allowed(&self, email: &str) -> bool {
        self.allowed_emails
            .as_ref()
            .is_none_or(|allowed| allowed.contains(email))
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
                storage: StudyStorageConfig::file(store_root),
                ..AccountRegistryData::default()
            })),
        }
    }

    #[must_use]
    pub fn with_postgres_url(database_url: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AccountRegistryData {
                storage: StudyStorageConfig::postgres(database_url),
                ..AccountRegistryData::default()
            })),
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

    fn clock(&self) -> fn() -> i64 {
        self.lock_data().now_fn
    }

    fn now(&self) -> i64 {
        (self.clock())()
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
struct MagicLinkRequest {
    debug_link: Option<String>,
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
struct AppAccount {
    browser_session_id: String,
    account_id: String,
    session_token: String,
    csrf_token: String,
}

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

    fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.to_owned(),
        }
    }

    fn too_many_requests(message: &'static str) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.to_owned(),
        }
    }

    fn internal(message: String) -> Self {
        report_internal_error(&message);
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
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

fn client_rate_limit_key(headers: &HeaderMap) -> String {
    ["fly-client-ip", "x-real-ip", "x-forwarded-for"]
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

fn csrf_token(value: Option<&String>) -> &str {
    value.map(String::as_str).map(str::trim).unwrap_or_default()
}

fn html_with_browser_session(account: &AppAccount, html: String) -> Response {
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie_header(&account.browser_session_id))
            .expect("session cookie header"),
    );
    response
}

fn html_with_cleared_browser_session(html: String) -> Response {
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session_cookie_header()).expect("clear cookie header"),
    );
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
        write!(encoded, "{byte:02x}").expect("hash encoding");
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

fn new_csrf_token() -> String {
    format!("csrf_{:032x}", rand::random::<u128>())
}

fn new_magic_link_token() -> String {
    format!("magic_{:032x}", rand::random::<u128>())
}

const APP_SESSION_COOKIE_NAME: &str = "__Host-memory_engine_session";
const APP_SESSION_MAX_AGE_SECONDS: u64 = 60 * 60 * 24 * 14;
const APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS: u32 = 5;
const APP_ACCOUNT_RATE_LIMIT_WINDOW_MS: i64 = 15 * 60 * 1_000;
// 30 minutes: links travel through email, where spam checks and device
// switches routinely burn ten minutes. Found in dogfood: a link expired
// before the operator could click it.
const AUTH_CHALLENGE_TTL_MS: i64 = 30 * 60 * 1_000;

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

fn write_atomic(path: &FsPath, contents: &str) -> Result<(), ApiFailure> {
    let temp_path = path.with_extension(format!("tmp-{:032x}", rand::random::<u128>()));
    fs::write(&temp_path, contents).map_err(|error| ApiFailure::internal(error.to_string()))?;
    fs::rename(&temp_path, path).map_err(|error| ApiFailure::internal(error.to_string()))
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

fn app_session_max_age_ms() -> i64 {
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
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
    };

    use axum::{
        body::{to_bytes, Body},
        http::{header::SET_COOKIE, Request, StatusCode},
    };
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use std::sync::atomic::{AtomicI64, Ordering};

    use memory_engine_study::DEFAULT_BETA_STUDY_NOW;

    use super::{
        render_generation_notices, router, routes, AccountRegistry, ApiState, AuthConfig,
        AUTH_CHALLENGE_TTL_MS,
    };

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
        assert!(body.contains("Add something you want to learn"));
        assert!(body.contains("Save capture"));
        assert!(body.contains(r#"placeholder="Paste anything worth remembering.""#));
        assert!(body.contains(r#"name="capture""#));
        assert!(!body.contains(r#"name="title""#));
        assert!(!body.contains(r#"name="body""#));
        assert!(body.contains(r#"<form action="/app/account" method="post">"#));
        assert!(!body.contains("NATO practice notes"));
        assert!(!body.contains("Concept: NATO letter A"));
        assert!(
            body.find("/app/start").expect("start form")
                < body.find("/app/account").expect("account form")
        );
    }

    #[test]
    fn generation_notices_render_as_a_visible_section() {
        let html = render_generation_notices(&[
            "No review items could be generated from this source yet.".to_owned(),
        ]);
        assert!(html.contains("Generation notes"));
        assert!(html.contains("No review items could be generated"));

        assert!(
            render_generation_notices(&[]).is_empty(),
            "a clean run renders nothing"
        );
    }

    #[test]
    fn generation_notices_escape_html() {
        let html = render_generation_notices(&["<script>alert(1)</script>".to_owned()]);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[tokio::test]
    async fn mobile_form_flow_generates_keeps_reveals_and_submits_review() {
        let app = router(ApiState::default());
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("capture", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        assert!(started.contains("Capture saved"));
        assert!(started.contains("Saved material"));
        assert!(!started.contains("Choose what to keep"));

        let generated = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                &cookie,
                &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("generate");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_text(generated).await;
        assert_keep_flow_html(&generated);
        let draft_id = html_value(&generated, "draftId");

        let approved = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("approve");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        assert_due_review_html(&approved);
        let review_unit_id = html_value(&approved, "reviewUnitId");

        let revealed = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/reveal",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
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
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
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
        assert_submitted_review_html(&submitted);

        let next = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("next");
        assert_eq!(next.status(), StatusCode::OK);
        let next = response_text(next).await;
        assert!(next.contains("0 due"));
        assert!(next.contains("Add something you want to learn"));
        assert!(!next.contains("Progress"));
    }

    #[tokio::test]
    async fn mobile_submit_review_shows_human_result_and_item_history() {
        let app = router(ApiState::default());
        let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
        let generated = generate_source_html(&app, &cookie, &csrf_token, &source_id).await;
        let draft_id = html_value(&generated, "draftId");
        let approved = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("approve");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        let review_unit_id = html_value(&approved, "reviewUnitId");

        let submitted = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("reviewUnitId", &review_unit_id),
                    ("answer", "BRAVO"),
                    ("responseTimeMs", "1800"),
                    ("idempotencyKey", "mobile-feedback-nato-a"),
                ],
            ))
            .await
            .expect("submit");
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_text(submitted).await;

        assert!(submitted.contains("Try again"));
        assert!(submitted.contains("Expected answer"));
        assert!(submitted.contains("ALFA"));
        assert!(submitted.contains("This item: 1 attempt"));
        assert!(submitted.contains("0 of 1 correct (0.0%)"));
        assert!(submitted.contains("last seen just now"));
        assert!(submitted.contains("nato letter a"));
        assert_not_contains_any(
            &submitted,
            &[
                "Wrong(",
                "reviewState",
                "scheduleChange",
                "Generated material",
                "validation",
            ],
        );
    }

    #[tokio::test]
    async fn mobile_submit_review_shows_concept_rollup_for_shared_concept() {
        let app = router(ApiState::default());
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("capture", &shared_concept_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        let generated = generate_source_html(&app, &cookie, &csrf_token, &source_id).await;
        let draft_ids = html_values(&generated, "draftId");
        assert_eq!(draft_ids.len(), 2);

        for draft_id in &draft_ids {
            let approved = app
                .clone()
                .oneshot(form_request_with_cookie(
                    "POST",
                    "/app/approve",
                    &cookie,
                    &[("csrfToken", &csrf_token), ("draftId", draft_id)],
                ))
                .await
                .expect("approve");
            assert_eq!(approved.status(), StatusCode::OK);
        }
        let current = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("current");
        assert_eq!(current.status(), StatusCode::OK);
        let current = response_text(current).await;
        let first_id = html_value(&current, "reviewUnitId");

        let first = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("reviewUnitId", &first_id),
                    ("answer", "ALFA"),
                    ("responseTimeMs", "1800"),
                    ("idempotencyKey", "shared-concept-first"),
                ],
            ))
            .await
            .expect("first submit");
        assert_eq!(first.status(), StatusCode::OK);
        let next = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("next");
        assert_eq!(next.status(), StatusCode::OK);
        let next = response_text(next).await;
        let second_id = html_value(&next, "reviewUnitId");

        let submitted = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("reviewUnitId", &second_id),
                    ("answer", "BRAVO"),
                    ("responseTimeMs", "1800"),
                    ("idempotencyKey", "shared-concept-second"),
                ],
            ))
            .await
            .expect("second submit");
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_text(submitted).await;

        assert!(submitted.contains("nato letter a"));
        assert!(submitted.contains("1 of 2 correct (50.0%)"));
        assert!(submitted.contains("trend is declining"));
    }

    #[tokio::test]
    async fn management_surface_lists_concepts_worst_first() {
        let app = router(ApiState::default());
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("capture", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        let generated = generate_source_html(&app, &cookie, &csrf_token, &source_id).await;
        let draft_ids = html_values(&generated, "draftId");
        assert_eq!(draft_ids.len(), 2);

        approve_drafts_html(&app, &cookie, &csrf_token, &draft_ids).await;
        let current = next_review_html(&app, &cookie, &csrf_token, "current").await;
        submit_review_from_html(&app, &cookie, &csrf_token, &current, "management-first").await;
        let next = next_review_html(&app, &cookie, &csrf_token, "next").await;
        submit_review_from_html(&app, &cookie, &csrf_token, &next, "management-second").await;
        let workspace = next_review_html(&app, &cookie, &csrf_token, "workspace").await;

        assert!(workspace.contains("Concept health"));
        let weak = workspace
            .find("<strong>nato letter a</strong>")
            .expect("weak concept");
        let strong = workspace
            .find("<strong>nato cat composition</strong>")
            .expect("strong concept");
        assert!(weak < strong, "{workspace}");
        assert!(workspace.contains("struggling"));
        assert!(!workspace.contains("Choose what to keep"));
        assert_not_contains_any(&workspace, &["chart", "streak", "badge"]);
    }

    #[tokio::test]
    async fn auth_rendered_forms_do_not_expose_session_credentials() {
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
        let cookie = session_cookie(&started);
        let started = response_text(started).await;

        assert!(
            !started.contains(r#"name="accountId""#),
            "rendered app forms must not expose account ids as credentials"
        );
        assert!(
            !started.contains(r#"name="sessionToken""#),
            "rendered app forms must not expose session tokens"
        );
        assert!(started.contains(r#"name="csrfToken""#));

        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        let generated = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                &cookie,
                &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("generate");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_text(generated).await;
        let draft_id = html_value(&generated, "draftId");
        let rejected = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("draftId", &draft_id)],
            ))
            .await
            .expect("approve without csrf");
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

        let approved = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("approve with csrf");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        assert!(!approved.contains(r#"name="sessionToken""#));
        assert!(!approved.contains("acct_"));
        assert!(approved.contains("Reveal answer"));
    }

    #[tokio::test]
    async fn review_escape_hatches_render_and_drive_the_mobile_queue() {
        let app = router(ApiState::default());
        let started = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("capture", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        let generated = generate_source_html(&app, &cookie, &csrf_token, &source_id).await;
        let draft_ids = html_values(&generated, "draftId");
        let exercise_draft_id = draft_ids
            .iter()
            .find(|id| id.contains("nato-cat-composition"))
            .expect("exercise draft");
        let approved = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", exercise_draft_id)],
            ))
            .await
            .expect("approve exercise");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        assert!(approved.contains("1 due"));
        assert!(approved.contains("Reveal answer"));
        assert!(approved.contains("Spell CAT over the phone"));
        assert!(approved.contains("Reference"));
        assert!(approved.contains("Skip"));
        assert!(approved.contains("Snooze"));
        assert!(approved.contains("Bridge"));
        let parent_id = html_value(&approved, "reviewUnitId");

        let referenced = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/reference",
                &cookie,
                &[("csrfToken", &csrf_token), ("reviewUnitId", &parent_id)],
            ))
            .await
            .expect("reference");
        assert_eq!(referenced.status(), StatusCode::OK);
        let referenced = response_text(referenced).await;
        assert!(referenced.contains("Reference"));
        assert!(referenced.contains("C is CHARLIE"));

        let bridged = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/bridge",
                &cookie,
                &[("csrfToken", &csrf_token), ("reviewUnitId", &parent_id)],
            ))
            .await
            .expect("bridge");
        assert_eq!(bridged.status(), StatusCode::OK);
        let bridged = response_text(bridged).await;
        let bridge_id = html_value(&bridged, "reviewUnitId");
        assert!(bridge_id.starts_with("bridge-"));

        let skipped = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/skip",
                &cookie,
                &[("csrfToken", &csrf_token), ("reviewUnitId", &bridge_id)],
            ))
            .await
            .expect("skip");
        assert_eq!(skipped.status(), StatusCode::OK);
        let skipped = response_text(skipped).await;
        let next_bridge_id = html_value(&skipped, "reviewUnitId");
        assert!(next_bridge_id.starts_with("bridge-"));
        assert_ne!(next_bridge_id, bridge_id);

        let snoozed = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/snooze",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("reviewUnitId", &next_bridge_id),
                ],
            ))
            .await
            .expect("snooze");
        assert_eq!(snoozed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn app_session_mutations_require_csrf() {
        let app = router(ApiState::default());
        let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

        assert_source_session_mutations_require_csrf(&app, &cookie, &source_id).await;
        let review_unit_id = approve_review_for_csrf(&app, &cookie, &csrf_token, &source_id).await;
        assert_review_mutations_require_csrf(&app, &cookie, &review_unit_id).await;
    }

    async fn start_app_session_for_csrf(app: &axum::Router) -> (String, String, String) {
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
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");

        (cookie, csrf_token, source_id)
    }

    async fn assert_source_session_mutations_require_csrf(
        app: &axum::Router,
        cookie: &str,
        source_id: &str,
    ) {
        assert_forbidden_form(
            app,
            cookie,
            "/app/source",
            &[
                ("title", "Latin notes"),
                ("body", "Poena means punishment."),
            ],
            "source without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/source/archive",
            &[("sourceId", source_id)],
            "archive without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/generate",
            &[("sourceId", source_id)],
            "generate without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/approve",
            &[("draftId", "draft-withheld")],
            "approve without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/save-account",
            &[("email", "learner@example.com")],
            "save without csrf",
        )
        .await;
        assert_forbidden_form(app, cookie, "/app/logout", &[], "logout without csrf").await;
    }

    async fn approve_review_for_csrf(
        app: &axum::Router,
        cookie: &str,
        csrf_token: &str,
        source_id: &str,
    ) -> String {
        let generated = generate_source_html(app, cookie, csrf_token, source_id).await;
        let draft_id = html_value(&generated, "draftId");

        let approved = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                cookie,
                &[("csrfToken", csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("approve with csrf");
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        html_value(&approved, "reviewUnitId")
    }

    async fn assert_review_mutations_require_csrf(
        app: &axum::Router,
        cookie: &str,
        review_unit_id: &str,
    ) {
        assert_forbidden_form(app, cookie, "/app/next", &[], "next without csrf").await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/reveal",
            &[("reviewUnitId", review_unit_id)],
            "reveal without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/reference",
            &[("reviewUnitId", review_unit_id)],
            "reference without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/skip",
            &[("reviewUnitId", review_unit_id)],
            "skip without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/snooze",
            &[("reviewUnitId", review_unit_id)],
            "snooze without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/bridge",
            &[("reviewUnitId", review_unit_id)],
            "bridge without csrf",
        )
        .await;
        assert_forbidden_form(
            app,
            cookie,
            "/app/submit",
            &[
                ("reviewUnitId", review_unit_id),
                ("answer", "ALFA"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "csrf-matrix-submit"),
            ],
            "submit without csrf",
        )
        .await;
    }

    async fn generate_source_html(
        app: &axum::Router,
        cookie: &str,
        csrf_token: &str,
        source_id: &str,
    ) -> String {
        let generated = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                cookie,
                &[("csrfToken", csrf_token), ("sourceId", source_id)],
            ))
            .await
            .expect("generate with csrf");
        assert_eq!(generated.status(), StatusCode::OK);
        response_text(generated).await
    }

    async fn approve_drafts_html(
        app: &axum::Router,
        cookie: &str,
        csrf_token: &str,
        draft_ids: &[String],
    ) {
        for draft_id in draft_ids {
            let approved = app
                .clone()
                .oneshot(form_request_with_cookie(
                    "POST",
                    "/app/approve",
                    cookie,
                    &[("csrfToken", csrf_token), ("draftId", draft_id)],
                ))
                .await
                .expect("approve");
            assert_eq!(approved.status(), StatusCode::OK);
        }
    }

    async fn next_review_html(
        app: &axum::Router,
        cookie: &str,
        csrf_token: &str,
        context: &str,
    ) -> String {
        let response = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                cookie,
                &[("csrfToken", csrf_token)],
            ))
            .await
            .unwrap_or_else(|error| panic!("{context}: {error}"));
        assert_eq!(response.status(), StatusCode::OK, "{context}");
        response_text(response).await
    }

    async fn submit_review_from_html(
        app: &axum::Router,
        cookie: &str,
        csrf_token: &str,
        body: &str,
        idempotency_key: &str,
    ) {
        let review_unit_id = html_value(body, "reviewUnitId");
        let answer = management_answer_for_prompt(body);
        let submitted = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                cookie,
                &[
                    ("csrfToken", csrf_token),
                    ("reviewUnitId", &review_unit_id),
                    ("answer", answer),
                    ("responseTimeMs", "1800"),
                    ("idempotencyKey", idempotency_key),
                ],
            ))
            .await
            .expect("submit review");
        assert_eq!(submitted.status(), StatusCode::OK);
    }

    async fn assert_forbidden_form(
        app: &axum::Router,
        cookie: &str,
        uri: &str,
        fields: &[(&str, &str)],
        context: &str,
    ) {
        let response = app
            .clone()
            .oneshot(form_request_with_cookie("POST", uri, cookie, fields))
            .await
            .unwrap_or_else(|error| panic!("{context}: {error}"));
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{context}");
    }

    #[tokio::test]
    async fn auth_magic_link_cross_device_resume() {
        let store_root = temp_store_root("magic-link-resume");
        let app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root)
                .with_auth_config(AuthConfig::default().with_debug_links(true)),
        ));
        let account = create_account(&app, "learner@example.com").await;
        save_source(&app, &account, "NATO practice notes", &source_body()).await;

        let requested = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", " Learner@Example.COM ")],
            ))
            .await
            .expect("request magic link");
        assert_eq!(requested.status(), StatusCode::OK);
        let requested = response_text(requested).await;
        assert!(requested.contains("Check your email"));
        assert!(!requested.contains(r#"name="sessionToken""#));
        let verify_path = debug_sign_in_path(&requested);

        let restarted_app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root)
                .with_auth_config(AuthConfig::default().with_debug_links(true)),
        ));
        let verified = restarted_app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&verify_path)
                    .body(Body::empty())
                    .expect("verify request"),
            )
            .await
            .expect("verify magic link");
        assert_eq!(verified.status(), StatusCode::OK);
        let cookie = session_cookie(&verified);
        let verified = response_text(verified).await;
        assert!(verified.contains("NATO practice notes"));
        assert!(!verified.contains("acct_fc9e1ff15d47bd67"));
        assert!(!verified.contains("Save account email"));
        assert!(!verified.contains(r#"name="email""#));
        assert!(!verified.contains(r#"name="sessionToken""#));
        assert!(cookie.starts_with("__Host-memory_engine_session="));
    }

    #[tokio::test]
    async fn auth_rejects_magic_link_replay() {
        let app = router(ApiState::new(
            AccountRegistry::default()
                .with_auth_config(AuthConfig::default().with_debug_links(true)),
        ));
        let requested = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", "learner@example.com")],
            ))
            .await
            .expect("request magic link");
        let verify_path = debug_sign_in_path(&response_text(requested).await);

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&verify_path)
                    .body(Body::empty())
                    .expect("first verify request"),
            )
            .await
            .expect("first verify");
        assert_eq!(first.status(), StatusCode::OK);

        let replay = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&verify_path)
                    .body(Body::empty())
                    .expect("replay verify request"),
            )
            .await
            .expect("replay verify");
        assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn auth_magic_link_writes_configured_outbox() {
        let store_root = temp_store_root("magic-link-outbox");
        let outbox_path = store_root.join("auth-outbox.tsv");
        let app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root).with_auth_config(
                AuthConfig::allow_emails(vec!["owner@example.com".to_owned()])
                    .with_link_outbox(&outbox_path),
            ),
        ));

        let requested = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", "owner@example.com")],
            ))
            .await
            .expect("request owner magic link");
        assert_eq!(requested.status(), StatusCode::OK);
        let requested = response_text(requested).await;
        assert!(requested.contains("Check your email"));
        assert!(
            !requested.contains("Debug sign-in link"),
            "production delivery must not reveal the login link in HTML"
        );

        let outbox = fs::read_to_string(outbox_path).expect("outbox");
        assert!(outbox.starts_with("owner@example.com\t/app/login/verify?token="));
        let verify_path = outbox.trim().split('\t').nth(1).expect("outbox link");

        let verified = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(verify_path)
                    .body(Body::empty())
                    .expect("verify request"),
            )
            .await
            .expect("verify owner magic link");
        assert_eq!(verified.status(), StatusCode::OK);
        assert!(session_cookie(&verified).starts_with("__Host-memory_engine_session="));
    }

    #[tokio::test]
    async fn auth_login_request_does_not_enumerate_emails() {
        let store_root = temp_store_root("login-enumeration");
        let outbox_path = store_root.join("auth-outbox.tsv");
        let app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root).with_auth_config(
                AuthConfig::allow_emails(vec!["owner@example.com".to_owned()])
                    .with_link_outbox(&outbox_path),
            ),
        ));

        let owner = app
            .clone()
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", "owner@example.com")],
            ))
            .await
            .expect("owner login request");
        assert_eq!(owner.status(), StatusCode::OK);
        let owner = response_text(owner).await;

        let stranger = app
            .oneshot(form_request(
                "POST",
                "/app/account",
                &[("email", "stranger@example.com")],
            ))
            .await
            .expect("stranger login request");
        assert_eq!(stranger.status(), StatusCode::OK);
        let stranger = response_text(stranger).await;

        assert_eq!(owner, stranger);
        assert!(owner.contains("If that address can sign in, a link is on the way."));
        assert!(!owner.contains("owner@example.com"));
        assert!(!owner.contains("stranger@example.com"));
        let outbox = fs::read_to_string(outbox_path).expect("outbox");
        assert!(outbox.contains("owner@example.com"));
        assert!(!outbox.contains("stranger@example.com"));
    }

    #[tokio::test]
    async fn app_account_rate_limits_by_email_and_ip() {
        let store_root = temp_store_root("login-rate-limit");
        let outbox_path = store_root.join("auth-outbox.tsv");
        let app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root).with_auth_config(
                AuthConfig::allow_emails(vec![
                    "owner@example.com".to_owned(),
                    "other@example.com".to_owned(),
                ])
                .with_link_outbox(&outbox_path),
            ),
        ));

        for _ in 0..super::APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS {
            let response = app
                .clone()
                .oneshot(form_request_with_ip(
                    "POST",
                    "/app/account",
                    "203.0.113.10",
                    &[("email", "owner@example.com")],
                ))
                .await
                .expect("allowed request");
            assert_eq!(response.status(), StatusCode::OK);
        }

        let same_email_new_ip = app
            .clone()
            .oneshot(form_request_with_ip(
                "POST",
                "/app/account",
                "203.0.113.11",
                &[("email", "owner@example.com")],
            ))
            .await
            .expect("email limited");
        assert_eq!(same_email_new_ip.status(), StatusCode::TOO_MANY_REQUESTS);

        let same_ip_new_email = app
            .oneshot(form_request_with_ip(
                "POST",
                "/app/account",
                "203.0.113.10",
                &[("email", "other@example.com")],
            ))
            .await
            .expect("ip limited");
        assert_eq!(same_ip_new_email.status(), StatusCode::TOO_MANY_REQUESTS);

        let outbox = fs::read_to_string(outbox_path).expect("outbox");
        assert_eq!(outbox.lines().count(), 5);
        assert!(!outbox.contains("other@example.com"));
    }

    #[tokio::test]
    async fn app_account_rejected_ip_does_not_spend_email_quota() {
        let store_root = temp_store_root("login-rate-limit-poison");
        let outbox_path = store_root.join("auth-outbox.tsv");
        let app = router(ApiState::new(
            AccountRegistry::with_store_root(&store_root).with_auth_config(
                AuthConfig::allow_emails(vec![
                    "blocked@example.com".to_owned(),
                    "victim@example.com".to_owned(),
                ])
                .with_link_outbox(&outbox_path),
            ),
        ));

        for _ in 0..super::APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS {
            let response = app
                .clone()
                .oneshot(form_request_with_ip(
                    "POST",
                    "/app/account",
                    "203.0.113.20",
                    &[("email", "blocked@example.com")],
                ))
                .await
                .expect("spend ip bucket");
            assert_eq!(response.status(), StatusCode::OK);
        }

        for _ in 0..super::APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS {
            let rejected = app
                .clone()
                .oneshot(form_request_with_ip(
                    "POST",
                    "/app/account",
                    "203.0.113.20",
                    &[("email", "victim@example.com")],
                ))
                .await
                .expect("blocked ip victim attempt");
            assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        let clean_ip = app
            .oneshot(form_request_with_ip(
                "POST",
                "/app/account",
                "203.0.113.21",
                &[("email", "victim@example.com")],
            ))
            .await
            .expect("victim clean ip");
        assert_eq!(clean_ip.status(), StatusCode::OK);

        let outbox = fs::read_to_string(outbox_path).expect("outbox");
        assert_eq!(
            outbox
                .lines()
                .filter(|line| line.starts_with("victim@example.com\t"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn app_logout_revokes_the_browser_session() {
        let store_root = temp_store_root("logout-revokes");
        let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));
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
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        assert!(started.contains(r#"action="/app/logout""#));

        let logged_out = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/logout",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("logout");
        assert_eq!(logged_out.status(), StatusCode::OK);
        let set_cookie = logged_out
            .headers()
            .get(SET_COOKIE)
            .expect("clear cookie")
            .to_str()
            .expect("set-cookie");
        assert!(set_cookie.contains("__Host-memory_engine_session="));
        assert!(set_cookie.contains("Max-Age=0"));

        let restarted_app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));
        let rejected = restarted_app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("next after logout");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_logout_requires_csrf_without_revoking_session() {
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
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");

        let rejected = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/logout",
                &cookie,
                &[],
            ))
            .await
            .expect("logout without csrf");
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

        let still_active = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/next",
                &cookie,
                &[("csrfToken", &csrf_token)],
            ))
            .await
            .expect("next after failed logout");
        assert_eq!(still_active.status(), StatusCode::OK);
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
        let guest_cookie = session_cookie(&started);
        let started = response_text(started).await;
        let guest_csrf_token = html_value(&started, "csrfToken");

        let saved = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/save-account",
                &guest_cookie,
                &[
                    ("csrfToken", &guest_csrf_token),
                    ("email", " Learner@Example.COM "),
                ],
            ))
            .await
            .expect("save account");
        assert_eq!(saved.status(), StatusCode::OK);
        let saved_cookie = session_cookie(&saved);
        let saved = response_text(saved).await;
        assert!(saved.contains("NATO practice notes"));
        assert!(saved.contains("Saved material"));
        assert!(!saved.contains("Keep this"));
        assert!(!saved.contains("acct_fc9e1ff15d47bd67"));
        assert!(!saved.contains("Save account email"));
        let saved_csrf_token = html_value(&saved, "csrfToken");
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
        assert!(replay.contains("Check your email"));
        assert!(!replay.contains("Account already exists."));

        let generated = restarted_app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                &saved_cookie,
                &[("csrfToken", &saved_csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("generate after resume");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_text(generated).await;
        assert!(generated.contains("Choose what to keep"));
    }

    #[tokio::test]
    async fn mobile_source_archive_hides_source_and_blocks_regeneration() {
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
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");
        assert!(started.contains("NATO practice notes"));

        let archived = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/source/archive",
                &cookie,
                &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("archive source");
        assert_eq!(archived.status(), StatusCode::OK);
        let archived = response_text(archived).await;
        assert!(archived.contains("Source removed"));
        assert!(archived.contains("Add something you want to learn"));
        assert!(!archived.contains("NATO practice notes"));
        assert!(!archived.contains("What is the NATO phonetic alphabet word for A?"));

        let regenerated = app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                &cookie,
                &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("generate archived source");
        assert_eq!(regenerated.status(), StatusCode::OK);
        let regenerated = response_text(regenerated).await;
        assert!(regenerated.contains("Source not found."));
        assert!(!regenerated.contains("Choose what to keep"));
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
    async fn create_account_enforces_the_email_allowlist() {
        let state = ApiState::new(
            AccountRegistry::default()
                .with_auth_config(AuthConfig::allow_emails(["owner@example.com".to_owned()])),
        );
        let app = router(state);

        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"stranger@example.com"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let body = response_json(denied).await;
        assert!(body.get("sessionToken").is_none());

        let allowed = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"owner@example.com"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(allowed.status(), StatusCode::CREATED);
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

    #[tokio::test]
    async fn source_archive_hides_source_and_blocks_api_generation() {
        let app = router(ApiState::default());
        let account = create_account(&app, "learner@example.com").await;
        let source = save_source(&app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id").to_owned();

        let archived = app
            .clone()
            .oneshot(empty_request(
                "DELETE",
                &format!("/accounts/{}/sources/{source_id}", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("archive source");
        assert_eq!(archived.status(), StatusCode::NO_CONTENT);

        let sources = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("sources after archive");
        assert_eq!(sources.status(), StatusCode::OK);
        let sources = response_json(sources).await;
        assert_eq!(sources["sources"], json!([]));

        let generated = app
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate archived source");
        assert_eq!(generated.status(), StatusCode::NOT_FOUND);
        let generated = response_json(generated).await;
        assert_eq!(generated["error"], json!("Source not found."));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn postgres_backend_source_archive_hides_source_and_blocks_generation() {
        let Some(database) = PostgresTestDatabase::new("source_archive") else {
            return;
        };
        let app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let account = create_account(&app, "learner@example.com").await;
        let source = save_source(&app, &account, "NATO practice notes", &source_body()).await;
        let source_id = source["sourceId"].as_str().expect("source id").to_owned();

        let archived = app
            .clone()
            .oneshot(empty_request(
                "DELETE",
                &format!("/accounts/{}/sources/{source_id}", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("archive source");
        assert_eq!(archived.status(), StatusCode::NO_CONTENT);

        let sources = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("sources after archive");
        assert_eq!(sources.status(), StatusCode::OK);
        let sources = response_json(sources).await;
        assert_eq!(sources["sources"], json!([]));

        let generated = app
            .oneshot(empty_request(
                "POST",
                &format!(
                    "/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate archived source");
        assert_eq!(generated.status(), StatusCode::NOT_FOUND);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn postgres_backend_browser_session_resumes_after_restart() {
        let Some(database) = PostgresTestDatabase::new("browser_session") else {
            return;
        };
        let app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let started = app
            .oneshot(form_request(
                "POST",
                "/app/start",
                &[("title", "NATO practice notes"), ("body", &source_body())],
            ))
            .await
            .expect("start");
        assert_eq!(started.status(), StatusCode::OK);
        let cookie = session_cookie(&started);
        let started = response_text(started).await;
        assert!(!started.contains(r#"name="sessionToken""#));
        let csrf_token = html_value(&started, "csrfToken");
        let source_id = html_value(&started, "sourceId");

        let restarted_app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let generated = restarted_app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/generate",
                &cookie,
                &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
            ))
            .await
            .expect("generate after restart");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_text(generated).await;
        let draft_id = html_value(&generated, "draftId");

        let approved = restarted_app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/approve",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("approve after restart");

        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_text(approved).await;
        assert!(approved.contains("Reveal answer"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn postgres_backend_source_routes_are_account_scoped() {
        let Some(database) = PostgresTestDatabase::new("source_scope") else {
            return;
        };
        let app = router(ApiState::new(AccountRegistry::with_postgres_url(
            database.scoped_url.clone(),
        )));
        let first = create_account(&app, "first@example.com").await;
        let second = create_account(&app, "second@example.com").await;

        let first_source =
            save_source(&app, &first, "NATO notes", "ALFA is the code word for A.").await;
        let second_source =
            save_source(&app, &second, "Latin notes", "Poena means punishment.").await;

        let first_sources = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", first.account_id),
                &first.session_token,
            ))
            .await
            .expect("first sources");
        assert_eq!(first_sources.status(), StatusCode::OK);
        let first_sources = response_json(first_sources).await;
        assert_eq!(
            first_sources["sources"].as_array().expect("sources").len(),
            1
        );
        assert_eq!(
            first_sources["sources"][0]["sourceId"],
            first_source["sourceId"]
        );
        assert_ne!(
            first_sources["sources"][0]["sourceId"],
            second_source["sourceId"]
        );

        let second_sources = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", second.account_id),
                &second.session_token,
            ))
            .await
            .expect("second sources");
        assert_eq!(second_sources.status(), StatusCode::OK);
        let second_sources = response_json(second_sources).await;
        assert_eq!(
            second_sources["sources"].as_array().expect("sources").len(),
            1
        );
        assert_eq!(
            second_sources["sources"][0]["sourceId"],
            second_source["sourceId"]
        );

        let cross_read = app
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/accounts/{}/sources", second.account_id),
                &first.session_token,
            ))
            .await
            .expect("cross read");
        assert_eq!(cross_read.status(), StatusCode::FORBIDDEN);

        let cross_write = app
            .oneshot(json_request(
                "POST",
                &format!("/accounts/{}/sources", second.account_id),
                &first.session_token,
                &json!({
                    "title": "Wrong account",
                    "body": "This write must be rejected."
                }),
            ))
            .await
            .expect("cross write");
        assert_eq!(cross_write.status(), StatusCode::FORBIDDEN);
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
    async fn v1_json_api_drives_full_loop_with_bearer_token() {
        let app = router(ApiState::default());
        let account = create_account_v1(&app, "scry@example.com").await;
        let source_id =
            create_source_v1(&app, &account, "NATO practice notes", &source_body()).await;
        let draft_id = generate_source_v1(&app, &account, &source_id).await;
        let review_unit_id = approve_draft_v1(&app, &account, &draft_id).await;

        assert_eq!(
            next_review_v1(&app, &account).await,
            review_unit_id,
            "v1 queue/next must expose the approved review unit"
        );
        assert_eq!(
            reveal_review_v1(&app, &account, &review_unit_id).await,
            "ALFA"
        );
        assert_eq!(
            submit_review_v1(&app, &account, &review_unit_id, "ALFA").await,
            (String::from("correct"), 1)
        );

        archive_source_v1(&app, &account, &source_id).await;
    }

    #[tokio::test]
    async fn v1_json_api_returns_post_answer_feedback_and_concept_progress() {
        let app = router(ApiState::default());
        let account = create_account_v1(&app, "feedback@example.com").await;
        let source_id = create_source_v1(
            &app,
            &account,
            "NATO practice notes",
            &shared_concept_body(),
        )
        .await;
        let generated = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate source");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_json(generated).await;
        let draft_ids = generated["drafts"]
            .as_array()
            .expect("drafts")
            .iter()
            .map(|draft| draft["id"].as_str().expect("draft id").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(draft_ids.len(), 2);
        for draft_id in &draft_ids {
            approve_draft_v1(&app, &account, draft_id).await;
        }

        let first_id = next_review_v1(&app, &account).await;
        let _ =
            submit_review_v1_body(&app, &account, &first_id, "ALFA", "api-feedback-first").await;
        let second_id = next_review_v1(&app, &account).await;
        let submitted =
            submit_review_v1_body(&app, &account, &second_id, "BRAVO", "api-feedback-second").await;

        assert_eq!(
            submitted["current"]["feedback"]["verdict"],
            json!("Try again")
        );
        assert_eq!(
            submitted["current"]["feedback"]["expectedAnswer"],
            json!("ALFA")
        );
        assert_eq!(
            submitted["current"]["feedback"]["itemHistory"]["successRate"],
            json!("0 of 1 correct (0.0%)")
        );
        assert_eq!(
            submitted["current"]["feedback"]["itemHistory"]["lastResponseTimeMs"],
            json!(1800)
        );
        assert_eq!(
            submitted["current"]["feedback"]["itemHistory"]["averageResponseTimeMs"],
            json!(1800)
        );
        assert_eq!(
            submitted["current"]["feedback"]["itemHistory"]["responseTimeTrend"],
            json!("not enough data")
        );
        assert_eq!(
            submitted["current"]["feedback"]["itemHistory"]["lastSeenSummary"],
            json!("last seen just now")
        );
        assert_eq!(
            submitted["current"]["choices"]
                .as_array()
                .expect("choices")
                .len(),
            3
        );
        assert!(submitted["current"]["feedback"]["itemHistory"]["lastSeen"]
            .as_i64()
            .is_some());
        assert_eq!(
            submitted["conceptProgress"]
                .as_array()
                .expect("concepts")
                .len(),
            1
        );
        assert_eq!(
            submitted["conceptProgress"][0]["conceptKey"],
            json!("nato-letter-a")
        );
        assert_eq!(
            submitted["conceptProgress"][0]["successRate"],
            json!("1 of 2 correct (50.0%)")
        );
        assert_eq!(
            submitted["conceptProgress"][0]["averageResponseTimeMs"],
            json!(1800)
        );
    }

    #[tokio::test]
    async fn v1_json_api_exposes_review_escape_hatches() {
        let app = router(ApiState::default());
        let account = create_account_v1(&app, "bridge@example.com").await;
        let source_id =
            create_source_v1(&app, &account, "NATO practice notes", &source_body()).await;
        let generated = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate source");
        assert_eq!(generated.status(), StatusCode::OK);
        let generated = response_json(generated).await;
        let exercise_draft_id = generated["drafts"]
            .as_array()
            .expect("drafts")
            .iter()
            .find_map(|draft| {
                let id = draft["id"].as_str()?;
                id.contains("nato-cat-composition").then_some(id.to_owned())
            })
            .expect("exercise draft");
        let parent_id = approve_draft_v1(&app, &account, &exercise_draft_id);
        let parent_id = parent_id.await;

        let referenced = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{parent_id}/reference",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("reference");
        assert_eq!(referenced.status(), StatusCode::OK);
        let referenced = response_json(referenced).await;
        assert!(referenced["current"]["referenceText"]
            .as_str()
            .expect("reference text")
            .contains("C is CHARLIE"));

        let bridged = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{parent_id}/bridge",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("bridge");
        assert_eq!(bridged.status(), StatusCode::OK);
        let bridged = response_json(bridged).await;
        let bridge_id = bridged["current"]["reviewUnitId"]
            .as_str()
            .expect("bridge id");
        assert!(bridge_id.starts_with("bridge-"));
        assert_eq!(bridged["summary"]["attemptCount"], json!(0));

        let skipped = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{bridge_id}/skip",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("skip");
        assert_eq!(skipped.status(), StatusCode::OK);
        let skipped = response_json(skipped).await;
        let next_bridge_id = skipped["current"]["reviewUnitId"]
            .as_str()
            .expect("next bridge id");
        assert!(next_bridge_id.starts_with("bridge-"));
        assert_ne!(next_bridge_id, bridge_id);

        let snoozed = app
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{next_bridge_id}/snooze",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("snooze");
        assert_eq!(snoozed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v1_openapi_artifact_matches_registered_routes() {
        let response = router(ApiState::default())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/openapi.json")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("openapi response");
        assert_eq!(response.status(), StatusCode::OK);
        let contract = response_json(response).await;
        assert_eq!(contract["openapi"], json!("3.1.0"));
        assert_eq!(contract["info"]["version"], json!("1.0.0"));

        let paths = contract["paths"].as_object().expect("paths object");
        let actual = paths
            .iter()
            .flat_map(|(path, methods)| {
                methods
                    .as_object()
                    .expect("methods object")
                    .keys()
                    .map(move |method| (method.to_ascii_uppercase(), path.clone()))
            })
            .collect::<std::collections::BTreeSet<_>>();
        let expected = routes::v1_contract_operations()
            .iter()
            .map(|operation| (operation.method.to_owned(), operation.path.to_owned()))
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(actual, expected);
        assert_schema_requires(&contract, "StudyCurrent", &["choices"]);
        assert_schema_requires(
            &contract,
            "StudyItemHistory",
            &[
                "trend",
                "lastResponseTimeMs",
                "averageResponseTimeMs",
                "responseTimeTrend",
            ],
        );
        assert_schema_requires(
            &contract,
            "ConceptProgress",
            &["averageResponseTimeMs", "responseTimeTrend"],
        );
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

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_duplicate_review_submit_counts_one_attempt() {
        let app = router(ApiState::default());
        let account = create_account(&app, "learner@example.com").await;
        let review_unit_id = prepare_review_unit(&app, &account).await;
        let submit_uri = format!(
            "/accounts/{}/review/{review_unit_id}/submit",
            account.account_id
        );
        let barrier = Arc::new(Barrier::new(16));
        let mut workers = Vec::new();
        for _ in 0..16 {
            let app = app.clone();
            let barrier = Arc::clone(&barrier);
            let submit_uri = submit_uri.clone();
            let session_token = account.session_token.clone();
            workers.push(thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                barrier.wait();
                runtime.block_on(async move {
                    let response = app
                        .oneshot(json_request(
                            "POST",
                            &submit_uri,
                            &session_token,
                            &json!({
                                "answer": "ALFA",
                                "responseTimeMs": 1800,
                                "idempotencyKey": "concurrent-submit-nato-a"
                            }),
                        ))
                        .await
                        .expect("submit");
                    let status = response.status();
                    let body = response_json(response).await;

                    (status, body)
                })
            }));
        }

        for result in workers {
            let (status, body) = result.join().expect("worker");
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["summary"]["attemptCount"], json!(1), "{body}");
            assert_eq!(body["summary"]["lastOutcome"], json!("correct"), "{body}");
        }
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

    async fn create_account_v1(app: &axum::Router, email: &str) -> TestAccount {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/accounts")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "email": email }).to_string()))
                    .expect("request"),
            )
            .await
            .expect("v1 account response");
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

    async fn create_source_v1(
        app: &axum::Router,
        account: &TestAccount,
        title: &str,
        body: &str,
    ) -> String {
        let response = app
            .clone()
            .oneshot(v1_json_request(
                "POST",
                &format!("/v1/accounts/{}/sources", account.account_id),
                &account.session_token,
                &json!({ "title": title, "body": body }),
            ))
            .await
            .expect("create source");
        assert_eq!(response.status(), StatusCode::CREATED);

        response_json(response).await["sourceId"]
            .as_str()
            .expect("source id")
            .to_owned()
    }

    async fn generate_source_v1(
        app: &axum::Router,
        account: &TestAccount,
        source_id: &str,
    ) -> String {
        let response = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/sources/{source_id}/generate",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("generate source");
        assert_eq!(response.status(), StatusCode::OK);

        response_json(response).await["drafts"][0]["id"]
            .as_str()
            .expect("draft id")
            .to_owned()
    }

    async fn approve_draft_v1(app: &axum::Router, account: &TestAccount, draft_id: &str) -> String {
        let response = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/drafts/{draft_id}/approve",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("approve draft");
        assert_eq!(response.status(), StatusCode::OK);

        response_json(response).await["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id")
            .to_owned()
    }

    async fn next_review_v1(app: &axum::Router, account: &TestAccount) -> String {
        let response = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!("/v1/accounts/{}/review/next", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("next review");
        assert_eq!(response.status(), StatusCode::OK);

        response_json(response).await["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id")
            .to_owned()
    }

    async fn reveal_review_v1(
        app: &axum::Router,
        account: &TestAccount,
        review_unit_id: &str,
    ) -> String {
        let response = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{review_unit_id}/reveal",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("reveal");
        assert_eq!(response.status(), StatusCode::OK);

        response_json(response).await["current"]["expectedAnswer"]
            .as_str()
            .expect("expected answer")
            .to_owned()
    }

    async fn submit_review_v1(
        app: &axum::Router,
        account: &TestAccount,
        review_unit_id: &str,
        answer: &str,
    ) -> (String, u64) {
        let response = app
            .clone()
            .oneshot(v1_json_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": answer,
                    "responseTimeMs": 1800,
                    "idempotencyKey": "v1-scry-loop-nato-a"
                }),
            ))
            .await
            .expect("submit");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        (
            body["current"]["grade"]["verdict"]
                .as_str()
                .expect("verdict")
                .to_owned(),
            body["summary"]["attemptCount"]
                .as_u64()
                .expect("attempt count"),
        )
    }

    async fn submit_review_v1_body(
        app: &axum::Router,
        account: &TestAccount,
        review_unit_id: &str,
        answer: &str,
        idempotency_key: &str,
    ) -> Value {
        let response = app
            .clone()
            .oneshot(v1_json_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{review_unit_id}/submit",
                    account.account_id
                ),
                &account.session_token,
                &json!({
                    "answer": answer,
                    "responseTimeMs": 1800,
                    "idempotencyKey": idempotency_key
                }),
            ))
            .await
            .expect("submit");
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    async fn archive_source_v1(app: &axum::Router, account: &TestAccount, source_id: &str) {
        let response = app
            .clone()
            .oneshot(v1_empty_request(
                "DELETE",
                &format!("/v1/accounts/{}/sources/{source_id}", account.account_id),
                &account.session_token,
            ))
            .await
            .expect("archive source");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    fn v1_json_request(
        method: &str,
        uri: &str,
        session_token: &str,
        body: &Value,
    ) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {session_token}"))
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    fn v1_empty_request(method: &str, uri: &str, session_token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {session_token}"))
            .body(Body::empty())
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

    fn form_request_with_ip(
        method: &str,
        uri: &str,
        ip: &str,
        fields: &[(&str, &str)],
    ) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("fly-client-ip", ip)
            .body(Body::from(form_body(fields)))
            .expect("request")
    }

    fn form_request_with_cookie(
        method: &str,
        uri: &str,
        cookie: &str,
        fields: &[(&str, &str)],
    ) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", cookie)
            .body(Body::from(form_body(fields)))
            .expect("request")
    }

    fn session_cookie(response: &axum::response::Response) -> String {
        let set_cookie = response
            .headers()
            .get(SET_COOKIE)
            .expect("session set-cookie")
            .to_str()
            .expect("session cookie header");
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("Secure"));
        assert!(set_cookie.contains("SameSite=Lax"));
        assert!(set_cookie.contains("Path=/"));
        assert!(!set_cookie.contains("Domain="));
        set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned()
    }

    fn debug_sign_in_path(html: &str) -> String {
        let marker = r#"<a href=""#;
        let start = html.find(marker).expect("debug sign-in link") + marker.len();
        let end = html[start..].find('"').expect("debug sign-in link end") + start;
        html[start..end].replace("&amp;", "&")
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

    fn html_values(html: &str, name: &str) -> Vec<String> {
        let marker = format!(r#"name="{name}" value=""#);
        let mut values = Vec::new();
        let mut remaining = html;
        while let Some(index) = remaining.find(&marker) {
            let start = index + marker.len();
            let end = remaining[start..].find('"').expect("field end") + start;
            values.push(remaining[start..end].to_owned());
            remaining = &remaining[end..];
        }
        values
    }

    fn assert_keep_flow_html(body: &str) {
        assert!(body.contains("Choose what to keep"));
        assert!(body.contains("What is the NATO phonetic alphabet word for A?"));
        assert!(body.contains("Keep this"));
        assert_not_contains_any(
            body,
            &[
                "Generated material",
                "validation",
                "recognition-3",
                "activity_stage",
                "Save account email",
                "Session ready for",
                "acct_",
            ],
        );
    }

    fn assert_due_review_html(body: &str) {
        assert!(body.contains("1 due"));
        assert!(body.contains("Reveal answer"));
        assert!(body.contains("What is the NATO phonetic alphabet word for A?"));
        assert_not_contains_any(
            body,
            &[
                "Sources",
                "Generated material",
                "Choose what to keep",
                "Progress",
                "drafts",
                "attempts",
                "validation",
                "recognition-3",
                "Save account email",
                "Session ready for",
                "acct_",
            ],
        );
    }

    fn assert_submitted_review_html(body: &str) {
        assert!(body.contains("Correct"));
        assert!(body.contains("Answer feedback"));
        assert!(body.contains("Expected answer"));
        assert!(body.contains("This item: 1 attempt"));
        assert!(body.contains("1 of 1 correct (100.0%)"));
        assert!(body.contains("last seen just now"));
        assert!(body.contains("Concept health"));
        assert!(body.contains("Next"));
        assert_not_contains_any(
            body,
            &[
                "Last result",
                "Progress",
                "Correct(",
                "reviewState",
                "scheduleChange",
            ],
        );
    }

    fn management_answer_for_prompt(body: &str) -> &'static str {
        if body.contains("Spell CAT over the phone") {
            "CHARLIE ALFA TANGO"
        } else {
            "BRAVO"
        }
    }

    fn assert_not_contains_any(body: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                !body.contains(needle),
                "body unexpectedly contained {needle:?}"
            );
        }
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

    async fn prepare_review_unit(app: &axum::Router, account: &TestAccount) -> String {
        let source = save_source(app, account, "NATO practice notes", &source_body()).await;
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
        assert_eq!(generated.status(), StatusCode::OK);
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
        assert_eq!(approved.status(), StatusCode::OK);
        let approved = response_json(approved).await;

        approved["current"]["reviewUnitId"]
            .as_str()
            .expect("review unit id")
            .to_owned()
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

    fn shared_concept_body() -> String {
        [
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for A?",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
            "",
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: cued-recall",
            "Question: Type the code word used for the letter A.",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: A is represented by ALFA in the NATO phonetic alphabet.",
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
                let mut client = memory_engine_persistence_postgres::connect_client(&admin_url)
                    .expect("postgres test connect");
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
                if let Ok(mut client) =
                    memory_engine_persistence_postgres::connect_client(&self.admin_url)
                {
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

    fn assert_schema_requires(contract: &Value, schema_name: &str, fields: &[&str]) {
        let required = contract["components"]["schemas"][schema_name]["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{schema_name} required fields"));
        let required = required
            .iter()
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        for field in fields {
            assert!(
                required.contains(field),
                "{schema_name} schema should require {field}; required fields were {required:?}"
            );
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

    #[test]
    fn default_registry_clock_is_wall_time() {
        let wall = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("epoch")
                .as_millis(),
        )
        .expect("ms fits i64");
        let now = (AccountRegistry::default().clock())();

        assert!(
            (now - wall).abs() < 60_000,
            "default clock {now} is not wall time {wall}"
        );
    }

    static EXPIRY_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn expiry_clock() -> i64 {
        EXPIRY_CLOCK.load(Ordering::SeqCst)
    }

    #[test]
    fn file_store_magic_link_consumption_is_atomic() {
        let store_root = temp_store_root("magic-link-atomic");
        let storage = super::StudyStorage::file(store_root.clone(), expiry_clock);
        storage
            .save_auth_challenge(
                "atomic-challenge",
                "learner@example.com",
                DEFAULT_BETA_STUDY_NOW + AUTH_CHALLENGE_TTL_MS,
            )
            .expect("save challenge");

        let storage = Arc::new(storage);
        let barrier = Arc::new(Barrier::new(64));
        let mut workers = Vec::new();
        for _ in 0..64 {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                storage
                    .consume_auth_challenge("atomic-challenge", DEFAULT_BETA_STUDY_NOW)
                    .expect("consume")
                    .is_some()
            }));
        }

        let consumed = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .filter(|consumed| *consumed)
            .count();
        assert_eq!(consumed, 1, "only one caller may consume a magic link");
    }

    #[test]
    fn magic_link_is_rejected_after_its_ttl_elapses() {
        EXPIRY_CLOCK.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
        let registry = AccountRegistry::default()
            .with_clock(expiry_clock)
            .with_auth_config(
                AuthConfig::allow_emails(["learner@example.com".to_owned()]).with_debug_links(true),
            );

        let fresh = registry
            .request_magic_link("learner@example.com", "test-client")
            .expect("fresh link")
            .debug_link
            .expect("debug link");
        let fresh_token = fresh.split("token=").nth(1).expect("token").to_owned();
        assert!(registry.verify_magic_link(&fresh_token).is_ok());

        let stale = registry
            .request_magic_link("learner@example.com", "test-client")
            .expect("stale link")
            .debug_link
            .expect("debug link");
        let stale_token = stale.split("token=").nth(1).expect("token").to_owned();
        EXPIRY_CLOCK.fetch_add(AUTH_CHALLENGE_TTL_MS + 1, Ordering::SeqCst);

        assert!(
            registry.verify_magic_link(&stale_token).is_err(),
            "magic link must expire once its TTL elapses"
        );
    }

    static SESSION_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn session_clock() -> i64 {
        SESSION_CLOCK.load(Ordering::SeqCst)
    }

    #[test]
    fn browser_session_is_rejected_after_it_expires() {
        SESSION_CLOCK.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
        let registry = AccountRegistry::default()
            .with_clock(session_clock)
            .with_auth_config(
                AuthConfig::allow_emails(["learner@example.com".to_owned()]).with_debug_links(true),
            );
        let link = registry
            .request_magic_link("learner@example.com", "test-client")
            .expect("link")
            .debug_link
            .expect("debug link");
        let token = link.split("token=").nth(1).expect("token").to_owned();
        let session = registry.verify_magic_link(&token).expect("session");

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!(
                "{}={}",
                super::APP_SESSION_COOKIE_NAME,
                session.browser_session_id
            )
            .parse()
            .expect("cookie header"),
        );
        assert!(registry
            .require_browser_session(&headers, &session.csrf_token)
            .is_ok());

        SESSION_CLOCK.fetch_add(
            super::app_session_max_age_ms().saturating_add(1),
            Ordering::SeqCst,
        );
        assert!(
            registry
                .require_browser_session(&headers, &session.csrf_token)
                .is_err(),
            "an expired browser session must be rejected server-side"
        );
        assert!(
            registry
                .require_browser_session(&headers, &session.csrf_token)
                .is_err(),
            "an expired session reloaded from storage must also be rejected"
        );
    }

    static SCHEDULE_CLOCK: AtomicI64 = AtomicI64::new(0);

    fn schedule_clock() -> i64 {
        SCHEDULE_CLOCK.load(Ordering::SeqCst)
    }

    #[test]
    fn correct_answer_is_not_due_again_until_real_time_passes() {
        SCHEDULE_CLOCK.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
        let registry = AccountRegistry::default().with_clock(schedule_clock);
        let account = registry
            .create_account("learner@example.com")
            .expect("account");

        let source = registry
            .save_source(
                &account.account_id,
                &account.session_token,
                &super::CreateSourceRequest {
                    title: "NATO practice notes".to_owned(),
                    body: source_body(),
                },
            )
            .expect("source");
        let generated = registry
            .generate_source(
                &account.account_id,
                &account.session_token,
                &source.source_id,
            )
            .expect("generate");
        let draft_id = generated.drafts.first().expect("draft").id.clone();
        registry
            .approve_draft(&account.account_id, &account.session_token, &draft_id)
            .expect("approve");

        let due = registry
            .next_review(&account.account_id, &account.session_token)
            .expect("next review");
        let current = due.current.expect("approved unit is due");
        let review_unit_id = current.review_unit_id.to_string();
        let answered = registry
            .submit_review(
                &account.account_id,
                &account.session_token,
                &review_unit_id,
                &super::SubmitReviewRequest {
                    answer: "ALFA".to_owned(),
                    response_time_ms: 1_500,
                    idempotency_key: "clock-test-1".to_owned(),
                },
            )
            .expect("submit");
        assert_eq!(answered.summary.attempt_count, 1);

        let same_moment = registry
            .next_review(&account.account_id, &account.session_token)
            .expect("next review");
        assert!(
            same_moment.current.is_none(),
            "a correctly answered unit must not be due again at the same moment"
        );

        SCHEDULE_CLOCK.fetch_add(30 * 86_400_000, Ordering::SeqCst);
        let later = registry
            .next_review(&account.account_id, &account.session_token)
            .expect("next review");
        assert!(
            later.current.is_some(),
            "the unit must come due again once enough real time passes"
        );
    }
}
