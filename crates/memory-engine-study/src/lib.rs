//! Beta-study session orchestration.
//!
//! This crate owns the repo-local beta-study workflow: source intake,
//! deterministic draft generation, draft approval, reveal state, grading, and
//! queue advancement. It composes the generation, persistence, and service
//! crates without moving filesystem, HTTP, or UI concerns into the pure core.

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::PathBuf,
    rc::Rc,
};

use memory_engine_core::{
    pick_next_queue_candidate, reviewable_queue_candidates, GradeResult, Prompt, QueueCandidate,
    QueueSelectionOptions, ReviewUnitId, ReviewUnitLifecycle, ScheduleState, ScheduleStatus,
    Verdict,
};
use memory_engine_generation::{
    run_beta_generation, run_beta_generation_with_provider, run_bridge_generation,
    run_bridge_generation_with_provider, BetaGenerationError, BetaGenerationRequest,
    BetaGenerationStore, BridgeGenerationRequest, BridgeMaterialProvider, DraftProvider,
    FakeModelProvider, ReferenceNoteProvider, ReferenceNoteRequest, SourceAuthorizationContext,
    SourceAuthorizationError,
};
use memory_engine_persistence::{
    BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreError, BetaStoreSnapshot,
    ConceptReferenceNote, GeneratedLearningActivityKind, GeneratedPromptDraft,
    GeneratedPromptValidationStatus, LearnerDraftDecision, SourceDocument, SourceDocumentKind,
};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, ServiceError,
};
use serde::{Deserialize, Serialize};

pub use memory_engine_persistence::SourcePermission;

pub const DEFAULT_BETA_STUDY_NOW: i64 = 1_779_465_600_000;
pub const DEFAULT_SKIP_DEFER_MS: i64 = 15 * 60 * 1_000;
pub const DEFAULT_SNOOZE_DEFER_MS: i64 = 86_400_000;
pub const DEFAULT_BRIDGE_PARENT_DEFER_MS: i64 = 60 * 60 * 1_000;

#[derive(Clone, Debug)]
pub struct BetaStudyOptions {
    pub path: PathBuf,
    pub now: fn() -> i64,
}

