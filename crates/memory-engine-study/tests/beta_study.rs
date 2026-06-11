use std::{fs, path::PathBuf};

use memory_engine_core::{ScheduleStatus, Verdict};
use memory_engine_generation::{FakeModelProvider, FallbackProvider, StructuredBlockProvider};
use memory_engine_study::{
    BetaStudyOptions, BetaStudySession, BetaStudySourceInput, BetaStudyStatus,
};
use serde_json::json;

const NOW: i64 = 1_779_984_000_000;

#[test]
fn creates_source_generates_approves_reviews_reveals_and_advances_queue() {
    let directory = TempDirectory::new("happy-path");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");

    let started = study.start().expect("start");
    assert_eq!(started.status, BetaStudyStatus::Drafting);
    assert_eq!(started.summary.source_count, 0);
    assert_eq!(started.summary.attempt_count, 0);

    let sourced = study.add_source(source_input()).expect("source");
    assert_eq!(sourced.summary.source_count, 1);

    let generated = study.generate(None).expect("generate");
    assert_eq!(
        generated
            .drafts
            .iter()
            .map(|draft| draft.id.as_str())
            .collect::<Vec<_>>(),
        [
            "study-run-1-draft-src-nato-1-nato-letter-a",
            "study-run-1-draft-src-nato-2-nato-cat-composition"
        ]
    );
    assert_eq!(
        generated.drafts[1].worked_solution.as_deref(),
        Some("C is CHARLIE, A is ALFA, and T is TANGO.")
    );

    study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");
    assert_eq!(approved.status, BetaStudyStatus::Answering);
    assert_eq!(approved.summary.approved_review_unit_count, 2);
    let current = approved.current.expect("current");
    assert_eq!(
        current.prompt,
        "Spell CAT over the phone using the NATO phonetic alphabet."
    );
    assert_eq!(current.activity_kind.to_string_for_test(), "exercise");
    assert_eq!(current.expected_answer, None);
    assert_eq!(current.review_state, None);

    let revealed = study.reveal().expect("reveal");
    let revealed_current = revealed.current.expect("revealed current");
    assert_eq!(
        revealed_current.expected_answer.as_deref(),
        Some("CHARLIE ALFA TANGO")
    );
    assert_eq!(
        revealed_current.worked_solution.as_deref(),
        Some("C is CHARLIE, A is ALFA, and T is TANGO.")
    );

    let reviewed = study
        .submit_answer("CHARLIE ALFA TANGO", 4_200)
        .expect("submit");
    let reviewed_current = reviewed.current.expect("reviewed current");
    assert_eq!(reviewed.status, BetaStudyStatus::Graded);
    assert_eq!(
        reviewed_current.grade.expect("grade").verdict,
        Verdict::Correct
    );
    assert_eq!(
        reviewed_current.review_state.expect("review state").state,
        ScheduleStatus::Learning
    );
    assert_eq!(reviewed.summary.attempt_count, 1);
    assert_eq!(reviewed.summary.last_outcome, Some(Verdict::Correct));
    assert_eq!(
        reviewed_current
            .schedule_change
            .expect("schedule change")
            .after
            .last_review,
        Some(NOW)
    );

    let next = study.advance().expect("next");
    assert_eq!(next.status, BetaStudyStatus::Answering);
    assert_eq!(
        next.current.expect("next current").prompt,
        "What is the NATO phonetic alphabet word for A?"
    );
    assert_eq!(
        next.queue
            .iter()
            .map(|row| row
                .activity_kind
                .as_ref()
                .map(ToStringForTest::to_string_for_test))
            .collect::<Vec<_>>(),
        [Some("quiz".to_owned()), Some("exercise".to_owned())]
    );
}

