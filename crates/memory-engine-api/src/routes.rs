use std::{
    convert::Infallible,
    fmt::Write as _,
    time::{Duration, Instant},
};

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
use serde::{Deserialize, Serialize};
use tokio_stream::{wrappers::BroadcastStream, StreamExt as _};

#[path = "icons.rs"]
mod icons;

use memory_engine_study::infer_capture_title;

use memory_engine_api_render::{
    render_account_page, render_action_result_html, render_analytics_page, render_app_shell,
    render_auth_recovery, render_content_feedback_recovery_html,
    render_content_feedback_result_html, render_create_page, render_edit_review_html,
    render_library_page, render_login_requested, render_return_notification_confirmation,
    render_return_notification_disabled, render_return_notification_recovery,
    render_submit_action_result_html, render_submit_recovery, render_waitlist_joined,
    render_waitlist_recovery, AnalyticsConceptFilter, AnalyticsConceptSort, AnalyticsViewOptions,
    ContentFeedbackRecovery, LEDGER_CSS,
};
use memory_engine_api_state::{
    csrf_token, html_with_browser_session, html_with_cleared_browser_session, normalize_email,
    read_session_token, report_submit_browser_performance, report_submit_server_performance,
    AccountCreated, ApiFailure, ApiState, AppAccount, BrowserSubmitReceipt, ContentFeedbackRequest,
    CreateAccountRequest, CreateProjectDeckRequest, CreateSourceRequest, EnqueueOutcome,
    GenerationJob, HealthResponse, InvalidateProjectDeckRequest, JobStatus, ProjectDeckRecord,
    ReadinessResponse, ScheduledReturnNotificationReport, SourceList, SourcePermission,
    SourceRecord, StudyViewResponse, SubmitPerformanceOutcome, SubmitReviewRequest,
    SubmitReviewTimings, SubmitViewport, WaitlistEntry,
};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct V1ContractOperation {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
}

