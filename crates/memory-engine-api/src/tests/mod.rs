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

use super::{router, routes, AccountRegistry, ApiState, AuthConfig, AUTH_CHALLENGE_TTL_MS};

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
async fn mobile_home_is_auth_first_and_hides_the_dead_end_guest_capture() {
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
    // Onboarding leads with the magic-link email form, not the guest path.
    assert!(body.contains(r#"<form action="/app/account" method="post">"#));
    assert!(body.contains(r#"name="email""#));
    assert!(body.contains(r#"placeholder="you@example.com""#));
    assert!(body.contains("Get started"));
    assert!(body.contains("Spaced repetition, made effortless"));
    assert!(body.contains("Remember it for good."));
    // Regression: the anonymous home must NOT offer the guest capture form,
    // which dead-ends on the account allowlist ("not allowed to register").
    assert!(!body.contains(r#"action="/app/start""#));
    assert!(!body.contains(r#"name="capture""#));
    assert!(!body.contains("Start remembering"));
    // No source/account internals leak onto the anonymous home.
    assert!(!body.contains("NATO practice notes"));
    assert!(!body.contains("Concept: NATO letter A"));
}

#[tokio::test]
async fn mobile_capture_enqueues_generation_then_auto_schedules_cards() {
    let state = ApiState::default();
    let app = router(state.clone());
    // Bootstrap a session (the start route only seeds an empty source).
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", "seed topic notes")],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");

    // One action: capture. Generation is enqueued and the handler returns
    // immediately — no cards in the response yet, just the "generating" notice
    // and a queued activity-log row. There is no manual keep gate anymore.
    let captured = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/capture",
            &cookie,
            &[("csrfToken", &csrf_token), ("capture", &source_body())],
        ))
        .await
        .expect("capture");
    assert_eq!(captured.status(), StatusCode::OK);
    let captured = response_text(captured).await;
    assert!(captured.contains("Generating your cards — they'll appear below as they're ready."));
    assert!(captured.contains(r#"<ul id="me-jobs""#));
    assert_not_contains_any(&captured, &["Add all to reviews", ">Keep</button>"]);

    // Drain the background job: real generation + auto-approve every accepted
    // card, scheduling it immediately due. The activity log now shows the
    // finished job, both NATO concepts scheduled for review.
    state.run_pending_jobs_blocking();
    let workspace = workspace_html(&app, &cookie).await;
    assert_activity_succeeded_html(&workspace, 2);

    // The scheduled cards drive the review flow with no keep step in between.
    let review = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/next",
            &cookie,
            &[("csrfToken", &csrf_token)],
        ))
        .await
        .expect("next");
    assert_eq!(review.status(), StatusCode::OK);
    let review = response_text(review).await;
    assert!(review.contains("2 due"));
    assert!(review.contains("Reveal answer"));
    assert!(!review.contains("Add all to reviews"));
}

#[tokio::test]
async fn signed_in_home_surfaces_review_cta_after_generation() {
    // Regression: a learner who generated cards could see "scheduled for
    // review" in the activity log but had no way to start reviewing. Two gaps
    // fed it. Workspace re-renders passed no study view, so the header read
    // "0 due" and the "Start review" button — gated on due_count > 0 — never
    // rendered. And GET / ignored the session entirely, always serving the
    // signed-out cover. After draining a real generation job, the signed-in
    // workspace must surface the due count and Start review CTA on both the
    // POST refresh and a plain GET /.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    // Generate from the seeded source and drain the queue: real generation,
    // auto-approve, cards scheduled immediately due. The helper returns the
    // refreshed workspace, which fetches the live study view — so the due
    // callout and Start review CTA appear, not "0 due" and not the cover.
    let workspace = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert!(
        workspace.contains("Start review"),
        "workspace must surface the Start review CTA: {workspace}"
    );
    assert!(workspace.contains("Due now"));
    assert!(!workspace.contains("0 due"));

    // A plain GET / carrying the session cookie renders the signed-in
    // workspace, not the signed-out cover — the way into review survives a
    // reload rather than depending on the last POST's response.
    let home = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("home request"),
        )
        .await
        .expect("home");
    assert_eq!(home.status(), StatusCode::OK);
    let home = response_text(home).await;
    assert!(
        home.contains("Start review"),
        "signed-in GET / must surface the Start review CTA: {home}"
    );
    assert!(home.contains("Due now"));
    assert_not_contains_any(&home, &["Get started"]);

    // The CTA must actually work: the CSRF token embedded by the GET render
    // has to validate when its form POSTs. A cookie-only GET carries no token
    // in the request, so the home derives it from the session — submitting
    // that derived token to /app/next must open the review, not bounce on
    // "CSRF token does not match session."
    let home_csrf = html_value(&home, "csrfToken");
    let review = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/next",
            &cookie,
            &[("csrfToken", &home_csrf)],
        ))
        .await
        .expect("start review from home CTA");
    assert_eq!(review.status(), StatusCode::OK);
    let review = response_text(review).await;
    assert!(
        review.contains("Reveal answer"),
        "the home Start review CTA must open a review: {review}"
    );
    assert!(!review.contains("CSRF token does not match"));
}

#[tokio::test]
async fn mcq_review_is_click_to_answer_and_grades_case_insensitively() {
    // The NATO letter-A card is multiple choice (answer ALFA, distractors
    // BRAVO/CHARLIE). It must render as clickable options that submit the
    // exact choice — not a static list plus a confusing "type the letter"
    // box — and a typed answer in the wrong case must still grade correct
    // (the bug where a learner typed the right word and was marked wrong).
    let state = ApiState::default();
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &source_body())],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

    let mcq = advance_to_prompt(
        &app,
        &cookie,
        &csrf_token,
        "NATO phonetic alphabet word for A",
    )
    .await;
    assert!(
        mcq.contains(r#"class="me-choice""#),
        "MCQ must render clickable choice buttons: {mcq}"
    );
    assert!(
        mcq.contains(r#"name="answer" value="ALFA""#),
        "each option submits its exact choice text: {mcq}"
    );
    assert!(!mcq.contains("Type the letter"));
    let review_unit_id = html_value(&mcq, "reviewUnitId");

    // Typed answer in the wrong case grades correct.
    let graded = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "alfa"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "review-mcq-case"),
            ],
        ))
        .await
        .expect("submit lowercase answer");
    assert_eq!(graded.status(), StatusCode::OK);
    let graded = response_text(graded).await;
    assert!(
        graded.contains("me-verdict") && graded.contains(">Correct<"),
        "lowercase 'alfa' must grade correct against stored 'ALFA': {graded}"
    );
}

