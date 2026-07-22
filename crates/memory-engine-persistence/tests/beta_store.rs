use memory_engine_core::{
    ExactPrompt, ExactPromptKind, Prompt, QueueCandidate, Rating, ReviewUnitId,
    ReviewUnitLifecycle, ScheduleState, ScheduleStatus,
};
use memory_engine_persistence::{
    ApproveGeneratedPromptDraftOptions, BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreError,
    ConceptReferenceNote, GeneratedLearningActivityKind, GeneratedPromptDraft,
    GeneratedPromptModel, GeneratedPromptValidation, GeneratedPromptValidationStatus,
    GenerationRun, PersistedQueueCandidate, ReferenceSpan, SourceDocument, SourceDocumentKind,
    SourcePermission, SourcePermissionReceipt,
};
use memory_engine_service::{
    record_content_feedback, ContentFeedback, ContentFeedbackSource, ContentFeedbackVerdict,
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, RecordContentFeedbackCommand,
    ServiceAttemptRecord, ServiceError,
};

const NOW: i64 = 1_779_989_400_000;

#[test]
fn reads_legacy_source_snapshot_without_permission_as_model_eligible() {
    let directory = TempDirectory::new("legacy-permission");
    let path = directory.path().join("store.json");
    fs::write(
        &path,
        r#"{"version":1,"sourceDocuments":[{"id":"legacy-source","kind":"text","title":"Legacy source","body":"old notes","uri":null,"freshness":1779989400000,"createdAt":1779989400000}],"referenceSpans":[],"generatedPromptDrafts":[],"reviewUnits":[],"schedules":[],"attempts":[],"generationRuns":[],"appliedReviews":[],"conceptReferenceNotes":[]}"#,
    )
    .expect("legacy snapshot");

    let store = BetaPersistenceStore::open(path).expect("legacy source loads");
    assert_eq!(
        store.snapshot().source_documents[0].permission,
        SourcePermission::ModelEligible
    );
}

#[test]
fn updates_source_permission_but_rejects_archived_sources() {
    let directory = TempDirectory::new("permission-update");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(path).expect("open store");
    store
        .save_source_document(source_document("editable-source"))
        .expect("source");

    let updated = store
        .update_source_document_permission("editable-source", SourcePermission::LocalOnly)
        .expect("permission update");
    assert_eq!(updated.permission, SourcePermission::LocalOnly);
    assert_eq!(
        store.snapshot().source_documents[0].permission,
        SourcePermission::LocalOnly
    );
    store
        .archive_source_document("editable-source", NOW)
        .expect("archive");
    assert_eq!(
        store.update_source_document_permission("editable-source", SourcePermission::ModelEligible),
        Err(BetaStoreError::SourceDocumentArchived(
            "editable-source".to_owned()
        ))
    );
}

#[test]
fn persists_sources_drafts_reviews_attempts_and_queue_across_reload() {
    let directory = TempDirectory::new("persist-reload");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");

    let source = store
        .save_source_document(source_document("src-latin-prayer"))
        .expect("source");
    let reference = store
        .save_reference_span(reference_span("ref-pater", &source.id))
        .expect("reference");
    let draft = store
        .save_generated_prompt_draft(accepted_draft(
            "draft-pater",
            "beta-pater-noster",
            &[source.id.as_str()],
            &[reference.id.as_str()],
            Some("run-pater"),
        ))
        .expect("draft");
    store
        .save_generation_run(generation_run(
            "run-pater",
            &[source.id.as_str()],
            &[draft.id.as_str()],
        ))
        .expect("run");
    store
        .approve_generated_prompt_draft(
            &draft.id,
            ApproveGeneratedPromptDraftOptions {
                initial_schedule_state: Some(schedule_state(2, ScheduleStatus::Review)),
            },
        )
        .expect("approve");

    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);
    let review = service
        .grade_apply_review(GradeApplyReviewCommand {
            prompt: draft.prompt.clone(),
            submitted_answer: "Our Father".to_owned(),
            response_time_ms: 1_800,
            prompt_id: Some(draft.prompt_id.clone()),
            occurred_at: Some(NOW),
            idempotency_key: None,
        })
        .expect("review");

    assert_eq!(review.grade.rating, Rating::Good);
    assert_eq!(review.schedule_state.reps, 3);

    let reloaded = BetaPersistenceStore::open(&path).expect("reload");
    let snapshot = reloaded.snapshot();
    let queue = reloaded.list_queue_candidates().expect("queue");

    assert_eq!(snapshot.source_documents, [source]);
    assert_eq!(snapshot.reference_spans, [reference]);
    assert_eq!(snapshot.generated_prompt_drafts, [draft]);
    assert_eq!(snapshot.generation_runs.len(), 1);
    assert_eq!(snapshot.attempts, [review.attempt]);
    assert_eq!(
        snapshot.schedules,
        [memory_engine_persistence::ScheduleRecord {
            review_unit_id: review_unit_id("beta-pater-noster"),
            state: review.schedule_state.clone(),
        }]
    );
    assert_eq!(
        queue,
        [QueueCandidate {
            review_unit_id: review_unit_id("beta-pater-noster"),
            schedule_state: Some(review.schedule_state.clone()),
            due: review.schedule_state.due,
            lifecycle: ReviewUnitLifecycle::active(),
            progression: None,
            concept_key: Some("lords-prayer-opening".to_owned()),
            source_key: Some("latin-prayer-note".to_owned()),
            domain_key: Some("latin".to_owned()),
        }]
    );
}

