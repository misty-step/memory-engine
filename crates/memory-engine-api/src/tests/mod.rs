use std::{
    fs,
    path::Path as FsPath,
    sync::{mpsc, Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

use std::os::unix::fs::PermissionsExt;

use axum::{
    body::{to_bytes, Body},
    http::{header::SET_COOKIE, Request, StatusCode},
};
use serde_json::{json, Value};
use tower::ServiceExt;

use std::sync::atomic::{AtomicI64, Ordering};

use memory_engine_persistence::{GeneratedPromptValidationStatus, SourcePermission};
use memory_engine_study::DEFAULT_BETA_STUDY_NOW;

use super::{
    router, routes, AccountRegistry, ApiFailure, ApiState, AuthConfig, CreateSourceRequest,
    ReturnNotificationSchedulerConfig, AUTH_CHALLENGE_TTL_MS,
    RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS, WAITLIST_RATE_LIMIT_MAX_ATTEMPTS,
};
use memory_engine_api_state::{
    ContentFeedbackRequest, EnqueueOutcome, RETURN_NOTIFICATION_INTERVAL_MS,
};

/// A controllable test clock owned by exactly one test: each expansion carries
/// its own static, so no test can advance another test's time. Returns
/// `(&'static AtomicI64, fn() -> i64)` because the registry/storage clock seam
/// takes a plain `fn() -> i64`, which cannot capture per-test state — sharing
/// one module-level static across tests raced under parallel execution
/// (memory-engine-101: hosted CI runs 29450012146 / 29451670231 / 29451827236).
macro_rules! isolated_test_clock {
    ($init:expr) => {{
        static CLOCK: AtomicI64 = AtomicI64::new($init);
        fn isolated_now() -> i64 {
            CLOCK.load(Ordering::SeqCst)
        }
        (&CLOCK, isolated_now as fn() -> i64)
    }};
}

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
async fn manual_return_scheduler_fails_closed_on_absent_and_wrong_token_and_starts_on_correct_token(
) {
    let state = ApiState::new(AccountRegistry::default().with_auth_config(
        AuthConfig::default().with_scheduler_manual_token("operator-scheduler-token"),
    ));
    let app = router(state);
    let absent = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/scheduler/return-notifications")
                .body(Body::empty())
                .expect("manual scheduler request"),
        )
        .await
        .expect("manual scheduler response");
    assert_eq!(absent.status(), StatusCode::FORBIDDEN);

    let wrong = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/scheduler/return-notifications")
                .header("x-scheduler-token", "not-the-operator-token")
                .body(Body::empty())
                .expect("wrong-token scheduler request"),
        )
        .await
        .expect("wrong-token scheduler response");
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    let authorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/scheduler/return-notifications")
                .header("x-scheduler-token", "operator-scheduler-token")
                .body(Body::empty())
                .expect("authorized scheduler request"),
        )
        .await
        .expect("authorized scheduler response");
    assert_eq!(authorized.status(), StatusCode::OK);
    assert_eq!(response_json(authorized).await["examined"], json!(0));

    let health = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health response");
    let health = response_json(health).await;
    assert_eq!(
        health["returnNotificationScheduler"]["enabled"],
        json!(false)
    );
    assert_eq!(
        health["returnNotificationScheduler"]["failureCount"],
        json!(0)
    );
}

#[test]
fn scheduler_health_retains_last_success_when_a_report_has_failures() {
    let store_root = temp_store_root("scheduler-health-failure");
    fs::create_dir_all(&store_root).expect("health failure root");
    let state = ApiState::new(AccountRegistry::with_store_root(&store_root));
    let first = state
        .run_scheduled_return_notifications()
        .expect("empty scheduler run");
    assert_eq!(first.failed, 0);
    let last_success = state.scheduler_health().last_success_at_ms;

    let account_dir = store_root.join("account-malformed-study");
    fs::create_dir_all(&account_dir).expect("health failure account");
    fs::write(
        account_dir.join("return-notifications.json"),
        r#"{"email":"health@example.com","enabled":true,"lastSentAtMs":null,"unsubscribeNonce":"health-nonce"}"#,
    )
    .expect("health failure preference");
    fs::write(account_dir.join("study.json"), b"not-json").expect("health failure study");
    let second = state
        .run_scheduled_return_notifications()
        .expect("scheduler report with account failure");
    assert_eq!(second.failed, 1);
    assert_eq!(state.scheduler_health().last_success_at_ms, last_success);
    let _ = fs::remove_dir_all(store_root);
}

#[tokio::test]
async fn readyz_distinguishes_a_live_process_from_a_started_worker() {
    let response = router(ApiState::default())
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response_json(response).await;
    assert_eq!(body["status"], json!("not_ready"));
    assert_eq!(body["workerStarted"], json!(false));
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
    assert!(body.contains("Scry"));
    assert!(body.contains("Remember everything"));
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
async fn mobile_capture_enqueues_generation_then_requires_learner_decisions() {
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
    // and a queued activity-log row. Candidates remain pending for review.
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
    assert!(captured.contains("Generating your cards. They'll appear below as they're ready."));
    assert!(captured.contains(r#"<ul id="me-jobs""#));
    assert_not_contains_any(&captured, &["Add all to reviews", ">Keep</button>"]);

    // Drain the background job: real generation leaves accepted candidates
    // pending. Inspect the rendered candidates, then explicitly keep them.
    state.run_pending_jobs_blocking();
    let workspace = workspace_html(&app, &cookie).await;
    assert_activity_succeeded_html(&workspace, 2);
    for draft_id in html_values(&workspace, "draftId") {
        let kept = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/draft/keep",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("keep pending draft");
        assert_eq!(kept.status(), StatusCode::OK);
    }

    // Only explicit keeps drive the review flow.
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
async fn mobile_capture_and_edit_expose_permission_without_leaking_local_only_bytes() {
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    let captured = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/capture",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("capture", "private learner notes"),
                ("permission", "local-only"),
            ],
        ))
        .await
        .expect("local-only capture");
    assert_eq!(captured.status(), StatusCode::OK);
    let captured = response_text(captured).await;
    assert!(captured.contains("Local only · never sent to a model"));

    let edited = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/source/permission",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("sourceId", &source_id),
                ("permission", "local-only"),
            ],
        ))
        .await
        .expect("edit permission");
    assert_eq!(edited.status(), StatusCode::OK);
    assert!(response_text(edited)
        .await
        .contains("Source permission updated."));

    let workspace = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert!(workspace.contains("Due now"));
    assert!(workspace.contains("Start review"));

    let invalid = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/source/permission",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("sourceId", &source_id),
                ("permission", "invalid"),
            ],
        ))
        .await
        .expect("invalid permission");
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
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
    // auto-keep, cards scheduled immediately due. The helper returns the
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
async fn home_get_does_not_send_or_mutate_return_notification_state() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("home-read-only-return-notifications");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(AuthConfig::default().with_link_outbox(&outbox_path)),
    );
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    let saved = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/save-account",
            &cookie,
            &[("csrfToken", &csrf_token), ("email", "learner@example.com")],
        ))
        .await
        .expect("save account");
    assert_eq!(saved.status(), StatusCode::OK);
    let cookie = session_cookie(&saved);
    let saved = response_text(saved).await;
    let csrf_token = html_value(&saved, "csrfToken");

    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let enabled = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/return-notifications",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("enabled", "on"),
                ("reminderEmail", "learner@example.com"),
            ],
        ))
        .await
        .expect("enable return notifications");
    assert_eq!(enabled.status(), StatusCode::OK);
    let outbox_before = fs::read_to_string(&outbox_path).expect("outbox after explicit enable");
    let preference_before = read_return_notification_preference(&store_root);
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);

    let home = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("home request"),
        )
        .await
        .expect("home response");
    assert_eq!(home.status(), StatusCode::OK);
    assert!(!response_text(home).await.contains("Get started"));
    assert_eq!(
        fs::read_to_string(&outbox_path).expect("outbox after home GET"),
        outbox_before,
        "home GET must not send a return notification"
    );
    assert_eq!(
        read_return_notification_preference(&store_root),
        preference_before,
        "home GET must not mutate return notification persistence"
    );
}

#[test]
fn scheduled_return_notification_sends_live_due_count_without_request_traffic() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("scheduled-return-notification");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(AuthConfig::default().with_link_outbox(&outbox_path)),
    );
    let created = state
        .create_account("scheduled@example.com")
        .expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    let source = state
        .save_source(
            account.account_id(),
            account.session_token(),
            &CreateSourceRequest {
                title: "Scheduled reminder source".to_owned(),
                body: source_body(),
                permission: SourcePermission::ModelEligible,
            },
        )
        .expect("save source");
    let generated = state
        .generate_source(
            account.account_id(),
            account.session_token(),
            &source.source_id,
        )
        .expect("generate source");
    for draft in &generated.drafts {
        state
            .keep_draft(account.account_id(), account.session_token(), &draft.id)
            .expect("keep generated draft");
    }
    let view = state
        .study_view(account.account_id(), account.session_token())
        .expect("study view");
    assert!(view.due_count > 0);
    state
        .set_return_notification(&account, Some("scheduled@example.com"), true)
        .expect("enable reminders");
    assert!(state
        .maybe_send_due_count_notification(&account, view.due_count, true)
        .expect("confirmation"));
    let before_scheduler = fs::read_to_string(&outbox_path).expect("confirmation outbox");
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);

    let report = state
        .run_scheduled_return_notifications()
        .expect("scheduled run");

    assert_eq!(report.sent, 1);
    let messages = fs::read_to_string(&outbox_path)
        .expect("scheduled reminder outbox")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), before_scheduler.lines().count() + 1);
    assert!(messages
        .last()
        .is_some_and(|message| message.contains("scheduled@example.com")
            && message.contains(&format!("\t{}\t", view.due_count))));
}

#[test]
fn scheduled_return_notification_retries_with_durable_backoff() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("scheduled-return-retry-backoff");
    fs::create_dir_all(&store_root).expect("retry store root");
    let mailer_command = retry_provider_script(&store_root);
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::default()
                    .with_unsubscribe_secret("retry-backoff-secret")
                    .with_mailer_command(&mailer_command),
            ),
    );
    let created = state
        .create_account("backoff@example.com")
        .expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    state
        .set_return_notification(&account, Some("backoff@example.com"), true)
        .expect("enable reminders");
    assert!(state
        .maybe_send_due_count_notification(&account, 0, true)
        .is_err());
    let provider_log = store_root.join("retry-provider.tsv");
    assert_eq!(
        fs::read_to_string(&provider_log)
            .expect("failed send")
            .lines()
            .count(),
        1
    );
    let preference: Value = serde_json::from_str(&read_return_notification_preference(&store_root))
        .expect("retry preference");
    assert!(preference["nextRetryAtMs"]
        .as_i64()
        .is_some_and(|retry_at| { retry_at > test_clock.load(Ordering::SeqCst) }));

    let immediate = state
        .run_scheduled_return_notifications()
        .expect("immediate scheduled run");
    assert_eq!(immediate.sent, 0);
    assert_eq!(
        fs::read_to_string(&provider_log)
            .expect("backoff log")
            .lines()
            .count(),
        1
    );

    test_clock.fetch_add(60_001, Ordering::SeqCst);
    let recovered = state
        .run_scheduled_return_notifications()
        .expect("backoff recovery run");
    assert_eq!(recovered.sent, 1);
    assert_eq!(
        fs::read_to_string(&provider_log)
            .expect("recovery log")
            .lines()
            .count(),
        2
    );
}

#[test]
fn scheduled_return_notification_batch_quota_rotates_eligible_accounts() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("scheduled-return-quota");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(AuthConfig::default().with_link_outbox(&outbox_path)),
    );
    for email in ["quota-a@example.com", "quota-b@example.com"] {
        let created = state.create_account(email).expect("quota account");
        let account = state
            .create_browser_session(&created)
            .expect("quota session");
        let source = state
            .save_source(
                account.account_id(),
                account.session_token(),
                &CreateSourceRequest {
                    title: "Quota source".to_owned(),
                    body: source_body(),
                    permission: SourcePermission::ModelEligible,
                },
            )
            .expect("quota source");
        let generated = state
            .generate_source(
                account.account_id(),
                account.session_token(),
                &source.source_id,
            )
            .expect("quota generation");
        for draft in &generated.drafts {
            state
                .keep_draft(account.account_id(), account.session_token(), &draft.id)
                .expect("quota keep");
        }
        state
            .set_return_notification(&account, Some(email), true)
            .expect("quota enable");
        assert!(state
            .maybe_send_due_count_notification(&account, 0, true)
            .expect("quota confirmation"));
    }
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);

    let first = state
        .run_scheduled_return_notifications_with_config(ReturnNotificationSchedulerConfig {
            batch_size: 1,
        })
        .expect("first quota run");
    assert_eq!(first.examined, 1);
    assert_eq!(first.sent, 1);
    assert!(first.truncated);

    let second = state
        .run_scheduled_return_notifications_with_config(ReturnNotificationSchedulerConfig {
            batch_size: 1,
        })
        .expect("second quota run");
    assert_eq!(second.examined, 1);
    assert_eq!(
        second.sent, 1,
        "the first account must not starve the batch"
    );
    assert_eq!(
        fs::read_to_string(&outbox_path)
            .expect("quota outbox")
            .lines()
            .filter(|line| line.starts_with("due-count\t"))
            .count(),
        4
    );
}

#[test]
fn concurrent_scheduler_instances_share_one_durable_file_claim() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("concurrent-scheduled-return-notification");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let auth_config = || AuthConfig::default().with_link_outbox(&outbox_path);
    let first = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let created = first
        .create_account("concurrent-scheduled@example.com")
        .expect("account");
    let account = first
        .create_browser_session(&created)
        .expect("browser session");
    let source = first
        .save_source(
            account.account_id(),
            account.session_token(),
            &CreateSourceRequest {
                title: "Concurrent scheduler source".to_owned(),
                body: source_body(),
                permission: SourcePermission::ModelEligible,
            },
        )
        .expect("source");
    let generated = first
        .generate_source(
            account.account_id(),
            account.session_token(),
            &source.source_id,
        )
        .expect("generation");
    for draft in &generated.drafts {
        first
            .keep_draft(account.account_id(), account.session_token(), &draft.id)
            .expect("keep");
    }
    let view = first
        .study_view(account.account_id(), account.session_token())
        .expect("study view");
    first
        .set_return_notification(&account, Some("concurrent-scheduled@example.com"), true)
        .expect("enable");
    assert!(first
        .maybe_send_due_count_notification(&account, view.due_count, true)
        .expect("confirmation"));
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);

    let second = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let barrier = Arc::new(Barrier::new(2));
    let workers = [first, second]
        .into_iter()
        .map(|state| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                state
                    .run_scheduled_return_notifications()
                    .expect("scheduler run")
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("scheduler worker"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().map(|report| report.sent).sum::<usize>(), 1);
    assert_eq!(
        fs::read_to_string(&outbox_path)
            .expect("shared outbox")
            .lines()
            .filter(|line| line.starts_with("due-count\t"))
            .count(),
        2
    );
}

#[tokio::test]
async fn return_notification_enable_route_retries_the_same_provider_envelope_after_restart() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("return-notification-route-retry");
    fs::create_dir_all(&store_root).expect("retry store root");
    let mailer_command = retry_provider_script(&store_root);
    let auth_config = || {
        AuthConfig::default()
            .with_unsubscribe_secret("claim-secret")
            .with_mailer_command(&mailer_command)
    };
    let first_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let first_app = router(first_state);
    let (cookie, csrf_token, _) = start_app_session_for_csrf(&first_app).await;
    let saved = first_app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/save-account",
            &cookie,
            &[("csrfToken", &csrf_token), ("email", "retry@example.com")],
        ))
        .await
        .expect("save account");
    let cookie = session_cookie(&saved);
    let saved = response_text(saved).await;
    let csrf_token = html_value(&saved, "csrfToken");
    let first_attempt = post_return_notification_enable(&first_app, &cookie, &csrf_token).await;
    assert!(first_attempt.contains("return notification mailer command exited"));
    let first_payload =
        fs::read_to_string(store_root.join("retry-provider.tsv")).expect("failed payload");
    assert_eq!(first_payload.lines().count(), 1);
    let failed_preference: Value =
        serde_json::from_str(&read_return_notification_preference(&store_root))
            .expect("failed preference");
    assert!(failed_preference["pendingDeliveryKey"].is_string());
    assert!(failed_preference["pendingUnsubscribeExpiresAtMs"].is_number());
    assert!(failed_preference["claimId"].is_null());

    test_clock.fetch_add(123_456, Ordering::SeqCst);
    let recovery_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let recovery_app = router(recovery_state);
    let second_attempt = post_return_notification_enable(&recovery_app, &cookie, &csrf_token).await;
    assert!(second_attempt.contains("Due-count reminders are on"));
    let payloads = fs::read_to_string(store_root.join("retry-provider.tsv"))
        .expect("provider payloads")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        payloads[0], payloads[1],
        "route retry payload must be identical"
    );
    assert_eq!(payloads[0].split('\t').count(), 4);
    assert_eq!(
        payloads[0].split('\t').nth(3),
        payloads[1].split('\t').nth(3),
        "route retry must preserve the idempotency key"
    );
    let completed_preference: Value =
        serde_json::from_str(&read_return_notification_preference(&store_root))
            .expect("completed preference");
    for field in [
        "claimId",
        "pendingDeliveryKey",
        "pendingDueCount",
        "pendingUnsubscribeExpiresAtMs",
    ] {
        assert!(
            completed_preference[field].is_null(),
            "{field} must clear on success"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_return_sender_does_not_block_health_requests() {
    let store_root = temp_store_root("return-notification-blocked-sender");
    fs::create_dir_all(&store_root).expect("blocked sender root");
    let slow_command = slow_provider_script(&store_root);
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root).with_auth_config(
            AuthConfig::default()
                .with_unsubscribe_secret("blocked-sender-secret")
                .with_mailer_command(&slow_command),
        ),
    );
    let app = router(state);
    let (cookie, csrf_token, _) = start_app_session_for_csrf(&app).await;
    let saved = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/save-account",
            &cookie,
            &[("csrfToken", &csrf_token), ("email", "retry@example.com")],
        ))
        .await
        .expect("save blocked sender account");
    let cookie = session_cookie(&saved);
    let saved = response_text(saved).await;
    let csrf_token = html_value(&saved, "csrfToken");
    let request_app = app.clone();
    let request = tokio::spawn(async move {
        request_app
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/return-notifications",
                &cookie,
                &[
                    ("csrfToken", &csrf_token),
                    ("enabled", "on"),
                    ("reminderEmail", "retry@example.com"),
                ],
            ))
            .await
            .expect("blocked sender request")
    });

    let started = Instant::now();
    while !store_root.join("slow-provider.tsv").exists() {
        assert!(started.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let health_started = Instant::now();
    let health = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);
    assert!(
        health_started.elapsed() < Duration::from_millis(250),
        "health request waited on the blocked sender"
    );
    assert_eq!(request.await.expect("sender task").status(), StatusCode::OK);
    let _ = fs::remove_dir_all(store_root);
}

