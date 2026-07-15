use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Form, Path, Query, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio_stream::{wrappers::BroadcastStream, StreamExt as _};

#[path = "icons.rs"]
mod icons;

use memory_engine_study::infer_capture_title;

use memory_engine_api_render::{
    render_account_page, render_action_result_html, render_app_shell, render_auth_recovery,
    render_login_requested, render_return_notification_confirmation,
    render_return_notification_disabled, LEDGER_CSS,
};
use memory_engine_api_state::{
    client_rate_limit_key, csrf_token, html_with_browser_session,
    html_with_cleared_browser_session, normalize_email, read_session_token, AccountCreated,
    ApiFailure, ApiState, AppAccount, ContentFeedbackRequest, CreateAccountRequest,
    CreateProjectDeckRequest, CreateSourceRequest, EnqueueOutcome, HealthResponse,
    InvalidateProjectDeckRequest, ProjectDeckRecord, SourceList, SourceRecord, StudyViewResponse,
    SubmitReviewRequest,
};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct V1ContractOperation {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum V1Route {
    OpenApi,
    Accounts,
    Sources,
    Source,
    ProjectDecks,
    ProjectDeckInvalidate,
    ContentFeedback,
    Generate,
    Approve,
    Next,
    Reveal,
    Reference,
    Skip,
    Snooze,
    SnoozeConcept,
    Bridge,
    Submit,
}

const V1_OPENAPI_JSON: &str = include_str!("../../../docs/api/openapi.v1.json");
const V1_OPENAPI_PATH: &str = "/v1/openapi.json";
const V1_ACCOUNTS_PATH: &str = "/v1/accounts";
const V1_SOURCES_PATH: &str = "/v1/accounts/{account_id}/sources";
const V1_SOURCE_PATH: &str = "/v1/accounts/{account_id}/sources/{source_id}";
const V1_PROJECT_DECKS_PATH: &str = "/v1/accounts/{account_id}/project-decks";
const V1_PROJECT_DECK_INVALIDATE_PATH: &str =
    "/v1/accounts/{account_id}/project-decks/{deck_id}/invalidate";
const V1_GENERATE_PATH: &str = "/v1/accounts/{account_id}/sources/{source_id}/generate";
const V1_APPROVE_PATH: &str = "/v1/accounts/{account_id}/drafts/{draft_id}/approve";
const V1_NEXT_PATH: &str = "/v1/accounts/{account_id}/review/next";
const V1_REVEAL_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/reveal";
const V1_REFERENCE_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/reference";
const V1_SKIP_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/skip";
const V1_SNOOZE_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/snooze";
const V1_SNOOZE_CONCEPT_PATH: &str =
    "/v1/accounts/{account_id}/review/{review_unit_id}/snooze-concept";
const V1_BRIDGE_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/bridge";
const V1_SUBMIT_PATH: &str = "/v1/accounts/{account_id}/review/{review_unit_id}/submit";
const V1_CONTENT_FEEDBACK_PATH: &str =
    "/v1/accounts/{account_id}/review/{review_unit_id}/content-feedback";

const V1_ROUTES: &[V1Route] = &[
    V1Route::OpenApi,
    V1Route::Accounts,
    V1Route::Sources,
    V1Route::Source,
    V1Route::ProjectDecks,
    V1Route::ProjectDeckInvalidate,
    V1Route::Generate,
    V1Route::Approve,
    V1Route::Next,
    V1Route::Reveal,
    V1Route::Reference,
    V1Route::Skip,
    V1Route::Snooze,
    V1Route::SnoozeConcept,
    V1Route::Bridge,
    V1Route::Submit,
    V1Route::ContentFeedback,
];

impl V1Route {
    fn mount(self, router: Router<ApiState>) -> Router<ApiState> {
        match self {
            Self::OpenApi => router.route(V1_OPENAPI_PATH, get(v1_openapi)),
            Self::Accounts => router.route(V1_ACCOUNTS_PATH, post(create_account)),
            Self::Sources => router.route(V1_SOURCES_PATH, get(list_sources).post(create_source)),
            Self::Source => router.route(V1_SOURCE_PATH, delete(archive_source)),
            Self::ProjectDecks => router.route(V1_PROJECT_DECKS_PATH, post(create_project_deck)),
            Self::ProjectDeckInvalidate => router.route(
                V1_PROJECT_DECK_INVALIDATE_PATH,
                post(invalidate_project_deck),
            ),
            Self::Generate => router.route(V1_GENERATE_PATH, post(generate_source)),
            Self::Approve => router.route(V1_APPROVE_PATH, post(approve_draft)),
            Self::Next => router.route(V1_NEXT_PATH, post(next_review)),
            Self::Reveal => router.route(V1_REVEAL_PATH, post(reveal_review)),
            Self::Reference => router.route(V1_REFERENCE_PATH, post(reference_review)),
            Self::Skip => router.route(V1_SKIP_PATH, post(skip_review)),
            Self::Snooze => router.route(V1_SNOOZE_PATH, post(snooze_review)),
            Self::SnoozeConcept => {
                router.route(V1_SNOOZE_CONCEPT_PATH, post(snooze_concept_review))
            }
            Self::Bridge => router.route(V1_BRIDGE_PATH, post(bridge_review)),
            Self::Submit => router.route(V1_SUBMIT_PATH, post(submit_review)),
            Self::ContentFeedback => router.route(V1_CONTENT_FEEDBACK_PATH, post(content_feedback)),
        }
    }

    #[cfg(test)]
    fn operations(self) -> &'static [V1ContractOperation] {
        match self {
            Self::OpenApi => &[V1ContractOperation {
                method: "GET",
                path: V1_OPENAPI_PATH,
            }],
            Self::Accounts => &[V1ContractOperation {
                method: "POST",
                path: V1_ACCOUNTS_PATH,
            }],
            Self::Sources => &[
                V1ContractOperation {
                    method: "GET",
                    path: V1_SOURCES_PATH,
                },
                V1ContractOperation {
                    method: "POST",
                    path: V1_SOURCES_PATH,
                },
            ],
            Self::Source => &[V1ContractOperation {
                method: "DELETE",
                path: V1_SOURCE_PATH,
            }],
            Self::ProjectDecks => &[V1ContractOperation {
                method: "POST",
                path: V1_PROJECT_DECKS_PATH,
            }],
            Self::ProjectDeckInvalidate => &[V1ContractOperation {
                method: "POST",
                path: V1_PROJECT_DECK_INVALIDATE_PATH,
            }],
            Self::Generate => &[V1ContractOperation {
                method: "POST",
                path: V1_GENERATE_PATH,
            }],
            Self::Approve => &[V1ContractOperation {
                method: "POST",
                path: V1_APPROVE_PATH,
            }],
            Self::Next => &[V1ContractOperation {
                method: "POST",
                path: V1_NEXT_PATH,
            }],
            Self::Reveal => &[V1ContractOperation {
                method: "POST",
                path: V1_REVEAL_PATH,
            }],
            Self::Reference => &[V1ContractOperation {
                method: "POST",
                path: V1_REFERENCE_PATH,
            }],
            Self::Skip => &[V1ContractOperation {
                method: "POST",
                path: V1_SKIP_PATH,
            }],
            Self::Snooze => &[V1ContractOperation {
                method: "POST",
                path: V1_SNOOZE_PATH,
            }],
            Self::SnoozeConcept => &[V1ContractOperation {
                method: "POST",
                path: V1_SNOOZE_CONCEPT_PATH,
            }],
            Self::Bridge => &[V1ContractOperation {
                method: "POST",
                path: V1_BRIDGE_PATH,
            }],
            Self::Submit => &[V1ContractOperation {
                method: "POST",
                path: V1_SUBMIT_PATH,
            }],
            Self::ContentFeedback => &[V1ContractOperation {
                method: "POST",
                path: V1_CONTENT_FEEDBACK_PATH,
            }],
        }
    }
}