#[cfg(test)]
macro_rules! single_operation {
    ($method:literal, $path:expr) => {
        &[V1ContractOperation {
            method: $method,
            path: $path,
        }]
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum V1Route {
    OpenApi,
    Accounts,
    ServiceSessions,
    ApiSessionRevoke,
    ApiSessionsRevokeAll,
    Sources,
    Source,
    ProjectDecks,
    ProjectDeckInvalidate,
    ContentFeedback,
    Generate,
    GenerationJobs,
    GenerationJob,
    Keep,
    EditDraft,
    RejectDraft,
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
const V1_SERVICE_SESSIONS_PATH: &str = "/v1/service-sessions";
const V1_API_SESSION_REVOKE_PATH: &str = "/v1/accounts/{account_id}/service-sessions/current";
const V1_API_SESSIONS_REVOKE_ALL_PATH: &str = "/v1/accounts/{account_id}/service-sessions/all";
const V1_SOURCES_PATH: &str = "/v1/accounts/{account_id}/sources";
const V1_SOURCE_PATH: &str = "/v1/accounts/{account_id}/sources/{source_id}";
const V1_PROJECT_DECKS_PATH: &str = "/v1/accounts/{account_id}/project-decks";
const V1_PROJECT_DECK_INVALIDATE_PATH: &str =
    "/v1/accounts/{account_id}/project-decks/{deck_id}/invalidate";
const V1_GENERATE_PATH: &str = "/v1/accounts/{account_id}/sources/{source_id}/generate";
const V1_GENERATION_JOBS_PATH: &str =
    "/v1/accounts/{account_id}/sources/{source_id}/generation-jobs";
const V1_GENERATION_JOB_PATH: &str = "/v1/accounts/{account_id}/generation-jobs/{job_id}";
const V1_KEEP_PATH: &str = "/v1/accounts/{account_id}/drafts/{draft_id}/keep";
const V1_EDIT_DRAFT_PATH: &str = "/v1/accounts/{account_id}/drafts/{draft_id}/edit";
const V1_REJECT_DRAFT_PATH: &str = "/v1/accounts/{account_id}/drafts/{draft_id}/reject";
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
    V1Route::ServiceSessions,
    V1Route::ApiSessionRevoke,
    V1Route::ApiSessionsRevokeAll,
    V1Route::Sources,
    V1Route::Source,
    V1Route::ProjectDecks,
    V1Route::ProjectDeckInvalidate,
    V1Route::Generate,
    V1Route::GenerationJobs,
    V1Route::GenerationJob,
    V1Route::Keep,
    V1Route::EditDraft,
    V1Route::RejectDraft,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationJobResource {
    id: String,
    source_id: String,
    title: String,
    status: JobStatus,
    card_count: usize,
    attempts: u32,
    retryable: bool,
    error: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl From<GenerationJob> for GenerationJobResource {
    fn from(job: GenerationJob) -> Self {
        Self {
            id: job.id,
            source_id: job.source_id,
            title: job.title,
            status: job.status,
            card_count: job.card_count,
            attempts: job.attempts,
            retryable: job.retryable,
            error: job.error,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnqueuedGenerationJobResource {
    #[serde(flatten)]
    job: GenerationJobResource,
    coalesced: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditDraftRequest {
    prompt: String,
    expected_answer: String,
}

#[derive(Debug, Default, Deserialize)]
struct AnalyticsQuery {
    filter: Option<String>,
    sort: Option<String>,
    page: Option<usize>,
}

impl AnalyticsQuery {
    fn options(self) -> AnalyticsViewOptions {
        AnalyticsViewOptions {
            filter: match self.filter.as_deref() {
                Some("at-risk") => AnalyticsConceptFilter::AtRisk,
                Some("struggling") => AnalyticsConceptFilter::Struggling,
                Some("mixed") => AnalyticsConceptFilter::Mixed,
                Some("solid") => AnalyticsConceptFilter::Solid,
                Some("untried") => AnalyticsConceptFilter::Untried,
                _ => AnalyticsConceptFilter::All,
            },
            sort: match self.sort.as_deref() {
                Some("name") => AnalyticsConceptSort::Name,
                Some("success") => AnalyticsConceptSort::Success,
                _ => AnalyticsConceptSort::Health,
            },
            page: self.page.unwrap_or(1),
        }
    }
}

impl V1Route {
    fn mount(self, router: Router<ApiState>) -> Router<ApiState> {
        match self {
            Self::OpenApi => router.route(V1_OPENAPI_PATH, get(v1_openapi)),
            Self::Accounts => router.route(V1_ACCOUNTS_PATH, post(create_account)),
            Self::ServiceSessions => {
                router.route(V1_SERVICE_SESSIONS_PATH, post(issue_service_session))
            }
            Self::ApiSessionRevoke => {
                router.route(V1_API_SESSION_REVOKE_PATH, delete(revoke_api_session))
            }
            Self::ApiSessionsRevokeAll => router.route(
                V1_API_SESSIONS_REVOKE_ALL_PATH,
                delete(revoke_all_api_sessions),
            ),
            Self::Sources => router.route(V1_SOURCES_PATH, get(list_sources).post(create_source)),
            Self::Source => router.route(
                V1_SOURCE_PATH,
                delete(archive_source).patch(update_source_permission),
            ),
            Self::ProjectDecks => router.route(V1_PROJECT_DECKS_PATH, post(create_project_deck)),
            Self::ProjectDeckInvalidate => router.route(
                V1_PROJECT_DECK_INVALIDATE_PATH,
                post(invalidate_project_deck),
            ),
            Self::Generate => router.route(V1_GENERATE_PATH, post(generate_source)),
            Self::GenerationJobs => {
                router.route(V1_GENERATION_JOBS_PATH, post(enqueue_generation_job))
            }
            Self::GenerationJob => router.route(V1_GENERATION_JOB_PATH, get(get_generation_job)),
            Self::Keep => router.route(V1_KEEP_PATH, post(keep_draft)),
            Self::EditDraft => router.route(V1_EDIT_DRAFT_PATH, post(edit_draft)),
            Self::RejectDraft => router.route(V1_REJECT_DRAFT_PATH, post(reject_draft)),
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
    #[allow(clippy::too_many_lines)]
    fn operations(self) -> &'static [V1ContractOperation] {
        match self {
            Self::OpenApi => single_operation!("GET", V1_OPENAPI_PATH),
            Self::Accounts => single_operation!("POST", V1_ACCOUNTS_PATH),
            Self::ServiceSessions => single_operation!("POST", V1_SERVICE_SESSIONS_PATH),
            Self::ApiSessionRevoke => single_operation!("DELETE", V1_API_SESSION_REVOKE_PATH),
            Self::ApiSessionsRevokeAll => {
                single_operation!("DELETE", V1_API_SESSIONS_REVOKE_ALL_PATH)
            }
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
            Self::Source => &[
                V1ContractOperation {
                    method: "DELETE",
                    path: V1_SOURCE_PATH,
                },
                V1ContractOperation {
                    method: "PATCH",
                    path: V1_SOURCE_PATH,
                },
            ],
            Self::ProjectDecks => single_operation!("POST", V1_PROJECT_DECKS_PATH),
            Self::ProjectDeckInvalidate => {
                single_operation!("POST", V1_PROJECT_DECK_INVALIDATE_PATH)
            }
            Self::Generate => single_operation!("POST", V1_GENERATE_PATH),
            Self::GenerationJobs => single_operation!("POST", V1_GENERATION_JOBS_PATH),
            Self::GenerationJob => single_operation!("GET", V1_GENERATION_JOB_PATH),
            Self::Keep => single_operation!("POST", V1_KEEP_PATH),
            Self::EditDraft => single_operation!("POST", V1_EDIT_DRAFT_PATH),
            Self::RejectDraft => single_operation!("POST", V1_REJECT_DRAFT_PATH),
            Self::Next => single_operation!("POST", V1_NEXT_PATH),
            Self::Reveal => single_operation!("POST", V1_REVEAL_PATH),
            Self::Reference => single_operation!("POST", V1_REFERENCE_PATH),
            Self::Skip => single_operation!("POST", V1_SKIP_PATH),
            Self::Snooze => single_operation!("POST", V1_SNOOZE_PATH),
            Self::SnoozeConcept => single_operation!("POST", V1_SNOOZE_CONCEPT_PATH),
            Self::Bridge => single_operation!("POST", V1_BRIDGE_PATH),
            Self::Submit => single_operation!("POST", V1_SUBMIT_PATH),
            Self::ContentFeedback => single_operation!("POST", V1_CONTENT_FEEDBACK_PATH),
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
        .route("/readyz", get(readyz))
        .route("/", get(app_home))
        .route("/static/ledger.css", get(static_ledger_css))
        .route("/static/app.js", get(static_app_js))
        .route("/manifest.webmanifest", get(static_manifest))
        .route("/favicon.png", get(static_favicon))
        .route("/icon-192.png", get(static_icon_192))
        .route("/icon-512.png", get(static_icon_512))
        .route("/apple-touch-icon.png", get(static_apple_touch_icon))
        .route(
            "/internal/scheduler/return-notifications",
            post(run_return_notification_scheduler),
        )
        .route("/internal/waitlist", get(list_waitlist))
        .route("/internal/waitlist/export", get(export_waitlist))
        .route("/internal/waitlist/invite", post(invite_waitlist))
        .route("/internal/waitlist/delete", post(delete_waitlist))
        .route("/accounts", post(create_account));

    let router = mount_v1_routes(router)
        .route("/app/start", post(start_app_study))
        .route("/app/analytics", get(app_analytics))
        .route("/app/create", get(app_create))
        .route("/app/library", get(app_library))
        .route("/app/account", post(create_app_account))
        .route("/app/waitlist", post(create_app_waitlist))
        .route("/app/login/verify", get(verify_app_login))
        .route("/app/logout", post(logout_app_session))
        .route("/app/logout-all", post(logout_all_app_sessions))
        .route(
            "/app/return-notifications",
            get(return_notification_page).post(update_return_notifications),
        )
        .route("/app/save-account", post(save_app_account))
        .route("/app/source", post(create_app_source))
        .route("/app/capture", post(capture_app_source))
        .route("/app/source/permission", post(update_app_source_permission))
        .route("/app/source/archive", post(archive_app_source))
        .route("/app/generate", post(generate_app_source))
        .route("/app/jobs/events", get(app_jobs_events))
        .route("/app/jobs/retry", post(retry_app_job))
        .route("/app/next", post(next_app_review))
        .route("/app/draft/keep", post(keep_app_draft))
        .route("/app/draft/edit", post(edit_app_draft))
        .route("/app/draft/reject", post(reject_app_draft))
        .route("/app/reveal", post(reveal_app_review))
        .route("/app/reference", post(reference_app_review))
        .route("/app/skip", post(skip_app_review))
        .route("/app/snooze", post(snooze_app_review))
        .route("/app/snooze-concept", post(snooze_concept_app_review))
        .route("/app/edit", post(edit_app_review))
        .route("/app/edit/save", post(save_app_review))
        .route("/app/delete", post(delete_app_review))
        .route("/app/bridge", post(bridge_app_review))
        .route("/app/submit", post(submit_app_review))
        .route(
            "/app/performance/submit",
            post(record_submit_browser_performance),
        )
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
            "/accounts/{account_id}/drafts/{draft_id}/keep",
            post(keep_draft),
        )
        .route(
            "/accounts/{account_id}/drafts/{draft_id}/edit",
            post(edit_draft),
        )
        .route(
            "/accounts/{account_id}/drafts/{draft_id}/reject",
            post(reject_draft),
        )
        .route("/accounts/{account_id}/review/next", get(next_review));
    mount_review_routes(router).with_state(state)
}

/// The review "escape hatch" routes: reveal, cross-reference, skip, snooze,
/// snooze-concept, bridge, submit, and content-feedback for one review unit.
/// Split out of [`router`] to keep that function under the workspace line
/// budget; these routes share no state setup with the rest of the router.
fn mount_review_routes(router: Router<ApiState>) -> Router<ApiState> {
    router
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
}

async fn healthz(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "memory-engine-api",
        return_notification_scheduler: state.scheduler_health(),
    })
}

async fn run_return_notification_scheduler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ScheduledReturnNotificationReport>, ApiFailure> {
    let token = headers
        .get("x-scheduler-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let report =
        tokio::task::spawn_blocking(move || state.run_manual_return_notification_scheduler(&token))
            .await
            .map_err(|error| {
                ApiFailure::internal(format!("scheduler worker join failed: {error}"))
            })??;
    Ok(Json(report))
}

async fn readyz(State(state): State<ApiState>) -> (StatusCode, Json<ReadinessResponse>) {
    let readiness = state.readiness();
    let status = if readiness.status == "ready" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(readiness))
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
  "name": "Scry",
  "short_name": "Scry",
  "description": "Remember everything",
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

async fn app_analytics(
    State(state): State<ApiState>,
    Query(query): Query<AnalyticsQuery>,
    headers: HeaderMap,
) -> Html<String> {
    match state.require_browser_session_readonly(&headers) {
        Ok(account) => match state.app_study_view(&account) {
            Ok(view) => Html(render_analytics_page(&account, &view, query.options())),
            Err(error) => Html(render_account_page(
                &state,
                &account,
                None,
                Some(&error.message),
            )),
        },
        Err(_) => Html(render_app_shell(None, &[], None, &[], None)),
    }
}

async fn app_create(State(state): State<ApiState>, headers: HeaderMap) -> Html<String> {
    match state.require_browser_session_readonly(&headers) {
        Ok(account) => Html(render_create_page(&state, &account, None)),
        Err(_) => Html(render_app_shell(None, &[], None, &[], None)),
    }
}

async fn app_library(State(state): State<ApiState>, headers: HeaderMap) -> Html<String> {
    match state.require_browser_session_readonly(&headers) {
        Ok(account) => Html(render_library_page(&state, &account, None, None)),
        Err(_) => Html(render_app_shell(None, &[], None, &[], None)),
    }
}

async fn create_account(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountCreated>), ApiFailure> {
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))?;
    let account = if state.anonymous_account_creation_allowed() {
        state.create_account(&email)?
    } else {
        let admin_token = admin_token_from_headers(&headers);
        state.issue_service_session(admin_token, &email)?
    };

    Ok((StatusCode::CREATED, Json(account)))
}

/// Issue an independent account-scoped service-session credential for a
/// machine consumer. Gated by the operator admin token; callers can revoke one
/// credential or all credentials through the explicit DELETE routes.
///
/// The admin token is verified before the body is deserialized, so an
/// unauthorized caller can never exercise the JSON parser on this
/// credential-minting surface.
async fn issue_service_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<AccountCreated>), ApiFailure> {
    let admin_token = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    state.verify_admin_token(admin_token)?;
    let Json(request) = Json::<CreateAccountRequest>::from_bytes(&body)
        .map_err(|_| ApiFailure::bad_request("Request body must be JSON with an email field."))?;
    let account = state.issue_service_session(admin_token, &request.email)?;

    Ok((StatusCode::CREATED, Json(account)))
}

async fn revoke_api_session(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    state.revoke_api_session(&account_id, session_token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_all_api_sessions(
    State(state): State<ApiState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    state.revoke_all_api_sessions(&account_id, session_token)?;
    Ok(StatusCode::NO_CONTENT)
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

async fn enqueue_generation_job(
    State(state): State<ApiState>,
    Path((account_id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<EnqueuedGenerationJobResource>), ApiFailure> {
    let session_token = read_session_token(&headers)?;
    let (job, coalesced) =
        state.enqueue_generation_job_for_session(&account_id, session_token, &source_id)?;
    let status = if coalesced {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };

    Ok((
        status,
        Json(EnqueuedGenerationJobResource {
            job: job.into(),
            coalesced,
        }),
    ))
}

async fn get_generation_job(
    State(state): State<ApiState>,
    Path((account_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<GenerationJobResource>, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    let job = state.generation_job_for_session(&account_id, session_token, &job_id)?;

    Ok(Json(job.into()))
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSourcePermissionRequest {
    permission: SourcePermission,
}

async fn update_source_permission(
    State(state): State<ApiState>,
    Path((account_id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<UpdateSourcePermissionRequest>,
) -> Result<StatusCode, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    state.update_source_permission(&account_id, session_token, &source_id, request.permission)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn keep_draft(
    State(state): State<ApiState>,
    Path((account_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    Ok(Json(state.keep_draft(
        &account_id,
        session_token,
        &draft_id,
    )?))
}

async fn edit_draft(
    State(state): State<ApiState>,
    Path((account_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<EditDraftRequest>,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    Ok(Json(state.edit_pending_draft(
        &account_id,
        session_token,
        &draft_id,
        &request.prompt,
        &request.expected_answer,
    )?))
}

async fn reject_draft(
    State(state): State<ApiState>,
    Path((account_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StudyViewResponse>, ApiFailure> {
    let session_token = read_session_token(&headers)?;
    Ok(Json(state.reject_pending_draft(
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
struct AppWaitlistForm {
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
    permission: Option<SourcePermission>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppSourceForm {
    csrf_token: Option<String>,
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
    permission: Option<SourcePermission>,
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
struct AppSourcePermissionForm {
    csrf_token: Option<String>,
    source_id: String,
    permission: SourcePermission,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppJobActionForm {
    csrf_token: Option<String>,
    job_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppDraftActionForm {
    csrf_token: Option<String>,
    draft_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppDraftEditForm {
    csrf_token: Option<String>,
    draft_id: String,
    prompt: String,
    expected_answer: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewActionForm {
    csrf_token: Option<String>,
    review_unit_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppReviewEditForm {
    csrf_token: Option<String>,
    review_unit_id: String,
    prompt: String,
    expected_answer: String,
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
    performance_trace_id: Option<String>,
}

const BROWSER_SUBMIT_PERFORMANCE_SCHEMA: &str = "memory_engine.browser_submit.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum BrowserSubmitViewport {
    Mobile,
    Tablet,
    Desktop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserSubmitPerformance {
    schema: String,
    csrf_token: Option<String>,
    request_id: String,
    trace_id: String,
    tap_to_ack_ms: u64,
    request_to_response_ms: u64,
    transfer_ms: u64,
    navigation_ms: u64,
    graded_visible_ms: u64,
    viewport: BrowserSubmitViewport,
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
        Err(error) => app_failure_response(&error),
    })
}

async fn create_app_waitlist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppWaitlistForm>,
) -> Response {
    let result = state.join_waitlist(&form.email, "first-run", &client_rate_limit_key(&headers));

    no_store_response(match result {
        Ok(()) => Html(render_waitlist_joined()).into_response(),
        Err(error) => waitlist_failure_response(&error),
    })
}

fn waitlist_failure_response(error: &ApiFailure) -> Response {
    let status = match error.status() {
        StatusCode::BAD_REQUEST => StatusCode::BAD_REQUEST,
        StatusCode::TOO_MANY_REQUESTS => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    let (title, message) = match status {
        StatusCode::BAD_REQUEST => (
            "Check the email address",
            "That email address is not valid. Check it and try again.",
        ),
        StatusCode::TOO_MANY_REQUESTS => (
            "Please try again later",
            "We’re taking a short break from new joins. Please wait a little while, then try again.",
        ),
        _ => (
            "We couldn’t save that",
            "We couldn’t save your request right now. Please try again shortly.",
        ),
    };
    let mut response = Html(render_waitlist_recovery(title, message)).into_response();
    *response.status_mut() = status;
    response
}

fn admin_token_from_headers(headers: &HeaderMap) -> &str {
    headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

/// Operator-only waitlist readout, gated by the admin token. Not part of the
/// versioned `/v1` contract — like the return-notification scheduler route,
/// this is internal tooling, not a public API surface.
async fn list_waitlist(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<WaitlistEntry>>, ApiFailure> {
    Ok(Json(
        state.list_waitlist(admin_token_from_headers(&headers))?,
    ))
}

/// Encode one CSV cell so opening the export in a spreadsheet cannot
/// execute attacker-controlled content as a formula (classic CSV/formula
/// injection). Any value whose first character is `=`, `+`, `-`, or `@` is
/// spreadsheet-formula-shaped in Excel/Sheets/LibreOffice, so it gets a
/// stable, deterministic `'` prefix that forces text interpretation;
/// applied uniformly regardless of which column carries attacker input.
/// This only changes the CSV wire encoding -- storage and JSON keep the
/// exact underlying value.
fn csv_field(value: &str) -> String {
    let mut value = value.to_owned();
    if value.starts_with(['=', '+', '-', '@']) {
        value.insert(0, '\'');
    }
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

/// Operator-only waitlist CSV export, gated by the admin token. Same
/// listing as `GET /internal/waitlist`; only the wire format differs, so an
/// operator can open the result directly in a spreadsheet. Anonymous callers
/// control the email column (`POST /app/waitlist`), so every cell runs
/// through `csv_field`, which neutralizes formula-leading values and quotes
/// CR/LF/comma/quote before the row is written -- opening this export can
/// never execute attacker-controlled content as a spreadsheet formula.
async fn export_waitlist(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiFailure> {
    let entries = state.list_waitlist(admin_token_from_headers(&headers))?;
    let mut csv = String::from("email,createdAtMs,updatedAtMs,source,invitedAtMs\n");
    for entry in entries {
        let invited_at_ms = entry
            .invited_at_ms
            .map_or_else(String::new, |value| value.to_string());
        let _ = writeln!(
            csv,
            "{},{},{},{},{invited_at_ms}",
            csv_field(&entry.email),
            entry.created_at_ms,
            entry.updated_at_ms,
            csv_field(&entry.source),
        );
    }
    Ok(([(CONTENT_TYPE, "text/csv; charset=utf-8")], csv).into_response())
}

#[derive(Clone, Debug, Deserialize)]
struct WaitlistEmailRequest {
    email: String,
}

/// Operator-only waitlist invite transition, gated by the admin token.
/// Idempotent: inviting an already-invited address again returns its
/// existing `invitedAtMs` unchanged.
async fn invite_waitlist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<WaitlistEmailRequest>,
) -> Result<Json<WaitlistEntry>, ApiFailure> {
    Ok(Json(state.mark_waitlist_invited(
        admin_token_from_headers(&headers),
        &request.email,
    )?))
}

#[derive(Clone, Debug, Serialize)]
struct WaitlistDeleteResponse {
    deleted: bool,
}

/// Operator-only waitlist delete, gated by the admin token. Removes only the
/// operational row; the append-only audit trail is unaffected.
async fn delete_waitlist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<WaitlistEmailRequest>,
) -> Result<Json<WaitlistDeleteResponse>, ApiFailure> {
    state.delete_waitlist_entry(admin_token_from_headers(&headers), &request.email)?;
    Ok(Json(WaitlistDeleteResponse { deleted: true }))
}

async fn verify_app_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<AppLoginVerifyQuery>,
) -> Response {
    match state.verify_magic_link_for_client(&query.token, &client_rate_limit_key(&headers)) {
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
        Err(error) => no_store_response(app_failure_response(&error)),
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
    let result = tokio::task::spawn_blocking(move || -> Result<Response, ApiFailure> {
        if let Some(token) = query.token.as_deref() {
            state.validate_return_notification_token(token)?;
            return Ok(no_store_response(
                Html(render_return_notification_confirmation(token)).into_response(),
            ));
        }
        let account = state.require_browser_session_readonly(&headers)?;
        Ok(no_store_response(
            Html(render_account_page(&state, &account, None, None)).into_response(),
        ))
    })
    .await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) if error.is_return_notification_link_invalid() => {
            return_notification_token_recovery_response(&error)
        }
        Ok(Err(error)) => no_store_response(app_failure_response(&error)),
        Err(error) => no_store_response(app_failure_response(&ApiFailure::internal(format!(
            "notification page worker join failed: {error}"
        )))),
    }
}

/// Style a return-notification (due-count reminder) token failure as a
/// direct recovery page instead of the raw JSON `app_failure_response`
/// would otherwise render. This route is reached straight from an emailed
/// link, never through the authenticated app shell, so there is no session
/// to recover — the only safe next action is back to the study space.
fn return_notification_token_recovery_response(error: &ApiFailure) -> Response {
    let status = error.status();
    let mut response = Html(render_return_notification_recovery(
        "That reminder link needs a refresh",
        &error.message,
    ))
    .into_response();
    *response.status_mut() = status;
    no_store_response(response)
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
        let token = token.to_owned();
        let result = tokio::task::spawn_blocking(move || state.disable_return_notification(&token))
            .await
            .map_err(|error| {
                ApiFailure::internal(format!("unsubscribe worker join failed: {error}"))
            })
            .and_then(|result| result);
        return no_store_response(match result {
            Ok(()) => Html(render_return_notification_disabled()).into_response(),
            Err(error) if error.is_return_notification_link_invalid() => {
                return_notification_token_recovery_response(&error)
            }
            Err(error) => app_failure_response(&error),
        });
    }
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return no_store_response(app_failure_response(&error)),
        };
    let enabled = form.enabled.as_deref() == Some("on");
    let reminder_email = form.reminder_email.clone();
    let state_for_work = state.clone();
    let account_for_work = account.clone();
    let result = tokio::task::spawn_blocking(move || {
        let due_count = state_for_work
            .app_study_view(&account_for_work)
            .map_or(0, |view| view.due_count);
        state_for_work
            .set_return_notification(&account_for_work, reminder_email.as_deref(), enabled)
            .and_then(|()| {
                if enabled {
                    state_for_work
                        .maybe_send_due_count_notification(&account_for_work, due_count, true)
                        .map(|_| ())
                } else {
                    Ok(())
                }
            })
    })
    .await
    .map_err(|error| ApiFailure::internal(format!("notification worker join failed: {error}")))
    .and_then(|result| result);
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
        Err(error) => app_failure_response(&error),
    }
}

async fn logout_all_app_sessions(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    match state.revoke_all_browser_sessions(&headers, csrf_token(form.csrf_token.as_ref())) {
        Ok(()) => html_with_cleared_browser_session(render_app_shell(None, &[], None, &[], None)),
        Err(error) => app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
        };
    let source_view = state.app_study_view(&source_account).ok();
    let result = normalize_email(&form.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))
        .and_then(|email| state.save_account(&source_account, &email));

    match result {
        Ok(account) => {
            let account = match state.create_browser_session(&account) {
                Ok(account) => account,
                Err(error) => return app_failure_response(&error),
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
    if !state.anonymous_account_creation_allowed() {
        let mut response = Html(render_auth_recovery(
            "Sign-in required",
            "Guest study spaces are disabled here. Request an invite link to continue.",
        ))
        .into_response();
        *response.status_mut() = StatusCode::FORBIDDEN;
        return no_store_response(response);
    }
    let account = match state.create_guest_account() {
        Ok(account) => account,
        Err(error) => {
            return Html(render_app_shell(None, &[], None, &[], Some(&error.message)))
                .into_response();
        }
    };
    let account = match state.create_browser_session(&account) {
        Ok(account) => account,
        Err(error) => return app_failure_response(&error),
    };
    let result = state.save_app_source(
        &account,
        &capture_request(form.title, form.body, form.capture, form.permission),
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
            Err(error) => return app_failure_response(&error),
        };
    let result = state.save_app_source(
        &account,
        &capture_request(form.title, form.body, form.capture, form.permission),
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
            Err(error) => return app_failure_response(&error),
        };
    let request = capture_request(form.title, form.body, form.capture, form.permission);
    let notice = match state.save_app_source(&account, &request) {
        Ok(source) => {
            match state.enqueue_generation_job_by_source(
                &account,
                &source.source_id,
                &request.title,
            ) {
                EnqueueOutcome::Started(_) => {
                    "Generating your cards. They'll appear below as they're ready.".to_owned()
                }
                EnqueueOutcome::AlreadyInFlight(_) => "Already generating this source.".to_owned(),
                EnqueueOutcome::Rejected(reason) | EnqueueOutcome::Unavailable(reason) => reason,
            }
        }
        Err(error) => {
            return Html(render_create_page(&state, &account, Some(&error.message))).into_response()
        }
    };

    Html(render_create_page(&state, &account, Some(&notice))).into_response()
}

fn render_save_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<(), ApiFailure>,
) -> String {
    match result {
        Ok(()) => render_library_page(
            state,
            account,
            None,
            Some("Capture saved. Create review when you are ready."),
        ),
        Err(error) => render_library_page(state, account, None, Some(&error.message)),
    }
}

fn capture_request(
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
    permission: Option<SourcePermission>,
) -> CreateSourceRequest {
    let body = capture.or(body).unwrap_or_default();
    let title = title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| infer_capture_title(&body));

    CreateSourceRequest {
        title,
        body,
        permission: permission.unwrap_or_default(),
    }
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
            Err(error) => return app_failure_response(&error),
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
        EnqueueOutcome::Started(_) => "Generating. Watch the activity log.".to_owned(),
        EnqueueOutcome::AlreadyInFlight(_) => "Already generating this source.".to_owned(),
        EnqueueOutcome::Rejected(reason) | EnqueueOutcome::Unavailable(reason) => reason,
    };

    Html(render_library_page(&state, &account, None, Some(&notice))).into_response()
}

async fn archive_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
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
            Html(render_library_page(
                &state,
                &account,
                Some(&view),
                Some(&notice),
            ))
            .into_response()
        }
        Err(error) => Html(render_library_page(
            &state,
            &account,
            None,
            Some(&error.message),
        ))
        .into_response(),
    }
}

async fn update_app_source_permission(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourcePermissionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    match state.update_app_source_permission(&account, &form.source_id, form.permission) {
        Ok(()) => Html(render_library_page(
            &state,
            &account,
            None,
            Some("Source permission updated."),
        ))
        .into_response(),
        Err(error) => Html(render_library_page(
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
            Err(error) => return app_failure_response(&error),
        };
    let notice = if state.retry_generation_job(&account, &form.job_id) {
        "Retrying. Generating again in the background."
    } else {
        "That job can't be retried."
    };

    Html(render_library_page(&state, &account, None, Some(notice))).into_response()
}

/// Live job-status stream (SSE). Pushes this account's job updates as they
/// happen so the activity log updates without a reload. The page is already
/// server-authoritative, so this is pure progressive enhancement.
async fn app_jobs_events(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let account = match state.require_browser_session_readonly(&headers) {
        Ok(account) => account,
        Err(error) => return app_failure_response(&error),
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

async fn keep_app_draft(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppDraftActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let result = state.keep_draft(
        account.account_id(),
        account.session_token(),
        &form.draft_id,
    );
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

async fn edit_app_draft(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppDraftEditForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let result = state.edit_pending_draft(
        account.account_id(),
        account.session_token(),
        &form.draft_id,
        &form.prompt,
        &form.expected_answer,
    );
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

async fn reject_app_draft(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppDraftActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let result = state.reject_pending_draft(
        account.account_id(),
        account.session_token(),
        &form.draft_id,
    );
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

async fn next_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let result = state.next_app_review(&account);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

fn client_rate_limit_key(headers: &HeaderMap) -> String {
    // DigitalOcean's edge overwrites this header from the accepted client's address before forwarding the request. Generic forwarding headers remain
    // untrusted and are intentionally ignored. Missing edge identity is one
    // deterministic bucket rather than an attacker-controlled bypass.
    headers
        .get("do-connecting-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| "unknown".to_owned(), str::to_owned)
}

fn app_failure_response(error: &ApiFailure) -> Response {
    let status = error.status();
    let (title, message) = if error.is_session_expired() {
        (
            "Your session expired",
            "Your study data is safe. Sign in again to continue where you left off.",
        )
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        (
            "Please try again later",
            "We’re taking a short break from sign-in attempts. Wait a little while, then request a fresh link.",
        )
    } else if error.is_magic_link_recovery() {
        (
            "Sign-in link expired",
            "That link is no longer valid. Request a fresh link and return to your study space.",
        )
    } else {
        (
            "We couldn’t complete sign-in",
            "We couldn’t complete that request. Request a fresh sign-in link and try again.",
        )
    };
    let mut response = Html(render_auth_recovery(title, message)).into_response();
    *response.status_mut() = status;
    no_store_response(response)
}
fn submit_recovery_response(status: StatusCode, title: &str, message: &str) -> Response {
    let mut response = Html(render_submit_recovery(title, message)).into_response();
    *response.status_mut() = status;
    no_store_response(response)
}

fn submit_failure_response(error: &ApiFailure) -> Response {
    if error.is_session_expired() {
        return app_failure_response(error);
    }
    submit_recovery_response(
        error.status(),
        "Review not submitted",
        "Reload the app and try again. Your study data is safe.",
    )
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
            Err(error) => return app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
        };
    let result = state.delete_app_review(&account, &form.review_unit_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn edit_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let view = match state.next_app_review(&account) {
        Ok(view) => view,
        Err(error) => return error.into_response(),
    };
    let Some(current) = view.current.as_ref() else {
        return ApiFailure::not_found("Review unit not found.").into_response();
    };
    if current.review_unit_id.to_string() != form.review_unit_id {
        return ApiFailure::not_found("Review unit not found.").into_response();
    }

    Html(render_edit_review_html(&state, &account, &view, None)).into_response()
}

async fn save_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewEditForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let result = state.edit_app_review(
        &account,
        &form.review_unit_id,
        &form.prompt,
        &form.expected_answer,
    );

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

async fn snooze_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
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
            Err(error) => return app_failure_response(&error),
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
    form: Result<Form<AppReviewSubmitForm>, axum::extract::rejection::FormRejection>,
) -> Response {
    let started = Instant::now();
    let request_id = format!("req_{:032x}", rand::random::<u128>());
    let form = match form {
        Ok(Form(form)) => form,
        Err(rejection) => {
            let status = rejection.into_response().status();
            let render_started = Instant::now();
            let response = submit_recovery_response(
                status,
                "Review not submitted",
                "Reload the app and try again. Your study data is safe.",
            );
            let (response, total_ms) = submit_response_headers(
                response,
                &request_id,
                None,
                bounded_request_ms(started),
                SubmitReviewTimings::default(),
                bounded_request_ms(render_started),
            );
            report_submit_server_performance(total_ms, SubmitPerformanceOutcome::ClientRejected);
            return no_store_response(response);
        }
    };
    let trace_id = form
        .performance_trace_id
        .as_deref()
        .filter(|value| strict_opaque_id(value, "trace_"))
        .map(str::to_owned);
    let mut postgres = SubmitReviewTimings::default();

    let (response, render_ms, outcome) = match state.require_browser_session_with_timings(
        &headers,
        csrf_token(form.csrf_token.as_ref()),
        &mut postgres,
    ) {
        Ok(account) => {
            let result = state.submit_app_review(
                &account,
                &form.review_unit_id,
                &SubmitReviewRequest {
                    answer: form.answer,
                    response_time_ms: sanitize_response_time_ms(form.response_time_ms.as_deref()),
                    idempotency_key: form.idempotency_key,
                },
                &mut postgres,
            );
            let outcome = match result.as_ref() {
                Ok(_) => SubmitPerformanceOutcome::Succeeded,
                Err(error) => submit_outcome(error.status()),
            };
            let render_started = Instant::now();
            let postgres_before_render = postgres;
            let response = Html(render_submit_action_result_html(
                &state,
                &account,
                result,
                &request_id,
                trace_id.as_deref(),
                &mut postgres,
            ))
            .into_response();
            let render_ms = render_only_ms(render_started, postgres_before_render, postgres);
            (response, render_ms, outcome)
        }
        Err(error) => {
            let outcome = submit_outcome(error.status());
            let render_started = Instant::now();
            let response = submit_failure_response(&error);
            (response, bounded_request_ms(render_started), outcome)
        }
    };

    let (response, total_ms) = submit_response_headers(
        response,
        &request_id,
        trace_id.as_deref(),
        bounded_request_ms(started),
        postgres,
        render_ms,
    );
    report_submit_server_performance(total_ms, outcome);
    no_store_response(response)
}

fn submit_response_headers(
    mut response: Response,
    request_id: &str,
    trace_id: Option<&str>,
    total_ms: u64,
    postgres: SubmitReviewTimings,
    render_ms: u64,
) -> (Response, u64) {
    let (total_ms, pgconnect_ms, pgop_ms, render_ms) =
        normalize_submit_durations(total_ms, postgres, render_ms);
    let mut timing = format!(r#"request;desc="{request_id}""#);
    if let Some(trace_id) = trace_id {
        let _ = write!(timing, r#", handoff;desc="{trace_id}""#);
    }
    let _ = write!(timing, ", total;dur={total_ms}");
    if let Some(duration_ms) = pgconnect_ms {
        let _ = write!(timing, ", pgconnect;dur={duration_ms}");
    }
    if let Some(duration_ms) = pgop_ms {
        let _ = write!(timing, ", pgop;dur={duration_ms}");
    }
    if let Some(statement_count) = postgres.postgres_statement_count() {
        let _ = write!(timing, r#", pgstmt;desc="{statement_count}""#);
    }
    let _ = write!(timing, ", render;dur={render_ms}");

    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(request_id).unwrap_or(HeaderValue::from_static(
            "req_00000000000000000000000000000000",
        )),
    );
    response.headers_mut().insert(
        "server-timing",
        HeaderValue::from_str(&timing).unwrap_or(HeaderValue::from_static(
            r#"request;desc="req_00000000000000000000000000000000", total;dur=60000, render;dur=60000"#,
        )),
    );
    (response, total_ms)
}

/// Postgres connect/operation and render are measured as disjoint,
/// sequential phases; each is independently clamped to at least 1ms
/// (`bounded_request_ms`), so their raw values are never rescaled or
/// otherwise fabricated here. A request whose wall-clock `total_ms` came in
/// under that floor-clamped phase sum (only possible on extremely fast
/// requests where every phase rounds up to the 1ms floor) has its `total_ms`
/// raised to the true phase sum instead: the reported total must always
/// honestly bound its own parts, and raising the coarser aggregate is honest
/// where shrinking the measured parts to fit would not be. Deliberately no
/// upper cap is applied after the raise: each phase is already independently
/// clamped to at most `60_000ms` by `bounded_request_ms`/`bounded_elapsed_ms`,
/// so a request-wide `.min(60_000)` here could put `total_ms` back *below*
/// `phase_sum` on a genuinely slow request (e.g. connect + operation + render
/// each near the 60s ceiling) — reintroducing the exact fabrication this
/// function exists to prevent.
fn normalize_submit_durations(
    total_ms: u64,
    postgres: SubmitReviewTimings,
    render_ms: u64,
) -> (u64, Option<u64>, Option<u64>, u64) {
    let phases = [
        postgres.postgres_connect_ms(),
        postgres.postgres_operation_ms(),
        Some(render_ms),
    ];
    let phase_sum = phases
        .iter()
        .flatten()
        .copied()
        .fold(0_u64, u64::saturating_add);
    let total_ms = total_ms.max(phase_sum);

    (total_ms, phases[0], phases[1], phases[2].unwrap_or(1))
}

fn strict_opaque_id(value: &str, prefix: &str) -> bool {
    value.len() == prefix.len() + 32
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded_request_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .clamp(1, 60_000)
}

/// Wall-clock render duration with any Postgres time recorded *during* the
/// render call subtracted out.
///
/// `render_submit_action_result_html` (via `render_submit_account_page`)
/// performs timed reads — `app_study_view_with_timings`,
/// `list_app_sources_with_timings`, `jobs_for_app_account_with_timings` — on
/// the error/empty-queue path, inside this same wall-clock window, while
/// accumulating their milliseconds into the shared `SubmitReviewTimings`.
/// Without subtracting that nested time back out, `normalize_submit_durations`
/// would count it twice: once as `pgconnect`/`pgop`, once folded into
/// `render`. `before`/`after` MUST be snapshots of the same accumulating
/// timings taken immediately before and after the render call; both counters
/// only ever grow, so the delta can never underflow.
fn render_only_ms(
    render_started: Instant,
    before: SubmitReviewTimings,
    after: SubmitReviewTimings,
) -> u64 {
    bounded_request_ms(render_started)
        .saturating_sub(nested_postgres_delta_ms(before, after))
        .max(1)
}

/// Postgres connect+operation milliseconds recorded strictly after `before`
/// was captured, given both are snapshots of the same monotonically
/// accumulating [`SubmitReviewTimings`].
fn nested_postgres_delta_ms(before: SubmitReviewTimings, after: SubmitReviewTimings) -> u64 {
    after
        .postgres_connect_ms()
        .unwrap_or_default()
        .saturating_sub(before.postgres_connect_ms().unwrap_or_default())
        .saturating_add(
            after
                .postgres_operation_ms()
                .unwrap_or_default()
                .saturating_sub(before.postgres_operation_ms().unwrap_or_default()),
        )
}

fn submit_outcome(status: StatusCode) -> SubmitPerformanceOutcome {
    if status.is_server_error() {
        SubmitPerformanceOutcome::ServerFailed
    } else {
        SubmitPerformanceOutcome::ClientRejected
    }
}

async fn record_submit_browser_performance(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(raw_event): Json<serde_json::Value>,
) -> Response {
    let csrf = raw_event
        .get("csrfToken")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if let Err(error) = state.require_browser_session(&headers, csrf) {
        return error.into_response();
    }
    let Ok(event) = serde_json::from_value::<BrowserSubmitPerformance>(raw_event) else {
        return ApiFailure::bad_request("Browser performance receipt is invalid.").into_response();
    };
    if !valid_browser_submit_performance(&event) {
        return ApiFailure::bad_request("Browser performance receipt is invalid.").into_response();
    }

    let viewport = match event.viewport {
        BrowserSubmitViewport::Mobile => SubmitViewport::Mobile,
        BrowserSubmitViewport::Tablet => SubmitViewport::Tablet,
        BrowserSubmitViewport::Desktop => SubmitViewport::Desktop,
    };
    report_submit_browser_performance(BrowserSubmitReceipt {
        request_id: &event.request_id,
        trace_id: &event.trace_id,
        tap_to_ack_ms: event.tap_to_ack_ms,
        request_to_response_ms: event.request_to_response_ms,
        transfer_ms: event.transfer_ms,
        navigation_ms: event.navigation_ms,
        graded_visible_ms: event.graded_visible_ms,
        viewport,
    });
    StatusCode::NO_CONTENT.into_response()
}

fn valid_browser_submit_performance(event: &BrowserSubmitPerformance) -> bool {
    if event.schema != BROWSER_SUBMIT_PERFORMANCE_SCHEMA
        || !strict_opaque_id(&event.request_id, "req_")
        || !strict_opaque_id(&event.trace_id, "trace_")
    {
        return false;
    }
    let durations = [
        event.tap_to_ack_ms,
        event.request_to_response_ms,
        event.transfer_ms,
        event.navigation_ms,
        event.graded_visible_ms,
    ];
    if durations.iter().any(|duration| *duration > 60_000)
        || event.tap_to_ack_ms > event.request_to_response_ms
        || event.request_to_response_ms > event.graded_visible_ms
    {
        return false;
    }
    let reconstructed = event
        .request_to_response_ms
        .saturating_add(event.transfer_ms)
        .saturating_add(event.navigation_ms);
    event.graded_visible_ms.abs_diff(reconstructed) <= 4
}

fn render_content_feedback_follow_up_failure(
    state: &ApiState,
    account: &AppAccount,
    error: ApiFailure,
) -> Response {
    if error.is_session_expired() {
        return app_failure_response(&error);
    }
    let status = error.status();
    let message = error.message;
    let html = match state.next_app_review(account) {
        Ok(view) => render_content_feedback_result_html(state, account, &view, &message),
        Err(_) => render_account_page(state, account, None, Some(&message)),
    };
    let mut response = Html(html).into_response();
    *response.status_mut() = status;
    response
}

pub(crate) fn resolve_content_feedback_recovery_revision(
    conflict_retry: bool,
    review_unit_id: &str,
    idempotency_key: &str,
    supersedes_id: Option<&str>,
    head_result: Option<Result<Option<String>, ApiFailure>>,
    status: &mut StatusCode,
    message: &mut String,
) -> (String, Option<String>) {
    let refreshed_head = match head_result {
        Some(Ok(head)) => Some(head),
        Some(Err(error)) => {
            *status = error.status();
            "Feedback was not saved, and the latest revision could not be loaded. Retry when storage is available.".clone_into(message);
            None
        }
        None => None,
    };
    if let Some(head) = refreshed_head {
        return (
            format!("feedback-retry-{:032x}", rand::random::<u128>()),
            head,
        );
    }
    if !conflict_retry && idempotency_key.trim().is_empty() {
        return (
            format!(
                "feedback-retry-{}-{}",
                review_unit_id,
                supersedes_id.unwrap_or("new")
            ),
            supersedes_id.map(str::to_owned),
        );
    }
    (idempotency_key.to_owned(), supersedes_id.map(str::to_owned))
}

pub(crate) fn render_content_feedback_persistence_failure(
    state: &ApiState,
    account: &AppAccount,
    review_unit_id: &str,
    request: &ContentFeedbackRequest,
    error: ApiFailure,
) -> Response {
    if error.is_session_expired() {
        return app_failure_response(&error);
    }
    let mut status = error.status();
    let mut message = error.message;
    let verdict = match request.verdict {
        memory_engine_service::ContentFeedbackVerdict::Kept => "kept",
        memory_engine_service::ContentFeedbackVerdict::Dropped => "dropped",
    };
    let conflict_retry =
        status == StatusCode::CONFLICT && !request.idempotency_key.trim().is_empty();
    let head_result =
        conflict_retry.then(|| state.app_content_feedback_head(account, review_unit_id));
    let (idempotency_key, supersedes_id) = resolve_content_feedback_recovery_revision(
        conflict_retry,
        review_unit_id,
        &request.idempotency_key,
        request.supersedes_id.as_deref(),
        head_result,
        &mut status,
        &mut message,
    );
    let html = render_content_feedback_recovery_html(
        state,
        account,
        &ContentFeedbackRecovery {
            review_unit_id,
            verdict,
            rationale: request.rationale.as_deref(),
            idempotency_key: &idempotency_key,
            supersedes_id: supersedes_id.as_deref(),
            message: &message,
        },
    );
    let mut response = Html(html).into_response();
    *response.status_mut() = status;
    no_store_response(response)
}

pub(crate) fn render_content_feedback_follow_up(
    state: &ApiState,
    account: &AppAccount,
    next_review: Result<StudyViewResponse, ApiFailure>,
) -> Response {
    match next_review {
        Ok(view) => Html(render_content_feedback_result_html(
            state,
            account,
            &view,
            "Saved. This card will help improve future generation.",
        ))
        .into_response(),
        Err(error) => render_content_feedback_follow_up_failure(state, account, error),
    }
}

async fn record_app_content_feedback(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppContentFeedbackForm>,
) -> Response {
    let account =
        match state.require_browser_session(&headers, csrf_token(form.csrf_token.as_ref())) {
            Ok(account) => account,
            Err(error) => return app_failure_response(&error),
        };
    let request = ContentFeedbackRequest {
        verdict: form.verdict,
        rationale: form.rationale,
        idempotency_key: form.idempotency_key,
        supersedes_id: form.supersedes_id,
    };
    let result = state.record_app_content_feedback(&account, &form.review_unit_id, &request);
    match result {
        Ok(_) => {
            render_content_feedback_follow_up(&state, &account, state.next_app_review(&account))
        }
        Err(error) => render_content_feedback_persistence_failure(
            &state,
            &account,
            &form.review_unit_id,
            &request,
            error,
        ),
    }
}
