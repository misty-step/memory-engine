use std::{cell::Cell, fs, path::PathBuf};

use memory_engine_core::{ExactPromptKind, Prompt};
use memory_engine_generation::{
    classify_learning_intent, run_beta_generation_with_provider, BetaGenerationRequest,
    DraftCandidate, DraftProvider, DraftRejection, FakeModelProvider, FallbackProvider,
    LearningIntent, ProviderDrafts, ProviderFailure, ProviderUsage,
};
use memory_engine_persistence::{
    BetaPersistenceStore, GeneratedLearningActivityKind, GeneratedPromptModel,
    GeneratedPromptValidationStatus, GenerationRunUsage, SourceDocument, SourceDocumentKind,
    SourcePermission,
};

const NOW: i64 = 1_780_162_400_000;

const PROSE: &str = "Mitochondria are organelles that generate most of the cell's supply of \
adenosine triphosphate. The number of mitochondria in a cell varies widely by organism and \
tissue type. They are sometimes called the powerhouse of the cell because they produce usable \
chemical energy.";

#[test]
fn fallback_provider_rejects_local_only_source_before_forwarding() {
    let source = SourceDocument {
        id: "src-local-only".to_owned(),
        kind: SourceDocumentKind::Text,
        title: "Private notes".to_owned(),
        project_key: None,
        body: Some("private notes".to_owned()),
        uri: None,
        permission: SourcePermission::LocalOnly,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: None,
    };
    let fallback = FallbackProvider::new(&FakeModelProvider);
    let failure = fallback
        .generate_drafts(&source)
        .expect_err("local-only source must not reach either provider");

    assert!(matches!(
        failure.kind(),
        memory_engine_generation::ProviderFailureKind::LocalOnlySource(id)
            if id == "src-local-only"
    ));

    let failure = fallback
        .repair_drafts(&source, &[])
        .expect_err("local-only repair must not reach either provider");
    assert!(matches!(
        failure.kind(),
        memory_engine_generation::ProviderFailureKind::LocalOnlySource(id)
            if id == "src-local-only"
    ));
}

#[test]
fn fallback_provider_rejects_archived_source_before_forwarding() {
    let mut source = SourceDocument {
        id: "archived-source".to_owned(),
        title: "Archived source".to_owned(),
        kind: SourceDocumentKind::Text,
        project_key: None,
        body: Some("must never leave".to_owned()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: Some(123),
    };
    let provider = FallbackProvider::new(&FakeModelProvider);

    let failure = provider
        .generate_drafts(&source)
        .expect_err("archived source must fail before forwarding");
    assert!(matches!(
        failure.kind(),
        memory_engine_generation::ProviderFailureKind::ArchivedSource(id) if id == "archived-source"
    ));

    source.archived_at = Some(456);
    let failure = provider
        .repair_drafts(&source, &[])
        .expect_err("archived repair source must fail before forwarding");
    assert!(matches!(
        failure.kind(),
        memory_engine_generation::ProviderFailureKind::ArchivedSource(id) if id == "archived-source"
    ));
}

#[test]
fn fake_model_provider_generates_grounded_drafts_from_arbitrary_prose() {
    let directory = TempDirectory::new("fake-provider");
    let mut store = open_store_with_prose(&directory);

    let result = run_beta_generation_with_provider(
        &mut store,
        &FakeModelProvider,
        request("run-prose", "src-prose"),
    )
    .expect("generation");

    assert!(
        !result.accepted_draft_ids.is_empty(),
        "expected at least one accepted draft from arbitrary prose, got failures: {:?}",
        result.validation_failures
    );
    let snapshot = store.snapshot();
    let reference = &snapshot.reference_spans[0];
    assert!(
        PROSE.contains(&reference.text),
        "reference span must quote the source verbatim: {:?}",
        reference.text
    );
    assert_eq!(snapshot.generation_runs[0].model, "fake-model");
}

#[test]
fn one_word_capture_generates_an_accepted_grounded_draft() {
    let directory = TempDirectory::new("one-word-capture");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(source_document("src-term", "Mitochondria", "Mitochondria"))
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &FakeModelProvider,
        request("run-term", "src-term"),
    )
    .expect("generation");

    assert!(
        !result.accepted_draft_ids.is_empty(),
        "one-word captures should not be rejected as under-evidenced: {:?}",
        result.validation_failures
    );
    assert!(result.rejected_draft_ids.is_empty());
    let snapshot = store.snapshot();
    assert_eq!(snapshot.reference_spans[0].text, "Mitochondria");
    assert_eq!(
        snapshot.generated_prompt_drafts[0].validation.status,
        GeneratedPromptValidationStatus::Accepted
    );
}

