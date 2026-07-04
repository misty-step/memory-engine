//! Rust command-line dogfood clients.
//!
//! This crate keeps fixture content, confidence capture, calibration receipts,
//! and in-memory CLI storage outside the reusable learning kernel. The CLI asks
//! the service crate for deep workflows instead of reassembling grading,
//! scheduling, and queue selection itself.

use std::{collections::BTreeMap, error::Error, fmt};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, ProgressionMetadata, Prompt, QueueCandidate, Rating,
    ReviewUnitId, ReviewUnitLifecycle, ScheduleState, ScheduleStatus, Verdict,
};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, NextQueueCommand, NextQueueOptions,
    ServiceAttemptRecord, ServiceError,
};
use serde::Serialize;

const NOW: i64 = 1_779_984_000_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliReviewReceipt {
    pub fixture: String,
    pub commands: Vec<String>,
    pub confidence: f64,
    pub calibration_error: f64,
    pub attempt_count: usize,
    pub graded_verdict: String,
    pub graded_rating: u8,
    pub scheduled_reps: u32,
    pub next_review_unit_id: Option<String>,
    pub stayed_outside_kernel: Vec<String>,
}

#[derive(Clone, Debug)]
struct CliReviewUnit {
    review_unit_id: ReviewUnitId,
    prompt_id: String,
    prompt: Prompt,
    submitted_answer: String,
    confidence: f64,
    response_time_ms: u32,
    queue: CliQueueMetadata,
}

#[derive(Clone, Debug)]
struct CliQueueMetadata {
    progression: Option<ProgressionMetadata>,
    concept_key: Option<String>,
    source_key: Option<String>,
    domain_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliReviewError {
    EmptyFixture,
    Store(CliStoreError),
    Service(ServiceError<CliStoreError>),
}

impl fmt::Display for CliReviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFixture => formatter.write_str("CLI review fixture must not be empty"),
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
        }
    }
}

impl Error for CliReviewError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Service(error) => Some(error),
            Self::EmptyFixture => None,
        }
    }
}

impl From<CliStoreError> for CliReviewError {
    fn from(error: CliStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ServiceError<CliStoreError>> for CliReviewError {
    fn from(error: ServiceError<CliStoreError>) -> Self {
        Self::Service(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliStoreError {
    UnknownReviewUnit(ReviewUnitId),
}

impl fmt::Display for CliStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
        }
    }
}

impl Error for CliStoreError {}

#[derive(Clone, Debug)]
struct CliReviewStore {
    units: BTreeMap<ReviewUnitId, CliReviewUnit>,
    attempts: Vec<ServiceAttemptRecord>,
    schedules: BTreeMap<ReviewUnitId, ScheduleState>,
}

impl CliReviewStore {
    fn new(units: Vec<CliReviewUnit>) -> Self {
        Self {
            units: units
                .into_iter()
                .map(|unit| (unit.review_unit_id.clone(), unit))
                .collect(),
            attempts: Vec::new(),
            schedules: BTreeMap::new(),
        }
    }

    fn attempt_count(&self) -> usize {
        self.attempts.len()
    }

    fn scheduled_reps(&self, review_unit_id: &ReviewUnitId) -> u32 {
        self.schedules
            .get(review_unit_id)
            .map_or(0, |schedule| schedule.reps)
    }

    fn assert_known(&self, review_unit_id: &ReviewUnitId) -> Result<(), CliStoreError> {
        if self.units.contains_key(review_unit_id) {
            Ok(())
        } else {
            Err(CliStoreError::UnknownReviewUnit(review_unit_id.clone()))
        }
    }
}

impl MemoryServiceStore for CliReviewStore {
    type Error = CliStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.assert_known(&attempt.review_unit_id)?;
        self.attempts.push(attempt);
        Ok(())
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        self.assert_known(review_unit_id)?;
        Ok(self.schedules.get(review_unit_id).cloned())
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        _expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        self.assert_known(review_unit_id)?;
        self.attempts.push(attempt);
        self.schedules
            .insert(review_unit_id.clone(), schedule_state);
        Ok(())
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self
            .units
            .values()
            .map(|unit| {
                let schedule_state = self.schedules.get(&unit.review_unit_id).cloned();
                QueueCandidate {
                    review_unit_id: unit.review_unit_id.clone(),
                    due: schedule_state
                        .as_ref()
                        .map_or(NOW - 60_000, |state| state.due),
                    schedule_state,
                    lifecycle: ReviewUnitLifecycle::active(),
                    progression: unit.queue.progression.clone(),
                    concept_key: unit.queue.concept_key.clone(),
                    source_key: unit.queue.source_key.clone(),
                    domain_key: unit.queue.domain_key.clone(),
                }
            })
            .collect())
    }
}

