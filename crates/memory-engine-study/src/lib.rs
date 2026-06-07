//! Beta-study session orchestration.
//!
//! This crate owns the repo-local beta-study workflow: source intake,
//! deterministic draft generation, draft approval, reveal state, grading, and
//! queue advancement. It composes the generation, persistence, and service
//! crates without moving filesystem, HTTP, or UI concerns into the pure core.

use std::{error::Error, fmt, path::PathBuf};

use memory_engine_core::{
    GradeResult, Prompt, QueueCandidate, ReviewUnitId, ScheduleState, ScheduleStatus, Verdict,
};
use memory_engine_generation::{
    run_beta_generation, BetaGenerationError, BetaGenerationRequest, BetaGenerationStore,
};
use memory_engine_persistence::{
    ApproveGeneratedPromptDraftOptions, BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreError,
    GeneratedLearningActivityKind, GeneratedPromptDraft, GeneratedPromptValidationStatus,
    SourceDocument, SourceDocumentKind, SourcePermission,
};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, NextQueueCommand, NextQueueOptions,
    ServiceError,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_BETA_STUDY_NOW: i64 = 1_779_465_600_000;

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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyView {
    pub status: BetaStudyStatus,
    pub sources: Vec<BetaStudySourceRow>,
    pub drafts: Vec<BetaStudyDraftRow>,
    pub queue: Vec<BetaStudyQueueRow>,
    pub current: Option<BetaStudyCurrent>,
    pub summary: BetaStudySummary,
    pub api_pressure: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudySourceRow {
    pub id: String,
    pub title: String,
    pub kind: SourceDocumentKind,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStudyDraftRow {
    pub id: String,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub prompt: String,
    pub validation_status: GeneratedPromptValidationStatus,
    pub validation_reasons: Vec<String>,
    pub worked_solution: Option<String>,
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
    pub prompt_id: String,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub prompt: String,
    pub revision_expected_answer: String,
    pub expected_answer: Option<String>,
    pub reference_text: Option<String>,
    pub worked_solution: Option<String>,
    pub grade: Option<BetaStudyGrade>,
    pub review_state: Option<ReviewStateProjection>,
    pub schedule_change: Option<ScheduleChange>,
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
    NoActiveReviewUnit,
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
            Self::NoActiveReviewUnit => {
                formatter.write_str("Beta study session has no active review unit")
            }
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
            Self::NoActiveReviewUnit => None,
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

    /// Promote an accepted generated draft into a review unit.
    ///
    /// # Errors
    ///
    /// Returns the store error when the draft is unknown, rejected, or cannot be
    /// promoted.
    fn approve_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        options: ApproveGeneratedPromptDraftOptions,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error>;

    /// Replace the prompt text and expected answer for an approved review unit.
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
}

impl BetaStudyStore for BetaPersistenceStore {
    fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::save_source_document(self, document)
    }

    fn approve_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        options: ApproveGeneratedPromptDraftOptions,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        BetaPersistenceStore::approve_generated_prompt_draft(self, draft_id, options)
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
}

pub struct BetaStudySession<S = BetaPersistenceStore> {
    store: S,
    now: fn() -> i64,
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
        let status = if store.snapshot().source_documents.is_empty() {
            BetaStudyStatus::Empty
        } else {
            BetaStudyStatus::Drafting
        };

        Ok(Self {
            store,
            now: options.now,
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
        let status = match store.snapshot() {
            Ok(snapshot) if snapshot.source_documents.is_empty() => BetaStudyStatus::Empty,
            Ok(_) | Err(_) => BetaStudyStatus::Drafting,
        };

        Self {
            store,
            now,
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
        self.store
            .save_source_document(SourceDocument {
                id: input.id,
                kind: SourceDocumentKind::Text,
                title: input.title,
                body: Some(input.body),
                uri: None,
                permission: SourcePermission::ModelEligible,
                freshness: Some((self.now)()),
                created_at: (self.now)(),
            })
            .map_err(BetaStudyError::Store)?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Generate deterministic drafts from all saved sources or the supplied ids.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when generation or store writes fail.
    pub fn generate(
        &mut self,
        source_document_ids: Option<Vec<String>>,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let snapshot = self.store.snapshot().map_err(BetaStudyError::Store)?;
        let ids = source_document_ids.unwrap_or_else(|| {
            snapshot
                .source_documents
                .iter()
                .map(|source| source.id.clone())
                .collect()
        });
        run_beta_generation(
            &mut self.store,
            BetaGenerationRequest {
                run_id: format!("study-run-{}", snapshot.generation_runs.len() + 1),
                source_document_ids: ids,
                started_at: (self.now)(),
                completed_at: Some((self.now)()),
                default_due: (self.now)() - 60_000,
                model: None,
            },
        )?;
        self.status = BetaStudyStatus::Drafting;
        self.view()
    }

    /// Approve one accepted draft and select the next candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStudyError`] when approval or queue selection fails.
    pub fn approve_draft(
        &mut self,
        draft_id: &str,
    ) -> Result<BetaStudyView, BetaStudyError<<S as MemoryServiceStore>::Error>> {
        self.store
            .approve_generated_prompt_draft(draft_id, ApproveGeneratedPromptDraftOptions::default())
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
        let active = self
            .current
            .as_ref()
            .ok_or(BetaStudyError::NoActiveReviewUnit)?;
        let snapshot = self.store.snapshot().map_err(BetaStudyError::Store)?;
        self.reference_text = reference_text(&snapshot.reference_spans, active);
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
        self.store
            .snooze_review_unit_until(&active.review_unit_id, snoozed_until)
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
        let snapshot = self.store.snapshot().map_err(BetaStudyError::Store)?;
        let mut queue = self
            .store
            .list_queue_candidates()
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))?;
        queue.sort_by_key(|candidate| candidate.due);
        let next_review_unit_id = queue
            .first()
            .map(|candidate| candidate.review_unit_id.clone());
        let current = self
            .current
            .as_ref()
            .map(|draft| {
                self.store
                    .read_schedule_state(&draft.review_unit_id)
                    .map(|schedule| {
                        current_view(
                            draft,
                            schedule.as_ref(),
                            self.expected_answer.clone(),
                            self.reference_text.clone(),
                            self.grade.clone(),
                            self.schedule_change.clone(),
                        )
                    })
            })
            .transpose()
            .map_err(|error| BetaStudyError::Service(ServiceError::Store(error)))?;

        Ok(BetaStudyView {
            status: self.status,
            sources: snapshot.source_documents.iter().map(source_row).collect(),
            drafts: snapshot
                .generated_prompt_drafts
                .iter()
                .map(draft_row)
                .collect(),
            queue: queue
                .iter()
                .map(|candidate| queue_row(&snapshot.generated_prompt_drafts, candidate))
                .collect(),
            current,
            summary: BetaStudySummary {
                source_count: snapshot.source_documents.len(),
                accepted_draft_count: snapshot
                    .generated_prompt_drafts
                    .iter()
                    .filter(|draft| {
                        draft.validation.status == GeneratedPromptValidationStatus::Accepted
                    })
                    .count(),
                approved_review_unit_count: snapshot
                    .review_units
                    .iter()
                    .filter(|unit| unit.archived_at.is_none())
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
        })
    }

    fn select_next(&mut self) -> Result<(), BetaStudyError<<S as MemoryServiceStore>::Error>> {
        let selected = {
            let mut service =
                MemoryService::with_clock(&mut self.store, mastered_after_three_reviews, self.now);
            service.next_queue(NextQueueCommand {
                options: NextQueueOptions::default(),
            })?
        };
        let snapshot = self.store.snapshot().map_err(BetaStudyError::Store)?;
        self.current = selected
            .candidate
            .as_ref()
            .and_then(|candidate| find_approved_draft(&snapshot, candidate));
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
        let Ok(snapshot) = self.store.snapshot() else {
            self.current = None;
            return;
        };
        self.current = snapshot
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == active.review_unit_id)
            .and_then(|unit| approved_draft_from_unit(&snapshot.generated_prompt_drafts, unit));
    }
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
        created_at: source.created_at,
    }
}

fn draft_row(draft: &GeneratedPromptDraft) -> BetaStudyDraftRow {
    BetaStudyDraftRow {
        id: draft.id.clone(),
        activity_kind: draft.activity_kind.clone(),
        activity_stage: draft.activity_stage.clone(),
        prompt: prompt_text(&draft.prompt).to_owned(),
        validation_status: draft.validation.status.clone(),
        validation_reasons: draft.validation.reasons.clone(),
        worked_solution: draft.worked_solution.clone(),
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

fn current_view(
    draft: &GeneratedPromptDraft,
    schedule: Option<&ScheduleState>,
    expected_answer: Option<String>,
    reference_text: Option<String>,
    grade: Option<BetaStudyGrade>,
    schedule_change: Option<ScheduleChange>,
) -> BetaStudyCurrent {
    BetaStudyCurrent {
        review_unit_id: draft.review_unit_id.clone(),
        prompt_id: draft.prompt_id.clone(),
        activity_kind: draft.activity_kind.clone(),
        activity_stage: draft.activity_stage.clone(),
        prompt: prompt_text(&draft.prompt).to_owned(),
        revision_expected_answer: prompt_expected_answer(&draft.prompt),
        worked_solution: expected_answer
            .as_ref()
            .and_then(|_| draft.worked_solution.clone()),
        expected_answer,
        reference_text,
        grade,
        review_state: project_schedule(schedule),
        schedule_change,
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

fn reference_text(
    spans: &[memory_engine_persistence::ReferenceSpan],
    draft: &GeneratedPromptDraft,
) -> Option<String> {
    let text = draft
        .reference_span_ids
        .iter()
        .filter_map(|id| spans.iter().find(|span| &span.id == id))
        .map(|span| span.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
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