#[tokio::test]
async fn free_response_review_shows_a_prominent_input_not_choice_buttons() {
    // The CAT exercise is free-response: it must show the bounded answer box
    // (not clickable options, and not a hairline underline that reads as a
    // divider).
    let state = ApiState::default();
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &source_body())],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let free = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    assert!(
        free.contains(r#"class="ae-input me-answer-input""#),
        "free-response must show the prominent answer box: {free}"
    );
    assert!(!free.contains(r#"class="me-choice""#));
    let review_unit_id = html_value(&free, "reviewUnitId");

    // Graded (D): a free-response card has no options to mark, so the answer
    // is revealed on one line, with the verdict and the when-it-returns line.
    // No metrics wall, no concept health on the card.
    let graded = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "CHARLIE ALFA TANGO"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "free-graded"),
            ],
        ))
        .await
        .expect("submit free response");
    assert_eq!(graded.status(), StatusCode::OK);
    let graded = response_text(graded).await;
    assert!(graded.contains(r#"<span class="me-verdict">Correct</span>"#));
    assert!(
        graded.contains(r#"<p class="me-answer"><span class="me-answer-label">Answer</span>"#),
        "free-response graded must reveal the answer on one line: {graded}"
    );
    assert!(graded.contains("CHARLIE ALFA TANGO"));
    assert!(graded.contains(r#"<p class="me-next-when"#));
    // No choice rows in the markup (the .me-graded-choice CSS rule still ships
    // in the inline stylesheet — assert on the element, not the class string).
    assert!(!graded.contains(r#"<li class="me-graded-choice"#));
    assert_not_contains_any(
        &graded,
        &["Answer feedback", "Concept health", "This item:"],
    );
}

#[tokio::test]
async fn review_delete_removes_the_card_for_good() {
    // A learner who hits a bad card must be able to delete it from review.
    // Delete archives the current card and drops straight to the next: the
    // due count falls by one, the deleted prompt is gone from the response,
    // and it never resurfaces when the queue is driven again.
    let state = ApiState::default();
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &source_body())],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

    // Land on a known card and delete it.
    let target = "Spell CAT over the phone";
    let on_card = advance_to_prompt(&app, &cookie, &csrf_token, target).await;
    assert!(on_card.contains("2 due"));
    assert!(on_card.contains(">Delete</button>"));
    let review_unit_id = html_value(&on_card, "reviewUnitId");

    let deleted = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/delete",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
            ],
        ))
        .await
        .expect("delete");
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted = response_text(deleted).await;
    // One card remains, due count dropped, and the deleted prompt is gone.
    assert!(
        deleted.contains("1 due"),
        "due count must drop after delete: {deleted}"
    );
    assert!(
        !deleted.contains(target),
        "deleted card must not remain on screen: {deleted}"
    );

    // Drive the queue: the deleted card must never resurface.
    for _ in 0..4 {
        let page = next_review_html(&app, &cookie, &csrf_token, "post-delete").await;
        assert!(
            !page.contains(target),
            "deleted card must not return to the queue: {page}"
        );
    }
}

#[tokio::test]
async fn mobile_form_flow_generates_reveals_and_submits_review() {
    let state = ApiState::default();
    let app = router(state.clone());
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
    assert!(!started.contains("Add all to reviews"));

    // Regenerate from the saved source: enqueue, drain, and reload. Both
    // accepted cards are auto-approved and scheduled — no keep gate, no
    // per-draft approve. The activity log shows the finished job.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

    // Open the review queue: both scheduled cards are due. Take whichever
    // card surfaces first (auto-approve fixes no order), reveal it — every
    // expected answer here contains "ALFA" — and answer it correctly.
    let opened = next_review_html(&app, &cookie, &csrf_token, "open queue").await;
    assert_due_review_html(&opened, 2);
    let review_unit_id = html_value(&opened, "reviewUnitId");

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
                ("answer", correct_answer_for_prompt(&revealed)),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "mobile-review-first"),
            ],
        ))
        .await
        .expect("submit");
    assert_eq!(submitted.status(), StatusCode::OK);
    let submitted = response_text(submitted).await;
    assert_submitted_review_html(&submitted);

    // Both cards were scheduled; clear the remaining one so the queue drains
    // to empty and the workspace returns to its blank state.
    let remaining = next_review_html(&app, &cookie, &csrf_token, "remaining").await;
    let remaining_id = html_value(&remaining, "reviewUnitId");
    let cleared = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &remaining_id),
                ("answer", correct_answer_for_prompt(&remaining)),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "mobile-review-second"),
            ],
        ))
        .await
        .expect("clear remaining");
    assert_eq!(cleared.status(), StatusCode::OK);

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
    assert!(next.contains("What do you want to remember?"));
    assert!(!next.contains("Progress"));
}

