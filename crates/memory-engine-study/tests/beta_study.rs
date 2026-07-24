use std::{cell::Cell, fs, path::PathBuf};

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
    BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreError, GeneratedLearningActivityKind,
    GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
    GeneratedPromptValidationStatus, PersistedQueueCandidate, SourceDocument, SourceDocumentKind,
    SourcePermission,
};
use memory_engine_service::{MemoryServiceStore, ServiceAttemptRecord};
use memory_engine_study::{
    infer_capture_title, BetaStudyOptions, BetaStudySession, BetaStudySourceInput, BetaStudyStatus,
    DEFAULT_BRIDGE_PARENT_DEFER_MS, DEFAULT_SKIP_DEFER_MS, DEFAULT_SNOOZE_DEFER_MS,
};
use serde_json::json;

const NOW: i64 = 1_779_984_000_000;

#[test]
fn queued_generation_is_pending_before_lease_publication() {
    let directory = TempDirectory::new("queued-pending-publication");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");

    let generated = study
        .generate_with_run_id_pending(None, "queued-run")
        .expect("queued generation");
    assert!(
        generated.drafts.is_empty(),
        "unfinalized drafts must stay off the learner workspace"
    );
    let snapshot = BetaPersistenceStore::open(&path)
        .expect("reload")
        .snapshot();
    let draft_id = snapshot
        .generated_prompt_drafts
        .first()
        .expect("durable pending draft")
        .id
        .clone();
    assert!(snapshot.review_units.is_empty());
    assert!(snapshot.schedules.is_empty());
    assert!(snapshot
        .generation_runs
        .iter()
        .any(|run| run.id == "queued-run" && run.completed_at == Some(i64::MIN)));

    let mut decisions = BetaPersistenceStore::open(&path).expect("decision store");
    assert!(matches!(
        decisions.keep_generated_prompt_draft(&draft_id, NOW),
        Err(BetaStoreError::MissingGenerationRunForAcceptedDraft)
    ));
}

#[test]
fn unfinalized_generation_drafts_are_hidden_and_undecidable() {
    let directory = TempDirectory::new("unfinalized-draft-visibility");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    let generated = study
        .generate_with_run_id_pending(None, "visibility-run")
        .expect("queued generation");
    assert!(generated.drafts.is_empty());
    let draft_id = BetaPersistenceStore::open(&path)
        .expect("reload")
        .snapshot()
        .generated_prompt_drafts
        .first()
        .expect("durable draft")
        .id
        .clone();
    assert!(matches!(
        study.keep_draft(&draft_id),
        Err(memory_engine_study::BetaStudyError::Store(
            BetaStoreError::MissingGenerationRunForAcceptedDraft
        ))
    ));
}

