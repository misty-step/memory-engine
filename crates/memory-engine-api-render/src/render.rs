//! Server-rendered study UI.
//!
//! The markup consumes the Ledger design system (repo-owned at
//! `assets/ledger.css`, served from `/static/ledger.css`; the binding
//! contract is the root `DESIGN.md`): warm paper and ink, register-based
//! type, mono tabular numerals, verdicts as printed marks. The app is a
//! single adaptive screen that swaps between a workspace (capture, manage,
//! reflect) and a review (prompt, answer, grade). Every interaction is a
//! full-page form POST; `assets/app.js` layers progressive enhancement
//! (honest response timing, job SSE, the Create pending state) on top.
//! Graded pages never advance on their own — the learner reviews the verdict,
//! answer key, and dossier until they explicitly continue or submit the
//! generated-card quality feedback (operator rulings, memory-engine-081/116).

use std::fmt::Write as _;

use memory_engine_persistence::GeneratedPromptValidationStatus;
use memory_engine_study::{BetaStudyConceptProgress, BetaStudyCurrent, SourcePermission};

use memory_engine_api_state::{
    ApiFailure, ApiState, AppAccount, GenerationJob, JobStatus, SourceRecord, StudyViewResponse,
    SubmitReviewTimings,
};

pub struct ContentFeedbackRecovery<'a> {
    pub review_unit_id: &'a str,
    pub verdict: &'a str,
    pub rationale: Option<&'a str>,
    pub idempotency_key: &'a str,
    pub supersedes_id: Option<&'a str>,
    pub message: &'a str,
}

#[must_use]
pub fn render_action_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
) -> String {
    render_action_result_html_with_head(state, account, result, None, "")
}

#[must_use]
pub fn render_content_feedback_result_html(
    state: &ApiState,
    account: &AppAccount,
    view: &StudyViewResponse,
    notice: &str,
) -> String {
    if view.current.is_none() && view.summary.approved_review_unit_count > 0 {
        let inner = render_signed_in(
            account,
            &[],
            Some(view),
            &[],
            Some(notice),
            SignedInSurface::ReviewComplete,
        );
        document_with_head(&inner, "")
    } else {
        render_account_page_with_head(state, account, Some(view), Some(notice), "")
    }
}

#[must_use]
pub fn render_content_feedback_recovery_html(
    state: &ApiState,
    account: &AppAccount,
    recovery: &ContentFeedbackRecovery<'_>,
) -> String {
    let supersedes = recovery.supersedes_id.map_or_else(String::new, |id| {
        format!(
            r#"<input type="hidden" name="supersedesId" value="{}">"#,
            escape_html(id)
        )
    });
    let choice = if recovery.verdict == "dropped" {
        "Drop this card"
    } else {
        "Keep this card"
    };
    let body = format!(
        r#"<section class="ae-group me-content-feedback">
<p class="me-kicker">Feedback not saved</p>
<h1 class="me-display">Try that feedback again.</h1>
<p class="ae-lede">Selected: <strong>{choice}</strong></p>
<form action="/app/content-feedback" method="post">
{csrf}<input type="hidden" name="reviewUnitId" value="{review_unit_id}">
<input type="hidden" name="verdict" value="{verdict}">
<input type="hidden" name="idempotencyKey" value="{idempotency_key}">
{supersedes}
<label class="ae-label" for="me-content-feedback-retry-rationale">Why? <span class="ae-dim">(optional)</span></label>
<textarea class="ae-input me-content-feedback-rationale" id="me-content-feedback-retry-rationale" name="rationale" rows="2">{rationale}</textarea>
<div class="me-actions"><button class="ae-button" type="submit">Retry feedback</button></div>
</form>
</section>"#,
        choice = choice,
        csrf = hidden_csrf_input(account),
        review_unit_id = escape_html(recovery.review_unit_id),
        verdict = escape_html(recovery.verdict),
        idempotency_key = escape_html(recovery.idempotency_key),
        supersedes = supersedes,
        rationale = escape_html(recovery.rationale.unwrap_or_default()),
    );
    let due = state
        .app_study_view(account)
        .map_or(0, |view| view.due_count);
    document(&render_signed_in_body(
        account,
        due,
        Some(recovery.message),
        &[],
        &body,
    ))
}

#[must_use]
pub fn render_submit_action_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
    request_id: &str,
    trace_id: Option<&str>,
    timings: &mut SubmitReviewTimings,
) -> String {
    let trace = trace_id.map_or_else(String::new, |trace_id| {
        format!(
            r#"<meta name="memory-engine-submit-handoff" content="{}">"#,
            escape_html(trace_id)
        )
    });
    let head = format!(
        r#"<meta name="memory-engine-csrf-token" content="{}">
<meta name="memory-engine-submit-request" content="{}">
{}"#,
        escape_html(account.csrf_token()),
        escape_html(request_id),
        trace,
    );
    match result {
        Ok(view) => render_submit_account_page(state, account, Some(&view), None, &head, timings),
        Err(error) => {
            let view = state.app_study_view_with_timings(account, timings).ok();
            render_submit_account_page(
                state,
                account,
                view.as_ref(),
                Some(&error.message),
                &head,
                timings,
            )
        }
    }
}

fn render_submit_account_page(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
    head: &str,
    timings: &mut SubmitReviewTimings,
) -> String {
    let active_review = view.is_some_and(|view| view.current.is_some());
    let sources = if active_review {
        Vec::new()
    } else {
        state
            .list_app_sources_with_timings(account, timings)
            .unwrap_or_default()
    };
    let jobs = if active_review && !notice.is_some_and(is_generating_notice) {
        Vec::new()
    } else {
        state.jobs_for_app_account_with_timings(account, timings)
    };
    render_app_shell_with_head(Some(account), &sources, view, &jobs, notice, head)
}

fn render_action_result_html_with_head(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
    notice: Option<&str>,
    head: &str,
) -> String {
    match result {
        Ok(view) => render_account_page_with_head(state, account, Some(&view), notice, head),
        Err(error) => {
            render_account_page_with_head(state, account, None, Some(&error.message), head)
        }
    }
}

#[must_use]
pub fn render_account_page(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
) -> String {
    render_account_page_with_head(state, account, view, notice, "")
}

fn render_account_page_with_head(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
    head: &str,
) -> String {
    render_account_page_with_loaders_and_head(
        state,
        account,
        view,
        notice,
        head,
        || state.list_app_sources(account).unwrap_or_default(),
        || state.jobs_for_app_account(account),
    )
}

#[must_use]
pub fn render_edit_review_html(
    state: &ApiState,
    account: &AppAccount,
    view: &StudyViewResponse,
    notice: Option<&str>,
) -> String {
    let jobs = state.jobs_for_app_account(account);
    document(&render_signed_in(
        account,
        &[],
        Some(view),
        &jobs,
        notice,
        SignedInSurface::Edit,
    ))
}

#[cfg(test)]
fn render_account_page_with_loaders(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
    load_sources: impl FnOnce() -> Vec<SourceRecord>,
    load_jobs: impl FnOnce() -> Vec<GenerationJob>,
) -> String {
    render_account_page_with_loaders_and_head(
        state,
        account,
        view,
        notice,
        "",
        load_sources,
        load_jobs,
    )
}

fn render_account_page_with_loaders_and_head(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
    head: &str,
    load_sources: impl FnOnce() -> Vec<SourceRecord>,
    load_jobs: impl FnOnce() -> Vec<GenerationJob>,
) -> String {
    // When the caller doesn't supply a view (capture, generate, retry-refresh,
    // GET home), fetch the live study view so the due count and "Start review"
    // CTA reflect committed state instead of rendering an empty placeholder that
    // reads "0 due" and hides the way into review. Review screens pass their own
    // view (with an active `current`) and keep it.
    let fetched = if view.is_none() {
        state.app_study_view(account).ok()
    } else {
        None
    };
    render_account_page_with_resolved_view_and_loaders(
        account,
        view.or(fetched.as_ref()),
        notice,
        head,
        load_sources,
        load_jobs,
    )
}

fn render_account_page_with_resolved_view_and_loaders(
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
    head: &str,
    load_sources: impl FnOnce() -> Vec<SourceRecord>,
    load_jobs: impl FnOnce() -> Vec<GenerationJob>,
) -> String {
    let active_review = view.is_some_and(|view| view.current.is_some());
    // Active review responses render only the supplied card. Loading the full
    // source list here repeats a complete account snapshot while
    // `render_signed_in` never reads it in the review branch.
    let sources = if active_review {
        Vec::new()
    } else {
        load_sources()
    };
    // Jobs only affect an active review when they validate a "Generating…"
    // notice. Keep that truth check, but avoid a second Postgres connection for
    // ordinary next/submit responses where no such notice can be rendered.
    let jobs = if active_review && !notice.is_some_and(is_generating_notice) {
        Vec::new()
    } else {
        load_jobs()
    };
    render_app_shell_with_head(Some(account), &sources, view, &jobs, notice, head)
}

#[must_use]
pub fn render_app_shell(
    account: Option<&AppAccount>,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
    notice: Option<&str>,
) -> String {
    render_app_shell_with_head(account, sources, view, jobs, notice, "")
}