impl BetaStudyOptions {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            now: || DEFAULT_BETA_STUDY_NOW,
        }
    }

    #[must_use]
    pub fn with_clock(mut self, now: fn() -> i64) -> Self {
        self.now = now;
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BetaStudyStatus {
    Empty,
    Drafting,
    Answering,
    Revealed,
    Graded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudySourceInput {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_key: Option<String>,
    pub ttl_expires_at: Option<i64>,
    pub permission: SourcePermission,
}

impl BetaStudySourceInput {
    #[must_use]
    pub fn from_capture(id: impl Into<String>, body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            id: id.into(),
            title: infer_capture_title(&body),
            body,
            project_key: None,
            ttl_expires_at: None,
            permission: SourcePermission::ModelEligible,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyView {
    pub status: BetaStudyStatus,
    pub sources: Vec<BetaStudySourceRow>,
    pub drafts: Vec<BetaStudyDraftRow>,
    pub queue: Vec<BetaStudyQueueRow>,
    pub due_count: usize,
    pub current: Option<BetaStudyCurrent>,
    pub concept_progress: Vec<BetaStudyConceptProgress>,
    pub summary: BetaStudySummary,
    pub api_pressure: Vec<String>,
    /// Human-readable explanations for the most recent generation run:
    /// provider failures, rejected drafts, and an empty-result message when a
    /// run produced no drafts. Empty when the last run yielded drafts cleanly.
    pub generation_notices: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudySourceRow {
    pub id: String,
    pub title: String,
    pub kind: SourceDocumentKind,
    pub permission: SourcePermission,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyReferenceSpanRow {
    pub id: String,
    pub source_document_id: String,
    pub label: String,
    pub text: String,
    pub locator: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyGenerationProvenance {
    pub generation_run_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub prompt_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyDraftRow {
    pub id: String,
    pub review_unit_id: ReviewUnitId,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub prompt: String,
    pub answer: String,
    pub concept_label: String,
    pub validation_status: GeneratedPromptValidationStatus,
    pub validation_reasons: Vec<String>,
    pub worked_solution: Option<String>,
    pub approved: bool,
    pub learner_decision: Option<LearnerDraftDecision>,
    pub source_spans: Vec<BetaStudyReferenceSpanRow>,
    pub provenance: Option<BetaStudyGenerationProvenance>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyQueueRow {
    pub review_unit_id: ReviewUnitId,
    pub due: i64,
    pub reps: u32,
    pub state: Option<ScheduleStatus>,
    pub activity_kind: Option<GeneratedLearningActivityKind>,
    pub activity_stage: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyCurrent {
    pub review_unit_id: ReviewUnitId,
    pub concept_key: Option<String>,
    pub prompt_id: String,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub prompt: String,
    pub choices: Vec<String>,
    pub revision_expected_answer: String,
    pub expected_answer: Option<String>,
    pub reference_text: Option<String>,
    pub worked_solution: Option<String>,
    pub grade: Option<BetaStudyGrade>,
    pub review_state: Option<ReviewStateProjection>,
    pub schedule_change: Option<ScheduleChange>,
    pub feedback: Option<BetaStudyFeedback>,
    pub content_feedback_head_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyGrade {
    pub verdict: Verdict,
    pub rating: memory_engine_core::Rating,
    pub is_correct: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReviewStateProjection {
    pub due: i64,
    pub reps: u32,
    pub lapses: u32,
    pub state: ScheduleStatus,
    pub last_review: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleChange {
    pub before: Option<ReviewStateProjection>,
    pub after: ReviewStateProjection,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyFeedback {
    pub verdict: String,
    pub expected_answer: String,
    pub item_history: BetaStudyItemHistory,
    pub concept_progress: Option<BetaStudyConceptProgress>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyItemHistory {
    pub attempts: usize,
    pub correct: usize,
    pub success_rate: String,
    pub trend: String,
    pub last_seen: Option<i64>,
    pub last_seen_summary: String,
    pub last_response_time_ms: Option<u32>,
    pub average_response_time_ms: Option<u32>,
    pub response_time_trend: String,
    pub stage: String,
    pub next_review: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyConceptProgress {
    pub concept_key: String,
    pub concept_label: String,
    pub attempts: usize,
    pub correct: usize,
    pub success_rate: String,
    pub trend: String,
    pub average_response_time_ms: Option<u32>,
    pub response_time_trend: String,
    pub health: String,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudySummary {
    pub source_count: usize,
    pub accepted_draft_count: usize,
    pub approved_review_unit_count: usize,
    pub attempt_count: usize,
    pub last_outcome: Option<Verdict>,
    pub next_review_unit_id: Option<ReviewUnitId>,
}

#[derive(Debug, PartialEq)]
pub enum BetaStudyError<E = BetaStoreError> {
    Store(E),
    Generation(BetaGenerationError<E>),
    Service(ServiceError<E>),
    UnknownReferenceSpan(String),
    NoActiveReviewUnit,
    NoConceptKey,
}

impl<E> fmt::Display for BetaStudyError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::Generation(error) => write!(formatter, "generation error: {error}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
            Self::UnknownReferenceSpan(id) => write!(formatter, "Unknown reference span: {id}"),
            Self::NoActiveReviewUnit => {
                formatter.write_str("Beta study session has no active review unit")
            }
            Self::NoConceptKey => formatter.write_str("The active review unit has no concept key"),
        }
    }
}

impl<E> Error for BetaStudyError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Generation(error) => Some(error),
            Self::Service(error) => Some(error),
            Self::UnknownReferenceSpan(_) | Self::NoActiveReviewUnit | Self::NoConceptKey => None,
        }
    }
}

impl From<BetaStoreError> for BetaStudyError<BetaStoreError> {
    fn from(error: BetaStoreError) -> Self {
        Self::Store(error)
    }
}

impl<E> From<BetaGenerationError<E>> for BetaStudyError<E> {
    fn from(error: BetaGenerationError<E>) -> Self {
        Self::Generation(error)
    }
}

impl<E> From<ServiceError<E>> for BetaStudyError<E> {
    fn from(error: ServiceError<E>) -> Self {
        Self::Service(error)
    }
}

pub trait BetaStudyStore:
    BetaGenerationStore<Error = <Self as MemoryServiceStore>::Error> + MemoryServiceStore
{
    /// Save source material for later generation.
    ///
    /// # Errors
    ///
    /// Returns the store error when the source is rejected or cannot be persisted.
    fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error>;

    /// Hide source material from learner-facing generation and study views.
    ///
    /// # Errors
    ///
    /// Returns the store error when the source is unknown or cannot be archived.
    fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error>;

    /// Update an active source's model-sharing permission.
    ///
    /// # Errors
    ///
    /// Returns the store error when the source is unknown, archived, or cannot
    /// be persisted.
    fn update_source_document_permission(
        &mut self,
        source_document_id: &str,
        permission: SourcePermission,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error>;

    /// Promote an accepted generated draft into the learner queue.
    ///
    /// # Errors
    ///
    /// Returns the store error when the draft is unknown, already decided, or
    /// cannot be persisted.
    fn keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Edit an accepted generated draft and promote it into the learner queue.
    ///
    /// # Errors
    ///
    /// Returns the store error when the draft is unknown, invalid, already
    /// decided, or cannot be persisted.
    fn edit_and_keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        prompt_text: &str,
        expected_answer: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Record a terminal rejection for an accepted generated draft.
    ///
    /// # Errors
    ///
    /// Returns the store error when the draft is unknown, already decided, or
    /// cannot be persisted.
    fn reject_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<GeneratedPromptDraft, <Self as MemoryServiceStore>::Error>;

    /// Replace the prompt text and expected answer for an kept review unit.
    ///
    /// # Errors
    ///
    /// Returns the store error when the review unit is unknown, archived, or
    /// cannot be updated.
    fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Hide a review unit from the active queue.
    ///
    /// # Errors
    ///
    /// Returns the store error when the review unit is unknown or cannot be
    /// archived.
    fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Move a review unit's beta queue availability forward.
    ///
    /// # Errors
    ///
    /// Returns the store error when the review unit is unknown, archived, or
    /// cannot be snoozed.
    fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Move every non-archived review unit under one persisted concept key
    /// forward without changing any schedule or attempt receipt.
    ///
    /// # Errors
    ///
    /// Returns the store error when the atomic concept operation fails.
    fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, <Self as MemoryServiceStore>::Error>;

    /// Replace volatile lifecycle metadata on a review unit.
    ///
    /// # Errors
    ///
    /// Returns the store error when the review unit is unknown, archived, or
    /// cannot be updated.
    fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;
}

impl BetaStudyStore for BetaPersistenceStore {
    fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::save_source_document(self, document)
    }

    fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::archive_source_document(self, source_document_id, archived_at)
    }

    fn update_source_document_permission(
        &mut self,
        source_document_id: &str,
        permission: SourcePermission,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::update_source_document_permission(
            self,
            source_document_id,
            permission,
        )
    }

    fn keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::keep_generated_prompt_draft(self, draft_id, decided_at)
    }

    fn edit_and_keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        prompt_text: &str,
        expected_answer: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::edit_and_keep_generated_prompt_draft(
            self,
            draft_id,
            prompt_text,
            expected_answer,
            decided_at,
        )
    }

    fn reject_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<GeneratedPromptDraft, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::reject_generated_prompt_draft(self, draft_id, decided_at)
    }

    fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::update_review_unit_prompt_text(
            self,
            review_unit_id,
            prompt_text,
            expected_answer,
        )
    }

    fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::archive_review_unit(self, review_unit_id, archived_at)
    }

    fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::snooze_review_unit_until(self, review_unit_id, snoozed_until)
    }

    fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::snooze_review_units_for_concept_until(
            self,
            concept_key,
            snoozed_until,
        )
    }

    fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::set_review_unit_lifecycle(self, review_unit_id, lifecycle)
    }
}

pub struct BetaStudySession<S = BetaPersistenceStore> {
    store: S,
    now: fn() -> i64,
    cached_snapshot: RefCell<Option<Rc<BetaStoreSnapshot>>>,
    current: Option<GeneratedPromptDraft>,
    status: BetaStudyStatus,
    expected_answer: Option<String>,
    reference_text: Option<String>,
    grade: Option<BetaStudyGrade>,
    schedule_change: Option<ScheduleChange>,
}

impl BetaStudySession<BetaPersistenceStore> {
    /// Open a beta-study session backed by a JSON persistence store.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when the beta store cannot be opened.
    pub fn open(options: BetaStudyOptions) -> Result<Self, BetaStudyError> {
        let store = BetaPersistenceStore::open(options.path).map_err(BetaStudyError::Store)?;
        let snapshot = store.snapshot();
        let status = if snapshot.source_documents.is_empty() {
            BetaStudyStatus::Empty
        } else {
            BetaStudyStatus::Drafting
        };

        Ok(Self {
            store,
            now: options.now,
            cached_snapshot: RefCell::new(Some(Rc::new(snapshot))),
            current: None,
            status,
            expected_answer: None,
            reference_text: None,
            grade: None,
            schedule_change: None,
        })
    }
}

impl<S> BetaStudySession<S>
where
    S: BetaStudyStore,
{
    #[must_use]
    pub fn from_store(store: S, now: fn() -> i64) -> Self {
        let snapshot = store.snapshot();
        let status = match &snapshot {
            Ok(snapshot) if !has_active_sources(snapshot) => BetaStudyStatus::Empty,
            Ok(_) | Err(_) => BetaStudyStatus::Drafting,
        };

        Self {
            store,
            now,
            cached_snapshot: RefCell::new(snapshot.ok().map(Rc::new)),
            current: None,
            status,
            expected_answer: None,
            reference_text: None,
            grade: None,
            schedule_change: None,
        }
    }

    /// Start or resume the session by selecting the next review unit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when queue selection fails.
    pub fn start(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.select_next()?;
        self.view()
    }

    /// Save source material for later generation.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when source validation or persistence fails.
    pub fn add_source(
        &mut self,
        input: BetaStudySourceInput,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let body = input.body.trim().to_owned();
        let title = normalize_capture_title(&input.title, &body);
        self.invalidate_snapshot();
        self.store
            .save_source_document(SourceDocument {
                id: input.id,
                kind: SourceDocumentKind::Text,
                title,
                project_key: input.project_key,
                body: Some(body),
                uri: None,
                permission: input.permission,
                freshness: Some((self.now)()),
                ttl_expires_at: input.ttl_expires_at,
                created_at: (self.now)(),
                archived_at: None,
            })
            .map_err(BetaStudyError::Store)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Update the sharing permission of an active source document.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when the source is unknown, archived, or
    /// persistence fails.
    pub fn update_source_permission(
        &mut self,
        source_document_id: &str,
        permission: SourcePermission,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.invalidate_snapshot();
        self.store
            .update_source_document_permission(source_document_id, permission)
            .map_err(BetaStudyError::Store)?;
        self.view()
    }

    /// Archive source material and remove its generated reviews from the active queue.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when the source is unknown or persistence fails.
    ///
    /// Returns the resulting view plus the count of review units actually
    /// archived by this call — every card generated from this source, across
    /// every generation run, that was still live. The learner-facing notice
    /// (memory-engine-088) reports this count instead of a generic message,
    /// since this single action can silently retire many cards.
    pub fn archive_source(
        &mut self,
        source_document_id: &str,
    ) -> Result<(BetaStudyView, usize), BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let archived_at = (self.now)();
        let related_review_unit_ids = snapshot
            .generated_prompt_drafts
            .iter()
            .filter(|draft| draft_references_source(draft, source_document_id))
            .map(|draft| draft.review_unit_id.clone())
            .collect::<Vec<_>>();

        self.invalidate_snapshot();
        self.store
            .archive_source_document(source_document_id, archived_at)
            .map_err(BetaStudyError::Store)?;

        let mut archived_count = 0usize;
        for review_unit_id in related_review_unit_ids {
            if snapshot
                .review_units
                .iter()
                .any(|unit| unit.review_unit_id == review_unit_id && unit.archived_at.is_none())
            {
                self.store
                    .archive_review_unit(&review_unit_id, archived_at)
                    .map_err(BetaStudyError::Store)?;
                archived_count += 1;
            }
        }

        if self
            .current
            .as_ref()
            .is_some_and(|draft| draft_references_source(draft, source_document_id))
        {
            self.current = None;
            self.expected_answer = None;
            self.reference_text = None;
            self.grade = None;
            self.schedule_change = None;
        }

        let snapshot = self.snapshot()?;
        self.status = if has_active_sources(&snapshot) {
            BetaStudyStatus::Drafting
        } else {
            BetaStudyStatus::Empty
        };
        Ok((self.view()?, archived_count))
    }

    /// Mark a project deck/source and its generated reviews obsolete.
    ///
    /// This is event invalidation, not human forgetting: the source remains in
    /// persisted receipts, but it stops acting as an active deck source and
    /// generated cards stop scheduling through lifecycle policy evaluated by
    /// the kernel queue.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when persistence fails.
    pub fn invalidate_project_deck(
        &mut self,
        source_document_id: &str,
        invalidated_at: i64,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let related_review_unit_ids = snapshot
            .generated_prompt_drafts
            .iter()
            .filter(|draft| draft_references_source(draft, source_document_id))
            .map(|draft| draft.review_unit_id.clone())
            .collect::<Vec<_>>();

        self.invalidate_snapshot();
        self.store
            .archive_source_document(source_document_id, invalidated_at)
            .map_err(BetaStudyError::Store)?;

        for review_unit_id in related_review_unit_ids {
            if let Some(review_unit) = snapshot
                .review_units
                .iter()
                .find(|unit| unit.review_unit_id == review_unit_id && unit.archived_at.is_none())
            {
                self.store
                    .set_review_unit_lifecycle(
                        &review_unit_id,
                        review_unit
                            .queue
                            .lifecycle
                            .with_invalidated_at(Some(invalidated_at)),
                    )
                    .map_err(BetaStudyError::Store)?;
            }
        }

        if self
            .current
            .as_ref()
            .is_some_and(|draft| draft_references_source(draft, source_document_id))
        {
            self.current = None;
            self.expected_answer = None;
            self.reference_text = None;
            self.grade = None;
            self.schedule_change = None;
        }
        self.select_next()?;
        let snapshot = self.snapshot()?;
        if self.current.is_none() && !has_active_sources(&snapshot) {
            self.status = BetaStudyStatus::Empty;
        }
        self.view()
    }

    /// Generate drafts from saved sources using the deterministic
    /// structured-block provider.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when generation or store writes fail.
    pub fn generate(
        &mut self,
        source_document_ids: Option<Vec<String>>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let request = self.generation_request(&snapshot, source_document_ids)?;
        self.invalidate_snapshot();
        run_beta_generation(&mut self.store, request)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Generate deterministic drafts with a caller-owned run id. Background
    /// jobs use this to bind provider usage to their durable attempt rather
    /// than looking up the latest run for an account/source pair.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when generation or store writes fail.
    pub fn generate_with_run_id(
        &mut self,
        source_document_ids: Option<Vec<String>>,
        run_id: impl Into<String>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let request =
            self.generation_request_with_run_id(&snapshot, source_document_ids, run_id.into())?;
        self.invalidate_snapshot();
        run_beta_generation(&mut self.store, request)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Generate drafts from saved sources using the supplied provider.
    ///
    /// Lets the consumer choose a model-backed provider for arbitrary prose
    /// while the crate stays provider-neutral; the provenance trust gate runs
    /// inside generation regardless of provider.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when generation or store writes fail.
    pub fn generate_with_provider(
        &mut self,
        source_document_ids: Option<Vec<String>>,
        provider: &dyn DraftProvider,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let request = self.generation_request(&snapshot, source_document_ids)?;
        self.invalidate_snapshot();
        run_beta_generation_with_provider(&mut self.store, provider, request)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Remove a generation run after its worker lease fence rejects the commit.
    ///
    /// # Errors
    /// Returns the underlying store error when rollback cannot be committed.
    pub fn discard_generation_run(
        &mut self,
        run_id: &str,
    ) -> Result<(), BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.store
            .discard_generation_run(run_id)
            .map_err(BetaStudyError::Store)
    }

    /// Mark a queued generation run pending until its worker lease fence commits.
    ///
    /// # Errors
    /// Returns the underlying store error when the state cannot be persisted.
    pub fn mark_generation_run_pending(
        &mut self,
        run_id: &str,
    ) -> Result<(), BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.store
            .mark_generation_run_pending(run_id)
            .map_err(BetaStudyError::Store)
    }

    /// Atomically fence a generation run and remove stale output when needed.
    /// The persistence adapter performs this under its account-scoped lock.
    ///
    /// # Errors
    /// Returns the underlying store error when finalization cannot commit.
    pub fn finalize_generation_run(
        &mut self,
        run_id: &str,
        generation_attempt: i32,
        lease_token: &str,
        now_ms: i64,
        lease_valid: bool,
    ) -> Result<bool, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.store
            .finalize_generation_run(run_id, generation_attempt, lease_token, now_ms, lease_valid)
            .map_err(BetaStudyError::Store)
    }

    /// Provider-backed counterpart to [`Self::generate_with_run_id`].
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when provider generation or store writes fail.
    pub fn generate_with_provider_and_run_id(
        &mut self,
        source_document_ids: Option<Vec<String>>,
        provider: &dyn DraftProvider,
        run_id: impl Into<String>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let request =
            self.generation_request_with_run_id(&snapshot, source_document_ids, run_id.into())?;
        self.invalidate_snapshot();
        run_beta_generation_with_provider(&mut self.store, provider, request)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    fn generation_request(
        &self,
        snapshot: &BetaStoreSnapshot,
        source_document_ids: Option<Vec<String>>,
    ) -> Result<BetaGenerationRequest, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.generation_request_with_run_id(
            snapshot,
            source_document_ids,
            format!("study-run-{}", snapshot.generation_runs.len() + 1),
        )
    }

    fn generation_request_with_run_id(
        &self,
        snapshot: &BetaStoreSnapshot,
        source_document_ids: Option<Vec<String>>,
        run_id: String,
    ) -> Result<BetaGenerationRequest, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let requested_ids = source_document_ids.unwrap_or_else(|| {
            active_source_ids(snapshot)
                .iter()
                .map(std::string::ToString::to_string)
                .collect()
        });
        for source_id in &requested_ids {
            let source = snapshot
                .source_documents
                .iter()
                .find(|source| &source.id == source_id)
                .ok_or_else(|| {
                    BetaStudyError::Generation(BetaGenerationError::UnknownSourceDocument(
                        source_id.clone(),
                    ))
                })?;
            if source.archived_at.is_some() {
                return Err(BetaStudyError::Generation(
                    BetaGenerationError::ArchivedSourceDocument(source_id.clone()),
                ));
            }
        }

        Ok(BetaGenerationRequest {
            run_id,
            source_document_ids: requested_ids,
            parent_review_unit_id: None,
            started_at: (self.now)(),
            completed_at: Some((self.now)()),
            default_due: (self.now)() - 60_000,
            model: None,
        })
    }

    /// Keep one accepted draft and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when keeping a draft or selecting a queue item fails.
    pub fn keep_draft(
        &mut self,
        draft_id: &str,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.invalidate_snapshot();
        self.store
            .keep_generated_prompt_draft(draft_id, (self.now)())
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Edit one accepted draft, keep it, and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when the draft decision or queue selection fails.
    pub fn edit_and_keep_draft(
        &mut self,
        draft_id: &str,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.invalidate_snapshot();
        self.store
            .edit_and_keep_generated_prompt_draft(
                draft_id,
                prompt_text,
                expected_answer,
                (self.now)(),
            )
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Reject one accepted draft and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when the draft decision or queue selection fails.
    pub fn reject_draft(
        &mut self,
        draft_id: &str,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.invalidate_snapshot();
        self.store
            .reject_generated_prompt_draft(draft_id, (self.now)())
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Show source/reference material for the active item without revealing the answer.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError::NoActiveReviewUnit`] when no item is active.
    pub fn learn_more(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.learn_more_internal(&FakeModelProvider, false)
    }

    /// Show reference material, generating and caching a concept note when no source span exists.
    ///
    /// Source-backed items always prefer their cited source spans. Generated
    /// fallback notes are cached by concept key, so repeated reference views do
    /// not call the provider again.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError::NoActiveReviewUnit`] when no item is active,
    /// or a generation/store error when fallback note creation fails.
    pub fn learn_more_with_provider(
        &mut self,
        provider: &dyn ReferenceNoteProvider,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.learn_more_internal(provider, true)
    }

    /// Return the active review unit's source authorization context.
    ///
    /// This lets higher layers choose the local deterministic path whenever a
    /// `LocalOnly` source is active, without reimplementing source lookup.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError::NoActiveReviewUnit`] when nothing is active,
    /// or a generation/store error when source resolution fails.
    pub fn current_source_authorization(
        &mut self,
    ) -> Result<SourceAuthorizationContext, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let current = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        let draft = snapshot
            .generated_prompt_drafts
            .iter()
            .find(|draft| draft.review_unit_id == current.review_unit_id)
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        ensure_active_source_model_eligible(&snapshot, draft, false)
    }

    fn learn_more_internal(
        &mut self,
        provider: &dyn ReferenceNoteProvider,
        enforce_source_permission: bool,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        let snapshot = self.snapshot()?;
        validate_reference_spans::<<S as MemoryServiceStore>::Error>(&snapshot, active)?;
        let authorization =
            ensure_active_source_model_eligible(&snapshot, active, enforce_source_permission)?;
        if let Some(text) = reference_text(&snapshot, active) {
            self.reference_text = Some(text);
            return self.view();
        }

        let (concept_key, concept_label) = concept_identity_for_draft(active);
        if let Some(note) = snapshot
            .concept_reference_notes
            .iter()
            .find(|note| note.concept_key == concept_key)
        {
            self.reference_text = Some(note.body.clone());
            return self.view();
        }

        let note = provider
            .explain_concept(&ReferenceNoteRequest::new(
                concept_key.clone(),
                concept_label,
                prompt_text(&active.prompt),
                prompt_expected_answer(&active.prompt),
                Vec::new(),
                authorization,
            ))
            .map_err(|failure| {
                BetaStudyError::Generation(BetaGenerationError::ProviderFailure(
                    failure.to_string(),
                ))
            })?;
        self.invalidate_snapshot();
        let note = self
            .store
            .save_concept_reference_note(ConceptReferenceNote {
                concept_key,
                title: note.title,
                body: note.body,
                model: provider.model(),
                created_at: (self.now)(),
                updated_at: (self.now)(),
            })
            .map_err(BetaStudyError::Store)?;
        self.reference_text = Some(note.body);
        self.view()
    }

    /// Edit the active approved review prompt without revealing or rescheduling it.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or persistence fails.
    pub fn edit_current_prompt(
        &mut self,
        prompt_text: impl Into<String>,
        expected_answer: impl Into<String>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        self.invalidate_snapshot();
        self.store
            .update_review_unit_prompt_text(
                &active.review_unit_id,
                &prompt_text.into(),
                &expected_answer.into(),
            )
            .map_err(BetaStudyError::Store)?;
        self.reload_current();
        self.expected_answer = None;
        self.reference_text = None;
        self.grade = None;
        self.schedule_change = None;
        self.status = BetaStudyStatus::Answering;
        self.view()
    }

    /// Archive the active review item and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or persistence fails.
    pub fn archive_current(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        self.invalidate_snapshot();
        self.store
            .archive_review_unit(&active.review_unit_id, (self.now)())
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Snooze the active review item's beta queue availability and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or persistence fails.
    pub fn snooze_current_until(
        &mut self,
        snoozed_until: i64,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        self.invalidate_snapshot();
        self.store
            .snooze_review_unit_until(&active.review_unit_id, snoozed_until)
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Snooze every active review unit sharing the current unit's persisted
    /// concept key, then select the next due candidate.
    ///
    /// Concept membership is deliberately limited to the existing queue key;
    /// review-unit ids and labels are never guessed into a grouping.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active, the active item has
    /// no concept key, or persistence fails.
    pub fn snooze_current_concept_until(
        &mut self,
        snoozed_until: i64,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let review_unit_id = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?
            .review_unit_id
            .clone();
        let concept_key = self
            .snapshot()?
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == review_unit_id)
            .and_then(|unit| unit.queue.concept_key.as_deref())
            .filter(|key| !key.trim().is_empty())
            .map(str::to_owned)
            .ok_or(BetaStudyError::NoConceptKey)?;
        self.snooze_concept_until(&concept_key, snoozed_until)
    }

    /// Snooze all active review units under an existing concept key.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when persistence fails.
    pub fn snooze_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.invalidate_snapshot();
        self.store
            .snooze_review_units_for_concept_until(concept_key, snoozed_until)
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Snooze the current concept for the default long interval.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active, the active item has
    /// no concept key, or persistence fails.
    pub fn snooze_current_concept(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.snooze_current_concept_until((self.now)() + DEFAULT_SNOOZE_DEFER_MS)
    }

    /// Skip the active review item briefly without grading it.
    ///
    /// Skipping is queue deferral only: it records no attempt and leaves
    /// `ScheduleState` unchanged. The item becomes available again after the
    /// default short skip interval.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or persistence fails.
    pub fn skip_current(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.snooze_current_until((self.now)() + DEFAULT_SKIP_DEFER_MS)
    }

    /// Snooze the active review item for the default long interval.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or persistence fails.
    pub fn snooze_current(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.snooze_current_until((self.now)() + DEFAULT_SNOOZE_DEFER_MS)
    }

    /// Generate bridge material using the deterministic CI-safe provider.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active, generation fails, or
    /// persistence rejects the bridge drafts.
    pub fn generate_bridge_material(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.generate_bridge_material_internal(&FakeModelProvider, false)
    }

    /// Generate easier bridge items for the active review and defer the parent.
    ///
    /// The bridge drafts are due immediately and cite the cached concept
    /// reference note. The parent item is snoozed after draft persistence so
    /// the next queue selection naturally surfaces bridge material first.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active, generation fails, or
    /// persistence rejects the bridge drafts.
    pub fn generate_bridge_material_with_provider(
        &mut self,
        provider: &dyn BridgeMaterialProvider,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.generate_bridge_material_internal(provider, true)
    }

    fn generate_bridge_material_internal(
        &mut self,
        provider: &dyn BridgeMaterialProvider,
        enforce_source_permission: bool,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?
            .clone();
        let snapshot = self.snapshot()?;
        let now = (self.now)();
        let bridge_due = snapshot
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == active.review_unit_id)
            .map_or(now - 60_000, |unit| unit.queue.due.saturating_sub(1_000));
        self.invalidate_snapshot();
        let bridge_request = BridgeGenerationRequest {
            run_id: format!("bridge-run-{}", snapshot.generation_runs.len() + 1),
            parent_review_unit_id: active.review_unit_id.clone(),
            started_at: now,
            completed_at: Some(now),
            default_due: bridge_due,
            model: None,
        };
        let _bridge = if enforce_source_permission {
            run_bridge_generation_with_provider(&mut self.store, provider, bridge_request)?
        } else {
            run_bridge_generation(&mut self.store, bridge_request)?
        };
        // Accepted bridge drafts remain pending for learner review.
        self.store
            .snooze_review_unit_until(&active.review_unit_id, now + DEFAULT_BRIDGE_PARENT_DEFER_MS)
            .map_err(BetaStudyError::Store)?;
        self.select_next()?;
        self.view()
    }

    /// Reveal the expected answer for the active review unit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError::NoActiveReviewUnit`] when no item is active.
    pub fn reveal(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        if self.status == BetaStudyStatus::Graded {
            return self.view();
        }
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        self.expected_answer = Some(prompt_expected_answer(&active.prompt));
        self.status = BetaStudyStatus::Revealed;
        self.view()
    }

    /// Grade an answer and atomically apply its schedule update.
    ///
    /// Duplicate submit calls after a successful grade are view-only, matching
    /// the migrated beta-study session contract.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or service execution fails.
    pub fn submit_answer(
        &mut self,
        answer: impl Into<String>,
        response_time_ms: u32,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.submit_answer_with_idempotency_key(answer, response_time_ms, None::<String>)
    }

    /// Grade an answer using a caller-supplied idempotency key.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when no item is active or service execution fails.
    pub fn submit_answer_with_idempotency_key(
        &mut self,
        answer: impl Into<String>,
        response_time_ms: u32,
        idempotency_key: Option<impl Into<String>>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        if self.status == BetaStudyStatus::Graded {
            return self.view();
        }
        let active = self
            .current
            .clone()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        let answer = answer.into();
        let prior_schedule = self
            .store
            .read_schedule_state(&active.review_unit_id)
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))?;
        self.invalidate_snapshot();
        let review = {
            let mut service =
                MemoryService::with_clock(&mut self.store, mastered_after_three_reviews, self.now);
            service.grade_apply_review(GradeApplyReviewCommand {
                prompt: active.prompt.clone(),
                submitted_answer: answer.clone(),
                response_time_ms,
                prompt_id: Some(active.prompt_id.clone()),
                occurred_at: None,
                idempotency_key: Some(idempotency_key.map_or_else(
                    || {
                        format!(
                            "beta-study:{}:{}:{answer}",
                            active.review_unit_id, active.prompt_id
                        )
                    },
                    Into::into,
                )),
            })?
        };

        self.expected_answer = Some(review.grade.expected_answer.clone());
        self.grade = Some(BetaStudyGrade::from_grade(&review.grade));
        self.schedule_change = Some(ScheduleChange {
            before: project_schedule(prior_schedule.as_ref()),
            after: project_required_schedule(&review.schedule_state),
        });
        self.status = BetaStudyStatus::Graded;
        self.view()
    }

    /// Move to the next queue candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when queue selection fails.
    pub fn advance(
        &mut self,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.select_next()?;
        self.view()
    }

    /// Read the current API view.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when schedule or queue reads fail.
    pub fn view(&self) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let mut queue = self
            .store
            .list_queue_candidates()
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))?;
        queue.sort_by_key(|candidate| candidate.due);
        let active_source_ids = active_source_ids(&snapshot);
        let active_sources = snapshot
            .source_documents
            .iter()
            .filter(|source| source_is_active(source))
            .collect::<Vec<_>>();
        let active_drafts = snapshot
            .generated_prompt_drafts
            .iter()
            .filter(|draft| draft_has_active_source(draft, &active_source_ids))
            .collect::<Vec<_>>();
        let now = (self.now)();
        let active_queue = queue
            .iter()
            .filter(|candidate| {
                candidate.lifecycle.is_schedulable(now)
                    && queue_candidate_has_active_source(
                        &snapshot.generated_prompt_drafts,
                        candidate,
                        &active_source_ids,
                    )
            })
            .collect::<Vec<_>>();
        let next_review_unit_id = active_queue
            .first()
            .map(|candidate| candidate.review_unit_id.clone());
        let due_count = active_queue
            .iter()
            .filter(|candidate| candidate.due <= now)
            .count();
        let concept_progress = concept_progress(&snapshot, &active_source_ids);
        let current =
            self.current_projection(&snapshot, &active_source_ids, &concept_progress, now)?;

        Ok(BetaStudyView {
            status: self.status,
            sources: active_sources.iter().copied().map(source_row).collect(),
            drafts: active_drafts
                .iter()
                .copied()
                .map(|draft| draft_row(draft, &snapshot))
                .collect(),
            queue: active_queue
                .iter()
                .map(|candidate| queue_row(&snapshot.generated_prompt_drafts, candidate))
                .collect(),
            due_count,
            current,
            concept_progress,
            summary: BetaStudySummary {
                source_count: active_sources.len(),
                accepted_draft_count: active_drafts
                    .iter()
                    .filter(|draft| {
                        draft.validation.status == GeneratedPromptValidationStatus::Accepted
                    })
                    .count(),
                approved_review_unit_count: snapshot
                    .review_units
                    .iter()
                    .filter(|unit| {
                        unit.archived_at.is_none()
                            && review_unit_has_active_source(
                                &snapshot.generated_prompt_drafts,
                                unit,
                                &active_source_ids,
                            )
                    })
                    .count(),
                attempt_count: snapshot.attempts.len(),
                last_outcome: snapshot
                    .attempts
                    .last()
                    .and_then(|attempt| attempt.grade.as_ref())
                    .map(|grade| grade.verdict),
                next_review_unit_id,
            },
            api_pressure: api_pressure(),
            generation_notices: generation_notices(&snapshot),
        })
    }

    fn current_projection(
        &self,
        snapshot: &BetaStoreSnapshot,
        active_source_ids: &BTreeSet<String>,
        concept_progress: &[BetaStudyConceptProgress],
        now: i64,
    ) -> Result<Option<BetaStudyCurrent>, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.current
            .as_ref()
            .filter(|draft| draft_has_active_source(draft, active_source_ids))
            .map(|draft| {
                self.store
                    .read_schedule_state(&draft.review_unit_id)
                    .map(|schedule| {
                        let feedback = self.grade.as_ref().map(|grade| {
                            feedback_for_current(
                                snapshot,
                                draft,
                                schedule.as_ref(),
                                grade,
                                concept_progress,
                                now,
                            )
                        });
                        current_view(CurrentViewParts {
                            snapshot,
                            draft,
                            schedule: schedule.as_ref(),
                            expected_answer: self.expected_answer.clone(),
                            reference_text: self.reference_text.clone(),
                            grade: self.grade.clone(),
                            schedule_change: self.schedule_change.clone(),
                            feedback,
                        })
                    })
            })
            .transpose()
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))
    }

    fn select_next(&mut self) -> Result<(), BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.snapshot()?;
        let candidates = self
            .store
            .list_queue_candidates()
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))?;
        let now = (self.now)();
        self.current =
            select_current_review_unit(&snapshot, &candidates, now).and_then(|review_unit_id| {
                candidates
                    .iter()
                    .find(|candidate| candidate.review_unit_id == review_unit_id)
                    .and_then(|candidate| find_approved_draft(&snapshot, candidate))
            });
        self.status = if self.current.is_some() {
            BetaStudyStatus::Answering
        } else {
            BetaStudyStatus::Drafting
        };
        self.expected_answer = None;
        self.reference_text = None;
        self.grade = None;
        self.schedule_change = None;

        Ok(())
    }

    fn reload_current(&mut self) {
        let Some(active) = self.current.as_ref() else {
            return;
        };
        let Ok(snapshot) = self.snapshot() else {
            self.current = None;
            return;
        };
        let active_source_ids = active_source_ids(&snapshot);
        self.current = snapshot
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == active.review_unit_id)
            .and_then(|unit| approved_draft_from_unit(&snapshot.generated_prompt_drafts, unit))
            .filter(|draft| draft_has_active_source(draft, &active_source_ids));
    }
    fn snapshot(
        &self,
    ) -> Result<Rc<BetaStoreSnapshot>, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        if let Some(snapshot) = self.cached_snapshot.borrow().as_ref().cloned() {
            return Ok(snapshot);
        }

        let snapshot = Rc::new(self.store.snapshot().map_err(BetaStudyError::Store)?);
        *self.cached_snapshot.borrow_mut() = Some(Rc::clone(&snapshot));
        Ok(snapshot)
    }

    fn invalidate_snapshot(&self) {
        *self.cached_snapshot.borrow_mut() = None;
    }
}

