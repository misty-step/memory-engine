use std::fmt::Write as _;

use memory_engine_persistence::GeneratedPromptValidationStatus;
use memory_engine_study::{BetaStudyCurrent, BetaStudyDraftRow};

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
    notice: Option<&str>,
) -> String {
    let sources = state
        .accounts
        .list_sources(&account.account_id, &account.session_token)
        .unwrap_or_default();
    render_app_shell(Some(account), &sources, view, notice)
}

pub(crate) fn render_app_shell(
    account: Option<&AppAccount>,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
    notice: Option<&str>,
) -> String {
    let heading = if view.and_then(|view| view.current.as_ref()).is_some() {
        "Review"
    } else {
        "Add something you want to learn"
    };
    let notice_panel = render_notice(notice);
    let content = match account {
        Some(account) => {
            if let Some(view) = view.filter(|view| view.current.is_some()) {
                [
                    render_session_bar(account),
                    notice_panel,
                    render_current_review(account, view),
                ]
                .join("")
            } else {
                [
                    render_session_bar(account),
                    notice_panel,
                    render_workspace(account, sources, view),
                ]
                .join("")
            }
        }
        None => [notice_panel, render_start_form(None), render_sign_in_form()].join(""),
    };

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
      <h1>{heading}</h1>
    </header>
    {content}
  </main>
</body>
</html>"#
    )
}

fn render_workspace(
    account: &AppAccount,
    sources: &[SourceRecord],
    view: Option<&StudyViewResponse>,
) -> String {
    let mut html = String::new();
    if let Some(view) = view {
        html.push_str(&render_due_status(view.due_count));
        html.push_str(&render_generation_notices(&view.generation_notices));
        html.push_str(&render_keep_flow(account, &view.drafts));
    }
    html.push_str(&render_source_form(account));
    html.push_str(&render_sources(account, sources));
    html
}

fn render_notice(message: Option<&str>) -> String {
    message.map_or_else(String::new, |message| {
        format!(
            r#"<section class="notice" role="status">{}</section>"#,
            escape_html(message)
        )
    })
}

fn render_sign_in_form() -> String {
    r#"<section class="secondary">
  <h2>Sign in</h2>
  <form action="/app/account" method="post">
    <label>Email <input name="email" type="email" autocomplete="email" required></label>
    <button type="submit">Send sign-in link</button>
  </form>
</section>"#
        .to_owned()
}