#[test]
fn rejects_duplicate_reviews_and_failed_commits_without_corrupting_history() {
    let directory = TempDirectory::new("duplicate-safe");
    let path = directory.path().join("store.json");
    let unit_id = review_unit_id("beta-duplicate-safe");
    let prompt = short_answer_prompt(&unit_id, "Translate: Pater noster");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_review_unit(review_unit(
            &unit_id,
            "duplicate-safe-prompt",
            prompt.clone(),
            queue_candidate(&unit_id, NOW - 60_000),
        ))
        .expect("review unit");

    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);
    let first_review = service
        .grade_apply_review(GradeApplyReviewCommand {
            prompt: prompt.clone(),
            submitted_answer: "Our Father".to_owned(),
            response_time_ms: 1_800,
            prompt_id: Some("duplicate-safe-prompt".to_owned()),
            occurred_at: Some(NOW),
            idempotency_key: None,
        })
        .expect("first review");

    let duplicate = service
        .grade_apply_review(GradeApplyReviewCommand {
            prompt: prompt.clone(),
            submitted_answer: "Our Father".to_owned(),
            response_time_ms: 1_800,
            prompt_id: Some("duplicate-safe-prompt".to_owned()),
            occurred_at: Some(NOW),
            idempotency_key: None,
        })
        .expect_err("duplicate should fail");
    assert!(matches!(
        duplicate,
        ServiceError::Store(BetaStoreError::DuplicateAppliedReview(_))
    ));

    let mut store = service.into_store();
    assert_eq!(
        store.snapshot().attempts.as_slice(),
        std::slice::from_ref(&first_review.attempt)
    );
    assert_eq!(store.snapshot().schedules.len(), 1);

    store.fail_next_commit_for_test();
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);
    let failed = service
        .grade_apply_review(GradeApplyReviewCommand {
            prompt,
            submitted_answer: "Our Father".to_owned(),
            response_time_ms: 2_100,
            prompt_id: Some("duplicate-safe-prompt".to_owned()),
            occurred_at: Some(NOW + 1_000),
            idempotency_key: None,
        })
        .expect_err("commit failure");
    assert_eq!(
        failed,
        ServiceError::Store(BetaStoreError::InjectedCommitFailure)
    );

    let reloaded = BetaPersistenceStore::open(&path).expect("reload");
    assert_eq!(reloaded.snapshot().attempts, [first_review.attempt]);
    assert_eq!(reloaded.snapshot().schedules.len(), 1);
}

#[test]
fn stale_store_review_write_loses_without_overwriting_the_winner() {
    let directory = TempDirectory::new("same-row-review-race");
    let path = directory.path().join("store.json");
    let unit_id = review_unit_id("beta-same-row-review-race");
    let prompt = short_answer_prompt(&unit_id, "Translate: Pater noster");
    let prior_schedule = schedule_state(2, ScheduleStatus::Review);
    let mut winner = BetaPersistenceStore::open(&path).expect("open winner");
    winner
        .save_review_unit(review_unit(
            &unit_id,
            "same-row-review-prompt",
            prompt,
            queue_candidate(&unit_id, NOW - 60_000),
        ))
        .expect("review unit");
    winner
        .set_schedule_state(&unit_id, Some(prior_schedule.clone()))
        .expect("initial schedule");
    let mut stale = BetaPersistenceStore::open(&path).expect("open stale writer");

    let winning_attempt = attempt(&unit_id, NOW, "winner");
    let mut winning_schedule = schedule_state(3, ScheduleStatus::Review);
    winning_schedule.last_review = Some(NOW);
    winner
        .apply_review(
            &unit_id,
            winning_attempt.clone(),
            winning_schedule.clone(),
            Some(prior_schedule.clone()),
        )
        .expect("winner commits");

    let stale_attempt = attempt(&unit_id, NOW + 1_000, "stale");
    let mut stale_schedule = schedule_state(3, ScheduleStatus::Review);
    stale_schedule.last_review = Some(NOW + 1_000);
    assert_eq!(
        stale.apply_review(
            &unit_id,
            stale_attempt,
            stale_schedule,
            Some(prior_schedule),
        ),
        Err(BetaStoreError::StaleScheduleWrite(unit_id.clone()))
    );

    let snapshot = BetaPersistenceStore::open(&path)
        .expect("reload")
        .snapshot();
    assert_eq!(snapshot.attempts, [winning_attempt]);
    assert_eq!(snapshot.applied_reviews.len(), 1);
    assert_eq!(snapshot.schedules[0].state, winning_schedule);
}

#[test]
fn advisory_lock_handoff_ignores_the_orphaned_lock_file() {
    use std::{fs::OpenOptions, sync::mpsc, thread, time::Duration};

    let directory = TempDirectory::new("advisory-lock-handoff");
    let path = directory.path().join("store.json");
    let lock_path = path.with_extension("lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open lock file");
    lock.lock().expect("hold advisory lock");

    let (started_tx, started_rx) = mpsc::channel();
    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        let mut store = BetaPersistenceStore::open(&writer_path).expect("open waiting store");
        started_tx.send(()).expect("announce writer");
        store.save_source_document(source_document("src-after-lock-handoff"))
    });
    started_rx.recv().expect("writer started");
    thread::sleep(Duration::from_millis(25));
    assert!(!writer.is_finished(), "writer must wait for the live owner");

    lock.unlock().expect("release advisory lock");
    writer
        .join()
        .expect("writer thread")
        .expect("writer acquires released lock");
    assert!(lock_path.exists(), "an unlocked lock file is harmless");
}