#[test]
fn return_notification_new_envelopes_at_one_clock_tick_have_distinct_keys() {
    let (_, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("return-notification-random-envelope");
    fs::create_dir_all(&store_root).expect("envelope store root");
    let mailer_command = retry_provider_script(&store_root);
    fs::write(store_root.join("retry-provider.failed"), "").expect("successful provider marker");
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::default()
                    .with_unsubscribe_secret("claim-secret")
                    .with_mailer_command(mailer_command),
            ),
    );
    let account = state.create_account("random@example.com").expect("account");
    let account = state.create_browser_session(&account).expect("session");
    state
        .set_return_notification(&account, Some("random@example.com"), true)
        .expect("enable");
    assert!(state
        .maybe_send_due_count_notification(&account, 1, true)
        .expect("first envelope"));
    state
        .set_return_notification(&account, None, false)
        .expect("disable");
    state
        .set_return_notification(&account, Some("random@example.com"), true)
        .expect("re-enable");
    assert!(state
        .maybe_send_due_count_notification(&account, 1, true)
        .expect("second envelope"));
    let payloads = fs::read_to_string(store_root.join("retry-provider.tsv"))
        .expect("provider payloads")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 2);
    assert_ne!(
        payloads[0].split('\t').nth(3),
        payloads[1].split('\t').nth(3),
        "new envelopes at one clock tick must not collide"
    );
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
                (
                    "performanceTraceId",
                    "trace_0123456789abcdef0123456789abcdef",
                ),
            ],
        ))
        .await
        .expect("submit lowercase answer");
    assert_eq!(graded.status(), StatusCode::OK);
    let request_id = graded
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("submit request id")
        .to_owned();
    assert_eq!(request_id.len(), 36);
    assert!(request_id.starts_with("req_"));
    assert!(request_id[4..]
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    let server_timing = graded
        .headers()
        .get("server-timing")
        .and_then(|value| value.to_str().ok())
        .expect("submit server timing");
    assert!(server_timing.contains(&format!(r#"request;desc="{request_id}""#)));
    assert!(server_timing.contains(r#"handoff;desc="trace_0123456789abcdef0123456789abcdef""#));
    assert!(server_timing.contains("total;dur="));
    assert!(server_timing.contains("render;dur="));
    assert!(!server_timing.contains("pgconnect"));
    assert!(!server_timing.contains("pgop"));
    assert!(!server_timing.contains("pgstmt"));
    assert!(!server_timing.contains("alfa"));
    assert!(!server_timing.contains(&review_unit_id));
    assert!(!server_timing.contains("review-mcq-case"));
    let graded = response_text(graded).await;
    assert!(graded.contains(&format!(
        r#"<meta name="memory-engine-submit-request" content="{request_id}">"#
    )));
    assert!(graded.contains(
        r#"<meta name="memory-engine-submit-handoff" content="trace_0123456789abcdef0123456789abcdef">"#
    ));
    assert!(
        graded.contains("me-verdict") && graded.contains(">Correct<"),
        "lowercase 'alfa' must grade correct against stored 'ALFA': {graded}"
    );
}

async fn assert_submit_recovery_document(
    app: &axum::Router,
    cookie: &str,
    response: axum::response::Response,
) {
    let body = response_text(response).await;
    assert!(
        body.contains(r#"<script src="/static/app.js" defer></script>"#),
        "submit recovery must load the handoff consumer"
    );
    assert!(body.contains(r#"href="/""#));
    assert!(!body.contains("me-recovery-email"));

    let recovered = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("submit recovery request"),
        )
        .await
        .expect("submit recovery response");
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered = response_text(recovered).await;
    assert!(recovered.contains("csrfToken"));
    assert!(!recovered.contains("me-recovery-email"));
}

async fn assert_malformed_submit_recovery(app: &axum::Router, cookie: &str, csrf: &str) {
    let response = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            cookie,
            &[("csrfToken", csrf)],
        ))
        .await
        .expect("malformed submit");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.headers().contains_key("x-request-id"));
    let timing = response
        .headers()
        .get("server-timing")
        .and_then(|value| value.to_str().ok())
        .expect("malformed submit timing");
    assert!(timing.contains("total;dur="));
    assert!(!timing.contains("handoff"));
    assert_submit_recovery_document(app, cookie, response).await;
}

#[tokio::test]
async fn browser_submit_receipts_are_bounded_and_other_routes_stay_uninstrumented() {
    let state = ApiState::default();
    let app = router(state);
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
    let csrf = html_value(&response_text(started).await, "csrfToken");

    let receipt = app
        .clone()
        .oneshot(json_request_with_cookie(
            "POST",
            "/app/performance/submit",
            &cookie,
            &json!({
                "schema": "memory_engine.browser_submit.v1",
                "csrfToken": csrf,
                "requestId": "req_0123456789abcdef0123456789abcdef",
                "traceId": "trace_0123456789abcdef0123456789abcdef",
                "tapToAckMs": 5,
                "requestToResponseMs": 20,
                "transferMs": 4,
                "navigationMs": 11,
                "gradedVisibleMs": 35,
                "viewport": "mobile"
            }),
        ))
        .await
        .expect("browser submit receipt");
    assert_eq!(receipt.status(), StatusCode::NO_CONTENT);
    assert!(!receipt.headers().contains_key("server-timing"));
    assert!(!receipt.headers().contains_key("x-request-id"));

    let invalid = app
        .clone()
        .oneshot(json_request_with_cookie(
            "POST",
            "/app/performance/submit",
            &cookie,
            &json!({
                "schema": "memory_engine.browser_submit.v1",
                "csrfToken": csrf,
                "requestId": "req_0123456789abcdef0123456789abcdef",
                "traceId": "trace_0123456789abcdef0123456789abcdef",
                "tapToAckMs": 5,
                "requestToResponseMs": 20,
                "transferMs": 4,
                "navigationMs": 11,
                "gradedVisibleMs": 50,
                "viewport": "mobile"
            }),
        ))
        .await
        .expect("invalid browser submit receipt");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let wrong_shape = app
        .clone()
        .oneshot(json_request_with_cookie(
            "POST",
            "/app/performance/submit",
            &cookie,
            &json!({"csrfToken": csrf, "schema": 42}),
        ))
        .await
        .expect("wrong-shaped browser submit receipt");
    assert_eq!(wrong_shape.status(), StatusCode::BAD_REQUEST);

    assert_malformed_submit_recovery(&app, &cookie, &csrf).await;

    for request in [
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .expect("health request"),
        Request::builder()
            .uri("/static/ledger.css")
            .body(Body::empty())
            .expect("static request"),
        Request::builder()
            .uri("/app/jobs/events")
            .header("cookie", &cookie)
            .body(Body::empty())
            .expect("event stream request"),
    ] {
        let response = app.clone().oneshot(request).await.expect("excluded route");
        assert!(!response.headers().contains_key("server-timing"));
        assert!(!response.headers().contains_key("x-request-id"));
    }
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
    assert!(graded.contains("This item:"));
    assert!(graded.contains("Keep"));
    assert!(graded.contains("Drop"));
    assert!(graded.contains("me-content-feedback-rationale"));
    assert_not_contains_any(&graded, &["Answer feedback", "Concept health"]);
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
async fn review_edit_form_preserves_identity_queue_and_uses_edited_answer() {
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

    let original_prompt = "Spell CAT over the phone";
    let page = advance_to_prompt(&app, &cookie, &csrf_token, original_prompt).await;
    assert!(
        page.contains(">Edit</button>"),
        "review must expose Edit: {page}"
    );
    let review_unit_id = html_value(&page, "reviewUnitId");
    let due_before = page
        .split_once(" due</span>")
        .and_then(|(prefix, _)| prefix.rsplit_once('>'))
        .map(|(_, due)| due.to_owned())
        .expect("due count");

    let edit = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/edit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
            ],
        ))
        .await
        .expect("open edit form");
    assert_eq!(edit.status(), StatusCode::OK);
    let edit = response_text(edit).await;
    assert!(edit.contains(r#"<form class="me-edit-form" action="/app/edit/save" method="post">"#));
    assert!(edit.contains(r#"name="prompt""#));
    assert!(edit.contains(r#"name="expectedAnswer""#));
    assert!(edit.contains(original_prompt));

    assert_blank_review_edit_rejected(&app, &cookie, &csrf_token, &review_unit_id).await;

    let edited_prompt = "Spell CAT with the revised NATO wording";
    let edited_answer = "REVISED CHARLIE ALFA TANGO";
    let padded_prompt = format!("  {edited_prompt}  ");
    let padded_answer = format!("  {edited_answer}  ");
    let saved = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/edit/save",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("prompt", &padded_prompt),
                ("expectedAnswer", &padded_answer),
            ],
        ))
        .await
        .expect("save edit");
    assert_eq!(saved.status(), StatusCode::OK);
    let saved = response_text(saved).await;
    assert!(
        saved.contains(edited_prompt),
        "edited prompt must render: {saved}"
    );
    assert_eq!(html_value(&saved, "reviewUnitId"), review_unit_id);
    assert!(saved.contains(&format!(">{due_before} due</span>")));

    let submitted = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", edited_answer),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "review-edit-uses-new-answer"),
            ],
        ))
        .await
        .expect("submit edited card");
    assert_eq!(submitted.status(), StatusCode::OK);
    let submitted = response_text(submitted).await;
    assert!(
        submitted.contains(r#"<span class="me-verdict">Correct</span>"#),
        "grading must use edited answer: {submitted}"
    );
    assert!(submitted.contains(edited_prompt));
}

async fn assert_blank_review_edit_rejected(
    app: &axum::Router,
    cookie: &str,
    csrf_token: &str,
    review_unit_id: &str,
) {
    let blank = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/edit/save",
            cookie,
            &[
                ("csrfToken", csrf_token),
                ("reviewUnitId", review_unit_id),
                ("prompt", "   "),
                ("expectedAnswer", "still an answer"),
            ],
        ))
        .await
        .expect("reject blank edit");
    assert_eq!(blank.status(), StatusCode::BAD_REQUEST);
    assert!(response_text(blank)
        .await
        .contains("Review unit prompt must not be blank"));
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
    // accepted cards are auto-keepd and scheduled — no keep gate, no
    // per-draft keep. The activity log shows the finished job.
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);

    // Open the review queue: both scheduled cards are due. Take whichever
    // card surfaces first (auto-keep fixes no order), reveal it — every
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
    // Generation auto-keeps and schedules every accepted card; no manual
    // per-draft keep. Drive the queue to the NATO-A quiz card and answer
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
        .clone()
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

    // Ledger graded screen, wrong answer: the verdict reads "Try again", the
    // correct option is still revealed (marked) so the learner sees it, the
    // card's dossier renders (stage, last seen, success record — DESIGN.md
    // puts meta post-grade only), and a quiet line says when it returns. Raw
    // internals still never leak.
    assert!(submitted.contains(r#"<span class="me-verdict">Try again</span>"#));
    assert!(submitted.contains("ALFA"));
    assert!(submitted.contains(r#"<li class="me-graded-choice me-graded-choice-correct">"#));
    assert!(submitted.contains("you'll see this again"));
    assert!(submitted.contains(r#"class="me-meta-ledger""#));
    assert!(submitted.contains("Last seen"));
    assert_not_contains_any(
        &submitted,
        &[
            "Expected answer",
            "Answer feedback",
            "Concept health",
            "Wrong(",
            "reviewState",
            "scheduleChange",
            "Generated material",
            "validation",
        ],
    );

    let feedback = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("verdict", "dropped"),
                ("rationale", "The distractor makes the card misleading."),
                ("idempotencyKey", "content-feedback-mobile-nato-a"),
            ],
        ))
        .await
        .expect("record content feedback");
    assert_eq!(feedback.status(), StatusCode::OK);
    let feedback = response_text(feedback).await;
    assert!(feedback.contains("Saved. This card will help improve future generation."));
    assert!(feedback.contains("Review complete"));
    assert!(feedback.contains(r#"href="/">Back to workspace</a>"#));
    assert!(!feedback.contains("What do you want to remember?"));

    let replay = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("verdict", "dropped"),
                ("rationale", "The distractor makes the card misleading."),
                ("idempotencyKey", "content-feedback-mobile-nato-a"),
            ],
        ))
        .await
        .expect("replay content feedback");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = response_text(replay).await;
    assert!(replay.contains("Review complete"));
    assert!(!replay.contains("What do you want to remember?"));
}

#[tokio::test]
async fn app_content_feedback_advances_to_the_next_due_review() {
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let first_prompt =
        next_review_html(&app, &cookie, &csrf_token, "start first feedback review").await;
    let first_review_unit_id = html_value(&first_prompt, "reviewUnitId");
    let first_review_key = html_value(&first_prompt, "idempotencyKey");
    submit_review_ok(
        &app,
        &cookie,
        &csrf_token,
        &first_review_unit_id,
        "deliberately wrong",
        &first_review_key,
    )
    .await;

    let invalid = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &first_review_unit_id),
                ("verdict", "kept"),
                ("idempotencyKey", ""),
                ("rationale", "keep this retry note"),
            ],
        ))
        .await
        .expect("reject invalid content feedback");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let invalid = response_text(invalid).await;
    assert!(invalid.contains("Idempotency key must not be blank."));
    assert!(invalid.contains(r#"action="/app/content-feedback""#));
    assert!(invalid.contains("keep this retry note"));
    assert_eq!(html_value(&invalid, "reviewUnitId"), first_review_unit_id);
    assert_eq!(html_value(&invalid, "verdict"), "kept");
    let retry_feedback_key = html_value(&invalid, "idempotencyKey");
    assert!(!retry_feedback_key.is_empty());
    assert!(!invalid.contains(r#"action="/app/submit""#));
    assert!(!invalid.contains("What do you want to remember?"));

    let continued = submit_content_feedback_ok(
        &app,
        &cookie,
        &[
            ("csrfToken", &csrf_token),
            ("reviewUnitId", &first_review_unit_id),
            ("verdict", "kept"),
            ("idempotencyKey", &retry_feedback_key),
        ],
    )
    .await;

    assert!(continued.contains("Saved. This card will help improve future generation."));
    assert!(continued.contains(r#"action="/app/submit""#));
    assert!(!continued.contains("What do you want to remember?"));
    assert_ne!(
        html_value(&continued, "reviewUnitId"),
        first_review_unit_id,
        "feedback must advance into the next due review"
    );
}

#[tokio::test]
async fn app_content_feedback_persistence_recovery_fields_can_be_submitted() {
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let first_prompt = next_review_html(
        &app,
        &cookie,
        &csrf_token,
        "start persistence recovery review",
    )
    .await;
    let first_review_unit_id = html_value(&first_prompt, "reviewUnitId");
    let first_review_key = html_value(&first_prompt, "idempotencyKey");
    let graded = submit_review_ok(
        &app,
        &cookie,
        &csrf_token,
        &first_review_unit_id,
        "deliberately wrong",
        &first_review_key,
    )
    .await;
    let feedback_key = content_feedback_value(&graded, "idempotencyKey");

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        cookie.parse().expect("cookie header"),
    );
    let account = state
        .require_browser_session(&headers, &csrf_token)
        .expect("browser session");
    let unavailable = routes::render_content_feedback_persistence_failure(
        &state,
        &account,
        &first_review_unit_id,
        &ContentFeedbackRequest {
            verdict: memory_engine_service::ContentFeedbackVerdict::Kept,
            rationale: Some("keep this storage retry note".to_owned()),
            idempotency_key: feedback_key,
            supersedes_id: None,
        },
        ApiFailure::service_unavailable("Feedback storage is temporarily unavailable.".to_owned()),
    );
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    let unavailable = response_text(unavailable).await;
    assert!(unavailable.contains("Feedback storage is temporarily unavailable."));
    assert!(unavailable.contains(r#"action="/app/content-feedback""#));
    assert!(unavailable.contains("keep this storage retry note"));
    assert!(!unavailable.contains(r#"action="/app/submit""#));
    assert!(!unavailable.contains("What do you want to remember?"));

    let recovery_review_unit_id = html_value(&unavailable, "reviewUnitId");
    let recovery_verdict = html_value(&unavailable, "verdict");
    let recovery_key = html_value(&unavailable, "idempotencyKey");
    let continued = submit_content_feedback_ok(
        &app,
        &cookie,
        &[
            ("csrfToken", &csrf_token),
            ("reviewUnitId", &recovery_review_unit_id),
            ("verdict", &recovery_verdict),
            ("rationale", "keep this storage retry note"),
            ("idempotencyKey", &recovery_key),
        ],
    )
    .await;

    assert!(continued.contains("Saved. This card will help improve future generation."));
    assert!(continued.contains(r#"action="/app/submit""#));
    assert_ne!(
        html_value(&continued, "reviewUnitId"),
        first_review_unit_id,
        "a submitted persistence-recovery form must advance to the next due review"
    );
}

#[test]
fn app_content_feedback_head_refresh_failure_preserves_retry_revision() {
    let mut status = StatusCode::CONFLICT;
    let mut message = "Feedback conflicts with the latest saved revision.".to_owned();
    let (idempotency_key, supersedes_id) = routes::resolve_content_feedback_recovery_revision(
        true,
        "review-1",
        "feedback-original",
        Some("feedback-parent"),
        Some(Err(ApiFailure::service_unavailable(
            "feedback head unavailable".to_owned(),
        ))),
        &mut status,
        &mut message,
    );

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message,
        "Feedback was not saved, and the latest revision could not be loaded. Retry when storage is available."
    );
    assert_eq!(idempotency_key, "feedback-original");
    assert_eq!(supersedes_id.as_deref(), Some("feedback-parent"));
}

#[tokio::test]
async fn app_content_feedback_revision_carries_current_head_and_refreshes_idempotency_key() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default().with_clock(test_now);
    let state = ApiState::new(registry);
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let first_prompt =
        advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let first_review_unit_id = html_value(&first_prompt, "reviewUnitId");
    let first_review_key = html_value(&first_prompt, "idempotencyKey");
    let first_graded = submit_review_ok(
        &app,
        &cookie,
        &csrf_token,
        &first_review_unit_id,
        "not the answer",
        &first_review_key,
    )
    .await;
    let first_feedback_key = content_feedback_value(&first_graded, "idempotencyKey");

    submit_content_feedback_ok(
        &app,
        &cookie,
        &[
            ("csrfToken", &csrf_token),
            ("reviewUnitId", &first_review_unit_id),
            ("verdict", "dropped"),
            ("rationale", "The card is misleading on the first pass."),
            ("idempotencyKey", &first_feedback_key),
        ],
    )
    .await;

    test_clock.fetch_add(86_400_000, Ordering::SeqCst);

    let second_prompt =
        advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let second_review_unit_id = html_value(&second_prompt, "reviewUnitId");
    let second_review_key = html_value(&second_prompt, "idempotencyKey");
    assert_eq!(second_review_unit_id, first_review_unit_id);

    let second_graded = submit_review_ok(
        &app,
        &cookie,
        &csrf_token,
        &second_review_unit_id,
        "CHARLIE ALFA TANGO",
        &second_review_key,
    )
    .await;
    let second_feedback_key = content_feedback_value(&second_graded, "idempotencyKey");
    let second_head = content_feedback_value(&second_graded, "supersedesId");
    assert_ne!(
        first_feedback_key, second_feedback_key,
        "a revised feedback render must not reuse the first idempotency key"
    );

    let stale_fields = [
        ("csrfToken", csrf_token.as_str()),
        ("reviewUnitId", second_review_unit_id.as_str()),
        ("verdict", "kept"),
        ("rationale", "The card is useful after all."),
        ("idempotencyKey", first_feedback_key.as_str()),
    ];
    let conflicting_replay = submit_content_feedback_conflict(&app, &cookie, &stale_fields).await;
    assert!(conflicting_replay.contains(r#"action="/app/content-feedback""#));
    assert!(!conflicting_replay.contains(r#"action="/app/next""#));
    assert!(conflicting_replay.contains("The card is useful after all."));
    assert!(conflicting_replay.contains("Try that feedback again."));
    assert!(
        conflicting_replay.contains(r#"name="supersedesId""#),
        "{conflicting_replay}"
    );
    let retry_feedback_key = html_value(&conflicting_replay, "idempotencyKey");
    let retry_head = html_value(&conflicting_replay, "supersedesId");
    let repeated_conflict = submit_content_feedback_conflict(&app, &cookie, &stale_fields).await;
    let repeated_retry_key = html_value(&repeated_conflict, "idempotencyKey");
    assert_ne!(retry_feedback_key, first_feedback_key);
    assert_ne!(retry_feedback_key, repeated_retry_key);
    assert_eq!(retry_head, second_head);

    let revised_page = submit_content_feedback_ok(
        &app,
        &cookie,
        &[
            ("csrfToken", &csrf_token),
            ("reviewUnitId", &second_review_unit_id),
            ("verdict", "kept"),
            ("rationale", "The card is useful after all."),
            ("idempotencyKey", &retry_feedback_key),
            ("supersedesId", &retry_head),
        ],
    )
    .await;
    assert!(revised_page.contains("Saved. This card will help improve future generation."));
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
    // Generation auto-keeps and schedules both cards (same concept). No
    // manual per-draft keep — the activity log confirms two cards landed.
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

    // Ledger puts the card's dossier on the graded screen, including its
    // concept line — the shared concept shows here AND rolls up on the
    // workspace.
    assert!(submitted.contains(r#"<span class="me-verdict">Try again</span>"#));
    assert!(submitted.contains(r#"class="me-meta-ledger""#));
    assert!(submitted.contains("nato letter a"));

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
    // Generation auto-keeps and schedules both cards — no manual keep.
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

    let analytics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/analytics")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("analytics request"),
        )
        .await
        .expect("analytics response");
    assert_eq!(analytics.status(), StatusCode::OK);
    let analytics = response_text(analytics).await;
    assert!(analytics.starts_with("<!doctype html>"));
    assert!(analytics
        .contains(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#));
    assert!(analytics.contains(r#"<link rel="stylesheet" href="/static/ledger.css">"#));
    assert!(analytics.contains(r#"<script src="/static/app.js" defer></script>"#));
    assert!(analytics.contains(r#"<h1 class="me-display me-analytics-title">Concept health</h1>"#));
    assert!(analytics.contains("Health"));
    assert!(analytics.contains("Health · at risk first"));
    let weak = analytics
        .find("<strong>nato letter a</strong>")
        .expect("weak concept in analytics");
    let strong = analytics
        .find("<strong>nato cat composition</strong>")
        .expect("strong concept in analytics");
    assert!(weak < strong, "{analytics}");
    assert!(analytics.contains(r#"name="filter"#));
    assert!(analytics.contains(r#"value="at-risk">At risk</option>"#));

    let filtered = app
        .oneshot(
            Request::builder()
                .uri("/app/analytics?filter=at-risk")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("filtered analytics request"),
        )
        .await
        .expect("filtered analytics response");
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered = response_text(filtered).await;
    assert!(filtered.contains("nato letter a"));
    assert!(!filtered.contains("nato cat composition"));
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
    // Generation auto-keeps and schedules both cards. Drive the queue to
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
    let bridge_draft_ids = html_values(&bridged, "draftId");
    assert_eq!(
        bridge_draft_ids.len(),
        2,
        "bridge candidates remain pending"
    );
    for draft_id in bridge_draft_ids {
        let kept = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/draft/keep",
                &cookie,
                &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("keep bridge draft");
        assert_eq!(kept.status(), StatusCode::OK);
    }
    let opened_bridge = next_review_html(&app, &cookie, &csrf_token, "bridge").await;
    let bridge_id = html_value(&opened_bridge, "reviewUnitId");
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

/// Generation auto-keeps and schedules cards (no keep gate); open the
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
        "/app/snooze-concept",
        &[("reviewUnitId", review_unit_id)],
        "concept snooze without csrf",
    )
    .await;
    assert_forbidden_form(
        app,
        cookie,
        "/app/snooze-concept",
        &[
            ("csrfToken", "csrf-invalid-for-matrix"),
            ("reviewUnitId", review_unit_id),
        ],
        "concept snooze with invalid csrf",
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
        "/app/edit",
        &[("reviewUnitId", review_unit_id)],
        "edit without csrf",
    )
    .await;
    assert_forbidden_form(
        app,
        cookie,
        "/app/edit/save",
        &[
            ("reviewUnitId", review_unit_id),
            ("prompt", "edited"),
            ("expectedAnswer", "ALFA"),
        ],
        "edit save without csrf",
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
/// structured-block generation), explicitly keep each pending candidate, and
/// return the reloaded workspace with a succeeded activity-log row. This helper
/// keeps legacy review-flow tests focused on review behavior; production never
/// auto-keeps generated candidates.
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
    // The handler returns immediately with the queued job, before any draft
    // exists. Drain the queue, then explicitly keep each accepted draft through
    // the learner action route before reloading the workspace.
    assert!(generated.contains("Generating. Watch the activity log."));
    state.run_pending_jobs_blocking();
    let pending = workspace_html(app, cookie).await;
    for draft_id in html_values(&pending, "draftId") {
        let response = app
            .clone()
            .oneshot(form_request_with_cookie(
                "POST",
                "/app/draft/keep",
                cookie,
                &[("csrfToken", csrf_token), ("draftId", &draft_id)],
            ))
            .await
            .expect("keep generated draft");
        assert_eq!(response.status(), StatusCode::OK);
    }
    workspace_html(app, cookie).await
}

/// Drive `/app/next` until the current review item's prompt contains
/// `needle`, returning that page. Explicit keeps schedule accepted cards
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

async fn assert_html_concept_key_is_hidden(concept_key: &Value) {
    let root = temp_store_root("concept_key_html");
    let state = ApiState::new(AccountRegistry::with_store_root(root.clone()));
    let app = router(state.clone());
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &shared_concept_body())],
        ))
        .await
        .expect("start HTML session");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    let study_path = fs::read_dir(&root)
        .expect("read HTML store root")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("study.json"))
        .find(|path| path.exists())
        .expect("HTML study snapshot");
    let mut snapshot: Value = serde_json::from_str(
        &fs::read_to_string(&study_path).expect("read HTML concept-key snapshot"),
    )
    .expect("decode HTML concept-key snapshot");
    for unit in snapshot["reviewUnits"]
        .as_array_mut()
        .expect("HTML review units")
    {
        unit["queue"]["conceptKey"] = concept_key.clone();
    }
    fs::write(
        &study_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&snapshot).expect("encode HTML concept-key snapshot")
        ),
    )
    .expect("write HTML concept-key snapshot");

    let page = next_review_html(&app, &cookie, &csrf_token, "concept key").await;
    assert!(!page.contains("Hide every card for this concept until tomorrow."));
    let review_unit_id = html_value(&page, "reviewUnitId");
    let rejected = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/snooze-concept",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
            ],
        ))
        .await
        .expect("concept-key HTML rejection");
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
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
    if uri == "/app/submit" {
        assert_submit_recovery_document(app, cookie, response).await;
    }
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
    assert_no_store_and_no_referrer(&verified);
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
    assert_no_store_and_no_referrer(&first);

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
    assert_no_store_and_no_referrer(&replay);
}

#[tokio::test]
async fn token_bearing_login_request_response_is_not_cached() {
    let app = router(ApiState::new(
        AccountRegistry::default().with_auth_config(AuthConfig::default().with_debug_links(true)),
    ));
    let requested = app
        .oneshot(form_request(
            "POST",
            "/app/account",
            &[("email", "learner@example.com")],
        ))
        .await
        .expect("request magic link");
    assert_eq!(requested.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&requested);
}

#[tokio::test]
async fn expired_magic_link_renders_direct_recovery_instead_of_json() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let app = router(ApiState::new(
        AccountRegistry::default()
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["learner@example.com".to_owned()]).with_debug_links(true),
            ),
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
    test_clock.fetch_add(AUTH_CHALLENGE_TTL_MS + 1, Ordering::SeqCst);

    let expired = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(verify_path)
                .body(Body::empty())
                .expect("expired verify request"),
        )
        .await
        .expect("expired verify response");
    assert_eq!(expired.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&expired);
    assert_eq!(
        expired
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let body = response_text(expired).await;
    assert!(body.contains("Sign-in link expired"));
    assert!(body.contains(r#"<form action="/app/account" method="post">"#));
    assert!(body.contains("Request a new link"));
    assert!(!body.contains(r#"{"error""#));
}

#[tokio::test]
async fn expired_browser_session_renders_direct_recovery_instead_of_json() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let app = router(ApiState::new(
        AccountRegistry::default().with_clock(test_now),
    ));
    let started = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", "session expiry proof")],
        ))
        .await
        .expect("start");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    test_clock.fetch_add(super::app_session_max_age_ms() + 1, Ordering::SeqCst);

    let expired = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/next",
            &cookie,
            &[("csrfToken", &csrf_token)],
        ))
        .await
        .expect("expired session response");
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
    assert_no_store_and_no_referrer(&expired);
    assert_eq!(
        expired
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let body = response_text(expired).await;
    assert!(body.contains("Your session expired"));
    assert!(body.contains(r#"<form action="/app/account" method="post">"#));
    assert!(!body.contains(r#"{"error""#));

    let feedback = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", "expired-session-review"),
                ("verdict", "kept"),
                ("idempotencyKey", "expired-session-feedback"),
            ],
        ))
        .await
        .expect("expired feedback session response");
    assert_eq!(feedback.status(), StatusCode::UNAUTHORIZED);
    assert_no_store_and_no_referrer(&feedback);
    assert_eq!(
        feedback
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let body = response_text(feedback).await;
    assert!(body.contains("Your session expired"));
    assert!(!body.contains(r#"{"error""#));
}

#[tokio::test]
async fn expired_browser_session_renders_concept_snooze_recovery_instead_of_json() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let state = ApiState::new(AccountRegistry::default().with_clock(test_now));
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    let page = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    let review_unit_id = html_value(&page, "reviewUnitId");
    test_clock.fetch_add(super::app_session_max_age_ms() + 1, Ordering::SeqCst);

    let expired = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/snooze-concept",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
            ],
        ))
        .await
        .expect("expired concept snooze session response");
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
    assert_no_store_and_no_referrer(&expired);
    assert_eq!(
        expired
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let body = response_text(expired).await;
    assert!(body.contains("Your session expired"));
    assert!(body.contains(r#"<form action="/app/account" method="post">"#));
    assert!(!body.contains(r#"{"error""#));
}

#[tokio::test]
async fn installability_assets_are_valid_and_linked_from_the_shell() {
    let app = router(ApiState::default());
    let home = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("home request"),
        )
        .await
        .expect("home response");
    let home = response_text(home).await;
    assert!(home.contains(r#"rel="manifest" href="/manifest.webmanifest""#));
    assert!(home.contains(r#"rel="icon" href="/favicon.png""#));
    assert!(home.contains(r#"rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180""#));
    assert!(home.contains(r#"name="theme-color""#));

    let manifest = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/manifest.webmanifest")
                .body(Body::empty())
                .expect("manifest request"),
        )
        .await
        .expect("manifest response");
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest = response_json(manifest).await;
    assert_eq!(manifest["name"], json!("Scry"));
    assert_eq!(manifest["display"], json!("standalone"));
    assert_eq!(manifest["start_url"], json!("/"));
    assert_eq!(manifest["icons"][0]["src"], json!("/icon-192.png"));
    assert_eq!(manifest["icons"][0]["sizes"], json!("192x192"));
    assert_eq!(manifest["icons"][0]["type"], json!("image/png"));
    assert_eq!(manifest["icons"][1]["src"], json!("/icon-512.png"));
    assert_eq!(manifest["icons"][1]["sizes"], json!("512x512"));
    assert_eq!(manifest["icons"][1]["type"], json!("image/png"));

    for (path, width, height) in [
        ("/favicon.png", 192_u32, 192_u32),
        ("/icon-192.png", 192, 192),
        ("/icon-512.png", 512, 512),
        ("/apple-touch-icon.png", 180, 180),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("icon request"),
            )
            .await
            .expect("icon response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("image/png")
        );
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("icon bytes");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().expect("width")),
            width
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().expect("height")),
            height
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn due_count_return_channel_is_opt_in_and_disable_is_sticky() {
    let store_root = temp_store_root("return-notifications");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root).with_auth_config(
            AuthConfig::allow_emails(["learner@example.com".to_owned()])
                .with_link_outbox(&outbox_path),
        ),
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
    let verify_path = fs::read_to_string(&outbox_path)
        .expect("auth outbox")
        .lines()
        .next()
        .and_then(|line| line.split('\t').nth(1))
        .map(str::to_owned)
        .expect("magic link");
    drop(requested);
    let verified = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&verify_path)
                .body(Body::empty())
                .expect("verify request"),
        )
        .await
        .expect("verify");
    let cookie = session_cookie(&verified);
    let verified = response_text(verified).await;
    let csrf_token = html_value(&verified, "csrfToken");

    let enabled = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/return-notifications",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("enabled", "on"),
                ("reminderEmail", "learner@example.com"),
            ],
        ))
        .await
        .expect("enable return channel");
    assert_eq!(enabled.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&enabled);
    let enabled = response_text(enabled).await;
    assert!(enabled.contains("Due-count reminders are on"));
    let after_enable = fs::read_to_string(&outbox_path).expect("outbox after enable");
    assert!(after_enable.contains("due-count\tlearner@example.com\t"));

    let settings = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/return-notifications")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("settings request"),
        )
        .await
        .expect("settings response");
    assert_eq!(settings.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&settings);

    let unsubscribe_link = after_enable
        .lines()
        .find(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .expect("signed unsubscribe link")
        .to_owned();
    let confirmation = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&unsubscribe_link)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("unsubscribe request"),
        )
        .await
        .expect("unsubscribe response");
    assert_eq!(confirmation.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&confirmation);
    assert!(response_text(confirmation)
        .await
        .contains("Turn off due-count reminders"));
    let preference_after_get = fs::read_dir(&store_root)
        .expect("store root")
        .flatten()
        .find_map(|entry| fs::read_to_string(entry.path().join("return-notifications.json")).ok())
        .expect("preference after GET");
    assert!(preference_after_get.contains("\"enabled\":true"));

    let disabled = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/return-notifications",
            &[(
                "unsubscribeToken",
                unsubscribe_link.split("token=").nth(1).expect("token"),
            )],
        ))
        .await
        .expect("disable return channel");
    assert_eq!(disabled.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&disabled);
    let disabled_body = response_text(disabled).await;
    assert!(
        disabled_body.contains("Reminders are off"),
        "unexpected token unsubscribe response: {disabled_body}"
    );
    let after_disable = fs::read_to_string(&outbox_path).expect("outbox after disable");
    assert_eq!(after_disable, after_enable);

    let re_enabled = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/return-notifications",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("enabled", "on"),
                ("reminderEmail", "learner@example.com"),
            ],
        ))
        .await
        .expect("re-enable return channel");
    assert_eq!(re_enabled.status(), StatusCode::OK);
    let after_reenable = fs::read_to_string(&outbox_path).expect("outbox after re-enable");
    let current_unsubscribe_link = after_reenable
        .lines()
        .rfind(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .expect("current signed unsubscribe link")
        .to_owned();
    assert_ne!(current_unsubscribe_link, unsubscribe_link);

    let stale_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&unsubscribe_link)
                .body(Body::empty())
                .expect("stale unsubscribe GET"),
        )
        .await
        .expect("stale unsubscribe GET response");
    assert_eq!(stale_get.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&stale_get);
    assert_eq!(
        stale_get
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let stale_get_body = response_text(stale_get).await;
    assert!(stale_get_body.contains("reminder link"));
    assert!(stale_get_body.contains(r#"<a class="ae-accent" href="/">Back to Scry</a>"#));
    assert!(!stale_get_body.contains(r#"{"error""#));
    assert!(!stale_get_body.contains("Memory Engine"));
    let stale_post = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/return-notifications",
            &[(
                "unsubscribeToken",
                unsubscribe_link
                    .split("token=")
                    .nth(1)
                    .expect("stale token"),
            )],
        ))
        .await
        .expect("stale unsubscribe POST response");
    assert_eq!(stale_post.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&stale_post);
    assert_eq!(
        stale_post
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let stale_post_body = response_text(stale_post).await;
    assert!(stale_post_body.contains("reminder link"));
    assert!(!stale_post_body.contains(r#"{"error""#));

    let current_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&current_unsubscribe_link)
                .body(Body::empty())
                .expect("current unsubscribe GET"),
        )
        .await
        .expect("current unsubscribe GET response");
    assert_eq!(current_get.status(), StatusCode::OK);
    assert_no_store_and_no_referrer(&current_get);
    let current_post = app
        .oneshot(form_request(
            "POST",
            "/app/return-notifications",
            &[(
                "unsubscribeToken",
                current_unsubscribe_link
                    .split("token=")
                    .nth(1)
                    .expect("current token"),
            )],
        ))
        .await
        .expect("current unsubscribe POST response");
    assert_eq!(current_post.status(), StatusCode::OK);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn invalid_or_expired_return_notification_token_renders_direct_recovery_instead_of_json() {
    // A token that was never issued: no `.` payload/signature separator, so
    // `verify_unsubscribe_token` fails before any HMAC computation. Needs no
    // account/auth setup — the failure is structural, not scope-related.
    let bogus_app = router(ApiState::default());
    let bogus_get = bogus_app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/return-notifications?token=not-a-real-token")
                .body(Body::empty())
                .expect("bogus token GET"),
        )
        .await
        .expect("bogus token GET response");
    assert_eq!(bogus_get.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&bogus_get);
    assert_eq!(
        bogus_get
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let bogus_get_body = response_text(bogus_get).await;
    assert!(bogus_get_body.contains("That reminder link needs a refresh"));
    assert!(bogus_get_body.contains(r#"<a class="ae-accent" href="/">Back to Scry</a>"#));
    assert!(!bogus_get_body.contains(r#"{"error""#));
    assert!(!bogus_get_body.contains("Memory Engine"));

    let bogus_post = bogus_app
        .oneshot(form_request(
            "POST",
            "/app/return-notifications",
            &[("unsubscribeToken", "not-a-real-token")],
        ))
        .await
        .expect("bogus token POST response");
    assert_eq!(bogus_post.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&bogus_post);
    assert_eq!(
        bogus_post
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let bogus_post_body = response_text(bogus_post).await;
    assert!(bogus_post_body.contains("That reminder link needs a refresh"));
    assert!(!bogus_post_body.contains(r#"{"error""#));

    // A real, correctly signed token whose `expires_at_ms` has since passed.
    let (expiry_clock, expiry_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    expiry_clock.store(DEFAULT_BETA_STUDY_NOW, Ordering::SeqCst);
    let store_root = temp_store_root("return-notification-token-expiry-recovery");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(expiry_now)
            .with_auth_config(
                AuthConfig::allow_emails(["learner@example.com".to_owned()])
                    .with_unsubscribe_secret("test-unsubscribe-secret")
                    .with_link_outbox(&outbox_path),
            ),
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
    let verify_path = fs::read_to_string(&outbox_path)
        .expect("auth outbox")
        .lines()
        .next()
        .and_then(|line| line.split('\t').nth(1))
        .map(str::to_owned)
        .expect("magic link");
    drop(requested);
    let verified = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&verify_path)
                .body(Body::empty())
                .expect("verify request"),
        )
        .await
        .expect("verify");
    let cookie = session_cookie(&verified);
    let verified = response_text(verified).await;
    let csrf_token = html_value(&verified, "csrfToken");

    app.clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/return-notifications",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("enabled", "on"),
                ("reminderEmail", "learner@example.com"),
            ],
        ))
        .await
        .expect("enable return channel");
    let outbox = fs::read_to_string(&outbox_path).expect("outbox after enable");
    let unsubscribe_link = outbox
        .lines()
        .find(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .expect("signed unsubscribe link")
        .to_owned();
    let token = unsubscribe_link
        .split("token=")
        .nth(1)
        .expect("token")
        .to_owned();

    expiry_clock.fetch_add(RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS + 1, Ordering::SeqCst);

    let expired_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&unsubscribe_link)
                .body(Body::empty())
                .expect("expired token GET"),
        )
        .await
        .expect("expired token GET response");
    assert_eq!(expired_get.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&expired_get);
    assert_eq!(
        expired_get
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let expired_get_body = response_text(expired_get).await;
    assert!(expired_get_body.contains("That reminder link needs a refresh"));
    assert!(expired_get_body.contains("invalid or expired"));
    assert!(!expired_get_body.contains(r#"{"error""#));

    let expired_post = app
        .oneshot(form_request(
            "POST",
            "/app/return-notifications",
            &[("unsubscribeToken", &token)],
        ))
        .await
        .expect("expired token POST response");
    assert_eq!(expired_post.status(), StatusCode::FORBIDDEN);
    assert_no_store_and_no_referrer(&expired_post);
    assert_eq!(
        expired_post
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/html; charset=utf-8"
    );
    let expired_post_body = response_text(expired_post).await;
    assert!(expired_post_body.contains("That reminder link needs a refresh"));
    assert!(!expired_post_body.contains(r#"{"error""#));
}

#[test]
fn return_notification_email_must_belong_to_authenticated_account() {
    let store_root = temp_store_root("return-notification-account-scope");
    let state = ApiState::new(
        AccountRegistry::with_store_root(&store_root).with_auth_config(
            AuthConfig::allow_emails([
                "account-a@example.com".to_owned(),
                "account-b@example.com".to_owned(),
            ])
            .with_unsubscribe_secret("test-unsubscribe-secret"),
        ),
    );
    let account_a = state
        .create_account("account-a@example.com")
        .expect("account A");
    let account_b = state
        .create_account("account-b@example.com")
        .expect("account B");
    let session_a = state
        .create_browser_session(&account_a)
        .expect("account A session");
    let session_b = state
        .create_browser_session(&account_b)
        .expect("account B session");

    assert_ne!(session_a.account_id(), session_b.account_id());
    let error = state
        .set_return_notification(&session_a, Some("account-b@example.com"), true)
        .expect_err("account A must not configure account B's allowlisted email");
    assert_eq!(
        error.message,
        "That reminder email must belong to the authenticated account."
    );
    state
        .set_return_notification(&session_a, Some("account-a@example.com"), true)
        .expect("account A's own allowlisted email");
    state
        .set_return_notification(&session_b, Some("account-b@example.com"), true)
        .expect("account B's own allowlisted email");
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_return_notification_nonce_invalidates_replayed_tokens() {
    let Some(database) = PostgresTestDatabase::new("return_unsubscribe_nonce") else {
        eprintln!(
            "skipping live Postgres unsubscribe nonce test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
        );
        return;
    };
    let outbox_path = temp_store_root("postgres-return-unsubscribe-nonce").join("outbox.tsv");
    let state = ApiState::new(
        AccountRegistry::with_postgres_url(&database.scoped_url).with_auth_config(
            AuthConfig::allow_emails(["postgres@example.com".to_owned()])
                .with_unsubscribe_secret("test-unsubscribe-secret")
                .with_link_outbox(&outbox_path),
        ),
    );
    let created = state
        .create_account("postgres@example.com")
        .expect("Postgres account");
    let account = state
        .create_browser_session(&created)
        .expect("Postgres browser session");
    state
        .set_return_notification(&account, Some("postgres@example.com"), true)
        .expect("enable Postgres reminders");
    assert!(state
        .maybe_send_due_count_notification(&account, 2, true)
        .expect("send initial Postgres reminder"));
    let first_token = fs::read_to_string(&outbox_path)
        .expect("initial Postgres reminder outbox")
        .lines()
        .find(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .and_then(|link| link.split("token=").nth(1))
        .expect("initial Postgres unsubscribe token")
        .to_owned();

    state
        .disable_return_notification(&first_token)
        .expect("disable Postgres reminders");
    state
        .set_return_notification(&account, Some("postgres@example.com"), true)
        .expect("re-enable Postgres reminders");
    assert!(state
        .maybe_send_due_count_notification(&account, 2, true)
        .expect("send current Postgres reminder"));
    let current_token = fs::read_to_string(&outbox_path)
        .expect("current Postgres reminder outbox")
        .lines()
        .rfind(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .and_then(|link| link.split("token=").nth(1))
        .expect("current Postgres unsubscribe token")
        .to_owned();
    assert_ne!(first_token, current_token);
    assert!(state
        .validate_return_notification_token(&first_token)
        .is_err());
    assert!(state.disable_return_notification(&first_token).is_err());
    assert!(state
        .validate_return_notification_token(&current_token)
        .is_ok());
    state
        .disable_return_notification(&current_token)
        .expect("current Postgres unsubscribe token");
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_return_notification_runs_through_real_postgres() {
    let Some(database) = PostgresTestDatabase::new("scheduled_return_notification") else {
        eprintln!(
            "skipping live Postgres scheduled reminder test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
        );
        return;
    };
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let outbox_path = temp_store_root("postgres-scheduled-return-notification").join("outbox.tsv");
    let state = ApiState::new(
        AccountRegistry::with_postgres_url(&database.scoped_url)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["scheduled-postgres@example.com".to_owned()])
                    .with_unsubscribe_secret("postgres-scheduler-secret")
                    .with_link_outbox(&outbox_path),
            ),
    );
    let created = state
        .create_account("scheduled-postgres@example.com")
        .expect("Postgres account");
    let account = state
        .create_browser_session(&created)
        .expect("Postgres browser session");
    let source = state
        .save_source(
            account.account_id(),
            account.session_token(),
            &CreateSourceRequest {
                title: "Postgres scheduled source".to_owned(),
                body: source_body(),
                permission: SourcePermission::ModelEligible,
            },
        )
        .expect("Postgres source");
    generate_source_queued(
        &state,
        account.account_id(),
        account.session_token(),
        &source.source_id,
        "Postgres scheduled source",
    )
    .await;
    let view = state
        .study_view(account.account_id(), account.session_token())
        .expect("Postgres study view");
    assert!(view.due_count > 0);
    state
        .set_return_notification(&account, Some("scheduled-postgres@example.com"), true)
        .expect("Postgres enable");
    assert!(state
        .maybe_send_due_count_notification(&account, view.due_count, true)
        .expect("Postgres confirmation"));
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);

    let report = state
        .run_scheduled_return_notifications()
        .expect("Postgres scheduled run");

    assert_eq!(report.sent, 1);
    let messages = fs::read_to_string(&outbox_path)
        .expect("Postgres reminder outbox")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert!(messages
        .last()
        .is_some_and(|message| message.contains("scheduled-postgres@example.com")
            && message.contains(&format!("\t{}\t", view.due_count))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_scheduler_retries_after_restart_and_contends_across_instances() {
    let Some(database) = PostgresTestDatabase::new("scheduled_return_notification_recovery") else {
        eprintln!(
            "skipping live Postgres scheduler recovery test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
        );
        return;
    };
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("postgres-scheduler-recovery");
    fs::create_dir_all(&store_root).expect("Postgres recovery root");
    let retry_command = retry_provider_script(&store_root);
    let auth_config = || {
        AuthConfig::allow_emails(["recovery@example.com".to_owned()])
            .with_unsubscribe_secret("postgres-recovery-secret")
            .with_mailer_command(&retry_command)
    };
    let first_state = ApiState::new(
        AccountRegistry::with_postgres_url(&database.scoped_url)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let account = prepare_postgres_due_account(&first_state, "recovery@example.com").await;
    first_state
        .set_return_notification(&account, Some("recovery@example.com"), true)
        .expect("Postgres recovery opt-in");
    let failed = first_state
        .run_scheduled_return_notifications()
        .expect("Postgres failed scheduler run");
    assert_eq!(failed.failed, 1);
    assert_eq!(failed.sent, 0);
    let failed_preference = first_state
        .load_return_notification_preference_for_test(account.account_id())
        .expect("failed preference read")
        .expect("failed preference");
    assert!(failed_preference.pending_delivery_key.is_some());
    assert!(failed_preference.next_retry_at_ms.is_some());

    test_clock.fetch_add(60_001, Ordering::SeqCst);
    let restarted_state = ApiState::new(
        AccountRegistry::with_postgres_url(&database.scoped_url)
            .with_clock(test_now)
            .with_auth_config(auth_config()),
    );
    let recovered = restarted_state
        .run_scheduled_return_notifications()
        .expect("Postgres restarted scheduler run");
    assert_eq!(
        recovered.sent, 1,
        "retry survives a new state/store adapter"
    );
    let payloads = fs::read_to_string(store_root.join("retry-provider.tsv"))
        .expect("retry provider capture")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0], payloads[1], "retry envelope remains stable");

    let slow_command = slow_provider_script(&store_root);
    test_clock.fetch_add(RETURN_NOTIFICATION_INTERVAL_MS + 1, Ordering::SeqCst);
    let reports = run_postgres_scheduler_contenders(&database.scoped_url, &slow_command, test_now);
    assert_eq!(
        reports.iter().sum::<usize>(),
        1,
        "two Postgres instances produce one logical send"
    );
    assert_eq!(
        fs::read_to_string(store_root.join("slow-provider.tsv"))
            .expect("slow provider capture")
            .lines()
            .count(),
        1,
        "the durable claim fences the second instance"
    );
}

#[tokio::test]
async fn unsubscribe_tokens_are_scoped_signed_expiring_and_get_is_read_only() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("unsubscribe-token-security");
    let outbox_path = store_root.join("auth-outbox.tsv");
    let registry = AccountRegistry::with_store_root(&store_root)
        .with_clock(test_now)
        .with_auth_config(
            AuthConfig::allow_emails([
                "owner@example.com".to_owned(),
                "other@example.com".to_owned(),
            ])
            .with_unsubscribe_secret("test-unsubscribe-secret")
            .with_link_outbox(&outbox_path),
        );
    let state = ApiState::new(registry);
    let owner = state.create_account("owner@example.com").expect("owner");
    let owner = state.create_browser_session(&owner).expect("owner session");
    state
        .set_return_notification(&owner, Some("owner@example.com"), true)
        .expect("enable owner");
    assert!(state
        .maybe_send_due_count_notification(&owner, 2, true)
        .expect("send owner"));
    let link = fs::read_to_string(&outbox_path)
        .expect("outbox")
        .lines()
        .find(|line| line.starts_with("due-count\t"))
        .and_then(|line| line.split('\t').nth(4))
        .expect("unsubscribe link")
        .to_owned();
    let token = link.split("token=").nth(1).expect("token");
    assert!(state.validate_return_notification_token(token).is_ok());

    let mut tampered = token.to_owned();
    let index = tampered.find('.').expect("signature separator") + 1;
    tampered.replace_range(
        index..=index,
        if &tampered[index..=index] == "0" {
            "1"
        } else {
            "0"
        },
    );
    assert!(
        state.validate_return_notification_token(&tampered).is_err(),
        "signature tampering must fail"
    );

    let mut wrong_scope = token.to_owned();
    let replacement = if &wrong_scope[..1] == "0" { "1" } else { "0" };
    wrong_scope.replace_range(..1, replacement);
    assert!(
        state
            .validate_return_notification_token(&wrong_scope)
            .is_err(),
        "account-scope tampering must fail"
    );

    let other = state.create_account("other@example.com").expect("other");
    let other = state.create_browser_session(&other).expect("other session");
    state
        .set_return_notification(&other, Some("other@example.com"), true)
        .expect("enable other");
    state
        .disable_return_notification(token)
        .expect("disable owner");
    assert!(!state
        .maybe_send_due_count_notification(&owner, 1, true)
        .expect("owner remains disabled"));
    assert!(state
        .maybe_send_due_count_notification(&other, 1, true)
        .expect("other remains independently enabled"));

    test_clock.fetch_add(RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS + 1, Ordering::SeqCst);
    assert!(
        state.validate_return_notification_token(token).is_err(),
        "expired unsubscribe links must fail"
    );
}

#[test]
fn file_return_notification_claim_allows_one_concurrent_sender() {
    let (_, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("return-notification-claim");
    let outbox_path = store_root.join("auth-outbox.tsv");
    // The clock is actually injected now: this test used to store a shared
    // test clock it never wired in, leaving the claim race running on wall
    // time (memory-engine-101).
    let registry = AccountRegistry::with_store_root(&store_root)
        .with_clock(test_now)
        .with_auth_config(
            AuthConfig::allow_emails(["claim@example.com".to_owned()])
                .with_unsubscribe_secret("claim-secret")
                .with_link_outbox(&outbox_path),
        );
    let state = ApiState::new(registry.clone());
    let created = state.create_account("claim@example.com").expect("account");
    let account = state.create_browser_session(&created).expect("session");
    state
        .set_return_notification(&account, Some("claim@example.com"), true)
        .expect("enable");

    let barrier = Arc::new(Barrier::new(32));
    let workers = (0..32)
        .map(|_| {
            let state = state.clone();
            let account = account.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                state
                    .maybe_send_due_count_notification(&account, 3, true)
                    .expect("claim send")
            })
        })
        .collect::<Vec<_>>();
    let sent = workers
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .filter(|sent| *sent)
        .count();
    assert_eq!(sent, 1, "one durable claim may send");
    let outbox = fs::read_to_string(&outbox_path).expect("outbox");
    assert_eq!(
        outbox
            .lines()
            .filter(|line| line.starts_with("due-count\t"))
            .count(),
        1
    );
}

#[test]
fn file_return_notification_retry_reuses_the_failed_provider_payload() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("return-notification-retry");
    fs::create_dir_all(&store_root).expect("retry store root");
    let failing_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["retry@example.com".to_owned()])
                    .with_unsubscribe_secret("claim-secret")
                    .with_mailer_command(retry_provider_script(&store_root)),
            ),
    );
    let retry_account = failing_state
        .create_account("retry@example.com")
        .expect("retry account");
    let retry_account = failing_state
        .create_browser_session(&retry_account)
        .expect("retry session");
    failing_state
        .set_return_notification(&retry_account, Some("retry@example.com"), true)
        .expect("enable retry");
    assert!(failing_state
        .maybe_send_due_count_notification(&retry_account, 1, true)
        .is_err());
    let first_payload =
        fs::read_to_string(store_root.join("retry-provider.tsv")).expect("failed provider payload");
    assert_eq!(first_payload.lines().count(), 1);
    let retry_path = fs::read_dir(&store_root)
        .expect("store root")
        .flatten()
        .find_map(|entry| {
            let path = entry.path().join("return-notifications.json");
            fs::read_to_string(path)
                .ok()
                .filter(|body| body.contains("retry@example.com"))
        })
        .expect("failed claim persisted");
    assert!(retry_path.contains("pendingDeliveryKey"));
    let recovery_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["retry@example.com".to_owned()])
                    .with_unsubscribe_secret("claim-secret")
                    .with_mailer_command(retry_provider_script(&store_root)),
            ),
    );
    test_clock.fetch_add(123_456, Ordering::SeqCst);
    assert!(recovery_state
        .maybe_send_due_count_notification(&retry_account, 1, true)
        .expect("retry send"));
    let payloads = fs::read_to_string(store_root.join("retry-provider.tsv"))
        .expect("provider payloads")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        payloads[0], payloads[1],
        "retry payload must be byte-identical"
    );
    let first_fields = payloads[0].split('\t').collect::<Vec<_>>();
    let second_fields = payloads[1].split('\t').collect::<Vec<_>>();
    assert_eq!(first_fields.len(), 4);
    assert_eq!(second_fields.len(), 4);
    assert_eq!(
        first_fields[3], second_fields[3],
        "retry idempotency key must persist"
    );
}

#[test]
fn notification_retry_and_success_timestamps_sample_after_slow_provider_returns() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("return-notification-provider-clock");
    fs::create_dir_all(&store_root).expect("provider clock root");
    let failing_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["clock@example.com".to_owned()])
                    .with_unsubscribe_secret("clock-secret")
                    .with_mailer_command(slow_failing_provider_script(&store_root)),
            ),
    );
    let created = failing_state
        .create_account("clock@example.com")
        .expect("clock account");
    let account = failing_state
        .create_browser_session(&created)
        .expect("clock session");
    failing_state
        .set_return_notification(&account, Some("clock@example.com"), true)
        .expect("clock opt-in");
    let failing_state_for_thread = failing_state.clone();
    let account_for_thread = account.clone();
    let send = thread::spawn(move || {
        failing_state_for_thread.maybe_send_due_count_notification(&account_for_thread, 1, true)
    });
    while !store_root.join("slow-failing-provider.started").exists() {
        thread::sleep(Duration::from_millis(10));
    }
    let after_provider_started = DEFAULT_BETA_STUDY_NOW + 123_456;
    test_clock.store(after_provider_started, Ordering::SeqCst);
    assert!(send.join().expect("slow failing sender").is_err());
    let failed = failing_state
        .load_return_notification_preference_for_test(account.account_id())
        .expect("failed preference")
        .expect("failed preference row");
    assert!(failed.next_retry_at_ms.expect("retry time") > after_provider_started);

    let success_state = ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_clock(test_now)
            .with_auth_config(
                AuthConfig::allow_emails(["clock@example.com".to_owned()])
                    .with_unsubscribe_secret("clock-secret")
                    .with_mailer_command(slow_provider_script(&store_root)),
            ),
    );
    test_clock.store(
        failed.next_retry_at_ms.expect("retry time") + 1,
        Ordering::SeqCst,
    );
    let success_state_for_thread = success_state.clone();
    let account_for_thread = account.clone();
    let send = thread::spawn(move || {
        success_state_for_thread.maybe_send_due_count_notification(&account_for_thread, 1, true)
    });
    while !store_root.join("slow-provider.tsv").exists() {
        thread::sleep(Duration::from_millis(10));
    }
    let completed_at = test_clock.load(Ordering::SeqCst) + 222_222;
    test_clock.store(completed_at, Ordering::SeqCst);
    assert!(send.join().expect("slow success sender").is_ok());
    let completed = success_state
        .load_return_notification_preference_for_test(account.account_id())
        .expect("completed preference")
        .expect("completed preference row");
    assert_eq!(completed.last_sent_at_ms, Some(completed_at));
    let _ = fs::remove_dir_all(store_root);
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
    // accepted drafts remain pending until explicit keeps.)
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
    assert!(queued.contains("Generating. Watch the activity log."));

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
    assert_eq!(
        restored.card_count, 0,
        "restored job records zero scheduled cards"
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

    // Generation reports zero scheduled cards; accepted drafts remain durable.
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
    assert_eq!(reported, 0, "generation must not schedule cards");
    assert_eq!(
        persisted, 0,
        "no review units exist before learner decisions"
    );
    assert!(
        study["generatedPromptDrafts"]
            .as_array()
            .is_some_and(|drafts| !drafts.is_empty()),
        "accepted drafts remain durable for learner review"
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

fn service_session_request(admin_token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/service-sessions")
        .header("content-type", "application/json");
    if let Some(token) = admin_token {
        builder = builder.header("x-admin-token", token);
    }
    builder.body(Body::from(body.to_owned())).expect("request")
}

fn service_session_state(admin_token: &str) -> ApiState {
    ApiState::new(AccountRegistry::default().with_auth_config(
        AuthConfig::allow_emails(["dogfood@example.com".to_owned()]).with_admin_token(admin_token),
    ))
}

#[tokio::test]
async fn service_session_issuance_is_disabled_without_a_configured_admin_token() {
    // No admin token configured: the endpoint refuses every caller, so a
    // default deployment exposes no service-session surface at all.
    let response = router(ApiState::default())
        .oneshot(service_session_request(
            Some("anything"),
            r#"{"email":"dogfood@example.com"}"#,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response_json(response).await;
    assert!(body.get("sessionToken").is_none());
}

#[tokio::test]
async fn service_session_issuance_rejects_a_wrong_or_missing_admin_token() {
    let app = router(service_session_state("operator-admin-token"));

    let wrong = app
        .clone()
        .oneshot(service_session_request(
            Some("not-the-token"),
            r#"{"email":"dogfood@example.com"}"#,
        ))
        .await
        .expect("wrong token response");
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    let missing = app
        .oneshot(service_session_request(
            None,
            r#"{"email":"dogfood@example.com"}"#,
        ))
        .await
        .expect("missing token response");
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    let body = response_json(missing).await;
    assert!(body.get("sessionToken").is_none());
}

#[tokio::test]
async fn service_session_issuance_refuses_unauthorized_bodies_before_parsing() {
    // An unauthorized caller must get 403 even with a malformed body: the
    // admin-token gate runs before the JSON parser ever sees the payload.
    let app = router(service_session_state("operator-admin-token"));

    let unauthorized = app
        .clone()
        .oneshot(service_session_request(
            Some("not-the-token"),
            "{not json at all",
        ))
        .await
        .expect("unauthorized malformed response");
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);

    // The same malformed body with a valid token is a 400 from our envelope.
    let malformed = app
        .oneshot(service_session_request(
            Some("operator-admin-token"),
            "{not json at all",
        ))
        .await
        .expect("authorized malformed response");
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn service_session_issuance_enforces_the_email_allowlist() {
    let app = router(service_session_state("operator-admin-token"));

    let denied = app
        .oneshot(service_session_request(
            Some("operator-admin-token"),
            r#"{"email":"stranger@example.com"}"#,
        ))
        .await
        .expect("response");

    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let body = response_json(denied).await;
    assert!(body.get("sessionToken").is_none());
}

#[tokio::test]
async fn service_session_issuance_rejects_a_malformed_email() {
    let app = router(service_session_state("operator-admin-token"));

    let response = app
        .oneshot(service_session_request(
            Some("operator-admin-token"),
            r#"{"email":"not-an-email"}"#,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn service_session_issues_a_credential_that_drives_the_account_api() {
    let app = router(service_session_state("operator-admin-token"));

    let issued = app
        .clone()
        .oneshot(service_session_request(
            Some("operator-admin-token"),
            r#"{"email":"dogfood@example.com"}"#,
        ))
        .await
        .expect("issue response");
    assert_eq!(issued.status(), StatusCode::CREATED);
    let issued = response_json(issued).await;
    let account_id = issued["accountId"].as_str().expect("account id");
    let session_token = issued["sessionToken"].as_str().expect("session token");
    assert!(session_token.starts_with("sess_"));

    let saved = app
        .oneshot(json_request(
            "POST",
            &format!("/v1/accounts/{account_id}/sources"),
            session_token,
            &json!({
                "title": "NATO notes",
                "body": "ALFA is the NATO code word for A."
            }),
        ))
        .await
        .expect("save source");
    assert_eq!(saved.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn service_session_reissue_revokes_the_prior_credential_immediately() {
    let app = router(service_session_state("operator-admin-token"));

    let first = response_json(
        app.clone()
            .oneshot(service_session_request(
                Some("operator-admin-token"),
                r#"{"email":"dogfood@example.com"}"#,
            ))
            .await
            .expect("first issue"),
    )
    .await;
    let second = response_json(
        app.clone()
            .oneshot(service_session_request(
                Some("operator-admin-token"),
                r#"{"email":"dogfood@example.com"}"#,
            ))
            .await
            .expect("second issue"),
    )
    .await;
    let account_id = second["accountId"].as_str().expect("account id");
    assert_eq!(first["accountId"], second["accountId"]);
    assert_ne!(first["sessionToken"], second["sessionToken"]);

    let revoked = app
        .clone()
        .oneshot(json_request(
            "GET",
            &format!("/v1/accounts/{account_id}/sources"),
            first["sessionToken"].as_str().expect("first token"),
            &json!({}),
        ))
        .await
        .expect("revoked read");
    assert_eq!(revoked.status(), StatusCode::FORBIDDEN);

    let live = app
        .oneshot(json_request(
            "GET",
            &format!("/v1/accounts/{account_id}/sources"),
            second["sessionToken"].as_str().expect("second token"),
            &json!({}),
        ))
        .await
        .expect("live read");
    assert_eq!(live.status(), StatusCode::OK);
}

#[tokio::test]
async fn service_session_credential_is_isolated_to_its_own_account() {
    let state = ApiState::new(
        AccountRegistry::default().with_auth_config(
            AuthConfig::allow_emails([
                "dogfood@example.com".to_owned(),
                "human@example.com".to_owned(),
            ])
            .with_admin_token("operator-admin-token"),
        ),
    );
    let app = router(state);
    let human = create_account(&app, "human@example.com").await;

    let issued = response_json(
        app.clone()
            .oneshot(service_session_request(
                Some("operator-admin-token"),
                r#"{"email":"dogfood@example.com"}"#,
            ))
            .await
            .expect("issue response"),
    )
    .await;
    let service_token = issued["sessionToken"].as_str().expect("service token");

    let cross_read = app
        .clone()
        .oneshot(json_request(
            "GET",
            &format!("/v1/accounts/{}/sources", human.account_id),
            service_token,
            &json!({}),
        ))
        .await
        .expect("cross read");
    assert_eq!(cross_read.status(), StatusCode::FORBIDDEN);

    let cross_write = app
        .oneshot(json_request(
            "POST",
            &format!("/v1/accounts/{}/sources", human.account_id),
            service_token,
            &json!({
                "title": "Injected",
                "body": "Should never land in another account."
            }),
        ))
        .await
        .expect("cross write");
    assert_eq!(cross_write.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn service_session_enqueues_and_observes_durable_generation_without_a_browser() {
    let state = service_session_state("operator-admin-token");
    let app = router(state.clone());
    let issued = response_json(
        app.clone()
            .oneshot(service_session_request(
                Some("operator-admin-token"),
                r#"{"email":"dogfood@example.com"}"#,
            ))
            .await
            .expect("issue response"),
    )
    .await;
    let account = TestAccount {
        account_id: issued["accountId"].as_str().expect("account id").to_owned(),
        session_token: issued["sessionToken"]
            .as_str()
            .expect("session token")
            .to_owned(),
    };
    let source_id = create_source_v1(
        &app,
        &account,
        "NATO notes",
        "Concept: NATO phonetic alphabet\nQuestion: What is NATO code word for A?\nAnswer: ALFA\nReference: ALFA is the NATO code word for A.",
    )
    .await;
    let enqueue_path = format!(
        "/v1/accounts/{}/sources/{source_id}/generation-jobs",
        account.account_id
    );

    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&enqueue_path)
                .body(Body::empty())
                .expect("unauthenticated enqueue"),
        )
        .await
        .expect("unauthenticated response");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let first = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &enqueue_path,
            &account.session_token,
        ))
        .await
        .expect("enqueue response");
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first = response_json(first).await;
    let job_id = first["id"].as_str().expect("job id").to_owned();
    assert_eq!(first["sourceId"], json!(source_id));
    assert_eq!(first["status"], json!("queued"));
    assert_eq!(first["coalesced"], json!(false));

    let repeated = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &enqueue_path,
            &account.session_token,
        ))
        .await
        .expect("repeated enqueue response");
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated = response_json(repeated).await;
    assert_eq!(repeated["id"], json!(job_id));
    assert_eq!(repeated["coalesced"], json!(true));
    assert_eq!(state.jobs_for_account_id(&account.account_id).len(), 1);

    let blocking_state = state.clone();
    tokio::task::spawn_blocking(move || blocking_state.run_pending_jobs_blocking())
        .await
        .expect("generation drain");

    let observed = app
        .oneshot(v1_empty_request(
            "GET",
            &format!(
                "/v1/accounts/{}/generation-jobs/{job_id}",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("job observation response");
    assert_eq!(observed.status(), StatusCode::OK);
    let observed = response_json(observed).await;
    assert_eq!(observed["id"], json!(job_id));
    assert_eq!(observed["sourceId"], json!(source_id));
    assert_eq!(observed["status"], json!("succeeded"));
    assert!(
        observed["error"].is_null(),
        "successful jobs must preserve the nullable error field: {observed}"
    );
    assert_eq!(
        observed["cardCount"],
        json!(0),
        "successful generation must leave candidates pending: {observed}"
    );
}

#[tokio::test]
async fn generation_job_observation_preserves_the_failed_contract() {
    let state = ApiState::default();
    let app = router(state.clone());
    let account = create_account_v1(&app, "failed-job@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Archived source",
        "Concept: durable jobs\nQuestion: What survives?\nAnswer: The job record.",
    )
    .await;
    let enqueued = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/sources/{source_id}/generation-jobs",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("enqueue response");
    assert_eq!(enqueued.status(), StatusCode::ACCEPTED);
    let job_id = response_json(enqueued).await["id"]
        .as_str()
        .expect("job id")
        .to_owned();

    archive_source_v1(&app, &account, &source_id).await;
    state.run_pending_jobs_blocking();

    let observed = app
        .oneshot(v1_empty_request(
            "GET",
            &format!(
                "/v1/accounts/{}/generation-jobs/{job_id}",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("failed job response");
    assert_eq!(observed.status(), StatusCode::OK);
    let observed = response_json(observed).await;
    assert_eq!(observed["status"], json!("failed"));
    assert_eq!(observed["cardCount"], json!(0));
    assert_eq!(observed["retryable"], json!(true));
    assert_eq!(observed["error"], json!("Source not found."));
}

#[tokio::test]
async fn generation_jobs_are_scoped_to_the_bearer_account() {
    let app = router(ApiState::default());
    let service = create_account_v1(&app, "dogfood@example.com").await;
    let human = create_account_v1(&app, "human@example.com").await;
    let source_id = create_source_v1(
        &app,
        &service,
        "NATO notes",
        "ALFA is the NATO code word for A.",
    )
    .await;
    let enqueue_path = format!(
        "/v1/accounts/{}/sources/{source_id}/generation-jobs",
        service.account_id
    );
    let enqueued = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &enqueue_path,
            &service.session_token,
        ))
        .await
        .expect("enqueue response");
    assert_eq!(enqueued.status(), StatusCode::ACCEPTED);
    let job_id = response_json(enqueued).await["id"]
        .as_str()
        .expect("job id")
        .to_owned();
    let cross_enqueue = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/sources/{source_id}/generation-jobs",
                service.account_id
            ),
            &human.session_token,
        ))
        .await
        .expect("cross-account enqueue response");
    assert_eq!(cross_enqueue.status(), StatusCode::FORBIDDEN);

    let cross_observation = app
        .clone()
        .oneshot(v1_empty_request(
            "GET",
            &format!(
                "/v1/accounts/{}/generation-jobs/{job_id}",
                service.account_id
            ),
            &human.session_token,
        ))
        .await
        .expect("cross-account observation response");
    assert_eq!(cross_observation.status(), StatusCode::FORBIDDEN);

    let hidden_source = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/sources/{source_id}/generation-jobs",
                human.account_id
            ),
            &human.session_token,
        ))
        .await
        .expect("account-scoped source response");
    assert_eq!(hidden_source.status(), StatusCode::NOT_FOUND);

    let hidden_job = app
        .oneshot(v1_empty_request(
            "GET",
            &format!("/v1/accounts/{}/generation-jobs/{job_id}", human.account_id),
            &human.session_token,
        ))
        .await
        .expect("account-scoped job response");
    assert_eq!(hidden_job.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn generation_job_routes_report_missing_owned_resources() {
    let app = router(ApiState::default());
    let account = create_account_v1(&app, "dogfood@example.com").await;

    let missing_source = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/sources/src-does-not-exist/generation-jobs",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("missing source response");
    assert_eq!(missing_source.status(), StatusCode::NOT_FOUND);

    let missing_job = app
        .oneshot(v1_empty_request(
            "GET",
            &format!(
                "/v1/accounts/{}/generation-jobs/job-does-not-exist",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("missing job response");
    assert_eq!(missing_job.status(), StatusCode::NOT_FOUND);
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
async fn cross_account_token_rejection_is_stable_after_registry_restart() {
    let store_root = temp_store_root("cross-account-auth-parity");
    let warm_app = router(ApiState::new(super::AccountRegistry::with_store_root(
        &store_root,
    )));
    let first = create_account(&warm_app, "warm-first@example.com").await;
    let second = create_account(&warm_app, "warm-second@example.com").await;

    let warm = warm_app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/accounts/{}/sources", second.account_id),
            &first.session_token,
        ))
        .await
        .expect("warm cross-account read");
    assert_eq!(warm.status(), StatusCode::FORBIDDEN);

    let cold_app = router(ApiState::new(super::AccountRegistry::with_store_root(
        &store_root,
    )));
    let cold = cold_app
        .oneshot(empty_request(
            "GET",
            &format!("/accounts/{}/sources", second.account_id),
            &first.session_token,
        ))
        .await
        .expect("cold cross-account read");
    assert_eq!(cold.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(warm).await["error"],
        json!("Session token does not match account.")
    );
    assert_eq!(
        response_json(cold).await["error"],
        json!("Session token does not match account.")
    );
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
    assert_eq!(generated.status(), StatusCode::CONFLICT);
    let generated = response_json(generated).await;
    assert_eq!(
        generated["error"],
        json!("Direct synchronous generation is disabled in production. Use the queued generation workflow.")
    );
}

fn server_timing_duration(timing: &str, name: &str) -> u64 {
    timing
        .split(',')
        .map(str::trim)
        .find_map(|metric| metric.strip_prefix(&format!("{name};dur=")))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("missing {name} duration in {timing}"))
}

fn assert_postgres_submit_timing(
    response: &axum::response::Response,
    expected_statement_count: u64,
) {
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("submit request id");
    assert!(
        request_id.len() == 36
            && request_id.starts_with("req_")
            && request_id[4..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "request id must be strict req_[0-9a-f]{{32}}: {request_id}"
    );
    let timing = response
        .headers()
        .get("server-timing")
        .and_then(|value| value.to_str().ok())
        .expect("Postgres submit timing");
    assert!(timing.contains("pgconnect;dur="), "{timing}");
    assert!(timing.contains("pgop;dur="), "{timing}");
    let pgconnect_ms = server_timing_duration(timing, "pgconnect");
    let pgop_ms = server_timing_duration(timing, "pgop");
    assert!(
        pgconnect_ms > 0,
        "pgconnect phase must be nonzero, not silently zeroed: {timing}"
    );
    assert!(
        pgop_ms > 0,
        "pgop phase must be nonzero, not silently zeroed: {timing}"
    );
    let total_ms = server_timing_duration(timing, "total");
    let render_ms = server_timing_duration(timing, "render");
    let phase_sum_ms = pgconnect_ms + pgop_ms + render_ms;
    assert!(
        phase_sum_ms <= total_ms,
        "Postgres phases must fit inside total duration: {timing}"
    );
    let statement_count = timing
        .split(',')
        .map(str::trim)
        .find_map(|metric| metric.strip_prefix(r#"pgstmt;desc=""#))
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.parse::<u64>().ok())
        .expect("Postgres statement count");
    assert_eq!(
        statement_count, expected_statement_count,
        "cold-cache submit must count auth, review, and render work: {timing}"
    );
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
}

/// Cross-checks the server's self-reported `total` against wall-clock time
/// the test measured around the whole in-process request.
///
/// `normalize_submit_durations` raises the reported total to at least the
/// summed phase durations, so a bare `phase_sum <= total` assertion is
/// tautologically true by construction (`total` is *defined* as
/// `total.max(phase_sum)`) and can never catch the total itself being
/// inflated beyond reality. This closes that gap: memory-engine-109 review
/// finding — Postgres reads nested inside the timed render window on the
/// empty-queue/error path (`app_study_view_with_timings`,
/// `list_app_sources_with_timings`, `jobs_for_app_account_with_timings`)
/// used to be double-counted into both `render` and `pgconnect`/`pgop`,
/// which raises `phase_sum`, and therefore the reported `total`, above the
/// real elapsed time. `measured_ms` is real wall-clock elapsed time around
/// the entire in-process `oneshot` call, so the honestly reported total can
/// never legitimately exceed it by more than harness/rounding slack.
fn assert_reported_total_is_not_inflated_beyond_measured_wall_clock(
    response: &axum::response::Response,
    measured_ms: u64,
) {
    let timing = response
        .headers()
        .get("server-timing")
        .and_then(|value| value.to_str().ok())
        .expect("submit server timing");
    let total_ms = server_timing_duration(timing, "total");
    assert!(
        total_ms <= measured_ms + 25,
        "reported total {total_ms}ms must not exceed the independently \
         measured wall clock {measured_ms}ms by more than harness slack: {timing}"
    );
}

async fn assert_postgres_submit_receipt(
    graded: axum::response::Response,
    expected_statement_count: u64,
) -> String {
    assert_eq!(graded.status(), StatusCode::OK);
    assert_postgres_submit_timing(&graded, expected_statement_count);
    let graded = response_text(graded).await;
    assert!(graded.contains("me-verdict") && graded.contains(">Correct<"));
    graded
}

async fn prepare_postgres_browser_review(
    database: &PostgresTestDatabase,
) -> (String, String, String) {
    let browser_source = [
        "Concept: NATO letter A",
        "Activity: quiz",
        "Stage: recognition-3",
        "Question: What is the NATO phonetic alphabet word for A?",
        "Answer: ALFA",
        "Distractors: BRAVO, CHARLIE",
        "Reference: The NATO phonetic alphabet word for A is ALFA.",
    ]
    .join("\n");
    let browser_state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let browser_app = router(browser_state.clone());
    let started = browser_app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &browser_source)],
        ))
        .await
        .expect("start Postgres browser session");
    assert_eq!(started.status(), StatusCode::OK);
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    let generated = generate_source_html(
        &browser_app,
        &browser_state,
        &cookie,
        &csrf_token,
        &source_id,
    )
    .await;
    assert_activity_succeeded_html(&generated, 1);
    let review = advance_to_prompt(
        &browser_app,
        &cookie,
        &csrf_token,
        "NATO phonetic alphabet word for A",
    )
    .await;
    (html_value(&review, "reviewUnitId"), cookie, csrf_token)
}

async fn assert_postgres_browser_submit_traces(database: &PostgresTestDatabase) {
    let (review_unit_id, cookie, csrf_token) = prepare_postgres_browser_review(database).await;
    // A fresh process has no in-memory account or browser-session cache.
    // The submit receipt must still account for its authentication queries.
    let cold_browser_app = router(ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    )));
    let graded = cold_browser_app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "ALFA"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "postgres-submit-timing"),
                (
                    "performanceTraceId",
                    "trace_0123456789abcdef0123456789abcdef",
                ),
            ],
        ))
        .await
        .expect("Postgres browser submit");
    assert_postgres_submit_receipt(graded, 23).await;

    let completed_browser_app = router(ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    )));
    let completed = next_review_html(
        &completed_browser_app,
        &cookie,
        &csrf_token,
        "complete queue",
    )
    .await;
    assert!(
        !completed.contains(r#"name="reviewUnitId""#),
        "the final card should return to workspace after Continue"
    );

    let workspace_browser_app = router(ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    )));
    let workspace_submit_started = Instant::now();
    let workspace_submit = workspace_browser_app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", "missing-review-unit"),
                ("answer", "ALFA"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", "postgres-error-submit-timing"),
            ],
        ))
        .await
        .expect("Postgres browser submit without an active review");
    let workspace_submit_measured_ms =
        u64::try_from(workspace_submit_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    assert_eq!(workspace_submit.status(), StatusCode::OK);
    assert_postgres_submit_timing(&workspace_submit, 16);
    // This is the exact empty-queue/error-render path (missing review unit,
    // no active review) that nests app_study_view_with_timings +
    // list_app_sources_with_timings + jobs_for_app_account_with_timings
    // inside the timed render window — see
    // assert_reported_total_is_not_inflated_beyond_measured_wall_clock.
    assert_reported_total_is_not_inflated_beyond_measured_wall_clock(
        &workspace_submit,
        workspace_submit_measured_ms,
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
    assert_eq!(generated.status(), StatusCode::CONFLICT);
    let generated = response_json(generated).await;
    assert_eq!(
        generated["error"],
        json!("Direct synchronous generation is disabled in production. Use the queued generation workflow.")
    );

    assert_postgres_browser_submit_traces(&database).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_save_account_copies_content_feedback_with_target_scope() {
    let Some(database) = PostgresTestDatabase::new("account_copy_feedback") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let created = state
        .create_account("copy-source@example.com")
        .expect("source account");
    let browser = state
        .create_browser_session(&created)
        .expect("source browser session");
    let app = router(state.clone());
    let source_account = TestAccount {
        account_id: browser.account_id().to_owned(),
        session_token: browser.session_token().to_owned(),
    };
    let source_id = create_source_v1(&app, &source_account, "Copy source", &source_body()).await;
    generate_source_queued(
        &state,
        &source_account.account_id,
        &source_account.session_token,
        &source_id,
        "Copy source",
    )
    .await;
    let review_unit_id = next_review_v1(&app, &source_account).await;
    let _ = submit_review_v1(&app, &source_account, &review_unit_id, "ALFA").await;
    let feedback = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/accounts/{}/review/{review_unit_id}/content-feedback",
                source_account.account_id
            ),
            &source_account.session_token,
            &json!({
                "verdict": "dropped",
                "idempotencyKey": "copy-feedback-a"
            }),
        ))
        .await
        .expect("source feedback");
    assert_eq!(feedback.status(), StatusCode::OK);

    // A revision's ancestry is independent of wall-clock order. The helper
    // persists a parent later than its child for the copy oracle.
    seed_out_of_order_copy_feedback(
        &database.scoped_url,
        &source_account.account_id,
        &review_unit_id,
    );

    let target = state
        .save_account(&browser, "copy-target@example.com")
        .expect("copy account");
    let snapshot = tokio::task::block_in_place(|| {
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(&database.scoped_url)
                .expect("verify copied account");
        let account = store.for_account(
            memory_engine_persistence_postgres::AccountScope::new(target.account_id.clone())
                .expect("target scope"),
        );
        account.snapshot().expect("target snapshot")
    });
    assert_eq!(snapshot.content_feedback.len(), 3);
    assert!(snapshot
        .content_feedback
        .iter()
        .any(|feedback| feedback.id == "copy-feedback-a"));
    assert!(snapshot
        .content_feedback
        .iter()
        .any(|feedback| feedback.id == "copy-feedback-parent"));
    assert!(snapshot
        .content_feedback
        .iter()
        .any(|feedback| feedback.id == "copy-feedback-child"));
    assert_eq!(snapshot.content_feedback[0].account_id, target.account_id);
}

fn seed_out_of_order_copy_feedback(database_url: &str, account_id: &str, review_unit_id: &str) {
    use memory_engine_core::ReviewUnitId;
    use memory_engine_service::{
        record_content_feedback, ContentFeedbackVerdict, RecordContentFeedbackCommand,
    };

    tokio::task::block_in_place(|| {
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(database_url)
                .expect("seed out-of-order feedback");
        let mut account = store.for_account(
            memory_engine_persistence_postgres::AccountScope::new(account_id.to_owned())
                .expect("source scope"),
        );
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        record_content_feedback(
            &mut account,
            RecordContentFeedbackCommand {
                feedback_id: "copy-feedback-parent".to_owned(),
                review_unit_id: review_unit_id.clone(),
                verdict: ContentFeedbackVerdict::Kept,
                rationale: Some("parent".to_owned()),
                account_id: account_id.to_owned(),
                occurred_at: 1_000,
                supersedes_id: Some("copy-feedback-a".to_owned()),
            },
        )
        .expect("parent feedback");
        record_content_feedback(
            &mut account,
            RecordContentFeedbackCommand {
                feedback_id: "copy-feedback-child".to_owned(),
                review_unit_id,
                verdict: ContentFeedbackVerdict::Dropped,
                rationale: Some("child".to_owned()),
                account_id: account_id.to_owned(),
                occurred_at: 500,
                supersedes_id: Some("copy-feedback-parent".to_owned()),
            },
        )
        .expect("child feedback");
    });
}

#[tokio::test]
async fn file_save_account_preserves_content_feedback_for_copy_parity() {
    let store_root = temp_store_root("account-copy-feedback-file");
    let state = ApiState::new(AccountRegistry::with_store_root(&store_root));
    let created = state
        .create_account("file-copy-source@example.com")
        .expect("source account");
    let browser = state
        .create_browser_session(&created)
        .expect("source browser session");
    let app = router(state.clone());
    let source_account = TestAccount {
        account_id: browser.account_id().to_owned(),
        session_token: browser.session_token().to_owned(),
    };
    let source_id = create_source_v1(&app, &source_account, "Copy source", &source_body()).await;
    let draft_id = generate_source_v1(&app, &source_account, &source_id).await;
    let review_unit_id = keep_draft_v1(&app, &source_account, &draft_id).await;
    let _ = submit_review_v1(&app, &source_account, &review_unit_id, "ALFA").await;
    let feedback = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                source_account.account_id
            ),
            &source_account.session_token,
            &json!({
                "verdict": "dropped",
                "idempotencyKey": "file-copy-feedback-a"
            }),
        ))
        .await
        .expect("source feedback");
    assert_eq!(feedback.status(), StatusCode::OK);

    let target = state
        .save_account(&browser, "file-copy-target@example.com")
        .expect("copy account");
    let target_browser = state
        .create_browser_session(&target)
        .expect("target browser session");
    let target_store = memory_engine_persistence::BetaPersistenceStore::open(
        store_root.join(&target.account_id).join("study.json"),
    )
    .expect("verify copied account");
    assert_eq!(target_store.snapshot().content_feedback.len(), 1);
    assert_eq!(
        target_store.snapshot().content_feedback[0].id,
        "file-copy-feedback-a"
    );
    assert_eq!(
        target_store.snapshot().content_feedback[0].account_id,
        target.account_id
    );

    let child = app
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                target.account_id
            ),
            target_browser.session_token(),
            &json!({
                "verdict": "kept",
                "idempotencyKey": "file-copy-feedback-child",
                "supersedesId": "file-copy-feedback-a"
            }),
        ))
        .await
        .expect("target child feedback");
    assert_eq!(child.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual Postgres latency receipt"]
async fn postgres_review_actions_emit_latency_receipt() {
    let Some(database) = PostgresTestDatabase::new("latency_receipt") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let next_started = Instant::now();
    let page = next_review_html(&app, &cookie, &csrf_token, "latency next").await;
    let next_elapsed = next_started.elapsed();

    let review_unit_id = html_value(&page, "reviewUnitId");
    let idempotency_key = html_value(&page, "idempotencyKey");
    let submit_started = Instant::now();
    let submitted = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", management_answer_for_prompt(&page)),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", &idempotency_key),
            ],
        ))
        .await
        .expect("submit review");
    let submit_elapsed = submit_started.elapsed();
    assert_eq!(submitted.status(), StatusCode::OK);
    let submitted = response_text(submitted).await;
    assert!(
        submitted.contains("me-verdict"),
        "latency receipt submit must render graded feedback: {submitted}"
    );
    eprintln!("postgres review latency: next={next_elapsed:?} submit={submit_elapsed:?}");
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

    // Drain the enqueued job: generation runs and the accepted drafts remain pending until explicit keeps.
    restarted_state.run_pending_jobs_blocking();

    let pending = restarted_app
        .clone()
        .oneshot(form_request_with_cookie("GET", "/", &cookie, &[]))
        .await
        .expect("pending after restart");
    assert_eq!(pending.status(), StatusCode::OK);
    let pending = response_text(pending).await;
    let draft_id = html_value(&pending, "draftId");
    let kept = restarted_app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/draft/keep",
            &cookie,
            &[("csrfToken", &csrf_token), ("draftId", &draft_id)],
        ))
        .await
        .expect("keep after restart");
    assert_eq!(kept.status(), StatusCode::OK);

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

