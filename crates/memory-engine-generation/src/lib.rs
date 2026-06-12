//! Beta content generation behind a provider boundary.
//!
//! This crate owns the [`DraftProvider`] boundary, deterministic providers
//! (structured-block parsing and a CI-safe fake model), draft validation,
//! cited reference creation, and generation-run bookkeeping. Model-backed
//! providers implement [`DraftProvider`] from their own boundary crates; this
//! crate never talks to a network.
//!
//! Every provider's output passes the same trust gate before persistence:
//! drafts must carry evidence quoting the source (verified by normalized
//! substring match), duplicates are rejected, and exercises require worked
//! solutions.

mod provider;

use std::{collections::BTreeSet, error::Error, fmt};

pub use provider::{
    DraftCandidate, DraftProvider, FakeModelProvider, FallbackProvider, ProviderDrafts,
    ProviderFailure, ProviderUsage, StructuredBlockProvider,
};

use memory_engine_core::{ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, ReviewUnitId};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaStoreError, BetaStoreSnapshot, GeneratedLearningActivityKind,
    GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
    GeneratedPromptValidationStatus, GenerationRun, GenerationRunUsage, PersistedQueueCandidate,
    ReferenceSpan, SourceDocument, SourceDocumentKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BetaGenerationRequest {
    pub run_id: String,
    pub source_document_ids: Vec<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub default_due: i64,
    pub model: Option<GeneratedPromptModel>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BetaGenerationResult {
    pub run_id: String,
    pub draft_ids: Vec<String>,
    pub accepted_draft_ids: Vec<String>,
    pub rejected_draft_ids: Vec<String>,
    pub validation_failures: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BetaGenerationError<E = BetaStoreError> {
    Store(E),
    UnknownSourceDocument(String),
    SourceDocumentHasNoTextBody(String),
}

impl<E> fmt::Display for BetaGenerationError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::UnknownSourceDocument(id) => write!(formatter, "Unknown source document: {id}"),
            Self::SourceDocumentHasNoTextBody(id) => {
                write!(formatter, "Source document has no text body: {id}")
            }
        }
    }
}

impl<E> Error for BetaGenerationError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::UnknownSourceDocument(_) | Self::SourceDocumentHasNoTextBody(_) => None,
        }
    }
}

impl From<BetaStoreError> for BetaGenerationError<BetaStoreError> {
    fn from(error: BetaStoreError) -> Self {
        Self::Store(error)
    }
}

pub trait BetaGenerationStore {
    type Error;

    /// Read the current beta-store snapshot used to select source inputs and
    /// detect duplicate generated drafts.
    ///
    /// # Errors
    ///
    /// Returns the store error when snapshot reconstruction fails.
    fn snapshot(&self) -> Result<BetaStoreSnapshot, Self::Error>;

    /// Save a generation run receipt.
    ///
    /// # Errors
    ///
    /// Returns the store error when the run is rejected or cannot be persisted.
    fn save_generation_run(&mut self, run: GenerationRun) -> Result<GenerationRun, Self::Error>;

    /// Save cited source evidence for a generated draft.
    ///
    /// # Errors
    ///
    /// Returns the store error when the reference is rejected or cannot be
    /// persisted.
    fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, Self::Error>;

    /// Save a generated prompt draft.
    ///
    /// # Errors
    ///
    /// Returns the store error when the draft is rejected or cannot be persisted.
    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error>;
}

impl BetaGenerationStore for BetaPersistenceStore {
    type Error = BetaStoreError;

    fn snapshot(&self) -> Result<BetaStoreSnapshot, Self::Error> {
        Ok(BetaPersistenceStore::snapshot(self))
    }

    fn save_generation_run(&mut self, run: GenerationRun) -> Result<GenerationRun, Self::Error> {
        BetaPersistenceStore::save_generation_run(self, run)
    }

    fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, Self::Error> {
        BetaPersistenceStore::save_reference_span(self, reference)
    }

    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error> {
        BetaPersistenceStore::save_generated_prompt_draft(self, draft)
    }
}

/// Generate deterministic beta drafts from structured source blocks.
///
/// Equivalent to [`run_beta_generation_with_provider`] with
/// [`StructuredBlockProvider`]; kept as the zero-configuration entry point.
///
/// # Errors
///
/// Returns [`BetaGenerationError`] when requested source documents are missing,
/// have no text body, or when the beta store rejects a generated entity.
pub fn run_beta_generation<S>(
    store: &mut S,
    request: BetaGenerationRequest,
) -> Result<BetaGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    run_beta_generation_with_provider(store, &StructuredBlockProvider, request)
}

