use std::{fs, path::PathBuf};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, GradeResult, GraderKind, ProgressionMetadata, Prompt, Rating,
    ReviewUnitId, ReviewUnitLifecycle, ScheduleStatus, Verdict,
};
use memory_engine_generation::{
    BridgeMaterial, BridgeMaterialProvider, BridgeMaterialRequest, DraftCandidate,
    FakeModelProvider, FallbackProvider, ProviderFailure, ReferenceNoteDraft,
    ReferenceNoteProvider, ReferenceNoteRequest,
};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaReviewUnitRecord, GeneratedLearningActivityKind,
    GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
    GeneratedPromptValidationStatus, PersistedQueueCandidate, RemediationPackStatus,
    SourceDocument, SourceDocumentKind, SourcePermission,
};
use memory_engine_service::{MemoryServiceStore, ServiceAttemptRecord};
use memory_engine_study::{
    infer_capture_title, BetaStudyOptions, BetaStudySession, BetaStudySourceInput, BetaStudyStatus,
    DEFAULT_BRIDGE_PARENT_DEFER_MS, DEFAULT_REMEDIATION_PACK_TTL_MS, DEFAULT_SKIP_DEFER_MS,
    DEFAULT_SNOOZE_DEFER_MS,
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
fn post_answer_feedback_summarizes_item_and_concept_history() {
    let directory = TempDirectory::new("post-answer-feedback");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve");
    assert_eq!(approved.concept_progress.len(), 1);
    assert_eq!(approved.concept_progress[0].health, "untried");

    let reviewed = study.submit_answer("BRAVO", 1_800).expect("submit");
    let current = reviewed.current.expect("current");
    let feedback = current.feedback.expect("feedback");

    assert_eq!(feedback.verdict, "Try again");
    assert_eq!(feedback.expected_answer, "ALFA");
    assert_eq!(feedback.item_history.attempts, 1);
    assert_eq!(feedback.item_history.correct, 0);
    assert_eq!(feedback.item_history.success_rate, "0 of 1 correct (0.0%)");
    assert_eq!(feedback.item_history.trend, "not enough data");
    assert_eq!(feedback.item_history.last_seen, Some(NOW));
    assert_eq!(
        feedback.item_history.last_seen_summary,
        "last seen just now"
    );
    assert_eq!(feedback.item_history.last_response_time_ms, Some(1_800));
    assert_eq!(feedback.item_history.average_response_time_ms, Some(1_800));
    assert_eq!(feedback.item_history.response_time_trend, "not enough data");
    assert!(feedback.item_history.stage.contains("Learning"));
    assert!(
        feedback.item_history.next_review.contains("again"),
        "next review should be written in human language: {:?}",
        feedback.item_history.next_review
    );
    let concept = feedback.concept_progress.expect("concept feedback");
    assert_eq!(concept.concept_key, "nato-letter-a");
    assert_eq!(concept.concept_label, "nato letter a");
    assert_eq!(concept.attempts, 1);
    assert_eq!(concept.correct, 0);
    assert_eq!(concept.success_rate, "0 of 1 correct (0.0%)");
    assert_eq!(concept.trend, "not enough data");
    assert_eq!(concept.average_response_time_ms, Some(1_800));
    assert_eq!(concept.response_time_trend, "not enough data");
    assert_eq!(concept.health, "struggling");
    assert!(
        concept.summary.contains("struggling"),
        "failing concept copy must be honest: {}",
        concept.summary
    );
    assert_eq!(reviewed.concept_progress, vec![concept]);
}

#[test]
fn multiple_choice_choices_rotate_between_reviews_without_changing_answer() {
    let directory = TempDirectory::new("mcq-choice-rotation");
    let path = directory.path().join("study.json");
    {
        let mut study =
            BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
        study.add_source(source_input()).expect("source");
        study.generate(None).expect("generate");
        study
            .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("approve");
    }

    let mut first_session =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("resume");
    let first = first_session
        .start()
        .expect("start")
        .current
        .expect("current");
    assert_eq!(first.revision_expected_answer, "ALFA");
    assert_eq!(
        sorted(first.choices.clone()),
        ["ALFA".to_owned(), "BRAVO".to_owned(), "CHARLIE".to_owned()]
    );

    record_graded_attempt_with_response_time(
        &path,
        "study-run-1-draft-src-nato-1-nato-letter-a",
        false,
        1,
        2_400,
    );
    let mut second_session =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let second = second_session
        .start()
        .expect("start")
        .current
        .expect("current");

    assert_eq!(second.review_unit_id, first.review_unit_id);
    assert_eq!(second.revision_expected_answer, "ALFA");
    assert!(second.choices.iter().any(|choice| choice == "ALFA"));
    assert_ne!(second.choices, first.choices);

    record_graded_attempt_with_response_time(
        &path,
        "study-run-1-draft-src-nato-1-nato-letter-a",
        true,
        2,
        1_900,
    );
    let mut third_session =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let third = third_session
        .start()
        .expect("start")
        .current
        .expect("current");

    assert_eq!(third.review_unit_id, first.review_unit_id);
    assert_ne!(third.choices, second.choices);
}

#[test]
fn queue_rotates_due_variants_with_the_same_concept_and_stage() {
    let directory = TempDirectory::new("variant-rotation");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(variant_concept_input()).expect("source");
    let generated = study.generate(None).expect("generate");
    assert_eq!(generated.drafts.len(), 3);
    assert!(generated.drafts.iter().all(|draft| {
        draft.validation_status == GeneratedPromptValidationStatus::Accepted
            && draft.activity_stage == "recognition-3"
    }));
    for draft_id in generated
        .drafts
        .iter()
        .map(|draft| draft.id.clone())
        .collect::<Vec<_>>()
    {
        study.approve_draft(&draft_id).expect("approve variant");
    }

    let first = study
        .start()
        .expect("first")
        .current
        .expect("first current");
    study
        .submit_answer("ALFA", 2_100)
        .expect("first submit through study boundary");
    let second = study
        .advance()
        .expect("second")
        .current
        .expect("second current");

    assert_eq!(second.activity_stage, "recognition-3");
    assert_ne!(second.review_unit_id, first.review_unit_id);
    assert_ne!(second.prompt, first.prompt);

    study
        .submit_answer("BRAVO", 2_800)
        .expect("second submit through study boundary");
    let third = study
        .advance()
        .expect("third")
        .current
        .expect("third current");

    assert_ne!(third.review_unit_id, first.review_unit_id);
    assert_ne!(third.review_unit_id, second.review_unit_id);
    assert_ne!(third.prompt, first.prompt);
    assert_ne!(third.prompt, second.prompt);
}

#[test]
fn post_answer_feedback_exposes_item_response_time_and_success_trends() {
    let directory = TempDirectory::new("response-time-trend");
    let path = directory.path().join("study.json");
    {
        let mut study =
            BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
        study.add_source(source_input()).expect("source");
        study.generate(None).expect("generate");
        study
            .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("approve");
    }
    record_graded_attempt_with_response_time(
        &path,
        "study-run-1-draft-src-nato-1-nato-letter-a",
        false,
        1,
        3_200,
    );
    record_graded_attempt_with_response_time(
        &path,
        "study-run-1-draft-src-nato-1-nato-letter-a",
        true,
        2,
        2_100,
    );

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    let pre_submit_choices = resumed
        .start()
        .expect("start")
        .current
        .expect("current")
        .choices;
    let reviewed = resumed
        .submit_answer("BRAVO", 2_900)
        .expect("submit latest");
    let current = reviewed.current.expect("current");
    assert_eq!(current.choices, pre_submit_choices);
    let feedback = current.feedback.expect("feedback");

    assert_eq!(feedback.item_history.attempts, 3);
    assert_eq!(feedback.item_history.correct, 1);
    assert_eq!(feedback.item_history.trend, "declining");
    assert_eq!(feedback.item_history.last_response_time_ms, Some(2_900));
    assert_eq!(feedback.item_history.average_response_time_ms, Some(2_733));
    assert_eq!(feedback.item_history.response_time_trend, "slower");
    let concept = feedback.concept_progress.expect("concept");
    assert_eq!(concept.trend, "declining");
    assert_eq!(concept.average_response_time_ms, Some(2_733));
    assert_eq!(concept.response_time_trend, "slower");
}

#[test]
fn concept_progress_lists_weakest_concepts_first() {
    let directory = TempDirectory::new("concept-progress");
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
        .expect("submit strong concept");
    study.advance().expect("next");
    let reviewed = study
        .submit_answer("BRAVO", 1_800)
        .expect("submit weak concept");

    let concepts = reviewed.concept_progress;
    assert_eq!(
        concepts
            .iter()
            .map(|concept| concept.concept_key.as_str())
            .collect::<Vec<_>>(),
        ["nato-letter-a", "nato-cat-composition"]
    );
    assert_eq!(concepts[0].health, "struggling");
    assert_eq!(concepts[0].success_rate, "0 of 1 correct (0.0%)");
    assert_eq!(concepts[1].health, "solid");
    assert_eq!(concepts[1].success_rate, "1 of 1 correct (100.0%)");
}

#[test]
fn concept_progress_includes_active_approved_untried_concepts_and_excludes_archived_sources() {
    let directory = TempDirectory::new("untried-concept-progress");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(shared_concept_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-shared-1-nato-letter-a")
        .expect("approve first");
    let reviewed = study
        .approve_draft("study-run-1-draft-src-shared-2-nato-letter-a")
        .expect("approve second");

    assert_eq!(reviewed.concept_progress.len(), 1);
    assert_eq!(reviewed.concept_progress[0].concept_key, "nato-letter-a");
    assert_eq!(reviewed.concept_progress[0].health, "untried");
    assert_eq!(reviewed.concept_progress[0].attempts, 0);
    assert_eq!(reviewed.concept_progress[0].correct, 0);

    let (archived, archived_count) = study.archive_source("src-shared").expect("archive");
    assert_eq!(archived_count, 2);
    assert!(archived.concept_progress.is_empty());
}

#[test]
fn concept_progress_rolls_up_items_with_the_same_concept_key() {
    let directory = TempDirectory::new("shared-concept-progress");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(shared_concept_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-shared-1-nato-letter-a")
        .expect("approve first");
    study
        .approve_draft("study-run-1-draft-src-shared-2-nato-letter-a")
        .expect("approve second");

    study.submit_answer("ALFA", 1_800).expect("first submit");
    study.advance().expect("next");
    let reviewed = study.submit_answer("BRAVO", 1_800).expect("second submit");

    assert_eq!(reviewed.concept_progress.len(), 1);
    let concept = &reviewed.concept_progress[0];
    assert_eq!(concept.concept_key, "nato-letter-a");
    assert_eq!(concept.concept_label, "nato letter a");
    assert_eq!(concept.attempts, 2);
    assert_eq!(concept.correct, 1);
    assert_eq!(concept.success_rate, "1 of 2 correct (50.0%)");
    assert_eq!(concept.trend, "declining");
    assert_ne!(
        concept.concept_key,
        "generated-quiz-src-shared-1-nato-letter-a"
    );
}

#[test]
fn concept_progress_tiebreaks_equal_rates_by_more_evidence() {
    let directory = TempDirectory::new("concept-progress-tiebreak");
    let path = directory.path().join("study.json");
    {
        let mut study =
            BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
        study.add_source(source_input()).expect("source");
        study.generate(None).expect("generate");
        study
            .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("approve quiz");
        study
            .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
            .expect("approve exercise");
    }
    record_graded_attempt(
        &path,
        "study-run-1-draft-src-nato-1-nato-letter-a",
        false,
        1,
    );
    record_graded_attempt(
        &path,
        "study-run-1-draft-src-nato-2-nato-cat-composition",
        false,
        2,
    );
    record_graded_attempt(
        &path,
        "study-run-1-draft-src-nato-2-nato-cat-composition",
        false,
        3,
    );

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("resume");
    let view = resumed.start().expect("start");

    assert_eq!(
        view.concept_progress
            .iter()
            .map(|concept| (concept.concept_key.as_str(), concept.success_rate.as_str()))
            .collect::<Vec<_>>(),
        [
            ("nato-cat-composition", "0 of 2 correct (0.0%)"),
            ("nato-letter-a", "0 of 1 correct (0.0%)")
        ]
    );
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
    assert!(
        deleted
            .drafts
            .iter()
            .find(|draft| draft.id == "study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("archived approved draft")
            .approved
    );
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
fn skip_defers_current_item_without_recording_a_review_attempt() {
    let directory = TempDirectory::new("skip");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let started = study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");
    let skipped_id = started.current.expect("current").review_unit_id;

    let skipped = study.skip_current().expect("skip");

    assert_eq!(skipped.summary.attempt_count, 0);
    assert_eq!(
        skipped.current.expect("next current").prompt,
        "What is the NATO phonetic alphabet word for A?"
    );
    let skipped_row = skipped
        .queue
        .iter()
        .find(|row| row.review_unit_id == skipped_id)
        .expect("skipped row");
    assert_eq!(skipped_row.due, NOW + DEFAULT_SKIP_DEFER_MS);
    assert_eq!(skipped_row.reps, 0);
    assert_eq!(skipped_row.state, None);
}

#[test]
fn snoozes_every_card_in_the_current_concept_without_creating_review_history() {
    let directory = TempDirectory::new("concept-snooze");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(concept_snooze_input()).expect("source");
    let generated = study.generate(None).expect("generate");
    for draft in &generated.drafts {
        study.approve_draft(&draft.id).expect("approve");
    }

    let started = study.start().expect("start");
    assert_eq!(
        started
            .current
            .as_ref()
            .and_then(|current| current.concept_key.as_deref()),
        Some("nato-letter-a")
    );

    let current_id = started
        .current
        .as_ref()
        .expect("current")
        .review_unit_id
        .clone();
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read study snapshot"))
            .expect("decode study snapshot");
    let concept_member_ids = persisted["reviewUnits"]
        .as_array()
        .expect("review units")
        .iter()
        .filter(|unit| unit["queue"]["conceptKey"] == "nato-letter-a")
        .map(|unit| ReviewUnitId::new(unit["reviewUnitId"].as_str().expect("unit id")))
        .collect::<Vec<_>>();
    assert_eq!(concept_member_ids.len(), 2);
    let reviewed = study.submit_answer("ALFA", 1_800).expect("review");
    let reviewed_schedule = reviewed
        .current
        .as_ref()
        .expect("reviewed current")
        .review_state
        .clone()
        .expect("reviewed schedule");

    let horizon = NOW + DEFAULT_SNOOZE_DEFER_MS;
    let snoozed = study
        .snooze_current_concept_until(horizon)
        .expect("snooze concept");

    assert_eq!(snoozed.summary.attempt_count, 1);
    assert_eq!(snoozed.due_count, 1);
    let snoozed_reviewed_row = snoozed
        .queue
        .iter()
        .find(|row| row.review_unit_id == current_id)
        .expect("snoozed reviewed row");
    assert_eq!(snoozed_reviewed_row.reps, reviewed_schedule.reps);
    assert_eq!(snoozed_reviewed_row.state, Some(reviewed_schedule.state));
    assert_eq!(
        snoozed
            .queue
            .iter()
            .filter(|row| concept_member_ids.contains(&row.review_unit_id))
            .map(|row| row.due)
            .collect::<Vec<_>>(),
        [horizon, horizon]
    );
    assert!(snoozed
        .queue
        .iter()
        .any(|row| !concept_member_ids.contains(&row.review_unit_id) && row.due < horizon));

    let mut resumed = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(after_snooze))
        .expect("resume");
    let resumed = resumed.start().expect("resume start");
    assert_eq!(resumed.due_count, 3);
    assert!(resumed
        .queue
        .iter()
        .filter(|row| concept_member_ids.contains(&row.review_unit_id))
        .all(|row| row.due <= after_snooze()));
}

#[test]
fn learn_more_generates_and_caches_concept_note_when_source_span_is_missing() {
    let directory = TempDirectory::new("reference-fallback");
    let path = directory.path().join("study.json");
    seed_spanless_review(&path);
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.start().expect("start");

    let explained = study.learn_more().expect("learn more");
    let current = explained.current.expect("current");
    assert_eq!(current.expected_answer, None);
    assert!(current
        .reference_text
        .as_deref()
        .expect("generated note")
        .contains("ALFA"));

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert_eq!(snapshot.concept_reference_notes.len(), 1);
    assert_eq!(
        snapshot.concept_reference_notes[0].concept_key,
        "nato-letter-a"
    );

    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(later)).expect("resume");
    resumed.start().expect("start");
    let cached = resumed
        .learn_more_with_provider(&PanicReferenceProvider)
        .expect("cached learn more");
    assert!(cached
        .current
        .expect("current")
        .reference_text
        .as_deref()
        .expect("cached note")
        .contains("ALFA"));
}

#[test]
fn bridge_material_creates_easier_due_items_before_the_parent() {
    let directory = TempDirectory::new("bridge");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz sibling");
    let parent_id = approved.current.expect("parent").review_unit_id;

    let bridged = study.generate_bridge_material().expect("bridge");
    let current = bridged.current.expect("bridge current");

    assert!(current.review_unit_id.as_str().starts_with("bridge-"));
    assert_eq!(current.activity_stage, "recognition-bridge");
    assert_eq!(bridged.summary.attempt_count, 0);
    assert_eq!(bridged.summary.approved_review_unit_count, 4);
    let bridge_rows = bridged
        .queue
        .iter()
        .filter(|row| {
            row.activity_stage
                .as_deref()
                .is_some_and(|stage| stage.contains("bridge"))
        })
        .collect::<Vec<_>>();
    assert_eq!(bridge_rows.len(), 2);
    assert_eq!(
        bridge_rows
            .iter()
            .filter_map(|row| row.activity_stage.as_deref())
            .collect::<Vec<_>>(),
        ["recognition-bridge", "cued-recall-bridge"]
    );
    let parent_row = bridged
        .queue
        .iter()
        .find(|row| row.review_unit_id == parent_id)
        .expect("parent row");
    assert_eq!(parent_row.due, NOW + DEFAULT_BRIDGE_PARENT_DEFER_MS);
    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert_eq!(
        snapshot
            .generation_runs
            .iter()
            .find(|run| run.id == "bridge-run-2")
            .expect("bridge run")
            .parent_review_unit_id
            .as_ref(),
        Some(&parent_id)
    );
    let sibling_row = bridged
        .queue
        .iter()
        .find(|row| row.review_unit_id.as_str() == "generated-quiz-src-nato-1-nato-letter-a")
        .expect("sibling row");
    assert!(
        bridge_rows.iter().all(|row| row.due < sibling_row.due),
        "bridge rows should sort ahead of existing due siblings: {:?}",
        bridged.queue
    );
}

#[test]
fn wrong_answer_triggers_remediation_pack_before_parent_returns() {
    let directory = TempDirectory::new("remediation-pack");
    let path = directory.path().join("study.json");
    let mut study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("open")
        .with_remediation_packs_enabled(true);
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let parent_id = approved.current.expect("parent").review_unit_id;

    let graded = study
        .submit_answer("wrong answer", 1_800)
        .expect("wrong submit");
    assert_eq!(
        graded
            .current
            .expect("graded current")
            .grade
            .expect("grade")
            .verdict,
        Verdict::Wrong
    );

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let pack = snapshot
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .cloned()
        .expect("remediation pack");
    assert_eq!(pack.status, RemediationPackStatus::Active);
    assert_eq!(pack.review_unit_ids.len(), 2);
    for member_id in &pack.review_unit_ids {
        let member = snapshot
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == *member_id)
            .expect("pack member row");
        let progression = member
            .queue
            .progression
            .as_ref()
            .expect("pack member progression");
        assert!(
            !progression.supersedes.contains(&parent_id),
            "remediation packs must never supersede the parent"
        );
    }
    let parent_row = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == parent_id)
        .expect("parent row");
    assert!(
        parent_row.snoozed_until.is_some_and(|until| until > NOW),
        "the failed parent must be deferred while the pack is active"
    );

    for _ in 0..pack.review_unit_ids.len() {
        let advanced = study.advance().expect("advance");
        let current = advanced.current.expect("pack member current");
        assert!(
            pack.review_unit_ids.contains(&current.review_unit_id),
            "queue must surface pack members before the deferred parent: {:?}",
            current.review_unit_id
        );
        assert_ne!(current.review_unit_id, parent_id);
        study
            .submit_answer("pack-member-answer", 1_000)
            .expect("submit pack member");
    }

    let mut resumed_after_pack =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(after_relearn))
            .expect("resume after relearn")
            .with_remediation_packs_enabled(true);
    let after_pack = resumed_after_pack
        .start()
        .expect("start after pack completion");
    let returned = after_pack.current.expect("parent returns");
    assert_eq!(
        returned.review_unit_id, parent_id,
        "the parent must return as soon as its own schedule allows, not wait out the remediation TTL"
    );

    let resolved = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let resolved_pack = resolved
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .expect("resolved pack");
    assert_eq!(resolved_pack.status, RemediationPackStatus::Completed);
    let parent_row = resolved
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == parent_id)
        .expect("parent row");
    assert!(
        parent_row.snoozed_until.is_some_and(|until| until <= NOW),
        "the parent's schedule history must return immediately, not stay buried"
    );
}