#[test]
fn fake_model_provider_branches_draft_shapes_by_learning_intent() {
    let model = FakeModelProvider;
    let verbatim = source_document(
        "src-poem",
        "Hope is the thing with feathers",
        "\"Hope\" is the thing with feathers -\nThat perches in the soul -\nAnd sings the tune without the words -\nAnd never stops - at all -",
    );
    let concept = source_document(
        "src-concept",
        "Mitochondria",
        "Mitochondria are organelles that generate most of the cell's supply of adenosine triphosphate because cells use ATP as chemical energy.",
    );
    let fact = source_document(
        "src-fact",
        "Cell facts",
        "ATP means cellular energy. DNA means genetic material. RNA means messenger material.",
    );
    let process = source_document(
        "src-process",
        "Sourdough starter",
        "To maintain a sourdough starter, discard all but 50 grams and feed the remainder with equal weights of flour and water. Always let it double before baking.",
    );

    assert_eq!(
        classify_learning_intent(&verbatim).intent,
        LearningIntent::VerbatimMemorization
    );
    assert_eq!(
        classify_learning_intent(&concept).intent,
        LearningIntent::ConceptUnderstanding
    );
    assert_eq!(
        classify_learning_intent(&fact).intent,
        LearningIntent::FactRecall
    );
    assert_eq!(
        classify_learning_intent(&process).intent,
        LearningIntent::ProcedureProcess
    );

    let verbatim_drafts = model.generate_drafts(&verbatim).expect("verbatim");
    assert_eq!(
        verbatim_drafts.learning_intent,
        Some(LearningIntent::VerbatimMemorization)
    );
    assert!(
        verbatim_drafts
            .candidates
            .iter()
            .all(|candidate| candidate.activity_kind == GeneratedLearningActivityKind::Exercise),
        "verbatim captures should produce recitation exercises, not quizzes: {:?}",
        verbatim_drafts.candidates
    );
    assert!(verbatim_drafts
        .candidates
        .iter()
        .any(|candidate| candidate.activity_stage.contains("cued")));
    assert!(verbatim_drafts
        .candidates
        .iter()
        .any(|candidate| candidate.activity_stage.contains("free")));

    let concept_drafts = model.generate_drafts(&concept).expect("concept");
    assert_eq!(
        concept_drafts.learning_intent,
        Some(LearningIntent::ConceptUnderstanding)
    );
    assert!(concept_drafts
        .candidates
        .iter()
        .any(|candidate| candidate.activity_stage.contains("free")));

    let fact_drafts = model.generate_drafts(&fact).expect("fact");
    assert_eq!(
        fact_drafts.learning_intent,
        Some(LearningIntent::FactRecall)
    );
    assert!(fact_drafts
        .candidates
        .iter()
        .any(|candidate| !candidate.distractors.is_empty()));

    let process_drafts = model.generate_drafts(&process).expect("process");
    assert_eq!(
        process_drafts.learning_intent,
        Some(LearningIntent::ProcedureProcess)
    );
    assert!(process_drafts
        .candidates
        .iter()
        .any(|candidate| candidate.activity_stage.contains("composition")));
}

#[test]
fn enumerable_sources_emit_every_mapping_in_the_non_derivable_direction() {
    let source = source_document(
        "src-enumerable",
        "NATO phonetic alphabet",
        "A is Alfa. B is Bravo. C is Charlie. D is Delta.",
    );

    let classification = classify_learning_intent(&source);
    assert_eq!(classification.intent, LearningIntent::EnumerableSet);
    assert_eq!(
        LearningIntent::from_label("enumerable_set"),
        Some(LearningIntent::EnumerableSet)
    );

    let drafts = FakeModelProvider
        .generate_drafts(&source)
        .expect("enumerable generation");
    assert_eq!(drafts.learning_intent, Some(LearningIntent::EnumerableSet));
    assert_eq!(drafts.candidates.len(), 4);
    assert_eq!(
        drafts
            .candidates
            .iter()
            .map(|candidate| candidate.answer.as_str())
            .collect::<Vec<_>>(),
        ["Alfa", "Bravo", "Charlie", "Delta"]
    );
    assert!(drafts.candidates.iter().all(|candidate| {
        candidate.distractors.is_empty()
            && candidate.question.contains("letter")
            && candidate.activity_stage == "production-recall"
            && candidate.evidence.is_some()
    }));
    assert!(drafts
        .candidates
        .iter()
        .all(|candidate| !candidate.question.contains("which letter")));
}

