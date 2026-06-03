//! Rust benchmark receipts for migrated memory-engine surfaces.
//!
//! These benchmarks are non-gating smoke receipts. They intentionally avoid
//! local timing thresholds and instead prove that the Rust facade, queue, and
//! service paths are executable at useful scale.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use memory_engine::{
    service::{
        GradeApplyReviewCommand, MemoryService, MemoryServiceStore, NextQueueCommand,
        NextQueueOptions, ServiceAttemptRecord,
    },
    ExactPrompt, ExactPromptKind, GradeContext, Grader, Prompt, QueueCandidate, Rating,
    ReviewUnitId, ScheduleState, ScheduleStatus,
};

const NOW: i64 = 1_779_465_600_000;

#[derive(Clone, Debug, PartialEq)]
struct BenchReceipt {
    name: &'static str,
    operations: u32,
    elapsed: Duration,
}

impl BenchReceipt {
    fn ops_per_ms(&self) -> f64 {
        f64::from(self.operations) / self.elapsed.as_secs_f64().max(0.001).mul_add(1_000.0, 0.0)
    }
}

#[derive(Clone, Debug)]
struct BenchCase {
    name: &'static str,
    operations: u32,
    run: fn(u32) -> Result<(), BenchError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BenchError {
    Store(BenchStoreError),
    Service(String),
}

impl fmt::Display for BenchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "store error: {error}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
        }
    }
}

impl Error for BenchError {}

impl From<BenchStoreError> for BenchError {
    fn from(error: BenchStoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BenchStoreError {
    UnknownReviewUnit(ReviewUnitId),
}

impl fmt::Display for BenchStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReviewUnit(id) => write!(formatter, "unknown review unit: {id}"),
        }
    }
}

impl Error for BenchStoreError {}

#[derive(Clone, Debug)]
struct BenchStore {
    candidates: Vec<QueueCandidate>,
    attempts: Vec<ServiceAttemptRecord>,
    schedules: BTreeMap<ReviewUnitId, ScheduleState>,
}

impl BenchStore {
    fn new(candidates: Vec<QueueCandidate>) -> Self {
        Self {
            candidates,
            attempts: Vec::new(),
            schedules: BTreeMap::new(),
        }
    }

    fn assert_known(&self, review_unit_id: &ReviewUnitId) -> Result<(), BenchStoreError> {
        if self
            .candidates
            .iter()
            .any(|candidate| candidate.review_unit_id == *review_unit_id)
        {
            Ok(())
        } else {
            Err(BenchStoreError::UnknownReviewUnit(review_unit_id.clone()))
        }
    }
}

impl MemoryServiceStore for BenchStore {
    type Error = BenchStoreError;

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
            .candidates
            .iter()
            .map(|candidate| {
                let schedule_state = self.schedules.get(&candidate.review_unit_id).cloned();
                QueueCandidate {
                    schedule_state: schedule_state
                        .clone()
                        .or_else(|| candidate.schedule_state.clone()),
                    due: schedule_state
                        .as_ref()
                        .map_or(candidate.due, |state| state.due),
                    ..candidate.clone()
                }
            })
            .collect())
    }
}

