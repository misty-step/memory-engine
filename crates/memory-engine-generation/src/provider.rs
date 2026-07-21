//! Provider boundary for draft generation.
//!
//! A [`DraftProvider`] turns one source document into draft candidates plus
//! human-readable failures. Providers own *candidate production* only; the
//! trust gate (provenance verification, duplicate suppression, persistence)
//! stays in [`run_beta_generation_with_provider`](crate::run_beta_generation_with_provider)
//! so every provider's output passes through the same validation. Model-backed
//! providers live in boundary crates outside this one; this crate ships the
//! deterministic providers used by tests, CI, and the structured-block flow.

use std::{error::Error, fmt};

use memory_engine_persistence::{
    GeneratedLearningActivityKind, GeneratedPromptModel, SourceDocument, SourcePermission,
};

/// One generated draft candidate before validation.
///
/// `index` is a stable 1-based position within the source (structured blocks
/// use their block number) and participates in generated draft ids.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftCandidate {
    pub index: usize,
    pub concept: String,
    pub question: String,
    pub answer: String,
    /// Verbatim quote from the source backing this draft. Drafts without
    /// evidence are dropped; drafts whose evidence cannot be found in the
    /// source are persisted as rejected.
    pub evidence: Option<String>,
    pub distractors: Vec<String>,
    pub worked_solution: Option<String>,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub unsupported: bool,
}

/// Token and cost accounting reported by a provider for one source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost in integer micro-USD; `None` when the provider is free/local.
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
}

/// Provider output for one source document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDrafts {
    pub model: GeneratedPromptModel,
    pub learning_intent: Option<LearningIntent>,
    pub candidates: Vec<DraftCandidate>,
    /// Human-readable, per-candidate problems the provider already detected
    /// (for example malformed blocks). Surfaced to the learner alongside
    /// validation failures.
    pub failures: Vec<String>,
    pub usage: Option<ProviderUsage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftRejection {
    pub index: usize,
    pub concept: String,
    pub question: String,
    pub answer: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPerformanceContext {
    pub review_unit_id: String,
    pub submitted_answer: String,
    pub verdict: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceAuthorizationError {
    ArchivedSourceDocument(String),
}

impl fmt::Display for SourceAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArchivedSourceDocument(id) => {
                write!(
                    formatter,
                    "Archived source document cannot be authorized: {id}"
                )
            }
        }
    }
}

impl Error for SourceAuthorizationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedSourceDocument {
    id: String,
    permission: SourcePermission,
}

impl AuthorizedSourceDocument {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Explicit source authorization attached to every reference/bridge request.
///
/// The wrapper is intentionally constructed from active persisted source
/// documents. A low-level provider therefore cannot receive an archived source
/// request without the authorization step failing first.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceAuthorizationContext {
    sources: Vec<AuthorizedSourceDocument>,
}

impl SourceAuthorizationContext {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Authorize source documents for a local or external provider boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when any source is archived.
    pub fn from_sources(sources: &[SourceDocument]) -> Result<Self, SourceAuthorizationError> {
        let mut authorized = Vec::with_capacity(sources.len());
        for source in sources {
            if source.archived_at.is_some() {
                return Err(SourceAuthorizationError::ArchivedSourceDocument(
                    source.id.clone(),
                ));
            }
            authorized.push(AuthorizedSourceDocument {
                id: source.id.clone(),
                permission: source.permission.clone(),
            });
        }
        Ok(Self {
            sources: authorized,
        })
    }