/// Resolve the same current due review unit that [`BetaStudySession::start`]
/// uses, from one consistent persisted snapshot.
#[must_use]
pub fn select_current_review_unit(
    snapshot: &BetaStoreSnapshot,
    candidates: &[QueueCandidate],
    now: i64,
) -> Option<ReviewUnitId> {
    let selected = pick_next_queue_candidate(
        candidates,
        mastered_after_three_reviews,
        &QueueSelectionOptions {
            now,
            ..QueueSelectionOptions::default()
        },
    );
    let active_source_ids = active_source_ids(snapshot);
    let mut current = select_due_variant(
        snapshot,
        candidates,
        &active_source_ids,
        now,
        selected.as_ref(),
    );
    if current.is_none() {
        current = selected
            .as_ref()
            .filter(|candidate| candidate.due <= now)
            .and_then(|candidate| find_approved_draft(snapshot, candidate));
    }
    if current
        .as_ref()
        .is_some_and(|draft| !draft_has_active_source(draft, &active_source_ids))
    {
        current = None;
    }
    if current.is_none() {
        let mut queue = candidates.to_vec();
        queue.sort_by_key(|candidate| candidate.due);
        current = queue
            .iter()
            .filter(|candidate| candidate.lifecycle.is_schedulable(now) && candidate.due <= now)
            .find_map(|candidate| {
                find_approved_draft(snapshot, candidate)
                    .filter(|draft| draft_has_active_source(draft, &active_source_ids))
            });
    }
    current.map(|draft| draft.review_unit_id)
}