fn main() {
    match run_benchmarks() {
        Ok(receipts) => print_receipts(&receipts),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn run_benchmarks() -> Result<Vec<BenchReceipt>, BenchError> {
    run_benchmark_cases(benchmark_cases())
}

fn benchmark_cases() -> Vec<BenchCase> {
    vec![
        BenchCase {
            name: "grading.short_answer",
            operations: 10_000,
            run: |operations| {
                bench_grading(operations);
                Ok(())
            },
        },
        BenchCase {
            name: "scheduling.next",
            operations: 10_000,
            run: bench_scheduling,
        },
        BenchCase {
            name: "queue.pick_next.1000",
            operations: 1_000,
            run: |operations| {
                bench_queue(operations);
                Ok(())
            },
        },
        BenchCase {
            name: "service.grade_apply_review.next_queue",
            operations: 500,
            run: bench_service,
        },
    ]
}

#[cfg(test)]
fn smoke_benchmark_cases() -> Vec<BenchCase> {
    benchmark_cases()
        .into_iter()
        .map(|bench_case| BenchCase {
            operations: 5,
            ..bench_case
        })
        .collect()
}

fn run_benchmark_cases(cases: Vec<BenchCase>) -> Result<Vec<BenchReceipt>, BenchError> {
    cases
        .into_iter()
        .map(|bench_case| {
            let start = Instant::now();
            (bench_case.run)(bench_case.operations)?;
            Ok(BenchReceipt {
                name: bench_case.name,
                operations: bench_case.operations,
                elapsed: start.elapsed(),
            })
        })
        .collect()
}

fn bench_grading(operations: u32) {
    let grader = Grader::new();
    let prompt = bench_prompt("bench-prompt");

    for index in 0_u32..operations {
        let _grade = grader.grade(
            &prompt,
            "Punishment",
            GradeContext {
                response_time_ms: 2_000,
                prior_reps: index % 5,
            },
        );
    }
}

fn bench_scheduling(operations: u32) -> Result<(), BenchError> {
    let mut state = None;

    for index in 0_i64..i64::from(operations) {
        state = Some(
            memory_engine::next(state.as_ref(), Rating::Good, NOW + index * 60_000)
                .map_err(|error| BenchError::Service(error.to_string()))?,
        );
    }

    Ok(())
}

fn bench_queue(operations: u32) {
    let candidates = bench_candidates(operations.max(3) as usize, "bench");
    let mastery_policy =
        |schedule: &ScheduleState| schedule.state == ScheduleStatus::Review && schedule.reps >= 3;

    for index in 0_usize..operations as usize {
        let start = index % (candidates.len() - 2);
        let recent_candidates = &candidates[start..start + 3];
        let _next = memory_engine::pick_next_queue_candidate(
            &candidates,
            mastery_policy,
            &memory_engine::queue::QueueSelectionOptions {
                now: NOW,
                recent_candidates,
                ..memory_engine::queue::QueueSelectionOptions::default()
            },
        );
    }
}

fn bench_service(operations: u32) -> Result<(), BenchError> {
    let candidates = bench_candidates(operations as usize, "bench-service");
    let store = BenchStore::new(candidates);
    let mastery_policy =
        |schedule: &ScheduleState| schedule.state == ScheduleStatus::Review && schedule.reps >= 3;
    let mut service = MemoryService::with_clock(store, mastery_policy, || NOW);

    for index in 0_u32..operations {
        let review_unit_id = format!("bench-service-{index:04}");
        service
            .grade_apply_review(GradeApplyReviewCommand {
                prompt: bench_prompt(&review_unit_id),
                submitted_answer: "Punishment".to_owned(),
                response_time_ms: 2_000,
                prompt_id: Some(format!("{review_unit_id}-prompt")),
                occurred_at: Some(NOW + i64::from(index) * 60_000),
                idempotency_key: None,
            })
            .map_err(|error| BenchError::Service(error.to_string()))?;
        service
            .next_queue(NextQueueCommand {
                options: NextQueueOptions::default(),
            })
            .map_err(|error| BenchError::Service(error.to_string()))?;
    }

    Ok(())
}

fn print_receipts(receipts: &[BenchReceipt]) {
    println!("memory-engine Rust benchmark receipts");
    println!("name,operations,elapsed_ms,ops_per_ms");

    for receipt in receipts {
        println!(
            "{},{},{:.3},{:.3}",
            receipt.name,
            receipt.operations,
            receipt.elapsed.as_secs_f64() * 1_000.0,
            receipt.ops_per_ms(),
        );
    }
}

fn bench_prompt(review_unit_id: &str) -> Prompt {
    Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: ReviewUnitId::new(review_unit_id),
        prompt: "Translate poena.".to_owned(),
        accepted_answers: vec!["punishment".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    })
}

fn bench_candidates(count: usize, prefix: &str) -> Vec<QueueCandidate> {
    (0..count)
        .map(|index| queue_candidate(prefix, index))
        .collect()
}

fn queue_candidate(prefix: &str, index: usize) -> QueueCandidate {
    let due_offset = if index.is_multiple_of(10) {
        3_600_000
    } else {
        60_000 + i64::try_from(index).expect("benchmark index fits in i64")
    };

    QueueCandidate {
        review_unit_id: ReviewUnitId::new(format!("{prefix}-{index:04}")),
        schedule_state: index.is_multiple_of(3).then(|| {
            schedule_state(
                3 + u32::try_from(index % 7).expect("small index fits"),
                2 + i64::try_from(index % 9).expect("small index fits"),
                NOW - due_offset,
            )
        }),
        due: NOW - due_offset,
        progression: None,
        concept_key: Some(format!("concept-{}", index % 17)),
        source_key: Some(format!("source-{}", index % 11)),
        domain_key: Some(format!("domain-{}", index % 5)),
    }
}

fn schedule_state(reps: u32, scheduled_days: i64, due: i64) -> ScheduleState {
    ScheduleState {
        due,
        stability: 2.3065,
        difficulty: 2.118_103_97,
        elapsed_days: 0,
        scheduled_days,
        reps,
        lapses: 0,
        state: ScheduleStatus::Review,
        last_review: Some(NOW - 86_400_000),
    }
}

#[cfg(test)]
mod tests {
    use super::{benchmark_cases, run_benchmark_cases, smoke_benchmark_cases};

    #[test]
    fn benchmark_receipts_cover_the_migrated_runtime_surfaces() {
        let receipts = run_benchmark_cases(smoke_benchmark_cases()).expect("benchmark receipts");
        let names = receipts
            .iter()
            .map(|receipt| receipt.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            benchmark_cases()
                .iter()
                .map(|bench_case| bench_case.name)
                .collect::<Vec<_>>()
        );
        assert!(receipts
            .iter()
            .all(|receipt| receipt.operations > 0 && receipt.elapsed.as_nanos() > 0));
    }
}