    #[must_use]
    pub fn local_only_source_id(&self) -> Option<&str> {
        self.sources
            .iter()
            .find(|source| source.permission == SourcePermission::LocalOnly)
            .map(AuthorizedSourceDocument::id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceNoteRequest {
    pub concept_key: String,
    pub concept_label: String,
    pub prompt: String,
    pub expected_answer: String,
    pub recent_performance: Vec<ReviewPerformanceContext>,
    authorization: SourceAuthorizationContext,
}

impl ReferenceNoteRequest {
    #[must_use]
    pub fn new(
        concept_key: impl Into<String>,
        concept_label: impl Into<String>,
        prompt: impl Into<String>,
        expected_answer: impl Into<String>,
        recent_performance: Vec<ReviewPerformanceContext>,
        authorization: SourceAuthorizationContext,
    ) -> Self {
        Self {
            concept_key: concept_key.into(),
            concept_label: concept_label.into(),
            prompt: prompt.into(),
            expected_answer: expected_answer.into(),
            recent_performance,
            authorization,
        }
    }

    #[must_use]
    pub fn authorization(&self) -> &SourceAuthorizationContext {
        &self.authorization
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceNoteDraft {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeMaterialRequest {
    pub concept_key: String,
    pub concept_label: String,
    pub parent_review_unit_id: memory_engine_core::ReviewUnitId,
    pub parent_prompt: String,
    pub parent_expected_answer: String,
    pub parent_stage_order: u32,
    pub cached_reference_note: Option<String>,
    pub recent_performance: Vec<ReviewPerformanceContext>,
    authorization: SourceAuthorizationContext,
}

impl BridgeMaterialRequest {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        concept_key: impl Into<String>,
        concept_label: impl Into<String>,
        parent_review_unit_id: memory_engine_core::ReviewUnitId,
        parent_prompt: impl Into<String>,
        parent_expected_answer: impl Into<String>,
        parent_stage_order: u32,
        cached_reference_note: Option<String>,
        recent_performance: Vec<ReviewPerformanceContext>,
        authorization: SourceAuthorizationContext,
    ) -> Self {
        Self {
            concept_key: concept_key.into(),
            concept_label: concept_label.into(),
            parent_review_unit_id,
            parent_prompt: parent_prompt.into(),
            parent_expected_answer: parent_expected_answer.into(),
            parent_stage_order,
            cached_reference_note,
            recent_performance,
            authorization,
        }
    }

    #[must_use]
    pub fn authorization(&self) -> &SourceAuthorizationContext {
        &self.authorization
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeMaterial {
    pub model: GeneratedPromptModel,
    pub reference_note: ReferenceNoteDraft,
    pub candidates: Vec<DraftCandidate>,
    pub usage: Option<ProviderUsage>,
}

/// A transport- or provider-level failure that prevented draft generation.
///
/// The message is shown to learners verbatim, so providers must phrase it as
/// a human sentence, never a debug dump. The `transient` bit records whether an
/// identical retry might succeed (the provider was unreachable, timed out, or
/// returned a 5xx/429) versus a permanent failure (a 4xx rejection, a malformed
/// response) — so a caller can retry the former without re-classifying a string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderFailureKind {
    General,
    LocalOnlySource(String),
    ArchivedSource(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    message: String,
    transient: bool,
    kind: ProviderFailureKind,
}

impl ProviderFailure {
    /// A permanent failure: an identical retry will fail the same way (a 4xx
    /// rejection, a malformed or unreadable response).
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
            kind: ProviderFailureKind::General,
        }
    }

    /// A transient failure: the request never got a usable answer through, so an
    /// identical retry may succeed. Callers may retry these once.
    #[must_use]
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
            kind: ProviderFailureKind::General,
        }
    }

    #[must_use]
    pub fn local_only_source(source_document_id: impl Into<String>) -> Self {
        let source_document_id = source_document_id.into();
        Self {
            message: format!(
                "Local-only source {source_document_id} cannot be sent to the model provider."
            ),
            transient: false,
            kind: ProviderFailureKind::LocalOnlySource(source_document_id),
        }
    }

    #[must_use]
    pub fn archived_source(source_document_id: impl Into<String>) -> Self {
        let source_document_id = source_document_id.into();
        Self {
            message: format!(
                "Archived source {source_document_id} cannot be sent to the model provider."
            ),
            transient: false,
            kind: ProviderFailureKind::ArchivedSource(source_document_id),
        }
    }

    #[must_use]
    pub fn kind(&self) -> &ProviderFailureKind {
        &self.kind
    }

    /// Whether an identical retry might succeed. See [`ProviderFailure::transient`].
    #[must_use]
    pub fn is_transient(&self) -> bool {
        self.transient
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ProviderFailure {}

/// Produces draft candidates from one source document.
pub trait DraftProvider {
    /// Identity recorded on generation runs, available before any generation
    /// happens so failed runs still carry a model receipt.
    fn model(&self) -> GeneratedPromptModel;

    /// Generate draft candidates for `source`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] when the provider cannot produce any
    /// output for the source (transport failure, malformed response). The
    /// message is surfaced to the learner.
    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure>;

    /// Regenerate draft candidates once after the shared trust gate rejected
    /// one or more first-pass candidates for a source.
    ///
    /// Providers that cannot use rejection feedback return `Ok(None)`. The
    /// generation runner still owns validation and persistence for repaired
    /// candidates.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] when a provider-specific repair request
    /// fails after it has chosen to attempt repair.
    fn repair_drafts(
        &self,
        _source: &SourceDocument,
        _rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        Ok(None)
    }
}

/// Produces a short concept-level explanation when no source span exists.
pub trait ReferenceNoteProvider {
    fn model(&self) -> GeneratedPromptModel;

    /// Generate one short note for the concept behind a review item.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] when the provider cannot produce a note.
    fn explain_concept(
        &self,
        request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure>;
}

/// Produces easier bridge material for a struggling parent item.
pub trait BridgeMaterialProvider: ReferenceNoteProvider {
    /// Generate a reference note plus easier draft candidates for the parent.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] when the provider cannot produce bridge material.
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearningIntent {
    VerbatimMemorization,
    EnumerableSet,
    ConceptUnderstanding,
    FactRecall,
    ProcedureProcess,
}

impl LearningIntent {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::VerbatimMemorization => "verbatim_memorization",
            Self::EnumerableSet => "enumerable_set",
            Self::ConceptUnderstanding => "concept_understanding",
            Self::FactRecall => "fact_recall",
            Self::ProcedureProcess => "procedure_process",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label.trim() {
            "verbatim_memorization" => Some(Self::VerbatimMemorization),
            "enumerable_set" => Some(Self::EnumerableSet),
            "concept_understanding" => Some(Self::ConceptUnderstanding),
            "fact_recall" => Some(Self::FactRecall),
            "procedure_process" => Some(Self::ProcedureProcess),
            _ => None,
        }
    }
}

impl fmt::Display for LearningIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningIntentClassification {
    pub intent: LearningIntent,
    pub rationale: String,
}

#[must_use]
pub fn classify_learning_intent(source: &SourceDocument) -> LearningIntentClassification {
    let body = source.body.as_deref().unwrap_or_default();
    let normalized = format!("{} {}", source.title, body).to_lowercase();
    let lines = non_empty_lines(body);
    let list_facts = count_fact_sentences(body);
    let enumerable = looks_enumerable(body, &lines);
    let process = looks_process(&normalized);
    let explicit_verbatim = looks_explicit_verbatim(&normalized);
    let ordered_process = looks_ordered_process(&normalized, &lines);
    if explicit_verbatim {
        return LearningIntentClassification {
            intent: LearningIntent::VerbatimMemorization,
            rationale: "source explicitly calls for exact sequential memorization".to_owned(),
        };
    }
    // A numbered procedure is both list-shaped and process-shaped. Let the
    // strong ordered-action signal win that overlap, while keeping ordinary
    // line-broken verse on the verbatim path and finite reference sets
    // enumerable even when they mention weak process words such as "first".
    if enumerable && ordered_process {
        return LearningIntentClassification {
            intent: LearningIntent::ProcedureProcess,
            rationale: "source describes ordered actions or conditional steps".to_owned(),
        };
    }
    if enumerable {
        return LearningIntentClassification {
            intent: LearningIntent::EnumerableSet,
            rationale: "source contains a finite set of independently recallable entries"
                .to_owned(),
        };
    }
    if looks_verbatim(&normalized, &lines) {
        return LearningIntentClassification {
            intent: LearningIntent::VerbatimMemorization,
            rationale: "source reads like a quoted passage or line-broken text".to_owned(),
        };
    }
    if process {
        return LearningIntentClassification {
            intent: LearningIntent::ProcedureProcess,
            rationale: "source describes ordered actions or conditional steps".to_owned(),
        };
    }
    if looks_concept(&normalized) {
        return LearningIntentClassification {
            intent: LearningIntent::ConceptUnderstanding,
            rationale: "source emphasizes causes, mechanisms, or explanatory ideas".to_owned(),
        };
    }
    if list_facts >= 2 || looks_like_tiny_fact(&normalized) {
        return LearningIntentClassification {
            intent: LearningIntent::FactRecall,
            rationale: "source is dominated by discrete facts".to_owned(),
        };
    }

    LearningIntentClassification {
        intent: LearningIntent::ConceptUnderstanding,
        rationale: "source asks for explanation or conceptual retention".to_owned(),
    }
}

/// Deterministic provider that parses `Concept:/Question:/Answer:` blocks.
///
/// This is the original beta generation behavior, now one provider among
/// several.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructuredBlockProvider;

impl DraftProvider for StructuredBlockProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "deterministic-beta-generator".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let mut failures = Vec::new();
        let candidates = parse_source_document(source, &mut failures);

        Ok(ProviderDrafts {
            model: self.model(),
            learning_intent: Some(classify_learning_intent(source).intent),
            candidates,
            failures,
            usage: None,
        })
    }
}

/// Routes structured blocks through the deterministic parser and falls back
/// to a secondary provider only when the parser produces no candidates.
///
/// Keeping the structured parser implicit makes the routing invariant part of
/// the type: repair may safely re-run this free, deterministic parse to recover
/// which provider handled the first pass.
pub struct FallbackProvider<'a> {
    fallback: &'a dyn DraftProvider,
}

impl<'a> FallbackProvider<'a> {
    #[must_use]
    pub fn new(fallback: &'a dyn DraftProvider) -> Self {
        Self { fallback }
    }
}

fn ensure_external_source_allowed(source: &SourceDocument) -> Result<(), ProviderFailure> {
    if source.archived_at.is_some() {
        Err(ProviderFailure::archived_source(source.id.clone()))
    } else if source.permission == SourcePermission::LocalOnly {
        Err(ProviderFailure::local_only_source(source.id.clone()))
    } else {
        Ok(())
    }
}

impl DraftProvider for FallbackProvider<'_> {
    fn model(&self) -> GeneratedPromptModel {
        self.fallback.model()
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        ensure_external_source_allowed(source)?;
        let primary = StructuredBlockProvider.generate_drafts(source)?;
        if primary.candidates.is_empty() {
            self.fallback.generate_drafts(source)
        } else {
            Ok(primary)
        }
    }

    fn repair_drafts(
        &self,
        source: &SourceDocument,
        rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        ensure_external_source_allowed(source)?;
        // Repair must stay on the provider that handled the first pass.
        // Re-run the deterministic primary router: falling through merely
        // because the primary has no repair implementation would let model
        // output replace rejected structured material.
        if StructuredBlockProvider
            .generate_drafts(source)?
            .candidates
            .is_empty()
        {
            self.fallback.repair_drafts(source, rejections)
        } else {
            StructuredBlockProvider.repair_drafts(source, rejections)
        }
    }
}

/// Deterministic stand-in for a model provider, used by CI and tests.
///
/// Emits one short-answer draft per leading sentence (up to three) with the
/// sentence itself as verbatim evidence, so drafts always pass the
/// provenance gate without any network access.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeModelProvider;

impl DraftProvider for FakeModelProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "fake-model".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let body = source.body.clone().unwrap_or_default();
        let classification = classify_learning_intent(source);
        let candidates = match classification.intent {
            LearningIntent::VerbatimMemorization => verbatim_candidates(source, &body),
            LearningIntent::EnumerableSet => enumerable_candidates(source, &body),
            LearningIntent::ConceptUnderstanding => concept_candidates(source, &body),
            LearningIntent::FactRecall => fact_candidates(source, &body),
            LearningIntent::ProcedureProcess => procedure_candidates(source, &body),
        };

        Ok(ProviderDrafts {
            model: DraftProvider::model(self),
            learning_intent: Some(classification.intent),
            candidates,
            failures: Vec::new(),
            usage: None,
        })
    }
}

