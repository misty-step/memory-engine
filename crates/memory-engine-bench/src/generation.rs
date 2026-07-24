//! Generation eval: deterministic judges over the prose→quiz corpus.
//!
//! `cargo run -p memory-engine-bench -- generation --model <id>` runs every
//! corpus source through the production beta generation runner, then scores
//! accepted output after trust-gate validation, duplicate suppression, and
//! repair. Deterministic judges keep models out of the loop (a `--judge
//! <model-id>` lane adds rubric quality scoring): runtime acceptance, provenance
//! (evidence-quote-actually-in-source, the same predicate the production
//! trust gate enforces), answerability, duplicates, expected draft counts,
//! and key-term coverage — alongside tokens, dollars, and latency.
//!
//! Receipts land in `docs/evals/`; CI never calls live models (the fake
//! provider is the default when no `--model` is given).

use std::{cell::RefCell, fmt::Write as _, fs, path::PathBuf};

use memory_engine::types::{Prompt, ReviewUnitId};
use memory_engine_generation::{
    candidates_duplicateish, classify_learning_intent, evidence_quote_matches,
    references_source_artifact, run_beta_generation_with_provider, BetaGenerationRequest,
    BetaGenerationStore, BridgeMaterialProvider, BridgeMaterialRequest, DraftCandidate,
    DraftProvider, FakeModelProvider, LearningIntent, ReviewPerformanceContext,
    SourceAuthorizationContext,
};
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider, PromptVariant};
use memory_engine_persistence::{
    BetaStoreSnapshot, ConceptReferenceNote, GeneratedLearningActivityKind, GeneratedPromptDraft,
    GeneratedPromptModel, GeneratedPromptValidationStatus, GenerationRun, ReferenceSpan,
    RemediationPackRecord, SourceDocument, SourceDocumentKind, SourcePermission,
};
use serde::Deserialize;

mod content_fit;
mod enumerable;

use content_fit::{ContentFitExpectation, ContentFitScore};
use enumerable::{EnumerableSetExpectation, EnumerableSetScore};

const NOW: i64 = 1_780_162_400_000;

#[derive(Clone, Debug, Deserialize)]
struct CorpusSource {
    id: String,
    title: String,
    category: String,
    body: String,
    expect: Expectations,
}

#[derive(Clone, Debug, Deserialize)]
struct Expectations {
    min_drafts: usize,
    max_drafts: usize,
    key_terms: Vec<String>,
    #[serde(default)]
    intent: Option<String>,
    #[serde(default)]
    required_activity_kinds: Vec<String>,
    #[serde(default)]
    required_activity_stage_terms: Vec<String>,
    #[serde(default)]
    forbidden_activity_kinds: Vec<String>,
    #[serde(default)]
    requires_distractors: bool,
    #[serde(default)]
    requires_variants: bool,
    #[serde(default)]
    content_fit: Option<ContentFitExpectation>,
    #[serde(default)]
    enumerable_set: Option<EnumerableSetExpectation>,
}

/// Deterministic judge scores for one source's provider output.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceScore {
    pub source_id: String,
    pub category: String,
    pub drafts: usize,
    /// Persisted drafts accepted by the runtime gate vs persisted drafts plus
    /// pre-persistence trust-gate failures.
    pub runtime_acceptance: f64,
    pub rejected_drafts: usize,
    pub validation_failures: usize,
    /// Fraction of drafts whose evidence quote is found in the source.
    pub provenance: f64,
    /// Fraction of drafts whose answer content appears in the source.
    pub answerability: f64,
    /// Fraction of drafts duplicating an earlier draft of the same source.
    pub duplicate_rate: f64,
    /// Draft count landed inside the annotated [min, max] expectation.
    pub count_in_range: bool,
    /// Fraction of annotated key terms covered by at least one draft.
    pub key_term_coverage: f64,
    /// Whether intent-specific activity kind/stage expectations were met.
    pub intent_shape_match: bool,
    /// Same-concept same-stage variant groups with distinct surface forms and
    /// no answer leakage in the question text.
    pub variant_quality: f64,
    /// Fraction of letter-keyed MCQ cards ("...the letter C?") whose answer and
    /// every distractor begin with the keyed letter — so the answer is never
    /// identifiable by its initial alone. 1.0 when no card keys on a letter.
    pub distractor_cohesion: f64,
    /// Fraction of cards whose question does not point back at the source
    /// artifact ("the source text", "the passage", ...). Guards the trust gate's
    /// self-referential rejection against regression.
    pub self_referential_free: f64,
    /// Content-type, coverage, shape, and directionality checks for fixtures
    /// whose learning objective is structural rather than sampled concept Q&A.
    pub content_fit: Option<ContentFitScore>,
    /// Exact membership, direction, and sequence checks for enumerable fixtures.
    pub enumerable_set: Option<EnumerableSetScore>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
    /// Transport-level provider failure, recorded instead of scores.
    pub provider_error: Option<String>,
    /// Model-judge rubric aggregate when `--judge` is enabled.
    pub judge: Option<crate::judge::JudgeAggregate>,
    pub judge_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeQualityScore {
    pub drafts: usize,
    pub easier_than_parent: f64,
    pub faithful_to_concept: f64,
    pub duplicate_rate: f64,
    pub passes: bool,
}

#[derive(Debug)]
pub struct GenerationBenchArgs {
    pub model: Option<String>,
    pub prompt: PromptVariant,
    pub judge: Option<String>,
    pub max_drafts: Option<usize>,
    pub out: Option<PathBuf>,
    pub baseline: Option<PathBuf>,
}

trait GenerationProvider: DraftProvider + BridgeMaterialProvider {}

impl<T> GenerationProvider for T where T: DraftProvider + BridgeMaterialProvider {}

/// Parse `generation` subcommand arguments.
///
/// # Errors
///
/// Returns a usage message on unknown flags or missing values.
pub fn parse_args(arguments: &[String]) -> Result<GenerationBenchArgs, String> {
    let mut parsed = GenerationBenchArgs {
        model: None,
        prompt: PromptVariant::Principled,
        judge: None,
        max_drafts: None,
        out: None,
        baseline: None,
    };
    let mut iterator = arguments.iter();
    while let Some(flag) = iterator.next() {
        match flag.as_str() {
            "--model" => {
                parsed.model = Some(
                    iterator
                        .next()
                        .ok_or("--model requires a model id, e.g. deepseek/deepseek-v4-flash")?
                        .clone(),
                );
            }
            "--prompt" => {
                parsed.prompt = match iterator
                    .next()
                    .ok_or("--prompt requires `minimal` or `principled`")?
                    .as_str()
                {
                    "minimal" => PromptVariant::Minimal,
                    "principled" => PromptVariant::Principled,
                    other => return Err(format!("unknown prompt variant: {other}")),
                };
            }
            "--judge" => {
                parsed.judge = Some(
                    iterator
                        .next()
                        .ok_or("--judge requires a model id, e.g. openai/gpt-5.4")?
                        .clone(),
                );
            }
            "--max-drafts" => {
                let value = iterator
                    .next()
                    .ok_or("--max-drafts requires a positive integer")?;
                let max_drafts = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --max-drafts value: {value}"))?;
                if max_drafts == 0 {
                    return Err("--max-drafts must be greater than zero".to_owned());
                }
                parsed.max_drafts = Some(max_drafts);
            }
            "--out" => {
                parsed.out = Some(PathBuf::from(
                    iterator.next().ok_or("--out requires a file path")?,
                ));
            }
            "--baseline" => {
                parsed.baseline = Some(PathBuf::from(
                    iterator.next().ok_or("--baseline requires a receipt path")?,
                ));
            }
            other => {
                return Err(format!(
                    "unknown flag {other}; usage: generation [--model <id>] [--prompt minimal|principled] [--judge <id>] [--max-drafts <n>] [--out <path>] [--baseline <receipt>]"
                ))
            }
        }
    }

    Ok(parsed)
}