#[test]
fn remediation_pack_generation_with_zero_accepted_drafts_leaves_parent_current() {
    let directory = TempDirectory::new("remediation-pack-rejected");
    let path = directory.path().join("study.json");
    let mut study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("open")
        .with_remediation_packs_enabled(true);
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");

    // Bridge the quiz once via the explicit escape hatch: the resulting
    // bridge item sits at the lowest progression stage (0). A later wrong
    // answer against that bridge item can never generate an "easier"
    // remediation candidate, so every candidate the fixture provider
    // proposes is rejected — the zero-accepted-drafts path.
    let bridged = study.generate_bridge_material().expect("bridge");
    let bridge_id = bridged.current.expect("bridge current").review_unit_id;
    assert!(bridge_id.as_str().starts_with("bridge-"));

    let graded = study
        .submit_answer("wrong answer", 1_800)
        .expect("wrong submit");
    assert_eq!(
        graded
            .current
            .expect("graded current")
            .grade
            .expect("grade")
            .verdict,
        Verdict::Wrong
    );

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let pack = snapshot
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == bridge_id)
        .expect("a rejected pack is still recorded, not silently dropped");
    assert_eq!(pack.status, RemediationPackStatus::Rejected);
    assert!(pack.review_unit_ids.is_empty());

    let bridge_row = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == bridge_id)
        .expect("bridge row");
    assert!(
        bridge_row.snoozed_until.is_none(),
        "a rejected pack must never defer or bury its parent"
    );

    let view = study.view().expect("view");
    assert_eq!(
        view.current.expect("current").review_unit_id,
        bridge_id,
        "with no accepted remediation drafts the parent stays current"
    );
}

