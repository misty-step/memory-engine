use std::fmt::Write as _;

use memory_engine_study::{BetaStudyCurrent, BetaStudyDraftRow, BetaStudySummary};

use crate::{ApiFailure, ApiState, AppAccount, SourceRecord, StudyViewResponse};

pub(crate) fn render_action_result_html(
    state: &ApiState,
    account: &AppAccount,
    result: Result<StudyViewResponse, ApiFailure>,
) -> String {
    match result {
        Ok(view) => render_account_page(state, account, Some(&view), None),
        Err(error) => render_account_page(state, account, None, Some(&error.message)),
    }
}

pub(crate) fn render_account_page(
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

pub(crate) fn render_app_shell(
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
  <h2>Sign in</h2>
  <form action="/app/account" method="post">
    <label>Email <input name="email" type="email" autocomplete="email" required></label>
    <button type="submit">Send sign-in link</button>
  </form>
</section>"#,
        render_start_form()
    )
}

pub(crate) fn render_login_requested(debug_link: Option<&str>) -> String {
    let debug_link = debug_link.map_or_else(String::new, |link| {
        format!(
            r#"<p class="muted"><a href="{}">Debug sign-in link</a></p>"#,
            escape_html(link)
        )
    });

    format!(
        r#"{}<section class="compact">
  <h2>Check your email</h2>
  <p class="muted">If that address can sign in, a link is on the way.</p>
  {}
</section>"#,
        render_start_form(),
        debug_link
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
  <form action="/app/logout" method="post">
    {}
    <button type="submit">Sign out</button>
  </form>
  <form action="/app/save-account" method="post">
    {}
    <label>Email <input name="email" type="email" autocomplete="email" required></label>
    <button type="submit">Save account email</button>
  </form>
</section>"#,
        escape_html(&account.account_id),
        hidden_csrf_input(account),
        hidden_csrf_input(account)
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
        hidden_csrf_input(account),
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
            hidden_csrf_input(account),
            escape_html(&source.source_id)
        )
        .expect("write source html");
    }

    format!(r"<section><h2>Sources</h2>{rows}</section>")
}

fn render_study(account: &AppAccount, view: &StudyViewResponse) -> String {
    [
        render_summary(&view.summary),
        render_generation_notices(&view.generation_notices),
        render_drafts(account, &view.drafts),
        render_current_review(account, view.current.as_ref()),
    ]
    .join("")
}

pub(crate) fn render_generation_notices(notices: &[String]) -> String {
    if notices.is_empty() {
        return String::new();
    }
    let mut items = String::new();
    for notice in notices {
        write!(items, "<li>{}</li>", escape_html(notice)).expect("write notice html");
    }

    format!(
        r#"<section class="compact notices"><h2>Generation notes</h2><ul>{items}</ul></section>"#
    )
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
                hidden_csrf_input(account),
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
            hidden_csrf_input(account)
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
        hidden_csrf_input(account),
        escape_html(&current.review_unit_id.to_string()),
        hidden_csrf_input(account),
        escape_html(&current.review_unit_id.to_string()),
        escape_html(&current.review_unit_id.to_string())
    )
}

fn hidden_csrf_input(account: &AppAccount) -> String {
    format!(
        r#"<input type="hidden" name="csrfToken" value="{}">"#,
        escape_html(&account.csrf_token)
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