/// Run the generation eval and print (and optionally write) the receipt.
///
/// # Errors
///
/// Returns a message when the corpus cannot be read or provider
/// configuration is invalid.
pub fn run(arguments: &[String]) -> Result<(), String> {
    let parsed = parse_args(arguments)?;
    let corpus = load_corpus()?;

    let (provider, label): (Box<dyn GenerationProvider>, String) = match &parsed.model {
        None => (Box::new(FakeModelProvider), "fixture/fake-model".to_owned()),
        Some(model) => {
            let mut config = OpenRouterConfig::from_env()?;
            config.model.clone_from(model);
            config.prompt = parsed.prompt;
            if let Some(max_drafts) = parsed.max_drafts {
                config.max_drafts = max_drafts;
            }
            let mut label = format!("openrouter/{model} ({})", parsed.prompt.label());
            if let Some(max_drafts) = parsed.max_drafts {
                let _ = write!(label, " · max_drafts: {max_drafts}");
            }
            (Box::new(OpenRouterProvider::new(config)), label)
        }
    };
    let judge = parsed
        .judge
        .as_ref()
        .map(|judge_model| -> Result<OpenRouterProvider, String> {
            let mut config = OpenRouterConfig::from_env()?;
            config.model.clone_from(judge_model);
            Ok(OpenRouterProvider::new(config))
        })
        .transpose()?;
    let mut label = label;
    if let Some(judge_model) = &parsed.judge {
        let _ = write!(label, " · judge: {judge_model}");
        let generator = parsed.model.as_deref().unwrap_or("fixture/fake-model");
        if crate::judge::same_model_family(generator, judge_model) {
            label.push_str(" ⚠ same model family as generator (self-preference risk)");
        }
    }

    let scores: Vec<SourceScore> = corpus
        .iter()
        .map(|source| score_source(provider.as_ref(), judge.as_ref(), source))
        .collect();
    let bridge = bridge_quality_fixture(provider.as_ref());
    let baseline = parsed
        .baseline
        .as_ref()
        .map(|path| -> Result<Vec<(String, f64)>, String> {
            let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
            Ok(crate::stats::parse_keep_rates(&contents))
        })
        .transpose()?;
    let receipt = render_receipt(&label, &scores, &bridge, baseline.as_deref());
    println!("{receipt}");
    if let Some(path) = parsed.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &receipt).map_err(|error| error.to_string())?;
        println!("receipt written to {}", path.display());
    }

    Ok(())
}

fn load_corpus() -> Result<Vec<CorpusSource>, String> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/generation");
    let mut entries: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read corpus at {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    entries.sort();
    entries
        .iter()
        .map(|path| {
            let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))
        })
        .collect()
}

fn score_source(
    provider: &dyn DraftProvider,
    model_judge: Option<&OpenRouterProvider>,
    source: &CorpusSource,
) -> SourceScore {
    let document = SourceDocument {
        id: source.id.clone(),
        kind: SourceDocumentKind::Text,
        title: source.title.clone(),
        project_key: None,
        body: Some(source.body.clone()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        ttl_expires_at: None,
        created_at: NOW,
        archived_at: None,
    };

    let request = BetaGenerationRequest {
        run_id: format!("generation-bench-{}", source.id),
        source_document_ids: vec![source.id.clone()],
        parent_review_unit_id: None,
        started_at: NOW,
        completed_at: Some(NOW),
        default_due: NOW,
        model: None,
        pending: false,
    };
    let fallback_learning_intent = Some(classify_learning_intent(&document).intent);
    let mut bench_store = BenchGenerationStore::new(document);
    let recording_provider = RecordingDraftProvider::new(provider);

    match run_beta_generation_with_provider(&mut bench_store, &recording_provider, request) {
        Ok(result) => {
            let candidates = bench_store.accepted_candidates(&result.accepted_draft_ids);
            let run = bench_store.completed_run(&result.run_id);
            let usage = run.and_then(|run| run.usage.clone());
            let learning_intent = recording_provider
                .learning_intent()
                .or(fallback_learning_intent);
            let emitted = result.accepted_draft_ids.len()
                + result.rejected_draft_ids.len()
                + result.validation_failures.len();
            let mut score =
                deterministic_judges(&source.body, &source.expect, learning_intent, &candidates);
            score.source_id.clone_from(&source.id);
            score.category.clone_from(&source.category);
            score.rejected_drafts = result.rejected_draft_ids.len();
            score.validation_failures = result.validation_failures.len();
            score.runtime_acceptance = if emitted == 0 {
                1.0
            } else {
                fraction(result.accepted_draft_ids.len(), emitted)
            };
            if let Some(usage) = usage {
                score.input_tokens = usage.input_tokens;
                score.output_tokens = usage.output_tokens;
                score.cost_usd_micros = usage.cost_usd_micros;
                score.latency_ms = usage.latency_ms;
            }
            if let Some(judge) = model_judge {
                match crate::judge::judge_source(judge, &source.title, &source.body, &candidates) {
                    Ok(aggregate) => score.judge = aggregate,
                    Err(failure) => score.judge_error = Some(failure.to_string()),
                }
            }
            score
        }
        Err(failure) => SourceScore {
            source_id: source.id.clone(),
            category: source.category.clone(),
            drafts: 0,
            runtime_acceptance: 0.0,
            rejected_drafts: 0,
            validation_failures: 1,
            provenance: 0.0,
            answerability: 0.0,
            duplicate_rate: 0.0,
            count_in_range: false,
            key_term_coverage: 0.0,
            intent_shape_match: source.expect.intent.is_none(),
            variant_quality: 0.0,
            distractor_cohesion: 1.0,
            self_referential_free: 1.0,
            content_fit: None,
            enumerable_set: enumerable::score(source.expect.enumerable_set.as_ref(), &[]),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd_micros: None,
            latency_ms: 0,
            provider_error: Some(failure.to_string()),
            judge: None,
            judge_error: None,
        },
    }
}

struct RecordingDraftProvider<'a> {
    inner: &'a dyn DraftProvider,
    learning_intent: RefCell<Option<LearningIntent>>,
}

impl<'a> RecordingDraftProvider<'a> {
    fn new(inner: &'a dyn DraftProvider) -> Self {
        Self {
            inner,
            learning_intent: RefCell::new(None),
        }
    }

    fn learning_intent(&self) -> Option<LearningIntent> {
        *self.learning_intent.borrow()
    }

    fn record_learning_intent(&self, drafts: &memory_engine_generation::ProviderDrafts) {
        if let Some(intent) = drafts.learning_intent {
            *self.learning_intent.borrow_mut() = Some(intent);
        }
    }
}

impl DraftProvider for RecordingDraftProvider<'_> {
    fn model(&self) -> GeneratedPromptModel {
        self.inner.model()
    }

    fn generate_drafts(
        &self,
        source: &SourceDocument,
    ) -> Result<memory_engine_generation::ProviderDrafts, memory_engine_generation::ProviderFailure>
    {
        let drafts = self.inner.generate_drafts(source)?;
        self.record_learning_intent(&drafts);
        Ok(drafts)
    }

    fn repair_drafts(
        &self,
        source: &SourceDocument,
        rejections: &[memory_engine_generation::DraftRejection],
    ) -> Result<
        Option<memory_engine_generation::ProviderDrafts>,
        memory_engine_generation::ProviderFailure,
    > {
        let repaired = self.inner.repair_drafts(source, rejections)?;
        if let Some(drafts) = &repaired {
            self.record_learning_intent(drafts);
        }
        Ok(repaired)
    }
}

struct BenchGenerationStore {
    snapshot: BetaStoreSnapshot,
}

impl BenchGenerationStore {
    fn new(source: SourceDocument) -> Self {
        Self {
            snapshot: BetaStoreSnapshot {
                source_documents: vec![source],
                ..BetaStoreSnapshot::default()
            },
        }
    }

    fn completed_run(&self, run_id: &str) -> Option<&GenerationRun> {
        self.snapshot
            .generation_runs
            .iter()
            .find(|run| run.id == run_id && run.completed_at.is_some())
    }

    fn accepted_candidates(&self, accepted_draft_ids: &[String]) -> Vec<DraftCandidate> {
        accepted_draft_ids
            .iter()
            .filter_map(|id| {
                self.snapshot
                    .generated_prompt_drafts
                    .iter()
                    .find(|draft| {
                        draft.id == *id
                            && draft.validation.status == GeneratedPromptValidationStatus::Accepted
                    })
                    .map(|draft| self.draft_to_candidate(draft))
            })
            .collect()
    }