#[test]
fn resumes_from_persisted_state_without_regenerating_content() {
    let directory = TempDirectory::new("resume");
    let path = directory.path().join("study.json");
    {
        let mut study =
            BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
        study.add_source(source_input()).expect("source");
        study.generate(None).expect("generate");
        study
            .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
            .expect("approve exercise");
        study
            .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("approve quiz");
        study
            .submit_answer("CHARLIE ALFA TANGO", 4_200)
            .expect("submit");
    }

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let view = resumed.start().expect("start");

    assert_eq!(view.summary.source_count, 1);
    assert_eq!(view.summary.accepted_draft_count, 2);
    assert_eq!(view.summary.approved_review_unit_count, 2);
    assert_eq!(view.summary.attempt_count, 1);
    assert_eq!(view.summary.last_outcome, Some(Verdict::Correct));
    assert_eq!(view.drafts.len(), 2);
    assert_eq!(
        view.current.expect("current").prompt,
        "What is the NATO phonetic alphabet word for A?"
    );
    let exercise = view
        .queue
        .iter()
        .find(|row| {
            row.activity_kind
                .as_ref()
                .map(ToStringForTest::to_string_for_test)
                == Some("exercise".to_owned())
        })
        .expect("exercise row");
    assert_eq!(exercise.reps, 1);
    assert_eq!(exercise.state, Some(ScheduleStatus::Learning));
}

#[test]
fn duplicate_submit_after_grading_is_view_only() {
    let directory = TempDirectory::new("duplicate");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve");

    let first = study.submit_answer("ALFA", 1_800).expect("first");
    let duplicate = study.submit_answer("ALFA", 1_800).expect("duplicate");

    assert_eq!(duplicate.summary.attempt_count, 1);
    assert_eq!(
        duplicate
            .current
            .as_ref()
            .and_then(|current| current.review_state.clone()),
        first
            .current
            .as_ref()
            .and_then(|current| current.review_state.clone())
    );
    assert_eq!(
        duplicate
            .current
            .as_ref()
            .and_then(|current| current.schedule_change.clone()),
        first
            .current
            .as_ref()
            .and_then(|current| current.schedule_change.clone())
    );

    let revealed_after_grade = study.reveal().expect("reveal after grade");
    let after_reveal_submit = study
        .submit_answer("ALFA", 1_800)
        .expect("after reveal submit");

    assert_eq!(revealed_after_grade.status, BetaStudyStatus::Graded);
    assert_eq!(after_reveal_submit.summary.attempt_count, 1);
}

#[test]
fn inspects_and_edits_active_review_item_without_revealing_answer() {
    let directory = TempDirectory::new("inspect-edit");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let started = study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve");
    assert_eq!(started.status, BetaStudyStatus::Answering);
    assert_eq!(
        started.current.as_ref().expect("current").expected_answer,
        None
    );
    assert_eq!(
        started.current.as_ref().expect("current").reference_text,
        None
    );

    let inspected = study.learn_more().expect("learn more");
    let inspected_current = inspected.current.as_ref().expect("inspected current");
    assert_eq!(inspected.status, BetaStudyStatus::Answering);
    assert_eq!(inspected_current.expected_answer, None);
    assert_eq!(
        inspected_current.reference_text.as_deref(),
        Some("The NATO phonetic alphabet word for A is ALFA.")
    );

    let edited = study
        .edit_current_prompt("Name the NATO code word for the letter A.", "ALFA")
        .expect("edit");
    let edited_current = edited.current.as_ref().expect("edited current");
    assert_eq!(
        edited_current.prompt,
        "Name the NATO code word for the letter A."
    );
    assert_eq!(edited_current.revision_expected_answer, "ALFA");
    assert_eq!(edited_current.expected_answer, None);
    assert_eq!(edited_current.reference_text, None);

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let resumed = resumed.start().expect("start");
    assert_eq!(
        resumed.current.expect("resumed current").prompt,
        "Name the NATO code word for the letter A."
    );
}

#[test]
fn snoozes_and_deletes_active_review_items_without_touching_schedule_history() {
    let directory = TempDirectory::new("lifecycle");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");
    study
        .submit_answer("CHARLIE ALFA TANGO", 4_200)
        .expect("submit");
    let exercise_review_state = study
        .view()
        .expect("view")
        .current
        .expect("current")
        .review_state
        .expect("review state");

    let snoozed = study
        .snooze_current_until(NOW + 86_400_000)
        .expect("snooze");
    assert_eq!(snoozed.status, BetaStudyStatus::Answering);
    assert_eq!(
        snoozed.current.expect("next current").prompt,
        "What is the NATO phonetic alphabet word for A?"
    );
    let snoozed_row = snoozed
        .queue
        .iter()
        .find(|row| {
            row.activity_kind
                .as_ref()
                .map(ToStringForTest::to_string_for_test)
                == Some("exercise".to_owned())
        })
        .expect("snoozed row");
    assert_eq!(snoozed_row.due, NOW + 86_400_000);
    assert_eq!(snoozed_row.reps, exercise_review_state.reps);
    assert_eq!(snoozed_row.state, Some(exercise_review_state.state));

    let deleted = study.archive_current().expect("delete current");
    assert_eq!(deleted.status, BetaStudyStatus::Drafting);
    assert_eq!(deleted.summary.approved_review_unit_count, 1);
    assert_eq!(deleted.summary.attempt_count, 1);
    assert!(deleted.queue.iter().all(|row| {
        row.review_unit_id.as_str() != "study-run-1-draft-src-nato-1-nato-letter-a"
    }));

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let resumed = resumed.start().expect("start");
    assert_eq!(resumed.summary.approved_review_unit_count, 1);
    assert_eq!(resumed.summary.attempt_count, 1);
    assert!(resumed.current.is_none());
}

