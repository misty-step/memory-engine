use std::{fs, path::PathBuf};

use memory_engine_core::{ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, ReviewUnitId};
use memory_engine_generation::{
    run_beta_generation, run_bridge_generation_with_provider, BetaGenerationError,
    BetaGenerationRequest, BridgeGenerationRequest, BridgeMaterial, BridgeMaterialProvider,
    BridgeMaterialRequest, DraftCandidate, ProviderFailure, ReferenceNoteDraft,
    ReferenceNoteProvider, ReferenceNoteRequest,
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
            body: Some(source_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
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
        .approve_generated_prompt_draft(
            "run-nato-draft-src-nato-2-nato-cat-composition",
            memory_engine_persistence::ApproveGeneratedPromptDraftOptions::default(),
        )
        .expect("approve");
    let queue =
        memory_engine_service::MemoryServiceStore::list_queue_candidates(&store).expect("queue");

    assert_eq!(
        review_unit.generated_prompt_draft_id.as_deref(),
        Some("run-nato-draft-src-nato-2-nato-cat-composition")
    );
    assert_eq!(queue[0].review_unit_id, review_unit.review_unit_id);
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
        [
            "run-options-draft-src-options-2-gamma-definition",
            "run-options-draft-src-options-3-gamma-advice"
        ]
    );
    let drafts = store.snapshot().generated_prompt_drafts;
    assert_eq!(
        drafts[1].validation.reasons,
        ["Duplicate-ish generated draft"]
    );
    assert_eq!(
        drafts[2].validation.reasons,
        ["Unsupported by cited source material"]
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
fn records_missing_provenance_failures_without_saving_malformed_drafts() {
    let directory = TempDirectory::new("missing-provenance");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-missing-provenance".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Unsupported note".to_owned(),
            body: Some(
                [
                    "Concept: unsupported",
                    "Activity: quiz",
                    "Question: What is unsupported?",
                    "Answer: This has no cited source span.",
                ]
                .join("\n"),
            ),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    let result = run_beta_generation(
        &mut store,
        BetaGenerationRequest {
            run_id: "run-missing".to_owned(),
            source_document_ids: vec!["src-missing-provenance".to_owned()],
            parent_review_unit_id: None,
            started_at: NOW,
            completed_at: None,
            default_due: NOW,
            model: None,
        },
    )
    .expect("generation");

    assert!(result.draft_ids.is_empty());
    assert_eq!(
        result.validation_failures,
        ["src-missing-provenance block 1: generated drafts require source provenance"]
    );
    assert!(store.snapshot().generated_prompt_drafts.is_empty());
    assert_eq!(
        store.snapshot().generation_runs[0].validation_failures,
        result.validation_failures
    );
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
            body: Some(retrieval_depth_body()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
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
            body: Some("   ".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
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