    fn draft_to_candidate(&self, draft: &GeneratedPromptDraft) -> DraftCandidate {
        let (question, answer, distractors) = match &draft.prompt {
            Prompt::Mcq {
                prompt,
                choices,
                correct_choice,
                ..
            } => (
                prompt.clone(),
                correct_choice.clone(),
                choices
                    .iter()
                    .filter(|choice| *choice != correct_choice)
                    .cloned()
                    .collect(),
            ),
            Prompt::Boolean {
                prompt,
                correct_answer,
                ..
            } => (prompt.clone(), correct_answer.to_string(), Vec::new()),
            Prompt::Exact(exact) => (
                exact.prompt.clone(),
                exact.accepted_answers.first().cloned().unwrap_or_default(),
                Vec::new(),
            ),
        };
        let evidence = draft
            .reference_span_ids
            .first()
            .and_then(|reference_id| {
                self.snapshot
                    .reference_spans
                    .iter()
                    .find(|reference| reference.id == *reference_id)
            })
            .map(|reference| reference.text.clone());
        let concept = draft
            .queue
            .concept_key
            .clone()
            .unwrap_or_else(|| draft.review_unit_id.as_str().to_owned());

        DraftCandidate {
            index: 1,
            concept,
            question,
            answer,
            evidence,
            distractors,
            worked_solution: draft.worked_solution.clone(),
            activity_kind: draft.activity_kind.clone(),
            activity_stage: draft.activity_stage.clone(),
            unsupported: false,
        }
    }

    fn upsert_reference_span(&mut self, reference: ReferenceSpan) {
        self.snapshot
            .reference_spans
            .retain(|existing| existing.id != reference.id);
        self.snapshot.reference_spans.push(reference);
    }

    fn upsert_generation_run(&mut self, run: GenerationRun) {
        self.snapshot
            .generation_runs
            .retain(|existing| existing.id != run.id);
        self.snapshot.generation_runs.push(run);
    }

    fn upsert_concept_note(&mut self, note: ConceptReferenceNote) {
        self.snapshot
            .concept_reference_notes
            .retain(|existing| existing.concept_key != note.concept_key);
        self.snapshot.concept_reference_notes.push(note);
    }

    fn upsert_generated_prompt_draft(&mut self, draft: GeneratedPromptDraft) {
        self.snapshot
            .generated_prompt_drafts
            .retain(|existing| existing.id != draft.id);
        self.snapshot.generated_prompt_drafts.push(draft);
    }

    fn upsert_remediation_pack(&mut self, pack: RemediationPackRecord) {
        self.snapshot
            .remediation_packs
            .retain(|existing| existing.id != pack.id);
        self.snapshot.remediation_packs.push(pack);
    }
}

impl BetaGenerationStore for BenchGenerationStore {
    type Error = String;

    fn snapshot(&self) -> Result<BetaStoreSnapshot, Self::Error> {
        Ok(self.snapshot.clone())
    }

    fn save_generation_run(&mut self, run: GenerationRun) -> Result<GenerationRun, Self::Error> {
        self.upsert_generation_run(run.clone());
        Ok(run)
    }

    fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, Self::Error> {
        self.upsert_reference_span(reference.clone());
        Ok(reference)
    }

    fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, Self::Error> {
        self.upsert_concept_note(note.clone());
        Ok(note)
    }

    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error> {
        self.upsert_generated_prompt_draft(draft.clone());
        Ok(draft)
    }

    fn save_remediation_pack(
        &mut self,
        pack: RemediationPackRecord,
    ) -> Result<RemediationPackRecord, Self::Error> {
        self.upsert_remediation_pack(pack.clone());
        Ok(pack)
    }
}

/// Deterministic judges over one source's candidates. No model in the loop —
/// they verify mechanical properties only; rubric quality judgment is the
/// model judge's job (`crate::judge`).
fn deterministic_judges(
    body: &str,
    expect: &Expectations,
    learning_intent: Option<LearningIntent>,
    candidates: &[DraftCandidate],
) -> SourceScore {
    let drafts = candidates.len();
    let provenance = fraction(
        candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .evidence
                    .as_deref()
                    .is_some_and(|quote| evidence_quote_matches(body, quote))
            })
            .count(),
        drafts,
    );
    let answerability = fraction(
        candidates
            .iter()
            .filter(|candidate| answer_supported(body, &candidate.answer))
            .count(),
        drafts,
    );
    let duplicates = candidates
        .iter()
        .enumerate()
        .filter(|(index, candidate)| {
            candidates[..*index]
                .iter()
                .any(|seen| candidates_duplicateish(seen, candidate))
        })
        .count();
    let covered_terms = expect
        .key_terms
        .iter()
        .filter(|term| {
            candidates.iter().any(|candidate| {
                let haystack = format!(
                    "{} {} {} {}",
                    candidate.concept,
                    candidate.question,
                    candidate.answer,
                    candidate.distractors.join(" ")
                );
                normalize(&haystack).contains(&normalize(term))
            })
        })
        .count();
    let content_fit = content_fit::score(expect.content_fit.as_ref(), learning_intent, candidates);

    SourceScore {
        source_id: String::new(),
        category: String::new(),
        drafts,
        runtime_acceptance: 1.0,
        rejected_drafts: 0,
        validation_failures: 0,
        provenance,
        answerability,
        duplicate_rate: fraction(duplicates, drafts),
        count_in_range: drafts >= expect.min_drafts && drafts <= expect.max_drafts,
        key_term_coverage: fraction(covered_terms, expect.key_terms.len()),
        intent_shape_match: intent_shape_matches(expect, learning_intent, candidates),
        variant_quality: variant_quality(candidates),
        distractor_cohesion: distractor_cohesion(candidates),
        self_referential_free: self_referential_free(candidates),
        content_fit,
        enumerable_set: enumerable::score(expect.enumerable_set.as_ref(), candidates),
        input_tokens: 0,
        output_tokens: 0,
        cost_usd_micros: None,
        latency_ms: 0,
        provider_error: None,
        judge: None,
        judge_error: None,
    }
}

/// Letter-keyed MCQ cohesion. When a multiple-choice question fixes a specific
/// letter ("...the letter C?"), every option must begin with that letter, or the
/// answer is identifiable by its initial alone and the distractors are dead
/// weight (the Charlie/Delta/Bravo/Echo bug). Returns the fraction of such
/// letter-keyed MCQ cards whose answer and all distractors share the keyed
/// letter; a source with no letter-keyed MCQ scores 1.0 (nothing to fault).
fn distractor_cohesion(candidates: &[DraftCandidate]) -> f64 {
    let keyed = candidates
        .iter()
        .filter(|candidate| !candidate.distractors.is_empty())
        .filter_map(|candidate| {
            question_target_letter(&candidate.question).map(|letter| (candidate, letter))
        })
        .collect::<Vec<_>>();
    if keyed.is_empty() {
        return 1.0;
    }
    let cohesive = keyed
        .iter()
        .filter(|(candidate, letter)| {
            std::iter::once(&candidate.answer)
                .chain(candidate.distractors.iter())
                .all(|option| starts_with_letter(option, *letter))
        })
        .count();
    fraction(cohesive, keyed.len())
}

/// The single letter a question keys on, e.g. "...the letter C?" -> 'c'.
fn question_target_letter(question: &str) -> Option<char> {
    let lower = question.to_lowercase();
    let start = lower.find("letter ")? + "letter ".len();
    lower[start..]
        .chars()
        .next()
        .filter(char::is_ascii_alphabetic)
}

fn starts_with_letter(option: &str, letter: char) -> bool {
    option
        .chars()
        .find(|character| character.is_alphanumeric())
        .is_some_and(|character| character.eq_ignore_ascii_case(&letter))
}

/// Fraction of cards whose question does not point back at the source artifact.
/// A self-referential meta-question ("the subject of the source text") is
/// unanswerable once the source is gone; the trust gate rejects these, and this
/// eval independently guards that protection against regression.
fn self_referential_free(candidates: &[DraftCandidate]) -> f64 {
    if candidates.is_empty() {
        return 1.0;
    }
    let clean = candidates
        .iter()
        .filter(|candidate| !references_source_artifact(&candidate.question))
        .count();
    fraction(clean, candidates.len())
}

