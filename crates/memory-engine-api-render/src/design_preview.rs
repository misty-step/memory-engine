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
use memory_engine_persistence::{GeneratedLearningActivityKind, SourcePermission};
use memory_engine_study::{
    BetaStudyConceptProgress, BetaStudyCurrent, BetaStudyDraftRow, BetaStudyFeedback,
    BetaStudyGrade, BetaStudyItemHistory, BetaStudySummary,
};

use crate::{render_app_shell, render_login_requested};
use memory_engine_api_state::{
    AccountRegistry, ApiState, AppAccount, AuthConfig, GenerationJob, JobStatus, SourceRecord,
    StudyViewResponse,
};

fn account() -> AppAccount {
    let state = ApiState::new(
        AccountRegistry::default().with_auth_config(
            AuthConfig::allow_emails(["preview@example.com".to_owned()])
                .with_anonymous_account_creation(true),
        ),
    );
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
        permission: SourcePermission::default(),
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
    let pct = correct
        .saturating_mul(100)
        .checked_div(attempts)
        .unwrap_or(0);
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
        retryable: true,
        error: error.map(str::to_owned),
        created_at,
        updated_at: created_at,
        retry_at: None,
        lease_expires_at: None,
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
            Some("Couldn't generate. The model timed out."),
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
        concept_key: Some("nato-phonetic-alphabet".to_owned()),
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
        content_feedback_head_id: None,
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
        library: Vec::new(),
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
                Some("Generating your cards. They'll appear below as they're ready."),
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
    // Mirror the served path `/static/ledger.css` so the pages resolve their
    // stylesheet when this directory is served at the root over HTTP.
    fs::write(
        out.join("static/ledger.css"),
        include_str!("../assets/ledger.css"),
    )?;

    let pages = pages();
    let mut index = String::from(
        "<!doctype html><html lang=en><meta charset=utf-8>\
<meta name=viewport content='width=device-width,initial-scale=1'>\
<title>Scry — design preview</title>\
<body style='margin:0;background:#525252;font-family:system-ui,sans-serif'>\
<p style='color:#fff;font:13px/1.4 system-ui;padding:14px 18px;margin:0'>Scry — every UI state. Each frame is a full rendered page.</p>",
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

/// Durable Ledger conformance gate (root `DESIGN.md`): every rendered state
/// must consume the design system and must not reintroduce the defects the
/// operator's LAB-001 verdicts named. Runs in the normal suite (not
/// ignored), so a regression fails the gate.
#[test]
fn conformance_every_state_consumes_the_design_system() {
    for (name, html) in pages() {
        assert!(
            html.contains(r#"<link rel="stylesheet" href="/static/ledger.css">"#),
            "{name}: must link the Ledger design system"
        );
        assert!(
            html.contains(r#"class="ae-screen""#),
            "{name}: must use the .ae-screen chrome archetype"
        );
        // Token discipline: no <style> blocks and no raw hex in markup — the
        // whole visual system lives in ledger.css. The one sanctioned inline
        // style is a data-driven meter width, whose value is state, not
        // design.
        assert!(
            !html.contains("<style>"),
            "{name}: markup must not carry inline stylesheets; the system lives in ledger.css"
        );
        let foreign_inline_style = html
            .match_indices(" style=\"")
            .filter(|(index, _)| !html[*index..].starts_with(" style=\"width:"))
            .count();
        assert_eq!(
            foreign_inline_style, 0,
            "{name}: inline styles beyond the data-driven meter width are forbidden"
        );
        assert!(
            !html.contains("background: #") && !html.contains("color: #"),
            "{name}: raw hex belongs in ledger.css tokens, never in markup"
        );
        // The left-border accent stripe is a named operator kill (LAB-001);
        // it must never return in any state.
        assert!(
            !html.contains("border-left"),
            "{name}: left-border accent stripes are a design defect"
        );
        // The six-button permanent hatch row is a named operator kill: hatches
        // collapse behind the single More disclosure.
        assert!(
            !html.contains(r#"class="me-hatches""#),
            "{name}: the permanent hatch row must stay dead; use the More disclosure"
        );
        // Operator ruling (live dogfood, memory-engine-081): auto-advance is
        // dead law across every state — only an explicit Continue tap (or
        // Enter while it is focused) ever moves a graded page on. This
        // reverses the two-speed advance shipped in memory-engine-078.
        assert!(
            !html.contains("data-auto-advance"),
            "{name}: graded pages must never carry the auto-advance affordance"
        );
    }
}

#[test]
fn conformance_graded_review_holds_until_continue() {
    let pages = pages();
    let Some((_, graded)) = pages.iter().find(|(name, _)| *name == "07-graded-correct") else {
        panic!("graded-correct preview state missing");
    };
    assert!(
        graded.contains(r#"class="ae-icon ae-ok""#),
        "graded verdict must carry its status glyph"
    );
    assert!(
        graded.contains(r#"class="me-verdict""#),
        "graded verdict must use the verdict register"
    );
    // The card's dossier is post-grade only (DESIGN.md): present here…
    assert!(
        graded.contains(r#"class="me-meta-ledger""#),
        "graded review must show the meta ledger"
    );
    // Skip/Snooze/Bridge are pre-answer moves; they must vanish once graded.
    assert!(
        !graded.contains(r#"class="me-more""#),
        "graded review must not show escape hatches"
    );

    // The miss holds for study too — a correct verdict never advances on its
    // own either, so both states must be identical in this respect.
    let Some((_, wrong)) = pages.iter().find(|(name, _)| *name == "08-graded-wrong") else {
        panic!("graded-wrong preview state missing");
    };
    assert!(
        !wrong.contains("data-auto-advance"),
        "a miss must hold for study, never auto-advance"
    );
    assert!(
        wrong.contains(r#"class="me-meta-ledger""#),
        "the dossier shows on misses too"
    );

    // The pre-grade card is minimal: no dossier, hatches collapsed behind
    // the single More disclosure.
    let Some((_, answering)) = pages.iter().find(|(name, _)| *name == "06-review-open") else {
        panic!("review-open preview state missing");
    };
    assert!(
        !answering.contains(r#"class="me-meta-ledger""#),
        "card meta must never render pre-grade"
    );
    assert!(
        answering.contains(r#"<details class="me-more">"#),
        "pre-grade hatches must collapse behind the More disclosure"
    );
    assert!(
        answering.contains(r#"class="me-more-capture" href="/app/create""#),
        "Capture more must open Create, not Home: {answering}"
    );
}

/// Operator dogfood finding (memory-engine-081): the "Generating…" notice
/// must not linger once nothing is actually queued or running — it is
/// checked against live job state, not just echoed from the route that
/// enqueued it.
#[test]
fn conformance_generating_notice_only_shows_while_a_job_is_actually_in_flight() {
    let acct = account();
    let generating = "Generating your cards. They'll appear below as they're ready.";

    let in_flight = vec![job(
        "run-1",
        "Spanish irregular verbs",
        JobStatus::Running,
        0,
        None,
        100,
    )];
    let live = render_app_shell(Some(&acct), &[], None, &in_flight, Some(generating));
    assert!(
        live.contains(generating),
        "a live job must show the generating notice: {live}"
    );

    let resolved = vec![job(
        "run-1",
        "Spanish irregular verbs",
        JobStatus::Succeeded,
        12,
        None,
        100,
    )];
    let stale = render_app_shell(Some(&acct), &[], None, &resolved, Some(generating));
    assert!(
        !stale.contains(generating),
        "the generating notice must not linger once nothing is in flight: {stale}"
    );

    // Notices unrelated to generation are unconditional.
    let removed = render_app_shell(Some(&acct), &[], None, &resolved, Some("Source removed."));
    assert!(removed.contains("Source removed."));
}

/// Operator dogfood finding (memory-engine-081): too much explainer text on
/// the workspace. The welcome lede and capture hint are each one short line —
/// no second sentence restating the product pitch.
/// memory-engine-087: the workspace is now split into focused views — the
/// welcome lede lives on Home, the capture hint on Create.
#[test]
fn conformance_view_copy_trims_to_one_line_each() {
    let state = memory_engine_api_state::ApiState::new(
        memory_engine_api_state::AccountRegistry::default().with_auth_config(
            memory_engine_api_state::AuthConfig::allow_emails(["copy@example.com".to_owned()])
                .with_anonymous_account_creation(true),
        ),
    );
    let created = state.create_account("copy@example.com").expect("account");
    let acct = state.create_browser_session(&created).expect("session");
    let home = render_app_shell(Some(&acct), &[], None, &[], None);
    assert!(
        home.contains(
            r#"<p class="ae-lede me-welcome">Type a topic or paste anything worth remembering.</p>"#
        ),
        "the welcome lede must be a single short line on Home: {home}"
    );
    let create = crate::render_create_page(&state, &acct, None);
    assert!(
        create.contains(
            r#"<span class="ae-dim me-hint me-live-hint">Generates in the background.</span>"#
        ),
        "the capture hint must be a single short line on Create: {create}"
    );
}

/// memory-engine-087 + home chrome: persistent one-tap navigation is present
/// on every standing view (Home, Create, Library, Analytics) with
/// `aria-current` marking the active view, and absent from the full-bleed
/// review. The nav holds only the four view links — sign-out lives in the
/// header account menu. Home reminders stay collapsed by default.
#[test]
fn conformance_nav_present_on_standing_views_absent_on_review() {
    let state = memory_engine_api_state::ApiState::new(
        memory_engine_api_state::AccountRegistry::default().with_auth_config(
            memory_engine_api_state::AuthConfig::allow_emails(["nav@example.com".to_owned()])
                .with_anonymous_account_creation(true),
        ),
    );
    let created = state.create_account("nav@example.com").expect("account");
    let acct = state.create_browser_session(&created).expect("session");

    // Home (no active review) has nav with Home current.
    let home = render_app_shell(Some(&acct), &[], None, &[], None);
    assert!(
        home.contains(r#"<nav class="me-nav" aria-label="Views">"#),
        "Home must carry persistent nav: {home}"
    );
    assert!(
        home.contains(r#"href="/" aria-current="page">Home</a>"#),
        "Home nav must mark itself current: {home}"
    );
    assert_nav_is_four_views_only(&home, "Home");
    assert_account_menu_logout(&home, "Home");
    assert!(
        home.contains(r#"<details class="me-return-details">"#),
        "Home reminders must collapse behind a details disclosure: {home}"
    );
    assert!(
        home.contains(r#"<summary class="me-return-summary">Daily reminders</summary>"#),
        "Home reminders summary must read Daily reminders: {home}"
    );
    // Default-closed: no open attribute on the reminders details.
    assert!(
        !home.contains(r#"<details class="me-return-details" open"#)
            && !home.contains(r#"<details open class="me-return-details""#),
        "Home reminders must be collapsed by default: {home}"
    );
    // Enable is the only primary CTA inside the disclosure; turn-off is quiet.
    assert!(
        home.contains(r#"class="ae-button" type="submit">Enable due-count reminders</button>"#),
        "Home reminders must keep one primary enable CTA: {home}"
    );
    assert!(
        home.contains(r#"class="ae-button-quiet" type="submit">Turn off reminders</button>"#),
        "Home turn-off must stay a quiet secondary control: {home}"
    );
    assert!(
        !home.contains(r">Signed in</span>"),
        "bare Signed in spam must not appear: {home}"
    );

    // Create has nav with Create current.
    let create = crate::render_create_page(&state, &acct, None);
    assert!(
        create.contains(r#"<nav class="me-nav" aria-label="Views">"#),
        "Create must carry persistent nav: {create}"
    );
    assert!(
        create.contains(r#"href="/app/create" aria-current="page">Create</a>"#),
        "Create nav must mark itself current: {create}"
    );
    assert_nav_is_four_views_only(&create, "Create");
    assert_account_menu_logout(&create, "Create");

    // Library has nav with Library current.
    let library = crate::render_library_page(&state, &acct, None, None);
    assert!(
        library.contains(r#"<nav class="me-nav" aria-label="Views">"#),
        "Library must carry persistent nav: {library}"
    );
    assert!(
        library.contains(r#"href="/app/library" aria-current="page">Library</a>"#),
        "Library nav must mark itself current: {library}"
    );
    assert_nav_is_four_views_only(&library, "Library");
    assert_account_menu_logout(&library, "Library");

    // Analytics has nav with Analytics current.
    let view = view(vec![], None, vec![], 0, vec![]);
    let analytics =
        crate::render_analytics_page(&acct, &view, crate::AnalyticsViewOptions::default());
    assert!(
        analytics.contains(r#"<nav class="me-nav" aria-label="Views">"#),
        "Analytics must carry persistent nav: {analytics}"
    );
    assert!(
        analytics.contains(r#"href="/app/analytics" aria-current="page">Analytics</a>"#),
        "Analytics nav must mark itself current: {analytics}"
    );
    assert_nav_is_four_views_only(&analytics, "Analytics");
    assert_account_menu_logout(&analytics, "Analytics");

    // Review is full-bleed: no nav, but still has the header account menu.
    let pages = pages();
    let Some((_, review)) = pages.iter().find(|(name, _)| *name == "06-review-open") else {
        panic!("review-open preview state missing");
    };
    assert!(
        !review.contains(r#"<nav class="me-nav""#),
        "review must not carry nav (full-bleed): {review}"
    );
    assert_account_menu_logout(review, "review");
}

/// The bottom nav is only the four standing views — no logout forms inside.
fn assert_nav_is_four_views_only(html: &str, label: &str) {
    let start = html
        .find(r#"<nav class="me-nav" aria-label="Views">"#)
        .unwrap_or_else(|| panic!("{label}: missing me-nav open tag"));
    let after = &html[start..];
    let end_rel = after
        .find("</nav>")
        .unwrap_or_else(|| panic!("{label}: missing me-nav close tag"));
    let nav = &after[..end_rel + "</nav>".len()];
    assert!(
        !nav.contains("/app/logout"),
        "{label}: nav must not contain logout forms: {nav}"
    );
    assert!(
        !nav.contains("Sign out"),
        "{label}: nav must not contain sign-out controls: {nav}"
    );
    for (href, name) in [
        ("/", "Home"),
        ("/app/create", "Create"),
        ("/app/library", "Library"),
        ("/app/analytics", "Analytics"),
    ] {
        assert!(
            nav.contains(&format!(r#"href="{href}""#)) && nav.contains(name),
            "{label}: nav must contain {name} link: {nav}"
        );
    }
    let link_count = nav.matches(r#"class="me-nav-item""#).count();
    assert_eq!(
        link_count, 4,
        "{label}: nav must contain exactly four view links, found {link_count}: {nav}"
    );
}

/// Sign-out controls live in a header account menu with CSRF forms.
fn assert_account_menu_logout(html: &str, label: &str) {
    assert!(
        html.contains(r#"<details class="me-account">"#),
        "{label}: must carry header account menu: {html}"
    );
    assert!(
        html.contains(r#"<summary class="me-account-summary">Account</summary>"#),
        "{label}: account menu summary must read Account: {html}"
    );
    assert!(
        html.contains(r#"action="/app/logout" method="post""#),
        "{label}: account menu must include logout form: {html}"
    );
    assert!(
        html.contains(r#"action="/app/logout-all" method="post""#),
        "{label}: account menu must include logout-all form: {html}"
    );
    assert!(
        html.contains(">Sign out</button>") && html.contains(">Sign out everywhere</button>"),
        "{label}: account menu must label both sign-out actions: {html}"
    );
    // CSRF tokens ride the logout forms.
    assert!(
        html.contains(r#"name="csrfToken""#),
        "{label}: logout forms must carry CSRF: {html}"
    );
}
