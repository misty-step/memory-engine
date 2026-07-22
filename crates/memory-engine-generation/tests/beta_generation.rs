use std::{cell::Cell, fs, path::PathBuf};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, ReviewUnitId, ReviewUnitLifecycle,
};
use memory_engine_generation::{
    run_beta_generation, run_beta_generation_with_provider, run_bridge_generation,
    run_bridge_generation_with_provider, BetaGenerationError, BetaGenerationRequest,
    BridgeGenerationRequest, BridgeMaterial, BridgeMaterialProvider, BridgeMaterialRequest,
    DraftCandidate, DraftProvider, DraftRejection, FakeModelProvider, ProviderDrafts,
    ProviderFailure, ProviderUsage, ReferenceNoteDraft, ReferenceNoteProvider,
    ReferenceNoteRequest,
};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaReviewUnitRecord, GeneratedLearningActivityKind,
    GeneratedPromptModel, GeneratedPromptValidationStatus, PersistedQueueCandidate, SourceDocument,
    SourceDocumentKind, SourcePermission,
};

const NOW: i64 = 1_780_162_400_000;

#[test]
fn generates_accepted_quiz_and_exercise_drafts_with_provenance() {
    let directory = TempDirectory::new("accepted");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-nato".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "NATO phonetic alphabet notes".to_owned(),
            project_key: None,
            body: Some(source_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-nato".to_owned(),
            source_document_ids: vec!["src-nato".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.draft_ids,
        [
            "run-nato-draft-src-nato-1-nato-letter-a",
            "run-nato-draft-src-nato-2-nato-cat-composition"
        ]
    );
    assert_eq!(result.accepted_draft_ids, result.draft_ids);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());

    let snapshot = store.snapshot();
    assert_eq!(snapshot.reference_spans.len(), 2);
    assert_eq!(snapshot.generation_runs[0].draft_ids, result.draft_ids);
    assert_eq!(snapshot.generation_runs[0].completed_at, Some(NOW + 1_000));

    let quiz = &snapshot.generated_prompt_drafts[0];
    assert_eq!(quiz.activity_kind, GeneratedLearningActivityKind::Quiz);
    assert_eq!(
        quiz.validation.status,
        GeneratedPromptValidationStatus::Accepted
    );
    assert_eq!(quiz.queue.concept_key.as_deref(), Some("nato-letter-a"));
    match &quiz.prompt {
        Prompt::Mcq {
            prompt,
            choices,
            correct_choice,
            ..
        } => {
            assert_eq!(prompt, "What is the NATO phonetic alphabet word for A?");
            assert_eq!(choices, &["ALFA", "BRAVO", "CHARLIE"]);
            assert_eq!(correct_choice, "ALFA");
        }
        other => panic!("unexpected quiz prompt: {other:?}"),
    }

    let exercise = &snapshot.generated_prompt_drafts[1];
    assert_eq!(
        exercise.activity_kind,
        GeneratedLearningActivityKind::Exercise
    );
    assert_eq!(exercise.activity_stage, "composition");
    assert_eq!(
        exercise.worked_solution.as_deref(),
        Some("C is CHARLIE, A is ALFA, and T is TANGO.")
    );
    assert_eq!(
        exercise
            .queue
            .progression
            .as_ref()
            .expect("progression")
            .stage_order,
        4
    );

    let review_unit = store
        .keep_generated_prompt_draft("run-nato-draft-src-nato-2-nato-cat-composition", 0)
        .expect("keep");
    let queue =
        memory_engine_service::MemoryServiceStore::list_queue_candidates(&store).expect("queue");

    assert_eq!(
        review_unit.generated_prompt_draft_id.as_deref(),
        Some("run-nato-draft-src-nato-2-nato-cat-composition")
    );
    assert_eq!(queue[0].review_unit_id, review_unit.review_unit_id);
}