fn mount_v1_routes(router: Router<ApiState>) -> Router<ApiState> {
    V1_ROUTES
        .iter()
        .copied()
        .fold(router, |router, route| route.mount(router))
}

#[cfg(test)]
pub(crate) fn v1_contract_operations() -> Vec<V1ContractOperation> {
    V1_ROUTES
        .iter()
        .copied()
        .flat_map(V1Route::operations)
        .copied()
        .collect()
}

pub fn router(state: ApiState) -> Router {
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(app_home))
        .route("/static/ledger.css", get(static_ledger_css))
        .route("/static/app.js", get(static_app_js))
        .route("/manifest.webmanifest", get(static_manifest))
        .route("/favicon.png", get(static_favicon))
        .route("/icon-192.png", get(static_icon_192))
        .route("/icon-512.png", get(static_icon_512))
        .route("/apple-touch-icon.png", get(static_apple_touch_icon))
        .route("/accounts", post(create_account));

    mount_v1_routes(router)
        .route("/app/start", post(start_app_study))
        .route("/app/account", post(create_app_account))
        .route("/app/login/verify", get(verify_app_login))
        .route("/app/logout", post(logout_app_session))
        .route(
            "/app/return-notifications",
            get(return_notification_page).post(update_return_notifications),
        )
        .route("/app/save-account", post(save_app_account))
        .route("/app/source", post(create_app_source))
        .route("/app/capture", post(capture_app_source))
        .route("/app/source/archive", post(archive_app_source))
        .route("/app/generate", post(generate_app_source))
        .route("/app/jobs/events", get(app_jobs_events))
        .route("/app/jobs/retry", post(retry_app_job))
        .route("/app/next", post(next_app_review))
        .route("/app/reveal", post(reveal_app_review))
        .route("/app/reference", post(reference_app_review))
        .route("/app/skip", post(skip_app_review))
        .route("/app/snooze", post(snooze_app_review))
        .route("/app/snooze-concept", post(snooze_concept_app_review))
        .route("/app/delete", post(delete_app_review))
        .route("/app/bridge", post(bridge_app_review))
        .route("/app/submit", post(submit_app_review))
        .route("/app/content-feedback", post(record_app_content_feedback))
        .route(
            "/accounts/{account_id}/sources",
            get(list_sources).post(create_source),
        )
        .route(
            "/accounts/{account_id}/sources/{source_id}",
            delete(archive_source),
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
            "/accounts/{account_id}/review/{review_unit_id}/reference",
            post(reference_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/skip",
            post(skip_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/snooze",
            post(snooze_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/snooze-concept",
            post(snooze_concept_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/bridge",
            post(bridge_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/submit",
            post(submit_review),
        )
        .route(
            "/accounts/{account_id}/review/{review_unit_id}/content-feedback",
            post(content_feedback),
        )
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "memory-engine-api",
    })
}

/// Serve the Ledger design system stylesheet (DESIGN.md). The render crate
/// owns the markup and CSS; this HTTP crate only exposes the deployed path.
async fn static_ledger_css() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "text/css; charset=utf-8"),
            (CACHE_CONTROL, "public, max-age=3600"),
        ],
        LEDGER_CSS,
    )
}

