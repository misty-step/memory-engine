//! Deterministic beta content generation.
//!
//! This crate owns source-block parsing, draft validation, cited reference
//! creation, and generation-run bookkeeping for the repo-local beta workflow.
//! Model clients remain outside this crate until repeated beta pressure proves
//! a provider-neutral boundary.

use std::{collections::BTreeSet, error::Error, fmt};

use memory_engine_core::{ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, ReviewUnitId};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaStoreError, BetaStoreSnapshot, GeneratedLearningActivityKind,
    GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
    GeneratedPromptValidationStatus, GenerationRun, PersistedQueueCandidate, ReferenceSpan,
    SourceDocument, SourceDocumentKind,
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

/// Generate deterministic beta drafts from source blocks.
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
    let model = request.model.unwrap_or_else(default_model);
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

    store
        .save_generation_run(GenerationRun {
            id: request.run_id.clone(),
            source_document_ids: request.source_document_ids.clone(),
            draft_ids: Vec::new(),
            provider: model.provider.clone(),
            model: model.name.clone(),
            started_at: request.started_at,
            completed_at: None,
            validation_failures: Vec::new(),
        })
        .map_err(BetaGenerationError::Store)?;

    let mut seen_signatures = existing_draft_signatures(&snapshot.generated_prompt_drafts);
    for source in &sources {
        let candidates = parse_source_document(source, &mut validation_failures);
        for candidate in candidates {
            let Some(reference) = candidate.reference.as_ref() else {
                validation_failures.push(format!(
                    "{} block {}: generated drafts require source provenance",
                    source.id, candidate.block_index
                ));
                continue;
            };

            let signature = candidate_signature(&candidate);
            let duplicate = seen_signatures.contains(&signature);
            seen_signatures.insert(signature);

            let reference_span = store
                .save_reference_span(ReferenceSpan {
                    id: generated_id(&request.run_id, "ref", &candidate),
                    source_document_id: source.id.clone(),
                    label: format!("{} source evidence", candidate.concept),
                    text: reference.clone(),
                    locator: format!("block:{}", candidate.block_index),
                    created_at: request.started_at,
                })
                .map_err(BetaGenerationError::Store)?;
            let draft = store
                .save_generated_prompt_draft(build_draft(
                    &candidate,
                    &DraftContext {
                        run_id: &request.run_id,
                        reference_span_id: &reference_span.id,
                        model: &model,
                        due: request.default_due,
                        created_at: request.started_at,
                        duplicate,
                    },
                ))
                .map_err(BetaGenerationError::Store)?;

            draft_ids.push(draft.id.clone());
            if draft.validation.status == GeneratedPromptValidationStatus::Accepted {
                accepted_draft_ids.push(draft.id);
            } else {
                rejected_draft_ids.push(draft.id);
            }
        }
    }

    store
        .save_generation_run(GenerationRun {
            id: request.run_id.clone(),
            source_document_ids: request.source_document_ids,
            draft_ids: draft_ids.clone(),
            provider: model.provider,
            model: model.name,
            started_at: request.started_at,
            completed_at: Some(request.completed_at.unwrap_or(request.started_at)),
            validation_failures: validation_failures.clone(),
        })
        .map_err(BetaGenerationError::Store)?;

    Ok(BetaGenerationResult {
        run_id: request.run_id,
        draft_ids,
        accepted_draft_ids,
        rejected_draft_ids,
        validation_failures,
    })
}

#[derive(Clone, Debug)]
struct ParsedCandidate {
    source: SourceDocument,
    block_index: usize,
    activity_kind: GeneratedLearningActivityKind,
    activity_stage: String,
    concept: String,
    question: String,
    answer: String,
    reference: Option<String>,
    distractors: Vec<String>,
    worked_solution: Option<String>,
    unsupported: bool,
}

struct DraftContext<'a> {
    run_id: &'a str,
    reference_span_id: &'a str,
    model: &'a GeneratedPromptModel,
    due: i64,
    created_at: i64,
    duplicate: bool,
}