#[tokio::test(flavor = "multi_thread")]
async fn postgres_backend_v1_concept_snooze_is_authenticated_scoped_and_atomic() {
    let Some(database) = PostgresTestDatabase::new("v1_concept_snooze") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let app = router(state.clone());
    let first = create_account_v1(&app, "first-concept@example.com").await;
    let second = create_account_v1(&app, "second-concept@example.com").await;

    for account in [&first, &second] {
        let source_id = create_source_v1(
            &app,
            account,
            "Shared NATO concept notes",
            &shared_and_other_concept_body(),
        )
        .await;
        generate_source_queued(
            &state,
            &account.account_id,
            &account.session_token,
            &source_id,
            "Shared NATO concept notes",
        )
        .await;
    }

    let first_before = postgres_account_snapshot(&database.scoped_url, &first.account_id);
    let second_before = postgres_account_snapshot(&database.scoped_url, &second.account_id);
    assert_eq!(first_before.review_units.len(), 3);
    assert_eq!(second_before.review_units.len(), 3);
    assert_eq!(
        first_before
            .review_units
            .iter()
            .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-a"))
            .count(),
        2
    );

    let first_review_unit_id = next_review_v1(&app, &first).await;
    let cross_account = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{first_review_unit_id}/snooze-concept",
                second.account_id
            ),
            &first.session_token,
        ))
        .await
        .expect("cross-account concept snooze");
    assert_eq!(cross_account.status(), StatusCode::FORBIDDEN);

    let snoozed = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{first_review_unit_id}/snooze-concept",
                first.account_id
            ),
            &first.session_token,
        ))
        .await
        .expect("authenticated postgres concept snooze");
    assert_eq!(snoozed.status(), StatusCode::OK);
    let snoozed = response_json(snoozed).await;
    assert_eq!(snoozed["current"]["conceptKey"], json!("nato-letter-b"));
    assert_eq!(snoozed["dueCount"], json!(1));

    let first_after = postgres_account_snapshot(&database.scoped_url, &first.account_id);
    assert_eq!(first_after.attempts, first_before.attempts);
    assert_eq!(first_after.schedules, first_before.schedules);
    let first_shared = first_after
        .review_units
        .iter()
        .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-a"))
        .collect::<Vec<_>>();
    assert_eq!(first_shared.len(), 2);
    assert!(first_shared.iter().all(|unit| unit.snoozed_until.is_some()));
    assert_eq!(
        first_after
            .review_units
            .iter()
            .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-b"))
            .filter_map(|unit| unit.snoozed_until)
            .count(),
        0
    );

    let second_after = postgres_account_snapshot(&database.scoped_url, &second.account_id);
    assert_eq!(second_after, second_before);
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
async fn source_generation_keep_and_review_are_account_scoped() {
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

    let cross_keep = app
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/accounts/{}/drafts/{draft_id}/keep", first.account_id),
            &second.session_token,
        ))
        .await
        .expect("cross keep");
    assert_eq!(cross_keep.status(), StatusCode::FORBIDDEN);

    let approved = app
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/accounts/{}/drafts/{draft_id}/keep", first.account_id),
            &first.session_token,
        ))
        .await
        .expect("keep");
    assert_eq!(approved.status(), StatusCode::OK);
    let approved = response_json(approved).await;
    assert_eq!(approved["summary"]["approvedReviewUnitCount"], json!(1));
    let review_unit_id = approved["current"]["reviewUnitId"]
        .as_str()
        .expect("review unit id");
    assert_eq!(approved["current"]["expectedAnswer"], json!(null));

    assert_foreign_review_unit_is_not_found(&app, &first, &second).await;

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