async fn static_manifest() -> impl IntoResponse {
    const MANIFEST: &str = r##"{
  "name": "Memory Engine",
  "short_name": "Memory Engine",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "background_color": "#f6f2ea",
  "theme_color": "#f6f2ea",
  "icons": [
    { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable" },
    { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable" }
]
}"##;
    ([(CONTENT_TYPE, "application/manifest+json")], MANIFEST)
}

use icons::{APPLE_TOUCH_ICON_PNG, APP_ICON_192_PNG, APP_ICON_512_PNG};

async fn static_favicon() -> impl IntoResponse {
    ([(CONTENT_TYPE, "image/png")], APP_ICON_192_PNG)
}

async fn static_apple_touch_icon() -> impl IntoResponse {
    ([(CONTENT_TYPE, "image/png")], APPLE_TOUCH_ICON_PNG)
}

async fn static_icon_192() -> impl IntoResponse {
    ([(CONTENT_TYPE, "image/png")], APP_ICON_192_PNG)
}

async fn static_icon_512() -> impl IntoResponse {
    ([(CONTENT_TYPE, "image/png")], APP_ICON_512_PNG)
}

async fn v1_openapi() -> impl IntoResponse {
    ([(CONTENT_TYPE, "application/json")], V1_OPENAPI_JSON)
}

async fn app_home(State(state): State<ApiState>, headers: HeaderMap) -> Html<String> {
    // The home is the durable entry point, so it must respect an existing
    // session: a signed-in learner reloading or navigating to "/" lands on their
    // workspace (with the live due count and Start review CTA), not the
    // signed-out cover. Read-only session check — a GET carries no CSRF token.
    match state.require_browser_session_readonly(&headers) {
        Ok(account) => Html(render_account_page(&state, &account, None, None)),
        Err(_) => Html(render_app_shell(None, &[], None, &[], None)),
    }
}

async fn create_account(
    State(state): State<ApiState>,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountCreated>), ApiFailure> {
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))?;
    let account = state.create_account(&email)?;

    Ok((StatusCode::CREATED, Json(account)))
}

