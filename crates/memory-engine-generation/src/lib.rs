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
//! substring match), duplicates are filtered, and exercises require worked
//! solutions.

mod provider;

use std::{collections::BTreeSet, error::Error, fmt};

pub use provider::{
    classify_learning_intent, enforce_content_policy, BridgeMaterial, BridgeMaterialProvider,
    BridgeMaterialRequest, DraftCandidate, DraftProvider, DraftRejection, FakeModelProvider,
    FallbackProvider, LearningIntent, LearningIntentClassification, ProviderDrafts,
    ProviderFailure, ProviderFailureKind, ProviderUsage, ReferenceNoteDraft, ReferenceNoteProvider,
    ReferenceNoteRequest, ReviewPerformanceContext, SourceAuthorizationContext,
    SourceAuthorizationError, StructuredBlockProvider,
};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, ReviewUnitId, ReviewUnitLifecycle,
};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreError, BetaStoreSnapshot,
    ConceptReferenceNote, GeneratedLearningActivityKind, GeneratedPromptDraft,
    GeneratedPromptModel, GeneratedPromptValidation, GeneratedPromptValidationStatus,
    GenerationRun, GenerationRunUsage, PersistedQueueCandidate, ReferenceSpan, SourceDocument,
    SourceDocumentKind, SourcePermission, SourcePermissionReceipt,
};