#[test]
fn correct_answer_never_triggers_a_remediation_pack() {
    let directory = TempDirectory::new("remediation-pack-correct");
    let path = directory.path().join("study.json");
    let mut study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("open")
        .with_remediation_packs_enabled(true);
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");

    let graded = study.submit_answer("ALFA", 1_800).expect("correct submit");
    assert_eq!(
        graded
            .current
            .expect("graded current")
            .grade
            .expect("grade")
            .verdict,
        Verdict::Correct
    );

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert!(
        snapshot.remediation_packs.is_empty(),
        "a correct attempt must never trigger a remediation pack"
    );
}

#[test]
fn remediation_packs_stay_off_until_a_session_opts_in() {
    let directory = TempDirectory::new("remediation-pack-opt-out");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    assert!(!study.remediation_packs_enabled());
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .approve_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("approve quiz");

    study.submit_answer("BRAVO", 1_800).expect("wrong submit");

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert!(
        snapshot.remediation_packs.is_empty(),
        "existing sessions that never opt in must keep today's behavior"
    );
}

#[test]
fn stale_remediation_pack_expires_and_returns_the_parent() {
    let directory = TempDirectory::new("remediation-pack-expiry");
    let path = directory.path().join("study.json");
    let mut study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("open")
        .with_remediation_packs_enabled(true);
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let parent_id = approved.current.expect("parent").review_unit_id;

    study
        .submit_answer("wrong answer", 1_800)
        .expect("wrong submit");

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let pack = snapshot
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .cloned()
        .expect("remediation pack");
    assert_eq!(pack.status, RemediationPackStatus::Active);

    // Reopen well past the pack's TTL without ever finishing its members:
    // expiry must return the parent on its own, exactly like completion
    // does, instead of stranding it behind an abandoned pack forever.
    let mut resumed =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(after_pack_ttl))
            .expect("resume past ttl")
            .with_remediation_packs_enabled(true);
    let after_ttl = resumed.start().expect("start past ttl");
    let returned = after_ttl.current.expect("parent returns after expiry");
    assert_eq!(returned.review_unit_id, parent_id);

    let resolved = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let resolved_pack = resolved
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .expect("resolved pack");
    assert_eq!(resolved_pack.status, RemediationPackStatus::Expired);
    let parent_row = resolved
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == parent_id)
        .expect("parent row");
    assert!(parent_row
        .snoozed_until
        .is_some_and(|until| until <= after_pack_ttl()));
}