async fn assert_foreign_review_unit_is_not_found(
    app: &axum::Router,
    first: &TestAccount,
    second: &TestAccount,
) {
    let source_id =
        create_source_v1(app, second, "Second account NATO notes", &source_body()).await;
    let draft_id = generate_source_v1_draft_ids(app, second, &source_id)
        .await
        .into_iter()
        .next()
        .expect("second account draft");
    let foreign_review_unit_id = keep_draft_v1(app, second, &draft_id).await;
    let foreign_review = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{foreign_review_unit_id}/reveal",
                first.account_id
            ),
            &first.session_token,
        ))
        .await
        .expect("foreign review id");
    assert_eq!(foreign_review.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(foreign_review).await["error"],
        json!("Review unit not found.")
    );
}

#[tokio::test]
async fn v1_json_api_drives_full_loop_with_bearer_token() {
    let app = router(ApiState::default());
    let account = create_account_v1(&app, "scry@example.com").await;
    let source_id = create_source_v1(&app, &account, "NATO practice notes", &source_body()).await;
    let draft_id = generate_source_v1(&app, &account, &source_id).await;
    let review_unit_id = keep_draft_v1(&app, &account, &draft_id).await;

    assert_eq!(
        next_review_v1(&app, &account).await,
        review_unit_id,
        "v1 queue/next must expose the kept review unit"
    );
    assert_eq!(
        reveal_review_v1(&app, &account, &review_unit_id).await,
        "ALFA"
    );
    assert_eq!(
        submit_review_v1(&app, &account, &review_unit_id, "ALFA").await,
        (String::from("correct"), 1)
    );
    let feedback = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "verdict": "kept",
                "rationale": "The generated card is useful.",
                "idempotencyKey": "v1-content-feedback-scry-a"
            }),
        ))
        .await
        .expect("content feedback");
    assert_eq!(feedback.status(), StatusCode::OK);
    let feedback = response_json(feedback).await;
    assert_eq!(feedback["verdict"], "kept");
    assert_eq!(feedback["source"], "human");
    assert_eq!(feedback["accountId"], account.account_id);

    let unknown = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/unknown-review-unit/content-feedback",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "verdict": "kept",
                "idempotencyKey": "v1-content-feedback-unknown"
            }),
        ))
        .await
        .expect("unknown review unit feedback");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let invalid_parent = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "verdict": "kept",
                "supersedesId": "missing-feedback",
                "idempotencyKey": "v1-content-feedback-invalid-parent"
            }),
        ))
        .await
        .expect("invalid feedback parent");
    assert_eq!(invalid_parent.status(), StatusCode::BAD_REQUEST);

    let conflicting_replay = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                account.account_id
            ),
            &account.session_token,
            &json!({
                "verdict": "dropped",
                "idempotencyKey": "v1-content-feedback-scry-a"
            }),
        ))
        .await
        .expect("conflicting feedback replay");
    assert_eq!(conflicting_replay.status(), StatusCode::CONFLICT);

    archive_source_v1(&app, &account, &source_id).await;
}