#[test]
fn view_serializes_like_the_mobile_beta_api_contract() {
    let directory = TempDirectory::new("wire");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve");
    study.reveal().expect("reveal");

    let encoded = serde_json::to_value(study.view().expect("view")).expect("json");

    assert_eq!(encoded["status"], json!("revealed"));
    assert_eq!(encoded["sources"][0]["createdAt"], json!(NOW));
    assert_eq!(encoded["drafts"][0]["activityKind"], json!("quiz"));
    assert_eq!(encoded["drafts"][0]["validationStatus"], json!("accepted"));
    assert_eq!(encoded["current"]["revisionExpectedAnswer"], json!("ALFA"));
    assert_eq!(encoded["current"]["expectedAnswer"], json!("ALFA"));
    assert_eq!(encoded["current"]["workedSolution"], json!(null));
    assert_eq!(encoded["summary"]["approvedReviewUnitCount"], json!(1));
    assert_eq!(
        encoded["apiPressure"]
            .as_array()
            .expect("apiPressure")
            .len(),
        3
    );
}

#[test]
fn generates_drafts_from_arbitrary_prose_via_model_provider() {
    let directory = TempDirectory::new("prose-provider");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");

    study.start().expect("start");
    study
        .add_source(BetaStudySourceInput {
            id: "src-prose".to_owned(),
            title: "Mitochondria".to_owned(),
            body: "Mitochondria generate most of the cell's supply of adenosine triphosphate. \
                   They are sometimes called the powerhouse of the cell."
                .to_owned(),
        })
        .expect("source");

    // Structured-block primary finds nothing in prose, so the model provider runs.
    let structured = StructuredBlockProvider;
    let model = FakeModelProvider;
    let provider = FallbackProvider::new(&structured, &model);
    let generated = study
        .generate_with_provider(Some(vec!["src-prose".to_owned()]), &provider)
        .expect("generate");

    assert!(
        !generated.drafts.is_empty(),
        "arbitrary prose should yield drafts via the model provider"
    );
    assert!(
        generated.generation_notices.is_empty(),
        "a clean run must not raise notices: {:?}",
        generated.generation_notices
    );
}

#[test]
fn surfaces_a_human_readable_notice_when_a_source_yields_no_drafts() {
    let directory = TempDirectory::new("zero-draft-notice");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");

    study.start().expect("start");
    study
        .add_source(BetaStudySourceInput {
            id: "src-bare".to_owned(),
            title: "Unparseable note".to_owned(),
            body: "just some prose with no structured blocks".to_owned(),
        })
        .expect("source");

    // Structured provider alone produces no drafts from prose.
    let generated = study
        .generate(Some(vec!["src-bare".to_owned()]))
        .expect("generate");

    assert!(generated.drafts.is_empty());
    assert!(
        generated
            .generation_notices
            .iter()
            .any(|notice| notice.contains("No review items could be generated")),
        "zero-draft run must explain itself: {:?}",
        generated.generation_notices
    );
}

fn source_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-nato".to_owned(),
        title: "NATO practice notes".to_owned(),
        body: source_body(),
    }
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

fn now() -> i64 {
    NOW
}

fn later() -> i64 {
    NOW + 1_000
}

trait ToStringForTest {
    fn to_string_for_test(&self) -> String;
}

impl ToStringForTest for memory_engine_persistence::GeneratedLearningActivityKind {
    fn to_string_for_test(&self) -> String {
        match self {
            Self::Quiz => "quiz".to_owned(),
            Self::Exercise => "exercise".to_owned(),
        }
    }
}

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "memory-engine-rust-beta-study-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp directory");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