#[test]
fn enumerable_numbered_lists_preserve_order_and_source_evidence() {
    let source = source_document("src-list", "Ordered terms", "1. Alpha\n2. Beta\n3. Gamma");

    let drafts = FakeModelProvider
        .generate_drafts(&source)
        .expect("numbered list generation");
    assert_eq!(drafts.learning_intent, Some(LearningIntent::EnumerableSet));
    assert_eq!(
        drafts
            .candidates
            .iter()
            .map(|candidate| candidate.answer.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta", "Gamma"]
    );
    assert_eq!(drafts.candidates[0].index, 1);
    assert_eq!(drafts.candidates[2].index, 3);
    assert_eq!(drafts.candidates[1].evidence.as_deref(), Some("2. Beta"));
}

#[test]
fn sequential_sources_emit_one_verbatim_card_per_sentence() {
    let source = source_document(
        "src-sequence",
        "A quoted oath excerpt",
        "First faithful line. Second faithful line. Third faithful line.",
    );

    assert_eq!(
        classify_learning_intent(&source).intent,
        LearningIntent::VerbatimMemorization
    );
    let drafts = FakeModelProvider
        .generate_drafts(&source)
        .expect("sequential generation");

    assert_eq!(drafts.candidates.len(), 3);
    assert_eq!(
        drafts
            .candidates
            .iter()
            .map(|candidate| candidate.answer.as_str())
            .collect::<Vec<_>>(),
        [
            "First faithful line.",
            "Second faithful line.",
            "Third faithful line."
        ]
    );
    assert!(drafts.candidates.iter().all(|candidate| {
        candidate.activity_kind == GeneratedLearningActivityKind::Exercise
            && candidate.worked_solution.is_some()
            && candidate.evidence.as_deref() == Some(candidate.answer.as_str())
    }));
    assert_eq!(drafts.candidates[0].activity_stage, "free-recall");
    assert!(drafts.candidates[1..]
        .iter()
        .all(|candidate| candidate.activity_stage == "cued-recall"));
    assert!(drafts.candidates[1]
        .question
        .contains("First faithful line."));
}

#[test]
fn verbatim_intent_persists_recitation_prompt_ladder() {
    let directory = TempDirectory::new("verbatim-recitation");
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(source_document(
            "src-poem",
            "Hope is the thing with feathers",
            "\"Hope\" is the thing with feathers -\nThat perches in the soul -\nAnd sings the tune without the words -\nAnd never stops - at all -",
        ))
        .expect("source");

    let result = run_beta_generation_with_provider(
        &mut store,
        &FakeModelProvider,
        request("run-poem", "src-poem"),
    )
    .expect("generation");

    assert_eq!(result.rejected_draft_ids, Vec::<String>::new());
    let snapshot = store.snapshot();
    let stages = snapshot
        .generated_prompt_drafts
        .iter()
        .map(|draft| draft.activity_stage.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        stages.contains("cued-recall"),
        "missing cued stage: {stages:?}"
    );
    assert!(
        stages.contains("free-recall"),
        "missing free stage: {stages:?}"
    );
    assert!(
        snapshot.generated_prompt_drafts.iter().all(|draft| {
            draft.activity_kind == GeneratedLearningActivityKind::Exercise
                && matches!(
                    &draft.prompt,
                    Prompt::Exact(exact) if exact.kind == ExactPromptKind::Recitation
                )
        }),
        "verbatim sources must not become MC trivia: {:?}",
        snapshot.generated_prompt_drafts
    );
}

#[test]
fn provider_failure_is_recorded_as_human_readable_run_failure() {
    struct FailingProvider;

    impl DraftProvider for FailingProvider {
        fn model(&self) -> GeneratedPromptModel {
            test_model("failing-model")
        }

        fn generate_drafts(
            &self,
            _source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            Err(ProviderFailure::new(
                "model provider unavailable: connection refused",
            ))
        }
    }

    let directory = TempDirectory::new("failing-provider");
    let mut store = open_store_with_prose(&directory);

    let result = run_beta_generation_with_provider(
        &mut store,
        &FailingProvider,
        request("run-fail", "src-prose"),
    )
    .expect("generation completes despite provider failure");

    assert!(result.draft_ids.is_empty());
    assert_eq!(
        result.validation_failures,
        ["src-prose: model provider unavailable: connection refused"]
    );
    assert_eq!(
        store.snapshot().generation_runs[0].validation_failures,
        result.validation_failures
    );
}

#[test]
fn evidence_quote_not_found_in_source_is_rejected() {
    struct FabricatingProvider;

    impl DraftProvider for FabricatingProvider {
        fn model(&self) -> GeneratedPromptModel {
            test_model("fabricating-model")
        }

        fn generate_drafts(
            &self,
            _source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            Ok(ProviderDrafts {
                model: self.model(),
                learning_intent: Some(LearningIntent::FactRecall),
                candidates: vec![DraftCandidate {
                    index: 1,
                    concept: "Mitochondrial DNA".to_owned(),
                    question: "What shape is mitochondrial DNA?".to_owned(),
                    answer: "Circular".to_owned(),
                    evidence: Some("Mitochondrial DNA is circular.".to_owned()),
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

    let directory = TempDirectory::new("fabricating-provider");
    let mut store = open_store_with_prose(&directory);

    let result = run_beta_generation_with_provider(
        &mut store,
        &FabricatingProvider,
        request("run-fabricated", "src-prose"),
    )
    .expect("generation");

    assert_eq!(result.rejected_draft_ids.len(), 1);
    assert!(result.accepted_draft_ids.is_empty());
    let drafts = store.snapshot().generated_prompt_drafts;
    assert_eq!(
        drafts[0].validation.status,
        GeneratedPromptValidationStatus::Rejected
    );
    assert_eq!(
        drafts[0].validation.reasons,
        ["Evidence quote not found in cited source"]
    );
}

#[test]
fn provider_usage_is_aggregated_onto_the_generation_run() {
    struct MeteredProvider;

    impl DraftProvider for MeteredProvider {
        fn model(&self) -> GeneratedPromptModel {
            test_model("metered-model")
        }

        fn generate_drafts(
            &self,
            source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            let body = source.body.clone().unwrap_or_default();
            let sentence = body.split('.').next().unwrap_or_default().trim().to_owned();
            Ok(ProviderDrafts {
                model: self.model(),
                learning_intent: Some(LearningIntent::FactRecall),
                candidates: vec![DraftCandidate {
                    index: 1,
                    concept: "Mitochondria energy".to_owned(),
                    question: "What do mitochondria generate?".to_owned(),
                    answer: "Most of the cell's supply of adenosine triphosphate".to_owned(),
                    evidence: Some(sentence),
                    distractors: Vec::new(),
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition".to_owned(),
                    unsupported: false,
                }],
                failures: Vec::new(),
                usage: Some(ProviderUsage {
                    input_tokens: 1_200,
                    output_tokens: 300,
                    cost_usd_micros: Some(480),
                    latency_ms: 2_000,
                }),
            })
        }
    }

    let directory = TempDirectory::new("metered-provider");
    let mut store = open_store_with_prose(&directory);

    let result = run_beta_generation_with_provider(
        &mut store,
        &MeteredProvider,
        request("run-metered", "src-prose"),
    )
    .expect("generation");

    assert_eq!(result.accepted_draft_ids.len(), 1);
    assert_eq!(
        store.snapshot().generation_runs[0].usage,
        Some(GenerationRunUsage {
            input_tokens: 1_200,
            output_tokens: 300,
            cost_usd_micros: Some(480),
            latency_ms: 2_000,
        })
    );
}

#[test]
fn structured_duplicate_repair_never_calls_the_model_fallback() {
    struct ModelFallback;

    impl DraftProvider for ModelFallback {
        fn model(&self) -> GeneratedPromptModel {
            GeneratedPromptModel {
                provider: "model".to_owned(),
                name: "must-not-run".to_owned(),
                version: "v1".to_owned(),
            }
        }

        fn generate_drafts(
            &self,
            _source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            panic!("structured generation must not call the model fallback")
        }

        fn repair_drafts(
            &self,
            _source: &SourceDocument,
            _rejections: &[DraftRejection],
        ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
            panic!("structured duplicate repair must not call the model fallback")
        }
    }

    let directory = TempDirectory::new("structured-duplicate-repair");
    let mut store = open_store_with_prose(&directory);
    store
        .save_source_document(source_document(
            "src-structured",
            "Stable generation",
            "Concept: Stable generation\nQuestion: What stays stable?\nAnswer: The job identity.",
        ))
        .expect("structured source");
    let provider = FallbackProvider::new(&ModelFallback);

    let first = run_beta_generation_with_provider(
        &mut store,
        &provider,
        request("run-structured-1", "src-structured"),
    )
    .expect("first structured generation");
    assert_eq!(first.accepted_draft_ids.len(), 1);

    let replay = run_beta_generation_with_provider(
        &mut store,
        &provider,
        request("run-structured-2", "src-structured"),
    )
    .expect("replayed structured generation");
    assert!(
        replay.accepted_draft_ids.is_empty(),
        "the duplicate must stay rejected instead of escaping through model repair"
    );
}

#[test]
fn prose_repair_stays_with_the_model_fallback() {
    struct RepairingFallback {
        repaired: Cell<bool>,
    }

    impl DraftProvider for RepairingFallback {
        fn model(&self) -> GeneratedPromptModel {
            test_model("repairing-fallback")
        }

        fn generate_drafts(
            &self,
            _source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            Ok(ProviderDrafts {
                model: self.model(),
                learning_intent: Some(LearningIntent::FactRecall),
                candidates: vec![DraftCandidate {
                    index: 1,
                    concept: "Mitochondria energy".to_owned(),
                    question: "What do mitochondria generate?".to_owned(),
                    answer: "ATP".to_owned(),
                    evidence: Some("fabricated evidence".to_owned()),
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

        fn repair_drafts(
            &self,
            source: &SourceDocument,
            _rejections: &[DraftRejection],
        ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
            self.repaired.set(true);
            FakeModelProvider.generate_drafts(source).map(Some)
        }
    }

    let directory = TempDirectory::new("prose-fallback-repair");
    let mut store = open_store_with_prose(&directory);
    let fallback = RepairingFallback {
        repaired: Cell::new(false),
    };
    let provider = FallbackProvider::new(&fallback);

    let result = run_beta_generation_with_provider(
        &mut store,
        &provider,
        request("run-prose-repair", "src-prose"),
    )
    .expect("prose repair");

    assert!(
        fallback.repaired.get(),
        "fallback repair must run for prose"
    );
    assert!(
        !result.accepted_draft_ids.is_empty(),
        "the fallback's repaired draft must pass the shared gate: {:?}",
        result.validation_failures
    );
}

#[test]
fn fallback_stamps_drafts_with_the_provider_that_actually_ran() {
    let directory = TempDirectory::new("fallback-attribution");
    let mut store = open_store_with_prose(&directory);
    // A structured source the primary parser handles without the fallback.
    store
        .save_source_document(SourceDocument {
            id: "src-structured".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Structured".to_owned(),
            project_key: None,
            body: Some(
                [
                    "Concept: Mitochondria role",
                    "Question: What do mitochondria generate?",
                    "Answer: ATP",
                    "Reference: Mitochondria are organelles that generate most of the cell's supply of adenosine triphosphate.",
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
        .expect("structured source");

    let model = FakeModelProvider;
    let provider = FallbackProvider::new(&model);

    // Prose: primary finds nothing, fallback (fake model) runs.
    run_beta_generation_with_provider(&mut store, &provider, request("run-prose", "src-prose"))
        .expect("prose generation");
    // Structured: primary handles it, fallback never runs.
    run_beta_generation_with_provider(
        &mut store,
        &provider,
        request("run-structured", "src-structured"),
    )
    .expect("structured generation");

    let snapshot = store.snapshot();
    let prose_run = snapshot
        .generation_runs
        .iter()
        .find(|run| run.id == "run-prose")
        .expect("prose run");
    let structured_run = snapshot
        .generation_runs
        .iter()
        .find(|run| run.id == "run-structured")
        .expect("structured run");

    assert_eq!(prose_run.model, "fake-model");
    assert_eq!(
        structured_run.model, "deterministic-beta-generator",
        "structured drafts must be attributed to the parser that made them, not the fallback model"
    );
}

fn request(run_id: &str, source_id: &str) -> BetaGenerationRequest {
    BetaGenerationRequest {
        run_id: run_id.to_owned(),
        source_document_ids: vec![source_id.to_owned()],
        parent_review_unit_id: None,
        started_at: NOW,
        completed_at: Some(NOW + 1_000),
        default_due: NOW - 60_000,
        model: None,
    }
}

fn test_model(name: &str) -> GeneratedPromptModel {
    GeneratedPromptModel {
        provider: "test".to_owned(),
        name: name.to_owned(),
        version: "v1".to_owned(),
    }
}

fn open_store_with_prose(directory: &TempDirectory) -> BetaPersistenceStore {
    let path = directory.path().join("store.json");
    let mut store = BetaPersistenceStore::open(&path).expect("store");
    store
        .save_source_document(SourceDocument {
            id: "src-prose".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Mitochondria notes".to_owned(),
            project_key: None,
            body: Some(PROSE.to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    store
}

fn source_document(id: &str, title: &str, body: &str) -> SourceDocument {
    SourceDocument {
        id: id.to_owned(),
        kind: SourceDocumentKind::Text,
        title: title.to_owned(),
        project_key: None,
        body: Some(body.to_owned()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: None,
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
            "memory-engine-provider-{label}-{}-{stamp}",
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