#[tokio::test]
async fn v1_source_permission_round_trips_through_http_and_file_store() {
    let app = router(ApiState::default());
    let account = create_account_v1(&app, "privacy@example.com").await;
    let response = app
        .clone()
        .oneshot(v1_json_request(
            "POST",
            &format!("/v1/accounts/{}/sources", account.account_id),
            &account.session_token,
            &json!({
                "title": "Private notes",
                "body": "Do not send this outside the local study store.",
                "permission": "local-only"
            }),
        ))
        .await
        .expect("create local-only source");
    assert_eq!(response.status(), StatusCode::CREATED);
    let source = response_json(response).await;
    assert_eq!(source["permission"], json!("local-only"));

    let listed = app
        .clone()
        .oneshot(v1_empty_request(
            "GET",
            &format!("/v1/accounts/{}/sources", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("list local-only source");
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(
        response_json(listed).await["sources"][0]["permission"],
        json!("local-only")
    );

    let source_id = source["sourceId"].as_str().expect("source id");
    let updated = app
        .clone()
        .oneshot(v1_json_request(
            "PATCH",
            &format!("/v1/accounts/{}/sources/{source_id}", account.account_id),
            &account.session_token,
            &json!({"permission": "model-eligible"}),
        ))
        .await
        .expect("update source permission");
    assert_eq!(updated.status(), StatusCode::NO_CONTENT);

    let listed = app
        .clone()
        .oneshot(v1_empty_request(
            "GET",
            &format!("/v1/accounts/{}/sources", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("list updated source");
    assert_eq!(
        response_json(listed).await["sources"][0]["permission"],
        json!("model-eligible")
    );

    archive_source_v1(&app, &account, source_id).await;
    let archived_update = app
        .oneshot(v1_json_request(
            "PATCH",
            &format!("/v1/accounts/{}/sources/{source_id}", account.account_id),
            &account.session_token,
            &json!({"permission": "local-only"}),
        ))
        .await
        .expect("archived update");
    assert_eq!(archived_update.status(), StatusCode::NOT_FOUND);
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
        .expect("pending stale deck draft")
        .to_owned();
    let review_unit_id = keep_draft_v1(&app, &account, &draft_id).await;

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

    let kept_stale_draft = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/drafts/{stale_unapproved_draft_id}/keep",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("keep stale invalidated draft");
    assert_eq!(kept_stale_draft.status(), StatusCode::OK);
    let kept_stale_draft = response_json(kept_stale_draft).await;
    assert_eq!(kept_stale_draft["current"], json!(null));
    assert_eq!(kept_stale_draft["dueCount"], json!(0));

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
    let kept_expired = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/drafts/{expired_draft_id}/keep",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("keep expired deck draft");
    assert_eq!(kept_expired.status(), StatusCode::OK);
    let kept_expired = response_json(kept_expired).await;
    assert_eq!(kept_expired["current"], json!(null));
    assert_eq!(kept_expired["dueCount"], json!(0));
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
    let review_unit_id = keep_draft_v1(&app, &account, &draft_id).await;
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
        keep_draft_v1(&app, &account, draft_id).await;
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
async fn v1_json_concept_snooze_is_authenticated_and_defers_every_member() {
    let app = router(ApiState::default());
    let account = create_account_v1(&app, "concept-snooze@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Shared NATO concept notes",
        &shared_concept_body(),
    )
    .await;
    let draft_ids = generate_source_v1_draft_ids(&app, &account, &source_id).await;
    assert_eq!(draft_ids.len(), 2);
    for draft_id in &draft_ids {
        keep_draft_v1(&app, &account, draft_id).await;
    }

    let review_unit_id = next_review_v1(&app, &account).await;
    let snoozed = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/snooze-concept",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("snooze concept");
    assert_eq!(snoozed.status(), StatusCode::OK);
    let snoozed = response_json(snoozed).await;
    assert_eq!(snoozed["current"], json!(null));
    assert_eq!(snoozed["dueCount"], json!(0));

    let after = next_review_v1_body(&app, &account).await;
    assert_eq!(after["current"], json!(null));
    assert_eq!(after["dueCount"], json!(0));
}

#[tokio::test]
async fn file_concept_snooze_rejects_stale_archived_id_without_resurrection() {
    let root = temp_store_root("stale_concept_snooze");
    let state = ApiState::new(AccountRegistry::with_store_root(root.clone()));
    let app = router(state);
    let account = create_account_v1(&app, "stale-file-concept@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Shared NATO concept notes",
        &shared_and_other_concept_body(),
    )
    .await;
    let draft_ids = generate_source_v1_draft_ids(&app, &account, &source_id).await;
    for draft_id in &draft_ids {
        keep_draft_v1(&app, &account, draft_id).await;
    }
    let stale_id = next_review_v1(&app, &account).await;

    let study_path = root.join(&account.account_id).join("study.json");
    let mut snapshot: Value =
        serde_json::from_str(&fs::read_to_string(&study_path).expect("read file study snapshot"))
            .expect("decode file study snapshot");
    let units = snapshot["reviewUnits"]
        .as_array_mut()
        .expect("review units");
    let archived = units
        .iter_mut()
        .find(|unit| unit["reviewUnitId"] == stale_id)
        .expect("stale current unit");
    archived["archivedAt"] = json!(1);
    fs::write(
        &study_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&snapshot).expect("encode snapshot")
        ),
    )
    .expect("archive stale file unit");

    let rejected = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{stale_id}/snooze-concept",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("stale file concept snooze");
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);

    let after: Value = serde_json::from_str(
        &fs::read_to_string(&study_path).expect("read file snapshot after rejection"),
    )
    .expect("decode file snapshot after rejection");
    assert_eq!(
        after["reviewUnits"].as_array().expect("review units").len(),
        3
    );
    assert!(after["reviewUnits"]
        .as_array()
        .expect("review units")
        .iter()
        .filter(|unit| unit["queue"]["conceptKey"] == "nato-letter-a")
        .all(|unit| unit["snoozedUntil"].is_null()));
    assert_eq!(
        after["reviewUnits"]
            .as_array()
            .expect("review units")
            .iter()
            .find(|unit| unit["reviewUnitId"] == stale_id)
            .expect("archived stale unit")["archivedAt"],
        json!(1)
    );
}

#[tokio::test]
async fn file_two_registries_share_account_lock_and_reject_stale_snooze() {
    let root = temp_store_root("two_registry_concept_snooze");
    let state_one = ApiState::new(AccountRegistry::with_store_root(root.clone()));
    let state_two = ApiState::new(AccountRegistry::with_store_root(root.clone()));
    let app_one = router(state_one.clone());
    let app_two = router(state_two);
    let started = app_one
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/start",
            &[("capture", &shared_and_other_concept_body())],
        ))
        .await
        .expect("start first file registry");
    let cookie = session_cookie(&started);
    let started = response_text(started).await;
    let csrf_token = html_value(&started, "csrfToken");
    let source_id = html_value(&started, "sourceId");
    generate_source_html(&app_one, &state_one, &cookie, &csrf_token, &source_id).await;
    let page = next_review_html(&app_one, &cookie, &csrf_token, "first registry").await;
    let stale_id = html_value(&page, "reviewUnitId");

    let archived = app_one
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/delete",
            &cookie,
            &[("csrfToken", &csrf_token), ("reviewUnitId", &stale_id)],
        ))
        .await
        .expect("archive through first file registry");
    assert_eq!(archived.status(), StatusCode::OK);

    let rejected = app_two
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/snooze-concept",
            &cookie,
            &[("csrfToken", &csrf_token), ("reviewUnitId", &stale_id)],
        ))
        .await
        .expect("stale snooze through second file registry");
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);

    let study_path = fs::read_dir(&root)
        .expect("read two-registry root")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("study.json"))
        .find(|path| path.exists())
        .expect("two-registry study snapshot");
    let snapshot: Value =
        serde_json::from_str(&fs::read_to_string(study_path).expect("read two-registry snapshot"))
            .expect("decode two-registry snapshot");
    assert!(snapshot["reviewUnits"]
        .as_array()
        .expect("two-registry review units")
        .iter()
        .filter(|unit| unit["queue"]["conceptKey"] == "nato-letter-a")
        .all(|unit| unit["snoozedUntil"].is_null()));
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_concept_snooze_rejects_stale_archived_id_without_partial_update() {
    let Some(database) = PostgresTestDatabase::new("stale_concept_snooze") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let app = router(state.clone());
    let account = create_account_v1(&app, "stale-postgres-concept@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Shared NATO concept notes",
        &shared_and_other_concept_body(),
    )
    .await;
    generate_source_queued(
        &state,
        &account.account_id,
        &account.session_token,
        &source_id,
        "Shared NATO concept notes",
    )
    .await;
    let stale_id = next_review_v1(&app, &account).await;
    let before = postgres_account_snapshot(&database.scoped_url, &account.account_id);

    tokio::task::block_in_place(|| {
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(&database.scoped_url)
                .expect("connect stale postgres store");
        store
            .for_account(
                memory_engine_persistence_postgres::AccountScope::new(account.account_id.clone())
                    .expect("account scope"),
            )
            .archive_review_unit(
                &memory_engine_core::ReviewUnitId::new(stale_id.clone()),
                DEFAULT_BETA_STUDY_NOW,
            )
            .expect("archive stale postgres unit");
    });

    let rejected = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!(
                "/v1/accounts/{}/review/{stale_id}/snooze-concept",
                account.account_id
            ),
            &account.session_token,
        ))
        .await
        .expect("stale postgres concept snooze");
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);

    let after = postgres_account_snapshot(&database.scoped_url, &account.account_id);
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.schedules, before.schedules);
    assert!(after
        .review_units
        .iter()
        .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-a"))
        .all(|unit| unit.snoozed_until.is_none() || unit.review_unit_id.as_str() == stale_id));
    assert!(after
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id.as_str() == stale_id)
        .expect("archived postgres stale unit")
        .archived_at
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_two_connections_archive_requested_before_concept_snooze() {
    let Some(database) = PostgresTestDatabase::new("stale_concept_two_connections") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let app = router(state.clone());
    let account = create_account_v1(&app, "stale-two-connection@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Shared NATO concept notes",
        &shared_and_other_concept_body(),
    )
    .await;
    generate_source_queued(
        &state,
        &account.account_id,
        &account.session_token,
        &source_id,
        "Shared NATO concept notes",
    )
    .await;
    let stale_id = next_review_v1(&app, &account).await;
    let before = postgres_account_snapshot(&database.scoped_url, &account.account_id);
    let (first_ready_tx, first_ready_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_url = database.scoped_url.clone();
    let first_account_id = account.account_id.clone();
    let first_stale_id = stale_id.clone();
    let first = thread::spawn(move || {
        hold_postgres_account_lock_and_archive(
            &first_url,
            &first_account_id,
            &first_stale_id,
            &first_ready_tx,
            &release_first_rx,
        );
    });
    tokio::task::block_in_place(|| first_ready_rx.recv().expect("first transaction ready"));

    let (second_started_tx, second_started_rx) = mpsc::channel();
    let database_url = database.scoped_url.clone();
    let account_id = account.account_id.clone();
    let stale_id_for_second = stale_id.clone();
    let second = thread::spawn(move || {
        second_started_tx
            .send(())
            .expect("signal second transaction start");
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(&database_url)
                .expect("connect second concurrency client");
        let mut account_store = store.for_account(
            memory_engine_persistence_postgres::AccountScope::new(account_id)
                .expect("second account scope"),
        );
        let now = live_now_ms();
        account_store.snooze_current_review_unit_concept_until(
            &stale_id_for_second,
            now,
            now + memory_engine_study::DEFAULT_SNOOZE_DEFER_MS,
        )
    });
    tokio::task::block_in_place(|| {
        second_started_rx
            .recv()
            .expect("second transaction started");
    });
    release_first_tx
        .send(())
        .expect("release first archive transaction");
    first.join().expect("join first transaction");
    let second_result = second.join().expect("join second transaction");
    assert!(
        matches!(
            &second_result,
            Err(memory_engine_persistence_postgres::PostgresStoreError::UnknownReviewUnit(id))
                if id.as_str() == stale_id
        ),
        "the requested id did not become stale: requested={stale_id}, result={second_result:?}"
    );

    let after = postgres_account_snapshot(&database.scoped_url, &account.account_id);
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.schedules, before.schedules);
    assert!(after
        .review_units
        .iter()
        .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-a"))
        .all(|unit| unit.snoozed_until.is_none()));
    assert!(after
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id.as_str() == stale_id)
        .expect("archived unit after two-connection race")
        .archived_at
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_stale_full_record_save_cannot_regress_newer_review_state() {
    let Some(database) = PostgresTestDatabase::new("stale_full_record_save") else {
        return;
    };
    let state = ApiState::new(AccountRegistry::with_postgres_url(
        database.scoped_url.clone(),
    ));
    let app = router(state.clone());
    let account = create_account_v1(&app, "stale-full-record@example.com").await;
    let source_id = create_source_v1(
        &app,
        &account,
        "Shared NATO concept notes",
        &shared_and_other_concept_body(),
    )
    .await;
    generate_source_queued(
        &state,
        &account.account_id,
        &account.session_token,
        &source_id,
        "Shared NATO concept notes",
    )
    .await;
    let requested_id = next_review_v1(&app, &account).await;
    let before = postgres_account_snapshot(&database.scoped_url, &account.account_id);

    let (stale_ready_tx, stale_ready_rx) = mpsc::channel::<()>();
    let (allow_stale_save_tx, allow_stale_save_rx) = mpsc::channel();
    let stale_url = database.scoped_url.clone();
    let stale_account_id = account.account_id.clone();
    let stale_account_id_for_thread = stale_account_id.clone();
    let requested_id_for_stale = requested_id.clone();
    let stale_writer = spawn_stale_postgres_record_writer(
        stale_url,
        stale_account_id,
        stale_account_id_for_thread,
        requested_id_for_stale,
        stale_ready_tx,
        allow_stale_save_rx,
    );
    tokio::task::block_in_place(|| {
        stale_ready_rx
            .recv()
            .expect("receive stale record before concept commit");
    });

    let (latest_prompt, latest_lifecycle) = update_and_snooze_postgres_review_unit(
        &database.scoped_url,
        &account.account_id,
        &requested_id,
    );

    allow_stale_save_tx
        .send(())
        .expect("allow stale full-record save");
    stale_writer
        .join()
        .expect("stale writer did not panic")
        .expect("stale full-record save");

    let after = postgres_account_snapshot(&database.scoped_url, &account.account_id);
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.schedules, before.schedules);
    assert!(after
        .review_units
        .iter()
        .filter(|unit| unit.queue.concept_key.as_deref() == Some("nato-letter-a"))
        .all(|unit| unit.snoozed_until.is_some()));
    let persisted_requested = after
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id.as_str() == requested_id)
        .expect("persisted requested record");
    assert_eq!(persisted_requested.prompt, latest_prompt.prompt);
    assert_eq!(
        persisted_requested.queue.lifecycle,
        latest_lifecycle.queue.lifecycle
    );
}

