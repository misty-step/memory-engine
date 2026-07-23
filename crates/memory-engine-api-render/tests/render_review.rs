use std::sync::atomic::{AtomicUsize, Ordering};

use memory_engine_api_render::render_account_page;
use memory_engine_api_state::{ApiState, CreateSourceRequest, EnqueueOutcome, SourcePermission};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique_email(label: &str) -> String {
    let serial = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("render-085-{label}-{serial}@example.com")
}

fn nato_source_body() -> String {
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
fn active_review_render_skips_workspace_material() {
    let state = ApiState::default();
    let created = state
        .create_account(&unique_email("active"))
        .expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    let source = state
        .save_app_source(
            &account,
            &CreateSourceRequest {
                title: "NATO practice notes".to_owned(),
                body: nato_source_body(),
                permission: SourcePermission::default(),
            },
        )
        .expect("source");
    assert!(matches!(
        state.enqueue_generation_job_by_source(&account, &source.source_id, &source.title),
        EnqueueOutcome::Started(_)
    ));
    state.run_pending_jobs_blocking();

    let view = state.next_app_review(&account).expect("next review");
    assert!(
        view.current.is_some(),
        "fixture must reach an active review"
    );
    let html = render_account_page(&state, &account, Some(&view), None);

    assert!(!html.contains("Saved material"));
    assert!(!html.contains("NATO practice notes"));
    assert!(!html.contains("Generate cards"));
    assert!(html.contains("Reveal answer"));
}

#[test]
fn workspace_render_keeps_saved_material_without_active_review() {
    let state = ApiState::default();
    let created = state
        .create_account(&unique_email("workspace"))
        .expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    state
        .save_app_source(
            &account,
            &CreateSourceRequest {
                title: "NATO practice notes".to_owned(),
                body: nato_source_body(),
                permission: SourcePermission::default(),
            },
        )
        .expect("source");

    let html = render_account_page(&state, &account, None, None);
    assert!(html.contains("Saved material"));
    assert!(html.contains("NATO practice notes"));
}

#[test]
fn workspace_discloses_local_only_source_permission() {
    let state = ApiState::default();
    let created = state
        .create_account(&unique_email("local-only"))
        .expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    state
        .save_app_source(
            &account,
            &CreateSourceRequest {
                title: "Private notes".to_owned(),
                body: "Never send this text to a model.".to_owned(),
                permission: SourcePermission::LocalOnly,
            },
        )
        .expect("source");

    let html = render_account_page(&state, &account, None, None);
    assert!(html.contains("Local only · never sent to a model"));
}