impl ReferenceNoteProvider for FakeModelProvider {
    fn model(&self) -> GeneratedPromptModel {
        DraftProvider::model(self)
    }

    fn explain_concept(
        &self,
        request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        Ok(ReferenceNoteDraft {
            title: format!("Reference: {}", request.concept_label),
            body: format!(
                "{} connects the prompt \"{}\" to the expected answer \"{}\". \
                 Start by recognizing the smaller cue, then answer the original item.",
                request.concept_label, request.prompt, request.expected_answer
            ),
        })
    }
}

impl BridgeMaterialProvider for FakeModelProvider {
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        let note = request.cached_reference_note.as_ref().map_or_else(
            || {
                self.explain_concept(&ReferenceNoteRequest::new(
                    request.concept_key.clone(),
                    request.concept_label.clone(),
                    request.parent_prompt.clone(),
                    request.parent_expected_answer.clone(),
                    request.recent_performance.clone(),
                    request.authorization().clone(),
                ))
            },
            |body| {
                Ok(ReferenceNoteDraft {
                    title: format!("Reference: {}", request.concept_label),
                    body: body.clone(),
                })
            },
        )?;
        let answer = request.parent_expected_answer.clone();
        let cue = lead_words(&answer, 5);

        Ok(BridgeMaterial {
            model: ReferenceNoteProvider::model(self),
            reference_note: note,
            candidates: vec![
                DraftCandidate {
                    index: 1,
                    concept: request.concept_label.clone(),
                    question: format!(
                        "Which smaller cue helps with \"{}\"?",
                        request.parent_prompt
                    ),
                    answer: cue.clone(),
                    evidence: None,
                    distractors: vec![
                        "A different source detail".to_owned(),
                        "An unrelated answer".to_owned(),
                    ],
                    worked_solution: None,
                    activity_kind: GeneratedLearningActivityKind::Quiz,
                    activity_stage: "recognition-bridge".to_owned(),
                    unsupported: false,
                },
                DraftCandidate {
                    index: 2,
                    concept: request.concept_label.clone(),
                    question: format!(
                        "Use the cue \"{cue}\" to answer the original item in one step."
                    ),
                    answer,
                    evidence: None,
                    distractors: Vec::new(),
                    worked_solution: Some(format!(
                        "The bridge cue points back to: {}",
                        request.parent_expected_answer
                    )),
                    activity_kind: GeneratedLearningActivityKind::Exercise,
                    activity_stage: "cued-recall-bridge".to_owned(),
                    unsupported: false,
                },
            ],
            usage: None,
        })
    }
}