#[test]
fn concept_snooze_commit_failure_preserves_every_member_and_history() {
    let directory = TempDirectory::new("concept-snooze-atomic");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    let matching_ids = [
        review_unit_id("concept-snooze-a"),
        review_unit_id("concept-snooze-b"),
    ];
    let non_member_id = review_unit_id("concept-snooze-other");

    for (index, review_unit_id) in matching_ids.iter().enumerate() {
        let mut queue = queue_candidate(review_unit_id, NOW - 60_000);
        queue.concept_key = Some("shared-concept".to_owned());
        store
            .save_review_unit(review_unit(
                review_unit_id,
                "concept-snooze-prompt",
                short_answer_prompt(review_unit_id, "What is the answer?"),
                queue,
            ))
            .expect("matching review unit");
        store
            .set_schedule_state(
                review_unit_id,
                Some(schedule_state(
                    u32::try_from(index + 1).expect("reps"),
                    ScheduleStatus::Review,
                )),
            )
            .expect("matching schedule");
    }
    store
        .save_review_unit(review_unit(
            &non_member_id,
            "concept-snooze-prompt",
            short_answer_prompt(&non_member_id, "What is the answer?"),
            queue_candidate(&non_member_id, NOW - 60_000),
        ))
        .expect("non-member review unit");

    let before = store.snapshot();
    store.fail_next_commit_for_test();
    assert_eq!(
        store
            .snooze_review_units_for_concept_until("shared-concept", NOW + 86_400_000)
            .expect_err("the injected commit must fail"),
        BetaStoreError::InjectedCommitFailure
    );
    assert_eq!(
        store.snapshot(),
        before,
        "a failed concept snooze must not leave any member partially updated"
    );
}

#[test]
fn concept_snooze_preserves_nonblank_whitespace_key_exactly() {
    let directory = TempDirectory::new("concept-snooze-whitespace-key");
    let path = directory.path().join("store.json");
    let persisted_key = "  shared-concept  ";
    let matching_id = review_unit_id("concept-snooze-whitespace-match");
    let other_id = review_unit_id("concept-snooze-whitespace-other");
    let mut matching_queue = queue_candidate(&matching_id, NOW - 60_000);
    matching_queue.concept_key = Some(persisted_key.to_owned());
    let mut other_queue = queue_candidate(&other_id, NOW - 60_000);
    other_queue.concept_key = Some("shared-concept".to_owned());
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_review_unit(review_unit(
            &matching_id,
            "whitespace-prompt",
            short_answer_prompt(&matching_id, "What?"),
            matching_queue,
        ))
        .expect("matching unit");
    store
        .save_review_unit(review_unit(
            &other_id,
            "other-prompt",
            short_answer_prompt(&other_id, "What else?"),
            other_queue,
        ))
        .expect("other unit");

    let snoozed = store
        .snooze_review_units_for_concept_until(persisted_key, NOW + 86_400_000)
        .expect("exact persisted key snooze");
    assert_eq!(snoozed.len(), 1);
    assert_eq!(snoozed[0].review_unit_id, matching_id);
    assert_eq!(snoozed[0].queue.concept_key.as_deref(), Some(persisted_key));
    assert_eq!(store.snapshot().review_units[1].snoozed_until, None);
}

#[test]
fn reloads_queue_projection_with_schedule_due_and_progression_metadata() {
    let directory = TempDirectory::new("queue-projection");
    let path = directory.path().join("store.json");
    let stage_one = review_unit_id("catechism-commandments-worked-example");
    let stage_two = review_unit_id("catechism-commandments-free-recall");
    let due_review = review_unit_id("latin-psalm-due-review");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");

    let mut first_stage_queue = queue_candidate(&stage_one, NOW + 86_400_000);
    first_stage_queue.progression = Some(memory_engine_core::ProgressionMetadata {
        progression_group: Some("ten-commandments".to_owned()),
        stage_order: 1,
        requires: Vec::new(),
        supersedes: Vec::new(),
    });
    store
        .save_review_unit(review_unit(
            &stage_one,
            "commandments-worked",
            short_answer_prompt(&stage_one, "What is the first commandment?"),
            first_stage_queue,
        ))
        .expect("stage one");
    store
        .set_schedule_state(&stage_one, Some(schedule_state(4, ScheduleStatus::Review)))
        .expect("stage one schedule");

    let mut second_stage_queue = queue_candidate(&stage_two, NOW - 300_000);
    second_stage_queue.progression = Some(memory_engine_core::ProgressionMetadata {
        progression_group: Some("ten-commandments".to_owned()),
        stage_order: 2,
        requires: vec![stage_one.clone()],
        supersedes: vec![stage_one.clone()],
    });
    store
        .save_review_unit(review_unit(
            &stage_two,
            "commandments-recall",
            short_answer_prompt(&stage_two, "Recall the first commandment."),
            second_stage_queue,
        ))
        .expect("stage two");
    store
        .save_review_unit(review_unit(
            &due_review,
            "psalm-due",
            short_answer_prompt(&due_review, "Translate the psalm phrase."),
            queue_candidate(&due_review, NOW - 3_600_000),
        ))
        .expect("due review");
    store
        .set_schedule_state(
            &due_review,
            Some(ScheduleState {
                due: NOW - 3_600_000,
                ..schedule_state(5, ScheduleStatus::Review)
            }),
        )
        .expect("due schedule");

    let reloaded = BetaPersistenceStore::open(&path).expect("reload");
    let queue = reloaded.list_queue_candidates().expect("queue");

    assert_eq!(queue.len(), 3);
    assert_eq!(
        queue
            .iter()
            .find(|candidate| candidate.review_unit_id == due_review)
            .expect("due")
            .due,
        NOW - 3_600_000
    );
    assert_eq!(
        queue
            .iter()
            .find(|candidate| candidate.review_unit_id == stage_two)
            .and_then(|candidate| candidate.progression.as_ref())
            .expect("progression")
            .requires,
        [stage_one]
    );
}