impl BetaStudyGrade {
    fn from_grade(grade: &GradeResult) -> Self {
        Self {
            verdict: grade.verdict,
            rating: grade.rating,
            is_correct: grade.is_correct,
        }
    }
}

fn mastered_after_three_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 3
}

fn source_row(source: &SourceDocument) -> BetaStudySourceRow {
    BetaStudySourceRow {
        id: source.id.clone(),
        title: source.title.clone(),
        kind: source.kind.clone(),
        permission: source.permission.clone(),
        created_at: source.created_at,
    }
}

#[must_use]
pub fn infer_capture_title(body: &str) -> String {
    let candidate = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Untitled source");
    let candidate = candidate
        .trim_start_matches(|character: char| {
            character == '#' || character == '-' || character == '*' || character.is_whitespace()
        })
        .trim();
    let sentence_end = candidate
        .char_indices()
        .find_map(|(index, character)| matches!(character, '.' | '?' | '!').then_some(index))
        .unwrap_or(candidate.len());
    let candidate = candidate[..sentence_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let candidate = truncate_title(&candidate, 72);
    if candidate.trim().is_empty() {
        "Untitled source".to_owned()
    } else {
        candidate
    }
}

fn normalize_capture_title(title: &str, body: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        infer_capture_title(body)
    } else {
        truncate_title(&title, 72)
    }
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    if title.chars().count() <= max_chars {
        return title.to_owned();
    }
    let mut truncated = title
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    while truncated.chars().last().is_some_and(char::is_whitespace) {
        truncated.pop();
    }
    truncated.push_str("...");
    truncated
}

