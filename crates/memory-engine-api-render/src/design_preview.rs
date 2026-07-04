//! Design verification harness (dev-only; never compiled into the shipped
//! binary).
//!
//! `emit_preview_pages` renders every learner-facing UI state to standalone
//! HTML under `target/design-preview/` so the whole surface can be reviewed in a
//! browser without booting the server or driving auth. Run it with:
//!
//! ```text
//! cargo test -p memory-engine-api-render --lib emit_preview_pages -- --ignored --nocapture
//! ```
//!
//! The non-ignored `conformance_*` tests assert the aesthetic-kit Law and the
//! preserved form contracts over the rendered markup, so a regression in either
//! fails the normal gate.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use memory_engine_core::{Rating, ReviewUnitId, Verdict};
use memory_engine_persistence::GeneratedLearningActivityKind;
use memory_engine_study::{
    BetaStudyConceptProgress, BetaStudyCurrent, BetaStudyDraftRow, BetaStudyFeedback,
    BetaStudyGrade, BetaStudyItemHistory, BetaStudySummary,
};

use crate::{render_app_shell, render_login_requested};
use memory_engine_api_state::{
    ApiState, AppAccount, GenerationJob, JobStatus, SourceRecord, StudyViewResponse,
};

fn account() -> AppAccount {
    let state = ApiState::default();
    let created = state
        .create_account("preview@example.com")
        .unwrap_or_else(|error| panic!("preview account must be valid: {}", error.message));
    state
        .create_browser_session(&created)
        .unwrap_or_else(|error| panic!("preview browser session must be valid: {}", error.message))
}

fn source(id: &str, title: &str) -> SourceRecord {
    SourceRecord {
        source_id: id.to_owned(),
        title: title.to_owned(),
        body: String::new(),
        project_key: None,
        ttl_expires_at: None,
    }
}

fn summary() -> BetaStudySummary {
    BetaStudySummary {
        source_count: 2,
        accepted_draft_count: 3,
        approved_review_unit_count: 4,
        attempt_count: 12,
        last_outcome: Some(Verdict::Correct),
        next_review_unit_id: None,
    }
}

fn concept(
    label: &str,
    attempts: usize,
    correct: usize,
    health: &str,
    trend: &str,
    summary_text: &str,
) -> BetaStudyConceptProgress {
    let pct = if attempts > 0 {
        correct * 100 / attempts
    } else {
        0
    };
    BetaStudyConceptProgress {
        concept_key: label.to_lowercase().replace([' ', ':'], "-"),
        concept_label: label.to_owned(),
        attempts,
        correct,
        success_rate: format!("{correct} of {attempts} correct ({pct}%)"),
        trend: trend.to_owned(),
        average_response_time_ms: Some(2100),
        response_time_trend: "steady".to_owned(),
        health: health.to_owned(),
        summary: summary_text.to_owned(),
    }
}

fn item_history() -> BetaStudyItemHistory {
    BetaStudyItemHistory {
        attempts: 3,
        correct: 2,
        success_rate: "2 of 3 correct (67%)".to_owned(),
        trend: "improving".to_owned(),
        last_seen: Some(1_779_000_000_000),
        last_seen_summary: "last seen 2 days ago".to_owned(),
        last_response_time_ms: Some(1800),
        average_response_time_ms: Some(2200),
        response_time_trend: "faster than before".to_owned(),
        stage: "review".to_owned(),
        next_review: "next review in 4 days".to_owned(),
    }
}

fn job(
    id: &str,
    title: &str,
    status: JobStatus,
    card_count: usize,
    error: Option<&str>,
    created_at: i64,
) -> GenerationJob {
    GenerationJob {
        id: id.to_owned(),
        account_id: "preview-account-id".to_owned(),
        source_id: format!("src-{id}"),
        title: title.to_owned(),
        status,
        card_count,
        attempts: 1,
        error: error.map(str::to_owned),
        created_at,
        updated_at: created_at,
    }
}

/// A representative activity log: one job running, one failed (retryable), and
/// two finished decks already scheduled — newest first.
fn nato_jobs() -> Vec<GenerationJob> {
    vec![
        job("krebs", "Krebs cycle", JobStatus::Running, 0, None, 400),
        job(
            "spanish",
            "Spanish irregular verbs",
            JobStatus::Failed,
            0,
            Some("Couldn't generate — the model timed out."),
            300,
        ),
        job(
            "nato",
            "NATO phonetic alphabet",
            JobStatus::Succeeded,
            26,
            None,
            200,
        ),
        job(
            "photosynthesis",
            "Photosynthesis",
            JobStatus::Succeeded,
            12,
            None,
            100,
        ),
    ]
}