/// Generate beta drafts from the given provider's candidates.
///
/// Provider transport failures do not abort the run: they are recorded as
/// human-readable validation failures on the persisted run so the study UI
/// can explain zero-draft outcomes.
///
/// # Errors
///
/// Returns [`BetaGenerationError`] when requested source documents are missing,
/// have no text body, or when the beta store rejects a generated entity.
pub fn run_beta_generation_with_provider<S>(
    store: &mut S,
    provider: &dyn DraftProvider,
    request: BetaGenerationRequest,
) -> Result<BetaGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let model = request.model.clone().unwrap_or_else(|| provider.model());
    let snapshot = store.snapshot().map_err(BetaGenerationError::Store)?;
    let sources = request
        .source_document_ids
        .iter()
        .map(|source_document_id| {
            require_source::<S::Error>(&snapshot.source_documents, source_document_id)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut validation_failures = Vec::new();
    let mut draft_ids = Vec::new();
    let mut accepted_draft_ids = Vec::new();
    let mut rejected_draft_ids = Vec::new();
    let mut usage: Option<GenerationRunUsage> = None;
    // Tracks the distinct models that actually produced drafts so the run
    // header is honest: one producer names it, several (a composite running
    // different sub-providers across sources) fall back to the declared model
    // rather than last-write-wins. Per-draft model stamping stays exact.
    let mut producing_models: Vec<GeneratedPromptModel> = Vec::new();

    store
        .save_generation_run(run_receipt(&request, &model, RunProgress::Started))
        .map_err(BetaGenerationError::Store)?;

    let mut seen_signatures = existing_draft_signatures(&snapshot.generated_prompt_drafts);
    for source in &sources {
        let drafts = match provider.generate_drafts(source) {
            Ok(drafts) => drafts,
            Err(failure) => {
                validation_failures.push(format!("{}: {failure}", source.id));
                continue;
            }
        };
        validation_failures.extend(drafts.failures);
        usage = merge_usage(usage, drafts.usage);
        // Stamp drafts with the provider that actually generated them, not the
        // composite's declared identity. A caller override still wins.
        let source_model = request.model.clone().unwrap_or(drafts.model);
        if !drafts.candidates.is_empty() && !producing_models.contains(&source_model) {
            producing_models.push(source_model.clone());
        }

        for candidate in drafts.candidates {
            let Some(evidence) = candidate
                .evidence
                .as_deref()
                .filter(|quote| !normalize_for_match(quote).is_empty())
            else {
                validation_failures.push(format!(
                    "{} block {}: generated drafts require source provenance",
                    source.id, candidate.index
                ));
                continue;
            };
            let signature = candidate_signature(&candidate);
            let duplicate = seen_signatures.contains(&signature);
            seen_signatures.insert(signature);

            let draft = persist_candidate(
                store,
                source,
                &candidate,
                evidence,
                &PersistParams {
                    request: &request,
                    model: &source_model,
                    duplicate,
                    quote_verified: source_contains_quote(source, evidence),
                },
            )?;

            draft_ids.push(draft.id.clone());
            if draft.validation.status == GeneratedPromptValidationStatus::Accepted {
                accepted_draft_ids.push(draft.id);
            } else {
                rejected_draft_ids.push(draft.id);
            }
        }
    }

    let run_model = match producing_models.as_slice() {
        [single] => single.clone(),
        _ => model.clone(),
    };
    store
        .save_generation_run(run_receipt(
            &request,
            &run_model,
            RunProgress::Completed {
                draft_ids: draft_ids.clone(),
                validation_failures: validation_failures.clone(),
                usage,
            },
        ))
        .map_err(BetaGenerationError::Store)?;

    Ok(BetaGenerationResult {
        run_id: request.run_id,
        draft_ids,
        accepted_draft_ids,
        rejected_draft_ids,
        validation_failures,
    })
}

struct PersistParams<'a> {
    request: &'a BetaGenerationRequest,
    model: &'a GeneratedPromptModel,
    duplicate: bool,
    quote_verified: bool,
}