#[test]
fn validates_generated_drafts_before_promotion() {
    let directory = TempDirectory::new("draft-validation");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_source_document(source_document("src-generated"))
        .expect("source");
    store
        .save_reference_span(reference_span("ref-generated", "src-generated"))
        .expect("reference");

    let rejected = GeneratedPromptDraft {
        validation: GeneratedPromptValidation {
            status: GeneratedPromptValidationStatus::Rejected,
            reasons: vec!["Unsupported claim.".to_owned()],
        },
        ..accepted_draft(
            "draft-rejected",
            "rejected-unit",
            &["src-generated"],
            &["ref-generated"],
            None,
        )
    };
    store
        .save_generated_prompt_draft(rejected.clone())
        .expect("rejected draft persists");
    assert_eq!(
        store
            .approve_generated_prompt_draft(
                &rejected.id,
                ApproveGeneratedPromptDraftOptions::default()
            )
            .expect_err("cannot approve rejected"),
        BetaStoreError::RejectedGeneratedPromptDraft
    );

    let missing_reference = GeneratedPromptDraft {
        id: "draft-missing-reference".to_owned(),
        reference_span_ids: vec!["missing-reference".to_owned()],
        ..accepted_draft(
            "draft-missing-reference",
            "accepted-missing-reference",
            &["src-generated"],
            &["ref-generated"],
            Some("missing-run"),
        )
    };
    assert_eq!(
        store
            .save_generated_prompt_draft(missing_reference)
            .expect_err("missing reference"),
        BetaStoreError::UnknownReferenceSpan("missing-reference".to_owned())
    );

    let source_backed_without_span = accepted_draft(
        "draft-generated-without-reference",
        "generated-without-reference",
        &["src-generated"],
        &[],
        Some("run-without-reference"),
    );
    assert_eq!(
        store
            .save_generated_prompt_draft(source_backed_without_span)
            .expect_err("generated source draft requires reference span"),
        BetaStoreError::GeneratedPromptDraftRequiresReference
    );

    save_concept_backed_bridge_draft(&mut store);

    let accepted_without_run = accepted_draft(
        "draft-missing-generation-run",
        "accepted-without-run",
        &["src-generated"],
        &["ref-generated"],
        Some("missing-run"),
    );
    store
        .save_generated_prompt_draft(accepted_without_run.clone())
        .expect("accepted draft");
    assert_eq!(
        store
            .approve_generated_prompt_draft(
                &accepted_without_run.id,
                ApproveGeneratedPromptDraftOptions::default()
            )
            .expect_err("missing generation run"),
        BetaStoreError::MissingGenerationRunForAcceptedDraft
    );
}

#[test]
fn revises_snoozes_and_archives_review_units_without_rewriting_schedule_history() {
    let directory = TempDirectory::new("lifecycle");
    let path = directory.path().join("store.json");
    let (mut store, draft) = lifecycle_store(&path);

    let updated = store
        .update_review_unit_prompt_text(
            &draft.review_unit_id,
            "Translate the opening prayer.",
            "Our Father",
        )
        .expect("revise");
    assert!(matches!(updated.prompt, Prompt::Exact(_)));
    let snapshot = store.snapshot();
    assert_eq!(
        prompt_text(
            &snapshot
                .review_units
                .iter()
                .find(|unit| unit.review_unit_id == draft.review_unit_id)
                .expect("review unit")
                .prompt
        ),
        "Translate the opening prayer."
    );
    assert_eq!(
        snapshot
            .generated_prompt_drafts
            .iter()
            .find(|candidate| candidate.id == draft.id)
            .expect("draft")
            .critique_notes
            .last()
            .map(String::as_str),
        Some("Learner edited approved wording.")
    );
    assert_eq!(
        prompt_text(
            &snapshot
                .generated_prompt_drafts
                .iter()
                .find(|candidate| candidate.id == draft.id)
                .expect("draft")
                .prompt
        ),
        "Translate the opening prayer."
    );

    let prior_schedule = store
        .read_schedule_state(&draft.review_unit_id)
        .expect("read schedule")
        .expect("schedule");
    store
        .snooze_review_unit_until(&draft.review_unit_id, NOW + 86_400_000)
        .expect("snooze");
    assert_eq!(
        store
            .read_schedule_state(&draft.review_unit_id)
            .expect("read schedule")
            .expect("schedule"),
        prior_schedule
    );
    let queue = store.list_queue_candidates().expect("queue");
    assert_eq!(queue[0].due, NOW + 86_400_000);
    assert_eq!(queue[0].schedule_state, Some(prior_schedule.clone()));

    store
        .archive_review_unit(&draft.review_unit_id, NOW + 1_000)
        .expect("archive");
    assert!(store.list_queue_candidates().expect("queue").is_empty());
    assert_eq!(store.snapshot().review_units.len(), 1);
    assert_eq!(
        store
            .read_schedule_state(&draft.review_unit_id)
            .expect("read schedule")
            .expect("schedule"),
        prior_schedule
    );
}