fn looks_verbatim(normalized: &str, lines: &[String]) -> bool {
    looks_explicit_verbatim(normalized)
        || (lines.len() >= 3
            && lines.iter().all(|line| !line.contains(':'))
            && lines
                .iter()
                .filter(|line| line.chars().count() <= 96)
                .count()
                >= 3)
}

/// Checks whether `word` appears as a whole token in `haystack`, splitting on
/// any non-alphanumeric byte. Plain `str::contains` would let compound words
/// such as "universe", "diverse", or "quoted" trip on "verse"/"quote" and
/// silently convert ordinary conceptual prose into a recitation exercise.
fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == word)
}

fn looks_explicit_verbatim(normalized: &str) -> bool {
    [
        "recite", "memorize", "verbatim", "poem", "oath", "creed", "excerpt", "verse", "quote",
    ]
    .iter()
    .any(|keyword| contains_word(normalized, keyword))
}

/// Apply deterministic content policy after a provider has classified a source.
///
/// Finite sets and sequential passages are safety-critical coverage cases: a
/// model may omit entries or collapse a passage into a sampled quiz. The
/// source itself is the authority for these two shapes, so replace only those
/// candidates with exact, source-grounded drafts. Conceptual and ordinary fact
/// generation remains provider-owned and therefore keeps its fewer-better
/// behavior.
#[must_use]
pub fn enforce_content_policy(
    source: &SourceDocument,
    mut drafts: ProviderDrafts,
) -> ProviderDrafts {
    let classification = classify_learning_intent(source);
    let body = source.body.as_deref().unwrap_or_default();
    match classification.intent {
        LearningIntent::EnumerableSet => {
            drafts.learning_intent = Some(classification.intent);
            drafts.candidates =
                enumerable_candidates(source, source.body.as_deref().unwrap_or_default());
            assert_exhaustive_indices(&drafts.candidates);
            // The provider's own per-candidate failures described candidates
            // that no longer exist once the policy replaced them wholesale;
            // carrying them forward would misreport diagnostics against a
            // fully exhaustive, policy-owned draft set.
            drafts.failures.clear();
        }
        // The legacy 047/084 ordinal prose fixture remains fact-labelled for
        // intent-shape parity, but its source-owned coverage oracle is still a
        // finite ordinal set and must receive the complete enumerable drafts.
        // A two-entry ordinal mapping (e.g. a binary on/off toggle) is still a
        // finite, non-derivable set and deserves the same exhaustive coverage
        // as three-or-more entries.
        LearningIntent::FactRecall
            if is_definitive_mapping(body, ordinal_mapping_entries(body).len()) =>
        {
            drafts.candidates = enumerable_candidates(source, body);
            assert_exhaustive_indices(&drafts.candidates);
            drafts.failures.clear();
        }
        LearningIntent::VerbatimMemorization => {
            drafts.learning_intent = Some(classification.intent);
            drafts.candidates =
                verbatim_candidates(source, source.body.as_deref().unwrap_or_default());
            assert_exhaustive_indices(&drafts.candidates);
            drafts.failures.clear();
        }
        LearningIntent::ConceptUnderstanding
        | LearningIntent::FactRecall
        | LearningIntent::ProcedureProcess => {}
    }
    drafts
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnumerableEntry {
    cue: String,
    answer: String,
    evidence: String,
}

/// A finite key-to-value mapping (e.g. "0 is off. 1 is on.") is a
/// non-derivable set even with just two entries. But `mapping_entries`
/// splits on every '.', so a two-entry match can also be an incidental
/// fragment of an unrelated sentence buried inside a much larger,
/// already-structured document (for example a "Reference: C is CHARLIE. A
/// is ALFA." annotation line inside an authored multi-block source). Three
/// or more entries are unambiguous regardless of body size; exactly two are
/// trusted only when the whole body is small enough that the mapping is
/// plausibly the entire point of the source.
const TWO_ENTRY_MAPPING_BODY_WORD_LIMIT: usize = 40;

fn is_definitive_mapping(body: &str, entry_count: usize) -> bool {
    entry_count >= 3
        || (entry_count == 2
            && body.split_whitespace().count() <= TWO_ENTRY_MAPPING_BODY_WORD_LIMIT)
}

fn looks_enumerable(body: &str, lines: &[String]) -> bool {
    is_definitive_mapping(body, mapping_entries(body).len()) || list_entries(lines).len() >= 3
}

fn enumerable_entries(body: &str) -> Vec<EnumerableEntry> {
    let ordinal_mappings = ordinal_mapping_entries(body);
    if is_definitive_mapping(body, ordinal_mappings.len()) {
        return ordinal_mappings;
    }

    let mappings = mapping_entries(body);
    if is_definitive_mapping(body, mappings.len()) {
        return mappings;
    }

    list_entries(&non_empty_lines(body))
        .into_iter()
        .enumerate()
        .map(|(position, (answer, _evidence))| EnumerableEntry {
            cue: (position + 1).to_string(),
            answer,
            // A one-line entry such as `1. Alpha` is too short for the
            // production provenance floor. Cite the complete source list so
            // the draft remains grounded without weakening that trust gate.
            evidence: body.trim().to_owned(),
        })
        .collect()
}

fn ordinal_mapping_entries(body: &str) -> Vec<EnumerableEntry> {
    numbered_segments(body)
        .into_iter()
        .filter_map(|segment| {
            let (cue, rest) = segment.split_once(". ")?;
            let (answer, _fact) = rest.split_once(" is ")?;
            Some(EnumerableEntry {
                cue: cue.trim().to_owned(),
                answer: answer.trim().to_owned(),
                evidence: segment,
            })
        })
        .collect()
}

fn numbered_segments(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut starts = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let is_boundary = position == 0 || bytes[position - 1].is_ascii_whitespace();
        if is_boundary && bytes[position].is_ascii_digit() {
            let mut end = position;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end + 1 < bytes.len() && bytes[end] == b'.' && bytes[end + 1].is_ascii_whitespace() {
                starts.push(position);
            }
            position = end;
        } else {
            position += 1;
        }
    }

    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(bytes.len());
            body[*start..end].trim().to_owned()
        })
        .collect()
}