#[tokio::test]
async fn concept_snooze_null_and_blank_keys_are_json_400_and_hidden_from_html() {
    for (label, concept_key) in [("null", json!(null)), ("blank", json!("   "))] {
        let root = temp_store_root(&format!("concept_key_{label}"));
        let state = ApiState::new(AccountRegistry::with_store_root(root.clone()));
        let app = router(state.clone());
        let account = create_account_v1(&app, &format!("concept-key-{label}@example.com")).await;
        let source_id = create_source_v1(
            &app,
            &account,
            "One concept key edge case",
            &shared_concept_body(),
        )
        .await;
        let draft_ids = generate_source_v1_draft_ids(&app, &account, &source_id).await;
        for draft_id in &draft_ids {
            keep_draft_v1(&app, &account, draft_id).await;
        }
        let review_unit_id = next_review_v1(&app, &account).await;
        let study_path = root.join(&account.account_id).join("study.json");
        let mut snapshot: Value = serde_json::from_str(
            &fs::read_to_string(&study_path).expect("read concept-key snapshot"),
        )
        .expect("decode concept-key snapshot");
        for unit in snapshot["reviewUnits"]
            .as_array_mut()
            .expect("review units")
        {
            unit["queue"]["conceptKey"] = concept_key.clone();
        }
        fs::write(
            &study_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&snapshot).expect("encode concept-key snapshot")
            ),
        )
        .expect("write concept-key snapshot");

        let rejected = app
            .clone()
            .oneshot(v1_empty_request(
                "POST",
                &format!(
                    "/v1/accounts/{}/review/{review_unit_id}/snooze-concept",
                    account.account_id
                ),
                &account.session_token,
            ))
            .await
            .expect("concept-key JSON rejection");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(rejected).await["error"],
            json!("The active review unit must have a nonblank concept key.")
        );

        assert_html_concept_key_is_hidden(&concept_key).await;
    }
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
    let parent_id = keep_draft_v1(&app, &account, &exercise_draft_id);
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
    let bridge_draft_ids = bridged["drafts"]
        .as_array()
        .expect("bridge drafts")
        .iter()
        .filter_map(|draft| {
            let id = draft["id"].as_str()?;
            id.starts_with("bridge-").then_some(id.to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(bridge_draft_ids.len(), 2);
    assert!(
        bridged["current"].is_null(),
        "bridge candidates remain pending"
    );
    let mut bridge_id = String::new();
    for draft_id in bridge_draft_ids {
        bridge_id = keep_draft_v1(&app, &account, &draft_id).await;
    }
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
    let concept_responses = &contract["paths"]
        ["/v1/accounts/{account_id}/review/{review_unit_id}/snooze-concept"]["post"]["responses"];
    for status in ["200", "400", "403", "404", "409"] {
        assert!(
            concept_responses[status].is_object(),
            "concept snooze OpenAPI response {status} missing"
        );
    }

    assert_eq!(actual, expected);
    assert_schema_requires(
        &contract,
        "StudyCurrent",
        &["choices", "contentFeedbackHeadId"],
    );
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
    assert_schema_requires(
        &contract,
        "GenerationJob",
        &[
            "id",
            "sourceId",
            "status",
            "cardCount",
            "retryable",
            "error",
            "createdAt",
            "updatedAt",
        ],
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
            &format!("/accounts/{}/drafts/{draft_id}/keep", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("keep");
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
            &format!("/accounts/{}/drafts/{draft_id}/keep", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("keep");
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

/// Generate through the production queued workflow: enqueue the durable job
/// and drain it synchronously off the async runtime. Production
/// (Postgres-backed) states reject the direct synchronous generate route with
/// 409, so Postgres tests must set up scheduled cards through the same durable
/// job path the deployed worker uses; the helper keeps every accepted
/// draft as part of the job.
async fn generate_source_queued(
    state: &ApiState,
    account_id: &str,
    session_token: &str,
    source_id: &str,
    title: &str,
) {
    let blocking_state = state.clone();
    let account = account_id.to_owned();
    let source = source_id.to_owned();
    let title = title.to_owned();
    tokio::task::spawn_blocking(move || {
        match blocking_state.enqueue_generation_job_for_account_id(&account, &source, &title) {
            EnqueueOutcome::Started(_) | EnqueueOutcome::AlreadyInFlight(_) => {}
            EnqueueOutcome::Rejected(reason) | EnqueueOutcome::Unavailable(reason) => {
                panic!("queued generation rejected: {reason}")
            }
        }
        blocking_state.run_pending_jobs_blocking();
    })
    .await
    .expect("queued generation drain");
    let view = state
        .study_view(account_id, session_token)
        .expect("queued study view");
    for draft in view.drafts.iter().filter(|draft| {
        !draft.approved
            && draft.learner_decision.is_none()
            && draft.validation_status == GeneratedPromptValidationStatus::Accepted
    }) {
        state
            .keep_draft(account_id, session_token, &draft.id)
            .expect("keep queued generated draft");
    }
    let succeeded = state
        .jobs_for_account_id(account_id)
        .iter()
        .any(|job| job.status == crate::JobStatus::Succeeded && job.source_id == source_id);
    assert!(
        succeeded,
        "queued generation for {source_id} must succeed: {:?}",
        state.jobs_for_account_id(account_id)
    );
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

async fn keep_draft_v1(app: &axum::Router, account: &TestAccount, draft_id: &str) -> String {
    let response = app
        .clone()
        .oneshot(v1_empty_request(
            "POST",
            &format!("/v1/accounts/{}/drafts/{draft_id}/keep", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("keep draft");
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

fn json_request_with_cookie(method: &str, uri: &str, cookie: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
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
        .header("do-connecting-ip", ip)
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

async fn post_return_notification_enable(
    app: &axum::Router,
    cookie: &str,
    csrf_token: &str,
) -> String {
    let response = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/return-notifications",
            cookie,
            &[
                ("csrfToken", csrf_token),
                ("enabled", "on"),
                ("reminderEmail", "retry@example.com"),
            ],
        ))
        .await
        .expect("return notification enable");
    assert_eq!(response.status(), StatusCode::OK);
    response_text(response).await
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

fn html_values(html: &str, name: &str) -> Vec<String> {
    let marker = format!(r#"name="{name}" value=""#);
    let mut values = Vec::new();
    let mut remaining = html;
    while let Some(start) = remaining.find(&marker) {
        let value_start = start + marker.len();
        let Some(end) = remaining[value_start..].find('"') else {
            break;
        };
        let value = &remaining[value_start..value_start + end];
        if !values.iter().any(|existing| existing == value) {
            values.push(value.to_owned());
        }
        remaining = &remaining[value_start + end + 1..];
    }
    values
}

fn html_value(html: &str, name: &str) -> String {
    let marker = format!(r#"name="{name}" value=""#);
    let start = html.find(&marker).expect("field marker") + marker.len();
    let end = html[start..].find('"').expect("field end") + start;
    html[start..end].to_owned()
}

fn content_feedback_value(html: &str, name: &str) -> String {
    let section_start = html
        .find(r#"<section class="me-content-feedback""#)
        .expect("content feedback section");
    let section = &html[section_start..];
    let marker = format!(r#"name="{name}" value=""#);
    let start = section
        .find(&marker)
        .expect("content feedback field marker")
        + marker.len();
    let end = section[start..]
        .find('"')
        .expect("content feedback field end")
        + start;
    section[start..end].to_owned()
}

async fn submit_review_ok(
    app: &axum::Router,
    cookie: &str,
    csrf_token: &str,
    review_unit_id: &str,
    answer: &str,
    idempotency_key: &str,
) -> String {
    let response = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            cookie,
            &[
                ("csrfToken", csrf_token),
                ("reviewUnitId", review_unit_id),
                ("answer", answer),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", idempotency_key),
            ],
        ))
        .await
        .expect("review submit");
    assert_eq!(response.status(), StatusCode::OK);
    response_text(response).await
}

async fn submit_content_feedback_ok(
    app: &axum::Router,
    cookie: &str,
    fields: &[(&str, &str)],
) -> String {
    let response = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            cookie,
            fields,
        ))
        .await
        .expect("content feedback submit");
    assert_eq!(response.status(), StatusCode::OK);
    response_text(response).await
}

async fn submit_content_feedback_conflict(
    app: &axum::Router,
    cookie: &str,
    fields: &[(&str, &str)],
) -> String {
    let response = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            cookie,
            fields,
        ))
        .await
        .expect("conflicting content feedback submit");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    response_text(response).await
}

/// The async-model successor to `assert_keep_flow_html`: after a job drains,
/// the workspace shows a finished activity-log row (a succeeded job with a
/// card count, already scheduled for review) rather than a manual keep gate.
/// `expected_cards` pins how many cards the generation scheduled.
fn assert_activity_succeeded_html(body: &str, _expected_generated_cards: usize) {
    assert!(
        body.contains(r#"data-status="succeeded""#),
        "activity log must show a succeeded job: {body}"
    );
    assert!(
        body.contains("0 cards · pending your review"),
        "generation activity must report zero scheduled cards before learner decisions: {body}"
    );
    assert!(body.contains(r#"<ul id="me-jobs""#));
    // Generation exposes candidates for explicit learner decisions; raw
    // generation internals must not leak into learner-facing markup.
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
    // Ledger graded screen (DESIGN.md): the verdict, the answer revealed in
    // place (correct option marked), the card's meta ledger, one quiet line
    // on when it returns, and a primary Continue. Raw internals still never
    // leak.
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
    assert!(body.contains("Continue"));
    assert!(body.contains(r#"class="me-meta-ledger""#));
    assert!(body.contains("This item:"));
    assert!(body.contains("Was this generated card worth keeping?"));
    assert_not_contains_any(
        body,
        &[
            "Answer feedback",
            "Expected answer",
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
/// pinning the queue order auto-keep leaves unspecified.
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

fn retry_provider_script(store_root: &FsPath) -> String {
    let script_path = store_root.join("retry-provider.sh");
    let capture_path = store_root.join("retry-provider.tsv");
    let marker_path = store_root.join("retry-provider.failed");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\t%s\\t%s\\t%s\\n' \"$MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY\" >> \"{}\"\nif [ ! -e \"{}\" ]; then\n  touch \"{}\"\n  exit 1\nfi\nexit 0\n",
        capture_path.display(),
        marker_path.display(),
        marker_path.display(),
    );
    fs::write(&script_path, script).expect("retry provider script");
    let mut permissions = fs::metadata(&script_path)
        .expect("retry provider metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("retry provider permissions");
    script_path.to_string_lossy().into_owned()
}

fn slow_provider_script(store_root: &FsPath) -> String {
    let script_path = store_root.join("slow-provider.sh");
    let capture_path = store_root.join("slow-provider.tsv");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\t%s\\t%s\\t%s\\n' \"$MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE\" \"$MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY\" >> \"{}\"\nsleep 1\nexit 0\n",
        capture_path.display(),
    );
    fs::write(&script_path, script).expect("slow provider script");
    let mut permissions = fs::metadata(&script_path)
        .expect("slow provider metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("slow provider permissions");
    script_path.to_string_lossy().into_owned()
}

fn slow_failing_provider_script(store_root: &FsPath) -> String {
    let script_path = store_root.join("slow-failing-provider.sh");
    let marker_path = store_root.join("slow-failing-provider.started");
    let script = format!(
        "#!/bin/sh\ntouch \"{}\"\nsleep 1\nexit 1\n",
        marker_path.display()
    );
    fs::write(&script_path, script).expect("slow failing provider script");
    let mut permissions = fs::metadata(&script_path)
        .expect("slow failing provider metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("slow failing provider permissions");
    script_path.to_string_lossy().into_owned()
}

async fn prepare_postgres_due_account(state: &ApiState, email: &str) -> super::AppAccount {
    let created = state
        .create_account(email)
        .expect("Postgres recovery account");
    let account = state
        .create_browser_session(&created)
        .expect("Postgres recovery session");
    let source = state
        .save_source(
            account.account_id(),
            account.session_token(),
            &CreateSourceRequest {
                title: "Postgres recovery source".to_owned(),
                body: source_body(),
                permission: SourcePermission::ModelEligible,
            },
        )
        .expect("Postgres recovery source");
    generate_source_queued(
        state,
        account.account_id(),
        account.session_token(),
        &source.source_id,
        "Postgres recovery source",
    )
    .await;
    account
}

fn run_postgres_scheduler_contenders(
    database_url: &str,
    mailer_command: &str,
    now_fn: fn() -> i64,
) -> Vec<usize> {
    let make_state = || {
        ApiState::new(
            AccountRegistry::with_postgres_url(database_url)
                .with_clock(now_fn)
                .with_auth_config(
                    AuthConfig::allow_emails(["recovery@example.com".to_owned()])
                        .with_unsubscribe_secret("postgres-recovery-secret")
                        .with_mailer_command(mailer_command),
                ),
        )
    };
    let contender_a = make_state();
    let contender_b = make_state();
    let barrier = Arc::new(Barrier::new(2));
    let a_barrier = Arc::clone(&barrier);
    let b_barrier = Arc::clone(&barrier);
    let a = thread::spawn(move || {
        a_barrier.wait();
        contender_a
            .run_scheduled_return_notifications()
            .expect("Postgres contender A")
            .sent
    });
    let b = thread::spawn(move || {
        b_barrier.wait();
        contender_b
            .run_scheduled_return_notifications()
            .expect("Postgres contender B")
            .sent
    });
    vec![
        a.join().expect("Postgres contender A join"),
        b.join().expect("Postgres contender B join"),
    ]
}

fn assert_no_store_and_no_referrer(response: &axum::response::Response) {
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers()
            .get("referrer-policy")
            .and_then(|value| value.to_str().ok()),
        Some("no-referrer")
    );
}

async fn assert_waitlist_recovery_response(
    response: axum::response::Response,
    expected_status: StatusCode,
    submitted_email: &str,
) -> String {
    assert_eq!(response.status(), expected_status);
    assert_no_store_and_no_referrer(&response);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    let body = response_text(response).await;
    assert!(body.contains("Scry"));
    assert!(body.contains("Remember everything"));
    assert!(body.contains("Try again"));
    assert!(body.contains("Back to start"));
    assert!(body.contains(r#"action="/app/waitlist" method="post""#));
    assert!(!body.contains("{\"error\":"));
    assert!(!body.contains(submitted_email));
    let lower = body.to_ascii_lowercase();
    for forbidden in ["allowlist", "registered", "account state", "invite state"] {
        assert!(
            !lower.contains(forbidden),
            "waitlist recovery must not reveal {forbidden}: {body}"
        );
    }
    body
}

fn read_return_notification_preference(store_root: &FsPath) -> String {
    fs::read_dir(store_root)
        .expect("store root")
        .flatten()
        .find_map(|entry| fs::read_to_string(entry.path().join("return-notifications.json")).ok())
        .expect("return notification preference")
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
            &format!("/accounts/{}/drafts/{draft_id}/keep", account.account_id),
            &account.session_token,
        ))
        .await
        .expect("keep");
    assert_eq!(approved.status(), StatusCode::OK);
    let approved = response_json(approved).await;

    approved["current"]["reviewUnitId"]
        .as_str()
        .expect("review unit id")
        .to_owned()
}

#[allow(dead_code)]
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

fn shared_and_other_concept_body() -> String {
    [
        shared_concept_body(),
        [
            "Concept: NATO letter B",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for B?",
            "Answer: BRAVO",
            "Distractors: ALFA, CHARLIE",
            "Reference: The NATO phonetic alphabet word for B is BRAVO.",
        ]
        .join("\n"),
    ]
    .join("\n\n")
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

fn live_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn spawn_stale_postgres_record_writer(
    database_url: String,
    read_account_id: String,
    save_account_id: String,
    requested_id: String,
    ready: mpsc::Sender<()>,
    allow_save: mpsc::Receiver<()>,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(&database_url)
                .expect("connect stale writer");
        let mut stale_record = store
            .for_account(
                memory_engine_persistence_postgres::AccountScope::new(read_account_id)
                    .expect("stale writer account scope"),
            )
            .snapshot()
            .expect("read stale writer snapshot")
            .review_units
            .into_iter()
            .find(|unit| unit.review_unit_id.as_str() == requested_id)
            .expect("stale requested record");
        stale_record.snoozed_until = Some(live_now_ms());
        ready.send(()).expect("signal stale read");
        allow_save
            .recv()
            .expect("wait for concept commit before stale save");
        store
            .for_account(
                memory_engine_persistence_postgres::AccountScope::new(save_account_id)
                    .expect("stale save account scope"),
            )
            .save_review_unit(&stale_record)
            .map_err(|error| error.to_string())
    })
}

fn update_and_snooze_postgres_review_unit(
    database_url: &str,
    account_id: &str,
    requested_id: &str,
) -> (
    memory_engine_persistence::BetaReviewUnitRecord,
    memory_engine_persistence::BetaReviewUnitRecord,
) {
    tokio::task::block_in_place(|| {
        let mut latest_store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(database_url)
                .expect("connect mutable writer");
        let mut latest_account = latest_store.for_account(
            memory_engine_persistence_postgres::AccountScope::new(account_id.to_owned())
                .expect("mutable account scope"),
        );
        let requested_review_unit = memory_engine_core::ReviewUnitId::new(requested_id);
        let latest_prompt = latest_account
            .update_review_unit_prompt_text(&requested_review_unit, "Newer prompt wins", "ALFA")
            .expect("newer prompt update");
        let latest_lifecycle = latest_account
            .set_review_unit_lifecycle(
                &requested_review_unit,
                memory_engine_core::ReviewUnitLifecycle::ttl_expires_at(live_now_ms() + 86_400_000),
            )
            .expect("newer lifecycle update");
        let mut concept_store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(database_url)
                .expect("connect concept writer");
        let now = live_now_ms();
        concept_store
            .for_account(
                memory_engine_persistence_postgres::AccountScope::new(account_id.to_owned())
                    .expect("concept account scope"),
            )
            .snooze_current_review_unit_concept_until(
                requested_id,
                now,
                now + memory_engine_study::DEFAULT_SNOOZE_DEFER_MS,
            )
            .expect("concept writer commit");
        (latest_prompt, latest_lifecycle)
    })
}

fn hold_postgres_account_lock_and_archive(
    database_url: &str,
    account_id: &str,
    review_unit_id: &str,
    ready: &mpsc::Sender<()>,
    release: &mpsc::Receiver<()>,
) {
    let mut client = memory_engine_persistence_postgres::connect_client(database_url)
        .expect("connect first concurrency client");
    let mut transaction = client.transaction().expect("begin first transaction");
    transaction
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&account_id],
        )
        .expect("lock account for first concurrency client");
    transaction
        .execute(
            "UPDATE memory_engine_review_units
             SET archived_at_ms = $3,
                 record = jsonb_set(record, '{archivedAt}', to_jsonb($3::BIGINT), true)
             WHERE account_id = $1 AND review_unit_id = $2",
            &[&account_id, &review_unit_id, &DEFAULT_BETA_STUDY_NOW],
        )
        .expect("archive requested unit in first transaction");
    ready.send(()).expect("signal first transaction ready");
    release.recv().expect("wait to commit first transaction");
    transaction
        .commit()
        .expect("commit first archive transaction");
}

struct PostgresTestDatabase {
    admin_url: String,
    schema: String,
    scoped_url: String,
}

fn postgres_account_snapshot(
    database_url: &str,
    account_id: &str,
) -> memory_engine_persistence::BetaStoreSnapshot {
    tokio::task::block_in_place(|| {
        let mut store =
            memory_engine_persistence_postgres::PostgresStudyStore::connect(database_url)
                .expect("connect postgres snapshot store");
        store
            .for_account(
                memory_engine_persistence_postgres::AccountScope::new(account_id.to_owned())
                    .expect("account scope"),
            )
            .snapshot()
            .expect("postgres snapshot")
    })
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

#[test]
fn file_store_magic_link_consumption_is_atomic() {
    let (_, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let store_root = temp_store_root("magic-link-atomic");
    let storage = super::StudyStorage::file(store_root.clone(), test_now);
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
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default()
        .with_clock(test_now)
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
    test_clock.fetch_add(AUTH_CHALLENGE_TTL_MS + 1, Ordering::SeqCst);

    assert!(
        state.verify_magic_link(&stale_token).is_err(),
        "magic link must expire once its TTL elapses"
    );
}

#[test]
fn browser_session_is_rejected_after_it_expires() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default()
        .with_clock(test_now)
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

    test_clock.fetch_add(
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

#[test]
fn correct_answer_is_not_due_again_until_real_time_passes() {
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default().with_clock(test_now);
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
                permission: SourcePermission::default(),
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
        .keep_draft(&account.account_id, &account.session_token, &draft_id)
        .expect("keep");

    let due = state
        .next_review(&account.account_id, &account.session_token)
        .expect("next review");
    let current = due.current.expect("kept unit is due");
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

    test_clock.fetch_add(30 * 86_400_000, Ordering::SeqCst);
    let later = state
        .next_review(&account.account_id, &account.session_token)
        .expect("next review");
    assert!(
        later.current.is_some(),
        "the unit must come due again once enough real time passes"
    );
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
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default().with_clock(test_now);
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
    test_clock.fetch_add(86_400_000, Ordering::SeqCst);

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
    clock: &AtomicI64,
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
        advance_clock_past_next_review(clock, &response_text(graded).await);
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
fn advance_clock_past_next_review(clock: &AtomicI64, page: &str) {
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
    clock.fetch_add(days * 86_400_000, Ordering::SeqCst);
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
    let (test_clock, test_now) = isolated_test_clock!(DEFAULT_BETA_STUDY_NOW);
    let registry = AccountRegistry::default()
        .with_clock(test_now)
        .with_auth_config(AuthConfig::default().with_debug_links(true));
    let state = ApiState::new(registry);
    let app = router(state.clone());

    let slow = mature_cat_card_then_submit(&app, &state, test_clock, "slow", Some("6500")).await;
    assert!(slow.contains(r#"<span class="me-verdict">Correct</span>"#));
    let slow_days = next_review_days(&slow);

    let fast = mature_cat_card_then_submit(&app, &state, test_clock, "fast", Some("900")).await;
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
        let graded = mature_cat_card_then_submit(&app, &state, test_clock, label, dishonest).await;
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

#[tokio::test]
async fn review_pre_grade_is_minimal_with_collapsed_hatches() {
    // Ledger interaction law (DESIGN.md): before grading, the card shows the
    // prompt and the answer mechanism plus exactly one visible hatch (Reveal
    // answer) and one More disclosure. The other actions live inside the
    // disclosure with a capture punch-out. No card meta of any kind renders
    // pre-grade.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let page = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
    assert!(
        page.contains(r#"<details class="me-more">"#),
        "pre-grade must collapse secondary hatches behind one disclosure: {page}"
    );
    assert!(page.contains("Reveal answer"));
    for action in ["Reference", "Skip", "Snooze", "Bridge", "Delete"] {
        assert!(
            page.contains(&format!(">{action}</button>")),
            "the {action} hatch must survive inside the disclosure: {page}"
        );
    }
    assert!(
        page.contains(r#"class="me-more-capture""#),
        "the disclosure must carry the capture punch-out: {page}"
    );
    assert!(
        !page.contains(r#"class="me-hatches""#),
        "the permanent six-button hatch row is a design defect: {page}"
    );
    for meta_marker in ["me-meta-ledger", "Last seen", "last seen", "success rate"] {
        assert!(
            !page.contains(meta_marker),
            "card meta must not render pre-grade ({meta_marker}): {page}"
        );
    }
}

#[tokio::test]
async fn graded_review_shows_meta_ledger_and_holds_for_continue() {
    // Ledger interaction law (DESIGN.md), operator ruling from live dogfood
    // use (memory-engine-081): after grading the card shows its dossier
    // (stage, last seen, success rate, next horizon) and holds indefinitely
    // — correct or not, only an explicit Continue tap ever advances it. This
    // reverses the two-speed auto-advance shipped in memory-engine-078.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    // Correct free-response answer: meta ledger + auto-advance.
    let page = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;
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
        .expect("correct submit");
    assert_eq!(graded.status(), StatusCode::OK);
    let graded = response_text(graded).await;
    assert!(graded.contains(r#"<span class="me-verdict">Correct</span>"#));
    assert!(
        graded.contains(r#"class="me-meta-ledger""#),
        "graded card must show its meta ledger: {graded}"
    );
    for key in ["Stage", "Last seen", "Success"] {
        assert!(
            graded.contains(key),
            "meta ledger must carry {key}: {graded}"
        );
    }
    assert!(graded.contains("you'll see this again"));
    assert!(
        !graded.contains("data-auto-advance"),
        "a correct verdict must never auto-advance: {graded}"
    );
    assert!(
        graded.contains(">Continue"),
        "Continue must be the visible, only way to advance: {graded}"
    );

    // Wrong MCQ answer: dossier still shows, but no auto-advance — the
    // learner is studying the miss.
    let mcq = advance_to_prompt(&app, &cookie, &csrf_token, "What is the NATO phonetic").await;
    let review_unit_id = html_value(&mcq, "reviewUnitId");
    let idempotency_key = html_value(&mcq, "idempotencyKey");
    let missed = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/submit",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
                ("answer", "BRAVO"),
                ("responseTimeMs", "1500"),
                ("idempotencyKey", &idempotency_key),
            ],
        ))
        .await
        .expect("wrong submit");
    assert_eq!(missed.status(), StatusCode::OK);
    let missed = response_text(missed).await;
    assert!(missed.contains(r#"<span class="me-verdict">Try again</span>"#));
    assert!(missed.contains(r#"class="me-meta-ledger""#));
    assert!(
        !missed.contains("data-auto-advance"),
        "a miss must hold for study, never auto-advance: {missed}"
    );
    assert!(missed.contains(">Continue"));
}

#[tokio::test]
async fn every_page_serves_the_ledger_design_system() {
    // DESIGN.md: assets/ledger.css is the single stylesheet of record.
    let state = ApiState::default();
    let app = router(state.clone());

    let css = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/ledger.css")
                .body(Body::empty())
                .expect("css request"),
        )
        .await
        .expect("css response");
    assert_eq!(css.status(), StatusCode::OK);
    let css = response_text(css).await;
    assert!(css.contains("--lg-paper"), "ledger tokens must ship");
    assert!(css.contains("prefers-reduced-motion"));

    let home = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("home request"),
        )
        .await
        .expect("home");
    let home = response_text(home).await;
    assert!(
        home.contains(r#"<link rel="stylesheet" href="/static/ledger.css">"#),
        "pages must link the Ledger system: {home}"
    );
    assert!(
        !home.contains("aesthetic.css"),
        "the vendored aesthetic kit is superseded on this surface: {home}"
    );
}

#[tokio::test]
async fn generate_route_coalesces_a_duplicate_request_onto_the_in_flight_job() {
    // 082 dogfood repro: pressing "Create review" on a saved source while
    // that source's generation job is already queued/running enqueued a
    // second job (two activity rows, doubled card counts). A repeat request
    // must coalesce onto the existing job instead of duplicating it.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    let first = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("first generate");
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_text(first).await;
    assert!(first.contains("Generating. Watch the activity log."));
    assert_eq!(
        first.matches("data-job-id=\"").count(),
        1,
        "exactly one job after the first request: {first}"
    );

    // The job is still queued — nothing has drained it yet — so a second
    // press of "Create review" for the same source must coalesce, not
    // duplicate, and must surface the already-working notice.
    let second = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("second generate");
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_text(second).await;
    assert!(
        second.contains("Already generating this source."),
        "a repeat request while the job is in flight must surface the coalesce notice: {second}"
    );
    assert_eq!(
        second.matches("data-job-id=\"").count(),
        1,
        "the repeat request must not enqueue a second job: {second}"
    );

    // Draining the queue must produce cards for exactly one job's worth of
    // work, not double the count from a duplicate job.
    state.run_pending_jobs_blocking();
    let workspace = workspace_html(&app, &cookie).await;
    assert_activity_succeeded_html(&workspace, 2);
    assert_eq!(
        workspace.matches("data-job-id=\"").count(),
        1,
        "only one job must ever have existed for this source: {workspace}"
    );
}

#[tokio::test]
async fn activity_retry_control_only_renders_for_failed_jobs() {
    // Operator dogfood finding (memory-engine-081): an unstyled Retry button
    // rendered next to a RUNNING job. Retry only ever makes sense once a job
    // has actually failed.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    let queued = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("generate");
    let queued = response_text(queued).await;
    assert!(queued.contains(r#"data-status="queued""#));
    assert!(
        !queued.contains("me-job-retry-btn"),
        "a queued job must not offer Retry: {queued}"
    );

    // Archive the source so generation fails for a real reason, then confirm
    // the now-failed row does carry a styled Retry control.
    app.clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/source/archive",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("archive");
    state.run_pending_jobs_blocking();
    let failed_html = workspace_html(&app, &cookie).await;
    assert!(failed_html.contains(r#"data-status="failed""#));
    assert!(
        failed_html.contains("me-job-retry-btn"),
        "a failed job must offer a styled Retry control: {failed_html}"
    );
}

#[tokio::test]
async fn activity_glyphs_render_a_single_clean_mark_with_no_icon_overlap() {
    // Operator dogfood finding (memory-engine-081): the success glyph was a
    // solid pine dot with an awkwardly overlapping checkmark icon. Every job
    // glyph is a single flat status dot driven by CSS off `data-status` — no
    // icon layered inside it.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    assert_activity_succeeded_html(&generated, 2);
    assert!(
        generated.contains(r#"<span class="g-succeeded"></span>"#),
        "the succeeded glyph must be a single bare dot: {generated}"
    );
    assert!(
        !generated.contains(r#"class="g-succeeded"><svg"#),
        "no icon may render inside the succeeded glyph: {generated}"
    );
}

#[tokio::test]
async fn capture_form_progressive_enhancement_shows_a_pending_state() {
    // Ruling (memory-engine-081): Create must show an immediate in-page
    // pending state — the submit button disables and its label swaps to a
    // working state — via progressive enhancement. JS-off keeps the plain
    // form post: the server always renders the button enabled with its real
    // label, so the enhancement is additive only.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, _csrf_token, _source_id) = start_app_session_for_csrf(&app).await;

    let workspace = workspace_html(&app, &cookie).await;
    assert!(
        workspace.contains(r#"<form class="me-capture-form" action="/app/capture" method="post">"#),
        "the capture form needs a stable selector for the pending-state enhancement: {workspace}"
    );
    assert!(workspace.contains(">Create"));

    let script = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/app.js")
                .body(Body::empty())
                .expect("app.js request"),
        )
        .await
        .expect("app.js response");
    let script = response_text(script).await;
    assert!(
        script.contains("me-capture-form"),
        "app.js must target the capture form: {script}"
    );
    assert!(
        script.contains("Creating\u{2026}"),
        "app.js must swap the button label to a pending state: {script}"
    );
    assert!(
        script.contains(".disabled = true"),
        "app.js must disable the submit button during the pending state: {script}"
    );
}

#[tokio::test]
async fn saved_material_hides_generate_once_a_job_is_in_flight_or_done() {
    // Operator dogfood finding (memory-engine-081): tapping "Create review"
    // while a job for the same source was already running caused a
    // duplicate generation run. Saved material never offers to generate
    // again once a job for that source is queued, running, or already
    // succeeded, and the action explains itself.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;

    let fresh = workspace_html(&app, &cookie).await;
    assert!(
        fresh.contains("Generate cards"),
        "a source with no job yet must offer to generate: {fresh}"
    );
    assert!(
        !fresh.contains("Create review"),
        "the action must be relabeled to explain itself: {fresh}"
    );

    let queued = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/generate",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("generate");
    let queued = response_text(queued).await;
    assert!(queued.contains(r#"data-status="queued""#));
    assert!(
        !queued.contains("Generate cards"),
        "a source with a job already queued must not offer to generate again: {queued}"
    );

    state.run_pending_jobs_blocking();
    let succeeded = workspace_html(&app, &cookie).await;
    assert!(succeeded.contains(r#"data-status="succeeded""#));
    assert!(
        !succeeded.contains("Generate cards"),
        "a source that already succeeded must not offer to generate again: {succeeded}"
    );
    assert!(succeeded.contains("Remove"));
}

#[tokio::test]
async fn more_sheet_actions_carry_icons_and_truthful_tooltips() {
    // Operator dogfood finding (memory-engine-081): More-sheet actions were
    // unclear without tooltips, and Skip vs Snooze read as interchangeable.
    // Tooltips must be truthful to the actual route semantics: Skip
    // (`DEFAULT_SKIP_DEFER_MS`) is a short in-session deferral, Snooze
    // (`DEFAULT_SNOOZE_DEFER_MS`) defers until tomorrow.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    let page = advance_to_prompt(&app, &cookie, &csrf_token, "Spell CAT over the phone").await;

    let tooltips = [
        "Show background reading for this card.",
        "Show later this session.",
        "Hide until tomorrow.",
        "Hide every card for this concept until tomorrow.",
        "Generate easier warm-up cards, then revisit this one later.",
        "Remove this card from review for good.",
        "Capture new material without leaving review.",
    ];
    for tooltip in tooltips {
        let marker = format!(r#"title="{tooltip}"><svg class="ae-icon""#);
        assert!(
            page.contains(&marker),
            "expected a truthful tooltip with a leading icon ({tooltip}): {page}"
        );
    }

    let review_unit_id = html_value(&page, "reviewUnitId");
    let concept_snoozed = app
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/snooze-concept",
            &cookie,
            &[
                ("csrfToken", &csrf_token),
                ("reviewUnitId", &review_unit_id),
            ],
        ))
        .await
        .expect("snooze concept");
    assert_eq!(concept_snoozed.status(), StatusCode::OK);
}

#[tokio::test]
async fn served_assets_carry_the_instant_acknowledgment_enhancement() {
    // memory-engine-086: the review actions acknowledge a press before the
    // server responds. The behavior itself is client-side; this tripwire
    // pins that the served assets actually carry the enhancement hooks so a
    // refactor cannot silently drop them.
    let app = router(ApiState::default());

    let js = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/app.js")
                .body(Body::empty())
                .expect("js request"),
        )
        .await
        .expect("js response");
    assert_eq!(js.status(), StatusCode::OK);
    let js = response_text(js).await;
    for hook in ["data-busy", "data-pressed", "data-dim", "event.submitter"] {
        assert!(js.contains(hook), "app.js must carry {hook}");
    }
    assert!(
        !js.contains("control.disabled = true"),
        "the submitter must never be disabled pre-post (it would strip the MCQ answer value)"
    );

    let css = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/ledger.css")
                .body(Body::empty())
                .expect("css request"),
        )
        .await
        .expect("css response");
    let css = response_text(css).await;
    for hook in ["data-pressed", "data-dim", "html[data-busy]"] {
        assert!(css.contains(hook), "ledger.css must style {hook}");
    }
}

#[tokio::test]
async fn saved_material_remove_discloses_scope_before_the_tap() {
    // memory-engine-088: the operator dogfood found Remove was a bare,
    // unlabeled single-tap button archiving a source and every card
    // generated from it (across every generation run) with zero warning.
    // The control must now be a disclosure that states that truthfully
    // before the destructive submit is reachable.
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;

    let workspace = workspace_html(&app, &cookie).await;
    assert!(
        workspace.contains(r#"<details class="me-remove-confirm">"#),
        "Remove must be a disclosure, not a bare button: {workspace}"
    );
    assert!(
        workspace.contains("every card generated from it")
            && workspace.contains("every generation run"),
        "the disclosure must truthfully state the destructive scope: {workspace}"
    );
    // The confirming submit is the one that actually archives; a bare
    // "Remove" label alone (outside the disclosure) must not exist as a
    // reachable submit control.
    assert!(
        workspace.contains("Remove permanently"),
        "the confirming action must be explicit, not a repeat of the bare label: {workspace}"
    );
}

#[tokio::test]
async fn saved_material_remove_reports_how_many_cards_were_retired() {
    // memory-engine-088: after archiving, the notice must name the actual
    // count of cards retired, not a generic "Source removed."
    let state = ApiState::default();
    let app = router(state.clone());
    let (cookie, csrf_token, source_id) = start_app_session_for_csrf(&app).await;
    let generated = generate_source_html(&app, &state, &cookie, &csrf_token, &source_id).await;
    // The NATO fixture schedules 2 review units.
    assert_activity_succeeded_html(&generated, 2);

    let archived = app
        .clone()
        .oneshot(form_request_with_cookie(
            "POST",
            "/app/source/archive",
            &cookie,
            &[("csrfToken", &csrf_token), ("sourceId", &source_id)],
        ))
        .await
        .expect("archive");
    assert_eq!(archived.status(), StatusCode::OK);
    let archived = response_text(archived).await;
    assert!(
        archived.contains("2 cards retired"),
        "the notice must name the actual retired count: {archived}"
    );
    assert!(
        !archived.contains("Source removed.</p>") || archived.contains("2 cards retired"),
        "a generic notice with no count no longer satisfies the disclosure requirement: {archived}"
    );
}

// --- memory-engine-beta-waitlist: invite-beta waitlist first-run slice ---

#[tokio::test]
async fn waitlist_join_persists_entry_creates_no_session_and_is_reachable_by_admin() {
    let store_root = temp_store_root("waitlist-join-persist");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let joined = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "new-here@example.com")],
        ))
        .await
        .expect("waitlist join");
    assert_eq!(joined.status(), StatusCode::OK);
    assert!(joined.headers().get(SET_COOKIE).is_none());
    let joined = response_text(joined).await;
    assert!(joined.contains("Thanks for joining."));

    let admin_request = Request::builder()
        .method("GET")
        .uri("/internal/waitlist")
        .header("x-admin-token", "op-token")
        .body(Body::empty())
        .expect("admin request");
    let listed = app.oneshot(admin_request).await.expect("admin list");
    assert_eq!(listed.status(), StatusCode::OK);
    let entries = response_json(listed).await;
    let entries = entries.as_array().expect("waitlist array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["email"], json!("new-here@example.com"));
    assert_eq!(entries[0]["source"], json!("first-run"));
    assert!(entries[0]["invitedAtMs"].is_null());
}

#[tokio::test]
async fn waitlist_join_is_idempotent_for_duplicate_normalized_email() {
    let store_root = temp_store_root("waitlist-join-idempotent");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let first = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "Repeat@Example.com")],
        ))
        .await
        .expect("first join");
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_text(first).await;

    let second = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "repeat@example.com")],
        ))
        .await
        .expect("second join");
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_text(second).await;

    assert_eq!(
        first, second,
        "a repeat join must read identically to the first"
    );

    let admin_request = Request::builder()
        .method("GET")
        .uri("/internal/waitlist")
        .header("x-admin-token", "op-token")
        .body(Body::empty())
        .expect("admin request");
    let listed = app.oneshot(admin_request).await.expect("admin list");
    let entries = response_json(listed).await;
    let entries = entries.as_array().expect("waitlist array");
    assert_eq!(
        entries.len(),
        1,
        "normalized duplicates must collapse to one row"
    );
    assert_eq!(entries[0]["email"], json!("repeat@example.com"));
}

#[tokio::test]
async fn waitlist_join_does_not_reveal_allowlist_or_account_state() {
    let store_root = temp_store_root("waitlist-join-enumeration");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root).with_auth_config(
            AuthConfig::allow_emails(["allowlisted@example.com".to_owned()])
                .with_admin_token("op-token"),
        ),
    ));

    let allowlisted = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "allowlisted@example.com")],
        ))
        .await
        .expect("allowlisted join");
    assert_eq!(allowlisted.status(), StatusCode::OK);
    assert!(allowlisted.headers().get(SET_COOKIE).is_none());
    let allowlisted = response_text(allowlisted).await;

    let stranger = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "stranger@example.com")],
        ))
        .await
        .expect("stranger join");
    assert_eq!(stranger.status(), StatusCode::OK);
    assert!(stranger.headers().get(SET_COOKIE).is_none());
    let stranger = response_text(stranger).await;

    assert_eq!(
        allowlisted, stranger,
        "an allowlisted address and a stranger must get the identical response"
    );
    assert!(!allowlisted.contains("allowlisted@example.com"));
    assert!(!stranger.contains("stranger@example.com"));

    let admin_request = Request::builder()
        .method("GET")
        .uri("/internal/waitlist")
        .header("x-admin-token", "op-token")
        .body(Body::empty())
        .expect("admin request");
    let listed = app.oneshot(admin_request).await.expect("admin list");
    let entries = response_json(listed).await;
    assert_eq!(entries.as_array().expect("waitlist array").len(), 2);
}

#[tokio::test]
async fn waitlist_join_rejects_malformed_email_without_persisting() {
    let store_root = temp_store_root("waitlist-join-malformed");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let rejected = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "not-an-email")],
        ))
        .await
        .expect("malformed join");
    let rejected_body =
        assert_waitlist_recovery_response(rejected, StatusCode::BAD_REQUEST, "not-an-email").await;
    assert!(!rejected_body.to_ascii_lowercase().contains("allowlist"));

    let admin_request = Request::builder()
        .method("GET")
        .uri("/internal/waitlist")
        .header("x-admin-token", "op-token")
        .body(Body::empty())
        .expect("admin request");
    let listed = app.oneshot(admin_request).await.expect("admin list");
    let entries = response_json(listed).await;
    assert!(entries.as_array().expect("waitlist array").is_empty());
}

#[tokio::test]
async fn waitlist_storage_failure_renders_branded_503_without_leaking_email() {
    let store_root = temp_store_root("waitlist-storage-failure");
    fs::create_dir_all(&store_root).expect("storage failure root");
    // A directory at the waitlist file path makes the real file store return an
    // I/O error without mocking the storage boundary or touching unrelated state.
    fs::create_dir_all(store_root.join("_waitlist.json")).expect("blocking waitlist path");
    let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));

    let response = app
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "storage-failure@example.com")],
        ))
        .await
        .expect("storage failure response");

    let body = assert_waitlist_recovery_response(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage-failure@example.com",
    )
    .await;
    assert!(!body.contains("Is a directory"));
    assert!(!body.contains("waitlist store"));
}

#[tokio::test]
async fn waitlist_rate_limits_by_email_and_ip() {
    let store_root = temp_store_root("waitlist-rate-limit");
    let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));

    for _ in 0..WAITLIST_RATE_LIMIT_MAX_ATTEMPTS {
        let response = app
            .clone()
            .oneshot(form_request_with_ip(
                "POST",
                "/app/waitlist",
                "203.0.113.30",
                &[("email", "quota@example.com")],
            ))
            .await
            .expect("allowed join");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let same_email_new_ip = app
        .clone()
        .oneshot(form_request_with_ip(
            "POST",
            "/app/waitlist",
            "203.0.113.31",
            &[("email", "quota@example.com")],
        ))
        .await
        .expect("email limited");
    let _ = assert_waitlist_recovery_response(
        same_email_new_ip,
        StatusCode::TOO_MANY_REQUESTS,
        "quota@example.com",
    )
    .await;

    let same_ip_new_email = app
        .oneshot(form_request_with_ip(
            "POST",
            "/app/waitlist",
            "203.0.113.30",
            &[("email", "other-quota@example.com")],
        ))
        .await
        .expect("ip limited");
    assert_eq!(same_ip_new_email.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn waitlist_admin_listing_requires_a_valid_admin_token() {
    let store_root = temp_store_root("waitlist-admin-gate");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/internal/waitlist")
                .body(Body::empty())
                .expect("no-token request"),
        )
        .await
        .expect("missing token response");
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    let missing_body = response_json(missing).await;
    let missing_error = missing_body["error"].as_str().expect("error message");
    assert!(
        missing_error.to_ascii_lowercase().contains("admin token"),
        "error must name admin-token authorization: {missing_error}"
    );
    assert!(
        !missing_error
            .to_ascii_lowercase()
            .contains("session issuance"),
        "error must not misdirect to service-session issuance: {missing_error}"
    );

    let wrong = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/internal/waitlist")
                .header("x-admin-token", "not-the-token")
                .body(Body::empty())
                .expect("wrong-token request"),
        )
        .await
        .expect("wrong token response");
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn waitlist_export_neutralizes_formula_leading_locals_and_quotes_carriage_returns() {
    let store_root = temp_store_root("waitlist-export-csv-injection");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let dangerous_emails = [
        "=2+5+cmd|'/c calc'!a0@example.com",
        "+1+1@example.com",
        "-1+1@example.com",
        "cr\rinjected@example.com",
    ];
    for email in dangerous_emails {
        let joined = app
            .clone()
            .oneshot(form_request("POST", "/app/waitlist", &[("email", email)]))
            .await
            .expect("join");
        assert_eq!(joined.status(), StatusCode::OK);
    }

    // JSON keeps the raw, unmodified email: only the CSV encoding changes.
    let listed = app
        .clone()
        .oneshot(waitlist_admin_request("GET", "/internal/waitlist", None))
        .await
        .expect("admin list");
    let entries = response_json(listed).await;
    let entries = entries.as_array().expect("waitlist array");
    for email in dangerous_emails {
        assert!(
            entries.iter().any(|entry| entry["email"] == json!(email)),
            "JSON must preserve the raw email {email:?} verbatim"
        );
    }

    let exported = app
        .oneshot(waitlist_admin_request(
            "GET",
            "/internal/waitlist/export",
            None,
        ))
        .await
        .expect("export");
    let exported = response_text(exported).await;

    // A formula-leading local part must be neutralized with a stable
    // spreadsheet-safe prefix so opening the CSV cannot execute it.
    assert!(
        exported.contains("'=2+5+cmd|'/c calc'!a0@example.com,"),
        "= must be neutralized with a leading prefix: {exported}"
    );
    assert!(
        exported.contains("'+1+1@example.com,"),
        "+ must be neutralized with a leading prefix: {exported}"
    );
    assert!(
        exported.contains("'-1+1@example.com,"),
        "- must be neutralized with a leading prefix: {exported}"
    );
    // A bare CR (not just LF) must still trigger RFC 4180 quoting so the
    // row isn't corrupted by an unescaped control character.
    assert!(
        exported.contains("\"cr\rinjected@example.com\","),
        "embedded CR must be quoted: {exported}"
    );
    // The raw, un-neutralized formula must never appear at the start of a
    // CSV cell (i.e. right after a newline or the header terminator).
    assert!(
        !exported.contains("\n=2+5"),
        "raw formula must never open a CSV cell unguarded: {exported}"
    );
}

#[tokio::test]
async fn waitlist_home_page_offers_both_signin_and_join_actions() {
    let home = router(ApiState::default())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .body(Body::empty())
                .expect("home request"),
        )
        .await
        .expect("home response");
    assert_eq!(home.status(), StatusCode::OK);
    let home = response_text(home).await;
    assert!(home.contains(r#"action="/app/account""#));
    assert!(home.contains("Get started"));
    assert!(home.contains(r#"action="/app/waitlist""#));
    assert!(home.contains("Join the waitlist"));
}

#[tokio::test(flavor = "multi_thread")]
async fn waitlist_postgres_backed_registry_joins_and_lists_durably() {
    let Some(database) = PostgresTestDatabase::new("waitlist_join") else {
        return;
    };
    let state = ApiState::new(
        AccountRegistry::with_postgres_url(database.scoped_url.clone())
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    );
    let app = router(state);

    // Joining against a Postgres-backed registry now persists durably; the
    // production path no longer fails closed with 503.
    let joined = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "postgres-learner@example.com")],
        ))
        .await
        .expect("postgres join");
    assert_eq!(joined.status(), StatusCode::OK);
    let joined = response_text(joined).await;
    assert!(joined.contains("Thanks for joining."));

    // A duplicate join stays idempotent through the Postgres path too.
    let duplicate = app
        .clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "postgres-learner@example.com")],
        ))
        .await
        .expect("postgres duplicate join");
    assert_eq!(duplicate.status(), StatusCode::OK);

    let listed = app
        .clone()
        .oneshot(waitlist_admin_request("GET", "/internal/waitlist", None))
        .await
        .expect("admin list");
    assert_eq!(listed.status(), StatusCode::OK);
    let entries = response_json(listed).await;
    let entries = entries.as_array().expect("waitlist array");
    assert_eq!(
        entries.len(),
        1,
        "normalized duplicates must collapse to one row"
    );
    assert_eq!(entries[0]["email"], json!("postgres-learner@example.com"));
    assert!(entries[0]["invitedAtMs"].is_null());

    let exported = app
        .oneshot(waitlist_admin_request(
            "GET",
            "/internal/waitlist/export",
            None,
        ))
        .await
        .expect("admin export");
    assert_eq!(exported.status(), StatusCode::OK);
    let exported = response_text(exported).await;
    assert!(exported.starts_with("email,createdAtMs,updatedAtMs,source,invitedAtMs\n"));
    assert!(exported.contains("postgres-learner@example.com"));
}