#[test]
fn content_feedback_is_append_only_latest_wins_and_resolves_generation_provenance() {
    let directory = TempDirectory::new("content-feedback");
    let path = directory.path().join("store.json");
    let (mut store, draft) = lifecycle_store(&path);
    let first = record_content_feedback(
        &mut store,
        RecordContentFeedbackCommand {
            feedback_id: "feedback-first".to_owned(),
            review_unit_id: draft.review_unit_id.clone(),
            verdict: ContentFeedbackVerdict::Dropped,
            rationale: Some("The prompt is ambiguous.".to_owned()),
            account_id: "account-feedback".to_owned(),
            occurred_at: NOW,
            supersedes_id: None,
        },
    )
    .expect("first feedback");
    let replay = record_content_feedback(
        &mut store,
        RecordContentFeedbackCommand {
            feedback_id: first.id.clone(),
            review_unit_id: draft.review_unit_id.clone(),
            verdict: ContentFeedbackVerdict::Dropped,
            rationale: Some("The prompt is ambiguous.".to_owned()),
            account_id: "account-feedback".to_owned(),
            occurred_at: NOW,
            supersedes_id: None,
        },
    )
    .expect("replay feedback");
    assert_eq!(replay, first);

    record_content_feedback(
        &mut store,
        RecordContentFeedbackCommand {
            feedback_id: "feedback-revision".to_owned(),
            review_unit_id: draft.review_unit_id.clone(),
            verdict: ContentFeedbackVerdict::Kept,
            rationale: None,
            account_id: "account-feedback".to_owned(),
            occurred_at: NOW + 1_000,
            supersedes_id: Some(first.id),
        },
    )
    .expect("revision feedback");

    let snapshot = store.snapshot();
    assert_eq!(snapshot.content_feedback.len(), 2);
    let exported = store.export_content_feedback().expect("export feedback");
    assert_eq!(exported.len(), 1);
    assert!(exported[0].human_keep);
    assert!(exported[0].judge_keep);
    assert_eq!(exported[0].gen_ai_request_model, "deterministic-draft");
    assert_eq!(exported[0].gen_ai_system, "fixture");
    assert_eq!(exported[0].gen_ai_prompt_version, "v1");
    assert_eq!(exported[0].question, "Translate: Pater noster");
    assert!(exported[0].fixture.is_none());
    assert!(store
        .export_content_feedback_json()
        .expect("export json")
        .contains("gen_ai.prompt.version"));

    let reloaded = BetaPersistenceStore::open(&path).expect("reload store");
    assert_eq!(reloaded.snapshot().content_feedback.len(), 2);
}

#[test]
fn stale_store_commits_preserve_archives_schedule_and_new_foreground_output() {
    let directory = TempDirectory::new("stale-merge");
    let path = directory.path().join("store.json");
    let (mut stale_store, draft) = lifecycle_store(&path);
    let source_id = stale_store.snapshot().source_documents[0].id.clone();
    let review_unit_id = draft.review_unit_id.clone();

    let mut fresh_store = BetaPersistenceStore::open(&path).expect("open fresh store");
    fresh_store
        .archive_source_document(&source_id, NOW + 1_000)
        .expect("archive source");
    fresh_store
        .archive_review_unit(&review_unit_id, NOW + 2_000)
        .expect("archive review unit");
    fresh_store
        .set_schedule_state(&review_unit_id, None)
        .expect("clear schedule");

    let run = generation_run("run-stale-merge", &[source_id.as_str()], &[]);
    stale_store
        .save_generation_run(run.clone())
        .expect("save generation run");
    record_content_feedback(
        &mut stale_store,
        RecordContentFeedbackCommand {
            feedback_id: "feedback-stale-merge".to_owned(),
            review_unit_id: review_unit_id.clone(),
            verdict: ContentFeedbackVerdict::Kept,
            rationale: Some("Foreground output should survive stale merges.".to_owned()),
            account_id: "account-stale-merge".to_owned(),
            occurred_at: NOW + 3_000,
            supersedes_id: None,
        },
    )
    .expect("save content feedback");

    let reloaded = BetaPersistenceStore::open(&path).expect("reload store");
    let snapshot = reloaded.snapshot();
    assert_eq!(
        snapshot
            .source_documents
            .iter()
            .find(|source| source.id == source_id)
            .and_then(|source| source.archived_at),
        Some(NOW + 1_000)
    );
    assert_eq!(
        snapshot
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == review_unit_id)
            .and_then(|unit| unit.archived_at),
        Some(NOW + 2_000)
    );
    assert!(
        reloaded
            .read_schedule_state(&review_unit_id)
            .expect("read schedule")
            .is_none(),
        "stale commit must not resurrect a cleared schedule"
    );
    assert!(snapshot
        .generation_runs
        .iter()
        .any(|item| item.id == run.id));
    assert!(snapshot
        .content_feedback
        .iter()
        .any(|item| item.id == "feedback-stale-merge"));
}

#[test]
fn nested_store_commit_creates_parent_before_lock_acquisition() {
    let directory = TempDirectory::new("nested-lock-parent");
    let path = directory.path().join("nested").join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("open store");

    store
        .save_source_document(source_document("src-nested-lock"))
        .expect("save source");

    assert!(path.parent().expect("nested dir").exists());
    assert!(path.exists());
    assert_eq!(
        BetaPersistenceStore::open(&path)
            .expect("reload store")
            .snapshot()
            .source_documents
            .len(),
        1
    );
}

#[test]
fn file_feedback_rejects_forks_and_keeps_out_of_order_revision_as_the_head() {
    let directory = TempDirectory::new("content-feedback-fork");
    let path = directory.path().join("store.json");
    let (mut store, draft) = lifecycle_store(&path);
    let first = feedback("feedback-fork-root", &draft.review_unit_id, NOW, None);
    store.record_content_feedback(first.clone()).expect("root");
    let second = feedback(
        "feedback-fork-second",
        &draft.review_unit_id,
        NOW + 2_000,
        Some(&first.id),
    );
    store
        .record_content_feedback(second.clone())
        .expect("second");

    let fork = feedback(
        "feedback-fork-third",
        &draft.review_unit_id,
        NOW + 3_000,
        Some(&first.id),
    );
    assert_eq!(
        store.record_content_feedback(fork),
        Err(BetaStoreError::FeedbackSupersedesStale {
            expected_head: Some(second.id.clone()),
            supplied_parent: Some(first.id.clone()),
        })
    );

    let late = feedback(
        "feedback-fork-late",
        &draft.review_unit_id,
        NOW - 1_000,
        Some(&second.id),
    );
    store
        .record_content_feedback(late.clone())
        .expect("late revision");
    let exported = store.export_content_feedback().expect("export");
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].feedback_id, late.id);
}