#[test]
fn remediation_pack_state_survives_session_restart() {
    let directory = TempDirectory::new("remediation-pack-restart");
    let path = directory.path().join("study.json");
    let mut study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("open")
        .with_remediation_packs_enabled(true);
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let parent_id = approved.current.expect("parent").review_unit_id;

    study
        .submit_answer("wrong answer", 1_800)
        .expect("wrong submit");

    let before_restart = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let pack_before = before_restart
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .cloned()
        .expect("pack before restart");
    assert_eq!(pack_before.status, RemediationPackStatus::Active);
    assert_eq!(pack_before.review_unit_ids.len(), 2);

    // Simulate a process restart: open a brand-new session over the same
    // persisted file instead of reusing `study`.
    let mut restarted = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now))
        .expect("restart")
        .with_remediation_packs_enabled(true);
    let resumed = restarted.start().expect("start after restart");
    let current = resumed.current.expect("pack member survives restart");
    assert!(
        pack_before
            .review_unit_ids
            .contains(&current.review_unit_id),
        "the restarted session must still surface the durable pack members: {:?}",
        current.review_unit_id
    );
    assert_ne!(current.review_unit_id, parent_id);

    let after_restart = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert_eq!(
        after_restart
            .remediation_packs
            .iter()
            .filter(|pack| pack.parent_review_unit_id == parent_id)
            .count(),
        1,
        "restart must never duplicate the remediation pack"
    );
    let pack_after = after_restart
        .remediation_packs
        .iter()
        .find(|pack| pack.parent_review_unit_id == parent_id)
        .expect("pack after restart");
    assert_eq!(pack_after.id, pack_before.id);
    assert_eq!(pack_after.review_unit_ids, pack_before.review_unit_ids);
    assert_eq!(
        after_restart.generation_runs.len(),
        before_restart.generation_runs.len(),
        "resuming must not regenerate remediation content"
    );
}