#[tokio::test(flavor = "multi_thread")]
async fn waitlist_postgres_backed_registry_supports_invite_and_delete() {
    let Some(database) = PostgresTestDatabase::new("waitlist_invite_delete") else {
        return;
    };
    let state = ApiState::new(
        AccountRegistry::with_postgres_url(database.scoped_url.clone())
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    );
    let app = router(state);
    app.clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "postgres-learner@example.com")],
        ))
        .await
        .expect("postgres join");

    let invited = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/invite",
            Some(r#"{"email":"postgres-learner@example.com"}"#),
        ))
        .await
        .expect("admin invite");
    assert_eq!(invited.status(), StatusCode::OK);
    let invited = response_json(invited).await;
    assert!(invited["invitedAtMs"].is_number());

    // Inviting again is idempotent: the timestamp does not move.
    let invited_again = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/invite",
            Some(r#"{"email":"postgres-learner@example.com"}"#),
        ))
        .await
        .expect("admin invite again");
    let invited_again = response_json(invited_again).await;
    assert_eq!(invited["invitedAtMs"], invited_again["invitedAtMs"]);

    let deleted = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/delete",
            Some(r#"{"email":"postgres-learner@example.com"}"#),
        ))
        .await
        .expect("admin delete");
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted = response_json(deleted).await;
    assert_eq!(deleted["deleted"], json!(true));

    let listed_after_delete = app
        .oneshot(waitlist_admin_request("GET", "/internal/waitlist", None))
        .await
        .expect("admin list after delete");
    let entries = response_json(listed_after_delete).await;
    assert!(
        entries.as_array().expect("waitlist array").is_empty(),
        "delete must remove the operational row"
    );
}

