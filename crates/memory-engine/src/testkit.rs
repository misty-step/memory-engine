//! Consumer fixture corpora for Rust integration tests.
//!
//! These fixtures mirror the TypeScript `memory-engine/testkit` surface while
//! keeping fixture construction outside the reusable kernel crates.

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, GradeContext, GradeResult, GraderKind, ProgressionCandidate,
    ProgressionMetadata, Prompt, QueueCandidate, Rating, ReviewUnitId, ReviewUnitLifecycle,
    ScheduleState, ScheduleStatus, Verdict,
};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slice2MasteryPolicy {
    Ruminatio,
    Vault,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GradingFixture {
    pub name: String,
    pub prompt: Prompt,
    pub submitted: String,
    pub context: GradeContext,
    pub expected: GradeResult,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecitationFixture {
    pub name: String,
    pub prompt: ExactPrompt,
    pub submitted: String,
    pub context: GradeContext,
    pub expected: GradeResult,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerFixture {
    pub name: String,
    pub initial_state: Option<ScheduleState>,
    pub rating: Rating,
    pub now: i64,
    pub expected: ScheduleState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressionFixtureReview {
    pub reps: u32,
    pub state: ScheduleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProgressionFixtureMode {
    Strict,
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressionFixture {
    pub name: String,
    pub mode: ProgressionFixtureMode,
    pub mastery_policy: Slice2MasteryPolicy,
    pub candidates: Vec<ProgressionCandidate<ProgressionFixtureReview>>,
    pub population: Option<Vec<ProgressionCandidate<ProgressionFixtureReview>>>,
    pub expected_available_review_unit_ids: Vec<ReviewUnitId>,
    pub expected_locked_fresh_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueFixture {
    pub name: String,
    pub mastery_policy: Slice2MasteryPolicy,
    pub candidates: Vec<QueueCandidate>,
    pub now: i64,
    pub recent_candidates: Vec<QueueCandidate>,
    pub population: Option<Vec<QueueCandidate>>,
    pub recent_source_window: Option<usize>,
    pub expected_next_review_unit_id: Option<ReviewUnitId>,
}

#[must_use]
pub fn grading_fixtures() -> Vec<GradingFixture> {
    let mut fixtures = base_grading_fixtures();
    fixtures.extend(
        recitation_fixtures()
            .into_iter()
            .map(|fixture| GradingFixture {
                name: fixture.name,
                prompt: Prompt::Exact(fixture.prompt),
                submitted: fixture.submitted,
                context: fixture.context,
                expected: fixture.expected,
            }),
    );
    fixtures
}

#[must_use]
pub fn recitation_fixtures() -> Vec<RecitationFixture> {
    vec![RecitationFixture {
        name: "recitation grades long-form deterministic recall from the Ruminatio study oracle"
            .to_owned(),
        prompt: exact_prompt(
            ExactPromptKind::Recitation,
            "recitation-glory-be",
            "Recite the prayer.",
            &["Glory be to the Father, and to the Son, and to the Holy Spirit."],
        ),
        submitted: "Glory be to the Father and to the Son and to the Holy Spirit".to_owned(),
        context: GradeContext {
            response_time_ms: 3_200,
            prior_reps: 1,
        },
        expected: deterministic_grade(
            Verdict::Correct,
            Rating::Good,
            "Glory be to the Father and to the Son and to the Holy Spirit",
            "Glory be to the Father, and to the Son, and to the Holy Spirit.",
            true,
        ),
    }]
}

#[must_use]
/// # Panics
///
/// Panics if the checked-in scheduler fixture JSON is malformed. This is a
/// repository integrity failure, not a consumer input error.
pub fn scheduler_fixtures() -> Vec<SchedulerFixture> {
    let fixtures: Vec<SchedulerJsonFixture> =
        serde_json::from_str(include_str!("../../../fixtures/scheduler.json"))
            .expect("valid scheduler fixtures");

    fixtures
        .into_iter()
        .map(|fixture| SchedulerFixture {
            name: fixture.name,
            initial_state: Some(fixture.initial_state.into_schedule_state()),
            rating: into_rating(fixture.rating),
            now: fixture.now,
            expected: fixture.expected.into_schedule_state(),
        })
        .collect()
}

#[must_use]
pub fn progression_fixtures() -> Vec<ProgressionFixture> {
    vec![
        ProgressionFixture {
            name: "progression unlocks a later stage using the wider population".to_owned(),
            mode: ProgressionFixtureMode::Strict,
            mastery_policy: Slice2MasteryPolicy::Ruminatio,
            candidates: vec![progression_candidate(
                "a-stage-2",
                None,
                Some(progression(" concept-a ", 2, &[], &[])),
            )],
            population: Some(vec![
                progression_candidate(
                    "a-stage-1",
                    Some(ProgressionFixtureReview {
                        state: ScheduleStatus::Review,
                        reps: 2,
                    }),
                    Some(progression("Concept-A", 1, &[], &[])),
                ),
                progression_candidate(
                    "a-stage-2",
                    None,
                    Some(progression(" concept-a ", 2, &[], &[])),
                ),
            ]),
            expected_available_review_unit_ids: vec![review_unit_id("a-stage-2")],
            expected_locked_fresh_count: 0,
        },
        ProgressionFixture {
            name: "progression suppresses superseded units once a harder stage is mastered"
                .to_owned(),
            mode: ProgressionFixtureMode::Strict,
            mastery_policy: Slice2MasteryPolicy::Vault,
            candidates: vec![progression_candidate(
                "st-michael-01",
                None,
                Some(progression("st-michael-prayer", 1, &[], &[])),
            )],
            population: Some(vec![
                progression_candidate(
                    "st-michael-01",
                    None,
                    Some(progression("st-michael-prayer", 1, &[], &[])),
                ),
                progression_candidate(
                    "st-michael-03",
                    Some(ProgressionFixtureReview {
                        state: ScheduleStatus::Review,
                        reps: 4,
                    }),
                    Some(progression("st-michael-prayer", 3, &[], &["st-michael-01"])),
                ),
            ]),
            expected_available_review_unit_ids: Vec::new(),
            expected_locked_fresh_count: 1,
        },
        ProgressionFixture {
            name: "progression fallback returns the locked stage when nothing else is available"
                .to_owned(),
            mode: ProgressionFixtureMode::Fallback,
            mastery_policy: Slice2MasteryPolicy::Vault,
            candidates: vec![progression_candidate(
                "creed-02",
                None,
                Some(progression("nicene-creed", 2, &["missing-prereq"], &[])),
            )],
            population: None,
            expected_available_review_unit_ids: vec![review_unit_id("creed-02")],
            expected_locked_fresh_count: 1,
        },
    ]
}

#[must_use]
pub fn queue_fixtures() -> Vec<QueueFixture> {
    let now = 1_775_650_400_000;
    vec![
        queue_prefers_review_fixture(now),
        queue_avoids_source_clump_fixture(now),
        queue_disable_source_window_fixture(now),
        queue_progression_fallback_fixture(now),
    ]
}

fn base_grading_fixtures() -> Vec<GradingFixture> {
    vec![
        mcq_correct_fixture(),
        mcq_wrong_fixture(),
        boolean_correct_fixture(),
        short_answer_good_fixture(),
        short_answer_close_fixture(),
        cloze_equivalence_fixture(),
        short_answer_independent_near_miss_fixture(),
    ]
}

fn queue_prefers_review_fixture(now: i64) -> QueueFixture {
    QueueFixture {
        name: "queue prefers review candidates over fresh ones when both are due".to_owned(),
        mastery_policy: Slice2MasteryPolicy::Vault,
        now,
        candidates: vec![
            queue_candidate(
                "latin-01",
                None,
                now - 60_000,
                None,
                None,
                Some("mass-core"),
            ),
            queue_candidate(
                "mass-01",
                Some(schedule_state(
                    ScheduleStatus::Review,
                    4,
                    5,
                    now - 3_600_000,
                )),
                now - 3_600_000,
                None,
                None,
                Some("mass-core"),
            ),
        ],
        recent_candidates: Vec::new(),
        population: None,
        recent_source_window: None,
        expected_next_review_unit_id: Some(review_unit_id("mass-01")),
    }
}

fn queue_avoids_source_clump_fixture(now: i64) -> QueueFixture {
    QueueFixture {
        name: "queue avoids immediate same-source clumps when an alternative exists".to_owned(),
        mastery_policy: Slice2MasteryPolicy::Vault,
        now,
        candidates: source_clump_candidates(now),
        recent_candidates: recent_abolition_candidates(),
        population: None,
        recent_source_window: None,
        expected_next_review_unit_id: Some(review_unit_id("nato-01")),
    }
}

fn queue_disable_source_window_fixture(now: i64) -> QueueFixture {
    QueueFixture {
        name: "queue allows callers to disable source clumping with a zero source window"
            .to_owned(),
        mastery_policy: Slice2MasteryPolicy::Vault,
        now,
        candidates: source_clump_candidates(now),
        recent_candidates: recent_abolition_candidates(),
        population: None,
        recent_source_window: Some(0),
        expected_next_review_unit_id: Some(review_unit_id("abolition-11")),
    }
}

fn queue_progression_fallback_fixture(now: i64) -> QueueFixture {
    QueueFixture {
        name: "queue falls back to an unsatisfied stage instead of hiding it forever".to_owned(),
        mastery_policy: Slice2MasteryPolicy::Vault,
        now,
        candidates: vec![queue_candidate(
            "creed-02",
            None,
            now - 60_000,
            Some(progression("nicene-creed", 2, &["missing-prereq"], &[])),
            None,
            None,
        )],
        recent_candidates: Vec::new(),
        population: None,
        recent_source_window: None,
        expected_next_review_unit_id: Some(review_unit_id("creed-02")),
    }
}

fn source_clump_candidates(now: i64) -> Vec<QueueCandidate> {
    vec![
        queue_candidate(
            "abolition-11",
            Some(schedule_state(ScheduleStatus::Review, 3, 8, now - 120_000)),
            now - 120_000,
            None,
            None,
            Some("abolition-of-man"),
        ),
        queue_candidate(
            "nato-01",
            Some(schedule_state(ScheduleStatus::Review, 3, 8, now - 100_000)),
            now - 100_000,
            None,
            None,
            Some("nato-phonetic"),
        ),
    ]
}

fn recent_abolition_candidates() -> Vec<QueueCandidate> {
    vec![queue_candidate(
        "abolition-10",
        Some(schedule_state(ScheduleStatus::Review, 3, 8, 0)),
        0,
        None,
        None,
        Some("abolition-of-man"),
    )]
}

fn mcq_correct_fixture() -> GradingFixture {
    GradingFixture {
        name: "mcq trims the submission and grades the correct choice".to_owned(),
        prompt: Prompt::Mcq {
            review_unit_id: review_unit_id("mcq-correct"),
            prompt: "Pick one".to_owned(),
            choices: strings(&["Alpha", "Beta", "Gamma"]),
            correct_choice: "Alpha".to_owned(),
        },
        submitted: " Alpha ".to_owned(),
        context: grade_context(5_100, 0),
        expected: deterministic_grade(Verdict::Correct, Rating::Good, "Alpha", "Alpha", true),
    }
}

fn mcq_wrong_fixture() -> GradingFixture {
    GradingFixture {
        name: "mcq grades an incorrect choice as wrong".to_owned(),
        prompt: Prompt::Mcq {
            review_unit_id: review_unit_id("mcq-wrong"),
            prompt: "Pick one".to_owned(),
            choices: strings(&["Alpha", "Beta", "Gamma"]),
            correct_choice: "Alpha".to_owned(),
        },
        submitted: "Beta".to_owned(),
        context: grade_context(5_100, 0),
        expected: deterministic_grade(Verdict::Wrong, Rating::Again, "Beta", "Alpha", false),
    }
}

fn boolean_correct_fixture() -> GradingFixture {
    GradingFixture {
        name: "boolean uses normalized exact matching".to_owned(),
        prompt: Prompt::Boolean {
            review_unit_id: review_unit_id("boolean-correct"),
            prompt: "Is this true?".to_owned(),
            correct_answer: true,
        },
        submitted: " true ".to_owned(),
        context: grade_context(3_000, 3),
        expected: deterministic_grade(Verdict::Correct, Rating::Easy, "true", "True", true),
    }
}

fn short_answer_good_fixture() -> GradingFixture {
    GradingFixture {
        name: "short answer marks exact answers as good".to_owned(),
        prompt: Prompt::Exact(exact_prompt(
            ExactPromptKind::ShortAnswer,
            "short-answer-good",
            "Answer?",
            &["punishment"],
        )),
        submitted: "Punishment".to_owned(),
        context: grade_context(5_100, 0),
        expected: deterministic_grade(
            Verdict::Correct,
            Rating::Good,
            "Punishment",
            "punishment",
            true,
        ),
    }
}

fn short_answer_close_fixture() -> GradingFixture {
    GradingFixture {
        name: "short answer marks near misses as hard".to_owned(),
        prompt: Prompt::Exact(exact_prompt(
            ExactPromptKind::ShortAnswer,
            "short-answer-close",
            "Answer?",
            &["punishment"],
        )),
        submitted: "punishmant".to_owned(),
        context: grade_context(5_100, 0),
        expected: deterministic_grade(
            Verdict::Close,
            Rating::Hard,
            "punishmant",
            "punishment",
            false,
        ),
    }
}

fn cloze_equivalence_fixture() -> GradingFixture {
    GradingFixture {
        name: "cloze honors token equivalence groups".to_owned(),
        prompt: Prompt::Exact(ExactPrompt {
            equivalence_groups: vec![strings(&["o", "oh"])],
            ..exact_prompt(
                ExactPromptKind::Cloze,
                "cloze-equivalence",
                "Respond.",
                &["Glory to you, O Lord"],
            )
        }),
        submitted: "Glory to you oh lord".to_owned(),
        context: grade_context(3_500, 0),
        expected: deterministic_grade(
            Verdict::Correct,
            Rating::Good,
            "Glory to you oh lord",
            "Glory to you, O Lord",
            true,
        ),
    }
}

fn short_answer_independent_near_miss_fixture() -> GradingFixture {
    GradingFixture {
        name: "short answer evaluates near misses against each accepted answer independently"
            .to_owned(),
        prompt: Prompt::Exact(exact_prompt(
            ExactPromptKind::ShortAnswer,
            "short-answer-accepted-near-miss",
            "Answer?",
            &["Q", "Quebec"],
        )),
        submitted: "Quebecc".to_owned(),
        context: grade_context(5_100, 0),
        expected: deterministic_grade(Verdict::Close, Rating::Hard, "Quebecc", "Q / Quebec", false),
    }
}

fn grade_context(response_time_ms: u32, prior_reps: u32) -> GradeContext {
    GradeContext {
        response_time_ms,
        prior_reps,
    }
}

fn deterministic_grade(
    verdict: Verdict,
    rating: Rating,
    submitted_answer: &str,
    expected_answer: &str,
    is_correct: bool,
) -> GradeResult {
    GradeResult {
        verdict,
        rating,
        is_correct,
        submitted_answer: submitted_answer.to_owned(),
        expected_answer: expected_answer.to_owned(),
        grader_kind: GraderKind::Deterministic,
        grader_model: None,
        grader_confidence: None,
        feedback: String::new(),
        criterion_results: Vec::new(),
    }
}

fn exact_prompt(
    kind: ExactPromptKind,
    review_unit_id: &str,
    prompt: &str,
    accepted_answers: &[&str],
) -> ExactPrompt {
    ExactPrompt {
        kind,
        review_unit_id: ReviewUnitId::new(review_unit_id),
        prompt: prompt.to_owned(),
        accepted_answers: strings(accepted_answers),
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    }
}

fn progression_candidate(
    review_unit_id: &str,
    review: Option<ProgressionFixtureReview>,
    progression: Option<ProgressionMetadata>,
) -> ProgressionCandidate<ProgressionFixtureReview> {
    ProgressionCandidate {
        review_unit_id: ReviewUnitId::new(review_unit_id),
        review,
        progression,
    }
}

fn progression(
    progression_group: &str,
    stage_order: u32,
    requires: &[&str],
    supersedes: &[&str],
) -> ProgressionMetadata {
    ProgressionMetadata {
        progression_group: Some(progression_group.to_owned()),
        stage_order,
        requires: requires.iter().map(|id| review_unit_id(id)).collect(),
        supersedes: supersedes.iter().map(|id| review_unit_id(id)).collect(),
    }
}

fn queue_candidate(
    review_unit_id: &str,
    schedule_state: Option<ScheduleState>,
    due: i64,
    progression: Option<ProgressionMetadata>,
    concept_key: Option<&str>,
    source_key: Option<&str>,
) -> QueueCandidate {
    QueueCandidate {
        review_unit_id: ReviewUnitId::new(review_unit_id),
        schedule_state,
        due,
        lifecycle: ReviewUnitLifecycle::active(),
        progression,
        concept_key: concept_key.map(str::to_owned),
        source_key: source_key.map(str::to_owned),
        domain_key: None,
    }
}

fn schedule_state(
    state: ScheduleStatus,
    reps: u32,
    scheduled_days: i64,
    due: i64,
) -> ScheduleState {
    ScheduleState {
        due,
        stability: 0.0,
        difficulty: 0.0,
        elapsed_days: 0,
        scheduled_days,
        reps,
        lapses: 0,
        state,
        last_review: None,
    }
}

fn review_unit_id(value: &str) -> ReviewUnitId {
    ReviewUnitId::new(value)
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerJsonFixture {
    name: String,
    initial_state: WireScheduleState,
    rating: u8,
    now: i64,
    expected: WireScheduleState,
}

#[derive(Debug, Deserialize)]
struct WireScheduleState {
    due: i64,
    stability: f64,
    difficulty: f64,
    elapsed_days: i64,
    scheduled_days: i64,
    reps: u32,
    lapses: u32,
    state: u8,
    last_review: Option<i64>,
}

impl WireScheduleState {
    fn into_schedule_state(self) -> ScheduleState {
        ScheduleState {
            due: self.due,
            stability: self.stability,
            difficulty: self.difficulty,
            elapsed_days: self.elapsed_days,
            scheduled_days: self.scheduled_days,
            reps: self.reps,
            lapses: self.lapses,
            state: into_status(self.state),
            last_review: self.last_review,
        }
    }
}

fn into_rating(value: u8) -> Rating {
    match value {
        1 => Rating::Again,
        2 => Rating::Hard,
        3 => Rating::Good,
        4 => Rating::Easy,
        _ => panic!("invalid fixture rating: {value}"),
    }
}

fn into_status(value: u8) -> ScheduleStatus {
    match value {
        0 => ScheduleStatus::New,
        1 => ScheduleStatus::Learning,
        2 => ScheduleStatus::Review,
        3 => ScheduleStatus::Relearning,
        _ => panic!("invalid fixture schedule state: {value}"),
    }
}