#[test]
fn bridge_material_failure_keeps_the_parent_current() {
    let directory = TempDirectory::new("bridge-rejected");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .approve_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("approve exercise");
    let parent = approved.current.expect("parent");

    let failure = study
        .generate_bridge_material_with_provider(&RejectedBridgeProvider)
        .expect_err("flat or harder bridge pack should fail");
    assert!(failure.to_string().contains("no accepted drafts"));

    let view = study.view().expect("view");
    assert_eq!(
        view.current.expect("current").review_unit_id,
        parent.review_unit_id
    );
    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    let parent_row = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == parent.review_unit_id)
        .expect("parent row");
    assert_eq!(parent_row.snoozed_until, None);
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
            project_key: None,
            ttl_expires_at: None,
        })
        .expect("source");

    // Structured-block primary finds nothing in prose, so the model provider runs.
    let model = FakeModelProvider;
    let provider = FallbackProvider::new(&model);
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
fn infers_a_title_when_capture_does_not_provide_one() {
    let directory = TempDirectory::new("capture-title");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");

    let view = study
        .add_source(BetaStudySourceInput {
            id: "src-capture".to_owned(),
            title: String::new(),
            body: "  Mitochondria generate ATP for cells. The second sentence stays body text.  "
                .to_owned(),
            project_key: None,
            ttl_expires_at: None,
        })
        .expect("source");

    assert_eq!(view.sources[0].title, "Mitochondria generate ATP for cells");
    assert_eq!(
        infer_capture_title("\"Hope\" is the thing with feathers -\nThat perches in the soul -"),
        "\"Hope\" is the thing with feathers -"
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
            project_key: None,
            ttl_expires_at: None,
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
        project_key: None,
        ttl_expires_at: None,
    }
}