fn intent_shape_matches(
    expect: &Expectations,
    learning_intent: Option<LearningIntent>,
    candidates: &[DraftCandidate],
) -> bool {
    if expect.intent.is_none()
        && expect.required_activity_kinds.is_empty()
        && expect.required_activity_stage_terms.is_empty()
        && expect.forbidden_activity_kinds.is_empty()
        && !expect.requires_distractors
        && !expect.requires_variants
    {
        return true;
    }

    let intent_label_matches = expect
        .intent
        .as_deref()
        .is_none_or(|expected| learning_intent.is_some_and(|actual| actual.label() == expected));
    let required_kinds_match = expect.required_activity_kinds.iter().all(|kind| {
        candidates
            .iter()
            .any(|candidate| activity_kind_label(&candidate.activity_kind) == kind)
    });
    let required_stages_match = expect.required_activity_stage_terms.iter().all(|term| {
        candidates.iter().any(|candidate| {
            candidate
                .activity_stage
                .to_lowercase()
                .contains(&term.to_lowercase())
        })
    });
    let forbidden_kinds_absent = expect.forbidden_activity_kinds.iter().all(|kind| {
        candidates
            .iter()
            .all(|candidate| activity_kind_label(&candidate.activity_kind) != kind)
    });
    let distractors_match = !expect.requires_distractors
        || candidates
            .iter()
            .any(|candidate| !candidate.distractors.is_empty());
    let variants_match =
        !expect.requires_variants || (variant_quality(candidates) - 1.0).abs() < f64::EPSILON;

    intent_label_matches
        && required_kinds_match
        && required_stages_match
        && forbidden_kinds_absent
        && distractors_match
        && variants_match
}

fn variant_quality(candidates: &[DraftCandidate]) -> f64 {
    let mut groups: std::collections::BTreeMap<String, Vec<&DraftCandidate>> =
        std::collections::BTreeMap::new();
    for candidate in candidates {
        let key = [
            normalize(&candidate.concept),
            activity_kind_label(&candidate.activity_kind).to_owned(),
            candidate.activity_stage.trim().to_lowercase(),
        ]
        .join("\0");
        groups.entry(key).or_default().push(candidate);
    }
    let variant_groups = groups
        .into_values()
        .filter(|group| group.len() >= 2)
        .collect::<Vec<_>>();
    if variant_groups.is_empty() {
        return 0.0;
    }
    let passing = variant_groups
        .iter()
        .filter(|group| variant_group_passes(group))
        .count();
    fraction(passing, variant_groups.len())
}

fn variant_group_passes(candidates: &[&DraftCandidate]) -> bool {
    let mut surfaces = std::collections::BTreeSet::new();
    for candidate in candidates {
        let question = normalize(&candidate.question);
        let answer = normalize(&candidate.answer);
        if !surfaces.insert(question.clone()) {
            return false;
        }
        if question_leaks_answer(&question, &answer) {
            return false;
        }
    }

    for (index, left) in candidates.iter().enumerate() {
        for right in &candidates[index + 1..] {
            if surface_similarity(&left.question, &right.question) >= 0.85 {
                return false;
            }
        }
    }
    true
}

fn question_leaks_answer(question: &str, answer: &str) -> bool {
    let answer_tokens = answer.split_whitespace().collect::<Vec<_>>();
    if answer_tokens.is_empty() {
        return false;
    }
    let question_tokens = question.split_whitespace().collect::<Vec<_>>();
    question_tokens
        .windows(answer_tokens.len())
        .any(|window| window == answer_tokens.as_slice())
}

fn surface_similarity(left: &str, right: &str) -> f64 {
    let left_normalized = normalize(left);
    let left = left_normalized
        .split_whitespace()
        .collect::<std::collections::BTreeSet<_>>();
    let right_normalized = normalize(right);
    let right = right_normalized
        .split_whitespace()
        .collect::<std::collections::BTreeSet<_>>();
    let union = left.union(&right).count();
    if union == 0 {
        return 1.0;
    }
    fraction(left.intersection(&right).count(), union)
}

#[cfg(test)]
fn shape_signature(candidates: &[DraftCandidate]) -> String {
    candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}:{}:{}",
                activity_kind_label(&candidate.activity_kind),
                candidate.activity_stage,
                if candidate.distractors.is_empty() {
                    "short"
                } else {
                    "choice"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn activity_kind_label(kind: &GeneratedLearningActivityKind) -> &'static str {
    match kind {
        GeneratedLearningActivityKind::Quiz => "quiz",
        GeneratedLearningActivityKind::Exercise => "exercise",
    }
}

/// An answer is "supported" when at least 60% of its content words appear in
/// the source. String-level only; semantic paraphrase scores low by design.
fn answer_supported(body: &str, answer: &str) -> bool {
    let body = normalize(body);
    let body_words: std::collections::BTreeSet<&str> = body.split(' ').collect();
    let answer = normalize(answer);
    let mut words: Vec<&str> = answer.split(' ').filter(|word| word.len() > 2).collect();
    // Short answers (numbers, "to be") have no long content words; fall back to
    // matching the answer's actual words so they are not credited for free.
    if words.is_empty() {
        words = answer.split(' ').filter(|word| !word.is_empty()).collect();
    }
    if words.is_empty() {
        return false;
    }
    let found = words
        .iter()
        .filter(|word| body_words.contains(**word))
        .count();

    fraction(found, words.len()) >= 0.6
}

fn normalize(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space = true;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            previous_space = false;
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    while normalized.ends_with(' ') {
        normalized.pop();
    }

    normalized
}

#[allow(clippy::cast_precision_loss)]
fn fraction(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Render the model-judge section when `--judge` ran: per-source rubric
/// means (1-5), keep rate, and the judge's notes on rejected drafts.
fn render_model_judge(
    receipt: &mut String,
    scores: &[SourceScore],
    baseline: Option<&[(String, f64)]>,
) {
    if scores
        .iter()
        .all(|score| score.judge.is_none() && score.judge_error.is_none())
    {
        return;
    }

    let _ = writeln!(receipt, "## Model judge (rubric 1-5)");
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "| source | faithfulness | question quality | distractors | keep | judge cost |"
    );
    let _ = writeln!(receipt, "| --- | --- | --- | --- | --- | --- |");
    let mut judge_cost: i64 = 0;
    let mut reject_notes = Vec::new();
    for score in scores {
        if let Some(error) = &score.judge_error {
            let _ = writeln!(
                receipt,
                "| {} | — | — | — | — | JUDGE FAILED: {error} |",
                score.source_id
            );
            continue;
        }
        let Some(judge) = &score.judge else { continue };
        judge_cost += judge.cost_usd_micros.unwrap_or(0);
        reject_notes.extend(
            judge
                .reject_notes
                .iter()
                .map(|note| format!("{}: {note}", score.source_id)),
        );
        let _ = writeln!(
            receipt,
            "| {} | {:.1} | {:.1} | {:.1} | {:.0}% | {} |",
            score.source_id,
            judge.faithfulness,
            judge.question_quality,
            judge.distractor_quality,
            judge.keep_rate * 100.0,
            format_cost(judge.cost_usd_micros),
        );
    }
    let judged: Vec<_> = scores
        .iter()
        .filter_map(|score| score.judge.as_ref())
        .collect();
    if !judged.is_empty() {
        #[allow(clippy::cast_precision_loss)]
        let count = judged.len() as f64;
        let mean = |value: fn(&crate::judge::JudgeAggregate) -> f64| {
            judged.iter().map(|judge| value(judge)).sum::<f64>() / count
        };
        let _ = writeln!(receipt);
        let _ = writeln!(
            receipt,
            "- Judge means: faithfulness {:.1} · question quality {:.1} · distractors {:.1} · keep rate {:.0}% · judge cost {}",
            mean(|judge| judge.faithfulness),
            mean(|judge| judge.question_quality),
            mean(|judge| judge.distractor_quality),
            mean(|judge| judge.keep_rate) * 100.0,
            format_cost(Some(judge_cost)),
        );

        let current_keep: Vec<(String, f64)> = scores
            .iter()
            .filter_map(|score| {
                score
                    .judge
                    .as_ref()
                    .map(|judge| (score.source_id.clone(), judge.keep_rate))
            })
            .collect();
        render_keep_rate_rigor(receipt, &current_keep, baseline);
    }
    if !reject_notes.is_empty() {
        let _ = writeln!(receipt);
        let _ = writeln!(receipt, "Judge would not keep:");
        let _ = writeln!(receipt);
        for note in reject_notes {
            let _ = writeln!(receipt, "- {note}");
        }
    }
    let _ = writeln!(receipt);
}