/// Save one candidate's reference span and draft, returning the stored draft.
fn persist_candidate<S>(
    store: &mut S,
    source: &SourceDocument,
    candidate: &DraftCandidate,
    evidence: &str,
    params: &PersistParams<'_>,
) -> Result<GeneratedPromptDraft, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let reference_span = store
        .save_reference_span(ReferenceSpan {
            id: generated_id(&params.request.run_id, "ref", &source.id, candidate),
            source_document_id: source.id.clone(),
            label: format!("{} source evidence", candidate.concept),
            text: evidence.to_owned(),
            locator: format!("block:{}", candidate.index),
            created_at: params.request.started_at,
        })
        .map_err(BetaGenerationError::Store)?;

    store
        .save_generated_prompt_draft(build_draft(
            source,
            candidate,
            &DraftContext {
                run_id: &params.request.run_id,
                reference_span_id: &reference_span.id,
                model: params.model,
                due: params.request.default_due,
                created_at: params.request.started_at,
                duplicate: params.duplicate,
                quote_verified: params.quote_verified,
            },
        ))
        .map_err(BetaGenerationError::Store)
}

enum RunProgress {
    Started,
    Completed {
        draft_ids: Vec<String>,
        validation_failures: Vec<String>,
        usage: Option<GenerationRunUsage>,
    },
}

fn run_receipt(
    request: &BetaGenerationRequest,
    model: &GeneratedPromptModel,
    progress: RunProgress,
) -> GenerationRun {
    let (draft_ids, completed_at, validation_failures, usage) = match progress {
        RunProgress::Started => (Vec::new(), None, Vec::new(), None),
        RunProgress::Completed {
            draft_ids,
            validation_failures,
            usage,
        } => (
            draft_ids,
            Some(request.completed_at.unwrap_or(request.started_at)),
            validation_failures,
            usage,
        ),
    };

    GenerationRun {
        id: request.run_id.clone(),
        source_document_ids: request.source_document_ids.clone(),
        draft_ids,
        provider: model.provider.clone(),
        model: model.name.clone(),
        started_at: request.started_at,
        completed_at,
        validation_failures,
        usage,
    }
}

struct DraftContext<'a> {
    run_id: &'a str,
    reference_span_id: &'a str,
    model: &'a GeneratedPromptModel,
    due: i64,
    created_at: i64,
    duplicate: bool,
    quote_verified: bool,
}