fn concept_snooze_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-concept-snooze".to_owned(),
        title: "Concept snooze practice".to_owned(),
        body: [
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
            "",
            "Concept: NATO letter B",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for B?",
            "Answer: BRAVO",
            "Distractors: ALFA, CHARLIE",
            "Reference: The NATO phonetic alphabet word for B is BRAVO.",
        ]
        .join("\n"),
        project_key: None,
        ttl_expires_at: None,
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

fn shared_concept_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-shared".to_owned(),
        title: "Shared concept practice".to_owned(),
        body: shared_concept_body(),
        project_key: None,
        ttl_expires_at: None,
    }
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

fn variant_concept_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-variants".to_owned(),
        title: "NATO letter A variants".to_owned(),
        body: [
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
            "Stage: recognition-3",
            "Question: Choose the code word used for the letter A.",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
            "",
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: In radio spelling, which word represents A?",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
        ]
        .join("\n"),
        project_key: None,
        ttl_expires_at: None,
    }
}

fn record_graded_attempt(path: &std::path::Path, draft_id: &str, is_correct: bool, offset: i64) {
    record_graded_attempt_with_response_time(path, draft_id, is_correct, offset, 1_800);
}

fn record_graded_attempt_with_response_time(
    path: &std::path::Path,
    draft_id: &str,
    is_correct: bool,
    offset: i64,
    response_time_ms: u32,
) {
    let store = BetaPersistenceStore::open(path).expect("store");
    let snapshot = store.snapshot();
    let review_unit_id = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| draft.id == draft_id)
        .map(|draft| draft.review_unit_id.clone())
        .or_else(|| {
            snapshot
                .review_units
                .iter()
                .find(|unit| unit.generated_prompt_draft_id.as_deref() == Some(draft_id))
                .map(|unit| unit.review_unit_id.clone())
        })
        .expect("review unit id for draft");
    record_graded_attempt_for_review_unit(
        path,
        &review_unit_id,
        is_correct,
        offset,
        response_time_ms,
    );
}