#[tokio::test]
async fn mobile_submit_review_reveals_the_verdict_and_correct_answer() {
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    // Generation auto-approves and schedules every accepted card; no manual
    // per-draft approve. Drive the queue to the NATO-A quiz card and answer
    // it wrong to exercise the human result + item-history rollup.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);
    let current = advance_to_prompt(
        &app,
        &cookie,
        &csrf_token,
        "What is the NATO phonetic alphabet",
    )
    .await;
    let review_unit_id = html_value(&current, "reviewUnitId");

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

    // D graded screen, wrong answer: the verdict reads "Try again", the
    // correct option is still revealed (marked) so the learner sees it, and a
    // quiet line says when the card returns. No metrics wall, no concept note
    // — those live on the workspace now.
    assert!(submitted.contains(r#"<span class="me-verdict">Try again</span>"#));
    assert!(submitted.contains("ALFA"));
    assert!(submitted.contains(r#"<li class="me-graded-choice me-graded-choice-correct">"#));
    assert!(submitted.contains("you'll see this again"));
    assert_not_contains_any(
        &submitted,
        &[
            "Expected answer",
            "This item:",
            "last seen",
            "nato letter a",
            "Answer feedback",
            "Concept health",
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
    let state = ApiState::default();
    let app = router(state.clone());
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
    // Generation auto-approves and schedules both cards (same concept). No
    // manual per-draft approve — the activity log confirms two cards landed.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

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
        .clone()
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

    // D keeps the graded screen to the verdict — no concept note piled on.
    assert!(submitted.contains(r#"<span class="me-verdict">Try again</span>"#));
    assert_not_contains_any(&submitted, &["nato letter a", "Concept health"]);

    // Concept health rolls up on the workspace, off the per-card loop: once
    // the queue drains, Next lands there with both attempts on the shared
    // concept folded into one row.
    let workspace = next_review_html(&app, &cookie, &csrf_token, "workspace").await;
    assert!(workspace.contains("Concept health"));
    assert!(workspace.contains("nato letter a"));
    assert!(workspace.contains("1 of 2 correct (50.0%)"));
    assert!(workspace.contains("declining"));
}

#[tokio::test]
async fn management_surface_lists_concepts_worst_first() {
    let state = ApiState::default();
    let app = router(state.clone());
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
    // Generation auto-approves and schedules both cards — no manual approve.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

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
    assert!(!workspace.contains("Add all to reviews"));
    assert_not_contains_any(&workspace, &["chart", "streak", "badge"]);
}

#[tokio::test]
async fn auth_rendered_forms_do_not_expose_session_credentials() {
    let state = ApiState::default();
    let app = router(state.clone());
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
    // The workspace with a finished activity-log row must not leak credentials.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);
    assert!(!generated.contains(r#"name="accountId""#));
    assert!(!generated.contains(r#"name="sessionToken""#));
    assert!(!generated.contains("acct_"));

    // The review screen the scheduled cards drive must not leak them either.
    let review = next_review_html(&app, &cookie, &csrf_token, "review").await;
    assert!(!review.contains(r#"name="sessionToken""#));
    assert!(!review.contains("acct_"));
    assert!(review.contains("Reveal answer"));
}

#[tokio::test]
async fn review_escape_hatches_render_and_drive_the_mobile_queue() {
    let state = ApiState::default();
    let app = router(state.clone());
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
    // Generation auto-approves and schedules both cards. Drive the queue to
    // the CAT *exercise* card — the one that carries the escape hatches.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);
    let approved = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    assert!(approved.contains("Reveal answer"));
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
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    assert_source_session_mutations_require_csrf(&app, &cookie, &source_id).await;
    let review_unit_id =
        schedule_review_for_csrf(&app, &state, &cookie, &csrf_token, &source_id).await;
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
    // The keep flow is gone; the new CSRF-protected session mutation is the
    // job retry. It must reject a forged cross-site POST just like the rest.
    assert_forbidden_form(
        app,
        cookie,
        "/app/jobs/retry",
        &[("jobId", "job-withheld")],
        "retry without csrf",
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

/// Generation auto-approves and schedules cards (no keep gate); open the
/// review queue and return the current review unit id so the review-mutation
/// CSRF matrix has a real target.
async fn schedule_review_for_csrf(
    app: &axum::Router,
    state: &ApiState,
    cookie: &str,
    csrf_token: &str,
    source_id: &str,
) -> String {
    generate_source_html(app, state, cookie, csrf_token, source_id).await;
    let current = next_review_html(app, cookie, csrf_token, "csrf review").await;
    html_value(&current, "reviewUnitId")
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

/// Enqueue generation for a saved source, drain the job synchronously (real
/// structured-block generation + auto-approve/schedule), and return the
/// reloaded workspace — which now reflects the scheduled, due cards and a
/// succeeded activity-log row. This is the async-model replacement for the
/// old synchronous "generate → keep" dance.
async fn generate_source_html(
    app: &axum::Router,
    state: &ApiState,
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
    let generated = response_text(generated).await;
    // The handler returns immediately with the queued job, before any card
    // exists. Drain the queue so the deck is generated and scheduled, then
    // re-render the workspace so the activity log shows the finished job.
    assert!(generated.contains("Generating — watch the activity log."));
    state.run_pending_jobs_blocking();
    workspace_html(app, cookie).await
}

/// Drive `/app/next` until the current review item's prompt contains
/// `needle`, returning that page. Auto-approve schedules every accepted card
/// at once, so which card surfaces first is not fixed; this makes a flow that
/// targets a specific card order-independent. Skips non-matching items via
/// `/app/skip` so they rotate to the back of the queue.
async fn advance_to_prompt(
    app: &axum::Router,
    cookie: &str,
    csrf_token: &str,
    needle: &str,
) -> String {
    for _ in 0..8 {
        let page = next_review_html(app, cookie, csrf_token, "advance").await;
        if page.contains(needle) {
            return page;
        }
        let review_unit_id = html_value(&page, "reviewUnitId");
        let skipped = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/skip",
                cookie,
                &[("csrfToken", csrf_token), ("reviewUnitId", &review_unit_id)],
            ))
            .await
            .expect("skip while advancing");
        assert_eq!(skipped.status(), StatusCode::OK);
    }
    panic!("no review item matched prompt {needle:?}");
}

/// Re-render the signed-in workspace (the activity log + saved material) the
/// way a learner reloads it: a plain `GET /` carrying the session cookie. The
/// home reads the live job list and study view, so a job drained just before
/// this call shows its final `succeeded`/`failed` status and its scheduled
/// cards drive the due count and Start review CTA.
async fn workspace_html(app: &axum::Router, cookie: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("workspace request"),
        )
        .await
        .expect("workspace refresh");
    assert_eq!(response.status(), StatusCode::OK);
    response_text(response).await
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
        AccountRegistry::default().with_auth_config(AuthConfig::default().with_debug_links(true)),
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
    assert!(!saved.contains("Add all to reviews"));
    assert!(!saved.contains("acct_fc9e1ff15d47bd67"));
    assert!(!saved.contains("Save account email"));
    let saved_csrf_token = html_value(&saved, "csrfToken");
    let source_id = html_value(&saved, "sourceId");

    let restarted_state = ApiState::new(super::AccountRegistry::with_store_root(&store_root));
    let restarted_app = router(restarted_state.clone());
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

    // The resumed session can still regenerate the persisted source: enqueue,
    // drain, and confirm the cards land. (The old synchronous "Add all to
    // reviews" keep gate is gone — success now shows in the activity log and
    // the cards are auto-scheduled.)
    let generated = generate_source_html(
        &restarted_app,
        &restarted_state,
        &saved_cookie,
        &saved_csrf_token,
        &source_id,
    )
    .await;
    assert_activity_succeeded_html(&generated, 2);
}

#[tokio::test]
async fn mobile_source_archive_hides_source_and_blocks_regeneration() {
    let state = ApiState::default();
    let app = router(state.clone());
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
    assert!(archived.contains("What do you want to remember?"));
    assert!(!archived.contains("NATO practice notes"));
    assert!(!archived.contains("What is the NATO phonetic alphabet word for A?"));

    // Regenerating an archived source still enqueues a job — the request only
    // queues work, it does not validate the source. The worker is where it
    // fails: the source is gone, so generation surfaces "Source not found."
    // as the job's error in the activity log rather than as a sync response.
    let queued = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("generate archived source");
    assert_eq!(queued.status(), StatusCode::OK);
    let queued = response_text(queued).await;
    assert!(queued.contains("Generating — watch the activity log."));

    state.run_pending_jobs_blocking();
    let regenerated = workspace_html(&app, &cookie).await;
    assert!(
        regenerated.contains(r#"data-status="failed""#),
        "archived-source job must fail in the worker: {regenerated}"
    );
    assert!(
        regenerated.contains("Source not found."),
        "failed job meta must surface the source-not-found error: {regenerated}"
    );
    assert!(!regenerated.contains("Add all to reviews"));
    assert!(!regenerated.contains("What is the NATO phonetic alphabet word for A?"));
}

#[tokio::test]
async fn mobile_retry_requeues_and_reruns_a_failed_job() {
    // A failed job is the one thing a learner can act on in the activity
    // log. Setup mirrors the archive case so the job fails for a real reason
    // (the source is gone), then we drive the real /app/jobs/retry endpoint
    // and confirm the worker actually runs it a second time.
    let state = ApiState::default();
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("title", "NATO practice notes"), ("body", &source_body())],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");

    app.clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/source/archive",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("archive source");
    app.clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("generate archived source");

    state.run_pending_jobs_blocking();
    let failed_html = workspace_html(&app, &cookie).await;
    assert!(
        failed_html.contains(r#"data-status="failed""#),
        "precondition: the job must fail first: {failed_html}"
    );

    // The retry control carries the job id; recover it the way the browser
    // submits it, then assert the worker ran exactly once so far.
    let job_id = failed_html
        .split_once(r#"data-job-id=""#)
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(id, _)| id.to_owned())
        .expect("activity log renders a job id");
    assert_eq!(
        state.job(&job_id).expect("job exists").attempts,
        1,
        "the job ran once before retry"
    );

    let retried = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/jobs/retry",
            &cookie,
            &[("csrfToken", &csrf_token), ("jobId", &job_id)],
        ))
        .await
        .expect("retry");
    assert_eq!(retried.status(), StatusCode::OK);
    // The endpoint re-queues the failed job and clears its error before the
    // worker picks it back up.
    let requeued = state.job(&job_id).expect("job exists");
    assert_eq!(requeued.status, crate::JobStatus::Queued);
    assert!(requeued.error.is_none());

    // Draining runs it a second time — attempts increments, proving retry
    // actually re-ran generation (it fails again: the source is still gone).
    state.run_pending_jobs_blocking();
    let reran = state.job(&job_id).expect("job exists");
    assert_eq!(reran.status, crate::JobStatus::Failed);
    assert_eq!(reran.attempts, 2, "retry must re-run the worker");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_drains_an_enqueued_capture_and_broadcasts_success() {
    // The deterministic run_pending_blocking shim covers job *logic*; this is
    // the one test that exercises the real spawned worker (spawn_blocking +
    // the semaphore) and the broadcast the SSE activity feed subscribes to.
    let state = ApiState::default();
    state.start_worker();
    let mut updates = state.subscribe_jobs();
    let app = router(state.clone());

    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", "seed topic notes")],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let csrf_token = html_value(&response_text(started).await, "csrfToken");
    app.clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/capture",
            &cookie,
            &[("csrfToken", &csrf_token), ("capture", &source_body())],
        ))
        .await
        .expect("capture");

    // The spawned worker — not the test shim — must run the job and fan a
    // succeeded snapshot out over the broadcast channel.
    let account_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("broadcast closed before a succeeded job")
                }
            };
            let job: serde_json::Value =
                serde_json::from_str(&update.payload).expect("job payload is json");
            if job["status"] == "succeeded" {
                return update.account_id;
            }
        }
    })
    .await
    .expect("the worker must broadcast a succeeded job within 5s");

    // The broadcast carries the owning account — the field the SSE handler
    // filters on so a learner only ever receives their own jobs.
    assert!(
        !account_id.is_empty(),
        "a succeeded broadcast must be account-tagged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_history_survives_a_restart_through_the_file_backed_host() {
    // Two ApiState "processes" over one file-backed store: capture a job in
    // the first, then prove a fresh state built on the same store restores
    // it. Exercises the real router, the spawned worker, and the
    // ApiState::new -> with_persistence wiring end to end — the integration
    // the JobQueue unit tests can't reach on their own.
    let store =
        std::env::temp_dir().join(format!("me-057-restart-{:032x}", rand::random::<u128>()));

    let account_id;
    let job_id;
    {
        // Process 1: capture through the real router + spawned worker.
        let state = ApiState::new(AccountRegistry::with_store_root(store.clone()));
        state.start_worker();
        let app = router(state.clone());
        let started = app
            .clone()
            .oneshot(form_request("POST", "/app/start", &[("capture", "seed")]))
            .await
            .expect("start");
        let cookie = session_cookie(&started);
        let csrf = html_value(&response_text(started).await, "csrfToken");
        app.clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/capture",
                &cookie,
                &[("csrfToken", &csrf), ("capture", &source_body())],
            ))
            .await
            .expect("capture");

        // Wait for the durable write the next "process" will read — not just
        // the broadcast, which fires a step before persist().
        let jobs_file = store.join("_jobs.json");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !std::fs::read_to_string(&jobs_file)
                .is_ok_and(|text| text.contains("\"succeeded\""))
            {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("the worker must persist a succeeded job within 10s");

        let disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&jobs_file).unwrap()).unwrap();
        account_id = disk[0]["account_id"].as_str().unwrap().to_owned();
        job_id = disk[0]["id"].as_str().unwrap().to_owned();
    }

    // Process 2: a fresh state on the same store restores the history.
    let restarted = ApiState::new(AccountRegistry::with_store_root(store.clone()));
    let restored = restarted
        .job(&job_id)
        .expect("the job must be restored after a restart");
    assert_eq!(restored.status.as_str(), "succeeded");
    assert!(
        restored.card_count >= 1,
        "restored job keeps its card count"
    );
    assert_eq!(restored.account_id, account_id);
    assert_eq!(
        restarted.jobs_for_account_id(&account_id).len(),
        1,
        "the restored job is visible in the learner's activity log"
    );

    let _ = std::fs::remove_dir_all(&store);
}

/// Six distinct NATO-letter cards in the proven `source_body()` shape:
/// same-category distractors so the distractor gate passes, distinct content
/// so the dedup pass never collapses them across sources. Structured blocks
/// route through the deterministic provider — no network.
fn nato_letter_sources() -> Vec<String> {
    let words = [
        ('A', "ALFA"),
        ('B', "BRAVO"),
        ('C', "CHARLIE"),
        ('D', "DELTA"),
        ('E', "ECHO"),
        ('F', "FOXTROT"),
    ];
    words
        .iter()
        .enumerate()
        .map(|(i, (letter, word))| {
            let d1 = words[(i + 1) % words.len()].1;
            let d2 = words[(i + 2) % words.len()].1;
            format!(
                "Concept: NATO letter {letter}\nActivity: quiz\nStage: recognition-3\n\
                     Question: What is the NATO phonetic alphabet word for {letter}?\n\
                     Answer: {word}\nDistractors: {d1}, {d2}\n\
                     Reference: The NATO phonetic alphabet word for {letter} is {word}."
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_generations_for_one_account_do_not_clobber_each_other() {
    // Regression for the lost-update race (dogfood 2026-06-23): capturing
    // several sources for one account queues several generation jobs that the
    // worker runs concurrently, each read-modify-writing the whole study.json.
    // Without per-account serialization the writes clobber one another and
    // cards are silently lost — a NATO capture's 26 cards vanished when an
    // overlapping capture won the write race. Invariant under test: every card
    // a job reports scheduling is actually persisted.
    let store =
        std::env::temp_dir().join(format!("me-059-concurrent-{:032x}", rand::random::<u128>()));

    // Worker stays OFF while we save the sources *sequentially* (so the source
    // writes themselves don't race) and queue one generation job each.
    let state = ApiState::new(AccountRegistry::with_store_root(store.clone()));
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request("POST", "/app/start", &[("capture", "seed")]))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let csrf = html_value(&response_text(started).await, "csrfToken");

    let sources = nato_letter_sources();
    let source_count = sources.len();
    for body in &sources {
        app.clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/capture",
                &cookie,
                &[("csrfToken", &csrf), ("capture", body)],
            ))
            .await
            .expect("capture");
    }

    // Now drain every queued job at once — this is the race.
    state.start_worker();

    let jobs_file = store.join("_jobs.json");
    let terminal_count = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|jobs| {
                jobs.as_array().map(|a| {
                    a.iter()
                        .filter(|j| matches!(j["status"].as_str(), Some("succeeded" | "failed")))
                        .count()
                })
            })
            .unwrap_or(0)
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while std::fs::read_to_string(&jobs_file)
            .map(|text| terminal_count(&text))
            .unwrap_or(0)
            != source_count
        {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("all generation jobs must finish within 20s");

    // The cards each job reported scheduling must all be on disk.
    let jobs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&jobs_file).unwrap()).unwrap();
    let reported: i64 = jobs
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["card_count"].as_i64().unwrap_or(0))
        .sum();

    let account_dir = std::fs::read_dir(&store)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("acct_"))
        })
        .expect("account store dir");
    let study: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(account_dir.join("study.json")).unwrap())
            .unwrap();
    let persisted = i64::try_from(study["reviewUnits"].as_array().map_or(0, Vec::len)).unwrap();

    assert_eq!(
        jobs.as_array().unwrap().len(),
        source_count,
        "every capture must have produced a job"
    );
    assert!(reported > 0, "the jobs must have scheduled some cards");
    assert_eq!(
        persisted, reported,
        "every scheduled card must persist: {reported} reported across jobs, \
             {persisted} on disk — a shortfall is the concurrent-write clobber"
    );

    let _ = std::fs::remove_dir_all(&store);
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
    let source = save_source(&routed_app, &account, "NATO practice notes", &source_body()).await;
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

    // Simulate a restart: a fresh ApiState (and a fresh in-memory job queue)
    // over the same durable Postgres store. The source persisted, so the
    // resumed process can generate from it asynchronously.
    let restarted_state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let restarted_app = router(restarted_state.clone());
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

    // Drain the enqueued job: generation runs and the cards auto-schedule.
    restarted_state.run_pending_jobs_blocking();

    let next = restarted_app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/next",
            &cookie,
            &[("csrfToken", &csrf_token)],
        ))
        .await
        .expect("next after restart");
    assert_eq!(next.status(), StatusCode::OK);
    let next = response_text(next).await;
    assert!(next.contains("Reveal answer"));
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
    let second_source = save_source(&app, &second, "Latin notes", "Poena means punishment.").await;

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
    let source_id = create_source_v1(&app, &account, "NATO practice notes", &source_body()).await;
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
async fn v1_project_deck_ttl_and_event_invalidation_stop_scheduling() {
    let store_root = temp_store_root("volatile-project-deck");
    let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));
    let account = create_account_v1(&app, "volatile@example.com").await;
    let deck = create_project_deck_v1(
        &app,
        &account,
        "memory-engine",
        "API split decision notes",
        &source_body(),
        Some(i64::MAX),
    )
    .await;
    let deck_id = deck["deckId"].as_str().expect("deck id");
    let draft_ids = generate_source_v1_draft_ids(&app, &account, deck_id).await;
    let draft_id = draft_ids.first().expect("first deck draft").to_owned();
    let stale_unapproved_draft_id = draft_ids
        .iter()
        .find(|candidate| candidate.as_str() != draft_id.as_str())
        .expect("unapproved stale deck draft")
        .to_owned();
    let review_unit_id = approve_draft_v1(&app, &account, &draft_id).await;

    assert_eq!(
        next_review_v1(&app, &account).await,
        review_unit_id,
        "project deck starts as schedulable while TTL is in the future"
    );

    let invalidated = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/project-decks/{deck_id}/invalidate",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "event": "architecture-change: api-state split accepted"
            }),
        ))
        .await
        .expect("invalidate project deck");
    assert_eq!(invalidated.status(), StatusCode::OK);
    let invalidated = response_json(invalidated).await;
    assert_eq!(invalidated["current"], json!(null));
    assert_eq!(invalidated["dueCount"], json!(0));

    let next_after_event = next_review_v1_body(&app, &account).await;
    assert_eq!(next_after_event["current"], json!(null));
    assert_eq!(next_after_event["dueCount"], json!(0));

    let approved_stale_draft = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/drafts/{stale_unapproved_draft_id}/approve",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("approve stale invalidated draft");
    assert_eq!(approved_stale_draft.status(), StatusCode::OK);
    let approved_stale_draft = response_json(approved_stale_draft).await;
    assert_eq!(approved_stale_draft["current"], json!(null));
    assert_eq!(approved_stale_draft["dueCount"], json!(0));

    let expired_deck = create_project_deck_v1(
        &app,
        &account,
        "memory-engine",
        "Expired deployment note",
        &expired_project_deck_body(),
        Some(0),
    )
    .await;
    let expired_deck_id = expired_deck["deckId"].as_str().expect("expired deck id");
    let expired_draft_id = generate_source_v1_latest_draft(&app, &account, expired_deck_id).await;
    let approved_expired = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/drafts/{expired_draft_id}/approve",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("approve expired deck draft");
    assert_eq!(approved_expired.status(), StatusCode::OK);
    let approved_expired = response_json(approved_expired).await;
    assert_eq!(approved_expired["current"], json!(null));
    assert_eq!(approved_expired["dueCount"], json!(0));
}

