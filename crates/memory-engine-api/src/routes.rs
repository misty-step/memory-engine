use axum::{
    extract::{Form, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::{
    client_rate_limit_key, csrf_token, html_with_browser_session,
    html_with_cleared_browser_session, normalize_email, read_session_token, render_account_page,
    render_action_result_html, render_app_shell, render_login_requested, AccountCreated,
    ApiFailure, ApiState, CreateAccountRequest, CreateSourceRequest, HealthResponse, SourceList,
    SourceRecord, StudyViewResponse, SubmitReviewRequest,
};

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(app_home))
        .route("/accounts", post(create_account))
        .route("/app/start", post(start_app_study))
        .route("/app/account", post(create_app_account))
        .route("/app/login/verify", get(verify_app_login))
        .route("/app/logout", post(logout_app_session))
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
struct AppLoginVerifyQuery {
    token: String,
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
    csrf_token: Option<String>,
    title: String,
    body: String,
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
struct AppDraftActionForm {
    csrf_token: Option<String>,
    draft_id: String,
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
    response_time_ms: u32,
    idempotency_key: String,
}

async fn create_app_account(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountForm>,
) -> Response {
    let result = state
        .accounts
        .request_magic_link(&form.email, &client_rate_limit_key(&headers));

    match result {
        Ok(request) => Html(render_login_requested(request.debug_link.as_deref())).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn verify_app_login(
    State(state): State<ApiState>,
    Query(query): Query<AppLoginVerifyQuery>,
) -> Response {
    match state.accounts.verify_magic_link(&query.token) {
        Ok(account) => {
            let view = state
                .accounts
                .study_view(&account.account_id, &account.session_token)
                .ok();
            html_with_browser_session(
                &account,
                render_account_page(&state, &account, view.as_ref(), None),
            )
        }
        Err(error) => error.into_response(),
    }
}

async fn logout_app_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    match state
        .accounts
        .revoke_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(()) => html_with_cleared_browser_session(render_app_shell(None, &[], None, None)),
        Err(error) => error.into_response(),
    }
}

async fn save_app_account(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSaveAccountForm>,
) -> Response {
    let source_account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let source_view = state
        .accounts
        .study_view(&source_account.account_id, &source_account.session_token)
        .ok();
    let result = normalize_email(&form.email)
        .ok_or_else(|| ApiFailure::bad_request("Account email must contain one @ and a domain."))
        .and_then(|email| {
            state.accounts.save_account(
                &source_account.account_id,
                &source_account.session_token,
                &email,
            )
        });

    match result {
        Ok(account) => {
            let account = match state.accounts.create_browser_session(&account) {
                Ok(account) => account,
                Err(error) => return error.into_response(),
            };
            let view = state
                .accounts
                .study_view(&account.account_id, &account.session_token)
                .ok()
                .or(source_view);
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
    let account = match state.accounts.create_account(&email) {
        Ok(account) => account,
        Err(error) => {
            return Html(render_app_shell(None, &[], None, Some(&error.message))).into_response();
        }
    };
    let account = match state.accounts.create_browser_session(&account) {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
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

    html_with_browser_session(
        &account,
        render_action_result_html(&state, &account, result),
    )
}

async fn create_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result = state
        .accounts
        .save_source(
            &account.account_id,
            &account.session_token,
            &CreateSourceRequest {
                title: form.title,
                body: form.body,
            },
        )
        .and_then(|_| {
            state
                .accounts
                .list_sources(&account.account_id, &account.session_token)
        });

    match result {
        Ok(sources) => Html(render_app_shell(Some(&account), &sources, None, None)).into_response(),
        Err(error) => Html(render_account_page(
            &state,
            &account,
            None,
            Some(&error.message),
        ))
        .into_response(),
    }
}

async fn generate_app_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppSourceActionForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result = state.accounts.generate_source(
        &account.account_id,
        &account.session_token,
        &form.source_id,
    );

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn approve_app_draft(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppDraftActionForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result =
        state
            .accounts
            .approve_draft(&account.account_id, &account.session_token, &form.draft_id);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn next_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppAccountActionForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result = state
        .accounts
        .next_review(&account.account_id, &account.session_token);

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn reveal_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewActionForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result = state.accounts.reveal_review(
        &account.account_id,
        &account.session_token,
        &form.review_unit_id,
    );

    Html(render_action_result_html(&state, &account, result)).into_response()
}

async fn submit_app_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<AppReviewSubmitForm>,
) -> Response {
    let account = match state
        .accounts
        .require_browser_session(&headers, csrf_token(form.csrf_token.as_ref()))
    {
        Ok(account) => account,
        Err(error) => return error.into_response(),
    };
    let result = state.accounts.submit_review(
        &account.account_id,
        &account.session_token,
        &form.review_unit_id,
        &SubmitReviewRequest {
            answer: form.answer,
            response_time_ms: form.response_time_ms,
            idempotency_key: form.idempotency_key,
        },
    );

    Html(render_action_result_html(&state, &account, result)).into_response()
}