fn current(prompt: &str) -> BetaStudyCurrent {
    BetaStudyCurrent {
        review_unit_id: ReviewUnitId::new("ru-current"),
        prompt_id: "prompt-current".to_owned(),
        activity_kind: GeneratedLearningActivityKind::Quiz,
        activity_stage: "recognition".to_owned(),
        prompt: prompt.to_owned(),
        choices: Vec::new(),
        revision_expected_answer: "efímero".to_owned(),
        expected_answer: None,
        reference_text: None,
        worked_solution: None,
        grade: None,
        review_state: None,
        schedule_change: None,
        feedback: None,
    }
}

fn graded(verdict: Verdict, rating: Rating, with_feedback: bool) -> BetaStudyCurrent {
    let mut current = current("Translate to Spanish: “ephemeral”.");
    current.expected_answer = Some("efímero".to_owned());
    current.grade = Some(BetaStudyGrade {
        verdict,
        rating,
        is_correct: verdict == Verdict::Correct,
    });
    if with_feedback {
        current.feedback = Some(BetaStudyFeedback {
            verdict: format!("{verdict:?}").to_lowercase(),
            expected_answer: "efímero".to_owned(),
            item_history: item_history(),
            concept_progress: Some(concept(
                "Spanish: ephemeral words",
                11,
                6,
                "watch",
                "slipping",
                "One more like that lifts this concept above half.",
            )),
        });
    }
    current
}

fn view(
    drafts: Vec<BetaStudyDraftRow>,
    current: Option<BetaStudyCurrent>,
    concepts: Vec<BetaStudyConceptProgress>,
    due_count: usize,
    generation_notices: Vec<String>,
) -> StudyViewResponse {
    StudyViewResponse {
        drafts,
        current,
        concept_progress: concepts,
        summary: summary(),
        due_count,
        generation_notices,
    }
}

/// Every learner-facing UI state, in journey order, as `(slug, full-page html)`.
// Inherently a long enumeration of fixtures, one per UI state; splitting it
// would scatter the state matrix without making it clearer.
#[allow(clippy::too_many_lines)]
fn pages() -> Vec<(&'static str, String)> {
    let acct = account();
    let sources = vec![
        source("src-feynman", "The Feynman Lectures — Chapter 1"),
        source("src-spanish", "Spanish vocabulary — week 3"),
    ];
    let concepts = vec![
        concept(
            "Photosynthesis",
            7,
            6,
            "healthy",
            "improving",
            "Strong recall — intervals are widening.",
        ),
        concept(
            "Spanish: ephemeral words",
            11,
            6,
            "watch",
            "slipping",
            "Mixed recall — intervals staying short.",
        ),
    ];

    let mut multiple_choice =
        current("Which process converts light energy into chemical energy in plants?");
    multiple_choice.choices = vec![
        "Respiration".to_owned(),
        "Photosynthesis".to_owned(),
        "Transpiration".to_owned(),
        "Fermentation".to_owned(),
    ];

    let open = current("Translate to Spanish: “ephemeral”.");

    let mut revealed = graded(Verdict::Revealed, Rating::Again, false);
    revealed.reference_text = Some(
        "“efímero” — lasting a very short time; from the Greek ephēmeros, ‘lasting a day’."
            .to_owned(),
    );

    let jobs = nato_jobs();

    vec![
        (
            "01-signed-out",
            render_app_shell(None, &[], None, &[], None),
        ),
        (
            "02-capture-queued",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(vec![], None, vec![], 0, vec![])),
                &jobs,
                Some("Generating your cards — they'll appear below as they're ready."),
            ),
        ),
        (
            "03-activity",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(vec![], None, concepts.clone(), 8, vec![])),
                &jobs,
                None,
            ),
        ),
        (
            "04-workspace-due",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(vec![], None, concepts.clone(), 3, vec![])),
                &[],
                None,
            ),
        ),
        (
            "05-review-choices",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(
                    vec![],
                    Some(multiple_choice),
                    concepts.clone(),
                    3,
                    vec![],
                )),
                &[],
                None,
            ),
        ),
        (
            "06-review-open",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(vec![], Some(open), concepts.clone(), 3, vec![])),
                &[],
                None,
            ),
        ),
        (
            "07-graded-correct",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(
                    vec![],
                    Some(graded(Verdict::Correct, Rating::Good, true)),
                    concepts.clone(),
                    2,
                    vec![],
                )),
                &[],
                None,
            ),
        ),
        (
            "08-graded-wrong",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(
                    vec![],
                    Some(graded(Verdict::Wrong, Rating::Again, true)),
                    concepts.clone(),
                    3,
                    vec![],
                )),
                &[],
                None,
            ),
        ),
        (
            "09-revealed",
            render_app_shell(
                Some(&acct),
                &sources,
                Some(&view(vec![], Some(revealed), concepts, 3, vec![])),
                &[],
                None,
            ),
        ),
        (
            "10-check-email",
            render_login_requested(Some("/app/login/verify?token=preview-token")),
        ),
    ]
}