#[test]
fn file_feedback_concurrent_instances_are_idempotent_and_do_not_fork_heads() {
    let directory = TempDirectory::new("content-feedback-race");
    let path = directory.path().join("store.json");
    let (mut setup, draft) = lifecycle_store(&path);
    let root = feedback("feedback-race-root", &draft.review_unit_id, NOW, None);
    setup.record_content_feedback(root.clone()).expect("root");
    drop(setup);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        let feedback = feedback(
            "feedback-race-same-id",
            &draft.review_unit_id,
            NOW + 1_000,
            Some(&root.id),
        );
        handles.push(std::thread::spawn(move || {
            let mut store = BetaPersistenceStore::open(path).expect("open race store");
            barrier.wait();
            store.record_content_feedback(feedback)
        }));
    }
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("race worker"))
        .collect::<Vec<_>>();
    assert!(
        results.iter().all(Result::is_ok),
        "same id must replay: {results:?}"
    );

    let final_store = BetaPersistenceStore::open(path).expect("reload race store");
    assert_eq!(final_store.snapshot().content_feedback.len(), 2);
    assert_eq!(
        final_store.export_content_feedback().expect("export").len(),
        1
    );
}

#[test]
fn dropped_local_only_source_is_redacted_and_fixture_preserves_activity_kind() {
    let directory = TempDirectory::new("content-feedback-privacy");
    let path = directory.path().join("store.json");
    let (mut store, draft) = lifecycle_store(&path);
    let mut local = store.snapshot().source_documents[0].clone();
    local.permission = SourcePermission::LocalOnly;
    store.save_source_document(local).expect("local source");
    let mut exercise = draft.clone();
    exercise.activity_kind = GeneratedLearningActivityKind::Exercise;
    store
        .save_generated_prompt_draft(exercise)
        .expect("exercise draft");
    store
        .record_content_feedback(feedback(
            "feedback-private",
            &draft.review_unit_id,
            NOW,
            None,
        ))
        .expect("feedback");

    let exported = store.export_content_feedback().expect("export");
    let fixture = exported[0].fixture.as_ref().expect("fixture");
    assert_eq!(fixture.expect.required_activity_kinds, ["exercise"]);
    assert_eq!(fixture.title, "[redacted local-only source]");
    assert_eq!(fixture.body, "[redacted local-only source]");
    assert!(!store
        .export_content_feedback_json()
        .expect("json")
        .contains("Pater noster means"));
}

fn feedback(
    id: &str,
    review_unit_id: &ReviewUnitId,
    occurred_at: i64,
    supersedes_id: Option<&str>,
) -> ContentFeedback {
    ContentFeedback {
        id: id.to_owned(),
        review_unit_id: review_unit_id.clone(),
        verdict: ContentFeedbackVerdict::Dropped,
        rationale: None,
        source: ContentFeedbackSource::Human,
        account_id: "account-feedback".to_owned(),
        occurred_at,
        supersedes_id: supersedes_id.map(str::to_owned),
    }
}

#[test]
fn stale_review_unit_save_preserves_newer_prompt_lifecycle_and_snooze() {
    let directory = TempDirectory::new("stale-review-unit-save");
    let path = directory.path().join("store.json");
    let unit_id = review_unit_id("stale-review-unit");
    let original = review_unit(
        &unit_id,
        "stale-review-prompt",
        short_answer_prompt(&unit_id, "Original prompt"),
        queue_candidate(&unit_id, NOW - 60_000),
    );
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_review_unit(original.clone())
        .expect("save original review unit");

    let newer_prompt = store
        .update_review_unit_prompt_text(&unit_id, "Newer prompt", "Newer answer")
        .expect("newer prompt");
    let newer_lifecycle = store
        .set_review_unit_lifecycle(
            &unit_id,
            ReviewUnitLifecycle::ttl_expires_at(NOW + 86_400_000),
        )
        .expect("newer lifecycle");
    let newer_snooze = store
        .snooze_review_unit_until(&unit_id, NOW + 20_000)
        .expect("newer snooze");

    let mut stale = original;
    stale.snoozed_until = Some(NOW + 10_000);
    store
        .save_review_unit(stale)
        .expect("stale save is an idempotent no-op");

    let persisted = store
        .snapshot()
        .review_units
        .into_iter()
        .find(|unit| unit.review_unit_id == unit_id)
        .expect("persisted review unit");
    assert_eq!(persisted.prompt, newer_prompt.prompt);
    assert_eq!(persisted.queue.lifecycle, newer_lifecycle.queue.lifecycle);
    assert_eq!(persisted.snoozed_until, newer_snooze.snoozed_until);
}

#[test]
fn mcq_prompt_edit_keeps_new_answer_in_choices() {
    let directory = TempDirectory::new("mcq-prompt-edit");
    let path = directory.path().join("store.json");
    let unit_id = review_unit_id("mcq-prompt-edit");
    let prompt = Prompt::Mcq {
        review_unit_id: unit_id.clone(),
        prompt: "Original prompt".to_owned(),
        choices: vec!["Original answer".to_owned(), "Distractor".to_owned()],
        correct_choice: "Original answer".to_owned(),
    };
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_review_unit(review_unit(
            &unit_id,
            "mcq-prompt-edit",
            prompt,
            queue_candidate(&unit_id, NOW - 60_000),
        ))
        .expect("save MCQ review unit");

    let updated = store
        .update_review_unit_prompt_text(&unit_id, "Edited prompt", "Edited answer")
        .expect("edit MCQ prompt");
    match updated.prompt {
        Prompt::Mcq {
            choices,
            correct_choice,
            ..
        } => {
            assert_eq!(correct_choice, "Edited answer");
            assert!(choices.iter().any(|choice| choice == "Edited answer"));
        }
        prompt => panic!("expected edited MCQ prompt, got {prompt:?}"),
    }
}