/// The text a world-knowledge card grounds in: the captured input itself. A
/// card with no verbatim quote was expanded from the model's knowledge about
/// the input (a topic), so the input is its reference seed.
///
/// `require_source` (run before any persistence) guarantees a non-blank body, so
/// the body branch is always taken and the seed is never empty — the store's
/// non-blank reference-span check therefore cannot trip on a knowledge card. The
/// title fallback only matters if a future change lets an empty-body source
/// reach persistence.
fn knowledge_seed(source: &SourceDocument) -> &str {
    source
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| source.title.trim())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BetaGenerationRequest {
    pub run_id: String,
    pub source_document_ids: Vec<String>,
    pub parent_review_unit_id: Option<ReviewUnitId>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeGenerationRequest {
    pub run_id: String,
    pub parent_review_unit_id: ReviewUnitId,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub default_due: i64,
    pub model: Option<GeneratedPromptModel>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeGenerationResult {
    pub run_id: String,
    pub concept_key: String,
    pub reference_note_created: bool,
    pub accepted_draft_ids: Vec<String>,
    pub rejected_draft_ids: Vec<String>,
    pub validation_failures: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BetaGenerationError<E = BetaStoreError> {
    Store(E),
    UnknownSourceDocument(String),
    ArchivedSourceDocument(String),
    LocalOnlySource(String),
    UnknownReviewUnit(ReviewUnitId),
    SourceDocumentHasNoTextBody(String),
    ProviderFailure(String),
}

impl<E> fmt::Display for BetaGenerationError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::UnknownSourceDocument(id) => write!(formatter, "Unknown source document: {id}"),
            Self::ArchivedSourceDocument(id) => write!(formatter, "Archived source document: {id}"),
            Self::LocalOnlySource(id) => write!(
                formatter,
                "Local-only source {id} cannot be sent to the model provider."
            ),
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
            Self::SourceDocumentHasNoTextBody(id) => {
                write!(formatter, "Source document has no text body: {id}")
            }
            Self::ProviderFailure(message) => formatter.write_str(message),
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
            Self::UnknownSourceDocument(_)
            | Self::ArchivedSourceDocument(_)
            | Self::LocalOnlySource(_)
            | Self::UnknownReviewUnit(_)
            | Self::SourceDocumentHasNoTextBody(_)
            | Self::ProviderFailure(_) => None,
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

    /// Save provider-generated concept reference material.
    ///
    /// # Errors
    ///
    /// Returns the store error when the note is rejected or cannot be persisted.
    fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, Self::Error>;

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

    fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, Self::Error> {
        BetaPersistenceStore::save_concept_reference_note(self, note)
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
    run_beta_generation_internal(store, &StructuredBlockProvider, request, false)
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
    run_beta_generation_internal(store, provider, request, true)
}

fn run_beta_generation_internal<S>(
    store: &mut S,
    provider: &dyn DraftProvider,
    request: BetaGenerationRequest,
    enforce_source_permission: bool,
) -> Result<BetaGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let snapshot = store.snapshot().map_err(BetaGenerationError::Store)?;
    let sources = load_sources::<S::Error>(&snapshot, &request.source_document_ids)?;
    ensure_sources_not_archived(&sources)?;
    if enforce_source_permission {
        ensure_model_eligible(&sources)?;
    }
    let model = request.model.clone().unwrap_or_else(|| provider.model());
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

    save_generation_run_progress(
        store,
        &request,
        &model,
        RunProgress::Started,
        source_permission_receipts(&sources),
    )?;
    let mut seen_signatures =
        existing_accepted_candidate_signatures(&snapshot.generated_prompt_drafts);
    for source in &sources {
        let Some(source_usage) = process_generation_source(
            store,
            provider,
            &request,
            source,
            &mut seen_signatures,
            &mut validation_failures,
            &mut draft_ids,
            &mut accepted_draft_ids,
            &mut rejected_draft_ids,
            &mut producing_models,
        )?
        else {
            continue;
        };
        usage = merge_usage(usage, source_usage.drafts);
        usage = merge_usage(usage, source_usage.repair);
        save_generation_run_progress(
            store,
            &request,
            &model,
            RunProgress::InProgress {
                draft_ids: draft_ids.clone(),
                validation_failures: validation_failures.clone(),
                usage: usage.clone(),
            },
            source_permission_receipts(&sources),
        )?;
    }

    let run_model = match producing_models.as_slice() {
        [single] => single.clone(),
        _ => model.clone(),
    };
    save_generation_run_progress(
        store,
        &request,
        &run_model,
        RunProgress::Completed {
            draft_ids: draft_ids.clone(),
            validation_failures: validation_failures.clone(),
            usage,
        },
        source_permission_receipts(&sources),
    )?;

    Ok(BetaGenerationResult {
        run_id: request.run_id,
        draft_ids,
        accepted_draft_ids,
        rejected_draft_ids,
        validation_failures,
    })
}

/// The two provider-usage additions [`process_generation_source`] accumulated
/// for one source, mirrored in the same order the original inline loop body
/// merged them: draft generation first, then repair.
struct SourceGenerationUsage {
    drafts: Option<ProviderUsage>,
    repair: Option<ProviderUsage>,
}

/// Runs generation and repair for one source, mutating the run's shared
/// accumulators in place. Returns `None` when draft generation itself failed
/// for this source (the caller records the failure and skips this source's
/// progress checkpoint, matching the prior inline-loop `continue`), or
/// `Some(usage)` mirroring the two provider-usage additions the original
/// inline loop body accumulated.
#[allow(clippy::too_many_arguments)]
fn process_generation_source<S>(
    store: &mut S,
    provider: &dyn DraftProvider,
    request: &BetaGenerationRequest,
    source: &SourceDocument,
    seen_signatures: &mut Vec<CandidateSignature>,
    validation_failures: &mut Vec<String>,
    draft_ids: &mut Vec<String>,
    accepted_draft_ids: &mut Vec<String>,
    rejected_draft_ids: &mut Vec<String>,
    producing_models: &mut Vec<GeneratedPromptModel>,
) -> Result<Option<SourceGenerationUsage>, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let drafts = match provider.generate_drafts(source) {
        Ok(drafts) => drafts,
        Err(failure) => {
            validation_failures.push(format!("{}: {failure}", source.id));
            return Ok(None);
        }
    };
    let drafts = enforce_content_policy(source, drafts);
    validation_failures.extend(drafts.failures);
    // Stamp drafts with the provider that actually generated them, not the
    // composite's declared identity. A caller override still wins.
    let source_model = request.model.clone().unwrap_or(drafts.model);
    if !drafts.candidates.is_empty() && !producing_models.contains(&source_model) {
        producing_models.push(source_model.clone());
    }

    // Grounding is decided per card, by the generator, not by a heuristic
    // over the source: a card that cites a verbatim quote is source-grounded
    // (the quote must verify); a card with no quote is a world-knowledge
    // expansion grounded in the captured topic itself.
    let mut source_rejections = Vec::new();
    let mut max_candidate_index = 0;
    let learning_intent = drafts.learning_intent;
    persist_candidates(
        store,
        source,
        drafts.candidates,
        &mut PersistCandidatesContext {
            request,
            source_model: &source_model,
            seen_signatures,
            validation_failures,
            draft_ids,
            accepted_draft_ids,
            rejected_draft_ids,
            source_rejections: &mut source_rejections,
            max_candidate_index: &mut max_candidate_index,
            learning_intent,
        },
    )?;

    let repair_usage = try_repair_source(
        store,
        provider,
        source,
        request,
        &source_rejections,
        seen_signatures,
        validation_failures,
        draft_ids,
        accepted_draft_ids,
        rejected_draft_ids,
        producing_models,
        &mut max_candidate_index,
    )?;

    Ok(Some(SourceGenerationUsage {
        drafts: drafts.usage,
        repair: repair_usage,
    }))
}

fn load_sources<E>(
    snapshot: &BetaStoreSnapshot,
    source_document_ids: &[String],
) -> Result<Vec<SourceDocument>, BetaGenerationError<E>> {
    source_document_ids
        .iter()
        .map(|source_document_id| require_source(&snapshot.source_documents, source_document_id))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn try_repair_source<S>(
    store: &mut S,
    provider: &dyn DraftProvider,
    source: &SourceDocument,
    request: &BetaGenerationRequest,
    source_rejections: &[DraftRejection],
    seen_signatures: &mut Vec<CandidateSignature>,
    validation_failures: &mut Vec<String>,
    draft_ids: &mut Vec<String>,
    accepted_draft_ids: &mut Vec<String>,
    rejected_draft_ids: &mut Vec<String>,
    producing_models: &mut Vec<GeneratedPromptModel>,
    max_candidate_index: &mut usize,
) -> Result<Option<ProviderUsage>, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    if source_rejections.is_empty() {
        return Ok(None);
    }
    let repair_rejections =
        &source_rejections[..source_rejections.len().min(MAX_REPAIR_REJECTIONS)];

    let repair = match provider.repair_drafts(source, repair_rejections) {
        Ok(Some(repair)) => repair,
        Ok(None) => return Ok(None),
        Err(failure) => {
            validation_failures.push(format!("{} repair: {failure}", source.id));
            return Ok(None);
        }
    };
    let repair = enforce_content_policy(source, repair);
    validation_failures.extend(repair.failures);
    let usage = repair.usage;
    let learning_intent = repair.learning_intent;
    let repair_model = request.model.clone().unwrap_or(repair.model);
    if !repair.candidates.is_empty() && !producing_models.contains(&repair_model) {
        producing_models.push(repair_model.clone());
    }
    let mut repair_candidates = repair
        .candidates
        .into_iter()
        .take(repair_rejections.len())
        .collect::<Vec<_>>();
    for (offset, candidate) in repair_candidates.iter_mut().enumerate() {
        candidate.index = *max_candidate_index + offset + 1;
    }
    let mut repair_rejections = Vec::new();
    persist_candidates(
        store,
        source,
        repair_candidates,
        &mut PersistCandidatesContext {
            request,
            source_model: &repair_model,
            seen_signatures,
            validation_failures,
            draft_ids,
            accepted_draft_ids,
            rejected_draft_ids,
            source_rejections: &mut repair_rejections,
            max_candidate_index,
            learning_intent,
        },
    )?;

    Ok(usage)
}

fn save_generation_run_progress<S>(
    store: &mut S,
    request: &BetaGenerationRequest,
    model: &GeneratedPromptModel,
    progress: RunProgress,
    source_permissions: Vec<SourcePermissionReceipt>,
) -> Result<(), BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    store
        .save_generation_run(run_receipt(request, model, progress, source_permissions))
        .map(|_| ())
        .map_err(BetaGenerationError::Store)
}