#[test]
#[ignore = "writes HTML preview files; run explicitly with --ignored"]
fn emit_preview_pages() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/design-preview");
    fs::create_dir_all(out.join("static"))?;
    // Mirror the served path `/static/aesthetic.css` so the pages resolve their
    // stylesheet when this directory is served at the root over HTTP.
    fs::write(
        out.join("static/aesthetic.css"),
        include_str!("../assets/aesthetic.css"),
    )?;

    let pages = pages();
    let mut index = String::from(
        "<!doctype html><html lang=en><meta charset=utf-8>\
<meta name=viewport content='width=device-width,initial-scale=1'>\
<title>Memory Engine — design preview</title>\
<body style='margin:0;background:#525252;font-family:system-ui,sans-serif'>\
<p style='color:#fff;font:13px/1.4 system-ui;padding:14px 18px;margin:0'>Memory Engine — every UI state. Each frame is a full rendered page.</p>",
    );
    for (name, html) in &pages {
        fs::write(out.join(format!("{name}.html")), html)?;
        let _ = write!(
            index,
            "<section style='margin:0 0 18px'>\
<p style='color:#d4d4d4;font:600 12px/1 ui-monospace,monospace;letter-spacing:.08em;text-transform:uppercase;padding:10px 18px;margin:0'>{name}</p>\
<iframe src='{name}.html' loading='lazy' style='display:block;width:100%;max-width:920px;margin:0 auto;height:780px;border:0;background:#fff'></iframe>\
</section>"
        );
    }
    index.push_str("</body></html>");
    fs::write(out.join("index.html"), index)?;

    eprintln!("\ndesign preview written to {}\n", out.display());
    Ok(())
}

/// Durable aesthetic-kit conformance gate: every rendered state must consume the
/// design system and must not reintroduce the Law violations a review caught.
/// Runs in the normal suite (not ignored), so a regression fails the gate.
#[test]
fn conformance_every_state_consumes_the_design_system() {
    for (name, html) in pages() {
        assert!(
            html.contains(r#"<link rel="stylesheet" href="/static/aesthetic.css">"#),
            "{name}: must link the vendored design system"
        );
        assert!(
            html.contains(r#"class="ae-screen""#),
            "{name}: must use the .ae-screen chrome archetype"
        );
        // The app glue must never round a corner — the kit's box radius is 0.
        // (aesthetic.css is linked, not inlined, so any hit comes from STYLE.)
        // The lone sanctioned exception is `border-radius: 50%`, which is not a
        // rounded rectangle but a full circle: the activity-log spinner and the
        // live-status dot. Forbid every other radius so the synthetic "card"
        // look the review caught cannot return.
        let foreign_radius = html
            .match_indices("border-radius")
            .filter(|(index, _)| !html[*index..].starts_with("border-radius: 50%"))
            .count();
        assert_eq!(
            foreign_radius, 0,
            "{name}: app CSS must not round corners (only the circular 50% status glyphs are allowed)"
        );
        // The synthetic uppercase register the kit forbids must not return.
        let lower = html.to_ascii_lowercase();
        assert!(
            !lower.contains("text-transform: uppercase")
                && !lower.contains("text-transform:uppercase"),
            "{name}: must not force uppercase (no register the kit does not sanction)"
        );
    }
}

#[test]
fn conformance_graded_review_carries_a_status_glyph_and_no_escape_hatches() {
    let pages = pages();
    let Some((_, graded)) = pages.iter().find(|(name, _)| *name == "07-graded-correct") else {
        panic!("graded-correct preview state missing");
    };
    // Verdict reads as ink with the hue on the glyph, never as a colored word.
    assert!(
        graded.contains(r#"class="ae-icon ae-ok""#),
        "graded verdict must carry a status glyph"
    );
    assert!(
        graded.contains(r#"class="me-verdict""#),
        "graded verdict must use the loud-by-weight register"
    );
    // Skip/Snooze/Bridge are pre-answer moves; they must vanish once graded.
    assert!(
        !graded.contains(r#"class="me-hatches""#),
        "graded review must not show escape hatches"
    );
}