fn has_active_sources(snapshot: &BetaStoreSnapshot) -> bool {
    snapshot.source_documents.iter().any(source_is_active)
}

fn active_source_ids(snapshot: &BetaStoreSnapshot) -> BTreeSet<String> {
    snapshot
        .source_documents
        .iter()
        .filter(|source| source_is_active(source))
        .map(|source| source.id.clone())
        .collect()
}

fn source_is_active(source: &SourceDocument) -> bool {
    source.archived_at.is_none()
}

fn draft_references_source(draft: &GeneratedPromptDraft, source_document_id: &str) -> bool {
    draft
        .source_document_ids
        .iter()
        .any(|id| id == source_document_id)
}

fn ensure_active_source_model_eligible<E>(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
    enforce_source_permission: bool,
) -> Result<SourceAuthorizationContext, BetaStudyError<E>> {
    if enforce_source_permission && draft.source_document_ids.is_empty() {
        return Err(BetaStudyError::Generation(
            BetaGenerationError::UnknownSourceDocument(draft.review_unit_id.to_string()),
        ));
    }
    let mut sources = Vec::with_capacity(draft.source_document_ids.len());
    for source_id in &draft.source_document_ids {
        let source = snapshot
            .source_documents
            .iter()
            .find(|source| &source.id == source_id)
            .ok_or_else(|| {
                BetaStudyError::Generation(BetaGenerationError::UnknownSourceDocument(
                    source_id.clone(),
                ))
            })?;
        if source.archived_at.is_some() {
            return Err(BetaStudyError::Generation(
                BetaGenerationError::ArchivedSourceDocument(source.id.clone()),
            ));
        }
        if source.permission == SourcePermission::LocalOnly {
            sources.push(source.clone());
            continue;
        }
        sources.push(source.clone());
    }
    let authorization =
        SourceAuthorizationContext::from_sources(&sources).map_err(|error| match error {
            SourceAuthorizationError::ArchivedSourceDocument(id) => {
                BetaStudyError::Generation(BetaGenerationError::ArchivedSourceDocument(id))
            }
        })?;
    if let Some(source_id) = authorization
        .local_only_source_id()
        .filter(|_| enforce_source_permission)
    {
        return Err(BetaStudyError::Generation(
            BetaGenerationError::LocalOnlySource(source_id.to_owned()),
        ));
    }

    Ok(authorization)
}