pub(crate) fn render_login_requested(debug_link: Option<&str>) -> String {
    let debug_link = debug_link.map_or_else(String::new, |link| {
        format!(
            r#"<p class="muted"><a href="{}">Debug sign-in link</a></p>"#,
            escape_html(link)
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
      <h1>Check your email</h1>
    </header>
    <section class="secondary">
      <p class="muted">If that address can sign in, a link is on the way.</p>
      {debug_link}
    </section>
  </main>
</body>
</html>"#
    )
}

fn render_start_form(account: Option<&AppAccount>) -> String {
    let action = if account.is_some() {
        "/app/source"
    } else {
        "/app/start"
    };
    let csrf = account.map_or_else(String::new, hidden_csrf_input);

    format!(
        r#"<section class="capture">
  <h2>Add something you want to learn</h2>
  <form action="{action}" method="post">
    {csrf}
    <label>Title <input name="title" required placeholder="Short title"></label>
    <label>Text <textarea name="body" rows="7" required placeholder="Paste anything worth remembering."></textarea></label>
    <button type="submit">Create review</button>
  </form>
</section>"#
    )
}

fn render_source_form(account: &AppAccount) -> String {
    render_start_form(Some(account))
}

fn render_session_bar(account: &AppAccount) -> String {
    format!(
        r#"<nav class="session" aria-label="Session">
  <span>Signed in</span>
  <form action="/app/logout" method="post">
    {}
    <button type="submit">Sign out</button>
  </form>
</nav>"#,
        hidden_csrf_input(account)
    )
}

fn render_sources(account: &AppAccount, sources: &[SourceRecord]) -> String {
    if sources.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for source in sources {
        write!(
            rows,
            r#"<article class="source-item">
  <div>
    <h3>{}</h3>
    <p class="muted">Saved for review creation.</p>
  </div>
  <div class="row-actions">
    <form action="/app/generate" method="post">
      {}
      <input type="hidden" name="sourceId" value="{}">
      <button type="submit">Create review</button>
    </form>
    <form action="/app/source/archive" method="post">
      {}
      <input type="hidden" name="sourceId" value="{}">
      <button class="quiet" type="submit">Remove</button>
    </form>
  </div>
</article>"#,
            escape_html(&source.title),
            hidden_csrf_input(account),
            escape_html(&source.source_id),
            hidden_csrf_input(account),
            escape_html(&source.source_id)
        )
        .expect("write source html");
    }

    format!(r#"<section class="material"><h2>Saved material</h2>{rows}</section>"#)
}

fn render_due_status(due_count: usize) -> String {
    format!(r#"<p class="due-count">{due_count} due</p>"#)
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
        r#"<section class="secondary notices"><h2>Generation notes</h2><ul>{items}</ul></section>"#
    )
}

fn render_keep_flow(account: &AppAccount, drafts: &[BetaStudyDraftRow]) -> String {
    let accepted = drafts
        .iter()
        .filter(|draft| draft.validation_status == GeneratedPromptValidationStatus::Accepted)
        .collect::<Vec<_>>();
    if accepted.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for draft in accepted {
        write!(
            rows,
            r#"<article class="keep-item">
  <h3>Question</h3>
  <p class="prompt">{}</p>
  <form action="/app/approve" method="post">
    {}
    <input type="hidden" name="draftId" value="{}">
    <button type="submit">Keep this</button>
  </form>
</article>"#,
            escape_html(&draft.prompt),
            hidden_csrf_input(account),
            escape_html(&draft.id)
        )
        .expect("write keep html");
    }

    format!(r#"<section class="keep-flow"><h2>Choose what to keep</h2>{rows}</section>"#)
}

fn render_current_review(account: &AppAccount, view: &StudyViewResponse) -> String {
    let current = view.current.as_ref().expect("review view has current item");
    let answer = render_answer(current);
    let result = render_grade(account, current);
    let reveal = render_reveal_form(account, current);
    let submit = render_submit_form(account, current);

    format!(
        r#"{}
<section class="review">
  <p class="prompt">{}</p>
  {answer}
  {result}
  {reveal}
  {submit}
</section>"#,
        render_due_status(view.due_count),
        escape_html(&current.prompt)
    )
}

fn render_answer(current: &BetaStudyCurrent) -> String {
    current
        .expected_answer
        .as_ref()
        .map_or_else(String::new, |answer| {
            format!(
                r#"<div class="answer"><span>Answer</span><strong>{}</strong></div>"#,
                escape_html(answer)
            )
        })
}

fn render_grade(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    current.grade.as_ref().map_or_else(String::new, |grade| {
        format!(
            r#"<p class="result">{}</p>
<form action="/app/next" method="post">
  {}
  <button type="submit">Next</button>
</form>"#,
            verdict_label(grade.verdict),
            hidden_csrf_input(account)
        )
    })
}

fn render_reveal_form(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.expected_answer.is_some() || current.grade.is_some() {
        return String::new();
    }

    format!(
        r#"<form action="/app/reveal" method="post">
  {}
  <input type="hidden" name="reviewUnitId" value="{}">
  <button class="quiet" type="submit">Reveal answer</button>
</form>"#,
        hidden_csrf_input(account),
        escape_html(&current.review_unit_id.to_string())
    )
}

fn render_submit_form(account: &AppAccount, current: &BetaStudyCurrent) -> String {
    if current.grade.is_some() {
        return String::new();
    }

    format!(
        r#"<form action="/app/submit" method="post">
  {}
  <input type="hidden" name="reviewUnitId" value="{}">
  <input type="hidden" name="responseTimeMs" value="1800">
  <input type="hidden" name="idempotencyKey" value="review-{}">
  <label>Your answer <input name="answer" required autocomplete="off"></label>
  <button type="submit">Answer</button>
</form>"#,
        hidden_csrf_input(account),
        escape_html(&current.review_unit_id.to_string()),
        escape_html(&current.review_unit_id.to_string())
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

const APP_CSS: &str = r"
:root {
  color-scheme: light;
  font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  background: #f7f6f2;
  color: #1c2226;
}
* { box-sizing: border-box; }
body { margin: 0; }
main {
  width: min(100%, 680px);
  margin: 0 auto;
  padding: 22px 16px 44px;
}
header { padding: 10px 0 18px; }
.eyebrow {
  margin: 0 0 7px;
  font-size: 0.78rem;
  text-transform: uppercase;
  color: #61706b;
}
h1, h2, h3, p { overflow-wrap: anywhere; }
h1 { margin: 0; font-size: 1.85rem; line-height: 1.08; }
h2 { margin: 0 0 12px; font-size: 1.08rem; }
h3 { margin: 0 0 8px; font-size: 0.95rem; }
p { line-height: 1.45; }
section {
  margin: 14px 0;
  padding: 16px 0;
  border-top: 1px solid #d9d5ca;
}
.capture { border-top: 0; padding-top: 0; }
.secondary, .material, .keep-flow, .review {
  border-top-color: #cfd7d2;
}
.session {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 0;
  border-top: 1px solid #d9d5ca;
  border-bottom: 1px solid #d9d5ca;
  color: #53615c;
  font-size: 0.92rem;
}
.session form, .row-actions form { margin: 0; }
.notice {
  padding: 12px;
  border: 1px solid #d3a03b;
  background: #fff8e8;
  color: #4e3a12;
}
.due-count {
  margin: 14px 0 4px;
  color: #315f53;
  font-weight: 700;
}
.prompt {
  margin: 0 0 18px;
  font-size: 1.28rem;
  line-height: 1.28;
}
.answer {
  display: grid;
  gap: 4px;
  margin: 10px 0 16px;
  padding: 12px;
  border-left: 3px solid #315f53;
  background: #edf4f0;
}
.answer span, .muted {
  color: #61706b;
  font-size: 0.92rem;
}
.result {
  margin: 12px 0;
  font-size: 1.1rem;
  font-weight: 700;
  color: #315f53;
}
form {
  display: grid;
  gap: 12px;
}
label {
  display: grid;
  gap: 6px;
  font-size: 0.92rem;
  color: #3b4642;
}
input, textarea {
  width: 100%;
  min-height: 44px;
  border: 1px solid #bfc8c2;
  border-radius: 7px;
  padding: 11px 12px;
  font: inherit;
  background: #fffefb;
  color: #1c2226;
}
textarea { resize: vertical; }
button {
  min-height: 44px;
  border: 1px solid #315f53;
  border-radius: 7px;
  padding: 0 14px;
  font: inherit;
  font-weight: 700;
  background: #315f53;
  color: #ffffff;
}
button.quiet {
  background: transparent;
  color: #315f53;
}
.keep-item, .source-item {
  display: grid;
  gap: 12px;
  margin: 12px 0;
  padding: 12px 0;
  border-top: 1px solid #e1ddd3;
}
.keep-item:first-of-type, .source-item:first-of-type { border-top: 0; }
.row-actions {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}
.notices ul {
  margin: 0;
  padding-left: 20px;
}
@media (max-width: 480px) {
  main { padding-inline: 14px; }
  h1 { font-size: 1.62rem; }
  .prompt { font-size: 1.16rem; }
  .row-actions { grid-template-columns: 1fr; }
}
";
