//! Server-rendered study UI.
//!
//! The markup consumes the Misty Step `aesthetic` design system (vendored at
//! `assets/aesthetic.css`, served from `/static/aesthetic.css`): one font size
//! per surface, hierarchy from ink and weight, hairlines and radius 0, status
//! on the glyph, motion as feedback. The app is a single adaptive screen that
//! swaps between a workspace (capture, keep, manage, reflect) and a review
//! (prompt, answer, grade). Every interaction is a full-page form POST; there
//! is no client JavaScript.

use std::fmt::Write as _;

use memory_engine_study::{BetaStudyConceptProgress, BetaStudyCurrent};

use memory_engine_api_state::{
    ApiFailure, ApiState, AppAccount, GenerationJob, JobStatus, SourceRecord, StudyViewResponse,
};

#[must_use]
pub fn render_action_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
) -> String {
    match result {
        Ok(view) => render_account_page(state, account, Some(&view), None),
        Err(error) => render_account_page(state, account, None, Some(&error.message)),
    }
}

#[must_use]
pub fn render_account_page(
    state: &ApiState,
    account: &AppAccount,
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
) -> String {
    let sources = state.list_app_sources(account).unwrap_or_default();
    let jobs = state.jobs_for_app_account(account);
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
    let view = view.or(fetched.as_ref());
    render_app_shell(Some(account), &sources, view, &jobs, notice)
}