#[tokio::test]
async fn v1_project_deck_invalidation_rejects_regular_sources() {
    let store_root = temp_store_root("volatile-project-deck-regular-source");
    let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));
    let account = create_account_v1(&app, "regular-source@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Stable NATO note",
        &expired_project_deck_body(),
    )
    .await;

    let invalidated = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/project-decks/{source_id}/invalidate",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "event": "architecture-change: should not hit stable source"
            }),
        ))
        .await
        .expect("invalidate regular source as project deck");
    assert_eq!(invalidated.status(), StatusCode::NOT_FOUND);

    let draft_id = generate_source_v1(&app, &account, &source_id).await;
    let review_unit_id = approve_draft_v1(&app, &account, &draft_id).await;
    assert_eq!(
        next_review_v1(&app, &account).await,
        review_unit_id,
        "regular source must remain active after rejected project-deck invalidation"
    );
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
    let _ = submit_review_v1_body(&app, &account, &first_id, "ALFA", "api-feedback-first").await;
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
    let source_id = create_source_v1(&app, &account, "NATO practice notes", &source_body()).await;
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

async fn create_project_deck_v1(
    app: &axum::Router,
    account: &TestAccount,
    project_key: &str,
    title: &str,
    body: &str,
    ttl_expires_at: Option<i64>,
) -> Value {
    let response = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!("/v1/accounts/{}/project-decks", account.account_id),
            &account.session_token,
            &json!({
                "projectKey": project_key,
                "title": title,
                "body": body,
                "ttlExpiresAt": ttl_expires_at
            }),
        ))
        .await
        .expect("create project deck");
    assert_eq!(response.status(), StatusCode::CREATED);

    response_json(response).await
}