struct PersistCandidatesContext<'a> {
    request: &'a BetaGenerationRequest,
    source_model: &'a GeneratedPromptModel,
    seen_signatures: &'a mut Vec<CandidateSignature>,
    validation_failures: &'a mut Vec<String>,
    draft_ids: &'a mut Vec<String>,
    accepted_draft_ids: &'a mut Vec<String>,
    rejected_draft_ids: &'a mut Vec<String>,
    source_rejections: &'a mut Vec<DraftRejection>,
    max_candidate_index: &'a mut usize,
    learning_intent: Option<LearningIntent>,
}

fn persist_candidates<S>(
    store: &mut S,
    source: &SourceDocument,
    candidates: Vec<DraftCandidate>,
    context: &mut PersistCandidatesContext<'_>,
) -> Result<(), BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    for candidate in candidates {
        *context.max_candidate_index = (*context.max_candidate_index).max(candidate.index);
        // The generator decides grounding per card. A cited quote means the
        // card claims to come from the provided text, so the quote must verify
        // against it (a fabricated citation is rejected downstream). No quote
        // means a world-knowledge expansion of the input, which grounds in the
        // captured input itself as its reference seed.
        let cited = candidate
            .evidence
            .as_deref()
            .map(str::trim)
            .filter(|quote| !normalize_for_match(quote).is_empty());
        let grounded = cited.is_some();
        let evidence: &str = cited.unwrap_or_else(|| knowledge_seed(source));

        let signature = CandidateSignature::from_candidate(&candidate);
        let duplicate = context
            .seen_signatures
            .iter()
            .any(|seen| seen.duplicates(&signature));
        if duplicate {
            let reason = "Duplicate-ish generated draft".to_owned();
            context
                .validation_failures
                .push(format!("{} block {}: {reason}", source.id, candidate.index));
            context
                .source_rejections
                .push(candidate_rejection(&candidate, vec![reason]));
            continue;
        }

        let draft = persist_candidate(
            store,
            source,
            &candidate,
            evidence,
            &PersistParams {
                request: context.request,
                model: context.source_model,
                duplicate: false,
                grounded,
                quote_verified: !grounded || source_contains_quote(source, evidence),
                learning_intent: context.learning_intent,
            },
        )?;

        context.draft_ids.push(draft.id.clone());
        if draft.validation.status == GeneratedPromptValidationStatus::Accepted {
            context.seen_signatures.push(signature);
            context.accepted_draft_ids.push(draft.id);
        } else {
            context.source_rejections.push(candidate_rejection(
                &candidate,
                draft.validation.reasons.clone(),
            ));
            context.rejected_draft_ids.push(draft.id);
        }
    }

    Ok(())
}

fn candidate_rejection(candidate: &DraftCandidate, reasons: Vec<String>) -> DraftRejection {
    DraftRejection {
        index: candidate.index,
        concept: candidate.concept.clone(),
        question: candidate.question.clone(),
        answer: candidate.answer.clone(),
        reasons,
    }
}

/// Generate bridge material for one approved parent review unit.
///
/// Bridge generation is concept-backed rather than source-backed: if the
/// concept has no cached reference note, the provider writes one first; easier
/// bridge drafts cite that concept note and enter the queue with the supplied
/// due timestamp. The caller owns deferring the parent item.
///
/// # Errors
///
/// Returns [`BetaGenerationError`] when the parent is unknown or store writes
/// fail.
pub fn run_bridge_generation_with_provider<S>(
    store: &mut S,
    provider: &dyn BridgeMaterialProvider,
    request: BridgeGenerationRequest,
) -> Result<BridgeGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    run_bridge_generation_internal(store, provider, request, true)
}

/// Generate bridge material with the deterministic local provider path.
///
/// # Errors
///
/// Returns [`BetaGenerationError`] when the parent is unknown or store writes
/// fail.
pub fn run_bridge_generation<S>(
    store: &mut S,
    request: BridgeGenerationRequest,
) -> Result<BridgeGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    run_bridge_generation_internal(store, &FakeModelProvider, request, false)
}

fn run_bridge_generation_internal<S>(
    store: &mut S,
    provider: &dyn BridgeMaterialProvider,
    request: BridgeGenerationRequest,
    enforce_source_permission: bool,
) -> Result<BridgeGenerationResult, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let snapshot = store.snapshot().map_err(BetaGenerationError::Store)?;
    let context = bridge_generation_context::<S::Error>(&snapshot, &request.parent_review_unit_id)?;
    let source_documents =
        parent_source_documents::<S::Error>(&snapshot, &context.parent, enforce_source_permission)?;
    let source_document_ids = source_documents
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    let source_permissions = source_permission_receipts(&source_documents);
    let authorization = SourceAuthorizationContext::from_sources(&source_documents).map_err(
        |error| match error {
            SourceAuthorizationError::ArchivedSourceDocument(id) => {
                BetaGenerationError::ArchivedSourceDocument(id)
            }
        },
    )?;
    if enforce_source_permission {
        ensure_model_eligible_receipts(&source_permissions)?;
    }
    let provider_request = bridge_material_request(&snapshot, &context, authorization);
    let material = provider
        .generate_bridge_material(&provider_request)
        .map_err(|failure| BetaGenerationError::ProviderFailure(failure.to_string()))?;
    let model = request
        .model
        .clone()
        .unwrap_or_else(|| material.model.clone());
    let (note, note_body, reference_note_created) =
        bridge_reference_note(&context, &material, &model, request.started_at);

    let run_request = bridge_run_request(&request, &source_document_ids);
    store
        .save_generation_run(run_receipt(
            &run_request,
            &model,
            RunProgress::Started,
            source_permissions.clone(),
        ))
        .map_err(BetaGenerationError::Store)?;
    store
        .save_concept_reference_note(note)
        .map_err(BetaGenerationError::Store)?;

    let bridge_drafts = save_bridge_drafts(
        store,
        &snapshot,
        material.candidates,
        &BridgeDraftBaseContext {
            run_id: &request.run_id,
            concept_key: &context.concept_key,
            reference_note_body: &note_body,
            model: &model,
            due: request.default_due,
            created_at: request.started_at,
            parent_review_unit_id: &context.parent.review_unit_id,
            parent_progression: context.parent.queue.progression.as_ref(),
            parent_stage_order: context.parent_stage_order,
            source_document_ids: &source_document_ids,
            source_key: context.parent.queue.source_key.as_ref(),
        },
    )?;

    store
        .save_generation_run(run_receipt(
            &run_request,
            &model,
            RunProgress::Completed {
                draft_ids: bridge_drafts.draft_ids,
                validation_failures: bridge_drafts.validation_failures.clone(),
                usage: material.usage.as_ref().map(provider_usage_to_run_usage),
            },
            source_permissions,
        ))
        .map_err(BetaGenerationError::Store)?;

    if bridge_drafts.accepted_draft_ids.is_empty() {
        return Err(BetaGenerationError::ProviderFailure(format!(
            "Bridge material produced no accepted drafts: {}",
            bridge_drafts.validation_failures.join("; ")
        )));
    }

    Ok(BridgeGenerationResult {
        run_id: request.run_id,
        concept_key: context.concept_key,
        reference_note_created,
        accepted_draft_ids: bridge_drafts.accepted_draft_ids,
        rejected_draft_ids: bridge_drafts.rejected_draft_ids,
        validation_failures: bridge_drafts.validation_failures,
    })
}