#[must_use]
pub fn render_app_shell(
    account: Option<&AppAccount>,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
    notice: Option<&str>,
) -> String {
    let inner = match account {
        Some(account) => render_signed_in(account, sources, view, jobs, notice),
        None => render_signed_out(notice),
    };
    document(&inner)
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

/// Wrap a `.ae-screen` body in the full document, linking the design system.
fn document(inner: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>Memory Engine</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@100..900&family=Geist+Mono:wght@100..900&display=swap">
<link rel="stylesheet" href="/static/aesthetic.css">
<style>{STYLE}</style>
<script src="/static/app.js" defer></script>
</head>
<body>
{inner}
</body>
</html>"#
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
<a class="ae-name" href="/">MEMORY ENGINE</a>
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
    // only dead-end on it. The display promise leads, then one action: enter an
    // email and get a sign-in link. Capture and review live behind auth.
    let view = format!(
        r#"<div class="me-cover">
{notice}
<p class="me-kicker">Spaced repetition, made effortless</p>
<h1 class="me-display">Read it once.<br>Remember it for good.</h1>
<p class="ae-lede ae-dim me-support">Capture anything worth remembering. Memory Engine brings it back, timed to how memory fades.</p>
<section class="ae-group me-capture-hero">
<form action="/app/account" method="post">
<label class="ae-label" for="me-email">Your email</label>
<input class="ae-input me-hero-email" id="me-email" name="email" type="email" autocomplete="email" required placeholder="you@example.com" aria-label="Email address">
<div class="me-actions"><button class="ae-button" type="submit">Get started</button><span class="ae-dim me-hint">No password — we’ll email you a sign-in link.</span></div>
</form>
</section>
</div>"#,
        notice = render_notice(notice),
    );
    screen("", &view, FOOTER_TAGLINE)
}

fn render_signed_in(
    account: &AppAccount,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
    notice: Option<&str>,
) -> String {
    let due = view.map_or(0, |view| view.due_count);
    let header_right = format!(r#"<span class="me-due">{due} due</span>"#);
    let footer = format!(
        r#"<span class="ae-dim">Signed in</span>
<form class="me-foot-form" action="/app/logout" method="post">{}<button class="ae-button-quiet ae-button-compact" type="submit">Sign out</button></form>"#,
        hidden_csrf_input(account)
    );

    let body = if let Some(current) = view.and_then(|view| view.current.as_ref()) {
        render_current_review(account, current)
    } else {
        render_workspace(account, sources, view, jobs)
    };
    let view_inner = format!("{}{}", render_notice(notice), body);

    screen(&header_right, &view_inner, &footer)
}

fn render_workspace(
    account: &AppAccount,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    jobs: &[GenerationJob],
) -> String {
    // Generation is non-blocking: capture is the persistent action, the activity
    // log shows jobs running in the background (live over SSE, but also correct
    // on every full load), and review is always one tap away. No keep gate —
    // successful jobs are already scheduled.
    let mut html = String::new();
    if let Some(view) = view {
        html.push_str(&render_review_status(account, view));
    }
    if sources.is_empty() && jobs.is_empty() {
        html.push_str(
            r#"<p class="ae-lede me-welcome">Type a topic or paste anything worth remembering. Memory Engine turns it into review, timed so it sticks.</p>"#,
        );
    }
    html.push_str(&render_capture(account));
    html.push_str(&render_jobs(account, jobs));
    html.push_str(&render_sources(account, sources));
    if let Some(view) = view {
        html.push_str(&render_concept_progress(&view.concept_progress));
    }
    html
}

fn render_notice(message: Option<&str>) -> String {
    message.map_or_else(String::new, |message| {
        format!(
            r#"<p class="me-notice" role="status">{ICON_INFO}<span>{}</span></p>"#,
            escape_html(message)
        )
    })
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
            r#"<section class="ae-group me-caughtup"><p class="me-caughtup-line">{ICON_OK}<span class="ae-item">You're all caught up.</span></p><p class="ae-dim me-hint">Nothing is due right now — add more below, or come back later.</p></section>"#
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
<form action="/app/capture" method="post">
{csrf}
<label class="ae-label me-capture-label" for="me-capture">What do you want to remember?</label>
<textarea class="ae-input" id="me-capture" name="capture" rows="3" required placeholder="A topic like “NATO phonetic alphabet”, a list, or pasted notes."></textarea>
<div class="me-actions"><button class="ae-button" type="submit">Create {ICON_ARROW}</button><span class="ae-dim me-hint me-live-hint">Generates in the background — keep going.</span></div>
</form>
</section>"#,
        csrf = hidden_csrf_input(account),
    )
}

fn render_sources(account: &AppAccount, sources: &[SourceRecord]) -> String {
    if sources.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for source in sources {
        let _ = write!(
            rows,
            r#"<article class="me-source">
<p class="ae-item">{title}</p>
<div class="me-row-actions">
<form action="/app/generate" method="post">{csrf_generate}<input type="hidden" name="sourceId" value="{id}"><button class="ae-button ae-button-compact" type="submit">Create review</button></form>
<form action="/app/source/archive" method="post">{csrf_archive}<input type="hidden" name="sourceId" value="{id_archive}"><button class="ae-button-quiet ae-button-compact" type="submit">Remove</button></form>
</div>
</article>"#,
            title = escape_html(&source.title),
            csrf_generate = hidden_csrf_input(account),
            id = escape_html(&source.source_id),
            csrf_archive = hidden_csrf_input(account),
            id_archive = escape_html(&source.source_id),
        );
    }

    format!(
        r#"<section class="ae-group me-material"><h2 class="ae-h">Saved material</h2>{rows}</section>"#
    )
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
<span class="me-job-glyphs" aria-hidden="true"><span class="g-queued">{ICON_CLOCK}</span><span class="g-running"><span class="me-spinner"></span></span><span class="g-succeeded">{ICON_OK}</span><span class="g-failed">{ICON_ERR}</span></span>
<div class="me-job-body">
<p class="me-job-title">{title}</p>
<p class="me-job-meta">{meta}</p>
</div>
<form class="me-job-retry" action="/app/jobs/retry" method="post">{csrf}<input type="hidden" name="jobId" value="{id}"><button class="me-job-retry-btn" type="submit">Retry</button></form>
</li>"#,
        id = escape_html(&job.id),
        status = job.status.as_str(),
        title = escape_html(&job.title),
        meta = job_meta(job),
        csrf = hidden_csrf_input(account),
    )
}