fn render_app_shell_with_head(
    account: Option<&AppAccount>,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
    notice: Option<&str>,
    head: &str,
) -> String {
    let inner = match account {
        Some(account) => render_signed_in(
            account,
            sources,
            view,
            jobs,
            notice,
            SignedInSurface::Workspace,
        ),
        None => render_signed_out(notice),
    };
    document_with_head(&inner, head)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnalyticsConceptFilter {
    #[default]
    All,
    AtRisk,
    Struggling,
    Mixed,
    Solid,
    Untried,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnalyticsConceptSort {
    #[default]
    Health,
    Name,
    Success,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AnalyticsViewOptions {
    pub filter: AnalyticsConceptFilter,
    pub sort: AnalyticsConceptSort,
    pub page: usize,
}

/// Render the focused, bounded Analytics read surface. The API route owns
/// authentication and query parsing; this crate owns the presentation policy
/// so the default ordering and page window cannot drift between callers.
#[must_use]
pub fn render_analytics_page(
    account: &AppAccount,
    view: &StudyViewResponse,
    options: AnalyticsViewOptions,
) -> String {
    let header_right = r#"<a class="ae-button-quiet ae-button-compact" href="/">Workspace</a>"#;
    let footer = format!(
        r#"<span class="ae-dim">Signed in</span>
<form class="me-foot-form" action="/app/logout" method="post">{}<button class="ae-button-quiet ae-button-compact" type="submit">Sign out</button></form>"#,
        hidden_csrf_input(account)
    );
    let body = format!(
        r#"<section class="me-analytics">
<p class="me-kicker">Analytics</p>
<h1 class="me-display me-analytics-title">Concept health</h1>
<p class="ae-lede ae-dim me-analytics-support">Find the concepts that need another pass, then work down the ledger.</p>
{surface}
</section>"#,
        surface = render_concept_health_surface(&view.concept_progress, options),
    );
    document(&screen(header_right, &body, &footer))
}

#[must_use]
pub fn render_login_requested(debug_link: Option<&str>) -> String {
    let debug = debug_link.map_or_else(String::new, |link| {
        format!(
            r#"<p><a href="{}" class="ae-accent">Open sign-in link</a></p>"#,
            escape_html(link)
        )
    });
    let view = format!(
        r#"<p class="ae-lede"><span class="ae-item">Check your email.</span> If that address can sign in, a link is on the way.</p>
{debug}
<p><a class="ae-accent" href="/">Back to start</a></p>"#
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_waitlist_joined() -> String {
    let view = r#"<div class="me-cover">
<p class="me-kicker">You're on the list</p>
<h1 class="me-display">Thanks for joining.</h1>
<p class="ae-lede ae-dim me-support">We’ll email you when a spot opens. No account was created and nothing else happens until then.</p>
<p><a class="ae-accent" href="/">Back to start</a></p>
</div>"#;
    document(&screen_centered("", view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_waitlist_recovery(title: &str, message: &str) -> String {
    let view = format!(
        r#"<div class="me-cover">
<p class="me-kicker">Waitlist</p>
<h1 class="me-display">{}</h1>
<p class="ae-lede ae-dim me-support">{}</p>
<section class="ae-group me-capture-hero">
<form action="/app/waitlist" method="post">
<label class="ae-label" for="me-waitlist-recovery-email">Your email</label>
<input class="ae-input me-hero-email" id="me-waitlist-recovery-email" name="email" type="email" autocomplete="email" required placeholder="you@example.com" aria-label="Email address">
<div class="me-actions"><button class="ae-button" type="submit">Try again</button></div>
</form>
</section>
<p><a class="ae-accent" href="/">Back to start</a></p>
</div>"#,
        escape_html(title),
        escape_html(message),
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_auth_recovery(title: &str, message: &str) -> String {
    let view = format!(
        r#"<div class="me-cover">
<p class="me-kicker">Return to your workspace</p>
<h1 class="me-display">{}</h1>
<p class="ae-lede ae-dim me-support">{}</p>
<section class="ae-group me-capture-hero">
<form action="/app/account" method="post">
<label class="ae-label" for="me-recovery-email">Your email</label>
<input class="ae-input me-hero-email" id="me-recovery-email" name="email" type="email" autocomplete="email" required placeholder="you@example.com" aria-label="Email address">
<div class="me-actions"><button class="ae-button" type="submit">Request a new link</button></div>
</form>
</section>
<p><a class="ae-accent" href="/">Back to start</a></p>
</div>"#,
        escape_html(title),
        escape_html(message),
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}
#[must_use]
pub fn render_submit_recovery(title: &str, message: &str) -> String {
    let view = format!(
        r#"<div class="me-cover">
<p class="me-kicker">Review safely</p>
<h1 class="me-display">{}</h1>
<p class="ae-lede ae-dim me-support">{}</p>
<p><a class="ae-button" href="/">Return to your review</a></p>
</div>"#,
        escape_html(title),
        escape_html(message),
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_return_notification_confirmation(token: &str) -> String {
    let view = format!(
        r#"<div class="me-cover">
<p class="me-kicker">Return gently</p>
<h1 class="me-display">Turn off due-count reminders?</h1>
<p class="ae-lede ae-dim me-support">This confirmation is protected by the link sent to your reminder email. No sign-in is required.</p>
<section class="ae-group me-capture-hero">
<form action="/app/return-notifications" method="post">
<input type="hidden" name="unsubscribeToken" value="{}">
<div class="me-actions"><button class="ae-button" type="submit">Turn off reminders</button></div>
</form>
</section>
<p><a class="ae-accent" href="/">Keep reminders on</a></p>
</div>"#,
        escape_html(token)
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_return_notification_disabled() -> String {
    let view = r#"<div class="me-cover">
<p class="me-kicker">Return gently</p>
<h1 class="me-display">Reminders are off.</h1>
<p class="ae-lede ae-dim me-support">You will not receive further due-count reminders. You can opt in again from your study space.</p>
<p><a class="ae-accent" href="/">Back to Scry</a></p>
</div>"#;
    document(&screen_centered("", view, FOOTER_TAGLINE))
}

#[must_use]
pub fn render_return_notification_recovery(title: &str, message: &str) -> String {
    let view = format!(
        r#"<div class="me-cover">
<p class="me-kicker">Return gently</p>
<h1 class="me-display">{}</h1>
<p class="ae-lede ae-dim me-support">{}</p>
<p><a class="ae-accent" href="/">Back to Scry</a></p>
</div>"#,
        escape_html(title),
        escape_html(message),
    );
    document(&screen_centered("", &view, FOOTER_TAGLINE))
}

/// Wrap a `.ae-screen` body in the full document, linking the design system.
fn document(inner: &str) -> String {
    document_with_head(inner, "")
}

fn document_with_head(inner: &str, head: &str) -> String {
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta name="referrer" content="no-referrer">
<meta name="theme-color" content="#f6f2ea">
<meta name="mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="default">
<title>Scry</title>
<link rel="manifest" href="/manifest.webmanifest">
<link rel="icon" href="/favicon.png" type="image/png">
<link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@100..900&family=Geist+Mono:wght@100..900&display=swap">
<link rel="stylesheet" href="/static/ledger.css">
{head}
<script src="/static/app.js" defer></script>
</head>
<body>
{inner}
</body>
</html>"##
    )
}

/// The chrome shell: a fixed top bar, a scrolling stage, a fixed bottom bar.
/// Workspace and review screens top-align and scroll inside the stage.
fn screen(header_right: &str, view: &str, footer: &str) -> String {
    screen_with("ae-stage ae-stage-scroll", header_right, view, footer)
}

/// A short, focused screen (sign-in, check-email) whose content sits centered.
fn screen_centered(header_right: &str, view: &str, footer: &str) -> String {
    screen_with("ae-stage", header_right, view, footer)
}

fn screen_with(stage: &str, header_right: &str, view: &str, footer: &str) -> String {
    format!(
        r#"<div class="ae-screen">
<header class="ae-bar">
<a class="ae-name" href="/">SCRY</a>
{header_right}
</header>
<main class="{stage}">
<div class="ae-view">
{view}
</div>
</main>
<footer class="ae-bar">
{footer}
</footer>
</div>"#
    )
}

fn render_signed_out(notice: Option<&str>) -> String {
    // Onboarding is auth-first. Accounts are required (the magic-link
    // allowlist), so anonymous visitors never see the capture form — they would
    // only dead-end on it. The display promise leads, then two actions: sign in
    // with an allowlisted email, or join the invite-beta waitlist below it. Both
    // forms read identically to a visitor who doesn't know their own allowlist
    // state, so neither response can be used to probe it.
    let view = format!(
        r#"<div class="me-cover">
{notice}
<p class="me-kicker">Scry</p>
<h1 class="me-display">Remember everything.</h1>
<p class="ae-lede ae-dim me-support">Capture anything worth remembering. We bring it back when it matters.</p>
<section class="ae-group me-capture-hero">
<form action="/app/account" method="post">
<label class="ae-label" for="me-email">Your email</label>
<input class="ae-input me-hero-email" id="me-email" name="email" type="email" autocomplete="email" required placeholder="you@example.com" aria-label="Email address">
<div class="me-actions"><button class="ae-button" type="submit">Get started</button><span class="ae-dim me-hint">No password. We’ll email a link.</span></div>
</form>
</section>
<section class="ae-group me-capture-hero">
<p class="me-kicker">New here?</p>
<form class="me-waitlist-form" action="/app/waitlist" method="post">
<label class="ae-label" for="me-waitlist-email">Your email</label>
<input class="ae-input me-hero-email" id="me-waitlist-email" name="email" type="email" autocomplete="email" required placeholder="you@example.com" aria-label="Email address">
<div class="me-actions"><button class="ae-button-quiet" type="submit">Join the waitlist</button><span class="ae-dim me-hint">We’ll email you when a spot opens. No account yet.</span></div>
<p class="me-waitlist-status ae-dim" aria-live="polite"></p>
</form>
</section>
</div>"#,
        notice = render_notice(notice, &[]),
    );
    screen("", &view, FOOTER_TAGLINE)
}

#[derive(Clone, Copy)]
enum SignedInSurface {
    Workspace,
    Edit,
    ReviewComplete,
}

fn render_signed_in(
    account: &AppAccount,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
    notice: Option<&str>,
    surface: SignedInSurface,
) -> String {
    let due = view.map_or(0, |view| view.due_count);
    let body = match surface {
        SignedInSurface::Edit => view.and_then(|view| view.current.as_ref()).map_or_else(
            || render_workspace(account, sources, view, jobs),
            |current| render_edit_review(account, current),
        ),
        SignedInSurface::ReviewComplete => render_review_complete(),
        SignedInSurface::Workspace => view.and_then(|view| view.current.as_ref()).map_or_else(
            || render_workspace(account, sources, view, jobs),
            |current| {
                let mut review = render_current_review(account, current);
                review.push_str(&render_pending_drafts(account, view));
                review
            },
        ),
    };
    render_signed_in_body(account, due, notice, jobs, &body)
}

fn render_signed_in_body(
    account: &AppAccount,
    due: usize,
    notice: Option<&str>,
    jobs: &[GenerationJob],
    body: &str,
) -> String {
    let header_right = format!(r#"<span class="me-due">{due} due</span>"#);
    let footer = format!(
        r#"<span class="ae-dim">Signed in</span>
<form class="me-foot-form" action="/app/logout" method="post">{}<button class="ae-button-quiet ae-button-compact" type="submit">Sign out</button></form>"#,
        hidden_csrf_input(account)
    );
    let view_inner = format!("{}{}", render_notice(notice, jobs), body);
    screen(&header_right, &view_inner, &footer)
}

fn render_review_complete() -> String {
    r#"<section class="ae-group me-review-complete">
<p class="me-kicker">Review complete</p>
<h1 class="me-display">You're all caught up.</h1>
<p class="ae-lede">Nothing else is due right now.</p>
<div class="me-actions"><a class="ae-button" href="/">Back to workspace</a></div>
</section>"#
        .to_owned()
}

fn render_edit_review(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    format!(
        r#"<section class="ae-group me-edit">
<p class="me-kicker">Edit card</p>
<form class="me-edit-form" action="/app/edit/save" method="post">
{csrf}
<input type="hidden" name="reviewUnitId" value="{id}">
<label class="ae-label" for="me-edit-prompt">Prompt</label>
<textarea class="ae-input" id="me-edit-prompt" name="prompt" rows="4" required>{prompt}</textarea>
<label class="ae-label" for="me-edit-answer">Answer</label>
<input class="ae-input" id="me-edit-answer" name="expectedAnswer" value="{answer}" required autocomplete="off">
<div class="me-actions"><button class="ae-button" type="submit">Save changes</button><a class="ae-button-quiet" href="/">Cancel</a></div>
</form>
</section>"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&current.review_unit_id.to_string()),
        prompt = escape_html(&current.prompt),
        answer = escape_html(&current.revision_expected_answer),
    )
}

fn render_workspace(
    account: &AppAccount,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
) -> String {
    // Generation is non-blocking. Accepted drafts stay pending until the learner
    // inspects their evidence and explicitly keeps, edits, or rejects them.
    let mut html = String::new();
    if let Some(view) = view {
        html.push_str(&render_review_status(account, view));
    }
    if sources.is_empty() && jobs.is_empty() {
        html.push_str(
            r#"<p class="ae-lede me-welcome">Type a topic or paste anything worth remembering.</p>"#,
        );
    }
    html.push_str(&render_capture(account));
    html.push_str(&render_return_notifications(account));
    html.push_str(&render_jobs(account, jobs));
    html.push_str(&render_pending_drafts(account, view));
    html.push_str(&render_sources(account, sources, jobs));
    if let Some(view) = view {
        html.push_str(&render_concept_progress(&view.concept_progress));
    }
    html
}

fn render_return_notifications(account: &AppAccount) -> String {
    format!(
        r#"<section class="ae-group me-return-channel">
<h2 class="ae-h">Return gently</h2>
<p class="ae-lede">Opt in to one quiet email a day when reviews are waiting. No scores or promotional mail.</p>
<form action="/app/return-notifications" method="post">
{csrf}<label class="ae-label" for="me-reminder-email">Reminder email</label>
<input class="ae-input" id="me-reminder-email" name="reminderEmail" type="email" autocomplete="email" required placeholder="you@example.com">
<input type="hidden" name="enabled" value="on">
<button class="ae-button" type="submit">Enable due-count reminders</button>
</form>
<form action="/app/return-notifications" method="post" class="me-return-off">
{csrf}<input type="hidden" name="enabled" value="off">
<button class="ae-button-quiet" type="submit">Turn off reminders</button>
</form>
</section>"#,
        csrf = hidden_csrf_input(account),
    )
}

fn render_notice(message: Option<&str>, jobs: &[GenerationJob]) -> String {
    let message = message.filter(|text| generating_notice_is_live(text, jobs));
    message.map_or_else(String::new, |message| {
        format!(
            r#"<p class="me-notice" role="status">{ICON_INFO}<span>{}</span></p>"#,
            escape_html(message)
        )
    })
}

/// A "Generating…" notice only tells the truth while a job it could be
/// describing is actually queued or running. Every other notice (errors,
/// confirmations like "Source removed.") is unconditional — only a
/// generation-in-progress notice needs to be checked against live job state,
/// so it never lingers once nothing is left in flight (operator dogfood
/// finding, memory-engine-081).
fn is_generating_notice(text: &str) -> bool {
    text.contains("Generating")
}

fn generating_notice_is_live(text: &str, jobs: &[GenerationJob]) -> bool {
    if !is_generating_notice(text) {
        return true;
    }
    jobs.iter()
        .any(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
}

fn render_review_status(account: &AppAccount, view: &StudyViewResponse) -> String {
    if view.due_count > 0 {
        return format!(
            r#"<section class="ae-group me-callout">
<h2 class="ae-h">Due now</h2>
<p class="me-callout-line"><span class="me-callout-n">{due_count}</span> {items} ready to review.</p>
<form action="/app/next" method="post">{csrf}<button class="ae-button" type="submit">Start review {ICON_ARROW}</button></form>
</section>"#,
            due_count = view.due_count,
            items = plural(view.due_count, "item", "items"),
            csrf = hidden_csrf_input(account),
        );
    }
    // Caught up: only worth saying once the learner actually has reviews.
    if view.summary.approved_review_unit_count > 0 {
        return format!(
            r#"<section class="ae-group me-caughtup"><p class="me-caughtup-line">{ICON_OK}<span class="ae-item">You're all caught up.</span></p><p class="ae-dim me-hint">Nothing is due right now. Add more below, or come back later.</p></section>"#
        );
    }
    String::new()
}

fn render_capture(account: &AppAccount) -> String {
    // One action that returns immediately: typing and pressing the button
    // enqueues a background job, so the learner is free to create more or
    // review while cards generate. Progress shows in the activity log below.
    format!(
        r#"<section class="ae-group me-capture">
<form class="me-capture-form" action="/app/capture" method="post">
{csrf}
<label class="ae-label me-capture-label" for="me-capture">What do you want to remember?</label>
<textarea class="ae-input" id="me-capture" name="capture" rows="3" required placeholder="A topic like “NATO phonetic alphabet”, a list, or pasted notes."></textarea>
<label class="ae-label" for="me-capture-permission">Permission</label>
<select class="ae-input" id="me-capture-permission" name="permission" aria-describedby="me-capture-permission-hint"><option value="model-eligible" selected>Allow model help</option><option value="local-only">Keep local / Never send to a model</option></select>
<p class="ae-dim me-hint" id="me-capture-permission-hint">Allow model help is the default. Choose keep local / never send to a model to prevent model providers from receiving this capture.</p>
<div class="me-actions"><button class="ae-button" type="submit">Create {ICON_ARROW}</button><span class="ae-dim me-hint me-live-hint">Generates in the background.</span></div>
</form>
</section>"#,
        csrf = hidden_csrf_input(account),
    )
}

fn render_pending_drafts(account: &AppAccount, view: Option<&StudyViewResponse>) -> String {
    let Some(view) = view else {
        return String::new();
    };
    let pending = view.drafts.iter().filter(|draft| {
        !draft.approved
            && draft.learner_decision.is_none()
            && draft.validation_status == GeneratedPromptValidationStatus::Accepted
    });
    let mut rows = String::new();
    for draft in pending {
        let spans = if draft.source_spans.is_empty() {
            String::new()
        } else {
            let mut rendered = String::from("<ul class=\"me-provenance-spans\">");
            for span in &draft.source_spans {
                let _ = write!(
                    rendered,
                    "<li><strong>{}</strong> · {} <span class=\"ae-dim\">{}</span></li>",
                    escape_html(&span.label),
                    escape_html(&span.text),
                    escape_html(&span.locator)
                );
            }
            rendered.push_str("</ul>");
            rendered
        };
        let provenance = draft.provenance.as_ref().map_or_else(String::new, |p| {
            format!(
                "<p class=\"ae-dim me-draft-provenance\">Provider: {} · Model: {}{}</p>",
                escape_html(&p.provider),
                escape_html(&p.model),
                p.prompt_version
                    .as_deref()
                    .map_or_else(String::new, |v| format!(" · Prompt {}", escape_html(v)))
            )
        });
        let _ = write!(
            rows,
            r#"<article class="me-pending-draft">
<p class="me-kicker">Pending draft</p>
<h3 class="ae-h">{}</h3>
<p class="me-prompt">{}</p>
<p class="ae-dim">Expected answer: <span class="ae-item">{}</span></p>
{}
{}
<form action="/app/draft/edit" method="post">{}<input type="hidden" name="draftId" value="{}"><label class="ae-label" for="draft-prompt-{}">Edit prompt</label><textarea class="ae-input" id="draft-prompt-{}" name="prompt" rows="3" required>{}</textarea><label class="ae-label" for="draft-answer-{}">Edit answer</label><input class="ae-input" id="draft-answer-{}" name="expectedAnswer" value="{}" required><div class="me-actions"><button class="ae-button" type="submit">Edit and keep</button></div></form>
<div class="me-row-actions"><form action="/app/draft/keep" method="post">{}<input type="hidden" name="draftId" value="{}"><button class="ae-button-quiet ae-button-compact" type="submit">Keep as written</button></form><form action="/app/draft/reject" method="post">{}<input type="hidden" name="draftId" value="{}"><button class="ae-button-quiet ae-button-compact" type="submit">Reject</button></form></div>
</article>"#,
            escape_html(&draft.concept_label),
            escape_html(&draft.prompt),
            escape_html(&draft.answer),
            provenance,
            spans,
            hidden_csrf_input(account),
            escape_html(&draft.id),
            escape_html(&draft.id),
            escape_html(&draft.id),
            escape_html(&draft.prompt),
            escape_html(&draft.id),
            escape_html(&draft.id),
            escape_html(&draft.answer),
            hidden_csrf_input(account),
            escape_html(&draft.id),
            hidden_csrf_input(account),
            escape_html(&draft.id)
        );
    }
    if rows.is_empty() {
        return String::new();
    }
    format!("<section class=\"ae-group me-pending-drafts\"><h2 class=\"ae-h\">Review generated drafts</h2><p class=\"ae-lede ae-dim\">Nothing enters your queue until you choose.</p>{rows}</section>")
}

fn render_sources(
    account: &AppAccount,
    sources: &[SourceRecord],
    jobs: &[GenerationJob],
) -> String {
    if sources.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for source in sources {
        let generate = if source_generation_in_progress_or_done(source, jobs) {
            String::new()
        } else {
            format!(
                r#"<form action="/app/generate" method="post">{csrf_generate}<input type="hidden" name="sourceId" value="{id}"><button class="ae-button ae-button-compact" type="submit" title="Turn this material into review cards.">Generate cards</button></form>"#,
                csrf_generate = hidden_csrf_input(account),
                id = escape_html(&source.source_id),
            )
        };
        let permission = match &source.permission {
            SourcePermission::LocalOnly => {
                "<span class=\"ae-dim me-source-permission\">Local only · never sent to a model</span>"
            }
            SourcePermission::ModelEligible => {
                "<span class=\"ae-dim me-source-permission\">Model eligible</span>"
            }
        };
        let edit_permission = format!(
            r#"<form class="me-source-permission" action="/app/source/permission" method="post">{csrf}<input type="hidden" name="sourceId" value="{id}"><label class="ae-label" for="permission-{id_label}">Change permission</label><select class="ae-input" id="permission-{id_label}" name="permission" aria-label="Change permission for {title_attr}"><option value="model-eligible" {model_selected}>Allow model help</option><option value="local-only" {local_selected}>Keep local / Never send to a model</option></select><button class="ae-button-quiet ae-button-compact" type="submit">Save permission</button></form>"#,
            csrf = hidden_csrf_input(account),
            id = escape_html(&source.source_id),
            id_label = escape_html(&source.source_id),
            title_attr = escape_html(&source.title),
            model_selected = if source.permission == SourcePermission::ModelEligible {
                "selected"
            } else {
                ""
            },
            local_selected = if source.permission == SourcePermission::LocalOnly {
                "selected"
            } else {
                ""
            },
        );
        let _ = write!(
            rows,
            r#"<article class="me-source">
<p class="ae-item">{title}</p>
{permission}
{edit_permission}
<div class="me-row-actions">
{generate}
<details class="me-remove-confirm">
<summary class="ae-button-quiet ae-button-compact" title="Remove this saved material.">Remove</summary>
<p class="me-remove-warning">This removes the material and stops every card generated from it, across every generation run, from being reviewed.</p>
<form action="/app/source/archive" method="post">{csrf_archive}<input type="hidden" name="sourceId" value="{id_archive}"><button class="ae-button-quiet ae-button-compact" type="submit">Remove permanently</button></form>
</details>
</div>
</article>"#,
            title = escape_html(&source.title),
            permission = permission,
            csrf_archive = hidden_csrf_input(account),
            id_archive = escape_html(&source.source_id),
        );
    }

    format!(
        r#"<section class="ae-group me-material"><h2 class="ae-h">Saved material</h2>{rows}</section>"#
    )
}

/// A source whose generation is already queued, running, or done never
/// offers "Generate cards" again — the operator's first dogfood session hit
/// a duplicate generation run by tapping it while a job for the same source
/// was still in flight (memory-engine-081; the server-side duplicate guard
/// is memory-engine-082).
fn source_generation_in_progress_or_done(source: &SourceRecord, jobs: &[GenerationJob]) -> bool {
    jobs.iter().any(|job| {
        job.source_id == source.source_id
            && matches!(
                job.status,
                JobStatus::Queued | JobStatus::Running | JobStatus::Succeeded
            )
    })
}

/// The activity log: one row per background generation job, newest first.
///
/// Server-rendered and authoritative on every full load (works with JS off);
/// `app.js` enhances it to update live over SSE. Each row carries
/// `data-job-id` + `data-status` so the script can patch a single row in place,
/// and CSS drives the status glyph + retry visibility off `data-status`.
fn render_jobs(account: &AppAccount, jobs: &[GenerationJob]) -> String {
    if jobs.is_empty() {
        return String::new();
    }
    let mut rows = String::new();
    for job in jobs {
        rows.push_str(&render_job_row(account, job));
    }
    format!(
        r#"<section class="ae-group me-jobs"><h2 class="ae-h">Activity</h2><ul id="me-jobs" class="me-jobs-list">{rows}</ul></section>"#
    )
}

fn render_job_row(account: &AppAccount, job: &GenerationJob) -> String {
    format!(
        r#"<li class="me-job" data-job-id="{id}" data-status="{status}">
<span class="me-job-glyphs" aria-hidden="true"><span class="g-queued"></span><span class="g-running"><span class="me-spinner"></span></span><span class="g-succeeded"></span><span class="g-failed"></span></span>
<div class="me-job-body">
<p class="me-job-title">{title}</p>
<p class="me-job-meta">{meta}</p>
</div>
{retry}
</li>"#,
        id = escape_html(&job.id),
        status = job.status.as_str(),
        title = escape_html(&job.title),
        meta = job_meta(job),
        retry = render_job_retry(account, job),
    )
}

/// Retry only ever makes sense once a job has actually failed — the
/// operator's first dogfood session hit an unstyled Retry button rendered
/// next to a RUNNING job (memory-engine-081). A queued or running job has
/// nothing to retry, so it renders no control at all; `app.js`'s SSE
/// enhancement never adds one either (the list is server-authoritative, so a
/// job that fails live gets its Retry button on the next full page load).
fn render_job_retry(account: &AppAccount, job: &GenerationJob) -> String {
    if job.status != JobStatus::Failed || !job.retryable {
        return String::new();
    }
    format!(
        r#"<form class="me-job-retry" action="/app/jobs/retry" method="post">{csrf}<input type="hidden" name="jobId" value="{id}"><button class="me-job-retry-btn" type="submit">Retry</button></form>"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&job.id),
    )
}

/// The human meta line for a job's current status. Kept in sync with the
/// `metaFor` switch in `app.js`, which recomputes it on each SSE update.
fn job_meta(job: &GenerationJob) -> String {
    match job.status {
        JobStatus::Queued => "Queued…".to_owned(),
        JobStatus::Running => "Generating cards…".to_owned(),
        JobStatus::Retry => "Retrying after a temporary failure…".to_owned(),
        JobStatus::Succeeded => {
            "Generation succeeded; accepted drafts are pending your review.".to_owned()
        }
        JobStatus::Failed => escape_html(
            job.error
                .as_deref()
                .unwrap_or("Generation failed. Try again."),
        ),
    }
}

fn render_current_review(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.grade.is_some() {
        render_graded_review(account, current)
    } else {
        render_answering(account, current)
    }
}

/// Before grading: the prompt and the answer mechanism (clickable options or a
/// free-response box), plus the reveal and the escape hatches. The question owns
/// the screen — nothing reflective competes with it. If the learner used "Reveal
/// answer", the answer shows in place (the reveal button is then gone) while they
/// can still answer.
fn render_answering(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    let revealed = current
        .expected_answer
        .as_deref()
        .map_or_else(String::new, |answer| {
            format!(
                r#"<p class="me-answer"><span class="me-answer-label">Answer</span><span class="ae-item">{}</span></p>"#,
                escape_html(answer)
            )
        });
    format!(
        r#"<p class="me-prompt">{prompt}</p>
{answer_block}
{revealed}
{reference}
<div class="me-hatch-row">
{reveal}
{escape_hatches}
</div>"#,
        prompt = escape_html(&current.prompt),
        answer_block = render_answer_block(account, current),
        reference = render_reference(current),
        reveal = render_reveal_form(account, current),
        escape_hatches = render_escape_hatches(account, current),
    )
}

/// After grading: stay close to the card the learner just answered. The chosen
/// answer is revealed in place (the correct option marked, the rest dimmed; or a
/// one-line answer for free response), the verdict reads, one quiet line says
/// when it returns, and Next is the primary action. The per-item metrics and
/// concept health that used to pile up here live on the workspace, off the
/// per-card loop — so the review stays a fast, low-friction rhythm.
fn render_graded_review(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    format!(
        r#"<p class="me-prompt">{prompt}</p>
{reveal}
{verdict}
{meta}
{content_feedback}
{reference}
{next}"#,
        prompt = escape_html(&current.prompt),
        reveal = render_answer_reveal(current),
        verdict = render_graded_verdict(current),
        meta = render_meta_ledger(current),
        content_feedback = render_content_feedback(account, current),
        reference = render_reference(current),
        next = render_next(account, current),
    )
}

/// Capture a binary judgment about the generated card after the learner has
/// seen its answer. The stable id makes an accidental double submit a replay,
/// while a later client can supply `supersedesId` for a revised judgment.
fn render_content_feedback(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.grade.is_none() {
        return String::new();
    }
    let reps = current.review_state.as_ref().map_or(0, |state| state.reps);
    let head_id = current.content_feedback_head_id.as_deref().unwrap_or("new");
    let feedback_id = format!("feedback-{}-{reps}-{head_id}", current.review_unit_id);
    let supersedes = current
        .content_feedback_head_id
        .as_ref()
        .map(|head_id| {
            format!(
                r#"<input type="hidden" name="supersedesId" value="{}">"#,
                escape_html(head_id)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<section class="me-content-feedback" aria-labelledby="me-content-feedback-title">
<h2 class="ae-h" id="me-content-feedback-title">This item:</h2>
<p class="ae-dim">Was this generated card worth keeping?</p>
<form action="/app/content-feedback" method="post">
{csrf}<input type="hidden" name="reviewUnitId" value="{review_unit_id}">
<input type="hidden" name="idempotencyKey" value="{feedback_id}">
{supersedes}
<div class="me-feedback-actions">
<button class="ae-button ae-button-quiet" type="submit" name="verdict" value="kept" aria-label="Keep this card">👍 Keep</button>
<button class="ae-button ae-button-quiet" type="submit" name="verdict" value="dropped" aria-label="Drop this card">👎 Drop</button>
</div>
<label class="ae-label" for="me-content-feedback-rationale">Why? <span class="ae-dim">(optional)</span></label>
<textarea class="ae-input me-content-feedback-rationale" id="me-content-feedback-rationale" name="rationale" rows="2" placeholder="A quick note for future improvements…"></textarea>
</form>
</section>"#,
        csrf = hidden_csrf_input(account),
        review_unit_id = escape_html(&current.review_unit_id.to_string()),
        feedback_id = escape_html(&feedback_id),
        supersedes = supersedes,
    )
}

/// The card's dossier — post-grade only (DESIGN.md): stage, last seen, and
/// success record, plus the concept line when the grade rolled one up. This
/// never renders pre-grade; before the answer the question owns the screen.
fn render_meta_ledger(current: &BetaStudyCurrent) -> String {
    let Some(feedback) = current.feedback.as_ref() else {
        return String::new();
    };
    let history = &feedback.item_history;
    let mut rows = String::new();
    let _ = write!(
        rows,
        r"<div><dt>Stage</dt><dd>{}</dd></div>",
        escape_html(&history.stage)
    );
    let _ = write!(
        rows,
        r"<div><dt>Last seen</dt><dd>{}</dd></div>",
        escape_html(&history.last_seen_summary)
    );
    let _ = write!(
        rows,
        r"<div><dt>Success</dt><dd>{} · {}</dd></div>",
        escape_html(&history.success_rate),
        escape_html(&history.trend)
    );
    if let Some(concept) = feedback.concept_progress.as_ref() {
        let _ = write!(
            rows,
            r"<div><dt>Concept</dt><dd>{} · {}</dd></div>",
            escape_html(&concept.concept_label),
            escape_html(&concept.health)
        );
    }
    format!(r#"<dl class="me-meta-ledger">{rows}</dl>"#)
}

/// Reveal the answer in place. Multiple-choice marks the correct option and dims
/// the rest; free response shows the correct answer on one line.
fn render_answer_reveal(current: &BetaStudyCurrent) -> String {
    let correct = current
        .expected_answer
        .as_deref()
        .or_else(|| {
            current
                .feedback
                .as_ref()
                .map(|f| f.expected_answer.as_str())
        })
        .unwrap_or_default();
    if current.choices.is_empty() {
        return format!(
            r#"<p class="me-answer"><span class="me-answer-label">Answer</span><span class="ae-item">{}</span></p>"#,
            escape_html(correct)
        );
    }

    let mut rows = String::new();
    for choice in &current.choices {
        let is_correct = choice == correct;
        let class = if is_correct {
            "me-graded-choice me-graded-choice-correct"
        } else {
            "me-graded-choice me-graded-choice-dim"
        };
        let mark = if is_correct { ICON_OK } else { "" };
        let _ = write!(
            rows,
            r#"<li class="{class}"><span>{}</span>{mark}</li>"#,
            escape_html(choice)
        );
    }
    format!(r#"<ol class="me-choices me-choices-graded">{rows}</ol>"#)
}

/// The verdict (the moment) plus one quiet line on when the card returns.
fn render_graded_verdict(current: &BetaStudyCurrent) -> String {
    let when = current
        .feedback
        .as_ref()
        .map(|feedback| feedback.item_history.next_review.as_str())
        .filter(|phrase| !phrase.is_empty())
        .map_or_else(String::new, |phrase| {
            format!(
                r#"<p class="me-next-when ae-dim">{}</p>"#,
                escape_html(phrase)
            )
        });
    format!("{}{when}", render_verdict(current))
}

/// The answer mechanism, chosen by card shape. Pre-grade, multiple-choice cards
/// get clickable option buttons (no typing, no letter-guessing) and free-
/// response cards get a prominent text box. Post-grade, multiple-choice cards
/// show a read-only recap of the options.
fn render_answer_block(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.choices.is_empty() {
        render_free_response_form(account, current)
    } else {
        render_choice_buttons(account, current)
    }
}

/// Clickable multiple-choice options. Each button submits its exact choice text,
/// so grading matches without the learner typing or mapping an option letter.
fn render_choice_buttons(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    let mut buttons = String::new();
    for choice in &current.choices {
        let _ = write!(
            buttons,
            r#"<button class="me-choice" type="submit" name="answer" value="{value}">{label}</button>"#,
            value = escape_html(choice),
            label = escape_html(choice),
        );
    }
    format!(
        r#"<form class="me-choices-form" action="/app/submit" method="post">{hidden}{buttons}</form>"#,
        hidden = review_submit_fields(account, current),
    )
}

/// Free-response answer field. This is the whole interaction for a non-MCQ card,
/// so it is a clearly bounded, labelled box — not the hairline underline that
/// read as a divider.
fn render_free_response_form(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    format!(
        r#"<form class="me-submit" action="/app/submit" method="post">{hidden}
<label class="ae-label" for="me-answer">Your answer</label>
<input class="ae-input me-answer-input" id="me-answer" name="answer" required autocomplete="off" autocapitalize="off" placeholder="Type your answer…">
<div class="me-actions"><button class="ae-button" type="submit">Answer</button></div>
</form>"#,
        hidden = review_submit_fields(account, current),
    )
}

/// The hidden fields every `/app/submit` form carries.
fn review_submit_fields(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    // The idempotency key must be unique per review *attempt*, not per card: a
    // card is reviewed many times over its life, and applied-review receipts are
    // persisted, so a key keyed only on the review unit id collides on the
    // second-ever review ("Duplicate applied review"). The rep count is the
    // per-attempt discriminator — stable across an accidental double-submit of
    // one answer (so that stays idempotent), and incremented before the card is
    // shown again — so each attempt gets its own key. Fresh card: reps 0.
    // The response time ships blank on purpose: app.js fills in the real
    // presentation-to-submit elapsed milliseconds at the moment of submission,
    // and the server grades a blank (or otherwise unvouchable) value
    // conservatively — it can never rate `Easy`. A fabricated constant here
    // once made every mature correct answer look like fast recall.
    let reps = current.review_state.as_ref().map_or(0, |state| state.reps);
    format!(
        r#"{csrf}
<input type="hidden" name="reviewUnitId" value="{id}">
<input type="hidden" name="responseTimeMs" value="">
<input type="hidden" name="idempotencyKey" value="review-{id}-{reps}">"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&current.review_unit_id.to_string()),
    )
}

fn render_reference(current: &BetaStudyCurrent) -> String {
    current
        .reference_text
        .as_ref()
        .map_or_else(String::new, |reference| {
            format!(
                r#"<div class="me-reference"><h2 class="ae-h">Reference</h2><p>{}</p></div>"#,
                escape_html(reference)
            )
        })
}

fn render_verdict(current: &BetaStudyCurrent) -> String {
    current.grade.as_ref().map_or_else(String::new, |grade| {
        format!(
            r#"<p class="me-result">{icon}<span class="me-verdict">{label}</span></p>"#,
            icon = verdict_icon(grade.verdict),
            label = verdict_label(grade.verdict),
        )
    })
}

fn render_next(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.grade.is_none() {
        return String::new();
    }
    // Operator ruling (memory-engine-081, live dogfood): a graded page never
    // advances on its own. The learner reviews the verdict, answer key, and
    // dossier until they explicitly advance — Continue (or Enter while it is
    // focused) is the only way forward, correct or not. This reverses the
    // two-speed auto-advance shipped in memory-engine-078.
    format!(
        r#"<form class="me-next" action="/app/next" method="post">{csrf}<button class="ae-button" type="submit">Continue {ICON_ARROW}</button></form>"#,
        csrf = hidden_csrf_input(account),
    )
}

fn render_concept_progress(concepts: &[BetaStudyConceptProgress]) -> String {
    if concepts.is_empty() {
        return String::new();
    }

    let preview_count = concepts.len().min(WORKSPACE_CONCEPT_PREVIEW_SIZE);
    let rows = concepts
        .iter()
        .take(WORKSPACE_CONCEPT_PREVIEW_SIZE)
        .map(render_concept_row)
        .collect::<String>();

    format!(
        r#"<section class="ae-group me-concepts"><h2 class="ae-h"><span>Concept health</span><a class="me-concepts-link" href="/app/analytics">View analytics {ICON_ARROW}</a></h2><p class="me-concepts-summary ae-dim">Showing {preview_count} of {total} {concepts}; Analytics holds the full ledger.</p>{rows}</section>"#,
        preview_count = preview_count,
        total = concepts.len(),
        concepts = plural(concepts.len(), "concept", "concepts"),
    )
}

const ANALYTICS_PAGE_SIZE: usize = 12;
const WORKSPACE_CONCEPT_PREVIEW_SIZE: usize = 6;

fn render_concept_health_surface(
    concepts: &[BetaStudyConceptProgress],
    options: AnalyticsViewOptions,
) -> String {
    let mut concepts = concepts
        .iter()
        .filter(|concept| concept_matches_filter(concept, options.filter))
        .collect::<Vec<_>>();
    concepts.sort_by(|left, right| compare_concepts(left, right, options.sort));

    let total = concepts.len();
    let page_count = total.div_ceil(ANALYTICS_PAGE_SIZE).max(1);
    let page = options.page.max(1).min(page_count);
    let start = (page - 1) * ANALYTICS_PAGE_SIZE;
    let end = (start + ANALYTICS_PAGE_SIZE).min(total);
    let rows = concepts[start..end]
        .iter()
        .map(|concept| render_concept_row(concept))
        .collect::<String>();

    let count = if total == 0 {
        r#"<p class="me-analytics-empty ae-dim">No concepts match this filter.</p>"#.to_owned()
    } else {
        format!(
            r#"<p class="me-analytics-count ae-dim">Showing {}–{} of {} {}</p>"#,
            start + 1,
            end,
            total,
            concept_count_label(total, options.filter),
        )
    };
    let controls = render_analytics_controls(options);
    let pagination = render_analytics_pagination(page, page_count, options);
    let list = if rows.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="me-analytics-list">{rows}</div>"#)
    };

    format!(
        r#"<section class="ae-group me-analytics-group">
{controls}
{count}
{list}
{pagination}
</section>"#,
    )
}

fn render_analytics_controls(options: AnalyticsViewOptions) -> String {
    format!(
        r#"<form class="me-analytics-controls" action="/app/analytics" method="get">
<label class="ae-label" for="me-analytics-filter">Health<select class="ae-input me-analytics-select" id="me-analytics-filter" name="filter">
{filter_options}
</select></label>
<label class="ae-label" for="me-analytics-sort">Sort<select class="ae-input me-analytics-select" id="me-analytics-sort" name="sort">
{sort_options}
</select></label>
<button class="ae-button ae-button-compact" type="submit">Apply</button>
</form>"#,
        filter_options = analytics_filter_options(options.filter),
        sort_options = analytics_sort_options(options.sort),
    )
}

fn analytics_filter_options(selected: AnalyticsConceptFilter) -> String {
    let mut options = String::new();
    for (filter, value, label) in [
        (AnalyticsConceptFilter::All, "all", "All concepts"),
        (AnalyticsConceptFilter::AtRisk, "at-risk", "At risk"),
        (
            AnalyticsConceptFilter::Struggling,
            "struggling",
            "Struggling",
        ),
        (AnalyticsConceptFilter::Mixed, "mixed", "Mixed"),
        (AnalyticsConceptFilter::Solid, "solid", "Solid"),
        (AnalyticsConceptFilter::Untried, "untried", "Untried"),
    ] {
        let selected_attribute = if filter == selected { " selected" } else { "" };
        let _ = write!(
            options,
            r#"<option value="{value}"{selected_attribute}>{label}</option>"#
        );
    }
    options
}

fn analytics_sort_options(selected: AnalyticsConceptSort) -> String {
    let mut options = String::new();
    for (sort, value, label) in [
        (
            AnalyticsConceptSort::Health,
            "health",
            "Health · at risk first",
        ),
        (AnalyticsConceptSort::Name, "name", "Name"),
        (AnalyticsConceptSort::Success, "success", "Success rate"),
    ] {
        let selected_attribute = if sort == selected { " selected" } else { "" };
        let _ = write!(
            options,
            r#"<option value="{value}"{selected_attribute}>{label}</option>"#
        );
    }
    options
}

fn render_analytics_pagination(
    page: usize,
    page_count: usize,
    options: AnalyticsViewOptions,
) -> String {
    if page_count <= 1 {
        return String::new();
    }
    let previous = if page > 1 {
        format!(
            r#"<a class="ae-button-quiet ae-button-compact" href="{}">Previous</a>"#,
            analytics_page_href(options, page - 1)
        )
    } else {
        r#"<span class="me-pagination-spacer" aria-hidden="true"></span>"#.to_owned()
    };
    let next = if page < page_count {
        format!(
            r#"<a class="ae-button-quiet ae-button-compact" href="{}">Next</a>"#,
            analytics_page_href(options, page + 1)
        )
    } else {
        r#"<span class="me-pagination-spacer" aria-hidden="true"></span>"#.to_owned()
    };
    format!(
        r#"<nav class="me-analytics-pagination" aria-label="Concept pages">{previous}<span>{page} of {page_count}</span>{next}</nav>"#,
    )
}

fn analytics_page_href(options: AnalyticsViewOptions, page: usize) -> String {
    format!(
        "/app/analytics?filter={}&amp;sort={}&amp;page={page}",
        analytics_filter_value(options.filter),
        analytics_sort_value(options.sort),
    )
}

fn analytics_filter_value(filter: AnalyticsConceptFilter) -> &'static str {
    match filter {
        AnalyticsConceptFilter::All => "all",
        AnalyticsConceptFilter::AtRisk => "at-risk",
        AnalyticsConceptFilter::Struggling => "struggling",
        AnalyticsConceptFilter::Mixed => "mixed",
        AnalyticsConceptFilter::Solid => "solid",
        AnalyticsConceptFilter::Untried => "untried",
    }
}

fn analytics_sort_value(sort: AnalyticsConceptSort) -> &'static str {
    match sort {
        AnalyticsConceptSort::Health => "health",
        AnalyticsConceptSort::Name => "name",
        AnalyticsConceptSort::Success => "success",
    }
}

fn concept_count_label(total: usize, filter: AnalyticsConceptFilter) -> String {
    let noun = if total == 1 { "concept" } else { "concepts" };
    match filter {
        AnalyticsConceptFilter::All => noun.to_owned(),
        AnalyticsConceptFilter::AtRisk => format!("at-risk {noun}"),
        AnalyticsConceptFilter::Struggling => format!("struggling {noun}"),
        AnalyticsConceptFilter::Mixed => format!("mixed {noun}"),
        AnalyticsConceptFilter::Solid => format!("solid {noun}"),
        AnalyticsConceptFilter::Untried => format!("untried {noun}"),
    }
}

fn concept_matches_filter(
    concept: &BetaStudyConceptProgress,
    filter: AnalyticsConceptFilter,
) -> bool {
    match filter {
        AnalyticsConceptFilter::All => true,
        AnalyticsConceptFilter::AtRisk => is_at_risk(&concept.health),
        AnalyticsConceptFilter::Struggling => concept.health == "struggling",
        AnalyticsConceptFilter::Mixed => concept.health == "mixed",
        AnalyticsConceptFilter::Solid => concept.health == "solid",
        AnalyticsConceptFilter::Untried => concept.health == "untried",
    }
}

fn is_at_risk(health: &str) -> bool {
    matches!(health, "struggling" | "mixed")
}

fn compare_concepts(
    left: &BetaStudyConceptProgress,
    right: &BetaStudyConceptProgress,
    sort: AnalyticsConceptSort,
) -> std::cmp::Ordering {
    let ordering = match sort {
        AnalyticsConceptSort::Health => health_rank(&left.health).cmp(&health_rank(&right.health)),
        AnalyticsConceptSort::Name => left.concept_label.cmp(&right.concept_label),
        AnalyticsConceptSort::Success => compare_success_rate(left, right),
    };
    ordering
        .then_with(|| right.attempts.cmp(&left.attempts))
        .then_with(|| left.concept_label.cmp(&right.concept_label))
}

fn health_rank(health: &str) -> u8 {
    match health {
        "struggling" | "at risk" | "at-risk" | "weak" => 0,
        "mixed" | "watch" => 1,
        "solid" | "healthy" | "strong" => 2,
        _ => 3,
    }
}

fn compare_success_rate(
    left: &BetaStudyConceptProgress,
    right: &BetaStudyConceptProgress,
) -> std::cmp::Ordering {
    match (left.attempts == 0, right.attempts == 0) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => {
            // Compare left.correct / left.attempts with right.correct /
            // right.attempts without floating-point rounding. u128 keeps the
            // cross-products lossless for usize-sized counters.
            let left_cross = (left.correct as u128) * (right.attempts as u128);
            let right_cross = (right.correct as u128) * (left.attempts as u128);
            right_cross.cmp(&left_cross)
        }
    }
}

fn render_concept_row(concept: &BetaStudyConceptProgress) -> String {
    let pct = if concept.attempts > 0 {
        (concept.correct * 100 / concept.attempts).min(100)
    } else {
        0
    };
    format!(
        r#"<article class="me-concept" data-health="{health}">
<div class="me-concept-head"><div class="me-concept-label"><strong>{label}</strong><span class="me-health-label {fill}">{health}</span></div><span class="me-trend ae-dim">{trend_icon} {trend}</span></div>
<div class="ae-meter"><div class="ae-meter-fill {fill}" style="width:{pct}%"></div></div>
<p class="me-concept-note ae-dim">{success_rate} · {summary}</p>
</article>"#,
        label = escape_html(&concept.concept_label),
        health = escape_html(&concept.health),
        trend_icon = trend_icon(&concept.trend),
        trend = escape_html(&concept.trend),
        fill = health_fill_class(&concept.health),
        success_rate = escape_html(&concept.success_rate),
        summary = escape_html(&concept.summary),
    )
}

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

fn render_escape_hatches(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    // Only Reveal earns a permanent spot beside the card (DESIGN.md,
    // interaction law): Reference/Skip/Snooze/Bridge/Delete and the capture
    // punch-out live one tap deeper behind a single More disclosure, so
    // nothing competes with the answer. Delete stays last and visually set
    // apart so it isn't a stray tap. Every action carries a leading icon and
    // a tooltip truthful to what the route actually does (memory-engine-081:
    // Skip and Snooze were indistinguishable) — Skip is a short in-session
    // deferral (`DEFAULT_SKIP_DEFER_MS`, 15 minutes) and Snooze defers until
    // tomorrow (`DEFAULT_SNOOZE_DEFER_MS`, 24 hours); see
    // `memory_engine_study::skip_current`/`snooze_current`.
    format!(
        r#"<details class="me-more"><summary aria-label="More actions">···</summary><div class="me-more-sheet">{reference}{skip}{snooze}{concept_snooze}{bridge}{edit}<span class="me-hatch-delete">{delete}</span><a class="me-more-capture" href="/" title="Capture new material without leaving review.">{ICON_PLUS}Capture more</a></div></details>"#,
        reference = render_review_action(
            account,
            current,
            "/app/reference",
            "Reference",
            ICON_REFERENCE,
            "Show background reading for this card.",
        ),
        skip = render_review_action(
            account,
            current,
            "/app/skip",
            "Skip",
            ICON_SKIP,
            "Show later this session.",
        ),
        snooze = render_review_action(
            account,
            current,
            "/app/snooze",
            "Snooze",
            ICON_SNOOZE,
            "Hide until tomorrow.",
        ),
        concept_snooze = current
            .concept_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .map_or_else(String::new, |_| {
                render_review_action(
                    account,
                    current,
                    "/app/snooze-concept",
                    "Snooze concept",
                    ICON_SNOOZE,
                    "Hide every card for this concept until tomorrow.",
                )
            }),
        bridge = render_review_action(
            account,
            current,
            "/app/bridge",
            "Bridge",
            ICON_BRIDGE,
            "Generate easier warm-up cards, then revisit this one later.",
        ),
        edit = render_review_action(
            account,
            current,
            "/app/edit",
            "Edit",
            ICON_EDIT,
            "Correct the prompt or answer without changing review history.",
        ),
        delete = render_review_action(
            account,
            current,
            "/app/delete",
            "Delete",
            ICON_TRASH,
            "Remove this card from review for good.",
        ),
    )
}

fn render_review_action(
    account: &AppAccount,
    current: &BetaStudyCurrent,
    action: &str,
    label: &str,
    icon: &str,
    title: &str,
) -> String {
    format!(
        r#"<form action="{action}" method="post">{csrf}<input type="hidden" name="reviewUnitId" value="{id}"><button class="ae-button-quiet ae-button-compact" type="submit" title="{title}">{icon}{label}</button></form>"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&current.review_unit_id.to_string()),
        title = escape_html(title),
    )
}

fn render_reveal_form(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.expected_answer.is_some() || current.grade.is_some() {
        return String::new();
    }

    format!(
        r#"<form class="me-reveal" action="/app/reveal" method="post">{csrf}<input type="hidden" name="reviewUnitId" value="{id}"><button class="ae-button-quiet" type="submit">Reveal answer</button></form>"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&current.review_unit_id.to_string()),
    )
}

fn verdict_label(verdict: impl std::fmt::Debug) -> &'static str {
    match format!("{verdict:?}").as_str() {
        "Correct" => "Correct",
        "Close" => "Close",
        "Wrong" => "Try again",
        "Revealed" => "Revealed",
        _ => "Needs review",
    }
}

fn verdict_icon(verdict: impl std::fmt::Debug) -> &'static str {
    match format!("{verdict:?}").as_str() {
        "Correct" => ICON_OK,
        "Close" => ICON_WARN,
        "Wrong" => ICON_ERR,
        "Revealed" => ICON_REVEALED,
        _ => ICON_INFO,
    }
}

fn health_fill_class(health: &str) -> &'static str {
    match health {
        "healthy" | "strong" | "solid" => "ae-ok",
        "watch" | "mixed" => "ae-warn",
        "struggling" | "at risk" | "at-risk" | "weak" => "ae-err",
        _ => "",
    }
}

fn trend_icon(trend: &str) -> &'static str {
    match trend {
        "improving" | "rising" => ICON_UP,
        "slipping" | "declining" | "falling" => ICON_DOWN,
        _ => "",
    }
}

fn hidden_csrf_input(account: &AppAccount) -> String {
    format!(
        r#"<input type="hidden" name="csrfToken" value="{}">"#,
        escape_html(account.csrf_token())
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const FOOTER_TAGLINE: &str = r#"<span class="ae-dim">Scry — Remember everything</span>"#;

// Lucide icons (ISC), inlined for `.ae-icon`: 1.5px stroke, currentColor, no
// fill. Status hue rides the glyph; the sentence stays ink.
const ICON_OK: &str = r#"<svg class="ae-icon ae-ok" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>"#;
const ICON_WARN: &str = r#"<svg class="ae-icon ae-warn" viewBox="0 0 24 24" aria-hidden="true"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;
const ICON_ERR: &str = r#"<svg class="ae-icon ae-err" viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const ICON_REVEALED: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>"#;
const ICON_INFO: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/></svg>"#;
const ICON_ARROW: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>"#;
const ICON_UP: &str = r#"<svg class="ae-icon ae-ok" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 17 17 7"/><path d="M7 7h10v10"/></svg>"#;
const ICON_DOWN: &str = r#"<svg class="ae-icon ae-warn" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7 17 17"/><path d="M17 7v10H7"/></svg>"#;

// More-sheet action icons (memory-engine-081): each escape hatch carries a
// leading Lucide-style glyph (`.ae-icon`: 24x24 viewBox, 1.5px stroke).
const ICON_REFERENCE: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 7v14"/><path d="M3 18V6a1 1 0 0 1 1-1h5a3 3 0 0 1 3 3 3 3 0 0 1 3-3h5a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1h-6a2 2 0 0 0-2 2 2 2 0 0 0-2-2H4a1 1 0 0 1-1-1z"/></svg>"#;
const ICON_SKIP: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 5 9 7-9 7z"/><path d="M19 5v14"/></svg>"#;
const ICON_SNOOZE: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>"#;
const ICON_BRIDGE: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 19h18"/><path d="M6 19v-6a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v6"/><path d="M9 11V6"/><path d="M15 11V6"/></svg>"#;
const ICON_TRASH: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16"/><path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/><path d="M18 7l-1 13a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L6 7"/><path d="M10 11v6"/><path d="M14 11v6"/></svg>"#;
const ICON_EDIT: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/></svg>"#;
const ICON_PLUS: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14"/><path d="M5 12h14"/></svg>"#;

#[cfg(test)]
mod source_loading_tests {
    use std::cell::Cell;

    use memory_engine_api_state::{ApiState, CreateSourceRequest, EnqueueOutcome};
    use memory_engine_persistence::SourcePermission;

    use super::{render_account_page_with_loaders, render_capture};

    fn source_body() -> String {
        [
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for A?",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
        ]
        .join("\n")
    }

    #[test]
    fn active_review_loads_only_data_used_by_the_rendered_branch() {
        let state = ApiState::default();
        let created = state
            .create_account("render-loader-active@example.com")
            .unwrap();
        let account = state.create_browser_session(&created).unwrap();
        let source = state
            .save_app_source(
                &account,
                &CreateSourceRequest {
                    title: "NATO practice notes".to_owned(),
                    body: source_body(),
                    permission: SourcePermission::default(),
                },
            )
            .unwrap();
        assert!(matches!(
            state.enqueue_generation_job_by_source(&account, &source.source_id, &source.title),
            EnqueueOutcome::Started(_)
        ));
        state.run_pending_jobs_blocking();
        let pending_view = state.next_app_review(&account).unwrap();
        let active_view = state
            .keep_draft(
                account.account_id(),
                account.session_token(),
                &pending_view.drafts[0].id,
            )
            .unwrap();
        let active_source_loads = Cell::new(0);
        let active_job_loads = Cell::new(0);
        render_account_page_with_loaders(
            &state,
            &account,
            Some(&active_view),
            None,
            || {
                active_source_loads.set(active_source_loads.get() + 1);
                Vec::new()
            },
            || {
                active_job_loads.set(active_job_loads.get() + 1);
                Vec::new()
            },
        );
        assert_eq!(active_source_loads.get(), 0);
        assert_eq!(active_job_loads.get(), 0);
        render_account_page_with_loaders(
            &state,
            &account,
            Some(&active_view),
            Some("Generating review material…"),
            Vec::new,
            || {
                active_job_loads.set(active_job_loads.get() + 1);
                Vec::new()
            },
        );
        assert_eq!(
            active_job_loads.get(),
            1,
            "a generation notice still needs live jobs to suppress stale UI"
        );
        let notice_page = render_account_page_with_loaders(
            &state,
            &account,
            Some(&active_view),
            Some("That job can't be retried."),
            Vec::new,
            || {
                active_job_loads.set(active_job_loads.get() + 1);
                Vec::new()
            },
        );
        assert_eq!(
            active_job_loads.get(),
            1,
            "an unconditional notice does not require job history"
        );
        assert!(notice_page.contains("That job can't be retried."));
        assert_workspace_render_loads_all();
    }

    fn assert_workspace_render_loads_all() {
        let workspace_state = ApiState::default();
        let workspace_created = workspace_state
            .create_account("render-loader-workspace@example.com")
            .unwrap();
        let workspace_account = workspace_state
            .create_browser_session(&workspace_created)
            .unwrap();
        let workspace_source_loads = Cell::new(0);
        let workspace_job_loads = Cell::new(0);
        render_account_page_with_loaders(
            &workspace_state,
            &workspace_account,
            None,
            None,
            || {
                workspace_source_loads.set(workspace_source_loads.get() + 1);
                Vec::new()
            },
            || {
                workspace_job_loads.set(workspace_job_loads.get() + 1);
                Vec::new()
            },
        );
        assert_eq!(workspace_source_loads.get(), 1);
        assert_eq!(workspace_job_loads.get(), 1);
    }

    #[test]
    fn capture_form_exposes_an_accessible_permission_choice_and_default() {
        let state = ApiState::default();
        let account = state
            .create_account("render-permission@example.com")
            .and_then(|account| state.create_browser_session(&account))
            .expect("account");
        let html = render_capture(&account);

        assert!(html.contains(r#"id="me-capture-permission" name="permission""#));
        assert!(html.contains(r#"aria-describedby="me-capture-permission-hint""#));
        assert!(html.contains(r#"value="model-eligible" selected"#));
        assert!(html.contains("Keep local / Never send to a model"));
        assert!(html.contains("prevent model providers from receiving this capture"));
    }
}

#[cfg(test)]
mod analytics_tests {
    use memory_engine_api_state::{AppAccount, StudyViewResponse};
    use memory_engine_study::BetaStudyConceptProgress;
    use memory_engine_study::BetaStudySummary;

    use super::{
        render_concept_health_surface, AnalyticsConceptFilter, AnalyticsConceptSort,
        AnalyticsViewOptions,
    };

    fn concept(
        label: &str,
        health: &str,
        correct: usize,
        attempts: usize,
    ) -> BetaStudyConceptProgress {
        BetaStudyConceptProgress {
            concept_key: label.to_owned(),
            concept_label: label.to_owned(),
            attempts,
            correct,
            success_rate: format!("{correct} of {attempts} correct"),
            trend: "steady correct".to_owned(),
            average_response_time_ms: Some(900),
            response_time_trend: "steady".to_owned(),
            health: health.to_owned(),
            summary: format!("{label} is {health}"),
        }
    }

    #[test]
    fn analytics_surface_filters_risk_sorts_and_paginates() {
        let concepts = (0..13)
            .map(|index| {
                concept(
                    &format!("Concept {index:02}"),
                    if index == 0 { "solid" } else { "struggling" },
                    if index == 0 { 9 } else { 1 },
                    10,
                )
            })
            .collect::<Vec<_>>();

        let page = render_concept_health_surface(
            &concepts,
            AnalyticsViewOptions {
                filter: AnalyticsConceptFilter::AtRisk,
                sort: AnalyticsConceptSort::Health,
                page: 1,
            },
        );

        assert_eq!(page.matches("class=\"me-concept\"").count(), 12);
        assert!(page.contains("Concept 01"));
        assert!(!page.contains("Concept 00"));
        assert!(page.contains("Showing 1–12 of 12 at-risk concepts"));
        assert!(!page.contains("page=2"));
    }

    #[test]
    fn analytics_surface_keeps_page_two_bounded_and_preserves_controls() {
        let concepts = (0..25)
            .map(|index| concept(&format!("Concept {index:02}"), "solid", 9, 10))
            .collect::<Vec<_>>();

        let page = render_concept_health_surface(
            &concepts,
            AnalyticsViewOptions {
                filter: AnalyticsConceptFilter::All,
                sort: AnalyticsConceptSort::Name,
                page: 2,
            },
        );

        assert_eq!(page.matches("class=\"me-concept\"").count(), 12);
        assert!(!page.contains("Concept 00"));
        assert!(page.contains("Concept 12"));
        assert!(page.contains("Showing 13–24 of 25 concepts"));
        assert!(page.contains("filter=all&amp;sort=name&amp;page=1"));
        assert!(page.contains("filter=all&amp;sort=name&amp;page=3"));
    }

    #[test]
    fn analytics_pagination_keeps_a_centered_page_slot_at_both_boundaries() {
        let first = super::render_analytics_pagination(1, 3, AnalyticsViewOptions::default());
        let last = super::render_analytics_pagination(3, 3, AnalyticsViewOptions::default());

        assert!(first.contains(
            r#"<span class="me-pagination-spacer" aria-hidden="true"></span><span>1 of 3</span>"#
        ));
        assert!(last.contains(
            r#"<span>3 of 3</span><span class="me-pagination-spacer" aria-hidden="true"></span>"#
        ));
    }

    #[test]
    fn success_sort_uses_rate_not_lexicographic_counts_and_puts_untried_last() {
        let concepts = vec![
            concept("Nine of ten", "solid", 9, 10),
            concept("Perfect eight", "solid", 8, 8),
            concept("More evidence", "solid", 8, 10),
            concept("Less evidence", "solid", 4, 5),
            concept("Untried", "untried", 0, 0),
        ];

        let page = render_concept_health_surface(
            &concepts,
            AnalyticsViewOptions {
                filter: AnalyticsConceptFilter::All,
                sort: AnalyticsConceptSort::Success,
                page: 1,
            },
        );

        let perfect = page
            .find("<strong>Perfect eight</strong>")
            .expect("perfect");
        let nine = page.find("<strong>Nine of ten</strong>").expect("nine");
        let more_evidence = page
            .find("<strong>More evidence</strong>")
            .expect("more evidence");
        let less_evidence = page
            .find("<strong>Less evidence</strong>")
            .expect("less evidence");
        let untried = page.find("<strong>Untried</strong>").expect("untried");
        assert!(
            perfect < nine,
            "success rate must outrank raw correct count: {page}"
        );
        assert!(
            more_evidence < less_evidence,
            "equal rates prefer more evidence: {page}"
        );
        assert!(
            nine < untried,
            "untried concepts sort after measured concepts: {page}"
        );
    }

    #[test]
    fn analytics_page_is_a_complete_document_with_one_asset_contract() {
        let state = memory_engine_api_state::ApiState::default();
        let created = state
            .create_account("analytics-document@example.com")
            .expect("account");
        let account: AppAccount = state.create_browser_session(&created).expect("session");
        let view = StudyViewResponse {
            drafts: Vec::new(),
            current: None,
            concept_progress: Vec::new(),
            summary: BetaStudySummary {
                source_count: 0,
                accepted_draft_count: 0,
                approved_review_unit_count: 0,
                attempt_count: 0,
                last_outcome: None,
                next_review_unit_id: None,
            },
            due_count: 0,
            generation_notices: Vec::new(),
        };

        let page = super::render_analytics_page(&account, &view, AnalyticsViewOptions::default());

        assert!(page.starts_with("<!doctype html>"));
        assert!(page
            .contains(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#));
        assert!(page.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert_eq!(page.matches(r#"href="/static/ledger.css""#).count(), 1);
        assert_eq!(page.matches(r#"src="/static/app.js""#).count(), 1);
    }

    #[test]
    fn workspace_concept_health_is_a_bounded_preview_for_large_accounts() {
        let concepts = (0..20)
            .map(|index| concept(&format!("Concept {index:02}"), "struggling", 1, 10))
            .collect::<Vec<_>>();

        let page = super::render_concept_progress(&concepts);

        assert_eq!(page.matches(r#"class="me-concept""#).count(), 6);
        assert!(page.contains("Showing 6 of 20 concepts"));
        assert!(page.contains(r#"href="/app/analytics""#));
        assert!(page.contains("View analytics"));
    }

    #[test]
    fn analytics_filter_separates_untried_from_at_risk() {
        let concepts = vec![
            concept("Needs work", "struggling", 1, 10),
            concept("Needs data", "untried", 0, 0),
        ];

        let page = super::render_concept_health_surface(
            &concepts,
            AnalyticsViewOptions {
                filter: AnalyticsConceptFilter::Untried,
                sort: AnalyticsConceptSort::Health,
                page: 1,
            },
        );

        assert!(page.contains(r#"value="untried" selected>Untried</option>"#));
        assert!(page.contains("<strong>Needs data</strong>"));
        assert!(!page.contains("<strong>Needs work</strong>"));
        assert!(page.contains("Showing 1–1 of 1 untried concept"));
    }

    #[test]
    fn analytics_page_applies_untried_filter_to_a_study_view_response() {
        let state = memory_engine_api_state::ApiState::default();
        let created = state
            .create_account("analytics-untried@example.com")
            .expect("account");
        let account = state.create_browser_session(&created).expect("session");
        let view = StudyViewResponse {
            drafts: Vec::new(),
            current: None,
            concept_progress: vec![
                concept("Needs data", "untried", 0, 0),
                concept("Needs work", "struggling", 1, 10),
            ],
            summary: BetaStudySummary {
                source_count: 1,
                accepted_draft_count: 2,
                approved_review_unit_count: 2,
                attempt_count: 1,
                last_outcome: None,
                next_review_unit_id: None,
            },
            due_count: 0,
            generation_notices: Vec::new(),
        };

        let page = super::render_analytics_page(
            &account,
            &view,
            AnalyticsViewOptions {
                filter: AnalyticsConceptFilter::Untried,
                sort: AnalyticsConceptSort::Health,
                page: 1,
            },
        );

        assert!(page.contains(r#"<h1 class="me-display me-analytics-title">Concept health</h1>"#));
        assert!(page.contains(r#"<option value="untried" selected>Untried</option>"#));
        assert!(page.contains("<strong>Needs data</strong>"));
        assert!(!page.contains("<strong>Needs work</strong>"));
    }
}