/// Keep rate is the binary keep/drop signal; report it with a source-clustered
/// 95% CI and, against a baseline, a paired verdict — so a few points of keep-rate
/// movement read as noise, not a win.
fn render_keep_rate_rigor(
    receipt: &mut String,
    current_keep: &[(String, f64)],
    baseline: Option<&[(String, f64)]>,
) {
    let keep_values: Vec<f64> = current_keep.iter().map(|(_, rate)| *rate).collect();
    let Some(interval) = crate::stats::mean_ci_95(&keep_values) else {
        return;
    };
    let _ = writeln!(
        receipt,
        "- Keep rate (source-clustered, n={}): {:.0}% (95% CI ±{:.0}pp)",
        keep_values.len(),
        interval.mean * 100.0,
        interval.half_width * 100.0,
    );
    if let Some(verdict) =
        baseline.and_then(|baseline| crate::stats::paired_verdict(current_keep, baseline))
    {
        let _ = writeln!(
            receipt,
            "- Paired vs baseline ({} sources): keep Δ {:+.1}pp (95% CI ±{:.1}pp) — {}",
            verdict.paired,
            verdict.mean_delta * 100.0,
            verdict.half_width * 100.0,
            if verdict.within_noise {
                "**within noise** — CI includes 0 (not detected; not proof of no change)"
            } else {
                "**detectable** — CI excludes 0"
            },
        );
    }
    let _ = writeln!(
        receipt,
        "- Power: ~{} sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.",
        keep_values.len(),
    );
}

fn render_receipt(
    label: &str,
    scores: &[SourceScore],
    bridge: &Result<BridgeQualityScore, String>,
    baseline: Option<&[(String, f64)]>,
) -> String {
    let mut receipt = String::new();
    let _ = writeln!(receipt, "# Generation eval receipt");
    let _ = writeln!(receipt);
    let _ = writeln!(receipt, "- Provider: {label}");
    let _ = writeln!(receipt, "- Corpus: {} sources", scores.len());
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |"
    );
    let _ = writeln!(
        receipt,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    );
    render_score_rows(&mut receipt, scores);
    render_enumerable_scores(&mut receipt, scores);
    let _ = writeln!(receipt);
    render_model_judge(&mut receipt, scores, baseline);
    render_receipt_totals(&mut receipt, scores, bridge);

    receipt
}

fn render_score_rows(receipt: &mut String, scores: &[SourceScore]) {
    for score in scores {
        if let Some(error) = &score.provider_error {
            let _ = writeln!(
                receipt,
                "| {} | {} | — | — | 1 | 0% | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | FAILED: {error} |",
                score.source_id, score.category
            );
            continue;
        }
        let (content_kind, content_cover, content_shape, direction) =
            content_fit::cells(score.content_fit.as_ref());
        let _ = writeln!(
            receipt,
            "| {} | {} | {} | {} | {} | {:.0}% | {:.0}% | {:.0}% | {:.0}% | {} | {:.0}% | {} | {} | {} | {} | {} | {:.0}% | {:.0}% | {:.0}% | {}/{} | {} | {}ms |",
            score.source_id,
            score.category,
            score.drafts,
            score.rejected_drafts,
            score.validation_failures,
            score.runtime_acceptance * 100.0,
            score.provenance * 100.0,
            score.answerability * 100.0,
            score.duplicate_rate * 100.0,
            if score.count_in_range { "yes" } else { "NO" },
            score.key_term_coverage * 100.0,
            if score.intent_shape_match { "yes" } else { "NO" },
            content_kind,
            content_cover,
            content_shape,
            direction,
            score.variant_quality * 100.0,
            score.distractor_cohesion * 100.0,
            score.self_referential_free * 100.0,
            score.input_tokens,
            score.output_tokens,
            format_cost(score.cost_usd_micros),
            score.latency_ms,
        );
    }
}

fn render_enumerable_scores(receipt: &mut String, scores: &[SourceScore]) {
    let enumerable = scores
        .iter()
        .filter_map(|score| score.enumerable_set.as_ref().map(|set| (score, set)))
        .collect::<Vec<_>>();
    if enumerable.is_empty() {
        return;
    }
    let _ = writeln!(receipt);
    let _ = writeln!(receipt, "## Enumerable-set completeness");
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "| source | expected | observed | covered | missing | duplicate | invented | misassigned | reversed | order | direction | pass |"
    );
    let _ = writeln!(
        receipt,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |"
    );
    for (source, set) in enumerable {
        let _ = writeln!(
            receipt,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            source.source_id,
            set.expected,
            set.observed,
            set.covered,
            set.missing,
            set.duplicates,
            set.invented,
            set.misassigned,
            set.reversed,
            if set.order_ok { "yes" } else { "NO" },
            if set.direction_ok { "yes" } else { "NO" },
            if set.passes() { "yes" } else { "NO" },
        );
    }
}

fn render_receipt_totals(
    receipt: &mut String,
    scores: &[SourceScore],
    bridge: &Result<BridgeQualityScore, String>,
) {
    let judged: Vec<&SourceScore> = scores
        .iter()
        .filter(|score| score.provider_error.is_none())
        .collect();
    let failed = scores.len() - judged.len();
    let mut latencies: Vec<u64> = judged.iter().map(|score| score.latency_ms).collect();
    latencies.sort_unstable();
    let total_cost: i64 = judged
        .iter()
        .filter_map(|score| score.cost_usd_micros)
        .sum();
    let _ = writeln!(receipt, "## Totals");
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "- Provider failures: {failed}/{} sources",
        scores.len()
    );
    let _ = writeln!(
        receipt,
        "- Mean provenance: {:.0}% · mean answerability: {:.0}% · mean key-term coverage: {:.0}% · count-in-range: {}/{}",
        mean(&judged, |score| score.provenance) * 100.0,
        mean(&judged, |score| score.answerability) * 100.0,
        mean(&judged, |score| score.key_term_coverage) * 100.0,
        judged.iter().filter(|score| score.count_in_range).count(),
        judged.len(),
    );
    let shaped = judged
        .iter()
        .filter(|score| score.intent_shape_match)
        .count();
    let _ = writeln!(
        receipt,
        "- Intent shape matches: {shaped}/{} sources",
        judged.len()
    );
    let content_fit = judged
        .iter()
        .filter_map(|score| score.content_fit.as_ref())
        .collect::<Vec<_>>();
    if !content_fit.is_empty() {
        #[allow(clippy::cast_precision_loss)]
        let count = content_fit.len() as f64;
        let mean_coverage = content_fit.iter().map(|score| score.coverage).sum::<f64>() / count;
        let _ = writeln!(
            receipt,
            "- Content fit matches: {}/{} sources · mean required-unit coverage {:.0}%",
            content_fit.iter().filter(|score| score.passes()).count(),
            content_fit.len(),
            mean_coverage * 100.0,
        );
    }
    render_bridge_totals(receipt, bridge);
    let _ = writeln!(
        receipt,
        "- Total cost: {} · mean per source: {}",
        format_cost(Some(total_cost)),
        format_cost(Some(if judged.is_empty() {
            0
        } else {
            total_cost / i64::try_from(judged.len()).unwrap_or(1)
        })),
    );
    let _ = writeln!(
        receipt,
        "- Latency p50: {}ms · p95: {}ms",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
    );
}

fn render_bridge_totals(receipt: &mut String, bridge: &Result<BridgeQualityScore, String>) {
    match bridge {
        Ok(bridge) => {
            let _ = writeln!(
                receipt,
                "- Bridge fixture: easier {:.0}% · faithful {:.0}% · duplicate {:.0}% · {}",
                bridge.easier_than_parent * 100.0,
                bridge.faithful_to_concept * 100.0,
                bridge.duplicate_rate * 100.0,
                if bridge.passes { "pass" } else { "FAIL" },
            );
        }
        Err(error) => {
            let _ = writeln!(receipt, "- Bridge fixture: FAILED: {error}");
        }
    }
}