#[test]
fn creates_source_generates_keeps_reviews_reveals_and_advances_queue() {
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    let approved = study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz");
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
            .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
            .expect("keep exercise");
        study
            .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("keep quiz");
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
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep");

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
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep");
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
            .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("keep");
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
        study.keep_draft(&draft_id).expect("keep variant");
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
            .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("keep");
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz");

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
        .keep_draft("study-run-1-draft-src-shared-1-nato-letter-a")
        .expect("keep first");
    let reviewed = study
        .keep_draft("study-run-1-draft-src-shared-2-nato-letter-a")
        .expect("keep second");

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
        .keep_draft("study-run-1-draft-src-shared-1-nato-letter-a")
        .expect("keep first");
    study
        .keep_draft("study-run-1-draft-src-shared-2-nato-letter-a")
        .expect("keep second");

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
            .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
            .expect("keep quiz");
        study
            .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
            .expect("keep exercise");
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
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep");
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz");
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
fn archiving_source_preserves_provider_send_receipt_for_export() {
    let directory = TempDirectory::new("archive-provider-receipt");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");

    let archived = study.archive_source("src-nato").expect("archive source");
    assert_eq!(archived.1, 0);

    let snapshot = BetaPersistenceStore::open(&path).expect("store").snapshot();
    assert!(snapshot.source_documents[0].archived_at.is_some());
    let run = snapshot
        .generation_runs
        .iter()
        .find(|run| run.id == "study-run-1")
        .expect("generation receipt");
    assert_eq!(run.provider, "fixture");
    assert_eq!(run.model, "deterministic-beta-generator");
    assert_eq!(run.prompt_version, "v1");
    assert_eq!(run.source_permissions[0].source_document_id, "src-nato");
    assert!(run.source_permissions[0].consented);
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    let started = study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz");
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
        study.keep_draft(&draft.id).expect("keep");
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
fn local_only_source_blocks_model_reference_generation() {
    let directory = TempDirectory::new("local-only-reference");
    let path = directory.path().join("study.json");
    seed_spanless_review(&path);
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let mut source = store.snapshot().source_documents[0].clone();
    source.permission = SourcePermission::LocalOnly;
    store
        .save_source_document(source)
        .expect("local-only source");

    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.start().expect("start");
    let error = study
        .learn_more_with_provider(&PanicReferenceProvider)
        .expect_err("local-only source must not reach reference provider");

    assert!(error.to_string().contains("Local-only source"));
}

#[test]
fn missing_referenced_source_blocks_reference_provider_invocation() {
    let directory = TempDirectory::new("missing-active-source");
    let path = directory.path().join("study.json");
    seed_spanless_review(&path);

    let store = BetaPersistenceStore::open(&path).expect("store");
    let mut snapshot = store.snapshot();
    snapshot
        .generated_prompt_drafts
        .first_mut()
        .expect("draft")
        .source_document_ids
        .push("missing-source".to_owned());
    fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("persist missing source reference");
    drop(store);

    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.start().expect("start");
    let provider = CountingReferenceProvider::default();
    let error = study
        .learn_more_with_provider(&provider)
        .expect_err("missing referenced source must fail closed");
    assert!(matches!(
        error,
        memory_engine_study::BetaStudyError::Generation(
            memory_engine_generation::BetaGenerationError::UnknownSourceDocument(id)
        ) if id == "missing-source"
    ));
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn unknown_reference_span_fails_closed_before_reference_provider_invocation() {
    let directory = TempDirectory::new("missing-reference-span");
    let path = directory.path().join("study.json");
    seed_spanless_review(&path);
    let store = BetaPersistenceStore::open(&path).expect("store");
    let mut snapshot = store.snapshot();
    snapshot
        .generated_prompt_drafts
        .first_mut()
        .expect("draft")
        .reference_span_ids
        .push("missing-reference-span".to_owned());
    fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("persist missing reference");
    drop(store);

    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.start().expect("start");
    let provider = CountingReferenceProvider::default();
    let error = study
        .learn_more_with_provider(&provider)
        .expect_err("unknown reference span must fail closed");
    assert!(matches!(
        error,
        memory_engine_study::BetaStudyError::UnknownReferenceSpan(id)
            if id == "missing-reference-span"
    ));
    assert_eq!(provider.calls.get(), 0);
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    let approved = study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz sibling");
    let parent_id = approved.current.expect("parent").review_unit_id;

    let bridged = study.generate_bridge_material().expect("bridge");
    if bridged.current.is_some() {
        study
            .snooze_current_until(NOW + DEFAULT_BRIDGE_PARENT_DEFER_MS)
            .expect("defer existing queued sibling");
    }
    let bridge_draft_ids = bridged
        .drafts
        .iter()
        .filter(|draft| draft.review_unit_id.as_str().starts_with("bridge-"))
        .map(|draft| draft.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(bridge_draft_ids.len(), 2);
    assert_eq!(bridged.summary.attempt_count, 0);
    assert_eq!(bridged.summary.approved_review_unit_count, 2);

    study
        .keep_draft(&bridge_draft_ids[0])
        .expect("keep first bridge");
    let bridged = study
        .keep_draft(&bridge_draft_ids[1])
        .expect("keep second bridge");
    let current = bridged.current.expect("kept bridge current");
    assert!(current.review_unit_id.as_str().starts_with("bridge-"));
    assert_eq!(current.activity_stage, "recognition-bridge");
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
fn bridge_descendants_keep_local_only_provenance_and_block_provider_calls() {
    let directory = TempDirectory::new("bridge-local-only-descendant");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    study
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
    let approved = study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep quiz sibling");
    let _parent_id = approved.current.expect("parent").review_unit_id;

    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let mut source = store.snapshot().source_documents[0].clone();
    source.permission = SourcePermission::LocalOnly;
    store
        .save_source_document(source)
        .expect("local-only source");
    drop(store);

    study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("reopen");
    study.start().expect("start");
    let bridged = study.generate_bridge_material().expect("bridge");
    assert!(
        bridged.current.is_some(),
        "bridge generation should keep a current item"
    );

    let provider = CountingBridgeProvider::default();
    let error = study
        .generate_bridge_material_with_provider(&provider)
        .expect_err("local-only bridge descendant must not reach provider");
    assert!(matches!(
        error,
        memory_engine_study::BetaStudyError::Generation(
            memory_engine_generation::BetaGenerationError::LocalOnlySource(id)
        ) if id == "src-nato"
    ));
    assert_eq!(provider.calls.get(), 0);
    assert_eq!(
        study
            .view()
            .expect("view")
            .current
            .expect("current")
            .review_unit_id,
        bridged.current.expect("bridge current").review_unit_id
    );
}

#[test]
fn local_only_source_blocks_model_bridge_generation() {
    let directory = TempDirectory::new("local-only-bridge");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.add_source(source_input()).expect("source");
    study.generate(None).expect("generate");
    let approved = study
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep");
    let parent_id = approved.current.expect("parent").review_unit_id;

    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let mut source = store.snapshot().source_documents[0].clone();
    source.permission = SourcePermission::LocalOnly;
    store
        .save_source_document(source)
        .expect("local-only source");
    drop(store);

    study = BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("reopen");
    study.start().expect("start");

    let error = study
        .generate_bridge_material_with_provider(&RejectedBridgeProvider)
        .expect_err("local-only source must not reach bridge provider");

    assert!(error.to_string().contains("Local-only source"));
    assert_eq!(
        study
            .view()
            .expect("view")
            .current
            .expect("current")
            .review_unit_id,
        parent_id
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
        .keep_draft("study-run-1-draft-src-nato-2-nato-cat-composition")
        .expect("keep exercise");
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
        .keep_draft("study-run-1-draft-src-nato-1-nato-letter-a")
        .expect("keep");
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
            permission: SourcePermission::ModelEligible,
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
            permission: SourcePermission::ModelEligible,
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
            permission: SourcePermission::ModelEligible,
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

#[test]
fn generation_rejects_missing_and_archived_source_ids_instead_of_filtering_them() {
    let directory = TempDirectory::new("generation-source-validation");
    let path = directory.path().join("study.json");
    let mut study =
        BetaStudySession::open(BetaStudyOptions::new(&path).with_clock(now)).expect("open");
    study.start().expect("start");
    study.add_source(source_input()).expect("source");

    let missing = study
        .generate(Some(vec!["missing-source".to_owned()]))
        .expect_err("missing source id must fail closed");
    assert!(matches!(
        missing,
        memory_engine_study::BetaStudyError::Generation(
            memory_engine_generation::BetaGenerationError::UnknownSourceDocument(id)
        ) if id == "missing-source"
    ));

    study.archive_source("src-nato").expect("archive");
    let archived = study
        .generate(Some(vec!["src-nato".to_owned()]))
        .expect_err("archived source id must fail closed");
    assert!(matches!(
        archived,
        memory_engine_study::BetaStudyError::Generation(
            memory_engine_generation::BetaGenerationError::ArchivedSourceDocument(id)
        ) if id == "src-nato"
    ));
}

fn source_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-nato".to_owned(),
        title: "NATO practice notes".to_owned(),
        body: source_body(),
        project_key: None,
        ttl_expires_at: None,
        permission: SourcePermission::ModelEligible,
    }
}

fn concept_snooze_input() -> BetaStudySourceInput {
    BetaStudySourceInput {
        id: "src-concept-snooze".to_owned(),
        title: "Concept snooze practice".to_owned(),
        permission: SourcePermission::ModelEligible,
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
        permission: SourcePermission::ModelEligible,
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
        permission: SourcePermission::ModelEligible,
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
            learner_decision: None,
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
            created_at: NOW,
        })
        .expect("review unit");
}

struct PanicReferenceProvider;

#[derive(Default)]
struct CountingReferenceProvider {
    calls: Cell<usize>,
}

impl ReferenceNoteProvider for CountingReferenceProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "counting-reference".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn explain_concept(
        &self,
        _request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        self.calls.set(self.calls.get() + 1);
        Err(ProviderFailure::new(
            "reference provider must not receive a missing-source request",
        ))
    }
}

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

#[derive(Default)]
struct CountingBridgeProvider {
    calls: Cell<usize>,
}

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
            reference_note: self.explain_concept(&ReferenceNoteRequest::new(
                request.concept_key.clone(),
                request.concept_label.clone(),
                request.parent_prompt.clone(),
                request.parent_expected_answer.clone(),
                Vec::new(),
                request.authorization().clone(),
            ))?,
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

impl ReferenceNoteProvider for CountingBridgeProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "counting-bridge".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn explain_concept(
        &self,
        _request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        self.calls.set(self.calls.get() + 1);
        Err(ProviderFailure::new(
            "bridge provider must not receive a local-only descendant",
        ))
    }
}

impl BridgeMaterialProvider for CountingBridgeProvider {
    fn generate_bridge_material(
        &self,
        _request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        self.calls.set(self.calls.get() + 1);
        Err(ProviderFailure::new(
            "bridge provider must not receive a local-only descendant",
        ))
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
