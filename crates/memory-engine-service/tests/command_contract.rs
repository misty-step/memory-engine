use std::collections::{BTreeSet, HashMap};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, Prompt, QueueCandidate, Rating, ReviewUnitId, ScheduleState,
    ScheduleStatus,
};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceCommand, MemoryServiceResult,
    MemoryServiceStore, NextQueueCommand, NextQueueOptions, RecordAttemptCommand,
    RecordAttemptInput, ServiceAttemptRecord, ServiceError,
};

const NOW: i64 = 1_779_989_400_000;

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoreError {
    UnknownReviewUnit(ReviewUnitId),
    BlankAnswer,
    NonPositiveResponseTime,
    ApplyFailed,
}

#[derive(Default)]
struct TestStore {
    known: BTreeSet<ReviewUnitId>,
    attempts: Vec<ServiceAttemptRecord>,
    schedules: HashMap<ReviewUnitId, ScheduleState>,
    candidates: Vec<QueueCandidate>,
    fail_apply: bool,
}

impl TestStore {
    fn with_known(ids: &[&str]) -> Self {
        Self {
            known: ids.iter().map(|id| review_unit_id(id)).collect(),
            ..Self::default()
        }
    }

    fn assert_known(&self, review_unit_id: &ReviewUnitId) -> Result<(), StoreError> {
        if self.known.contains(review_unit_id) {
            Ok(())
        } else {
            Err(StoreError::UnknownReviewUnit(review_unit_id.clone()))
        }
    }

    fn assert_attempt(&self, attempt: &ServiceAttemptRecord) -> Result<(), StoreError> {
        self.assert_known(&attempt.review_unit_id)?;
        if attempt.submitted_answer.trim().is_empty() {
            return Err(StoreError::BlankAnswer);
        }
        if attempt.response_time_ms == 0 {
            return Err(StoreError::NonPositiveResponseTime);
        }

        Ok(())
    }
}

impl MemoryServiceStore for TestStore {
    type Error = StoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.assert_attempt(&attempt)?;
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
        if self.fail_apply {
            return Err(StoreError::ApplyFailed);
        }
        self.assert_known(review_unit_id)?;
        self.assert_attempt(&attempt)?;
        assert_eq!(review_unit_id, &attempt.review_unit_id);
        assert_eq!(schedule_state.last_review, Some(attempt.occurred_at));

        self.attempts.push(attempt);
        self.schedules
            .insert(review_unit_id.clone(), schedule_state);

        Ok(())
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self.candidates.clone())
    }
}

#[test]
fn records_an_attempt_without_grading_or_scheduling() {
    let store = TestStore::with_known(&["latin-amo"]);
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);

    let result = service.record_attempt(RecordAttemptCommand {
        attempt: RecordAttemptInput {
            review_unit_id: review_unit_id("latin-amo"),
            prompt_id: Some("latin-present-active".to_owned()),
            submitted_answer: "amo".to_owned(),
            response_time_ms: 2_400,
            occurred_at: None,
            idempotency_key: None,
        },
    });

    let attempt = result.expect("attempt should record");
    assert_eq!(
        attempt,
        ServiceAttemptRecord {
            review_unit_id: review_unit_id("latin-amo"),
            prompt_id: Some("latin-present-active".to_owned()),
            submitted_answer: "amo".to_owned(),
            response_time_ms: 2_400,
            occurred_at: NOW,
            idempotency_key: None,
            grade: None,
        }
    );
    assert_eq!(service.into_store().attempts, [attempt]);
}

#[test]
fn grades_and_applies_review_as_one_store_commit() {
    let store = TestStore::with_known(&["prayer-kyrie"]);
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);

    let result = service
        .grade_apply_review(GradeApplyReviewCommand {
            prompt: short_answer_prompt(
                "prayer-kyrie",
                "Kyrie eleison means what?",
                "Lord have mercy",
            ),
            submitted_answer: "Lord have mercy".to_owned(),
            response_time_ms: 3_200,
            prompt_id: None,
            occurred_at: None,
            idempotency_key: None,
        })
        .expect("review should apply");

    assert_eq!(result.grade.rating, Rating::Good);
    assert_eq!(result.grade.submitted_answer, "Lord have mercy");
    assert_eq!(result.schedule_state.reps, 1);
    assert_eq!(result.schedule_state.state, ScheduleStatus::Learning);
    assert_eq!(result.schedule_state.last_review, Some(NOW));

    let store = service.into_store();
    assert_eq!(
        store.attempts.as_slice(),
        std::slice::from_ref(&result.attempt)
    );
    assert_eq!(
        store.schedules.get(&review_unit_id("prayer-kyrie")),
        Some(&result.schedule_state)
    );
}