fn mean(scores: &[&SourceScore], value: impl Fn(&SourceScore) -> f64) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = scores.len() as f64;

    scores.iter().map(|score| value(score)).sum::<f64>() / count
}

fn bridge_quality_fixture<P>(provider: &P) -> Result<BridgeQualityScore, String>
where
    P: BridgeMaterialProvider + ?Sized,
{
    let request = BridgeMaterialRequest::new(
        "nato-cat-composition",
        "nato cat composition",
        ReviewUnitId::new("parent-nato-cat"),
        "Spell CAT over the phone using the NATO phonetic alphabet.",
        "CHARLIE ALFA TANGO",
        4,
        None,
        vec![ReviewPerformanceContext {
            review_unit_id: "parent-nato-cat".to_owned(),
            submitted_answer: "CHARLIE TANGO".to_owned(),
            verdict: Some("wrong".to_owned()),
        }],
        SourceAuthorizationContext::none(),
    );
    let material = provider
        .generate_bridge_material(&request)
        .map_err(|failure| failure.to_string())?;
    Ok(bridge_quality_judges(
        request.parent_stage_order,
        &request.concept_key,
        &[(
            request.concept_key.as_str(),
            request.parent_prompt.as_str(),
            request.parent_expected_answer.as_str(),
        )],
        &material.candidates,
    ))
}