/// The human meta line for a job's current status. Kept in sync with the
/// `metaFor` switch in `app.js`, which recomputes it on each SSE update.
fn job_meta(job: &GenerationJob) -> String {
    match job.status {
        JobStatus::Queued => "Queued…".to_owned(),
        JobStatus::Running => "Generating cards…".to_owned(),
        JobStatus::Succeeded => format!(
            "{} {} · scheduled for review",
            job.card_count,
            plural(job.card_count, "card", "cards")
        ),
        JobStatus::Failed => escape_html(
            job.error
                .as_deref()
                .unwrap_or("Generation failed — try again."),
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
{reveal}
{escape_hatches}"#,
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
{reference}
{next}"#,
        prompt = escape_html(&current.prompt),
        reveal = render_answer_reveal(current),
        verdict = render_graded_verdict(current),
        reference = render_reference(current),
        next = render_next(account, current),
    )
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
    format!(
        r#"<form class="me-next" action="/app/next" method="post">{csrf}<button class="ae-button" type="submit">Next {ICON_ARROW}</button></form>"#,
        csrf = hidden_csrf_input(account),
    )
}

fn render_concept_progress(concepts: &[BetaStudyConceptProgress]) -> String {
    if concepts.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for concept in concepts {
        let pct = if concept.attempts > 0 {
            (concept.correct * 100 / concept.attempts).min(100)
        } else {
            0
        };
        let _ = write!(
            rows,
            r#"<div class="me-concept">
<div class="me-concept-head"><strong>{label}</strong><span class="me-trend ae-dim">{trend_icon} {trend}</span></div>
<div class="ae-meter"><div class="ae-meter-fill {fill}" style="width:{pct}%"></div></div>
<p class="me-concept-note ae-dim">{success_rate} · {summary}</p>
</div>"#,
            label = escape_html(&concept.concept_label),
            trend_icon = trend_icon(&concept.trend),
            trend = escape_html(&concept.trend),
            fill = health_fill_class(&concept.health),
            success_rate = escape_html(&concept.success_rate),
            summary = escape_html(&concept.summary),
        );
    }

    format!(
        r#"<section class="ae-group me-concepts"><h2 class="ae-h">Concept health</h2>{rows}</section>"#
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
    // Reference/Skip/Snooze/Bridge defer or assist the current card; Delete is
    // the "this card is bad" exit — it removes the card from review for good.
    // Kept last and visually set apart so it isn't a stray tap.
    format!(
        r#"<div class="me-hatches">{reference}{skip}{snooze}{bridge}<span class="me-hatch-delete">{delete}</span></div>"#,
        reference = render_review_action(account, current, "/app/reference", "Reference"),
        skip = render_review_action(account, current, "/app/skip", "Skip"),
        snooze = render_review_action(account, current, "/app/snooze", "Snooze"),
        bridge = render_review_action(account, current, "/app/bridge", "Bridge"),
        delete = render_review_action(account, current, "/app/delete", "Delete"),
    )
}

fn render_review_action(
    account: &AppAccount,
    current: &BetaStudyCurrent,
    action: &str,
    label: &str,
) -> String {
    format!(
        r#"<form action="{action}" method="post">{csrf}<input type="hidden" name="reviewUnitId" value="{id}"><button class="ae-button-quiet ae-button-compact" type="submit">{label}</button></form>"#,
        csrf = hidden_csrf_input(account),
        id = escape_html(&current.review_unit_id.to_string()),
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
        "healthy" | "strong" => "ae-ok",
        "watch" | "mixed" => "ae-warn",
        "at risk" | "at-risk" | "weak" => "ae-err",
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

const FOOTER_TAGLINE: &str = r#"<span class="ae-dim">A memory instrument</span>"#;

// Lucide icons (ISC), inlined for `.ae-icon`: 1.5px stroke, currentColor, no
// fill. Status hue rides the glyph; the sentence stays ink.
const ICON_OK: &str = r#"<svg class="ae-icon ae-ok" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>"#;
const ICON_WARN: &str = r#"<svg class="ae-icon ae-warn" viewBox="0 0 24 24" aria-hidden="true"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;
const ICON_ERR: &str = r#"<svg class="ae-icon ae-err" viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const ICON_REVEALED: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>"#;
const ICON_INFO: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/></svg>"#;
const ICON_ARROW: &str = r#"<svg class="ae-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>"#;
const ICON_CLOCK: &str = r#"<svg class="ae-icon ae-dim" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>"#;
const ICON_UP: &str = r#"<svg class="ae-icon ae-ok" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 17 17 7"/><path d="M7 7h10v10"/></svg>"#;
const ICON_DOWN: &str = r#"<svg class="ae-icon ae-warn" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7 17 17"/><path d="M17 7v10H7"/></svg>"#;

// App-specific page glue. Structure and rhythm only — every costume comes from
// aesthetic.css. The accent (ultramarine) is the one steered token.
const STYLE: &str = r#"
:root { --ae-accent: #2643d0; --ae-accent-dark: #8c9eff; }
.dark, [data-ae-mode='dark'] { --ae-accent: var(--ae-accent-dark); }
/* Scrollbar fix: the kit puts the scroll on the measure-width column, so its
   bar floats mid-screen. Move the scroll to the full-width stage and keep the
   content in the measure via the view wrapper — now the bar hugs the screen
   edge. (Scoped to the scrolling stage; short centered screens are unaffected.) */
.ae-stage-scroll { width: 100%; }
.ae-stage-scroll > .ae-view { width: min(var(--ae-measure), 100% - 5rem); margin-inline: auto; }
.me-due { font-variant-numeric: tabular-nums; color: var(--ae-ink-muted); }
.me-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 1em; margin-top: 1.2em; }
.me-hint { font-size: 13px; }
/* the signed-out cover — the one surface where type goes to display scale.
   App screens stay strictly one size; this register is cover-only. A clean
   candidate to promote into misty-step as a sanctioned .ae-display primitive. */
.me-kicker { font-family: var(--ae-font-mono); font-size: 13px; font-weight: var(--ae-w-medium); letter-spacing: 0.1em; color: var(--ae-ink-muted); margin: 0 0 1.2em; }
.me-cover { padding-top: clamp(1.5rem, 9vh, 6rem); }
.me-display { font-weight: var(--ae-w-black); font-size: clamp(2.1rem, 1.1rem + 4vw, 2.9rem); line-height: 1.04; letter-spacing: -0.02em; margin: 0 0 0.55em; max-width: 13em; text-wrap: balance; }
.me-support { margin: 0 0 2em; }
.me-capture-hero { max-width: 34em; }
.me-hero-email { border: 1px solid var(--ae-line); padding: 0 0.85em; height: 46px; max-width: 26em; margin-top: 0.5em; }
.me-hero-email:focus-visible { border-color: var(--ae-ink); outline: 2px solid var(--ae-accent); outline-offset: 2px; }
@media (max-width: 640px) {
  .me-cover { padding-top: 1.4rem; }
  .me-support { margin-bottom: 1.3em; }
  textarea.ae-input { min-height: 4em; }
}
.me-callout-line { margin-bottom: 1.1em; }
.me-callout-n { font-family: var(--ae-font-mono); font-variant-numeric: tabular-nums; font-weight: var(--ae-w-black); margin-right: 0.15em; }
.me-caughtup-line { display: flex; align-items: center; gap: 0.5em; font-weight: var(--ae-w-medium); margin: 0 0 0.3em; }
/* the prompt is the focus of the review — loud by weight, never by size */
.me-prompt { font-weight: var(--ae-w-black); line-height: 1.45; max-width: 30em; margin: 0.2em 0 1.6em; }
/* post-grade recap: a quiet read-only list of the options that were offered */
.me-choices { list-style: none; margin: 0 0 1.6em; padding: 0; counter-reset: choice; }
.me-choices li { counter-increment: choice; display: flex; gap: 0.7em; padding: 0.28em 0; }
.me-choices li::before { content: counter(choice, upper-alpha) "."; color: var(--ae-ink-muted); font-family: var(--ae-font-mono); font-size: 13px; min-width: 1.3em; }
/* pre-grade: the options ARE the control — full-width clickable rows, one tap
   to answer. Lettered like the recap so they read as the same set. */
.me-choices-form { display: flex; flex-direction: column; gap: 0.5em; margin: 0 0 1.6em; counter-reset: choice; }
.me-choice { counter-increment: choice; display: flex; gap: 0.7em; align-items: baseline; width: 100%; text-align: left; font: inherit; color: var(--ae-ink); background: transparent; border: 1px solid var(--ae-line); padding: 0.7em 0.9em; cursor: pointer; transition: border-color var(--ae-quick) var(--ae-ease); }
.me-choice::before { content: counter(choice, upper-alpha) "."; color: var(--ae-ink-muted); font-family: var(--ae-font-mono); font-size: 13px; min-width: 1.3em; }
.me-choice:hover { border-color: var(--ae-ink); }
.me-choice:focus-visible { outline: 2px solid var(--ae-accent); outline-offset: 2px; }
/* free-response field: a clearly bounded box, not the hairline underline that
   read as a divider */
.me-answer-input { border: 1px solid var(--ae-line); padding: 0.7em 0.85em; }
.me-answer-input:focus-visible { border-color: var(--ae-ink); outline: 2px solid var(--ae-accent); outline-offset: 2px; }
/* graded multiple choice: the options become a read-only reveal — the correct
   one marked and tinted, the rest dimmed. Not a control: no hover, no pointer. */
.me-choices-graded { list-style: none; margin: 0 0 1.2em; padding: 0; display: flex; flex-direction: column; gap: 0.45em; }
.me-graded-choice { display: flex; align-items: center; gap: 0.7em; padding: 0.55em 0.7em; border: 1px solid transparent; }
.me-graded-choice > span { flex: 1; }
.me-graded-choice .ae-icon { width: 1.05em; height: 1.05em; color: var(--ae-ok); flex: none; }
.me-graded-choice-correct { border-color: var(--ae-ok); background: color-mix(in srgb, var(--ae-ok) 8%, transparent); font-weight: var(--ae-w-black); }
.me-graded-choice-dim { color: var(--ae-ink-faint); }
.me-answer { display: flex; flex-direction: column; gap: 0.2em; padding: 0.7em 0 0.7em 1em; border-left: 2px solid var(--ae-accent); margin: 0 0 1.2em; }
.me-answer-label { font-size: 13px; letter-spacing: 0.06em; color: var(--ae-ink-muted); }
/* the verdict is the moment: loud by weight, the glyph carries the hue and
   draws on once (reduced-motion users get it instantly via the kit's reset).
   One quiet line beneath says when the card returns; then Next is primary. */
.me-result { display: flex; align-items: center; gap: 0.55em; margin: 0.4em 0 0.35em; }
.me-result .ae-icon { width: 1.5em; height: 1.5em; }
.me-verdict { font-weight: var(--ae-w-black); }
.me-result .ae-ok { stroke-dasharray: 26; animation: me-draw var(--ae-gentle) var(--ae-ease) both; }
@keyframes me-draw { from { stroke-dashoffset: 26; } }
.me-next-when { font-size: 13px; margin: 0 0 1.5em; }
.me-next { margin: 0 0 1.6em; }
.me-reference { padding: 0.9em 0; border-top: 1px solid var(--ae-line); border-bottom: 1px solid var(--ae-line); margin: 0 0 1.5em; }
.me-reference p { margin: 0; }
.ae-lede { margin-bottom: 1.7em; }
.me-source { padding: 1.1em 0; border-top: 1px solid var(--ae-line); }
.me-source:first-of-type { border-top: 0; padding-top: 0.2em; }
.me-row-actions, .me-hatches { display: flex; flex-wrap: wrap; gap: 0.6em; margin-top: 0.8em; }
/* the activity log: one row per background generation job. data-status drives
   the status glyph, the meta colour, and retry visibility, so the SSE script
   only has to flip one attribute and rewrite one line of text. */
.me-jobs-list { list-style: none; margin: 0; padding: 0; }
.me-job { display: flex; gap: 13px; align-items: flex-start; padding: 0.95em 0; border-top: 1px solid var(--ae-line); }
.me-job:first-child { border-top: 0; padding-top: 0.3em; }
.me-job-body { flex: 1; min-width: 0; }
.me-job-title { font-weight: var(--ae-w-medium); margin: 0; }
.me-job-meta { font-size: 13px; color: var(--ae-ink-muted); margin: 0.2em 0 0; }
.me-job[data-status="failed"] .me-job-meta { color: var(--ae-err); }
.me-job-glyphs { flex: none; width: 18px; height: 18px; }
.me-job-glyphs > span { display: none; }
.me-job-glyphs .ae-icon { width: 18px; height: 18px; }
.me-job[data-status="queued"] .g-queued,
.me-job[data-status="running"] .g-running,
.me-job[data-status="succeeded"] .g-succeeded,
.me-job[data-status="failed"] .g-failed { display: inline-flex; }
.me-job-retry { margin: 0; flex: none; display: none; }
.me-job[data-status="failed"] .me-job-retry { display: block; }
.me-job-retry-btn { border: 0; background: none; padding: 0.1em 0; font: inherit; font-size: 13px; font-weight: var(--ae-w-medium); color: var(--ae-accent); cursor: pointer; }
.me-job-retry-btn:hover { text-decoration: underline; }
/* a loading ring is the one sanctioned round shape (radius 0 elsewhere) */
.me-spinner { display: inline-block; width: 15px; height: 15px; border: 2px solid var(--ae-line); border-top-color: var(--ae-accent); border-radius: 50%; animation: me-spin 0.8s linear infinite; }
@keyframes me-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .me-spinner { animation: none; } }
.me-live-hint::before { content: ""; display: inline-block; width: 6px; height: 6px; margin-right: 0.5em; background: var(--ae-accent); border-radius: 50%; vertical-align: middle; }
.me-capture-label { display: block; }
.me-welcome { margin-bottom: 1.7em; }
.me-hatches { margin: 1.9em 0 2.4em; }
.me-hatch-delete { margin-left: auto; }
.me-reveal { margin-top: 1.1em; }
/* the capture textarea is the hero affordance: give it a hairline frame so an
   empty field never reads as dead space (single-line inputs keep the line). */
textarea.ae-input { min-height: 5em; border: 1px solid var(--ae-line); padding: 0.7em 0.85em; margin-top: 0.5em; }
textarea.ae-input:focus-visible { border-color: var(--ae-ink); outline: 2px solid var(--ae-accent); outline-offset: 2px; }
.me-concept { margin-bottom: 1.4em; }
.me-concept:last-child { margin-bottom: 0; }
.me-concept-head { display: flex; justify-content: space-between; align-items: baseline; gap: 1em; margin-bottom: 0.55em; }
.me-concept-head strong { font-weight: var(--ae-w-medium); }
.me-concept-note { font-size: 13px; margin: 0.5em 0 0; }
.me-trend { display: inline-flex; align-items: center; gap: 0.3em; white-space: nowrap; }
.me-notice { display: flex; gap: 0.6em; align-items: baseline; padding: 0.85em 1em; border: 1px solid var(--ae-line); margin: 0 0 1.9em; }
.me-foot-form { margin: 0; }
.me-submit { margin: 0; }
"#;