/// Build an admin-token-gated request. `body` is `None` for a `GET`
/// (no request body); `Some(json)` sends a JSON body with the matching
/// content-type header for the `POST` mutation routes.
fn waitlist_admin_request(method: &str, uri: &str, body: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-admin-token", "op-token");
    match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_owned()))
            .expect("admin json request"),
        None => builder.body(Body::empty()).expect("admin request"),
    }
}

#[tokio::test]
async fn waitlist_join_file_store_handler_overhead_is_negligible() {
    let store_root = temp_store_root("waitlist-latency");
    let app = router(ApiState::new(AccountRegistry::with_store_root(&store_root)));

    // Regression guard only -- NOT proof of the "acknowledges valid input
    // before durable completion" production criterion. This measures the
    // file-store join handler's own floor latency (one local flock + fsync'd
    // write, no network, no model inference). The Postgres-backed production
    // path connects and runs its migration on every call (observed
    // 161-700ms TTFB), so a same-process sub-100ms *server* round trip is
    // not achievable there, and this test must not be read as one. The
    // acknowledgment guarantee production actually relies on is client-side:
    // app.js synchronously flips the waitlist button/aria-live status to
    // "Joining…" on submit, before the native POST's response ever lands
    // (proven by the Bun DOM contract in app_js_contract.test.js). This test
    // only guards against the file store itself regressing into a
    // bottleneck. A single sample is noisy under a shared test runner
    // (first-poll scheduling, concurrent-test disk contention), so this
    // takes the minimum of several samples: genuine systemic slowness (a
    // stray network round trip, a retry loop) would slow down every sample,
    // including the fastest, while one-off scheduler jitter only slows a
    // subset. Stays under `WAITLIST_RATE_LIMIT_MAX_ATTEMPTS` (5, shared by
    // IP) on a dedicated IP so the limiter never interferes.
    let sample_count = (WAITLIST_RATE_LIMIT_MAX_ATTEMPTS - 1) as usize;
    let mut samples = Vec::with_capacity(sample_count);
    for index in 0..sample_count {
        let started = Instant::now();
        let joined = app
            .clone()
            .oneshot(form_request_with_ip(
                "POST",
                "/app/waitlist",
                "203.0.113.99",
                &[("email", &format!("fast-ack-{index}@example.com"))],
            ))
            .await
            .expect("waitlist join");
        samples.push(started.elapsed());
        assert_eq!(joined.status(), StatusCode::OK);
    }
    let fastest = samples.iter().min().copied().expect("at least one sample");
    assert!(
        fastest < Duration::from_millis(100),
        "fastest of {} waitlist joins took {fastest:?}, want under 100ms; all samples: {samples:?}",
        samples.len(),
    );
}

#[tokio::test]
async fn waitlist_admin_can_export_invite_and_delete_through_the_file_store() {
    let store_root = temp_store_root("waitlist-admin-lifecycle");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    app.clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "operator-flow@example.com")],
        ))
        .await
        .expect("join");

    let exported = app
        .clone()
        .oneshot(waitlist_admin_request(
            "GET",
            "/internal/waitlist/export",
            None,
        ))
        .await
        .expect("export");
    assert_eq!(exported.status(), StatusCode::OK);
    assert_eq!(
        exported
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/csv; charset=utf-8")
    );
    let exported = response_text(exported).await;
    let mut lines = exported.lines();
    assert_eq!(
        lines.next(),
        Some("email,createdAtMs,updatedAtMs,source,invitedAtMs")
    );
    let row = lines.next().expect("one exported row");
    assert!(row.starts_with("operator-flow@example.com,"));
    assert!(row.ends_with(",first-run,"), "row: {row}");

    let invited = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/invite",
            Some(r#"{"email":"Operator-Flow@Example.com"}"#),
        ))
        .await
        .expect("invite");
    assert_eq!(invited.status(), StatusCode::OK);
    let invited = response_json(invited).await;
    assert_eq!(invited["email"], json!("operator-flow@example.com"));
    assert!(invited["invitedAtMs"].is_number());

    let deleted = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/delete",
            Some(r#"{"email":"operator-flow@example.com"}"#),
        ))
        .await
        .expect("delete");
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(response_json(deleted).await["deleted"], json!(true));

    let listed = app
        .oneshot(waitlist_admin_request("GET", "/internal/waitlist", None))
        .await
        .expect("list after delete");
    assert!(response_json(listed)
        .await
        .as_array()
        .expect("waitlist array")
        .is_empty());
}

#[tokio::test]
async fn waitlist_admin_invite_and_delete_reject_an_unknown_email() {
    let store_root = temp_store_root("waitlist-admin-unknown");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));

    let invited = app
        .clone()
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/invite",
            Some(r#"{"email":"nobody@example.com"}"#),
        ))
        .await
        .expect("invite unknown");
    assert_eq!(invited.status(), StatusCode::NOT_FOUND);

    let deleted = app
        .oneshot(waitlist_admin_request(
            "POST",
            "/internal/waitlist/delete",
            Some(r#"{"email":"nobody@example.com"}"#),
        ))
        .await
        .expect("delete unknown");
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn waitlist_admin_export_invite_and_delete_require_a_valid_admin_token() {
    let store_root = temp_store_root("waitlist-admin-gate-mutations");
    let app = router(ApiState::new(
        AccountRegistry::with_store_root(&store_root)
            .with_auth_config(AuthConfig::default().with_admin_token("op-token")),
    ));
    app.clone()
        .oneshot(form_request(
            "POST",
            "/app/waitlist",
            &[("email", "gate-check@example.com")],
        ))
        .await
        .expect("join");

    for (method, uri, body) in [
        ("GET", "/internal/waitlist/export", None),
        (
            "POST",
            "/internal/waitlist/invite",
            Some(r#"{"email":"gate-check@example.com"}"#),
        ),
        (
            "POST",
            "/internal/waitlist/delete",
            Some(r#"{"email":"gate-check@example.com"}"#),
        ),
    ] {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = match body {
            Some(json) => builder.body(Body::from(json.to_owned())),
            None => builder.body(Body::empty()),
        }
        .expect("ungated request");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("ungated response");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} must require the admin token"
        );
    }
}