fn mapping_entries(body: &str) -> Vec<EnumerableEntry> {
    body.split(['.', '?', '!', '\n'])
        .filter_map(|raw| {
            let evidence = raw.trim().to_owned();
            let (cue, answer) = raw.split_once(" is ")?;
            let cue = cue.trim();
            let answer = answer.trim().trim_matches(|character: char| {
                character == ':' || character == ';' || character == ','
            });
            if cue.chars().count() != 1 || answer.is_empty() {
                return None;
            }
            Some(EnumerableEntry {
                cue: cue.to_owned(),
                answer: answer.to_owned(),
                evidence,
            })
        })
        .collect()
}

fn list_entries(lines: &[String]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if let Some(answer) = trimmed
            .strip_prefix('-')
            .or_else(|| trimmed.strip_prefix('*'))
            .or_else(|| trimmed.strip_prefix('•'))
            .map(str::trim)
            .filter(|answer| !answer.is_empty())
        {
            entries.push((answer.to_owned(), trimmed.to_owned()));
            continue;
        }
        let Some((prefix, answer)) = trimmed.split_once(['.', ')', ':']) else {
            continue;
        };
        if prefix.trim().parse::<usize>().is_ok() && !answer.trim().is_empty() {
            entries.push((answer.trim().to_owned(), trimmed.to_owned()));
        }
    }
    entries
}