#[test]
fn boolean_prompt_edit_rejects_invalid_answer_without_mutation() {
    let directory = TempDirectory::new("boolean-prompt-edit");
    let path = directory.path().join("store.json");
    let unit_id = review_unit_id("boolean-prompt-edit");
    let prompt = Prompt::Boolean {
        review_unit_id: unit_id.clone(),
        prompt: "Is ALFA the NATO word for A?".to_owned(),
        correct_answer: true,
    };
    let mut store = BetaPersistenceStore::open(&path).expect("open store");
    store
        .save_review_unit(review_unit(
            &unit_id,
            "boolean-prompt-edit",
            prompt,
            queue_candidate(&unit_id, NOW - 60_000),
        ))
        .expect("save Boolean review unit");
    let before = store.snapshot();

    assert!(matches!(
        store.update_review_unit_prompt_text(&unit_id, "Changed prompt", "maybe"),
        Err(BetaStoreError::InvalidBooleanAnswer)
    ));
    assert_eq!(store.snapshot(), before);

    let updated = store
        .update_review_unit_prompt_text(&unit_id, "Changed prompt", "  FALSE ")
        .expect("trimmed case-insensitive false");
    assert!(matches!(
        updated.prompt,
        Prompt::Boolean {
            correct_answer: false,
            ..
        }
    ));
}

#[test]
fn repeated_draft_approval_returns_existing_record_without_rewriting_schedule() {
    let directory = TempDirectory::new("repeat-approval");
    let path = directory.path().join("store.json");
    let (mut store, draft) = lifecycle_store(&path);
    let unit_id = draft.review_unit_id.clone();
    let newer_prompt = store
        .update_review_unit_prompt_text(&unit_id, "Newer prompt", "Newer answer")
        .expect("newer prompt");
    let newer_lifecycle = store
        .set_review_unit_lifecycle(
            &unit_id,
            ReviewUnitLifecycle::ttl_expires_at(NOW + 86_400_000),
        )
        .expect("newer lifecycle");
    let newer_snooze = store
        .snooze_review_unit_until(&unit_id, NOW + 20_000)
        .expect("newer snooze");
    let newer_schedule = schedule_state(7, ScheduleStatus::Review);
    store
        .set_schedule_state(&unit_id, Some(newer_schedule.clone()))
        .expect("newer schedule");

    let reapproved = store
        .approve_generated_prompt_draft(
            &draft.id,
            ApproveGeneratedPromptDraftOptions {
                initial_schedule_state: Some(schedule_state(1, ScheduleStatus::New)),
            },
        )
        .expect("repeat approval");
    assert_eq!(reapproved.prompt, newer_prompt.prompt);
    assert_eq!(reapproved.queue.lifecycle, newer_lifecycle.queue.lifecycle);
    assert_eq!(reapproved.snoozed_until, newer_snooze.snoozed_until);

    let snapshot = store.snapshot();
    assert_eq!(
        snapshot
            .schedules
            .iter()
            .find(|schedule| schedule.review_unit_id == unit_id)
            .expect("persisted schedule")
            .state,
        newer_schedule
    );
}

fn lifecycle_store(path: &std::path::Path) -> (BetaPersistenceStore, GeneratedPromptDraft) {
    let mut store = BetaPersistenceStore::open(path).expect("open store");
    let source = store
        .save_source_document(source_document("src-lifecycle"))
        .expect("source");
    let reference = store
        .save_reference_span(reference_span("ref-lifecycle", &source.id))
        .expect("reference");
    let draft = store
        .save_generated_prompt_draft(accepted_draft(
            "draft-lifecycle",
            "unit-lifecycle",
            &[source.id.as_str()],
            &[reference.id.as_str()],
            Some("run-lifecycle"),
        ))
        .expect("draft");
    store
        .save_generation_run(generation_run(
            "run-lifecycle",
            &[source.id.as_str()],
            &[draft.id.as_str()],
        ))
        .expect("run");
    store
        .approve_generated_prompt_draft(
            &draft.id,
            ApproveGeneratedPromptDraftOptions {
                initial_schedule_state: Some(schedule_state(2, ScheduleStatus::Review)),
            },
        )
        .expect("approve");

    (store, draft)
}

#[test]
fn snapshot_envelope_uses_beta_store_wire_names() {
    let snapshot = memory_engine_persistence::BetaStoreSnapshot {
        version: 1,
        source_documents: vec![source_document("src-wire")],
        reference_spans: Vec::new(),
        generated_prompt_drafts: Vec::new(),
        review_units: Vec::new(),
        schedules: Vec::new(),
        attempts: Vec::new(),
        generation_runs: Vec::new(),
        content_feedback: Vec::new(),
        concept_reference_notes: Vec::new(),
        applied_reviews: Vec::new(),
    };
    let encoded = serde_json::to_value(snapshot).expect("snapshot json");

    assert!(encoded.get("sourceDocuments").is_some());
    assert!(encoded.get("conceptReferenceNotes").is_some());
    assert!(encoded.get("source_documents").is_none());
    assert!(encoded.get("concept_reference_notes").is_none());
    assert_eq!(encoded["sourceDocuments"][0]["createdAt"], NOW);
    assert_eq!(
        encoded["sourceDocuments"][0]["permission"],
        "model-eligible"
    );
}

fn mastered_after_three_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 3
}

fn source_document(id: &str) -> SourceDocument {
    SourceDocument {
        id: id.to_owned(),
        kind: SourceDocumentKind::Text,
        title: "Latin prayer note".to_owned(),
        project_key: None,
        body: Some("Pater noster means Our Father.".to_owned()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: None,
    }
}

fn reference_span(id: &str, source_document_id: &str) -> ReferenceSpan {
    ReferenceSpan {
        id: id.to_owned(),
        source_document_id: source_document_id.to_owned(),
        label: "Pater noster translation".to_owned(),
        text: "Pater noster means Our Father.".to_owned(),
        locator: "paragraph:1".to_owned(),
        created_at: NOW,
    }
}

fn concept_reference_note(concept_key: &str) -> ConceptReferenceNote {
    ConceptReferenceNote {
        concept_key: concept_key.to_owned(),
        title: "NATO letter A".to_owned(),
        body: "A is Alfa.".to_owned(),
        model: GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "reference-note".to_owned(),
            version: "v1".to_owned(),
        },
        created_at: NOW,
        updated_at: NOW,
    }
}