fn draft_has_active_source(
    draft: &GeneratedPromptDraft,
    active_source_ids: &BTreeSet<String>,
) -> bool {
    if draft.source_document_ids.is_empty() && draft.concept_reference_note_key.is_some() {
        return true;
    }

    draft
        .source_document_ids
        .iter()
        .any(|source_id| active_source_ids.contains(source_id))
}

fn queue_candidate_has_active_source(
    drafts: &[GeneratedPromptDraft],
    candidate: &QueueCandidate,
    active_source_ids: &BTreeSet<String>,
) -> bool {
    drafts
        .iter()
        .find(|draft| draft.review_unit_id == candidate.review_unit_id)
        .is_none_or(|draft| draft_has_active_source(draft, active_source_ids))
}

fn review_unit_has_active_source(
    drafts: &[GeneratedPromptDraft],
    review_unit: &memory_engine_persistence::BetaReviewUnitRecord,
    active_source_ids: &BTreeSet<String>,
) -> bool {
    approved_draft_from_unit(drafts, review_unit)
        .is_none_or(|draft| draft_has_active_source(&draft, active_source_ids))
}

fn draft_row(draft: &GeneratedPromptDraft, snapshot: &BetaStoreSnapshot) -> BetaStudyDraftRow {
    BetaStudyDraftRow {
        id: draft.id.clone(),
        review_unit_id: draft.review_unit_id.clone(),
        activity_kind: draft.activity_kind.clone(),
        activity_stage: draft.activity_stage.clone(),
        prompt: prompt_text(&draft.prompt).to_owned(),
        answer: prompt_expected_answer(&draft.prompt),
        concept_label: concept_identity_for_draft(draft).1,
        validation_status: draft.validation.status.clone(),
        validation_reasons: draft.validation.reasons.clone(),
        worked_solution: draft.worked_solution.clone(),
        approved: snapshot
            .review_units
            .iter()
            .any(|unit| unit.generated_prompt_draft_id.as_deref() == Some(draft.id.as_str())),
        learner_decision: draft.learner_decision.clone(),
        source_spans: draft
            .reference_span_ids
            .iter()
            .filter_map(|id| snapshot.reference_spans.iter().find(|span| &span.id == id))
            .map(|span| BetaStudyReferenceSpanRow {
                id: span.id.clone(),
                source_document_id: span.source_document_id.clone(),
                label: span.label.clone(),
                text: span.text.clone(),
                locator: span.locator.clone(),
            })
            .collect(),
        provenance: draft.generation_run_id.as_ref().and_then(|run_id| {
            snapshot
                .generation_runs
                .iter()
                .find(|run| &run.id == run_id)
                .map(|run| BetaStudyGenerationProvenance {
                    generation_run_id: Some(run.id.clone()),
                    provider: run.provider.clone(),
                    model: run.model.clone(),
                    prompt_version: (!run.prompt_version.is_empty())
                        .then(|| run.prompt_version.clone()),
                })
        }),
    }
}

fn queue_row(drafts: &[GeneratedPromptDraft], candidate: &QueueCandidate) -> BetaStudyQueueRow {
    let draft = drafts
        .iter()
        .find(|item| item.review_unit_id == candidate.review_unit_id);

    BetaStudyQueueRow {
        review_unit_id: candidate.review_unit_id.clone(),
        due: candidate.due,
        reps: candidate
            .schedule_state
            .as_ref()
            .map_or(0, |state| state.reps),
        state: candidate.schedule_state.as_ref().map(|state| state.state),
        activity_kind: draft.map(|draft| draft.activity_kind.clone()),
        activity_stage: draft.map(|draft| draft.activity_stage.clone()),
    }
}

struct CurrentViewParts<'a> {
    snapshot: &'a BetaStoreSnapshot,
    draft: &'a GeneratedPromptDraft,
    schedule: Option<&'a ScheduleState>,
    expected_answer: Option<String>,
    reference_text: Option<String>,
    grade: Option<BetaStudyGrade>,
    schedule_change: Option<ScheduleChange>,
    feedback: Option<BetaStudyFeedback>,
}

fn current_view(parts: CurrentViewParts<'_>) -> BetaStudyCurrent {
    let CurrentViewParts {
        snapshot,
        draft,
        schedule,
        expected_answer,
        reference_text,
        grade,
        schedule_change,
        feedback,
    } = parts;

    let concept_key = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == draft.review_unit_id)
        .and_then(|unit| unit.queue.concept_key.clone());

    BetaStudyCurrent {
        review_unit_id: draft.review_unit_id.clone(),
        concept_key,
        prompt_id: draft.prompt_id.clone(),
        activity_kind: draft.activity_kind.clone(),
        activity_stage: draft.activity_stage.clone(),
        prompt: prompt_text(&draft.prompt).to_owned(),
        choices: projected_choices(snapshot, draft, grade.is_some()),
        revision_expected_answer: prompt_expected_answer(&draft.prompt),
        worked_solution: expected_answer
            .as_ref()
            .and_then(|_| draft.worked_solution.clone()),
        expected_answer,
        reference_text,
        grade,
        review_state: project_schedule(schedule),
        schedule_change,
        feedback,
        content_feedback_head_id: current_feedback_head_id(snapshot, &draft.review_unit_id),
    }
}

fn feedback_for_current(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
    schedule: Option<&ScheduleState>,
    grade: &BetaStudyGrade,
    concept_progress: &[BetaStudyConceptProgress],
    now: i64,
) -> BetaStudyFeedback {
    let item_history = item_history(snapshot, &draft.review_unit_id, schedule, now);
    let (concept_key, _) = concept_identity_for_draft(draft);
    let concept_progress = concept_progress
        .iter()
        .find(|concept| concept.concept_key == concept_key)
        .cloned();

    BetaStudyFeedback {
        verdict: verdict_label(grade.verdict).to_owned(),
        expected_answer: prompt_expected_answer(&draft.prompt),
        item_history,
        concept_progress,
    }
}

fn current_feedback_head_id(
    snapshot: &BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
) -> Option<String> {
    snapshot
        .content_feedback
        .iter()
        .filter(|feedback| feedback.review_unit_id == *review_unit_id)
        .filter(|feedback| {
            !snapshot.content_feedback.iter().any(|other| {
                other.review_unit_id == *review_unit_id
                    && other.supersedes_id.as_deref() == Some(feedback.id.as_str())
            })
        })
        .max_by_key(|feedback| (feedback.occurred_at, feedback.id.as_str()))
        .map(|feedback| feedback.id.clone())
}

fn item_history(
    snapshot: &BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
    schedule: Option<&ScheduleState>,
    now: i64,
) -> BetaStudyItemHistory {
    let mut attempts = snapshot
        .attempts
        .iter()
        .filter(|attempt| attempt.review_unit_id == *review_unit_id)
        .collect::<Vec<_>>();
    attempts.sort_by_key(|attempt| attempt.occurred_at);
    let correct = attempts
        .iter()
        .filter(|attempt| attempt.grade.as_ref().is_some_and(|grade| grade.is_correct))
        .count();

    let last_seen = attempts.iter().map(|attempt| attempt.occurred_at).max();
    let outcomes = attempts
        .iter()
        .filter_map(|attempt| attempt.grade.as_ref().map(|grade| grade.is_correct))
        .collect::<Vec<_>>();
    let response_times = attempts
        .iter()
        .map(|attempt| attempt.response_time_ms)
        .collect::<Vec<_>>();

    BetaStudyItemHistory {
        attempts: attempts.len(),
        correct,
        success_rate: success_rate(correct, attempts.len()),
        trend: trend(&outcomes),
        last_seen,
        last_seen_summary: last_seen.map_or_else(
            || "not seen before".to_owned(),
            |last_seen| last_seen_phrase(last_seen, now),
        ),
        last_response_time_ms: response_times.last().copied(),
        average_response_time_ms: average_response_time_ms(&response_times),
        response_time_trend: response_time_trend(&response_times),
        stage: schedule.map_or_else(|| "New".to_owned(), schedule_stage),
        next_review: schedule.map_or_else(
            || "no review is scheduled yet".to_owned(),
            |schedule| next_review_phrase(schedule.due, now),
        ),
    }
}

