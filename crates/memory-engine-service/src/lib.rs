//! Typed service command boundary for memory-engine.
//!
//! This crate coordinates pure kernel operations into command workflows while
//! keeping persistence behind an injected store trait. It intentionally contains
//! no filesystem, network, UI, framework, logging, or model-client code.

use std::{error::Error, fmt};

use memory_engine_core::{
    pick_next_queue_candidate, FsrsScheduler, GradeContext, GradeResult, Grader, Prompt,
    QueueCandidate, QueueSelectionOptions, QueueSeparationPass, ReviewUnitId, ScheduleState,
    Scheduler, SchedulerError,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAttemptRecord {
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: Option<String>,
    pub submitted_answer: String,
    pub response_time_ms: u32,
    pub occurred_at: i64,
    pub idempotency_key: Option<String>,
    pub grade: Option<GradeResult>,
}

pub trait MemoryServiceStore {
    type Error;

    /// Persist an ungraded attempt.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the attempt is rejected or cannot be
    /// persisted.
    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error>;

    /// Read the current schedule for one review unit.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the review unit lookup fails.
    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error>;

    /// Persist an applied review attempt and its next schedule state.
    ///
    /// # Errors
    ///
    /// Returns the store's error when validation, idempotency, optimistic
    /// concurrency, or durable persistence fails.
    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error>;

    /// List candidates available to queue selection.
    ///
    /// # Errors
    ///
    /// Returns the store's error when candidate lookup fails.
    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error>;
}

impl<TStore> MemoryServiceStore for &mut TStore
where
    TStore: MemoryServiceStore + ?Sized,
{
    type Error = TStore::Error;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        (**self).record_attempt(attempt)
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        (**self).read_schedule_state(review_unit_id)
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        (**self).apply_review(
            review_unit_id,
            attempt,
            schedule_state,
            expected_prior_schedule_state,
        )
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        (**self).list_queue_candidates()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAttemptInput {
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: Option<String>,
    pub submitted_answer: String,
    pub response_time_ms: u32,
    pub occurred_at: Option<i64>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAttemptCommand {
    pub attempt: RecordAttemptInput,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeApplyReviewCommand {
    pub prompt: Prompt,
    pub submitted_answer: String,
    pub response_time_ms: u32,
    pub prompt_id: Option<String>,
    pub occurred_at: Option<i64>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextQueueOptions {
    pub recent_candidates: Vec<QueueCandidate>,
    pub population: Option<Vec<QueueCandidate>>,
    pub candidate_window: Option<usize>,
    pub recent_concept_window: Option<usize>,
    pub recent_source_window: Option<usize>,
    pub recent_domain_window: Option<usize>,
    pub separation_passes: Option<Vec<QueueSeparationPass>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextQueueCommand {
    pub options: NextQueueOptions,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MemoryServiceCommand {
    RecordAttempt {
        attempt: RecordAttemptInput,
    },
    #[serde(rename = "grade/apply-review")]
    #[serde(rename_all = "camelCase")]
    GradeApplyReview {
        prompt: Prompt,
        submitted_answer: String,
        response_time_ms: u32,
        prompt_id: Option<String>,
        occurred_at: Option<i64>,
        idempotency_key: Option<String>,
    },
    NextQueue {
        options: NextQueueOptions,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRecordedResult {
    pub attempt: ServiceAttemptRecord,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAppliedResult {
    pub attempt: ServiceAttemptRecord,
    pub grade: GradeResult,
    pub schedule_state: ScheduleState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueueSelectedResult {
    pub candidate: Option<QueueCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MemoryServiceResult {
    AttemptRecorded {
        attempt: Box<ServiceAttemptRecord>,
    },
    ReviewApplied {
        attempt: Box<ServiceAttemptRecord>,
        grade: Box<GradeResult>,
        schedule_state: Box<ScheduleState>,
    },
    QueueSelected {
        candidate: Option<Box<QueueCandidate>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceError<TStoreError> {
    Store(TStoreError),
    Scheduler(SchedulerError),
}

impl<TStoreError: fmt::Display> fmt::Display for ServiceError<TStoreError> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::Scheduler(error) => write!(formatter, "scheduler error: {error}"),
        }
    }
}

impl<TStoreError> Error for ServiceError<TStoreError> where TStoreError: Error + 'static {}

pub struct MemoryService<TStore, TMastery, TScheduler = FsrsScheduler> {
    store: TStore,
    mastery_policy: TMastery,
    now: fn() -> i64,
    grader: Grader,
    scheduler: TScheduler,
}

impl<TStore, TMastery> MemoryService<TStore, TMastery, FsrsScheduler>
where
    TStore: MemoryServiceStore,
    TMastery: Fn(&ScheduleState) -> bool + Copy,
{
    #[must_use]
    pub fn new(store: TStore, mastery_policy: TMastery) -> Self {
        Self::with_clock(store, mastery_policy, unix_epoch_now)
    }

    #[must_use]
    pub fn with_clock(store: TStore, mastery_policy: TMastery, now: fn() -> i64) -> Self {
        Self {
            store,
            mastery_policy,
            now,
            grader: Grader::new(),
            scheduler: FsrsScheduler,
        }
    }
}

impl<TStore, TMastery, TScheduler> MemoryService<TStore, TMastery, TScheduler>
where
    TStore: MemoryServiceStore,
    TMastery: Fn(&ScheduleState) -> bool + Copy,
    TScheduler: Scheduler,
{
    #[must_use]
    pub fn with_parts(
        store: TStore,
        mastery_policy: TMastery,
        now: fn() -> i64,
        grader: Grader,
        scheduler: TScheduler,
    ) -> Self {
        Self {
            store,
            mastery_policy,
            now,
            grader,
            scheduler,
        }
    }

    pub fn into_store(self) -> TStore {
        self.store
    }

    #[must_use]
    pub fn store(&self) -> &TStore {
        &self.store
    }

    /// Execute one typed service command.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceError::Store`] when the injected store rejects a read or
    /// write, and [`ServiceError::Scheduler`] when scheduling cannot advance the
    /// provided state.
    pub fn execute(
        &mut self,
        command: MemoryServiceCommand,
    ) -> Result<MemoryServiceResult, ServiceError<TStore::Error>> {
        match command {
            MemoryServiceCommand::RecordAttempt { attempt } => self
                .record_attempt(RecordAttemptCommand { attempt })
                .map(|attempt| MemoryServiceResult::AttemptRecorded {
                    attempt: Box::new(attempt),
                }),
            MemoryServiceCommand::GradeApplyReview {
                prompt,
                submitted_answer,
                response_time_ms,
                prompt_id,
                occurred_at,
                idempotency_key,
            } => self
                .grade_apply_review(GradeApplyReviewCommand {
                    prompt,
                    submitted_answer,
                    response_time_ms,
                    prompt_id,
                    occurred_at,
                    idempotency_key,
                })
                .map(|result| MemoryServiceResult::ReviewApplied {
                    attempt: Box::new(result.attempt),
                    grade: Box::new(result.grade),
                    schedule_state: Box::new(result.schedule_state),
                }),
            MemoryServiceCommand::NextQueue { options } => self
                .next_queue(NextQueueCommand { options })
                .map(|result| MemoryServiceResult::QueueSelected {
                    candidate: result.candidate.map(Box::new),
                }),
        }
    }

    /// Record an ungraded attempt.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceError::Store`] when the injected store rejects the
    /// attempt.
    pub fn record_attempt(
        &mut self,
        command: RecordAttemptCommand,
    ) -> Result<ServiceAttemptRecord, ServiceError<TStore::Error>> {
        let attempt = ServiceAttemptRecord {
            review_unit_id: command.attempt.review_unit_id,
            prompt_id: command.attempt.prompt_id,
            submitted_answer: command.attempt.submitted_answer,
            response_time_ms: command.attempt.response_time_ms,
            occurred_at: command.attempt.occurred_at.unwrap_or_else(|| (self.now)()),
            idempotency_key: command.attempt.idempotency_key,
            grade: None,
        };

        self.store
            .record_attempt(attempt.clone())
            .map_err(ServiceError::Store)?;

        Ok(attempt)
    }

    /// Grade an answer, advance its schedule, and ask the store to commit both.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceError::Store`] for store read/write failures and
    /// [`ServiceError::Scheduler`] for invalid schedule transitions.
    pub fn grade_apply_review(
        &mut self,
        command: GradeApplyReviewCommand,
    ) -> Result<ReviewAppliedResult, ServiceError<TStore::Error>> {
        let review_unit_id = prompt_review_unit_id(&command.prompt).clone();
        let prior_schedule = self
            .store
            .read_schedule_state(&review_unit_id)
            .map_err(ServiceError::Store)?;
        let occurred_at = command.occurred_at.unwrap_or_else(|| (self.now)());
        let grade = self.grader.grade(
            &command.prompt,
            &command.submitted_answer,
            GradeContext {
                response_time_ms: command.response_time_ms,
                prior_reps: prior_schedule.as_ref().map_or(0, |schedule| schedule.reps),
            },
        );
        let attempt = ServiceAttemptRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: command.prompt_id,
            submitted_answer: command.submitted_answer,
            response_time_ms: command.response_time_ms,
            occurred_at,
            idempotency_key: command.idempotency_key,
            grade: Some(grade.clone()),
        };
        let schedule_state = self
            .scheduler
            .advance(prior_schedule.as_ref(), grade.rating, occurred_at)
            .map_err(ServiceError::Scheduler)?;

        self.store
            .apply_review(
                &review_unit_id,
                attempt.clone(),
                schedule_state.clone(),
                prior_schedule,
            )
            .map_err(ServiceError::Store)?;

        Ok(ReviewAppliedResult {
            attempt,
            grade,
            schedule_state,
        })
    }

    /// Select the next due queue candidate.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceError::Store`] when the injected store cannot provide
    /// queue candidates.
    pub fn next_queue(
        &mut self,
        command: NextQueueCommand,
    ) -> Result<QueueSelectedResult, ServiceError<TStore::Error>> {
        let NextQueueCommand { options } = command;
        let candidates = self
            .store
            .list_queue_candidates()
            .map_err(ServiceError::Store)?;
        let defaults = QueueSelectionOptions::default();
        let separation_passes = options
            .separation_passes
            .as_deref()
            .unwrap_or(defaults.separation_passes);
        let options = QueueSelectionOptions {
            now: (self.now)(),
            recent_candidates: &options.recent_candidates,
            population: options.population.as_deref(),
            candidate_window: options
                .candidate_window
                .unwrap_or(defaults.candidate_window),
            recent_concept_window: options
                .recent_concept_window
                .unwrap_or(defaults.recent_concept_window),
            recent_source_window: options
                .recent_source_window
                .unwrap_or(defaults.recent_source_window),
            recent_domain_window: options
                .recent_domain_window
                .unwrap_or(defaults.recent_domain_window),
            separation_passes,
        };
        let candidate = pick_next_queue_candidate(&candidates, self.mastery_policy, &options);

        Ok(QueueSelectedResult { candidate })
    }
}

fn prompt_review_unit_id(prompt: &Prompt) -> &ReviewUnitId {
    match prompt {
        Prompt::Mcq { review_unit_id, .. } | Prompt::Boolean { review_unit_id, .. } => {
            review_unit_id
        }
        Prompt::Exact(prompt) => &prompt.review_unit_id,
    }
}

fn unix_epoch_now() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    i64::try_from(now.as_millis()).unwrap_or(i64::MAX)
}