async fn create_source(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<SourceRecord>), ApiFailure> {
    let session_token = read_session_token(&headers)?;
    let source = state.save_source(&account_id, session_token, &request)?;

    Ok((StatusCode::CREATED, Json(source)))
}

async fn create_project_deck(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateProjectDeckRequest>,
) -> Result<(StatusCode, Json<ProjectDeckRecord>), ApiFailure> {
    let session_token = read_session_token(&headers)?;
    let deck = state.create_project_deck(&account_id, session_token, &request)?;

    Ok((StatusCode::CREATED, Json(deck)))
}

async fn invalidate_project_deck(
    State(state): State<ApiState>,
    Path((account_id, deck_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<InvalidateProjectDeckRequest>,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.invalidate_project_deck(
        &account_id,
        session_token,
        &deck_id,
        &request,
    )?))
}

async fn list_sources(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SourceList>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(SourceList {
        sources: state.list_sources(&account_id, session_token)?,
    }))
}

async fn generate_source(
    State(state): State<ApiState>,
    Path((account_id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.generate_source(
        &account_id,
        session_token,
        &source_id,
    )?))
}

async fn archive_source(
    State(state): State<ApiState>,
    Path((account_id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    state.archive_source(&account_id, session_token, &source_id)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn approve_draft(
    State(state): State<ApiState>,
    Path((account_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.approve_draft(
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

    Ok(Json(state.next_review(&account_id, session_token)?))
}

async fn reveal_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.reveal_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn reference_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.learn_more_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn skip_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.skip_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn snooze_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.snooze_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn snooze_concept_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.snooze_concept_review(
        &account_id,
        session_token,
        &review_unit_id,
    )?))
}

async fn bridge_review(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.bridge_review(
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

    Ok(Json(state.submit_review(
        &account_id,
        session_token,
        &review_unit_id,
        &request,
    )?))
}

async fn content_feedback(
    State(state): State<ApiState>,
    Path((account_id, review_unit_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ContentFeedbackRequest>,
) -> Result<Json<memory_engine_service::ContentFeedback>, ApiFailure> {
    let session_token = read_session_token(&headers)?;

    Ok(Json(state.record_content_feedback(
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
struct AppLoginVerifyQuery {
    token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppStartForm {
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSourceForm {
    csrf_token: Option<String>,
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSaveAccountForm {
    csrf_token: Option<String>,
    email: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppAccountActionForm {
    csrf_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSourceActionForm {
    csrf_token: Option<String>,
    source_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppJobActionForm {
    csrf_token: Option<String>,
    job_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewActionForm {
    csrf_token: Option<String>,
    review_unit_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewSubmitForm {
    csrf_token: Option<String>,
    review_unit_id: String,
    answer: String,
    // Raw text, not a number: the field is filled by a client-side timer, so
    // it can arrive blank (JavaScript off), malformed, or hostile. Rejecting
    // those shapes would block the learner's answer; instead the submit
    // handler sanitizes them to a conservative value that can never rate
    // `Easy`.
    response_time_ms: Option<String>,
    idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppContentFeedbackForm {
    csrf_token: Option<String>,
    review_unit_id: String,
    verdict: memory_engine_service::ContentFeedbackVerdict,
    rationale: Option<String>,
    idempotency_key: String,
    supersedes_id: Option<String>,
}

async fn create_app_account(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountForm>,
) -> Response {
    let result = state.request_magic_link(&form.email, &client_rate_limit_key(&headers));

    no_store_response(match result {
        Ok(request) => Html(render_login_requested(request.debug_link.as_deref())).into_response(),
        Err(error) => app_failure_response(error),
    })
}

async fn verify_app_login(
    State(state): State<ApiState>,
    Query(query): Query<AppLoginVerifyQuery>,
) -> Response {
    match state.verify_magic_link(&query.token) {
        Ok(account) => {
            let view = state.app_study_view(&account).ok();
            no_store_response(html_with_browser_session(
                &account,
                render_account_page(&state, &account, view.as_ref(), None),
            ))
        }
        Err(error) if error.is_magic_link_recovery() => {
            let status = error.status();
            let mut response = Html(render_auth_recovery(
                "Sign-in link expired",
                "That link is no longer valid. Request a fresh link and return to your study space.",
            ))
            .into_response();
            *response.status_mut() = status;
            no_store_response(response)
        }
        Err(error) => no_store_response(app_failure_response(error)),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReturnNotificationForm {
    csrf_token: Option<String>,
    #[serde(rename = "unsubscribeToken")]
    unsubscribe_token: Option<String>,
    #[serde(rename = "reminderEmail")]
    reminder_email: Option<String>,
    enabled: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
struct AppReturnNotificationQuery {
    token: Option<String>,
}

async fn return_notification_page(
    State(state): State<ApiState>,
    Query(query): Query<AppReturnNotificationQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = query.token.as_deref() {
        return match state.validate_return_notification_token(token) {
            Ok(()) => no_store_response(
                Html(render_return_notification_confirmation(token)).into_response(),
            ),
            Err(error) => no_store_response(app_failure_response(error)),
        };
    }
    let account = match state.require_browser_session_readonly(&headers) {
        Ok(account) => account,
        Err(error) => return no_store_response(app_failure_response(error)),
    };
    no_store_response(Html(render_account_page(&state, &account, None, None)).into_response())
}

async fn update_return_notifications(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReturnNotificationForm>,
) -> Response {
    if let Some(token) = form
        .unsubscribe_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        return no_store_response(match state.disable_return_notification(token) {
            Ok(()) => Html(render_return_notification_disabled()).into_response(),
            Err(error) => app_failure_response(error),
        });
    }
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return no_store_response(app_failure_response(error)),
        };
    let enabled = form.enabled.as_deref() == Some("on");
    let due_count = state
        .app_study_view(&account)
        .map_or(0, |view| view.due_count);
    let result = state
        .set_return_notification(&account, form.reminder_email.as_deref(), enabled)
        .and_then(|()| {
            if enabled {
                state
                    .maybe_send_due_count_notification(&account, due_count, true)
                    .map(|_| ())
            } else {
                Ok(())
            }
        });
    let notice = match result {
        Ok(()) if enabled => "Due-count reminders are on. One confirmation was sent; reminders stay to one per day and can be turned off below.",
        Ok(()) => "Due-count reminders are off.",
        Err(error) => return no_store_response(Html(render_account_page(&state, &account, None, Some(&error.message))).into_response()),
    };
    no_store_response(
        Html(render_account_page(&state, &account, None, Some(notice))).into_response(),
    )
}

async fn logout_app_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    match state.revoke_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
        Ok(()) => html_with_cleared_browser_session(render_app_shell(None, &[], None, &[], None)),
        Err(error) => app_failure_response(error),
    }
}

async fn save_app_account(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSaveAccountForm>,
) -> Response {
    let source_account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let source_view = state.app_study_view(&source_account).ok();
    let result = normalize_email(&form.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))
        .and_then(|email| state.save_account(&source_account, &email));

    match result {
        Ok(account) => {
            let account = match state.create_browser_session(&account) {
                Ok(account) => account,
                Err(error) => return app_failure_response(error),
            };
            let view = state.app_study_view(&account).ok().or(source_view);
            html_with_browser_session(
                &account,
                render_account_page(&state, &account, view.as_ref(), None),
            )
        }
        Err(error) => Html(render_account_page(
            &state,
            &source_account,
            source_view.as_ref(),
            Some(&error.message),
        ))
        .into_response(),
    }
}

async fn start_app_study(
    State(state): State<ApiState>,
    Form(form): Form<AppStartForm>,
) -> Response {
    let email = format!("guest-{:032x}@memory-engine.local", rand::random::<u128>());
    let account = match state.create_account(&email) {
        Ok(account) => account,
        Err(error) => {
            return Html(render_app_shell(None, &[], None, &[], Some(&error.message)))
                .into_response();
        }
    };
    let account = match state.create_browser_session(&account) {
        Ok(account) => account,
        Err(error) => return app_failure_response(error),
    };
    let result = state.save_app_source(
        &account,
        &capture_request(form.title, form.body, form.capture),
    );

    html_with_browser_session(
        &account,
        render_save_result_html(&state, &account, result.map(|_| ())),
    )
}

async fn create_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.save_app_source(
        &account,
        &capture_request(form.title, form.body, form.capture),
    );

    Html(render_save_result_html(
        &state,
        &account,
        result.map(|_| ()),
    ))
    .into_response()
}

/// Capture material and enqueue generation. Returns immediately: the source is
/// saved synchronously (fast, local), then a background job generates cards
/// while the learner is free to do anything else. Progress shows in the
/// activity log, live over SSE.
async fn capture_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let request = capture_request(form.title, form.body, form.capture);
    let notice = match state.save_app_source(&account, &request) {
        Ok(source) => {
            match state.enqueue_generation_job_by_source(
                &account,
                &source.source_id,
                &request.title,
            ) {
                EnqueueOutcome::Started(_) => {
                    "Generating your cards. They'll appear below as they're ready."
                }
                EnqueueOutcome::AlreadyInFlight(_) => "Already generating this source.",
            }
        }
        Err(error) => {
            return Html(render_account_page(
                &state,
                &account,
                None,
                Some(&error.message),
            ))
            .into_response()
        }
    };

    Html(render_account_page(&state, &account, None, Some(notice))).into_response()
}

fn render_save_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<(), ApiFailure>,
) -> String {
    match result {
        Ok(()) => render_account_page(
            state,
            account,
            None,
            Some("Capture saved. Create review when you are ready."),
        ),
        Err(error) => render_account_page(state, account, None, Some(&error.message)),
    }
}

fn capture_request(
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
) -> CreateSourceRequest {
    let body = capture.or(body).unwrap_or_default();
    let title = title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| infer_capture_title(&body));

    CreateSourceRequest { title, body }
}

/// Re-generate from an already-saved source — enqueues a background job, same
/// as capture, and returns immediately.
async fn generate_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let title = state
        .list_app_sources(&account)
        .ok()
        .and_then(|sources| {
            sources
                .into_iter()
                .find(|source| source.source_id == form.source_id)
                .map(|source| source.title)
        })
        .unwrap_or_else(|| "New material".to_owned());
    let notice = match state.enqueue_generation_job_by_source(&account, &form.source_id, &title) {
        EnqueueOutcome::Started(_) => "Generating. Watch the activity log.",
        EnqueueOutcome::AlreadyInFlight(_) => "Already generating this source.",
    };

    Html(render_account_page(&state, &account, None, Some(notice))).into_response()
}

async fn archive_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.archive_app_source(&account, &form.source_id);

    match result {
        Ok((view, archived_count)) => {
            // memory-engine-088: the operator dogfood found "Source removed."
            // gave no sense of scope for an action that silently retires
            // every card generated from the source, across every generation
            // run. Name the actual count instead.
            let cards = if archived_count == 1 { "card" } else { "cards" };
            let notice = format!("Source removed. {archived_count} {cards} retired.");
            Html(render_account_page(
                &state,
                &account,
                Some(&view),
                Some(&notice),
            ))
            .into_response()
        }
        Err(error) => Html(render_account_page(
            &state,
            &account,
            None,
            Some(&error.message),
        ))
        .into_response(),
    }
}

/// Retry a failed generation job. Re-queues it for the worker and re-renders
/// the activity log with the job back in flight.
async fn retry_app_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppJobActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let notice = if state.retry_generation_job(&account, &form.job_id) {
        "Retrying. Generating again in the background."
    } else {
        "That job can't be retried."
    };

    Html(render_account_page(&state, &account, None, Some(notice))).into_response()
}

/// Live job-status stream (SSE). Pushes this account's job updates as they
/// happen so the activity log updates without a reload. The page is already
/// server-authoritative, so this is pure progressive enhancement.
async fn app_jobs_events(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let account = match state.require_browser_session_readonly(&headers) {
        Ok(account) => account,
        Err(error) => return app_failure_response(error),
    };
    let account_id = account.account_id().to_owned();
    // `tokio_stream::StreamExt::filter_map` is synchronous: the closure returns
    // an `Option` directly, not a future.
    let stream = BroadcastStream::new(state.subscribe_jobs()).filter_map(move |message| {
        match message {
            // Full-state snapshots, scoped to this learner. A lagged subscriber
            // simply skips to the next event — the page reload is authoritative.
            Ok(update) if update.account_id == account_id => Some(Ok::<Event, Infallible>(
                Event::default().event("job").data(update.payload),
            )),
            _ => None,
        }
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Serve the progressive-enhancement client script (vendored, like the CSS).
async fn static_app_js() -> impl IntoResponse {
    const JS: &str = include_str!("../assets/app.js");
    (
        [
            (CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (CACHE_CONTROL, "public, max-age=3600"),
        ],
        JS,
    )
}

async fn next_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.next_app_review(&account);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

fn app_failure_response(error: ApiFailure) -> Response {
    if error.is_session_expired() {
        let status = error.status();
        let mut response = Html(render_auth_recovery(
            "Your session expired",
            "Your study data is safe. Sign in again to continue where you left off.",
        ))
        .into_response();
        *response.status_mut() = status;
        return no_store_response(response);
    }
    error.into_response()
}

fn no_store_response(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}

async fn reveal_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.reveal_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn reference_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.learn_more_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn skip_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.skip_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn delete_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.delete_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn snooze_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.snooze_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn snooze_concept_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return error.into_response(),
        };
    let result = state.snooze_concept_app_review(&account, &form.review_unit_id);

    match result {
        Ok(view) => Html(render_action_result_html(&state, &account, Ok(view))).into_response(),
        Err(error) => {
            let status = error.status();
            (
                status,
                Html(render_action_result_html(&state, &account, Err(error))),
            )
                .into_response()
        }
    }
}

async fn bridge_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.bridge_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

/// Ceiling for a plausible single-answer response time (ten minutes).
///
/// The browser owns the timer, so the server cannot make the reported value
/// true — it can only refuse dishonest shapes. Anything missing, blank,
/// malformed, non-positive, or beyond this ceiling grades as the ceiling
/// itself: the slowest plausible answer. A broken or lying client can
/// therefore never manufacture the fast-answer `Easy` rating, and a learner
/// who walked away mid-card records ten minutes, not an absurd outlier.
pub(crate) const MAX_PLAUSIBLE_RESPONSE_TIME_MS: u32 = 600_000;

/// Sanitize the client-reported response time before it reaches the typed
/// review boundary. Valid positive millisecond counts pass through, clamped
/// to [`MAX_PLAUSIBLE_RESPONSE_TIME_MS`]; every other shape maps to that same
/// conservative ceiling.
pub(crate) fn sanitize_response_time_ms(raw: Option<&str>) -> u32 {
    raw.and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|&elapsed| elapsed > 0)
        .map_or(MAX_PLAUSIBLE_RESPONSE_TIME_MS, |elapsed| {
            elapsed.min(MAX_PLAUSIBLE_RESPONSE_TIME_MS)
        })
}

async fn submit_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewSubmitForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(error),
        };
    let result = state.submit_app_review(
        &account,
        &form.review_unit_id,
        &SubmitReviewRequest {
            answer: form.answer,
            response_time_ms: sanitize_response_time_ms(form.response_time_ms.as_deref()),
            idempotency_key: form.idempotency_key,
        },
    );

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn record_app_content_feedback(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppContentFeedbackForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return error.into_response(),
        };
    let result = state.record_app_content_feedback(
        &account,
        &form.review_unit_id,
        &ContentFeedbackRequest {
            verdict: form.verdict,
            rationale: form.rationale,
            idempotency_key: form.idempotency_key,
            supersedes_id: form.supersedes_id,
        },
    );
    match result {
        Ok(_) => Html(render_account_page(
            &state,
            &account,
            None,
            Some("Saved. This card will help improve future generation."),
        ))
        .into_response(),
        Err(error) => error.into_response(),
    }
}
