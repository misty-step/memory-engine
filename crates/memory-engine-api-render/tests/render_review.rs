use std::sync::atomic::{AtomicUsize, Ordering};

use memory_engine_api_render::{
    render_account_page, render_content_feedback_result_html, render_edit_review_html,
    render_library_page,
};
use memory_engine_api_state::{
    AccountRegistry, ApiState, AuthConfig, CreateSourceRequest, EnqueueOutcome, SourcePermission,
    StudyViewResponse,
};
use memory_engine_study::BetaStudySummary;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn render_test_state(email: &str) -> ApiState {
    ApiState::new(AccountRegistry::default().with_auth_config(
        AuthConfig::allow_emails([email.to_owned()]).with_anonymous_account_creation(true),
    ))
}

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
    let email = unique_email("active");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
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

    let pending = state.next_app_review(&account).expect("pending review");
    let view = state
        .keep_draft(
            account.account_id(),
            account.session_token(),
            &pending.drafts[0].id,
        )
        .expect("keep review");
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
fn pending_mcq_draft_shows_every_choice_and_distractor_fields() {
    let email = unique_email("pending-mcq");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
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

    let view = state.app_study_view(&account).expect("pending drafts");
    assert!(
        view.drafts.iter().any(|draft| !draft.choices.is_empty()),
        "NATO fixture must produce an MCQ draft: {:?}",
        view.drafts
            .iter()
            .map(|draft| (&draft.prompt, &draft.answer, &draft.choices))
            .collect::<Vec<_>>()
    );
    let html = render_account_page(&state, &account, Some(&view), None);

    assert!(
        html.contains("BRAVO"),
        "pending MCQ must show distractors before Keep: {html}"
    );
    assert!(
        html.contains("CHARLIE"),
        "pending MCQ must show every choice before Keep: {html}"
    );
    assert!(
        html.contains(r#"name="choices""#),
        "edit form must expose distractor fields: {html}"
    );
    assert!(
        html.contains("Keep as written"),
        "Keep as written must stay one tap: {html}"
    );
}

#[test]
fn edit_cancel_returns_to_in_progress_review() {
    let email = unique_email("edit-cancel");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
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
    let pending = state.next_app_review(&account).expect("pending review");
    let view = state
        .keep_draft(
            account.account_id(),
            account.session_token(),
            &pending.drafts[0].id,
        )
        .expect("keep review");
    assert!(view.current.is_some(), "kept draft must open review");

    let html = render_edit_review_html(&state, &account, &view, None);
    assert!(
        html.contains(r#"action="/app/next""#),
        "Cancel must restore the in-progress review: {html}"
    );
    assert!(
        html.contains(">Cancel</button>"),
        "Cancel must stay a tap, not a Home link: {html}"
    );
    assert!(
        !html.contains(r#"href="/">Cancel</a>"#),
        "Cancel must not dump the learner on Home: {html}"
    );
}

#[test]
fn library_render_keeps_saved_material_without_active_review() {
    let email = unique_email("library");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
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

    let html = render_library_page(&state, &account, None, None);
    assert!(html.contains("Saved material"));
    assert!(html.contains("NATO practice notes"));
}

#[test]
fn completed_feedback_action_requires_an_explicit_workspace_exit() {
    let email = unique_email("complete");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
    let account = state
        .create_browser_session(&created)
        .expect("browser session");
    let view = StudyViewResponse {
        drafts: Vec::new(),
        current: None,
        concept_progress: Vec::new(),
        summary: BetaStudySummary {
            source_count: 1,
            accepted_draft_count: 1,
            approved_review_unit_count: 1,
            attempt_count: 1,
            last_outcome: None,
            next_review_unit_id: None,
        },
        due_count: 0,
        generation_notices: Vec::new(),
        library: Vec::new(),
    };

    let html = render_content_feedback_result_html(&state, &account, &view, "Saved.");

    assert!(html.contains("Review complete"));
    assert!(html.contains(r#"href="/">Back to workspace</a>"#));
    assert!(!html.contains("What do you want to remember?"));
    assert!(!html.contains("Return gently"));
}

#[test]
fn library_discloses_local_only_source_permission() {
    let email = unique_email("local-only");
    let state = render_test_state(&email);
    let created = state.create_account(&email).expect("account");
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

    let html = render_library_page(&state, &account, None, None);
    assert!(html.contains("Local only · never sent to a model"));
}