struct BridgeGenerationContext {
    parent: BetaReviewUnitRecord,
    concept_key: String,
    concept_label: String,
    cached_note: Option<ConceptReferenceNote>,
    parent_stage_order: u32,
}

fn bridge_generation_context<E>(
    snapshot: &BetaStoreSnapshot,
    parent_review_unit_id: &ReviewUnitId,
) -> Result<BridgeGenerationContext, BetaGenerationError<E>> {
    let parent = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == *parent_review_unit_id)
        .cloned()
        .ok_or_else(|| BetaGenerationError::UnknownReviewUnit(parent_review_unit_id.clone()))?;
    let parent_draft = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| draft.review_unit_id == parent.review_unit_id);
    let concept_key = parent
        .queue
        .concept_key
        .clone()
        .or_else(|| parent_draft.and_then(|draft| draft.queue.concept_key.clone()))
        .unwrap_or_else(|| parent.review_unit_id.as_str().to_owned());
    let cached_note = snapshot
        .concept_reference_notes
        .iter()
        .find(|note| note.concept_key == concept_key)
        .cloned();
    let parent_stage_order = parent
        .queue
        .progression
        .as_ref()
        .map_or(1, |progression| progression.stage_order);

    Ok(BridgeGenerationContext {
        parent,
        concept_label: concept_key.replace('-', " "),
        concept_key,
        cached_note,
        parent_stage_order,
    })
}

fn bridge_material_request(
    snapshot: &BetaStoreSnapshot,
    context: &BridgeGenerationContext,
    authorization: SourceAuthorizationContext,
) -> BridgeMaterialRequest {
    BridgeMaterialRequest::new(
        context.concept_key.clone(),
        context.concept_label.clone(),
        context.parent.review_unit_id.clone(),
        prompt_text(&context.parent.prompt),
        expected_answer(&context.parent.prompt),
        context.parent_stage_order,
        context.cached_note.as_ref().map(|note| note.body.clone()),
        recent_performance_for_concept(snapshot, &context.concept_key),
        authorization,
    )
}

fn bridge_reference_note(
    context: &BridgeGenerationContext,
    material: &BridgeMaterial,
    model: &GeneratedPromptModel,
    created_at: i64,
) -> (ConceptReferenceNote, String, bool) {
    let created = context.cached_note.is_none();
    let body = context.cached_note.as_ref().map_or_else(
        || material.reference_note.body.clone(),
        |note| note.body.clone(),
    );
    let note = context
        .cached_note
        .clone()
        .unwrap_or_else(|| ConceptReferenceNote {
            concept_key: context.concept_key.clone(),
            title: material.reference_note.title.clone(),
            body: material.reference_note.body.clone(),
            model: model.clone(),
            created_at,
            updated_at: created_at,
        });

    (note, body, created)
}

struct BridgeDraftBaseContext<'a> {
    run_id: &'a str,
    concept_key: &'a str,
    reference_note_body: &'a str,
    model: &'a GeneratedPromptModel,
    due: i64,
    created_at: i64,
    parent_review_unit_id: &'a ReviewUnitId,
    parent_progression: Option<&'a ProgressionMetadata>,
    parent_stage_order: u32,
    source_document_ids: &'a [String],
    source_key: Option<&'a String>,
}

struct BridgeDraftPersistence {
    draft_ids: Vec<String>,
    accepted_draft_ids: Vec<String>,
    rejected_draft_ids: Vec<String>,
    validation_failures: Vec<String>,
}

fn save_bridge_drafts<S>(
    store: &mut S,
    snapshot: &BetaStoreSnapshot,
    candidates: Vec<DraftCandidate>,
    base: &BridgeDraftBaseContext<'_>,
) -> Result<BridgeDraftPersistence, BetaGenerationError<S::Error>>
where
    S: BetaGenerationStore,
{
    let mut result = BridgeDraftPersistence {
        draft_ids: Vec::new(),
        accepted_draft_ids: Vec::new(),
        rejected_draft_ids: Vec::new(),
        validation_failures: Vec::new(),
    };
    let mut seen_signatures = existing_draft_signatures(&snapshot.generated_prompt_drafts);
    seen_signatures.extend(existing_review_unit_signatures(&snapshot.review_units));
    let mut drafts = Vec::new();

    for candidate in candidates {
        let signature = candidate_signature(&candidate);
        let duplicate = seen_signatures.contains(&signature);
        seen_signatures.insert(signature);
        let easier = stage_order(&candidate.activity_stage, &candidate.activity_kind)
            < base.parent_stage_order;
        let draft = bridge_draft(
            &candidate,
            &BridgeDraftContext {
                run_id: base.run_id,
                concept_key: base.concept_key,
                reference_note_body: base.reference_note_body,
                model: base.model,
                due: base.due,
                created_at: base.created_at,
                duplicate,
                easier,
                parent_review_unit_id: base.parent_review_unit_id,
                parent_progression: base.parent_progression,
                source_document_ids: base.source_document_ids,
                source_key: base.source_key,
            },
        );
        drafts.push(draft);
    }

    enforce_bridge_pack_ladder(&mut drafts);
    for draft in drafts {
        record_bridge_draft(&mut result, &draft);
        store
            .save_generated_prompt_draft(draft)
            .map_err(BetaGenerationError::Store)?;
    }

    Ok(result)
}