fn record_graded_attempt_for_review_unit(
    path: &std::path::Path,
    review_unit_id: &ReviewUnitId,
    is_correct: bool,
    offset: i64,
    response_time_ms: u32,
) {
    let mut store = BetaPersistenceStore::open(path).expect("store");
    store
        .record_attempt(ServiceAttemptRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: None,
            submitted_answer: if is_correct { "ALFA" } else { "BRAVO" }.to_owned(),
            response_time_ms,
            occurred_at: NOW + offset,
            idempotency_key: Some(format!("{}-{offset}", review_unit_id.as_str())),
            grade: Some(grade_result(is_correct)),
        })
        .expect("record attempt");
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn grade_result(is_correct: bool) -> GradeResult {
    GradeResult {
        verdict: if is_correct {
            Verdict::Correct
        } else {
            Verdict::Wrong
        },
        rating: if is_correct {
            Rating::Good
        } else {
            Rating::Again
        },
        is_correct,
        submitted_answer: if is_correct { "ALFA" } else { "BRAVO" }.to_owned(),
        expected_answer: "ALFA".to_owned(),
        grader_kind: GraderKind::Deterministic,
        grader_model: None,
        grader_confidence: None,
        feedback: String::new(),
        criterion_results: Vec::new(),
    }
}

fn seed_spanless_review(path: &std::path::Path) {
    let mut store = BetaPersistenceStore::open(path).expect("store");
    let source = SourceDocument {
        id: "src-spanless".to_owned(),
        kind: SourceDocumentKind::Text,
        title: "NATO note".to_owned(),
        project_key: None,
        body: Some("The NATO phonetic alphabet word for A is ALFA.".to_owned()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: None,
    };
    store.save_source_document(source).expect("source");
    let review_unit_id = ReviewUnitId::new("spanless-nato-a");
    let prompt = Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: review_unit_id.clone(),
        prompt: "What is the NATO phonetic alphabet word for A?".to_owned(),
        accepted_answers: vec!["ALFA".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    });
    store
        .save_generated_prompt_draft(GeneratedPromptDraft {
            id: "spanless-draft".to_owned(),
            source_document_ids: vec!["src-spanless".to_owned()],
            reference_span_ids: Vec::new(),
            concept_reference_note_key: None,
            generation_run_id: None,
            review_unit_id: review_unit_id.clone(),
            prompt_id: "spanless-nato-a-prompt".to_owned(),
            prompt: prompt.clone(),
            queue: PersistedQueueCandidate {
                review_unit_id: review_unit_id.clone(),
                due: NOW - 60_000,
                lifecycle: ReviewUnitLifecycle::active(),
                progression: Some(ProgressionMetadata {
                    progression_group: Some("nato-letter-a".to_owned()),
                    stage_order: 1,
                    requires: Vec::new(),
                    supersedes: Vec::new(),
                }),
                concept_key: Some("nato-letter-a".to_owned()),
                source_key: Some("src-spanless".to_owned()),
                domain_key: Some("text".to_owned()),
            },
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            worked_solution: None,
            model: GeneratedPromptModel {
                provider: "fixture".to_owned(),
                name: "manual-spanless".to_owned(),
                version: "v1".to_owned(),
            },
            validation: GeneratedPromptValidation {
                status: GeneratedPromptValidationStatus::Accepted,
                reasons: Vec::new(),
            },
            critique_notes: Vec::new(),
            remediation_pack_id: None,
            created_at: NOW,
        })
        .expect("draft");
    store
        .save_review_unit(BetaReviewUnitRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: "spanless-nato-a-prompt".to_owned(),
            prompt,
            queue: PersistedQueueCandidate {
                review_unit_id,
                due: NOW - 60_000,
                lifecycle: ReviewUnitLifecycle::active(),
                progression: Some(ProgressionMetadata {
                    progression_group: Some("nato-letter-a".to_owned()),
                    stage_order: 1,
                    requires: Vec::new(),
                    supersedes: Vec::new(),
                }),
                concept_key: Some("nato-letter-a".to_owned()),
                source_key: Some("src-spanless".to_owned()),
                domain_key: Some("text".to_owned()),
            },
            reference_span_ids: Vec::new(),
            concept_reference_note_key: None,
            generated_prompt_draft_id: Some("spanless-draft".to_owned()),
            archived_at: None,
            snoozed_until: None,
            remediation_pack_id: None,
            created_at: NOW,
        })
        .expect("review unit");
}

