use std::{fs, path::PathBuf};

use memory_engine_generation::{
    run_beta_generation_with_provider, BetaGenerationRequest, DraftCandidate, DraftProvider,
    FakeModelProvider, FallbackProvider, ProviderDrafts, ProviderFailure, ProviderUsage,
    StructuredBlockProvider,
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
fn fallback_stamps_drafts_with_the_provider_that_actually_ran() {
    let directory = TempDirectory::new("fallback-attribution");
    let mut store = open_store_with_prose(&directory);
    // A structured source the primary parser handles without the fallback.
    store
        .save_source_document(SourceDocument {
            id: "src-structured".to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Structured".to_owned(),
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
            created_at: NOW,
            archived_at: None,
        })
        .expect("structured source");

    let structured = StructuredBlockProvider;
    let model = FakeModelProvider;
    let provider = FallbackProvider::new(&structured, &model);

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
            body: Some(PROSE.to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            created_at: NOW,
            archived_at: None,
        })
        .expect("source");

    store
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