fn bridge_quality_judges(
    parent_stage_order: u32,
    concept_key: &str,
    existing: &[(&str, &str, &str)],
    candidates: &[DraftCandidate],
) -> BridgeQualityScore {
    let drafts = candidates.len();
    let easier = candidates
        .iter()
        .filter(|candidate| {
            bench_stage_order(&candidate.activity_stage, &candidate.activity_kind)
                < parent_stage_order
        })
        .count();
    let concept = normalize(concept_key);
    let faithful = candidates
        .iter()
        .filter(|candidate| normalize(&candidate.concept).contains(&concept))
        .count();
    let existing_signatures = existing
        .iter()
        .map(|(concept, question, answer)| {
            [normalize(concept), normalize(question), normalize(answer)].join("\0")
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let duplicates = candidates
        .iter()
        .filter(|candidate| {
            let signature = [
                normalize(&candidate.concept),
                normalize(&candidate.question),
                normalize(&candidate.answer),
            ]
            .join("\0");
            existing_signatures.contains(&signature) || !seen.insert(signature)
        })
        .count();
    let easier_than_parent = fraction(easier, drafts);
    let faithful_to_concept = fraction(faithful, drafts);
    let duplicate_rate = fraction(duplicates, drafts);

    BridgeQualityScore {
        drafts,
        easier_than_parent,
        faithful_to_concept,
        duplicate_rate,
        passes: drafts > 0
            && (easier_than_parent - 1.0).abs() < f64::EPSILON
            && (faithful_to_concept - 1.0).abs() < f64::EPSILON
            && duplicate_rate.abs() < f64::EPSILON,
    }
}

fn bench_stage_order(stage: &str, activity_kind: &GeneratedLearningActivityKind) -> u32 {
    let normalized = stage.to_lowercase();
    if normalized.contains("bridge") && normalized.contains("recognition") {
        return 0;
    }
    if normalized.contains("bridge") && normalized.contains("cued") {
        return 1;
    }
    if normalized.contains("recognition") {
        return 1;
    }
    if normalized.contains("cued") {
        return 2;
    }
    if normalized.contains("free") {
        return 3;
    }
    if normalized.contains("composition") {
        return 4;
    }
    if activity_kind == &GeneratedLearningActivityKind::Exercise {
        5
    } else {
        1
    }
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * percent).div_ceil(100);

    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn format_cost(micros: Option<i64>) -> String {
    match micros {
        None => "—".to_owned(),
        #[allow(clippy::cast_precision_loss)]
        Some(micros) => format!("${:.4}", micros as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use memory_engine_generation::{
        BridgeMaterial, DraftRejection, ProviderDrafts, ProviderFailure, ProviderUsage,
        ReferenceNoteDraft, ReferenceNoteProvider, ReferenceNoteRequest,
    };
    use memory_engine_persistence::{GeneratedLearningActivityKind, GeneratedPromptModel};

    use super::*;

    fn candidate(question: &str, answer: &str, evidence: &str) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: question.to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: Some(evidence.to_owned()),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        }
    }

    fn mcq(question: &str, answer: &str, distractors: &[&str]) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: question.to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: None,
            distractors: distractors
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        }
    }

    fn expectations() -> Expectations {
        Expectations {
            min_drafts: 1,
            max_drafts: 3,
            key_terms: vec!["Alfa".to_owned(), "Bravo".to_owned()],
            intent: None,
            required_activity_kinds: Vec::new(),
            required_activity_stage_terms: Vec::new(),
            forbidden_activity_kinds: Vec::new(),
            requires_distractors: false,
            requires_variants: false,
            content_fit: None,
            enumerable_set: None,
        }
    }

    const BODY: &str = "A is Alfa. B is Bravo. C is Charlie.";
    const CORPUS_SOURCE_BODY: &str = "The source explains Alfa and Bravo as code words.";

    fn corpus_source() -> CorpusSource {
        CorpusSource {
            id: "letters".to_owned(),
            title: "Letters".to_owned(),
            category: "fixture".to_owned(),
            body: CORPUS_SOURCE_BODY.to_owned(),
            expect: expectations(),
        }
    }

    #[test]
    fn parse_args_accepts_max_drafts() {
        let args = [
            "--model".to_owned(),
            "google/gemini-3.5-flash".to_owned(),
            "--max-drafts".to_owned(),
            "4".to_owned(),
        ];

        let parsed = parse_args(&args).expect("args");

        assert_eq!(parsed.model.as_deref(), Some("google/gemini-3.5-flash"));
        assert_eq!(parsed.max_drafts, Some(4));
    }

    #[test]
    fn parse_args_rejects_zero_max_drafts() {
        let args = ["--max-drafts".to_owned(), "0".to_owned()];

        let error = parse_args(&args).expect_err("zero max drafts");

        assert!(error.contains("greater than zero"));
    }

    #[test]
    fn grounded_output_scores_perfect_provenance_and_coverage() {
        let candidates = vec![
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is B?", "Bravo", "B is Bravo."),
        ];

        let score = deterministic_judges(BODY, &expectations(), None, &candidates);

        assert!((score.provenance - 1.0).abs() < f64::EPSILON);
        assert!((score.answerability - 1.0).abs() < f64::EPSILON);
        assert!((score.key_term_coverage - 1.0).abs() < f64::EPSILON);
        assert!(score.duplicate_rate.abs() < f64::EPSILON);
        assert!(score.count_in_range);
    }

    #[test]
    fn distractor_cohesion_flags_mismatched_initials_on_letter_keyed_mcq() {
        // The dogfooded bug: a "letter C" question whose distractors do not start
        // with C, so the answer is identifiable by its initial alone.
        let bad = vec![mcq(
            "In the NATO phonetic alphabet, which code word represents the letter C?",
            "Charlie",
            &["Delta", "Bravo", "Echo"],
        )];
        assert!(
            distractor_cohesion(&bad).abs() < f64::EPSILON,
            "mismatched initials on a letter-keyed MCQ must score 0"
        );

        // Cohesive: every option begins with C, so the initial discriminates nothing.
        let good = vec![mcq(
            "In the NATO phonetic alphabet, which code word represents the letter C?",
            "Charlie",
            &["Cobra", "Caesar", "Casino"],
        )];
        assert!((distractor_cohesion(&good) - 1.0).abs() < f64::EPSILON);

        // A question that does not key on a letter is never penalized by this judge.
        let unkeyed = vec![mcq(
            "What is the capital of France?",
            "Paris",
            &["London", "Berlin"],
        )];
        assert!((distractor_cohesion(&unkeyed) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn self_referential_free_flags_meta_questions() {
        let meta = vec![mcq(
            "What is the name of the phonetic alphabet presented as the subject of the source text?",
            "NATO phonetic alphabet",
            &["ICAO spelling alphabet", "ITU phonetic alphabet"],
        )];
        assert!(
            self_referential_free(&meta).abs() < f64::EPSILON,
            "a question referencing the source artifact must score 0"
        );

        let standalone = vec![mcq(
            "In the NATO phonetic alphabet, what code word represents the letter A?",
            "Alfa",
            &["Apple", "Acorn"],
        )];
        assert!((self_referential_free(&standalone) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fabricated_evidence_and_duplicates_are_caught() {
        let candidates = vec![
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is D?", "Delta", "D is Delta."),
        ];

        let score = deterministic_judges(BODY, &expectations(), None, &candidates);

        assert!((score.provenance - 2.0 / 3.0).abs() < 0.01);
        assert!((score.duplicate_rate - 1.0 / 3.0).abs() < 0.01);
        assert!(score.count_in_range, "3 drafts sits inside [1, 3]");
    }

    #[test]
    fn score_source_uses_production_gate_to_filter_duplicates_before_scoring() {
        let score = score_source(&DuplicateDraftProvider, None, &corpus_source());

        assert_eq!(score.drafts, 1);
        assert!(score.duplicate_rate.abs() < f64::EPSILON);
        assert_eq!(score.rejected_drafts, 0);
        assert_eq!(score.validation_failures, 1);
        assert!((score.runtime_acceptance - 0.5).abs() < f64::EPSILON);

        let receipt = render_receipt(
            "fixture",
            &[score],
            &Ok(BridgeQualityScore {
                drafts: 0,
                easier_than_parent: 1.0,
                faithful_to_concept: 1.0,
                duplicate_rate: 0.0,
                passes: true,
            }),
            None,
        );
        assert!(
            receipt.contains("| source | category | accepted | rejected | failures | runtime |")
        );
        assert!(receipt.contains("| letters | fixture | 1 | 0 | 1 | 50% |"));
        assert!(
            receipt.contains("| N/A | N/A | N/A | N/A |"),
            "content-fit None must render explicitly as N/A: {receipt}"
        );
    }

    #[test]
    fn keep_rate_rigor_calls_a_small_delta_within_noise() {
        let mut receipt = String::new();
        let current = [
            ("a".to_owned(), 0.75),
            ("b".to_owned(), 0.50),
            ("c".to_owned(), 0.90),
        ];
        let baseline = [
            ("a".to_owned(), 0.80),
            ("b".to_owned(), 0.50),
            ("c".to_owned(), 0.71),
        ];
        render_keep_rate_rigor(&mut receipt, &current, Some(&baseline));
        assert!(
            receipt.contains("Keep rate (source-clustered, n=3)"),
            "missing keep-rate CI: {receipt}"
        );
        assert!(
            receipt.contains("within noise"),
            "a few points of movement must read as noise: {receipt}"
        );
        assert!(receipt.contains("Power:"), "missing power note: {receipt}");
    }

    #[test]
    fn keep_rate_rigor_calls_a_large_delta_detectable() {
        let mut receipt = String::new();
        let current = [
            ("a".to_owned(), 0.90),
            ("b".to_owned(), 0.92),
            ("c".to_owned(), 0.88),
        ];
        let baseline = [
            ("a".to_owned(), 0.40),
            ("b".to_owned(), 0.42),
            ("c".to_owned(), 0.38),
        ];
        render_keep_rate_rigor(&mut receipt, &current, Some(&baseline));
        assert!(
            receipt.contains("detectable"),
            "a large consistent gain must be detectable: {receipt}"
        );
    }

    #[test]
    fn score_source_includes_repaired_accepted_drafts_and_usage() {
        let provider = RepairingDraftProvider {
            repair_calls: Cell::new(0),
        };

        let score = score_source(&provider, None, &corpus_source());

        assert_eq!(score.drafts, 1);
        assert_eq!(provider.repair_calls.get(), 1);
        assert_eq!(score.input_tokens, 14);
        assert_eq!(score.output_tokens, 18);
        assert_eq!(score.cost_usd_micros, Some(24));
        assert_eq!(score.latency_ms, 30);
        assert_eq!(score.rejected_drafts, 1);
        assert_eq!(score.validation_failures, 0);
        assert!((score.runtime_acceptance - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn bridge_quality_scenario_requires_easier_faithful_non_duplicate_items() {
        let clean = bridge_quality_fixture(&FakeModelProvider).expect("fake bridge fixture");

        assert_eq!(clean.drafts, 2);
        assert!((clean.easier_than_parent - 1.0).abs() < f64::EPSILON);
        assert!((clean.faithful_to_concept - 1.0).abs() < f64::EPSILON);
        assert!(clean.duplicate_rate.abs() < f64::EPSILON);
        assert!(clean.passes);

        let duplicate_parent = DraftCandidate {
            index: 1,
            concept: "nato cat composition".to_owned(),
            question: "Spell CAT over the phone using the NATO phonetic alphabet.".to_owned(),
            answer: "CHARLIE ALFA TANGO".to_owned(),
            evidence: None,
            distractors: Vec::new(),
            worked_solution: Some("Use the NATO mapping.".to_owned()),
            activity_kind: GeneratedLearningActivityKind::Exercise,
            activity_stage: "composition".to_owned(),
            unsupported: false,
        };
        let failed = bridge_quality_judges(
            4,
            "nato-cat-composition",
            &[(
                "nato-cat-composition",
                "Spell CAT over the phone using the NATO phonetic alphabet.",
                "CHARLIE ALFA TANGO",
            )],
            &[duplicate_parent],
        );

        assert!(!failed.passes);
        assert!((failed.duplicate_rate - 1.0).abs() < f64::EPSILON);
        assert!(failed.easier_than_parent.abs() < f64::EPSILON);
    }

    #[test]
    fn variant_quality_requires_distinct_same_concept_stage_phrasings_without_answer_leakage() {
        let mut expect = expectations();
        expect.requires_variants = true;
        let variants = vec![
            nato_letter_a_variant("What is the NATO phonetic alphabet word for A?"),
            nato_letter_a_variant("Choose the code word used for the letter A."),
        ];

        let clean = deterministic_judges(BODY, &expect, None, &variants);

        assert!((clean.variant_quality - 1.0).abs() < f64::EPSILON);
        assert!(clean.intent_shape_match);

        let short_answer_clean = vec![
            alphabet_a_variant("What name marks the first NATO letter?"),
            alphabet_a_variant("Choose the initial alphabet symbol."),
        ];
        let short_answer = deterministic_judges(BODY, &expect, None, &short_answer_clean);

        assert!((short_answer.variant_quality - 1.0).abs() < f64::EPSILON);
        assert!(short_answer.intent_shape_match);

        assert_variant_quality_fails(
            &expect,
            &[
                variants[0].clone(),
                nato_letter_a_variant("Which option is ALFA for the letter A?"),
            ],
        );
        assert_variant_quality_fails(
            &expect,
            &[
                variants[0].clone(),
                nato_letter_a_variant("What is the NATO phonetic alphabet word for A?"),
            ],
        );
        assert_variant_quality_fails(
            &expect,
            &[
                variants[0].clone(),
                nato_letter_a_variant("What is the NATO phonetic alphabet word for the letter A?"),
            ],
        );
    }

    fn assert_variant_quality_fails(expect: &Expectations, variants: &[DraftCandidate]) {
        let failed = deterministic_judges(BODY, expect, None, variants);
        assert!(failed.variant_quality < 1.0);
        assert!(!failed.intent_shape_match);
    }

    fn nato_letter_a_variant(question: &str) -> DraftCandidate {
        DraftCandidate {
            concept: "NATO letter A".to_owned(),
            question: question.to_owned(),
            answer: "ALFA".to_owned(),
            distractors: vec!["BRAVO".to_owned(), "CHARLIE".to_owned()],
            activity_stage: "recognition-3".to_owned(),
            ..candidate(
                question,
                "ALFA",
                "The NATO phonetic alphabet word for A is ALFA",
            )
        }
    }

    fn alphabet_a_variant(question: &str) -> DraftCandidate {
        DraftCandidate {
            concept: "Alphabet first letter".to_owned(),
            question: question.to_owned(),
            answer: "A".to_owned(),
            activity_stage: "recognition-3".to_owned(),
            ..candidate(question, "A", "A is the first letter of the alphabet")
        }
    }

    #[test]
    fn bridge_quality_fixture_scores_the_selected_provider() {
        let score =
            bridge_quality_fixture(&DuplicateBridgeProvider).expect("duplicate bridge fixture");

        assert_eq!(score.drafts, 1);
        assert!(!score.passes);
        assert!((score.duplicate_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn provenance_matching_tolerates_case_and_punctuation() {
        let candidates = vec![candidate("What is C?", "Charlie", "c IS charlie")];

        let score = deterministic_judges(BODY, &expectations(), None, &candidates);

        assert!((score.provenance - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn paraphrased_answers_score_low_on_answerability() {
        let candidates = vec![candidate(
            "What does B stand for?",
            "Bravissimo encore fortissimo",
            "B is Bravo",
        )];

        let score = deterministic_judges(BODY, &expectations(), None, &candidates);

        assert!(score.answerability.abs() < f64::EPSILON);
    }

    #[test]
    fn count_in_range_respects_annotations() {
        let none = deterministic_judges(BODY, &expectations(), None, &[]);
        assert!(!none.count_in_range);
        assert!(none.provenance.abs() < f64::EPSILON);
    }

    #[test]
    fn percentile_picks_nearest_rank() {
        assert_eq!(percentile(&[10, 20, 30, 40], 50), 20);
        assert_eq!(percentile(&[10, 20, 30, 40], 95), 40);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn corpus_loads_and_fake_provider_scores_clean() {
        let corpus = load_corpus().expect("corpus");
        assert!(corpus.len() >= 10, "047 requires ≥10 sources");

        for source in &corpus {
            let score = score_source(&FakeModelProvider, None, source);
            assert!(score.provider_error.is_none());
            assert!(
                (score.provenance - 1.0).abs() < f64::EPSILON,
                "fake provider quotes verbatim; {} scored {}",
                source.id,
                score.provenance
            );
        }
    }

    #[test]
    fn presidents_fixture_is_exhaustively_supported_by_the_fake_provider() {
        let source = load_corpus()
            .expect("corpus")
            .into_iter()
            .find(|source| source.id == "us-presidents-ordinal")
            .expect("presidents fixture");

        let score = score_source(&FakeModelProvider, None, &source);
        let enumerable = score.enumerable_set.expect("enumerable score");

        assert_eq!(enumerable.expected, 47);
        assert_eq!(enumerable.observed, 47);
        assert_eq!(enumerable.covered, 47);
        assert!(
            enumerable.passes(),
            "the canonical fixture must pass: {enumerable:?}"
        );
    }

    #[test]
    fn intent_eval_fixtures_assert_different_item_shapes() {
        let corpus = load_corpus().expect("corpus");
        let intent_sources = corpus
            .iter()
            .filter(|source| source.expect.intent.is_some())
            .collect::<Vec<_>>();
        assert!(
            intent_sources.len() >= 4,
            "051 requires one eval fixture per capture intent"
        );
        let intent_labels = intent_sources
            .iter()
            .filter_map(|source| source.expect.intent.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            intent_labels,
            [
                "concept_understanding",
                "fact_recall",
                "enumerable_set",
                "procedure_process",
                "verbatim_memorization",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
        );

        let mut signatures = std::collections::BTreeSet::new();
        for source in intent_sources {
            let document = SourceDocument {
                id: source.id.clone(),
                kind: SourceDocumentKind::Text,
                title: source.title.clone(),
                project_key: None,
                body: Some(source.body.clone()),
                uri: None,
                permission: SourcePermission::ModelEligible,
                freshness: Some(NOW),
                ttl_expires_at: None,
                created_at: NOW,
                archived_at: None,
            };
            let drafts = FakeModelProvider
                .generate_drafts(&document)
                .expect("fake provider");
            assert!(
                intent_shape_matches(&source.expect, drafts.learning_intent, &drafts.candidates),
                "{} should satisfy its intent shape expectation",
                source.id
            );
            signatures.insert(shape_signature(&drafts.candidates));
        }

        assert!(
            signatures.len() >= 4,
            "intent fixtures should not collapse into one generic item shape: {signatures:?}"
        );
    }

    struct DuplicateBridgeProvider;

    struct DuplicateDraftProvider;

    impl DraftProvider for DuplicateDraftProvider {
        fn model(&self) -> GeneratedPromptModel {
            model("duplicate-drafts")
        }

        fn generate_drafts(
            &self,
            source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            let evidence = source.body.clone().unwrap_or_default();
            Ok(ProviderDrafts {
                model: self.model(),
                learning_intent: None,
                candidates: vec![
                    candidate("What is A?", "Alfa", &evidence),
                    candidate("What is A?", "Alfa", &evidence),
                ],
                failures: Vec::new(),
                usage: None,
            })
        }
    }

    struct RepairingDraftProvider {
        repair_calls: Cell<u32>,
    }

    impl DraftProvider for RepairingDraftProvider {
        fn model(&self) -> GeneratedPromptModel {
            model("repairing-drafts")
        }

        fn generate_drafts(
            &self,
            source: &SourceDocument,
        ) -> Result<ProviderDrafts, ProviderFailure> {
            let evidence = source.body.clone().unwrap_or_default();
            let mut rejected = candidate("What is A?", "Alfa", "A is Alfa.");
            rejected.evidence = Some(evidence.clone());
            rejected.unsupported = true;

            Ok(ProviderDrafts {
                model: self.model(),
                learning_intent: None,
                candidates: vec![rejected],
                failures: Vec::new(),
                usage: Some(ProviderUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                    cost_usd_micros: Some(7),
                    latency_ms: 10,
                }),
            })
        }

        fn repair_drafts(
            &self,
            source: &SourceDocument,
            rejections: &[DraftRejection],
        ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
            assert_eq!(rejections.len(), 1);
            self.repair_calls.set(self.repair_calls.get() + 1);
            let evidence = source.body.clone().unwrap_or_default();
            let mut repaired = candidate("What is B?", "Bravo", &evidence);
            repaired.index = 2;

            Ok(Some(ProviderDrafts {
                model: self.model(),
                learning_intent: None,
                candidates: vec![repaired],
                failures: Vec::new(),
                usage: Some(ProviderUsage {
                    input_tokens: 11,
                    output_tokens: 13,
                    cost_usd_micros: Some(17),
                    latency_ms: 20,
                }),
            }))
        }
    }

    fn model(name: &str) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: name.to_owned(),
            version: "v1".to_owned(),
        }
    }

    impl ReferenceNoteProvider for DuplicateBridgeProvider {
        fn model(&self) -> GeneratedPromptModel {
            GeneratedPromptModel {
                provider: "fixture".to_owned(),
                name: "duplicate-bridge".to_owned(),
                version: "v1".to_owned(),
            }
        }

        fn explain_concept(
            &self,
            _request: &ReferenceNoteRequest,
        ) -> Result<ReferenceNoteDraft, ProviderFailure> {
            Ok(ReferenceNoteDraft {
                title: "Duplicate bridge".to_owned(),
                body: "CAT is CHARLIE ALFA TANGO.".to_owned(),
            })
        }
    }

    impl BridgeMaterialProvider for DuplicateBridgeProvider {
        fn generate_bridge_material(
            &self,
            request: &BridgeMaterialRequest,
        ) -> Result<BridgeMaterial, ProviderFailure> {
            Ok(BridgeMaterial {
                model: self.model(),
                reference_note: self.explain_concept(&ReferenceNoteRequest::new(
                    request.concept_key.clone(),
                    request.concept_label.clone(),
                    request.parent_prompt.clone(),
                    request.parent_expected_answer.clone(),
                    request.recent_performance.clone(),
                    request.authorization().clone(),
                ))?,
                candidates: vec![DraftCandidate {
                    index: 1,
                    concept: request.concept_label.clone(),
                    question: request.parent_prompt.clone(),
                    answer: request.parent_expected_answer.clone(),
                    evidence: None,
                    distractors: Vec::new(),
                    worked_solution: Some("Duplicate of the parent.".to_owned()),
                    activity_kind: GeneratedLearningActivityKind::Exercise,
                    activity_stage: "composition".to_owned(),
                    unsupported: false,
                }],
                usage: None,
            })
        }
    }
}