fn assert_exhaustive_indices(candidates: &[DraftCandidate]) {
    for (position, candidate) in candidates.iter().enumerate() {
        debug_assert_eq!(
            candidate.index,
            position + 1,
            "content-policy candidates must cover sequential indices [1..N]"
        );
    }
}

fn enumerable_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    enumerable_entries(body)
        .into_iter()
        .enumerate()
        .map(|(position, entry)| {
            let question = if entry.cue.chars().count() == 1
                && entry.cue.chars().next().is_some_and(char::is_alphabetic)
            {
                format!(
                    "In {}, which word stands for the letter {}?",
                    source.title, entry.cue
                )
            } else {
                format!(
                    "In {}, what is the entry for number {}?",
                    source.title, entry.cue
                )
            };
            DraftCandidate {
                index: position + 1,
                concept: format!("{}: {}", source.title, entry.cue),
                question,
                answer: entry.answer.clone(),
                evidence: Some(entry.evidence),
                distractors: Vec::new(),
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "production-recall".to_owned(),
                unsupported: false,
            }
        })
        .collect()
}

fn looks_process(normalized: &str) -> bool {
    [
        "to maintain",
        "first",
        "then",
        "finally",
        "step",
        "process",
        "procedure",
        "always ",
        " once every ",
        "if it ",
        "before ",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn looks_ordered_process(normalized: &str, lines: &[String]) -> bool {
    [
        "step",
        "steps",
        "procedure",
        "process",
        "recipe",
        "instruction",
        "instructions",
        "workflow",
        "how to ",
        "follow these",
        "in order",
        "sequence",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
        || looks_like_imperative_sequence(lines)
}

fn looks_like_imperative_sequence(lines: &[String]) -> bool {
    let entries = list_entries(lines);
    entries.len() >= 3
        && entries
            .iter()
            .all(|(answer, _)| starts_with_action_verb(answer))
}

fn starts_with_action_verb(answer: &str) -> bool {
    let Some(first_word) = answer.split_whitespace().next() else {
        return false;
    };
    let first_word = first_word.trim_matches(|character: char| !character.is_alphabetic());
    [
        "add", "bake", "boil", "bring", "chop", "choose", "clean", "click", "close", "combine",
        "cook", "create", "cut", "discard", "feed", "fill", "fold", "gather", "heat", "insert",
        "install", "knead", "let", "load", "make", "measure", "mix", "open", "place", "pour",
        "preheat", "press", "remove", "repeat", "rinse", "run", "save", "select", "serve", "set",
        "start", "stir", "stop", "take", "turn", "use", "whisk", "write",
    ]
    .iter()
    .any(|verb| *verb == first_word.to_ascii_lowercase())
}

fn looks_concept(normalized: &str) -> bool {
    [
        " because ",
        " theory",
        " effect",
        " why ",
        "mechanism",
        "regulate",
        "descend",
        "principle",
        "algorithm",
        "system design",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn looks_like_tiny_fact(normalized: &str) -> bool {
    normalized.split_whitespace().count() <= 28
        || normalized.contains(" is ")
        || normalized.contains(" are ")
        || normalized.contains(" means ")
}

fn count_fact_sentences(body: &str) -> usize {
    split_sentences(body)
        .iter()
        .filter(|sentence| {
            let normalized = sentence.to_lowercase();
            normalized.contains(" is ")
                || normalized.contains(" are ")
                || normalized.contains(" means ")
                || normalized.contains(" assigns ")
        })
        .count()
}

fn verbatim_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let units = sequential_units(body);
    let source_evidence = body.trim().to_owned();
    units
        .iter()
        .enumerate()
        .map(|(position, unit)| {
            let (question, activity_stage) = if position == 0 {
                (
                    format!("Recite the opening line of {} exactly.", source.title),
                    "free-recall",
                )
            } else {
                (
                    format!("Recite the next line after: {}", units[position - 1]),
                    "cued-recall",
                )
            };
            DraftCandidate {
                index: position + 1,
                concept: format!("{} line {}", source.title, position + 1),
                question,
                answer: unit.clone(),
                // Keep the exact unit as the answer, but cite the complete
                // source so short verse lines clear the production trust
                // floor without weakening evidence_quote_matches.
                evidence: Some(source_evidence.clone()),
                distractors: Vec::new(),
                worked_solution: Some(format!("The exact source line is: {unit}")),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: activity_stage.to_owned(),
                unsupported: false,
            }
        })
        .collect()
}

fn sequential_units(body: &str) -> Vec<String> {
    let lines = non_empty_lines(body);
    if lines.len() >= 2 {
        return lines;
    }

    let units = body
        .split_inclusive(['.', '?', '!'])
        .map(str::trim)
        .filter(|unit| !unit.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if units.is_empty() && !body.trim().is_empty() {
        vec![body.trim().to_owned()]
    } else {
        units
    }
}

fn concept_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    split_sentences(body)
        .into_iter()
        .filter(|sentence| sentence.split_whitespace().count() >= 6)
        .take(3)
        .enumerate()
        .map(|(position, sentence)| DraftCandidate {
            index: position + 1,
            concept: format!("{} concept {}", source.title, position + 1),
            question: format!(
                "Explain the idea from \"{}\" in your own words: {}",
                source.title,
                lead_words(&sentence, 8)
            ),
            answer: sentence.clone(),
            evidence: Some(sentence),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "free-recall".to_owned(),
            unsupported: false,
        })
        .collect()
}

fn fact_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let facts = split_sentences(body)
        .into_iter()
        .filter(|sentence| {
            let normalized = sentence.to_lowercase();
            normalized.contains(" is ")
                || normalized.contains(" are ")
                || normalized.contains(" means ")
                || normalized.contains(" assigns ")
        })
        .take(6)
        .collect::<Vec<_>>();
    let facts = if facts.is_empty() {
        let sentences = split_sentences(body)
            .into_iter()
            .take(3)
            .collect::<Vec<_>>();
        if sentences.is_empty() && !body.trim().is_empty() {
            vec![body.trim().to_owned()]
        } else {
            sentences
        }
    } else {
        facts
    };

    let fact_rows = facts
        .into_iter()
        .map(|sentence| {
            let (question, answer) = fact_question_answer(source, &sentence);
            (question, answer, sentence)
        })
        .collect::<Vec<_>>();
    let fact_count = fact_rows.len();
    let mut candidates = fact_rows
        .iter()
        .enumerate()
        .map(|(position, (question, answer, sentence))| {
            let distractors = grounded_fact_distractors(&fact_rows, answer);
            DraftCandidate {
                index: position + 1,
                concept: format!("{} fact {}", source.title, position + 1),
                question: question.clone(),
                answer: answer.clone(),
                evidence: Some(sentence.clone()),
                distractors,
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "recognition".to_owned(),
                unsupported: false,
            }
        })
        .collect::<Vec<_>>();

    if (1..=2).contains(&fact_count) {
        if let Some(first) = candidates.first() {
            let concept = first.concept.clone();
            let answer = first.answer.clone();
            let evidence = first.evidence.clone();
            let distractors = first.distractors.clone();
            let activity_stage = first.activity_stage.clone();
            candidates.push(DraftCandidate {
                index: fact_count + 1,
                concept,
                question: format!(
                    "Which source fact answers a second wording about \"{}\"?",
                    source.title
                ),
                answer,
                evidence,
                distractors,
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage,
                unsupported: false,
            });
        }
    }

    candidates
}

fn grounded_fact_distractors(facts: &[(String, String, String)], answer: &str) -> Vec<String> {
    let distractors = facts
        .iter()
        .map(|(_, fact_answer, _)| fact_answer.trim())
        .filter(|fact_answer| !fact_answer.is_empty() && *fact_answer != answer.trim())
        .take(2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if distractors.len() >= 2 {
        distractors
    } else {
        Vec::new()
    }
}

fn procedure_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let lines = non_empty_lines(body);
    let ordered_entries = list_entries(&lines);
    if ordered_entries.len() >= 3 {
        let source_evidence = body.trim().to_owned();
        return ordered_entries
            .into_iter()
            .enumerate()
            .map(|(position, (answer, _entry_evidence))| DraftCandidate {
                index: position + 1,
                concept: format!("{} procedure step {}", source.title, position + 1),
                question: format!(
                    "What is ordered action {} in the {} procedure?",
                    position + 1,
                    source.title
                ),
                answer,
                evidence: Some(source_evidence.clone()),
                distractors: Vec::new(),
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "procedure-composition".to_owned(),
                unsupported: false,
            })
            .collect();
    }

    let sentences = split_sentences(body);
    let source_evidence = body.trim().to_owned();
    sentences
        .into_iter()
        .enumerate()
        .map(|(position, sentence)| DraftCandidate {
            index: position + 1,
            concept: format!("{} procedure step {}", source.title, position + 1),
            question: if position == 0 {
                format!("Describe the process in \"{}\" in order.", source.title)
            } else {
                format!("What further step matters in \"{}\"?", source.title)
            },
            answer: sentence,
            evidence: Some(source_evidence.clone()),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: if position == 0 {
                "procedure-composition".to_owned()
            } else {
                "procedure-check".to_owned()
            },
            unsupported: false,
        })
        .collect()
}

fn fact_question_answer(source: &SourceDocument, sentence: &str) -> (String, String) {
    for separator in [" is ", " are ", " means "] {
        if let Some((left, right)) = sentence.split_once(separator) {
            let subject = left.trim();
            let answer = right.trim().to_owned();
            return (
                format!("According to \"{}\", what is {subject}?", source.title),
                answer,
            );
        }
    }
    (
        format!("What fact should you recall from \"{}\"?", source.title),
        sentence.to_owned(),
    )
}

fn split_sentences(body: &str) -> Vec<String> {
    body.split(['.', '?', '!'])
        .map(str::trim)
        .filter(|sentence| sentence.split_whitespace().count() >= 3)
        .map(str::to_owned)
        .collect()
}

fn non_empty_lines(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn lead_words(sentence: &str, count: usize) -> String {
    sentence
        .split_whitespace()
        .take(count)
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_source_document(
    source: &SourceDocument,
    validation_failures: &mut Vec<String>,
) -> Vec<DraftCandidate> {
    structured_blocks(source.body.as_deref().unwrap_or_default())
        .enumerate()
        .filter_map(|(index, block)| {
            parse_candidate_block(source, &block, index + 1, validation_failures)
        })
        .collect()
}

fn structured_blocks(body: &str) -> impl Iterator<Item = String> + '_ {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks.into_iter()
}

fn parse_candidate_block(
    source: &SourceDocument,
    block: &str,
    block_index: usize,
    validation_failures: &mut Vec<String>,
) -> Option<DraftCandidate> {
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

    Some(DraftCandidate {
        index: block_index,
        activity_kind: parse_activity_kind(fields.iter().find_value("activity")),
        activity_stage: fields
            .iter()
            .find_value("stage")
            .unwrap_or("recognition")
            .to_owned(),
        concept: concept.to_owned(),
        question: question.to_owned(),
        answer: answer.to_owned(),
        evidence: fields.iter().find_value("reference").map(str::to_owned),
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