#[test]
fn short_enumerable_entries_are_promoted_with_substantive_evidence() {
    let directory = TempDirectory::new("short-enumerable-evidence");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-short-list".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Ordered terms".to_owned(),
            project_key: None,
            body: Some("1. Alpha\n2. Beta\n3. Gamma".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &FakeModelProvider,
        BetaGenerationRequest {
            run_id: "run-short-list".to_owned(),
            source_document_ids: vec!["src-short-list".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 3);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());
    let snapshot = store.snapshot();
    assert!(snapshot.generated_prompt_drafts.iter().all(|draft| {
        draft.validation.status == GeneratedPromptValidationStatus::Accepted
            && !draft.reference_span_ids.is_empty()
    }));
}

#[test]
fn short_verbatim_units_are_promoted_with_substantive_evidence() {
    let directory = TempDirectory::new("short-verbatim-evidence");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-short-verse".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Short verse".to_owned(),
            project_key: None,
            body: Some("Rise.\nShine bright.\nMove onward.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &FakeModelProvider,
        BetaGenerationRequest {
            run_id: "run-short-verse".to_owned(),
            source_document_ids: vec!["src-short-verse".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 3);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());
    let snapshot = store.snapshot();
    assert!(snapshot.generated_prompt_drafts.iter().all(|draft| {
        draft.validation.status == GeneratedPromptValidationStatus::Accepted
            && !draft.reference_span_ids.is_empty()
    }));
}

#[test]
fn browser_form_line_endings_preserve_multiple_structured_blocks() {
    let directory = TempDirectory::new("browser-line-endings");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-browser".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Browser textarea source".to_owned(),
            project_key: None,
            body: Some(source_body().replace('\n', "\r\n")),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-browser".to_owned(),
            source_document_ids: vec!["src-browser".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.accepted_draft_ids,
        [
            "run-browser-draft-src-browser-1-nato-letter-a",
            "run-browser-draft-src-browser-2-nato-cat-composition"
        ]
    );
}

#[test]
fn structured_generation_preserves_same_stage_variants_for_one_concept() {
    let directory = TempDirectory::new("variants");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-variants".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "NATO variants".to_owned(),
            project_key: None,
            body: Some(
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
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-variants".to_owned(),
            source_document_ids: vec!["src-variants".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 3);
    let snapshot = store.snapshot();
    let variants = snapshot
        .generated_prompt_drafts
        .iter()
        .filter(|draft| draft.queue.concept_key.as_deref() == Some("nato-letter-a"))
        .collect::<Vec<_>>();
    assert_eq!(variants.len(), 3);
    assert!(variants
        .iter()
        .all(|draft| draft.activity_stage == "recognition-3"));
    assert!(variants.iter().all(|draft| {
        draft
            .queue
            .progression
            .as_ref()
            .and_then(|progression| progression.progression_group.as_deref())
            == Some("nato-letter-a")
    }));
    assert_eq!(
        variants
            .iter()
            .map(|draft| match &draft.prompt {
                Prompt::Mcq { prompt, .. } => prompt.as_str(),
                other => panic!("variant should be MCQ: {other:?}"),
            })
            .collect::<Vec<_>>(),
        [
            "What is the NATO phonetic alphabet word for A?",
            "Choose the code word used for the letter A.",
            "In radio spelling, which word represents A?"
        ]
    );
}

#[test]
fn persists_rejected_unsupported_and_duplicate_drafts() {
    let directory = TempDirectory::new("rejected");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-options".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Options notes".to_owned(),
            project_key: None,
            body: Some(
                [
                    "Concept: Gamma definition",
                    "Activity: quiz",
                    "Stage: free-recall",
                    "Question: What does Gamma measure?",
                    "Answer: The rate of change of Delta.",
                    "Reference: Gamma measures the rate of change of Delta.",
                    "",
                    "Concept: Gamma definition",
                    "Activity: quiz",
                    "Stage: free-recall",
                    "Question: What does Gamma measure?",
                    "Answer: The rate of change of Delta.",
                    "Reference: Gamma measures the rate of change of Delta.",
                    "",
                    "Concept: Gamma advice",
                    "Activity: exercise",
                    "Stage: composition",
                    "Question: Should I buy this options position?",
                    "Answer: Buy the position.",
                    "Worked Solution: This would be personalized financial advice.",
                    "Reference: Gamma measures convexity, not whether a person should trade.",
                    "Unsupported: true",
                ]
                .join("\n"),
            ),
            uri: None,
            permission: SourcePermission::LocalOnly,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-options".to_owned(),
            source_document_ids: vec!["src-options".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.accepted_draft_ids,
        ["run-options-draft-src-options-1-gamma-definition"]
    );
    assert_eq!(
        result.rejected_draft_ids,
        ["run-options-draft-src-options-3-gamma-advice"]
    );
    assert_eq!(
        result.validation_failures,
        ["src-options block 2: Duplicate-ish generated draft"]
    );
    let drafts = store.snapshot().generated_prompt_drafts;
    assert_eq!(
        drafts[1].validation.reasons,
        ["Unsupported by cited source material"]
    );
}

#[test]
fn model_provider_is_not_called_for_local_only_source() {
    let directory = TempDirectory::new("local-only-provider-boundary");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-private".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Private notes".to_owned(),
            project_key: None,
            body: Some("Private notes must remain on this device.".to_owned()),
            uri: None,
            permission: SourcePermission::LocalOnly,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let provider = CountingProvider::default();
    let error = run_beta_generation_with_provider(
        &mut store,
        &provider,
        BetaGenerationRequest {
            run_id: "run-private".to_owned(),
            source_document_ids: vec!["src-private".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("local-only source must fail closed before provider invocation");

    assert!(error.to_string().contains("Local-only source"));
    assert_eq!(provider.calls.get(), 0);
    assert!(store.snapshot().generation_runs.is_empty());
}

#[test]
fn model_provider_is_not_called_for_archived_source() {
    let directory = TempDirectory::new("archived-provider");
    let mut store = BetaPersistenceStore::open(directory.path().join("store.json")).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "archived-source".to_owned(),
            title: "Archived source".to_owned(),
            body: Some("must never leave".to_owned()),
            kind: SourceDocumentKind::Text,
            project_key: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            uri: None,
            ttl_expires_at: None,
            archived_at: Some(123),
            created_at: NOW,
        })
        .expect("source");
    let provider = CountingProvider::default();

    let error = run_beta_generation_with_provider(
        &mut store,
        &provider,
        BetaGenerationRequest {
            run_id: "archived-run".to_owned(),
            source_document_ids: vec!["archived-source".to_owned()],
            parent_review_unit_id: None,
            started_at: 1,
            completed_at: Some(1),
            default_due: 0,
            model: None,
        },
    )
    .expect_err("archived source must fail before provider invocation");

    assert_eq!(
        error,
        BetaGenerationError::ArchivedSourceDocument("archived-source".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);

    let error = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "archived-local-run".to_owned(),
            source_document_ids: vec!["archived-source".to_owned()],
            parent_review_unit_id: None,
            started_at: 1,
            completed_at: Some(1),
            default_due: 0,
            model: None,
        },
    )
    .expect_err("archived source must fail before the deterministic provider too");
    assert_eq!(
        error,
        BetaGenerationError::ArchivedSourceDocument("archived-source".to_owned())
    );
}

#[test]
fn rejects_near_duplicate_questions_before_persistence_acceptance() {
    let directory = TempDirectory::new("near-duplicate");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-gamma-near".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Gamma notes".to_owned(),
            project_key: None,
            body: Some(
                [
                    "Concept: Gamma definition",
                    "Activity: quiz",
                    "Stage: free-recall",
                    "Question: What does Gamma measure?",
                    "Answer: The rate of change of Delta.",
                    "Reference: Gamma measures the rate of change of Delta.",
                    "",
                    "Concept: Gamma definition",
                    "Activity: quiz",
                    "Stage: free-recall",
                    "Question: What exactly does Gamma measure?",
                    "Answer: The rate of change of Delta.",
                    "Reference: Gamma measures the rate of change of Delta.",
                ]
                .join("\n"),
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-gamma-near".to_owned(),
            source_document_ids: vec!["src-gamma-near".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.accepted_draft_ids,
        ["run-gamma-near-draft-src-gamma-near-1-gamma-definition"]
    );
    assert!(result.rejected_draft_ids.is_empty());
    assert_eq!(
        result.validation_failures,
        ["src-gamma-near block 2: Duplicate-ish generated draft"]
    );
    let drafts = store.snapshot().generated_prompt_drafts;
    assert_eq!(drafts.len(), 1);
}

#[test]
fn repairs_zero_accepted_source_once_and_counts_repair_usage() {
    let directory = TempDirectory::new("repair");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-repair".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Repairable notes".to_owned(),
            project_key: None,
            body: Some("Repairable notes say spaced practice needs feedback.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &RepairingProvider,
        BetaGenerationRequest {
            run_id: "run-repair".to_owned(),
            source_document_ids: vec!["src-repair".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.rejected_draft_ids,
        ["run-repair-draft-src-repair-1-repair-feedback"]
    );
    assert_eq!(
        result.accepted_draft_ids,
        ["run-repair-draft-src-repair-2-repair-feedback"]
    );
    assert_ne!(result.rejected_draft_ids[0], result.accepted_draft_ids[0]);

    let snapshot = store.snapshot();
    assert_eq!(snapshot.generated_prompt_drafts.len(), 2);
    assert_eq!(
        snapshot.generated_prompt_drafts[0].validation.reasons,
        ["Exercises require a worked solution"]
    );
    assert_eq!(
        snapshot.generated_prompt_drafts[1].validation.status,
        GeneratedPromptValidationStatus::Accepted
    );
    let usage = snapshot.generation_runs[0]
        .usage
        .as_ref()
        .expect("run usage");
    assert_eq!(usage.input_tokens, 13);
    assert_eq!(usage.output_tokens, 24);
    assert_eq!(usage.cost_usd_micros, Some(125));
    assert_eq!(usage.latency_ms, 80);
}

#[test]
fn repairs_rejected_candidates_even_when_source_has_accepted_drafts() {
    let directory = TempDirectory::new("partial-repair");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-partial-repair".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Partial repair notes".to_owned(),
            project_key: None,
            body: Some("Partial repair notes say feedback improves recall.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &PartialRepairProvider,
        BetaGenerationRequest {
            run_id: "run-partial-repair".to_owned(),
            source_document_ids: vec!["src-partial-repair".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(
        result.rejected_draft_ids,
        ["run-partial-repair-draft-src-partial-repair-2-recall-feedback"]
    );
    assert_eq!(
        result.accepted_draft_ids,
        [
            "run-partial-repair-draft-src-partial-repair-1-recall-feedback",
            "run-partial-repair-draft-src-partial-repair-3-recall-feedback"
        ]
    );
    assert!(!result
        .accepted_draft_ids
        .contains(&result.rejected_draft_ids[0]));

    let snapshot = store.snapshot();
    assert_eq!(snapshot.generated_prompt_drafts.len(), 3);
    assert_eq!(
        snapshot.generated_prompt_drafts[1].validation.reasons,
        ["MCQ distractor duplicates the correct answer"]
    );
}

#[test]
fn repair_feedback_is_capped_before_provider_retry() {
    let directory = TempDirectory::new("repair-cap");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-repair-cap".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Repair cap notes".to_owned(),
            project_key: None,
            body: Some("Repair cap notes require worked solutions.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &RepairCapProvider,
        BetaGenerationRequest {
            run_id: "run-repair-cap".to_owned(),
            source_document_ids: vec!["src-repair-cap".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.rejected_draft_ids.len(), 5);
    assert_eq!(
        result.accepted_draft_ids,
        ["run-repair-cap-draft-src-repair-cap-6-repair-cap"]
    );
}

#[test]
fn bridge_generation_rejects_duplicate_of_manual_parent_review_unit() {
    let directory = TempDirectory::new("manual-parent-duplicate");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let parent = save_manual_parent(&mut store);

    let failure = run_bridge_generation_with_provider(
        &mut store,
        &DuplicateParentBridgeProvider,
        BridgeGenerationRequest {
            run_id: "bridge-run-manual-parent".to_owned(),
            parent_review_unit_id: parent,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 10_000,
            model: None,
        },
    )
    .expect_err("duplicate manual parent bridge should have no accepted drafts");

    assert!(
        failure
            .to_string()
            .contains("Duplicate-ish generated draft"),
        "unexpected failure: {failure}"
    );
    let snapshot = store.snapshot();
    let bridge_draft = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| draft.id.starts_with("bridge-run-manual-parent"))
        .expect("bridge draft persisted");
    assert_eq!(
        bridge_draft.validation.status,
        GeneratedPromptValidationStatus::Rejected
    );
    assert_eq!(
        bridge_draft.validation.reasons,
        ["Duplicate-ish generated draft"]
    );
}

#[test]
fn arbitrary_bridge_provider_is_denied_before_local_only_source_context_is_sent() {
    let directory = TempDirectory::new("local-only-bridge-provider");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-local-only-bridge".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Private bridge notes".to_owned(),
            project_key: None,
            body: Some(source_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");
    let generated = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "local-only-bridge-run".to_owned(),
            source_document_ids: vec!["src-local-only-bridge".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");
    let parent = store
        .keep_generated_prompt_draft(&generated.accepted_draft_ids[0], 0)
        .expect("parent")
        .review_unit_id;
    let mut source = store.snapshot().source_documents[0].clone();
    source.permission = SourcePermission::LocalOnly;
    store
        .save_source_document(source)
        .expect("local-only source");

    let provider = CountingBridgeProvider::default();
    let error = run_bridge_generation_with_provider(
        &mut store,
        &provider,
        BridgeGenerationRequest {
            run_id: "local-only-bridge-provider-run".to_owned(),
            parent_review_unit_id: parent,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("arbitrary provider must be denied");

    assert!(error.to_string().contains("Local-only source"));
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn missing_bridge_source_fails_before_provider_invocation() {
    let directory = TempDirectory::new("missing-bridge-source");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-missing-bridge".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Bridge source".to_owned(),
            project_key: None,
            body: Some(source_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");
    let generated = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "missing-bridge-parent-run".to_owned(),
            source_document_ids: vec!["src-missing-bridge".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("parent generation");
    let parent = store
        .keep_generated_prompt_draft(&generated.accepted_draft_ids[0], 0)
        .expect("parent")
        .review_unit_id;

    let mut snapshot = store.snapshot();
    snapshot.source_documents.clear();
    fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("remove source from persisted snapshot");
    drop(store);
    let mut store = BetaPersistenceStore::open(&path).expect("reopen store");

    let provider = CountingBridgeProvider::default();
    let error = run_bridge_generation_with_provider(
        &mut store,
        &provider,
        BridgeGenerationRequest {
            run_id: "missing-bridge-source-run".to_owned(),
            parent_review_unit_id: parent,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("missing referenced source must fail closed");

    assert_eq!(
        error,
        BetaGenerationError::UnknownSourceDocument("src-missing-bridge".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn missing_manual_parent_queue_source_fails_before_provider_invocation() {
    let directory = TempDirectory::new("missing-manual-parent-source");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let parent = save_manual_parent(&mut store);
    let mut snapshot = store.snapshot();
    snapshot.source_documents.clear();
    fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("remove manual source");
    drop(store);
    let mut store = BetaPersistenceStore::open(&path).expect("reopen store");
    let provider = CountingBridgeProvider::default();

    let error = run_bridge_generation_with_provider(
        &mut store,
        &provider,
        BridgeGenerationRequest {
            run_id: "missing-manual-parent-source-run".to_owned(),
            parent_review_unit_id: parent,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("missing manual queue source must fail closed");

    assert_eq!(
        error,
        BetaGenerationError::UnknownSourceDocument("manual-source".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn local_only_manual_parent_queue_source_fails_before_provider_invocation() {
    let directory = TempDirectory::new("local-only-manual-parent-source");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let parent = save_manual_parent(&mut store);
    let mut source = store.snapshot().source_documents[0].clone();
    source.permission = SourcePermission::LocalOnly;
    store
        .save_source_document(source)
        .expect("local-only source");
    let provider = CountingBridgeProvider::default();

    let error = run_bridge_generation_with_provider(
        &mut store,
        &provider,
        BridgeGenerationRequest {
            run_id: "local-only-manual-parent-source-run".to_owned(),
            parent_review_unit_id: parent,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("local-only manual queue source must fail closed");

    assert_eq!(
        error,
        BetaGenerationError::LocalOnlySource("manual-source".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn archived_manual_parent_source_fails_before_any_bridge_provider_invocation() {
    let directory = TempDirectory::new("archived-manual-parent-source");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    let parent = save_manual_parent(&mut store);
    store
        .archive_source_document("manual-source", NOW)
        .expect("archive source");

    let provider = CountingBridgeProvider::default();
    let request = BridgeGenerationRequest {
        run_id: "archived-manual-parent-source-run".to_owned(),
        parent_review_unit_id: parent,
        started_at: NOW,
        completed_at: Some(NOW + 1_000),
        default_due: NOW,
        model: None,
    };
    let error = run_bridge_generation_with_provider(&mut store, &provider, request.clone())
        .expect_err("archived manual queue source must fail closed");
    assert_eq!(
        error,
        BetaGenerationError::ArchivedSourceDocument("manual-source".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);

    let error = run_bridge_generation(&mut store, request)
        .expect_err("archived source must also block the deterministic bridge path");
    assert_eq!(
        error,
        BetaGenerationError::ArchivedSourceDocument("manual-source".to_owned())
    );
}

#[test]
fn bridge_descendants_preserve_full_provenance_and_fail_closed_once_provider_is_enabled() {
    let (_directory, path, parent_review_unit_id) = seed_bridge_descendants_provenance_fixture();
    let mut store = BetaPersistenceStore::open(&path).expect("reopen store");

    let child_generation = run_bridge_generation(
        &mut store,
        BridgeGenerationRequest {
            run_id: "bridge-descendants-child-run".to_owned(),
            parent_review_unit_id: parent_review_unit_id.clone(),
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("child generation");
    let child_review_unit_id = store
        .keep_generated_prompt_draft(&child_generation.accepted_draft_ids[0], 0)
        .expect("child")
        .review_unit_id;
    let child_snapshot = store.snapshot();
    let child_draft = child_snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| draft.review_unit_id == child_review_unit_id)
        .expect("child draft");
    assert_eq!(
        child_draft.source_document_ids,
        vec![
            "src-local-only-bridge".to_owned(),
            "src-secondary-bridge".to_owned()
        ]
    );
    assert_eq!(
        child_draft.queue.source_key.as_deref(),
        Some("src-local-only-bridge")
    );

    let provider = CountingBridgeProvider::default();
    let error = run_bridge_generation_with_provider(
        &mut store,
        &provider,
        BridgeGenerationRequest {
            run_id: "bridge-descendants-provider-run".to_owned(),
            parent_review_unit_id: child_review_unit_id.clone(),
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("local-only provenance must fail once provider is enabled");
    assert_eq!(
        error,
        BetaGenerationError::LocalOnlySource("src-local-only-bridge".to_owned())
    );
    assert_eq!(provider.calls.get(), 0);
}

fn seed_bridge_descendants_provenance_fixture() -> (TempDirectory, PathBuf, ReviewUnitId) {
    let directory = TempDirectory::new("bridge-descendants-provenance");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-local-only-bridge".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Private bridge notes".to_owned(),
            project_key: None,
            body: Some(source_body()),
            uri: None,
            permission: SourcePermission::LocalOnly,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("local-only source");
    store
        .save_source_document(SourceDocument {
            id: "src-secondary-bridge".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Secondary bridge notes".to_owned(),
            project_key: None,
            body: Some("Secondary bridge provenance note.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("secondary source");

    let parent_generation = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "bridge-descendants-parent-run".to_owned(),
            source_document_ids: vec!["src-local-only-bridge".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("parent generation");
    let parent_review_unit_id = store
        .keep_generated_prompt_draft(&parent_generation.accepted_draft_ids[0], 0)
        .expect("parent")
        .review_unit_id;

    let mut snapshot = store.snapshot();
    let parent_draft = snapshot
        .generated_prompt_drafts
        .iter_mut()
        .find(|draft| draft.review_unit_id == parent_review_unit_id)
        .expect("parent draft");
    parent_draft
        .source_document_ids
        .push("src-secondary-bridge".to_owned());
    fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("persist multi-source provenance");

    (directory, path, parent_review_unit_id)
}

#[test]
fn authored_block_without_a_reference_is_a_world_knowledge_card() {
    let directory = TempDirectory::new("no-reference");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-no-reference".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Authored note".to_owned(),
            project_key: None,
            body: Some(
                [
                    "Concept: nato letter a",
                    "Activity: quiz",
                    "Question: What is A in the NATO phonetic alphabet?",
                    "Answer: Alfa",
                ]
                .join("\n"),
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-no-reference".to_owned(),
            source_document_ids: vec!["src-no-reference".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    // No cited reference, so the card is a world-knowledge expansion: grounding
    // is decided per card by quote presence, and a quote-free card is accepted
    // and grounded in the captured input rather than rejected.
    assert_eq!(result.accepted_draft_ids.len(), 1);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());
    let draft = &store.snapshot().generated_prompt_drafts[0];
    assert_eq!(
        draft.validation.status,
        GeneratedPromptValidationStatus::Accepted
    );
    assert!(draft
        .critique_notes
        .iter()
        .any(|note| note.contains("Expanded from input")));
}

/// A model-style provider that expands the input from world knowledge: its
/// card carries no source quote, the way real topic expansion does.
struct KnowledgeCardProvider;

impl DraftProvider for KnowledgeCardProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fake-knowledge".to_owned(),
            name: "knowledge-expander".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: "NATO alphabet: B".to_owned(),
                question: "In the NATO phonetic alphabet, what word represents the letter B?"
                    .to_owned(),
                answer: "Bravo".to_owned(),
                evidence: None,
                distractors: Vec::new(),
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "recognition".to_owned(),
                unsupported: false,
            }],
            failures: Vec::new(),
            usage: None,
        })
    }
}

/// A provider whose card claims a source quote that is NOT in the source — the
/// fabricated-citation failure the anti-hallucination gate must catch.
struct FabricatedQuoteProvider;

impl DraftProvider for FabricatedQuoteProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fake-fabricator".to_owned(),
            name: "fabricator".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: "Mitochondria invention".to_owned(),
                question: "Who invented mitochondria in 1923?".to_owned(),
                answer: "Dr. Smith".to_owned(),
                evidence: Some("mitochondria were invented by Dr. Smith in 1923".to_owned()),
                distractors: Vec::new(),
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "recognition".to_owned(),
                unsupported: false,
            }],
            failures: Vec::new(),
            usage: None,
        })
    }
}

#[test]
fn world_knowledge_card_without_a_quote_is_accepted_and_seeded() {
    let directory = TempDirectory::new("knowledge-card");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-topic".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "NATO phonetic alphabet".to_owned(),
            project_key: None,
            body: Some("nato phonetic alphabet".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &KnowledgeCardProvider,
        BetaGenerationRequest {
            run_id: "run-topic".to_owned(),
            source_document_ids: vec!["src-topic".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 1);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());

    let snapshot = store.snapshot();
    // No per-fact quote, so the card grounds in the captured input itself as its
    // reference span — a real pointer the store can resolve.
    assert_eq!(snapshot.reference_spans.len(), 1);
    assert_eq!(snapshot.reference_spans[0].text, "nato phonetic alphabet");
    let draft = &snapshot.generated_prompt_drafts[0];
    assert_eq!(
        draft.validation.status,
        GeneratedPromptValidationStatus::Accepted
    );
    assert_eq!(draft.source_document_ids, ["src-topic"]);
    assert_eq!(draft.reference_span_ids.len(), 1);
    assert_eq!(draft.queue.concept_key.as_deref(), Some("nato-alphabet-b"));
    // Recorded as a world-knowledge expansion, not a source-grounded extraction.
    assert!(draft
        .critique_notes
        .iter()
        .any(|note| note.contains("Expanded from input")));
}

#[test]
fn fabricated_source_quote_is_rejected() {
    let directory = TempDirectory::new("fabricated-quote");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-passage".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Mitochondria".to_owned(),
            project_key: None,
            body: Some(
                "Mitochondria are membrane-bound organelles found in the cytoplasm of nearly \
                 all eukaryotic cells, where they generate most of the cell's supply of \
                 adenosine triphosphate through the process of oxidative phosphorylation."
                    .to_owned(),
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &FabricatedQuoteProvider,
        BetaGenerationRequest {
            run_id: "run-passage".to_owned(),
            source_document_ids: vec!["src-passage".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW - 60_000,
            model: None,
        },
    )
    .expect("generation");

    // A card that CLAIMS a source quote must have it verify. This quote is not
    // in the source, so the card is rejected — the anti-hallucination guarantee.
    assert!(result.accepted_draft_ids.is_empty());
    let rejected = &store.snapshot().generated_prompt_drafts[0];
    assert_eq!(
        rejected.validation.status,
        GeneratedPromptValidationStatus::Rejected
    );
    assert!(rejected
        .validation
        .reasons
        .iter()
        .any(|reason| reason.contains("Evidence quote not found in cited source")));
}

#[test]
fn flagged_unsupported_card_is_rejected_even_without_a_quote() {
    // A card the generator flags as unsupported must be rejected whether or not
    // it cites a quote: omitting the quote must not launder a flagged-bad answer
    // into the queue through the world-knowledge lane.
    let directory = TempDirectory::new("flagged-unsupported");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-flagged".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Flagged note".to_owned(),
            project_key: None,
            body: Some(
                [
                    "Concept: dubious",
                    "Activity: quiz",
                    "Question: What is dubious?",
                    "Answer: An answer the generator could not support.",
                    "Unsupported: true",
                ]
                .join("\n"),
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-flagged".to_owned(),
            source_document_ids: vec!["src-flagged".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert!(result.accepted_draft_ids.is_empty());
    let rejected = &store.snapshot().generated_prompt_drafts[0];
    assert_eq!(
        rejected.validation.status,
        GeneratedPromptValidationStatus::Rejected
    );
    assert!(rejected
        .validation
        .reasons
        .iter()
        .any(|reason| reason.contains("Unsupported by cited source material")));
}

#[test]
fn generation_preserves_retrieval_depth_progression_tiers() {
    let directory = TempDirectory::new("retrieval-depths");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-depths".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Retrieval depth notes".to_owned(),
            project_key: None,
            body: Some(retrieval_depth_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-depths".to_owned(),
            source_document_ids: vec!["src-depths".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: Some(NOW + 1_000),
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 4);
    assert!(result.rejected_draft_ids.is_empty());
    assert!(result.validation_failures.is_empty());

    let drafts = store.snapshot().generated_prompt_drafts;
    let tiers = drafts
        .iter()
        .map(|draft| {
            (
                draft.activity_stage.as_str(),
                draft
                    .queue
                    .progression
                    .as_ref()
                    .expect("progression")
                    .stage_order,
                &draft.prompt,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(tiers[0].0, "recognition");
    assert_eq!(tiers[0].1, 1);
    assert!(matches!(tiers[0].2, Prompt::Mcq { .. }));
    assert_eq!(tiers[1].0, "cued-recall");
    assert_eq!(tiers[1].1, 2);
    assert!(matches!(tiers[1].2, Prompt::Exact(_)));
    assert_eq!(tiers[2].0, "free-recall");
    assert_eq!(tiers[2].1, 3);
    assert!(matches!(tiers[2].2, Prompt::Exact(_)));
    assert_eq!(tiers[3].0, "composition");
    assert_eq!(tiers[3].1, 4);
    assert!(matches!(tiers[3].2, Prompt::Exact(_)));
}

#[test]
fn reports_unknown_and_empty_sources_before_starting_generation() {
    let directory = TempDirectory::new("source-errors");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");

    let missing = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-missing".to_owned(),
            source_document_ids: vec!["missing-source".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("missing source");
    assert_eq!(
        missing,
        BetaGenerationError::UnknownSourceDocument("missing-source".to_owned())
    );

    store
        .save_source_document(SourceDocument {
            id: "empty-source".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Empty source".to_owned(),
            project_key: None,
            body: Some("   ".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");
    let empty = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-empty".to_owned(),
            source_document_ids: vec!["empty-source".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect_err("empty source");

    assert_eq!(
        empty,
        BetaGenerationError::SourceDocumentHasNoTextBody("empty-source".to_owned())
    );
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

fn retrieval_depth_body() -> String {
    [
        "Concept: Retrieval depth",
        "Activity: quiz",
        "Stage: recognition",
        "Question: Which retrieval depth asks the learner to identify an answer?",
        "Answer: recognition",
        "Distractors: cued recall, composition",
        "Reference: Recognition asks the learner to identify the answer among options.",
        "",
        "Concept: Retrieval depth",
        "Activity: quiz",
        "Stage: cued-recall",
        "Question: With the cue 'identify among options', which depth is this?",
        "Answer: recognition",
        "Reference: Recognition asks the learner to identify the answer among options.",
        "",
        "Concept: Retrieval depth",
        "Activity: quiz",
        "Stage: free-recall",
        "Question: Name the retrieval depth that asks the learner to identify an answer among options.",
        "Answer: recognition",
        "Reference: Recognition asks the learner to identify the answer among options.",
        "",
        "Concept: Retrieval depth transfer",
        "Activity: exercise",
        "Stage: composition",
        "Question: Compose a study progression from recognizing a term to using it in a scenario.",
        "Answer: recognition to cued recall to free recall to composition",
        "Worked Solution: Start with recognition, remove options for cued recall, ask unaided free recall, then require composition in context.",
        "Reference: A study progression can move from recognition to cued recall to free recall to composition in context.",
    ]
    .join("\n")
}

fn save_manual_parent(store: &mut BetaPersistenceStore) -> ReviewUnitId {
    store
        .save_source_document(SourceDocument {
            id: "manual-source".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Manual source".to_owned(),
            project_key: None,
            body: Some("Manual parent source notes.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("manual source");
    let review_unit_id = ReviewUnitId::new("manual-nato-cat-parent");
    let prompt = Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: review_unit_id.clone(),
        prompt: "Spell CAT over the phone using the NATO phonetic alphabet.".to_owned(),
        accepted_answers: vec!["CHARLIE ALFA TANGO".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    });
    store
        .save_review_unit(BetaReviewUnitRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: "manual-nato-cat-parent-prompt".to_owned(),
            prompt,
            queue: PersistedQueueCandidate {
                review_unit_id: review_unit_id.clone(),
                due: NOW - 60_000,
                lifecycle: ReviewUnitLifecycle::active(),
                progression: Some(ProgressionMetadata {
                    progression_group: Some("nato-cat-composition".to_owned()),
                    stage_order: 4,
                    requires: Vec::new(),
                    supersedes: Vec::new(),
                }),
                concept_key: Some("nato-cat-composition".to_owned()),
                source_key: Some("manual-source".to_owned()),
                domain_key: Some("nato".to_owned()),
            },
            reference_span_ids: Vec::new(),
            concept_reference_note_key: None,
            generated_prompt_draft_id: None,
            archived_at: None,
            snoozed_until: None,
            created_at: NOW,
        })
        .expect("manual parent");

    review_unit_id
}

struct DuplicateParentBridgeProvider;

#[derive(Default)]
struct CountingBridgeProvider {
    calls: Cell<usize>,
}

impl ReferenceNoteProvider for CountingBridgeProvider {
    fn model(&self) -> GeneratedPromptModel {
        ReferenceNoteProvider::model(&FakeModelProvider)
    }

    fn explain_concept(
        &self,
        request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        FakeModelProvider.explain_concept(request)
    }
}

impl BridgeMaterialProvider for CountingBridgeProvider {
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        self.calls.set(self.calls.get() + 1);
        FakeModelProvider.generate_bridge_material(request)
    }
}

impl ReferenceNoteProvider for DuplicateParentBridgeProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "duplicate-parent".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn explain_concept(
        &self,
        _request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        Ok(ReferenceNoteDraft {
            title: "NATO CAT composition".to_owned(),
            body: "CAT is spelled CHARLIE ALFA TANGO.".to_owned(),
        })
    }
}

impl BridgeMaterialProvider for DuplicateParentBridgeProvider {
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        Ok(BridgeMaterial {
            model: GeneratedPromptModel {
                provider: "fixture".to_owned(),
                name: "duplicate-parent".to_owned(),
                version: "v1".to_owned(),
            },
            reference_note: ReferenceNoteDraft {
                title: "NATO CAT composition".to_owned(),
                body: "CAT is spelled CHARLIE ALFA TANGO.".to_owned(),
            },
            candidates: vec![DraftCandidate {
                index: 1,
                concept: request.concept_label.clone(),
                question: request.parent_prompt.clone(),
                answer: request.parent_expected_answer.clone(),
                evidence: None,
                distractors: vec!["BRAVO ECHO ECHO".to_owned(), "DELTA OSCAR GOLF".to_owned()],
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "recognition-bridge".to_owned(),
                unsupported: false,
            }],
            usage: None,
        })
    }
}

struct RepairingProvider;

impl DraftProvider for RepairingProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "repairing-provider".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: "Repair feedback".to_owned(),
                question: "Explain why spaced practice needs feedback.".to_owned(),
                answer: "Spaced practice needs feedback.".to_owned(),
                evidence: Some("Repairable notes say spaced practice needs feedback.".to_owned()),
                distractors: Vec::new(),
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "free-recall".to_owned(),
                unsupported: false,
            }],
            failures: Vec::new(),
            usage: Some(ProviderUsage {
                input_tokens: 10,
                output_tokens: 20,
                cost_usd_micros: Some(100),
                latency_ms: 50,
            }),
        })
    }

    fn repair_drafts(
        &self,
        _source: &SourceDocument,
        rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].reasons,
            ["Exercises require a worked solution"]
        );
        Ok(Some(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: "Repair feedback".to_owned(),
                question: "Explain why spaced practice needs feedback.".to_owned(),
                answer: "Spaced practice needs feedback.".to_owned(),
                evidence: Some("Repairable notes say spaced practice needs feedback.".to_owned()),
                distractors: Vec::new(),
                worked_solution: Some(
                    "Feedback tells the learner whether the spaced retrieval was correct."
                        .to_owned(),
                ),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "free-recall".to_owned(),
                unsupported: false,
            }],
            failures: Vec::new(),
            usage: Some(ProviderUsage {
                input_tokens: 3,
                output_tokens: 4,
                cost_usd_micros: Some(25),
                latency_ms: 30,
            }),
        }))
    }
}

struct PartialRepairProvider;

impl DraftProvider for PartialRepairProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "partial-repair-provider".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![
                DraftCandidate {
                    index: 1,
                    concept: "Recall feedback".to_owned(),
                    question: "What improves recall?".to_owned(),
                    answer: "feedback".to_owned(),
                    evidence: Some("Partial repair notes say feedback improves recall.".to_owned()),
                    distractors: vec!["spacing".to_owned(), "sleep".to_owned()],
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition".to_owned(),
                    unsupported: false,
                },
                DraftCandidate {
                    index: 2,
                    concept: "Recall feedback".to_owned(),
                    question: "Which item from the notes improves later recall?".to_owned(),
                    answer: "feedback".to_owned(),
                    evidence: Some("Partial repair notes say feedback improves recall.".to_owned()),
                    distractors: vec!["feedback".to_owned(), "guessing".to_owned()],
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition".to_owned(),
                    unsupported: false,
                },
            ],
            failures: Vec::new(),
            usage: None,
        })
    }

    fn repair_drafts(
        &self,
        _source: &SourceDocument,
        rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].reasons,
            ["MCQ distractor duplicates the correct answer"]
        );
        Ok(Some(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![
                DraftCandidate {
                    index: 1,
                    concept: "Recall feedback".to_owned(),
                    question: "What improves recall after practice?".to_owned(),
                    answer: "feedback".to_owned(),
                    evidence: Some("Partial repair notes say feedback improves recall.".to_owned()),
                    distractors: vec!["guessing".to_owned(), "forgetting".to_owned()],
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition".to_owned(),
                    unsupported: false,
                },
                DraftCandidate {
                    index: 1,
                    concept: "Excess repair".to_owned(),
                    question: "Which extra repair should be ignored?".to_owned(),
                    answer: "the over-budget repair".to_owned(),
                    evidence: Some("Partial repair notes say feedback improves recall.".to_owned()),
                    distractors: vec![
                        "the accepted repair".to_owned(),
                        "the rejected draft".to_owned(),
                    ],
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition".to_owned(),
                    unsupported: false,
                },
            ],
            failures: Vec::new(),
            usage: None,
        }))
    }
}

struct RepairCapProvider;

impl DraftProvider for RepairCapProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "repair-cap-provider".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: (1..=5)
                .map(|index| DraftCandidate {
                    index,
                    concept: format!("Repair cap {index}"),
                    question: format!("Explain repair cap item {index}."),
                    answer: "Repair cap notes require worked solutions.".to_owned(),
                    evidence: Some("Repair cap notes require worked solutions.".to_owned()),
                    distractors: Vec::new(),
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Exercise,
                    activity_stage: "free-recall".to_owned(),
                    unsupported: false,
                })
                .collect(),
            failures: Vec::new(),
            usage: None,
        })
    }

    fn repair_drafts(
        &self,
        _source: &SourceDocument,
        rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        assert_eq!(rejections.len(), 4);
        Ok(Some(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: vec![DraftCandidate {
                index: 1,
                concept: "Repair cap".to_owned(),
                question: "Explain why repair feedback is capped.".to_owned(),
                answer: "Repair feedback is capped to bound retry cost.".to_owned(),
                evidence: Some("Repair cap notes require worked solutions.".to_owned()),
                distractors: Vec::new(),
                worked_solution: Some("The retry receives a bounded rejection list.".to_owned()),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "free-recall".to_owned(),
                unsupported: false,
            }],
            failures: Vec::new(),
            usage: None,
        }))
    }
}

#[derive(Default)]
struct CountingProvider {
    calls: Cell<usize>,
}

impl DraftProvider for CountingProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "test".to_owned(),
            name: "counting-provider".to_owned(),
            version: "prompt-v1".to_owned(),
        }
    }

    fn generate_drafts(&self, _source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        self.calls.set(self.calls.get() + 1);
        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: None,
            candidates: Vec::new(),
            failures: Vec::new(),
            usage: None,
        })
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
            "memory-engine-generation-{label}-{}-{stamp}",
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