async fn generate_source_v1(app: &axum::Router, account: &TestAccount, source_id: &str) -> String {
    generate_source_v1_draft_ids(app, account, source_id)
        .await
        .into_iter()
        .next()
        .expect("draft id")
}

async fn generate_source_v1_draft_ids(
    app: &axum::Router,
    account: &TestAccount,
    source_id: &str,
) -> Vec<String> {
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

    response_json(response).await["drafts"]
        .as_array()
        .expect("drafts")
        .iter()
        .map(|draft| draft["id"].as_str().expect("draft id").to_owned())
        .collect()
}

async fn generate_source_v1_latest_draft(
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

    let body = response_json(response).await;
    body["drafts"]
        .as_array()
        .expect("drafts")
        .last()
        .expect("latest draft")["id"]
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

async fn next_review_v1_body(app: &axum::Router, account: &TestAccount) -> Value {
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

    response_json(response).await
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

fn v1_json_request(method: &str, uri: &str, session_token: &str, body: &Value) -> Request<Body> {
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

/// The async-model successor to `assert_keep_flow_html`: after a job drains,
/// the workspace shows a finished activity-log row (a succeeded job with a
/// card count, already scheduled for review) rather than a manual keep gate.
/// `expected_cards` pins how many cards the generation scheduled.
fn assert_activity_succeeded_html(body: &str, expected_cards: usize) {
    assert!(
        body.contains(r#"data-status="succeeded""#),
        "activity log must show a succeeded job: {body}"
    );
    assert!(
        body.contains(&format!(
            "{expected_cards} {} · scheduled for review",
            if expected_cards == 1 { "card" } else { "cards" }
        )),
        "activity meta must report {expected_cards} cards scheduled for review: {body}"
    );
    assert!(body.contains(r#"<ul id="me-jobs""#));
    // The keep gate is gone: no manual "Add all to reviews" / per-card Keep,
    // and no raw generation internals leak into the learner-facing markup.
    assert_not_contains_any(
        body,
        &[
            "Add all to reviews",
            ">Keep</button>",
            "Choose what to keep",
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

fn assert_due_review_html(body: &str, due_count: usize) {
    assert!(body.contains(&format!("{due_count} due")));
    assert!(body.contains("Reveal answer"));
    assert_not_contains_any(
        body,
        &[
            "Generated material",
            "Add all to reviews",
            "drafts",
            "validation",
            "recognition-3",
            "Save account email",
            "Session ready for",
            "acct_",
        ],
    );
}

fn assert_submitted_review_html(body: &str) {
    // D graded screen: the verdict, the answer revealed in place (correct
    // option marked), one quiet line on when it returns, and a primary Next.
    // The metrics wall and concept health moved to the workspace, off the
    // per-card loop — so they must NOT appear here.
    assert!(body.contains(r#"<span class="me-verdict">Correct</span>"#));
    // The first due card may be MCQ or free response, so accept either reveal
    // form: a marked correct option, or a one-line answer.
    let reveals_answer = body.contains(r#"<li class="me-graded-choice me-graded-choice-correct">"#)
        || body.contains(r#"<p class="me-answer">"#);
    assert!(
        reveals_answer,
        "graded screen must reveal the answer: {body}"
    );
    assert!(body.contains("you'll see this again"));
    assert!(body.contains("Next"));
    assert_not_contains_any(
        body,
        &[
            "Answer feedback",
            "Expected answer",
            "This item:",
            "Concept health",
            "response time",
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

/// The *correct* answer for whichever `source_body` card the page is showing:
/// the CAT spelling exercise expects "CHARLIE ALFA TANGO", the NATO-A quiz
/// expects "ALFA". Lets a flow answer the current card correctly without
/// pinning the queue order auto-approve leaves unspecified.
fn correct_answer_for_prompt(body: &str) -> &'static str {
    if body.contains("Spell CAT over the phone") {
        "CHARLIE ALFA TANGO"
    } else {
        "ALFA"
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

async fn save_source(app: &axum::Router, account: &TestAccount, title: &str, body: &str) -> Value {
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

fn expired_project_deck_body() -> String {
    [
        "Concept: NATO letter D",
        "Activity: quiz",
        "Stage: recognition-3",
        "Question: What is the NATO phonetic alphabet word for D?",
        "Answer: DELTA",
        "Distractors: ALFA, BRAVO",
        "Reference: The NATO phonetic alphabet word for D is DELTA.",
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
                let _ =
                    client.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema));
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
    let state = ApiState::new(registry);

    let fresh = state
        .request_magic_link("learner@example.com", "test-client")
        .expect("fresh link")
        .debug_link
        .expect("debug link");
    let fresh_token = fresh.split("token=").nth(1).expect("token").to_owned();
    assert!(state.verify_magic_link(&fresh_token).is_ok());

    let expired_link = state
        .request_magic_link("learner@example.com", "test-client")
        .expect("stale link")
        .debug_link
        .expect("debug link");
    let stale_token = expired_link
        .split("token=")
        .nth(1)
        .expect("token")
        .to_owned();
    EXPIRY_CLOCK.fetch_add(AUTH_CHALLENGE_TTL_MS + 1, Ordering::SeqCst);

    assert!(
        state.verify_magic_link(&stale_token).is_err(),
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
    let state = ApiState::new(registry);
    let link = state
        .request_magic_link("learner@example.com", "test-client")
        .expect("link")
        .debug_link
        .expect("debug link");
    let token = link.split("token=").nth(1).expect("token").to_owned();
    let session = state.verify_magic_link(&token).expect("session");

    let response = memory_engine_api_state::html_with_browser_session(&session, String::new());
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        response.headers()[SET_COOKIE].clone(),
    );
    assert!(state
        .require_browser_session(&headers, session.csrf_token())
        .is_ok());

    SESSION_CLOCK.fetch_add(
        super::app_session_max_age_ms().saturating_add(1),
        Ordering::SeqCst,
    );
    assert!(
        state
            .require_browser_session(&headers, session.csrf_token())
            .is_err(),
        "an expired browser session must be rejected server-side"
    );
    assert!(
        state
            .require_browser_session(&headers, session.csrf_token())
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
    let state = ApiState::new(registry);
    let account = state
        .create_account("learner@example.com")
        .expect("account");

    let source = state
        .save_source(
            &account.account_id,
            &account.session_token,
            &super::CreateSourceRequest {
                title: "NATO practice notes".to_owned(),
                body: source_body(),
            },
        )
        .expect("source");
    let generated = state
        .generate_source(
            &account.account_id,
            &account.session_token,
            &source.source_id,
        )
        .expect("generate");
    let draft_id = generated.drafts.first().expect("draft").id.clone();
    state
        .approve_draft(&account.account_id, &account.session_token, &draft_id)
        .expect("approve");

    let due = state
        .next_review(&account.account_id, &account.session_token)
        .expect("next review");
    let current = due.current.expect("approved unit is due");
    let review_unit_id = current.review_unit_id.to_string();
    let answered = state
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

    let same_moment = state
        .next_review(&account.account_id, &account.session_token)
        .expect("next review");
    assert!(
        same_moment.current.is_none(),
        "a correctly answered unit must not be due again at the same moment"
    );

    SCHEDULE_CLOCK.fetch_add(30 * 86_400_000, Ordering::SeqCst);
    let later = state
        .next_review(&account.account_id, &account.session_token)
        .expect("next review");
    assert!(
        later.current.is_some(),
        "the unit must come due again once enough real time passes"
    );
}

static RESUBMIT_CLOCK: AtomicI64 = AtomicI64::new(0);

fn resubmit_clock() -> i64 {
    RESUBMIT_CLOCK.load(Ordering::SeqCst)
}

#[tokio::test]
async fn answering_the_same_card_across_due_cycles_does_not_collide() {
    // Regression: the review form embedded a constant idempotency key
    // (`review-{review_unit_id}`), so the second-ever review of any card
    // collided with the persisted first one ("Duplicate applied review") and
    // a learner could answer each card exactly once — fatal for spaced
    // repetition. The key must vary per attempt; the form now appends the
    // rep count. Drive one card through two due cycles under an advancing
    // clock and confirm the second review applies cleanly.
    RESUBMIT_CLOCK.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
    let registry = AccountRegistry::default().with_clock(resubmit_clock);
    let state = ApiState::new(registry);
    let app = router(state.clone());

    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &source_body())],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    // First review of the CAT card: capture the rendered idempotency key and
    // answer wrong, so it relearns and is due again within minutes.
    let first = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let review_unit_id = html_value(&first, "reviewUnitId");
    let first_key = html_value(&first, "idempotencyKey");
    let graded_once = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "not the answer"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", &first_key),
            ],
        ))
        .await
        .expect("first submit");
    assert_eq!(graded_once.status(), StatusCode::OK);
    let graded_once = response_text(graded_once).await;
    assert!(
        graded_once.contains("me-verdict"),
        "first answer must grade: {graded_once}"
    );

    // Real time passes; the card comes due again.
    RESUBMIT_CLOCK.fetch_add(86_400_000, Ordering::SeqCst);

    // Second review of the same card: the key must differ from the first,
    // and the submit must apply cleanly rather than colliding.
    let second = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let second_key = html_value(&second, "idempotencyKey");
    assert_ne!(
        first_key, second_key,
        "the idempotency key must change between review attempts"
    );
    let graded_twice = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "CHARLIE ALFA TANGO"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", &second_key),
            ],
        ))
        .await
        .expect("second submit");
    assert_eq!(graded_twice.status(), StatusCode::OK);
    let graded_twice = response_text(graded_twice).await;
    assert!(
        !graded_twice.contains("Duplicate applied review"),
        "re-reviewing a card must not collide on idempotency: {graded_twice}"
    );
    assert!(
        graded_twice.contains("me-verdict"),
        "second answer must grade: {graded_twice}"
    );
}

#[tokio::test]
async fn review_form_leaves_response_time_blank_for_honest_measurement() {
    // The review form used to embed a fabricated constant response time, so
    // every browser answer looked like a fast 1.8-second recall. The rendered
    // form must instead leave the field blank for the progressive-enhancement
    // script to fill with the real presentation-to-submit elapsed time; with
    // JavaScript off the blank submits as-is and the server grades it
    // conservatively.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let page = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    assert!(
        page.contains(r#"<input type="hidden" name="responseTimeMs" value="">"#),
        "review form must render a blank response time for the script to fill: {page}"
    );
    assert!(
        !page.contains(r#"name="responseTimeMs" value="1800""#),
        "review form must not fabricate a constant response time: {page}"
    );
}

static HONEST_TIMING_CLOCK: AtomicI64 = AtomicI64::new(0);

fn honest_timing_clock() -> i64 {
    HONEST_TIMING_CLOCK.load(Ordering::SeqCst)
}

/// Sign back in over the rendered magic-link flow and return the fresh
/// browser session cookie and CSRF token. Maturing a card spans weeks of
/// simulated clock, which outlives any single 14-day browser session — the
/// learner signs back in between review sessions, exactly like the product.
async fn refresh_login(app: &axum::Router, email: &str) -> (String, String) {
    let requested = app
        .clone()
        .oneshot(form_request("POST", "/app/account", &[("email", email)]))
        .await
        .expect("request magic link");
    assert_eq!(requested.status(), StatusCode::OK);
    let verify_path = debug_sign_in_path(&response_text(requested).await);
    let verified = app
        .clone()
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
    let csrf_token = html_value(&verified, "csrfToken");
    (cookie, csrf_token)
}

/// Bootstrap a fresh account, mature its CAT card through three correct
/// reviews under the advancing test clock (signing back in as each session
/// expires), then submit a fourth correct answer carrying `timing` exactly as
/// a browser form field would (`None` omits the field entirely, as a client
/// that never rendered it). Returns the graded page for that fourth, mature
/// review.
async fn mature_cat_card_then_submit(
    app: &axum::Router,
    state: &ApiState,
    label: &str,
    timing: Option<&str>,
) -> String {
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &source_body())],
        ))
        .await
        .expect("start");
    let guest_cookie = session_cookie(&started);
    let started = response_text(started).await;
    let guest_csrf_token = html_value(&started, "csrfToken");

    // Attach an email so the account survives session expiry via magic-link
    // sign-in while the clock advances through the review cycles below.
    // Saving rotates the account and session, so adopt the rotated cookie,
    // CSRF token, and source id from the saved page.
    let email = format!("{label}@example.com");
    let saved = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/save-account",
            &guest_cookie,
            &[("csrfToken", &guest_csrf_token), ("email", &email)],
        ))
        .await
        .expect("save account email");
    assert_eq!(saved.status(), StatusCode::OK, "save email for {label}");
    let mut cookie = session_cookie(&saved);
    let saved = response_text(saved).await;
    let mut csrf_token = html_value(&saved, "csrfToken");
    let source_id = html_value(&saved, "sourceId");

    generate_source_html(app, state, &cookie, &csrf_token, &source_id).await;

    for cycle in 0..3 {
        let page = advance_to_prompt(app, &cookie, &csrf_token, "Spell CAT over the phone").await;
        let review_unit_id = html_value(&page, "reviewUnitId");
        let idempotency_key = html_value(&page, "idempotencyKey");
        let graded = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/submit",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("reviewUnitId", &review_unit_id),
                    ("answer", "CHARLIE ALFA TANGO"),
                    ("responseTimeMs", "1500"),
                    ("idempotencyKey", &idempotency_key),
                ],
            ))
            .await
            .expect("maturing submit");
        assert_eq!(graded.status(), StatusCode::OK, "maturing cycle {cycle}");
        // Advance just past the card's next-review horizon so it is due
        // again, then sign back in: the horizon can outlive the fixed 14-day
        // browser session.
        advance_clock_past_next_review(&response_text(graded).await);
        (cookie, csrf_token) = refresh_login(app, &email).await;
    }

    let page = advance_to_prompt(app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let review_unit_id = html_value(&page, "reviewUnitId");
    let idempotency_key = html_value(&page, "idempotencyKey");
    let mut fields: Vec<(&str, &str)> = vec![
        ("csrfToken", &csrf_token),
        ("reviewUnitId", &review_unit_id),
        ("answer", "CHARLIE ALFA TANGO"),
        ("idempotencyKey", &idempotency_key),
    ];
    if let Some(timing) = timing {
        fields.push(("responseTimeMs", timing));
    }
    let graded = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &fields,
        ))
        .await
        .expect("mature submit");
    assert_eq!(
        graded.status(),
        StatusCode::OK,
        "mature submit with timing {timing:?}"
    );
    response_text(graded).await
}

/// Advance the test clock just past the graded page's next-review horizon so
/// the card is due again on the next cycle. Hour-scale (or missing) horizons
/// advance one day; day-scale horizons advance one day beyond the rounded
/// count to absorb the phrase's rounding.
fn advance_clock_past_next_review(page: &str) {
    let marker = "you'll see this again in ~";
    let days = page.find(marker).map_or(1, |start| {
        let rest = &page[start + marker.len()..];
        let digits = rest
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if rest[digits.len()..].trim_start().starts_with("day") {
            digits.parse::<i64>().unwrap_or(0) + 1
        } else {
            1
        }
    });
    HONEST_TIMING_CLOCK.fetch_add(days * 86_400_000, Ordering::SeqCst);
}

/// Parse the "~N days" horizon out of a graded page's next-review phrase.
/// Panics when the phrase is missing or hour-scale: a mature card's next
/// interval must be day-scale for the rating comparison to mean anything.
fn next_review_days(page: &str) -> i64 {
    let marker = "you'll see this again in ~";
    let start = page
        .find(marker)
        .unwrap_or_else(|| panic!("graded page carries no next-review phrase: {page}"))
        + marker.len();
    let rest = &page[start..];
    let digits = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    let unit = rest[digits.len()..].trim_start();
    assert!(
        unit.starts_with("day"),
        "mature card must schedule in days, got: {}",
        &rest[..rest.len().min(40)]
    );
    digits.parse().expect("day count")
}

#[tokio::test]
async fn mature_correct_answers_rate_easy_only_when_genuinely_fast() {
    // The scheduler rates a mature correct answer Easy only when it was
    // genuinely fast. A slow correct answer rates Good — a strictly shorter
    // next interval — and any timing shape the client cannot vouch for
    // (missing, blank, malformed, negative, zero, absurdly large) grades as
    // the slowest plausible answer: the same Good-rated interval as the slow
    // control, never the longer Easy interval.
    HONEST_TIMING_CLOCK.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
    let registry = AccountRegistry::default()
        .with_clock(honest_timing_clock)
        .with_auth_config(AuthConfig::default().with_debug_links(true));
    let state = ApiState::new(registry);
    let app = router(state.clone());

    let slow = mature_cat_card_then_submit(&app, &state, "slow", Some("6500")).await;
    assert!(slow.contains(r#"<span class="me-verdict">Correct</span>"#));
    let slow_days = next_review_days(&slow);

    let fast = mature_cat_card_then_submit(&app, &state, "fast", Some("900")).await;
    assert!(fast.contains(r#"<span class="me-verdict">Correct</span>"#));
    let fast_days = next_review_days(&fast);
    assert!(
        fast_days > slow_days,
        "a genuinely fast mature correct answer must rate Easy and schedule \
         further out than a slow one: fast {fast_days} vs slow {slow_days} days"
    );

    for (label, dishonest) in [
        ("missing", None),
        ("blank", Some("")),
        ("malformed", Some("not-a-number")),
        ("negative", Some("-250")),
        ("zero", Some("0")),
        ("huge", Some("99999999999999999999")),
    ] {
        let graded = mature_cat_card_then_submit(&app, &state, label, dishonest).await;
        assert!(
            graded.contains(r#"<span class="me-verdict">Correct</span>"#),
            "dishonest timing {dishonest:?} must still grade the answer: {graded}"
        );
        assert_eq!(
            next_review_days(&graded),
            slow_days,
            "dishonest timing {dishonest:?} must grade conservatively (Good), never Easy"
        );
    }
}

#[test]
fn sanitize_response_time_maps_dishonest_shapes_to_the_conservative_ceiling() {
    use super::routes::{sanitize_response_time_ms, MAX_PLAUSIBLE_RESPONSE_TIME_MS};

    for dishonest in [
        None,
        Some(""),
        Some("   "),
        Some("not-a-number"),
        Some("-250"),
        Some("0"),
        Some("1.5"),
        Some("99999999999999999999"),
    ] {
        assert_eq!(
            sanitize_response_time_ms(dishonest),
            MAX_PLAUSIBLE_RESPONSE_TIME_MS,
            "dishonest shape {dishonest:?} must map to the conservative ceiling"
        );
    }

    assert_eq!(sanitize_response_time_ms(Some("900")), 900);
    assert_eq!(sanitize_response_time_ms(Some(" 6500 ")), 6_500);
    assert_eq!(
        sanitize_response_time_ms(Some("86400000")),
        MAX_PLAUSIBLE_RESPONSE_TIME_MS,
        "implausibly large timings clamp to the ceiling"
    );
}