fn default_model() -> GeneratedPromptModel {
    GeneratedPromptModel {
        provider: "fixture".to_owned(),
        name: "deterministic-beta-generator".to_owned(),
        version: "v1".to_owned(),
    }
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

fn parse_source_document(
    source: &SourceDocument,
    validation_failures: &mut Vec<String>,
) -> Vec<ParsedCandidate> {
    source
        .body
        .as_deref()
        .unwrap_or_default()
        .split("\n\n")
        .enumerate()
        .filter_map(|(index, block)| {
            parse_candidate_block(source, block, index + 1, validation_failures)
        })
        .collect()
}

fn parse_candidate_block(
    source: &SourceDocument,
    block: &str,
    block_index: usize,
    validation_failures: &mut Vec<String>,
) -> Option<ParsedCandidate> {
    let fields = parse_fields(block);
    let concept = fields.iter().find_value("concept");
    let question = fields.iter().find_value("question");
    let answer = fields.iter().find_value("answer");

    let (Some(concept), Some(question), Some(answer)) = (concept, question, answer) else {
        validation_failures.push(format!(
            "{} block {block_index}: concept, question, and answer are required",
            source.id
        ));
        return None;
    };

    Some(ParsedCandidate {
        source: source.clone(),
        block_index,
        activity_kind: parse_activity_kind(fields.iter().find_value("activity")),
        activity_stage: fields
            .iter()
            .find_value("stage")
            .unwrap_or("recognition")
            .to_owned(),
        concept: concept.to_owned(),
        question: question.to_owned(),
        answer: answer.to_owned(),
        reference: fields.iter().find_value("reference").map(str::to_owned),
        distractors: split_list(fields.iter().find_value("distractors")),
        worked_solution: fields
            .iter()
            .find_value("worked solution")
            .or_else(|| fields.iter().find_value("worked"))
            .map(str::to_owned),
        unsupported: parse_boolean(fields.iter().find_value("unsupported")).unwrap_or(false)
            || parse_boolean(fields.iter().find_value("supported")) == Some(false),
    })
}

fn parse_fields(block: &str) -> Vec<(String, String)> {
    block
        .lines()
        .filter_map(|raw_line| {
            let line = raw_line.trim();
            let separator = line.find(':')?;
            let key = line[..separator].trim().to_lowercase();
            let value = line[separator + 1..].trim().to_owned();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

trait FindField<'a> {
    fn find_value(self, key: &str) -> Option<&'a str>;
}

impl<'a, T> FindField<'a> for T
where
    T: Iterator<Item = &'a (String, String)>,
{
    fn find_value(self, key: &str) -> Option<&'a str> {
        self.filter_map(|(candidate_key, value)| {
            if candidate_key == key {
                Some(value.as_str())
            } else {
                None
            }
        })
        .last()
    }
}

fn parse_activity_kind(value: Option<&str>) -> GeneratedLearningActivityKind {
    if value.is_some_and(|value| value.eq_ignore_ascii_case("exercise")) {
        GeneratedLearningActivityKind::Exercise
    } else {
        GeneratedLearningActivityKind::Quiz
    }
}

fn parse_boolean(value: Option<&str>) -> Option<bool> {
    match value?.to_lowercase().as_str() {
        "true" | "yes" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

fn split_list(value: Option<&str>) -> Vec<String> {
    value.map_or_else(Vec::new, |value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn build_draft(candidate: &ParsedCandidate, context: &DraftContext<'_>) -> GeneratedPromptDraft {
    let unit_id = ReviewUnitId::new(generated_id(
        "generated",
        activity_kind_slug(&candidate.activity_kind),
        candidate,
    ));
    let reasons = validation_reasons(candidate, context.duplicate);
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
        id: generated_id(context.run_id, "draft", candidate),
        source_document_ids: vec![candidate.source.id.clone()],
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
            source_key: Some(candidate.source.id.clone()),
            domain_key: Some(source_kind_key(&candidate.source.kind).to_owned()),
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

fn build_prompt(candidate: &ParsedCandidate, review_unit_id: &ReviewUnitId) -> Prompt {
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

fn validation_reasons(candidate: &ParsedCandidate, duplicate: bool) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.unsupported {
        reasons.push("Unsupported by cited source material".to_owned());
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

fn candidate_signature(candidate: &ParsedCandidate) -> String {
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

fn generated_id(prefix: &str, kind: &str, candidate: &ParsedCandidate) -> String {
    [
        prefix.to_owned(),
        kind.to_owned(),
        slug(&candidate.source.id),
        candidate.block_index.to_string(),
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