struct PanicReferenceProvider;

impl ReferenceNoteProvider for PanicReferenceProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "panic-reference".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn explain_concept(
        &self,
        _request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        Err(ProviderFailure::new(
            "reference provider should not run when a cached concept note exists",
        ))
    }
}

struct RejectedBridgeProvider;

impl ReferenceNoteProvider for RejectedBridgeProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "rejected-bridge".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn explain_concept(
        &self,
        request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        Ok(ReferenceNoteDraft {
            title: format!("Reference: {}", request.concept_label),
            body: "Use the full original answer.".to_owned(),
        })
    }
}

impl BridgeMaterialProvider for RejectedBridgeProvider {
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        Ok(BridgeMaterial {
            model: ReferenceNoteProvider::model(self),
            reference_note: self.explain_concept(&ReferenceNoteRequest {
                concept_key: request.concept_key.clone(),
                concept_label: request.concept_label.clone(),
                prompt: request.parent_prompt.clone(),
                expected_answer: request.parent_expected_answer.clone(),
                recent_performance: Vec::new(),
            })?,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: request.concept_label.clone(),
                question: request.parent_prompt.clone(),
                answer: request.parent_expected_answer.clone(),
                evidence: None,
                distractors: Vec::new(),
                worked_solution: Some("This repeats the parent.".to_owned()),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "composition".to_owned(),
                unsupported: false,
            }],
            usage: None,
        })
    }
}

fn now() -> i64 {
    NOW
}

fn later() -> i64 {
    NOW + 1_000
}

fn after_snooze() -> i64 {
    NOW + DEFAULT_SNOOZE_DEFER_MS + 1
}

/// Past the fixed first-learning-step relearn interval (1 minute) a wrong
/// answer schedules, but well short of the remediation-pack TTL — proves
/// pack completion returns the parent on its own short relearn schedule
/// instead of waiting out the full defer window.
fn after_relearn() -> i64 {
    NOW + 5 * 60_000
}

/// Past the remediation-pack TTL — proves an abandoned pack expires and
/// returns the parent on its own, rather than staying active forever.
fn after_pack_ttl() -> i64 {
    NOW + DEFAULT_REMEDIATION_PACK_TTL_MS + 1
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