fn merge_usage(
    total: Option<GenerationRunUsage>,
    addition: Option<ProviderUsage>,
) -> Option<GenerationRunUsage> {
    let Some(addition) = addition else {
        return total;
    };
    let total = total.unwrap_or(GenerationRunUsage {
        input_tokens: 0,
        output_tokens: 0,
        cost_usd_micros: None,
        latency_ms: 0,
    });

    Some(GenerationRunUsage {
        input_tokens: total.input_tokens + addition.input_tokens,
        output_tokens: total.output_tokens + addition.output_tokens,
        cost_usd_micros: match (total.cost_usd_micros, addition.cost_usd_micros) {
            (None, None) => None,
            (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
        },
        latency_ms: total.latency_ms + addition.latency_ms,
    })
}

fn source_contains_quote(source: &SourceDocument, quote: &str) -> bool {
    evidence_quote_matches(source.body.as_deref().unwrap_or_default(), quote)
}

/// Minimum number of words an evidence quote must carry to count as proof.
///
/// A single-word quote like "the" substring-matches almost any prose, so it
/// proves nothing about a fabricated answer; requiring a short phrase closes
/// that hole without rejecting legitimately terse citations such as
/// "100 degrees Celsius".
const MIN_EVIDENCE_WORDS: usize = 3;

/// Whether an evidence quote is substantive and appears in the source text
/// after folding case, punctuation, and whitespace.
///
/// Cheap models paraphrase formatting even when quoting faithfully, so exact
/// matching would reject grounded drafts — hence the normalization. A quote
/// shorter than the minimum evidence-word threshold is rejected outright: it would
/// substring-match trivially and cannot support an answer. This is the same
/// predicate the generation trust gate applies, exported so eval judges score
/// exactly what production enforces.
#[must_use]
pub fn evidence_quote_matches(source_text: &str, quote: &str) -> bool {
    let quote = normalize_for_match(quote);
    if quote.split(' ').filter(|word| !word.is_empty()).count() < MIN_EVIDENCE_WORDS {
        return false;
    }

    normalize_for_match(source_text).contains(&quote)
}

fn normalize_for_match(text: &str) -> String {
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

fn require_source<E>(
    sources: &[SourceDocument],
    source_document_id: &str,
) -> Result<SourceDocument, BetaGenerationError<E>> {
    let source = sources
        .iter()
        .find(|candidate| candidate.id == source_document_id)
        .cloned()
        .ok_or_else(|| BetaGenerationError::UnknownSourceDocument(source_document_id.to_owned()))?;
    if source
        .body
        .as_deref()
        .is_none_or(|body| body.trim().is_empty())
    {
        return Err(BetaGenerationError::SourceDocumentHasNoTextBody(
            source_document_id.to_owned(),
        ));
    }

    Ok(source)
}

fn build_draft(
    source: &SourceDocument,
    candidate: &DraftCandidate,
    context: &DraftContext<'_>,
) -> GeneratedPromptDraft {
    let unit_id = ReviewUnitId::new(generated_id(
        "generated",
        activity_kind_slug(&candidate.activity_kind),
        &source.id,
        candidate,
    ));
    let reasons = validation_reasons(candidate, context.duplicate, context.quote_verified);
    let status = if reasons.is_empty() {
        GeneratedPromptValidationStatus::Accepted
    } else {
        GeneratedPromptValidationStatus::Rejected
    };
    let critique_notes = if status == GeneratedPromptValidationStatus::Accepted {
        vec![format!("Grounded in {}.", context.reference_span_id)]
    } else {
        reasons
            .iter()
            .map(|reason| format!("Rejected: {reason}"))
            .collect()
    };

    GeneratedPromptDraft {
        id: generated_id(context.run_id, "draft", &source.id, candidate),
        source_document_ids: vec![source.id.clone()],
        reference_span_ids: vec![context.reference_span_id.to_owned()],
        generation_run_id: Some(context.run_id.to_owned()),
        review_unit_id: unit_id.clone(),
        prompt_id: format!("{unit_id}-prompt"),
        prompt: build_prompt(candidate, &unit_id),
        queue: PersistedQueueCandidate {
            review_unit_id: unit_id.clone(),
            due: context.due,
            progression: Some(ProgressionMetadata {
                progression_group: Some(slug(&candidate.concept)),
                stage_order: stage_order(&candidate.activity_stage, &candidate.activity_kind),
                requires: Vec::new(),
                supersedes: Vec::new(),
            }),
            concept_key: Some(slug(&candidate.concept)),
            source_key: Some(source.id.clone()),
            domain_key: Some(source_kind_key(&source.kind).to_owned()),
        },
        activity_kind: candidate.activity_kind.clone(),
        activity_stage: candidate.activity_stage.clone(),
        worked_solution: candidate.worked_solution.clone(),
        model: context.model.clone(),
        validation: GeneratedPromptValidation { status, reasons },
        critique_notes,
        created_at: context.created_at,
    }
}

fn build_prompt(candidate: &DraftCandidate, review_unit_id: &ReviewUnitId) -> Prompt {
    if candidate.activity_kind == GeneratedLearningActivityKind::Quiz
        && candidate.distractors.len() >= 2
    {
        return Prompt::Mcq {
            review_unit_id: review_unit_id.clone(),
            prompt: candidate.question.clone(),
            choices: unique(
                std::iter::once(candidate.answer.clone())
                    .chain(candidate.distractors.clone())
                    .collect(),
            ),
            correct_choice: candidate.answer.clone(),
        };
    }

    Prompt::Exact(ExactPrompt {
        kind: if candidate.activity_kind == GeneratedLearningActivityKind::Exercise {
            ExactPromptKind::Recitation
        } else {
            ExactPromptKind::ShortAnswer
        },
        review_unit_id: review_unit_id.clone(),
        prompt: candidate.question.clone(),
        accepted_answers: vec![candidate.answer.clone()],
        equivalence_groups: Vec::new(),
        ignored_tokens: vec![
            ".".to_owned(),
            ",".to_owned(),
            ";".to_owned(),
            ":".to_owned(),
        ],
    })
}

fn validation_reasons(
    candidate: &DraftCandidate,
    duplicate: bool,
    quote_verified: bool,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.unsupported {
        reasons.push("Unsupported by cited source material".to_owned());
    }
    if !quote_verified {
        reasons.push("Evidence quote not found in cited source".to_owned());
    }
    if duplicate {
        reasons.push("Duplicate-ish generated draft".to_owned());
    }
    if candidate.activity_kind == GeneratedLearningActivityKind::Exercise
        && candidate.worked_solution.is_none()
    {
        reasons.push("Exercises require a worked solution".to_owned());
    }

    reasons
}

fn existing_draft_signatures(drafts: &[GeneratedPromptDraft]) -> BTreeSet<String> {
    drafts
        .iter()
        .map(|draft| {
            [
                draft.queue.concept_key.clone().unwrap_or_default(),
                prompt_text(&draft.prompt).to_lowercase(),
                expected_answer(&draft.prompt).to_lowercase(),
            ]
            .join("\0")
        })
        .collect()
}

fn candidate_signature(candidate: &DraftCandidate) -> String {
    [
        slug(&candidate.concept),
        candidate.question.to_lowercase(),
        candidate.answer.to_lowercase(),
    ]
    .join("\0")
}

fn prompt_text(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Mcq { prompt, .. }
        | Prompt::Boolean { prompt, .. }
        | Prompt::Exact(ExactPrompt { prompt, .. }) => prompt.clone(),
    }
}

fn expected_answer(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Mcq { correct_choice, .. } => correct_choice.clone(),
        Prompt::Boolean { correct_answer, .. } => correct_answer.to_string(),
        Prompt::Exact(prompt) => prompt.accepted_answers.first().cloned().unwrap_or_default(),
    }
}