fn concept_progress(
    snapshot: &BetaStoreSnapshot,
    active_source_ids: &BTreeSet<String>,
) -> Vec<BetaStudyConceptProgress> {
    let mut rows: BTreeMap<String, ConceptAccumulator> = BTreeMap::new();
    let tracked_review_unit_ids = snapshot
        .review_units
        .iter()
        .filter(|unit| {
            unit.archived_at.is_none()
                && approved_draft_from_unit(&snapshot.generated_prompt_drafts, unit)
                    .is_some_and(|draft| draft_has_active_source(&draft, active_source_ids))
        })
        .map(|unit| unit.review_unit_id.as_str().to_owned())
        .collect::<BTreeSet<_>>();

    for review_unit_id in &tracked_review_unit_ids {
        let (concept_key, concept_label) =
            concept_identity_for_review_unit(snapshot, &ReviewUnitId::new(review_unit_id));
        rows.entry(concept_key.clone())
            .or_insert_with(|| ConceptAccumulator::new(concept_key, concept_label));
    }

    let mut attempts = snapshot
        .attempts
        .iter()
        .filter(|attempt| {
            attempt.grade.is_some()
                && tracked_review_unit_ids.contains(attempt.review_unit_id.as_str())
        })
        .collect::<Vec<_>>();
    attempts.sort_by_key(|attempt| attempt.occurred_at);
    for attempt in attempts {
        let (concept_key, concept_label) =
            concept_identity_for_review_unit(snapshot, &attempt.review_unit_id);
        let row = rows
            .entry(concept_key.clone())
            .or_insert_with(|| ConceptAccumulator::new(concept_key, concept_label));
        row.record(attempt);
    }

    let mut progress = rows
        .into_values()
        .map(ConceptAccumulator::into_progress)
        .collect::<Vec<_>>();
    progress.sort_by(|left, right| {
        health_sort_key(left)
            .cmp(&health_sort_key(right))
            .then_with(|| right.attempts.cmp(&left.attempts))
            .then_with(|| left.concept_label.cmp(&right.concept_label))
    });
    progress
}

#[derive(Debug)]
struct ConceptAccumulator {
    concept_key: String,
    concept_label: String,
    attempts: usize,
    correct: usize,
    outcomes: Vec<bool>,
    response_times: Vec<u32>,
}

impl ConceptAccumulator {
    fn new(concept_key: String, concept_label: String) -> Self {
        Self {
            concept_key,
            concept_label,
            attempts: 0,
            correct: 0,
            outcomes: Vec::new(),
            response_times: Vec::new(),
        }
    }

    fn record(&mut self, attempt: &memory_engine_service::ServiceAttemptRecord) {
        self.attempts += 1;
        let is_correct = attempt.grade.as_ref().is_some_and(|grade| grade.is_correct);
        if is_correct {
            self.correct += 1;
        }
        self.outcomes.push(is_correct);
        self.response_times.push(attempt.response_time_ms);
    }

    fn into_progress(self) -> BetaStudyConceptProgress {
        let success_rate = success_rate(self.correct, self.attempts);
        let trend = trend(&self.outcomes);
        let response_time_trend = response_time_trend(&self.response_times);
        let health = health(self.correct, self.attempts).to_owned();
        let summary = concept_summary(
            &self.concept_label,
            &health,
            &success_rate,
            &trend,
            &response_time_trend,
        );

        BetaStudyConceptProgress {
            concept_key: self.concept_key,
            concept_label: self.concept_label,
            attempts: self.attempts,
            correct: self.correct,
            success_rate,
            trend,
            average_response_time_ms: average_response_time_ms(&self.response_times),
            response_time_trend,
            health,
            summary,
        }
    }
}

fn concept_identity_for_review_unit(
    snapshot: &BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
) -> (String, String) {
    let unit = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == *review_unit_id);
    let key = unit
        .and_then(|unit| unit.queue.concept_key.clone())
        .or_else(|| {
            unit.and_then(|unit| {
                unit.generated_prompt_draft_id
                    .as_ref()
                    .and_then(|draft_id| {
                        snapshot
                            .generated_prompt_drafts
                            .iter()
                            .find(|draft| &draft.id == draft_id)
                            .and_then(|draft| draft.queue.concept_key.clone())
                    })
            })
        })
        .or_else(|| unit.and_then(|unit| unit.concept_reference_note_key.clone()));

    if let Some(key) = key {
        let label = concept_label_for_key(&key);
        (key, label)
    } else {
        let label = unit.map_or_else(
            || "this item".to_owned(),
            |unit| prompt_text(&unit.prompt).to_owned(),
        );
        (review_unit_id.as_str().to_owned(), label)
    }
}

fn health_sort_key(progress: &BetaStudyConceptProgress) -> usize {
    progress
        .correct
        .saturating_mul(10_000)
        .checked_div(progress.attempts)
        .unwrap_or(usize::MAX)
}

fn success_rate(correct: usize, attempts: usize) -> String {
    if attempts == 0 {
        return "0 of 0 correct (0.0%)".to_owned();
    }
    let percent_tenths = correct.saturating_mul(1_000).saturating_add(attempts / 2) / attempts;
    format!(
        "{correct} of {attempts} correct ({}.{:01}%)",
        percent_tenths / 10,
        percent_tenths % 10
    )
}

fn trend(outcomes: &[bool]) -> String {
    match outcomes {
        [] | [_] => "not enough data".to_owned(),
        values => {
            let previous = values[values.len() - 2];
            let latest = values[values.len() - 1];
            match (previous, latest) {
                (false, true) => "improving".to_owned(),
                (true, false) => "declining".to_owned(),
                (true, true) => "steady correct".to_owned(),
                (false, false) => "still missing".to_owned(),
            }
        }
    }
}

fn average_response_time_ms(response_times: &[u32]) -> Option<u32> {
    if response_times.is_empty() {
        return None;
    }
    let total: u64 = response_times.iter().map(|value| u64::from(*value)).sum();
    let count = u64::try_from(response_times.len()).unwrap_or(1);
    u32::try_from(total / count).ok()
}

fn response_time_trend(response_times: &[u32]) -> String {
    match response_times {
        [] | [_] => "not enough data".to_owned(),
        values => {
            let previous = values[values.len() - 2];
            let latest = values[values.len() - 1];
            match latest.cmp(&previous) {
                Ordering::Less => "faster".to_owned(),
                Ordering::Greater => "slower".to_owned(),
                Ordering::Equal => "steady".to_owned(),
            }
        }
    }
}

fn health(correct: usize, attempts: usize) -> &'static str {
    if attempts == 0 {
        return "untried";
    }
    if correct.saturating_mul(2) < attempts {
        "struggling"
    } else if correct.saturating_mul(5) < attempts.saturating_mul(4) {
        "mixed"
    } else {
        "solid"
    }
}

fn concept_summary(
    label: &str,
    health: &str,
    success_rate: &str,
    trend: &str,
    response_time_trend: &str,
) -> String {
    format!(
        "{label} is {health}: {success_rate}; trend is {trend}; response time is {response_time_trend}."
    )
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Correct => "Correct",
        Verdict::Close => "Close",
        Verdict::Wrong => "Try again",
        Verdict::Revealed => "Revealed",
    }
}

fn schedule_stage(schedule: &ScheduleState) -> String {
    let state = match schedule.state {
        ScheduleStatus::New => "New",
        ScheduleStatus::Learning => "Learning",
        ScheduleStatus::Review => "Review",
        ScheduleStatus::Relearning => "Relearning",
    };
    format!(
        "{state}, interval {}",
        interval_phrase(schedule.scheduled_days)
    )
}

fn next_review_phrase(due: i64, now: i64) -> String {
    if due <= now {
        return "you'll see this again now".to_owned();
    }
    let delta_ms = due.saturating_sub(now);
    let rounded_days = rounded_time_units(delta_ms, DAY_MS);
    if rounded_days >= 1 {
        return format!(
            "you'll see this again in ~{} {}",
            rounded_days,
            if rounded_days == 1 { "day" } else { "days" }
        );
    }
    let rounded_hours = rounded_time_units(delta_ms, HOUR_MS).max(1);
    format!(
        "you'll see this again in ~{} {}",
        rounded_hours,
        if rounded_hours == 1 { "hour" } else { "hours" }
    )
}