fn enforce_bridge_pack_ladder(drafts: &mut [GeneratedPromptDraft]) {
    let accepted_stage_orders = drafts
        .iter()
        .filter(|draft| draft.validation.status == GeneratedPromptValidationStatus::Accepted)
        .map(|draft| stage_order(&draft.activity_stage, &draft.activity_kind))
        .collect::<BTreeSet<_>>();
    let accepted_count = drafts
        .iter()
        .filter(|draft| draft.validation.status == GeneratedPromptValidationStatus::Accepted)
        .count();
    if accepted_count <= 1 || accepted_stage_orders.len() >= 2 {
        return;
    }

    for draft in drafts
        .iter_mut()
        .filter(|draft| draft.validation.status == GeneratedPromptValidationStatus::Accepted)
    {
        draft.validation.status = GeneratedPromptValidationStatus::Rejected;
        draft
            .validation
            .reasons
            .push("Bridge drafts must form a scaffold ladder".to_owned());
        draft
            .critique_notes
            .push("Rejected: Bridge drafts must form a scaffold ladder".to_owned());
    }
}

fn record_bridge_draft(result: &mut BridgeDraftPersistence, draft: &GeneratedPromptDraft) {
    if draft.validation.status == GeneratedPromptValidationStatus::Accepted {
        result.accepted_draft_ids.push(draft.id.clone());
    } else {
        result
            .validation_failures
            .extend(draft.validation.reasons.clone());
        result.rejected_draft_ids.push(draft.id.clone());
    }
    result.draft_ids.push(draft.id.clone());
}

fn bridge_run_request(
    request: &BridgeGenerationRequest,
    source_document_ids: &[String],
) -> BetaGenerationRequest {
    BetaGenerationRequest {
        run_id: request.run_id.clone(),
        source_document_ids: source_document_ids.to_vec(),
        parent_review_unit_id: Some(request.parent_review_unit_id.clone()),
        started_at: request.started_at,
        completed_at: request.completed_at,
        default_due: request.default_due,
        model: request.model.clone(),
    }
}

struct PersistParams<'a> {
    request: &'a BetaGenerationRequest,
    model: &'a GeneratedPromptModel,
    duplicate: bool,
    grounded: bool,
    quote_verified: bool,
    learning_intent: Option<LearningIntent>,
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
    // A source-grounded card cites the verbatim quote that proves its answer; a
    // world-knowledge card cites the captured input it was expanded from. Either
    // way the span is a real pointer into the source the store can resolve.
    let label = if params.grounded {
        format!("{} source evidence", candidate.concept)
    } else {
        format!("{} input", candidate.concept)
    };
    let reference_span = store
        .save_reference_span(ReferenceSpan {
            id: generated_id(&params.request.run_id, "ref", &source.id, candidate),
            source_document_id: source.id.clone(),
            label,
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
                grounded: params.grounded,
                quote_verified: params.quote_verified,
                learning_intent: params.learning_intent,
            },
        ))
        .map_err(BetaGenerationError::Store)
}

enum RunProgress {
    Started,
    InProgress {
        draft_ids: Vec<String>,
        validation_failures: Vec<String>,
        usage: Option<GenerationRunUsage>,
    },
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
    source_permissions: Vec<SourcePermissionReceipt>,
) -> GenerationRun {
    let (draft_ids, completed_at, validation_failures, usage) = match progress {
        RunProgress::Started => (Vec::new(), None, Vec::new(), None),
        RunProgress::InProgress {
            draft_ids,
            validation_failures,
            usage,
        } => (draft_ids, None, validation_failures, usage),
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
        parent_review_unit_id: request.parent_review_unit_id.clone(),
        draft_ids,
        provider: model.provider.clone(),
        model: model.name.clone(),
        started_at: request.started_at,
        completed_at,
        validation_failures,
        usage,
        source_permissions,
        prompt_version: model.version.clone(),
    }
}

fn source_permission_receipts(sources: &[SourceDocument]) -> Vec<SourcePermissionReceipt> {
    sources
        .iter()
        .map(|source| SourcePermissionReceipt {
            source_document_id: source.id.clone(),
            permission: source.permission.clone(),
            consented: source.permission == SourcePermission::ModelEligible,
        })
        .collect()
}

fn ensure_model_eligible<E>(sources: &[SourceDocument]) -> Result<(), BetaGenerationError<E>> {
    ensure_sources_not_archived(sources)?;
    ensure_model_eligible_receipts(&source_permission_receipts(sources))
}

fn ensure_sources_not_archived<E>(
    sources: &[SourceDocument],
) -> Result<(), BetaGenerationError<E>> {
    if let Some(source) = sources.iter().find(|source| source.archived_at.is_some()) {
        return Err(BetaGenerationError::ArchivedSourceDocument(
            source.id.clone(),
        ));
    }
    Ok(())
}

fn ensure_model_eligible_receipts<E>(
    receipts: &[SourcePermissionReceipt],
) -> Result<(), BetaGenerationError<E>> {
    if let Some(receipt) = receipts
        .iter()
        .find(|receipt| receipt.permission == SourcePermission::LocalOnly)
    {
        return Err(BetaGenerationError::LocalOnlySource(
            receipt.source_document_id.clone(),
        ));
    }

    Ok(())
}