fn stage_order(stage: &str, activity_kind: &GeneratedLearningActivityKind) -> u32 {
    let normalized = stage.to_lowercase();
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

fn generated_id(prefix: &str, kind: &str, source_id: &str, candidate: &DraftCandidate) -> String {
    [
        prefix.to_owned(),
        kind.to_owned(),
        slug(source_id),
        candidate.index.to_string(),
        slug(&candidate.concept),
    ]
    .join("-")
}

fn slug(value: &str) -> String {
    let mut slugged = String::new();
    let mut previous_dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slugged.push(character);
            previous_dash = false;
        } else if !previous_dash && !slugged.is_empty() {
            slugged.push('-');
            previous_dash = true;
        }
    }
    while slugged.ends_with('-') {
        slugged.pop();
    }
    if slugged.is_empty() {
        "generated".to_owned()
    } else {
        slugged
    }
}

fn unique(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        if seen.insert(value.to_lowercase()) {
            result.push(value);
        }
    }
    result
}

fn activity_kind_slug(kind: &GeneratedLearningActivityKind) -> &'static str {
    match kind {
        GeneratedLearningActivityKind::Quiz => "quiz",
        GeneratedLearningActivityKind::Exercise => "exercise",
    }
}

fn source_kind_key(kind: &SourceDocumentKind) -> &'static str {
    match kind {
        SourceDocumentKind::Text => "text",
        SourceDocumentKind::Link => "link",
        SourceDocumentKind::File => "file",
        SourceDocumentKind::Image => "image",
        SourceDocumentKind::VideoTranscript => "video-transcript",
    }
}

#[cfg(test)]
mod tests {
    use super::evidence_quote_matches;

    const SOURCE: &str = "Photosynthesis occurs in the chloroplast and converts \
                          light into chemical energy stored as glucose.";

    #[test]
    fn substantive_verbatim_quote_matches() {
        assert!(evidence_quote_matches(SOURCE, "occurs in the chloroplast"));
    }

    #[test]
    fn match_tolerates_case_and_punctuation() {
        assert!(evidence_quote_matches(
            SOURCE,
            "OCCURS, in the! Chloroplast"
        ));
    }

    #[test]
    fn trivial_one_or_two_word_quote_is_rejected() {
        // The fabrication B1 closes: a real source word that proves nothing.
        assert!(!evidence_quote_matches(SOURCE, "the"));
        assert!(!evidence_quote_matches(SOURCE, "in the"));
    }

    #[test]
    fn three_word_quote_is_the_floor() {
        assert!(evidence_quote_matches(SOURCE, "into chemical energy"));
    }

    #[test]
    fn empty_quote_is_rejected() {
        assert!(!evidence_quote_matches(SOURCE, ""));
        assert!(!evidence_quote_matches(SOURCE, "   "));
    }

    #[test]
    fn substantive_quote_absent_from_source_is_rejected() {
        assert!(!evidence_quote_matches(
            SOURCE,
            "stored as fructose molecules"
        ));
    }
}