fn save_concept_backed_bridge_draft(store: &mut BetaPersistenceStore) {
    store
        .save_concept_reference_note(concept_reference_note("nato-letter-a"))
        .expect("concept note");
    let concept_backed_bridge = GeneratedPromptDraft {
        source_document_ids: Vec::new(),
        reference_span_ids: Vec::new(),
        concept_reference_note_key: Some("nato-letter-a".to_owned()),
        ..accepted_draft(
            "draft-bridge-concept-backed",
            "bridge-concept-backed",
            &[],
            &[],
            Some("bridge-run"),
        )
    };
    store
        .save_generated_prompt_draft(concept_backed_bridge)
        .expect("concept-backed bridge draft persists");
}

fn accepted_draft(
    id: &str,
    unit_id: &str,
    source_document_ids: &[&str],
    reference_span_ids: &[&str],
    generation_run_id: Option<&str>,
) -> GeneratedPromptDraft {
    let review_unit_id = review_unit_id(unit_id);

    GeneratedPromptDraft {
        id: id.to_owned(),
        source_document_ids: source_document_ids
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        reference_span_ids: reference_span_ids
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        concept_reference_note_key: None,
        generation_run_id: generation_run_id.map(str::to_owned),
        review_unit_id: review_unit_id.clone(),
        prompt_id: "pater-translation".to_owned(),
        prompt: short_answer_prompt(&review_unit_id, "Translate: Pater noster"),
        queue: queue_candidate(&review_unit_id, NOW - 60_000),
        activity_kind: GeneratedLearningActivityKind::Quiz,
        activity_stage: "free-recall".to_owned(),
        worked_solution: None,
        model: GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "deterministic-draft".to_owned(),
            version: "v1".to_owned(),
        },
        validation: GeneratedPromptValidation {
            status: GeneratedPromptValidationStatus::Accepted,
            reasons: Vec::new(),
        },
        critique_notes: vec!["Grounded in the cited source span.".to_owned()],
        created_at: NOW,
    }
}

fn generation_run(id: &str, source_document_ids: &[&str], draft_ids: &[&str]) -> GenerationRun {
    GenerationRun {
        id: id.to_owned(),
        source_document_ids: source_document_ids
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        parent_review_unit_id: None,
        draft_ids: draft_ids.iter().map(|value| (*value).to_owned()).collect(),
        provider: "fixture".to_owned(),
        model: "deterministic-draft".to_owned(),
        started_at: NOW - 1_000,
        completed_at: Some(NOW),
        validation_failures: Vec::new(),
        usage: None,
        source_permissions: source_document_ids
            .iter()
            .map(|source_document_id| SourcePermissionReceipt {
                source_document_id: (*source_document_id).to_owned(),
                permission: SourcePermission::ModelEligible,
                consented: true,
            })
            .collect(),
        prompt_version: "prompt-v1".to_owned(),
    }
}

fn review_unit(
    review_unit_id: &ReviewUnitId,
    prompt_id: &str,
    prompt: Prompt,
    queue: PersistedQueueCandidate,
) -> BetaReviewUnitRecord {
    BetaReviewUnitRecord {
        review_unit_id: review_unit_id.clone(),
        prompt_id: prompt_id.to_owned(),
        prompt,
        queue,
        reference_span_ids: Vec::new(),
        concept_reference_note_key: None,
        generated_prompt_draft_id: None,
        archived_at: None,
        snoozed_until: None,
        created_at: NOW,
    }
}

fn queue_candidate(review_unit_id: &ReviewUnitId, due: i64) -> PersistedQueueCandidate {
    PersistedQueueCandidate {
        review_unit_id: review_unit_id.clone(),
        due,
        lifecycle: ReviewUnitLifecycle::active(),
        progression: None,
        concept_key: Some("lords-prayer-opening".to_owned()),
        source_key: Some("latin-prayer-note".to_owned()),
        domain_key: Some("latin".to_owned()),
    }
}

fn short_answer_prompt(review_unit_id: &ReviewUnitId, prompt: &str) -> Prompt {
    Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: review_unit_id.clone(),
        prompt: prompt.to_owned(),
        accepted_answers: vec!["Our Father".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    })
}

fn attempt(
    review_unit_id: &ReviewUnitId,
    occurred_at: i64,
    idempotency_key: &str,
) -> ServiceAttemptRecord {
    ServiceAttemptRecord {
        review_unit_id: review_unit_id.clone(),
        prompt_id: Some("same-row-review-prompt".to_owned()),
        submitted_answer: "Our Father".to_owned(),
        response_time_ms: 1_800,
        occurred_at,
        idempotency_key: Some(idempotency_key.to_owned()),
        grade: None,
    }
}

fn schedule_state(reps: u32, status: ScheduleStatus) -> ScheduleState {
    ScheduleState {
        due: NOW - 60_000,
        stability: 4.2,
        difficulty: 3.1,
        elapsed_days: 1,
        scheduled_days: 1,
        reps,
        lapses: 0,
        state: status,
        last_review: Some(NOW - 86_400_000),
    }
}

fn review_unit_id(value: &str) -> ReviewUnitId {
    ReviewUnitId::new(value)
}

fn prompt_text(prompt: &Prompt) -> &str {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => prompt,
        Prompt::Exact(prompt) => &prompt.prompt,
    }
}

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "memory-engine-persistence-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");

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
use std::{fs, path::PathBuf};