fn last_seen_phrase(last_seen: i64, now: i64) -> String {
    if last_seen >= now {
        return "last seen just now".to_owned();
    }
    let delta_ms = now.saturating_sub(last_seen);
    let rounded_days = rounded_time_units(delta_ms, DAY_MS);
    if rounded_days >= 1 {
        return format!(
            "last seen ~{} {} ago",
            rounded_days,
            if rounded_days == 1 { "day" } else { "days" }
        );
    }
    let rounded_hours = rounded_time_units(delta_ms, HOUR_MS);
    if rounded_hours >= 1 {
        return format!(
            "last seen ~{} {} ago",
            rounded_hours,
            if rounded_hours == 1 { "hour" } else { "hours" }
        );
    }
    "last seen just now".to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VariantGroup {
    concept_key: String,
    stage_order: u32,
    activity_stage: String,
}

fn select_due_variant(
    snapshot: &BetaStoreSnapshot,
    candidates: &[QueueCandidate],
    active_source_ids: &BTreeSet<String>,
    now: i64,
    selected: Option<&QueueCandidate>,
) -> Option<GeneratedPromptDraft> {
    let selected = selected
        .filter(|candidate| candidate.lifecycle.is_schedulable(now) && candidate.due <= now)?;
    let selected_draft = find_approved_draft(snapshot, selected)?;
    if !draft_has_active_source(&selected_draft, active_source_ids) {
        return None;
    }
    let group = variant_group(&selected_draft)?;
    let options = QueueSelectionOptions {
        now,
        ..QueueSelectionOptions::default()
    };
    let variants = reviewable_queue_candidates(candidates, mastered_after_three_reviews, &options)
        .iter()
        .filter_map(|candidate| find_approved_draft(snapshot, candidate))
        .filter(|draft| draft_has_active_source(draft, active_source_ids))
        .filter(|draft| variant_group(draft).as_ref() == Some(&group))
        .collect::<Vec<_>>();
    if variants.len() <= 1 {
        return Some(selected_draft);
    }

    let mut variants = variants
        .into_iter()
        .map(|draft| (variant_attempt_key(snapshot, &draft), draft))
        .collect::<Vec<_>>();
    variants.sort_by(|(left_key, left_draft), (right_key, right_draft)| {
        left_key
            .cmp(right_key)
            .then_with(|| left_draft.review_unit_id.cmp(&right_draft.review_unit_id))
    });
    variants.into_iter().next().map(|(_, draft)| draft)
}

fn variant_group(draft: &GeneratedPromptDraft) -> Option<VariantGroup> {
    let progression = draft.queue.progression.as_ref()?;
    let concept_key = draft
        .queue
        .concept_key
        .clone()
        .or_else(|| progression.progression_group.clone())?;
    Some(VariantGroup {
        concept_key,
        stage_order: progression.stage_order,
        activity_stage: draft.activity_stage.clone(),
    })
}

fn variant_attempt_key(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
) -> (usize, Option<i64>) {
    snapshot
        .attempts
        .iter()
        .filter(|attempt| attempt.review_unit_id == draft.review_unit_id)
        .fold((0, None), |(count, latest), attempt| {
            (count + 1, latest.max(Some(attempt.occurred_at)))
        })
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

fn rounded_time_units(delta_ms: i64, unit_ms: i64) -> i64 {
    delta_ms.saturating_add(unit_ms / 2) / unit_ms
}

fn interval_phrase(days: i64) -> String {
    match days {
        0 => "under a day".to_owned(),
        1 => "~1 day".to_owned(),
        days => format!("~{days} days"),
    }
}

fn find_approved_draft(
    snapshot: &memory_engine_persistence::BetaStoreSnapshot,
    candidate: &QueueCandidate,
) -> Option<GeneratedPromptDraft> {
    let draft_id = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == candidate.review_unit_id)?
        .generated_prompt_draft_id
        .as_ref()?;
    snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| &draft.id == draft_id)
        .cloned()
}

fn approved_draft_from_unit(
    drafts: &[GeneratedPromptDraft],
    review_unit: &memory_engine_persistence::BetaReviewUnitRecord,
) -> Option<GeneratedPromptDraft> {
    let draft_id = review_unit.generated_prompt_draft_id.as_ref()?;
    drafts.iter().find(|draft| &draft.id == draft_id).cloned()
}

fn prompt_text(prompt: &Prompt) -> &str {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => prompt,
        Prompt::Exact(prompt) => &prompt.prompt,
    }
}

fn projected_choices(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
    hold_latest_attempt: bool,
) -> Vec<String> {
    let Prompt::Mcq { choices, .. } = &draft.prompt else {
        return Vec::new();
    };
    if choices.len() <= 1 {
        return choices.clone();
    }
    let attempts = snapshot
        .attempts
        .iter()
        .filter(|attempt| attempt.review_unit_id == draft.review_unit_id)
        .count();
    let display_attempts = if hold_latest_attempt {
        attempts.saturating_sub(1)
    } else {
        attempts
    };
    let mut projected = choices.clone();
    let offset = (stable_seed(draft.review_unit_id.as_str()).saturating_add(display_attempts))
        % projected.len();
    projected.rotate_left(offset);
    projected
}

fn stable_seed(value: &str) -> usize {
    value.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ usize::from(byte)
    })
}

fn prompt_expected_answer(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Mcq { correct_choice, .. } => correct_choice.clone(),
        Prompt::Boolean {
            correct_answer: true,
            ..
        } => "True".to_owned(),
        Prompt::Boolean {
            correct_answer: false,
            ..
        } => "False".to_owned(),
        Prompt::Exact(prompt) => prompt.accepted_answers.join(" / "),
    }
}

fn reference_text(snapshot: &BetaStoreSnapshot, draft: &GeneratedPromptDraft) -> Option<String> {
    let text = draft
        .reference_span_ids
        .iter()
        .filter_map(|id| snapshot.reference_spans.iter().find(|span| &span.id == id))
        .map(|span| span.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !text.is_empty() {
        return Some(text);
    }

    draft
        .concept_reference_note_key
        .as_ref()
        .and_then(|key| {
            snapshot
                .concept_reference_notes
                .iter()
                .find(|note| &note.concept_key == key)
        })
        .map(|note| note.body.trim().to_owned())
        .filter(|body| !body.is_empty())
}

fn validate_reference_spans<E>(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
) -> Result<(), BetaStudyError<E>> {
    for reference_span_id in &draft.reference_span_ids {
        if !snapshot
            .reference_spans
            .iter()
            .any(|span| &span.id == reference_span_id)
        {
            return Err(BetaStudyError::UnknownReferenceSpan(
                reference_span_id.clone(),
            ));
        }
    }
    Ok(())
}

fn concept_identity_for_draft(draft: &GeneratedPromptDraft) -> (String, String) {
    let key = draft
        .queue
        .concept_key
        .clone()
        .or_else(|| draft.concept_reference_note_key.clone());

    match key {
        Some(key) => {
            let label = concept_label_for_key(&key);
            (key, label)
        }
        None => (
            draft.review_unit_id.as_str().to_owned(),
            prompt_text(&draft.prompt).to_owned(),
        ),
    }
}

fn concept_label_for_key(key: &str) -> String {
    key.split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn project_schedule(schedule: Option<&ScheduleState>) -> Option<ReviewStateProjection> {
    schedule.map(project_required_schedule)
}

fn project_required_schedule(schedule: &ScheduleState) -> ReviewStateProjection {
    ReviewStateProjection {
        due: schedule.due,
        reps: schedule.reps,
        lapses: schedule.lapses,
        state: schedule.state,
        last_review: schedule.last_review,
    }
}

/// Build human-readable notices for the most recent generation run so the
/// study UI never shows a silent empty result. Surfaces run-level failures
/// (provider errors, missing provenance) and, when the run accepted no
/// drafts, an explicit empty-result sentence.
fn generation_notices(snapshot: &BetaStoreSnapshot) -> Vec<String> {
    let Some(run) = snapshot
        .generation_runs
        .iter()
        .max_by_key(|run| (run.started_at, run.completed_at))
    else {
        return Vec::new();
    };

    let mut notices = run.validation_failures.clone();
    let accepted = snapshot
        .generated_prompt_drafts
        .iter()
        .filter(|draft| {
            draft.generation_run_id.as_deref() == Some(run.id.as_str())
                && draft.validation.status == GeneratedPromptValidationStatus::Accepted
        })
        .count();
    if run.completed_at.is_some() && accepted == 0 {
        notices.push(
            "No review items could be generated from this source yet — \
             try pasting more complete prose, or generate again."
                .to_owned(),
        );
    }

    notices
}

fn api_pressure() -> Vec<String> {
    [
        "Beta study still owns source creation, draft approval, reveal state, and mobile UI state.",
        "The service boundary is usable for queue selection and grade/apply-review without promoting persistence into the pure kernel.",
        "Worked-solution display is activity metadata, not a kernel scheduling concern yet.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