/// Run the Latin-prayer CLI review dogfood flow.
///
/// # Errors
///
/// Returns [`CliReviewError`] if the fixture is empty or service execution
/// fails.
pub fn run_cli_review() -> Result<CliReviewReceipt, CliReviewError> {
    let fixture = cli_fixture();
    let first = fixture.first().ok_or(CliReviewError::EmptyFixture)?.clone();
    let store = CliReviewStore::new(fixture);
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);
    let review = service.grade_apply_review(GradeApplyReviewCommand {
        prompt: first.prompt.clone(),
        submitted_answer: first.submitted_answer.clone(),
        response_time_ms: first.response_time_ms,
        prompt_id: Some(first.prompt_id.clone()),
        occurred_at: Some(NOW),
        idempotency_key: None,
    })?;
    let next = service.next_queue(NextQueueCommand {
        options: NextQueueOptions::default(),
    })?;
    let store = service.into_store();
    let actual = u8::from(review.grade.is_correct);

    Ok(CliReviewReceipt {
        fixture: "latin-prayer-opening".to_owned(),
        commands: vec!["grade/apply-review".to_owned(), "next-queue".to_owned()],
        confidence: first.confidence,
        calibration_error: (first.confidence - f64::from(actual)).abs(),
        attempt_count: store.attempt_count(),
        graded_verdict: verdict_name(review.grade.verdict).to_owned(),
        graded_rating: rating_value(review.grade.rating),
        scheduled_reps: store.scheduled_reps(&first.review_unit_id),
        next_review_unit_id: next
            .candidate
            .as_ref()
            .map(|candidate| candidate.review_unit_id.as_str().to_owned()),
        stayed_outside_kernel: vec![
            "fixture content".to_owned(),
            "confidence capture".to_owned(),
            "calibration metric".to_owned(),
            "CLI receipt formatting".to_owned(),
            "in-memory dogfood store".to_owned(),
        ],
    })
}

fn cli_fixture() -> Vec<CliReviewUnit> {
    vec![
        CliReviewUnit {
            review_unit_id: review_unit_id("cli-credo-opening"),
            prompt_id: "cli-credo-opening-en".to_owned(),
            prompt: short_answer_prompt(
                "cli-credo-opening",
                "What does Credo in unum Deum mean?",
                "I believe in one God",
            ),
            submitted_answer: "I believe in one God".to_owned(),
            confidence: 0.72,
            response_time_ms: 2_800,
            queue: CliQueueMetadata {
                progression: None,
                concept_key: Some("creed-opening".to_owned()),
                source_key: Some("mass-core".to_owned()),
                domain_key: Some("latin".to_owned()),
            },
        },
        CliReviewUnit {
            review_unit_id: review_unit_id("cli-pater-opening"),
            prompt_id: "cli-pater-opening-en".to_owned(),
            prompt: short_answer_prompt(
                "cli-pater-opening",
                "What does Pater noster mean?",
                "Our Father",
            ),
            submitted_answer: "Our Father".to_owned(),
            confidence: 0.61,
            response_time_ms: 2_500,
            queue: CliQueueMetadata {
                progression: None,
                concept_key: Some("lords-prayer-opening".to_owned()),
                source_key: Some("mass-core".to_owned()),
                domain_key: Some("latin".to_owned()),
            },
        },
    ]
}

fn short_answer_prompt(id: &str, prompt: &str, answer: &str) -> Prompt {
    Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: review_unit_id(id),
        prompt: prompt.to_owned(),
        accepted_answers: vec![answer.to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    })
}

fn review_unit_id(value: &str) -> ReviewUnitId {
    ReviewUnitId::new(value)
}

fn mastered_after_three_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 3
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Correct => "correct",
        Verdict::Close => "close",
        Verdict::Wrong => "wrong",
        Verdict::Revealed => "revealed",
    }
}

fn rating_value(rating: Rating) -> u8 {
    match rating {
        Rating::Again => 1,
        Rating::Hard => 2,
        Rating::Good => 3,
        Rating::Easy => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::{run_cli_review, CliReviewStore, MemoryServiceStore};

    #[test]
    fn runs_a_calibration_aware_review_loop_through_the_rust_service_boundary() {
        let receipt = run_cli_review().expect("receipt");

        assert_eq!(receipt.fixture, "latin-prayer-opening");
        assert_eq!(receipt.commands, ["grade/apply-review", "next-queue"]);
        assert!((receipt.confidence - 0.72).abs() < f64::EPSILON);
        assert!((receipt.calibration_error - 0.28).abs() < f64::EPSILON);
        assert_eq!(receipt.attempt_count, 1);
        assert_eq!(receipt.graded_verdict, "correct");
        assert_eq!(receipt.graded_rating, 3);
        assert_eq!(receipt.scheduled_reps, 1);
        assert_eq!(
            receipt.next_review_unit_id.as_deref(),
            Some("cli-pater-opening")
        );
        assert!(receipt
            .stayed_outside_kernel
            .contains(&"confidence capture".to_owned()));
    }

    #[test]
    fn cli_store_rejects_unknown_review_units_at_the_boundary() {
        let store = CliReviewStore::new(Vec::new());
        let error = store
            .read_schedule_state(&memory_engine_core::ReviewUnitId::new("missing"))
            .expect_err("unknown");

        assert_eq!(error.to_string(), "Unknown review unit: missing");
    }
}