fn parent_source_documents<E>(
    snapshot: &BetaStoreSnapshot,
    parent: &BetaReviewUnitRecord,
    require_provenance: bool,
) -> Result<Vec<SourceDocument>, BetaGenerationError<E>> {
    let draft = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| draft.review_unit_id == parent.review_unit_id);
    let mut source_ids = draft
        .map(|draft| draft.source_document_ids.clone())
        .unwrap_or_default();
    if let Some(source_key) = parent.queue.source_key.as_ref() {
        if !source_ids.iter().any(|id| id == source_key) {
            source_ids.push(source_key.clone());
        }
    }
    if require_provenance && source_ids.is_empty() {
        return Err(BetaGenerationError::UnknownSourceDocument(
            parent.review_unit_id.to_string(),
        ));
    }
    let mut sources = Vec::with_capacity(source_ids.len());
    for source_id in &source_ids {
        let source = snapshot
            .source_documents
            .iter()
            .find(|source| &source.id == source_id)
            .ok_or_else(|| BetaGenerationError::UnknownSourceDocument(source_id.clone()))?;
        sources.push(source.clone());
    }
    ensure_sources_not_archived(&sources)?;
    Ok(sources)
}

struct DraftContext<'a> {
    run_id: &'a str,
    reference_span_id: &'a str,
    model: &'a GeneratedPromptModel,
    due: i64,
    created_at: i64,
    duplicate: bool,
    grounded: bool,
    quote_verified: bool,
    learning_intent: Option<LearningIntent>,
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

fn provider_usage_to_run_usage(usage: &ProviderUsage) -> GenerationRunUsage {
    GenerationRunUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd_micros: usage.cost_usd_micros,
        latency_ms: usage.latency_ms,
    }
}

fn source_contains_quote(source: &SourceDocument, quote: &str) -> bool {
    evidence_quote_matches(source.body.as_deref().unwrap_or_default(), quote)
}

/// Minimum number of words an evidence quote must carry to count as proof.
///
/// A single-word quote like "the" substring-matches almost any prose, so it
/// proves nothing about a fabricated answer; requiring a short phrase closes
/// that hole. Terse captures are the exception: when the entire source is one
/// or two words, the source can support itself exactly.
const MIN_EVIDENCE_WORDS: usize = 3;
const MAX_REPAIR_REJECTIONS: usize = 4;