#[test]
fn selects_next_queue_candidate_through_core_queue_policy() {
    let mut store = TestStore::with_known(&["fresh-new", "review-due"]);
    store.candidates = vec![
        candidate("fresh-new", None, NOW - 60_000),
        candidate(
            "review-due",
            Some(schedule(ScheduleStatus::Review, 4, NOW - 3_600_000)),
            NOW - 3_600_000,
        ),
    ];
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);

    let result = service
        .next_queue(NextQueueCommand {
            options: NextQueueOptions::default(),
        })
        .expect("queue should select");

    assert_eq!(
        result.candidate.expect("candidate").review_unit_id,
        review_unit_id("review-due")
    );
}

#[test]
fn execute_uses_a_closed_command_result_union() {
    let store = TestStore::with_known(&["latin-amo"]);
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);

    let result = service
        .execute(MemoryServiceCommand::RecordAttempt {
            attempt: RecordAttemptInput {
                review_unit_id: review_unit_id("latin-amo"),
                prompt_id: None,
                submitted_answer: "amo".to_owned(),
                response_time_ms: 1_100,
                occurred_at: None,
                idempotency_key: Some("attempt:latin-amo:1".to_owned()),
            },
        })
        .expect("command should execute");

    match result {
        MemoryServiceResult::AttemptRecorded { attempt } => {
            assert_eq!(attempt.occurred_at, NOW);
            assert_eq!(
                attempt.idempotency_key.as_deref(),
                Some("attempt:latin-amo:1")
            );
        }
        _ => panic!("unexpected command result"),
    }
}

#[test]
fn command_and_result_envelopes_keep_typescript_kind_tags() {
    let command = MemoryServiceCommand::GradeApplyReview {
        prompt: short_answer_prompt("known-unit", "Kyrie eleison means what?", "Lord have mercy"),
        submitted_answer: "Lord have mercy".to_owned(),
        response_time_ms: 1_200,
        prompt_id: Some("kyrie-en".to_owned()),
        occurred_at: Some(NOW),
        idempotency_key: Some("review:known-unit:1".to_owned()),
    };
    let encoded = serde_json::to_value(&command).expect("command json");

    assert_eq!(encoded["kind"], "grade/apply-review");
    assert_eq!(encoded["submitted_answer"], "Lord have mercy");

    let result = MemoryServiceResult::QueueSelected { candidate: None };
    let encoded = serde_json::to_value(&result).expect("result json");

    assert_eq!(
        encoded,
        serde_json::json!({
            "kind": "queue-selected",
            "candidate": null
        })
    );
}

#[test]
fn store_failures_do_not_return_successful_review_results() {
    let mut store = TestStore::with_known(&["known-unit"]);
    store.fail_apply = true;
    let mut service = MemoryService::with_clock(store, mastered_after_three_reviews, || NOW);

    let result = service.grade_apply_review(GradeApplyReviewCommand {
        prompt: short_answer_prompt("known-unit", "Kyrie eleison means what?", "Lord have mercy"),
        submitted_answer: "Lord have mercy".to_owned(),
        response_time_ms: 1_200,
        prompt_id: None,
        occurred_at: None,
        idempotency_key: None,
    });

    assert_eq!(result, Err(ServiceError::Store(StoreError::ApplyFailed)));
}

fn mastered_after_three_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 3
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

fn candidate(id: &str, schedule_state: Option<ScheduleState>, due: i64) -> QueueCandidate {
    QueueCandidate {
        review_unit_id: review_unit_id(id),
        schedule_state,
        due,
        progression: None,
        concept_key: None,
        source_key: None,
        domain_key: None,
    }
}

fn schedule(status: ScheduleStatus, reps: u32, due: i64) -> ScheduleState {
    ScheduleState {
        due,
        stability: 2.0,
        difficulty: 3.0,
        elapsed_days: 1,
        scheduled_days: 2,
        reps,
        lapses: 0,
        state: status,
        last_review: Some(NOW - 86_400_000),
    }
}

fn review_unit_id(value: &str) -> ReviewUnitId {
    ReviewUnitId::new(value)
}