/// Whether an evidence quote is substantive and appears in the source text
/// after folding case, punctuation, and whitespace.
///
/// Cheap models paraphrase formatting even when quoting faithfully, so exact
/// matching would reject grounded drafts — hence the normalization. A quote
/// shorter than the minimum evidence-word threshold is accepted only when it
/// equals the complete normalized source text, which lets a one-word capture be
/// studied without treating tiny substrings as proof. This is the same predicate
/// the generation trust gate applies, exported so eval judges score exactly what
/// production enforces.
#[must_use]
pub fn evidence_quote_matches(source_text: &str, quote: &str) -> bool {
    let quote = normalize_for_match(quote);
    if quote.is_empty() {
        return false;
    }

    let source = normalize_for_match(source_text);
    if quote.split(' ').filter(|word| !word.is_empty()).count() < MIN_EVIDENCE_WORDS {
        return quote == source;
    }

    source.contains(&quote)
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
    let reasons = validation_reasons(
        candidate,
        context.duplicate,
        context.grounded,
        context.quote_verified,
    );
    let status = if reasons.is_empty() {
        GeneratedPromptValidationStatus::Accepted
    } else {
        GeneratedPromptValidationStatus::Rejected
    };
    let critique_notes = if status == GeneratedPromptValidationStatus::Accepted {
        if context.grounded {
            vec![format!("Grounded in {}.", context.reference_span_id)]
        } else {
            vec![format!("Expanded from input \"{}\".", source.title)]
        }
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
        concept_reference_note_key: None,
        generation_run_id: Some(context.run_id.to_owned()),
        review_unit_id: unit_id.clone(),
        prompt_id: format!("{unit_id}-prompt"),
        prompt: build_prompt(candidate, &unit_id, context.learning_intent),
        queue: PersistedQueueCandidate {
            review_unit_id: unit_id.clone(),
            due: context.due,
            lifecycle: ReviewUnitLifecycle::active().with_ttl_expires_at(source.ttl_expires_at),
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

struct BridgeDraftContext<'a> {
    run_id: &'a str,
    concept_key: &'a str,
    reference_note_body: &'a str,
    model: &'a GeneratedPromptModel,
    due: i64,
    created_at: i64,
    duplicate: bool,
    easier: bool,
    parent_review_unit_id: &'a ReviewUnitId,
    parent_progression: Option<&'a ProgressionMetadata>,
    source_document_ids: &'a [String],
    source_key: Option<&'a String>,
}

fn bridge_draft(
    candidate: &DraftCandidate,
    context: &BridgeDraftContext<'_>,
) -> GeneratedPromptDraft {
    let unit_id = ReviewUnitId::new(bridge_generated_id(
        "bridge",
        activity_kind_slug(&candidate.activity_kind),
        context.parent_review_unit_id,
        candidate,
    ));
    let reasons = bridge_validation_reasons(candidate, context);
    let status = if reasons.is_empty() {
        GeneratedPromptValidationStatus::Accepted
    } else {
        GeneratedPromptValidationStatus::Rejected
    };
    let critique_notes = if status == GeneratedPromptValidationStatus::Accepted {
        vec![format!(
            "Grounded in concept reference note {}.",
            context.concept_key
        )]
    } else {
        reasons
            .iter()
            .map(|reason| format!("Rejected: {reason}"))
            .collect()
    };
    let stage_order = stage_order(&candidate.activity_stage, &candidate.activity_kind);
    let parent_group = context
        .parent_progression
        .and_then(|progression| progression.progression_group.clone())
        .unwrap_or_else(|| context.concept_key.to_owned());

    GeneratedPromptDraft {
        id: bridge_generated_id(
            context.run_id,
            "draft",
            context.parent_review_unit_id,
            candidate,
        ),
        source_document_ids: context.source_document_ids.to_vec(),
        reference_span_ids: Vec::new(),
        concept_reference_note_key: Some(context.concept_key.to_owned()),
        generation_run_id: Some(context.run_id.to_owned()),
        review_unit_id: unit_id.clone(),
        prompt_id: format!("{unit_id}-prompt"),
        // Bridge material is an easier derivative, not the original source's
        // verbatim intent; keep its exercise tolerant unless a future typed
        // bridge contract explicitly opts into recitation.
        prompt: build_prompt(candidate, &unit_id, None),
        queue: PersistedQueueCandidate {
            review_unit_id: unit_id.clone(),
            due: context.due.saturating_add(i64::from(stage_order)),
            lifecycle: ReviewUnitLifecycle::active(),
            progression: Some(ProgressionMetadata {
                progression_group: Some(parent_group),
                stage_order,
                requires: Vec::new(),
                supersedes: vec![context.parent_review_unit_id.clone()],
            }),
            concept_key: Some(context.concept_key.to_owned()),
            source_key: context
                .source_key
                .cloned()
                .or_else(|| context.source_document_ids.first().cloned()),
            domain_key: Some("bridge".to_owned()),
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

fn bridge_validation_reasons(
    candidate: &DraftCandidate,
    context: &BridgeDraftContext<'_>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.unsupported {
        reasons.push("Unsupported by bridge context".to_owned());
    }
    if context.reference_note_body.trim().is_empty() {
        reasons.push("Bridge drafts require a concept reference note".to_owned());
    }
    if !context.easier {
        reasons.push("Bridge drafts must target a lower progression stage".to_owned());
    }
    if context.duplicate {
        reasons.push("Duplicate-ish generated draft".to_owned());
    }
    if candidate.activity_kind == GeneratedLearningActivityKind::Exercise
        && candidate.worked_solution.is_none()
    {
        reasons.push("Exercises require a worked solution".to_owned());
    }

    reasons
}

fn build_prompt(
    candidate: &DraftCandidate,
    review_unit_id: &ReviewUnitId,
    learning_intent: Option<LearningIntent>,
) -> Prompt {
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
        kind: match learning_intent {
            Some(LearningIntent::VerbatimMemorization) => ExactPromptKind::Recitation,
            Some(
                LearningIntent::EnumerableSet
                | LearningIntent::ConceptUnderstanding
                | LearningIntent::FactRecall
                | LearningIntent::ProcedureProcess,
            )
            | None => ExactPromptKind::ShortAnswer,
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
    grounded: bool,
    quote_verified: bool,
) -> Vec<String> {
    let mut reasons = Vec::new();
    // A generator-flagged "unsupported" card is rejected in BOTH lanes: the flag
    // is a quality red flag, not a provenance claim, so it must bite even for a
    // quote-free world-knowledge card (otherwise omitting the quote would launder
    // an explicitly-flagged-bad answer into the queue).
    if candidate.unsupported {
        reasons.push("Unsupported by cited source material".to_owned());
    }
    // The quote check applies only to a card that CLAIMS a source quote: it must
    // verify, or the card is rejected — a fabricated citation is the
    // anti-hallucination failure this catches. A world-knowledge card has no
    // quote to verify; its factual correctness rests on the model and the
    // human keep-review the card still passes through before study.
    if grounded && !quote_verified {
        reasons.push("Evidence quote not found in cited source".to_owned());
    }
    if duplicate {
        reasons.push("Duplicate-ish generated draft".to_owned());
    }
    if references_source_artifact(&candidate.question) {
        reasons.push("Question references the source instead of standing alone".to_owned());
    }
    if candidate.activity_kind == GeneratedLearningActivityKind::Exercise
        && candidate.worked_solution.is_none()
    {
        reasons.push("Exercises require a worked solution".to_owned());
    }
    reasons.extend(mcq_quality_reasons(candidate));

    reasons
}

/// A card must stand alone — the learner sees only the question, never the
/// material it was generated from. A question that points back at the study
/// artifact ("the subject of the source text", "according to the passage") is
/// unanswerable once the source is gone, and is exactly the meta-card the
/// generation prompt is told to avoid. The model usually obeys; this gate
/// rejects the ones that slip through so the repair pass regenerates a
/// self-contained card. Phrases mirror the prompt's own forbidden list and are
/// multi-word to keep a legitimate bare "text"/"source" from tripping it.
///
/// Exported so the generation bench eval scores self-reference with the exact
/// same definition the runtime gate enforces. A private copy in the bench crate
/// had silently drifted to a shorter phrase list, which let the eval pass cards
/// (e.g. "the list above") that the runtime gate actually rejects.
#[must_use]
pub fn references_source_artifact(question: &str) -> bool {
    const ARTIFACT_PHRASES: &[&str] = &[
        "source text",
        "the source material",
        "subject of the source",
        "subject of the text",
        "the passage",
        "the excerpt",
        "the list above",
        "the text above",
        "the table above",
        "the article above",
        "the document above",
        "the reading above",
        "according to the source",
        "according to the passage",
        "according to the text",
        "according to the article",
        "in the passage",
    ];
    let normalized = normalize_for_match(question);
    ARTIFACT_PHRASES
        .iter()
        .any(|phrase| normalized.contains(phrase))
}

fn mcq_quality_reasons(candidate: &DraftCandidate) -> Vec<String> {
    if candidate.activity_kind != GeneratedLearningActivityKind::Quiz
        || candidate.distractors.is_empty()
    {
        return Vec::new();
    }

    let mut reasons = Vec::new();
    if compound_question(&candidate.question) {
        reasons.push("MCQ question tests multiple atoms".to_owned());
    }

    let answer = normalize_for_match(&candidate.answer);
    for distractor in &candidate.distractors {
        let normalized = normalize_for_match(distractor);
        if normalized.is_empty() {
            continue;
        }
        if normalized == answer {
            reasons.push("MCQ distractor duplicates the correct answer".to_owned());
        }
    }

    reasons.sort();
    reasons.dedup();
    reasons
}

fn compound_question(question: &str) -> bool {
    let normalized = normalize_for_match(question);
    (normalized.contains(" and ")
        || normalized.contains(" plus ")
        || normalized.contains(" along with "))
        && (normalized.contains("what ")
            || normalized.contains("which ")
            || normalized.contains("why ")
            || normalized.contains("how "))
}

fn existing_accepted_candidate_signatures(
    drafts: &[GeneratedPromptDraft],
) -> Vec<CandidateSignature> {
    drafts
        .iter()
        .filter(|draft| draft.validation.status == GeneratedPromptValidationStatus::Accepted)
        .map(CandidateSignature::from_draft)
        .collect()
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

fn existing_review_unit_signatures(review_units: &[BetaReviewUnitRecord]) -> BTreeSet<String> {
    review_units
        .iter()
        .map(|unit| {
            [
                unit.queue.concept_key.clone().unwrap_or_default(),
                prompt_text(&unit.prompt).to_lowercase(),
                expected_answer(&unit.prompt).to_lowercase(),
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CandidateSignature {
    concept: String,
    question: String,
    answer: String,
}

impl CandidateSignature {
    fn from_candidate(candidate: &DraftCandidate) -> Self {
        Self {
            concept: slug(&candidate.concept),
            question: normalize_for_match(&candidate.question),
            answer: normalize_for_match(&candidate.answer),
        }
    }

    fn from_draft(draft: &GeneratedPromptDraft) -> Self {
        Self {
            concept: draft.queue.concept_key.clone().unwrap_or_default(),
            question: normalize_for_match(&prompt_text(&draft.prompt)),
            answer: normalize_for_match(&expected_answer(&draft.prompt)),
        }
    }

    fn duplicates(&self, other: &Self) -> bool {
        self.concept == other.concept
            && self.answer == other.answer
            && question_similarity(&self.question, &other.question) >= 0.8
    }
}

/// Cheap production duplicate predicate shared with the deterministic eval
/// harness so bench duplicate rates match runtime acceptance.
#[must_use]
pub fn candidates_duplicateish(left: &DraftCandidate, right: &DraftCandidate) -> bool {
    CandidateSignature::from_candidate(left).duplicates(&CandidateSignature::from_candidate(right))
}

fn question_similarity(left: &str, right: &str) -> f64 {
    let left = left.split_whitespace().collect::<BTreeSet<_>>();
    let right = right.split_whitespace().collect::<BTreeSet<_>>();
    let union = left.union(&right).count();
    if union == 0 {
        return 1.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        left.intersection(&right).count() as f64 / union as f64
    }
}

fn recent_performance_for_concept(
    snapshot: &BetaStoreSnapshot,
    concept_key: &str,
) -> Vec<ReviewPerformanceContext> {
    snapshot
        .attempts
        .iter()
        .rev()
        .filter(|attempt| {
            snapshot
                .review_units
                .iter()
                .find(|unit| unit.review_unit_id == attempt.review_unit_id)
                .and_then(|unit| unit.queue.concept_key.as_ref())
                .is_some_and(|key| key == concept_key)
        })
        .take(5)
        .map(|attempt| ReviewPerformanceContext {
            review_unit_id: attempt.review_unit_id.to_string(),
            submitted_answer: attempt.submitted_answer.clone(),
            verdict: attempt
                .grade
                .as_ref()
                .map(|grade| format!("{:?}", grade.verdict).to_lowercase()),
        })
        .collect()
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

fn bridge_generated_id(
    prefix: &str,
    kind: &str,
    parent_review_unit_id: &ReviewUnitId,
    candidate: &DraftCandidate,
) -> String {
    [
        prefix.to_owned(),
        kind.to_owned(),
        slug(parent_review_unit_id.as_str()),
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
    use memory_engine_persistence::GeneratedLearningActivityKind;

    use super::{evidence_quote_matches, validation_reasons, DraftCandidate};

    const SOURCE: &str = "Photosynthesis occurs in the chloroplast and converts \
                          light into chemical energy stored as glucose.";

    fn quiz(question: &str, answer: &str, distractors: &[&str]) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: "Test concept".to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: Some("Photosynthesis occurs in the chloroplast".to_owned()),
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
    fn terse_quote_matches_only_when_it_is_the_complete_source() {
        assert!(evidence_quote_matches("Mitochondria", "mitochondria"));
        assert!(evidence_quote_matches("NATO alphabet", "NATO alphabet"));
        assert!(!evidence_quote_matches(
            "Mitochondria generate ATP.",
            "mitochondria"
        ));
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

    #[test]
    fn mcq_quality_rejects_compound_questions() {
        let candidate = quiz(
            "What do mitochondria generate and what process do they regulate?",
            "ATP and apoptosis",
            &["Glucose and mitosis", "RNA and transcription"],
        );

        let reasons = validation_reasons(&candidate, false, true, true);

        assert!(reasons.contains(&"MCQ question tests multiple atoms".to_owned()));
    }

    #[test]
    fn mcq_quality_rejects_answer_duplicate_distractors() {
        let candidate = quiz(
            "Where does photosynthesis occur?",
            "chloroplast",
            &["chloroplast", "mitochondrion"],
        );

        let reasons = validation_reasons(&candidate, false, true, true);

        assert!(reasons.contains(&"MCQ distractor duplicates the correct answer".to_owned()));
    }

    #[test]
    fn self_referential_meta_question_is_rejected() {
        // The exact garbage meta-card observed in dogfood: it asks about the
        // study artifact rather than the subject, so it is unanswerable once the
        // source is gone. The gate must reject it (the repair pass then writes a
        // standalone card) even though it carries a valid quote and no flags.
        let candidate = quiz(
            "What is the name of the phonetic alphabet presented as the subject of the source text?",
            "NATO phonetic alphabet",
            &["ICAO spelling alphabet", "ITU phonetic alphabet"],
        );

        let reasons = validation_reasons(&candidate, false, true, true);

        assert!(
            reasons
                .contains(&"Question references the source instead of standing alone".to_owned()),
            "self-referential meta-question must be rejected: {reasons:?}"
        );
    }

    #[test]
    fn standalone_question_naming_its_subject_is_accepted() {
        // The well-formed sibling: names its subject, mentions no artifact. It
        // must pass cleanly so the gate does not trip on a legitimate card that
        // merely contains words like "text" or "source" in other contexts.
        let candidate = quiz(
            "In the NATO phonetic alphabet, what code word represents the letter A?",
            "Alfa",
            &["Apple", "Alpha-One"],
        );

        let reasons = validation_reasons(&candidate, false, true, true);

        assert!(
            reasons.is_empty(),
            "a self-contained card must not be rejected: {reasons:?}"
        );
    }
}
