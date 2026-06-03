use std::fmt;

use crate::{Rating, ScheduleState, ScheduleStatus};

const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;
const S_MIN: f64 = 1e-3;
const S_MAX: f64 = 36_500.0;
const REQUEST_RETENTION: f64 = 0.9;
const LEARNING_STEPS_MINUTES: [i64; 2] = [1, 10];
const RELEARNING_STEPS_MINUTES: [i64; 1] = [10];

const W: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 1e-3, 1.8722, 0.1666, 0.796, 1.4835,
    0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    InvalidTimestamp { field: &'static str, value: i64 },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTimestamp { field, value } => {
                write!(formatter, "{field} timestamp is out of range: {value}")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

pub trait Scheduler {
    /// Advance one schedule state by one rating at a millisecond timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError`] when the provided timestamp is outside the
    /// JavaScript-compatible timestamp range used by the TypeScript oracle.
    fn advance(
        &self,
        state: Option<&ScheduleState>,
        rating: Rating,
        now: i64,
    ) -> Result<ScheduleState, SchedulerError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FsrsScheduler;

impl Scheduler for FsrsScheduler {
    fn advance(
        &self,
        state: Option<&ScheduleState>,
        rating: Rating,
        now: i64,
    ) -> Result<ScheduleState, SchedulerError> {
        validate_timestamp("now", now)?;
        let mut scheduler = FsrsTransition::new(state, now)?;

        Ok(scheduler.review(rating))
    }
}

/// Advance one schedule state using the default FSRS scheduler.
///
/// # Errors
///
/// Returns [`SchedulerError`] when `now`, `state.due`, or
/// `state.last_review` is outside the JavaScript-compatible timestamp range.
pub fn next(
    state: Option<&ScheduleState>,
    rating: Rating,
    now: i64,
) -> Result<ScheduleState, SchedulerError> {
    FsrsScheduler.advance(state, rating, now)
}

#[derive(Clone, Debug)]
struct FsrsTransition {
    last: Card,
    current: Card,
    now: i64,
    elapsed_days: i64,
}

impl FsrsTransition {
    fn new(state: Option<&ScheduleState>, now: i64) -> Result<Self, SchedulerError> {
        let last = state.map_or_else(|| Card::empty(now), Card::from);
        validate_timestamp("due", last.due)?;
        if let Some(last_review) = last.last_review {
            validate_timestamp("last_review", last_review)?;
        }

        let elapsed_days = if last.state == ScheduleStatus::New {
            0
        } else {
            last.last_review
                .map_or(0, |last_review| date_diff_days(last_review, now))
        };
        let mut current = last.clone();
        current.last_review = Some(now);
        current.elapsed_days = elapsed_days;
        current.reps += 1;

        Ok(Self {
            last,
            current,
            now,
            elapsed_days,
        })
    }

    fn review(&mut self, rating: Rating) -> ScheduleState {
        match self.last.state {
            ScheduleStatus::New => self.new_state(rating),
            ScheduleStatus::Learning | ScheduleStatus::Relearning => self.learning_state(rating),
            ScheduleStatus::Review => self.review_state(rating),
        }
        .into()
    }

    fn new_state(&self, rating: Rating) -> Card {
        let mut next = self.current.clone();
        next.difficulty = clamp(init_difficulty(rating), 1.0, 10.0);
        next.stability = init_stability(rating);
        self.apply_learning_steps(&mut next, rating, ScheduleStatus::Learning);

        next
    }

    fn learning_state(&self, rating: Rating) -> Card {
        let mut next = self.current.clone();
        next.difficulty = next_difficulty(self.last.difficulty, rating);
        next.stability = next_short_term_stability(self.last.stability, rating);
        self.apply_learning_steps(&mut next, rating, self.last.state);

        next
    }

    fn review_state(&self, rating: Rating) -> Card {
        let interval = self.elapsed_days;
        let difficulty = self.last.difficulty;
        let stability = self.last.stability;
        let retrievability = forgetting_curve(interval, stability);

        let mut next_again = self.current.clone();
        let mut next_hard = self.current.clone();
        let mut next_good = self.current.clone();
        let mut next_easy = self.current.clone();

        Self::next_difficulty_stability(
            &mut next_again,
            &mut next_hard,
            &mut next_good,
            &mut next_easy,
            difficulty,
            stability,
            retrievability,
        );
        self.next_interval(&mut next_hard, &mut next_good, &mut next_easy, interval);
        Self::set_review_state(&mut next_hard, &mut next_good, &mut next_easy);
        self.apply_learning_steps(&mut next_again, Rating::Again, ScheduleStatus::Relearning);
        next_again.lapses += 1;

        match rating {
            Rating::Again => next_again,
            Rating::Hard => next_hard,
            Rating::Good => next_good,
            Rating::Easy => next_easy,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn next_difficulty_stability(
        next_again: &mut Card,
        next_hard: &mut Card,
        next_good: &mut Card,
        next_easy: &mut Card,
        difficulty: f64,
        stability: f64,
        retrievability: f64,
    ) {
        next_again.difficulty = next_difficulty(difficulty, Rating::Again);
        let next_s_min = stability / f64::exp(W[17] * W[18]);
        let s_after_fail = next_forget_stability(difficulty, stability, retrievability);
        next_again.stability = clamp(round8(next_s_min), S_MIN, s_after_fail);

        next_hard.difficulty = next_difficulty(difficulty, Rating::Hard);
        next_hard.stability =
            next_recall_stability(difficulty, stability, retrievability, Rating::Hard);

        next_good.difficulty = next_difficulty(difficulty, Rating::Good);
        next_good.stability =
            next_recall_stability(difficulty, stability, retrievability, Rating::Good);

        next_easy.difficulty = next_difficulty(difficulty, Rating::Easy);
        next_easy.stability =
            next_recall_stability(difficulty, stability, retrievability, Rating::Easy);
    }

    fn next_interval(
        &self,
        next_hard: &mut Card,
        next_good: &mut Card,
        next_easy: &mut Card,
        elapsed_days: i64,
    ) {
        let mut hard_interval = next_interval(next_hard.stability, elapsed_days);
        let mut good_interval = next_interval(next_good.stability, elapsed_days);
        hard_interval = hard_interval.min(good_interval);
        good_interval = good_interval.max(hard_interval + 1);
        let easy_interval = next_interval(next_easy.stability, elapsed_days).max(good_interval + 1);

        next_hard.scheduled_days = hard_interval;
        next_hard.due = self.now + hard_interval * DAY_MS;

        next_good.scheduled_days = good_interval;
        next_good.due = self.now + good_interval * DAY_MS;

        next_easy.scheduled_days = easy_interval;
        next_easy.due = self.now + easy_interval * DAY_MS;
    }

    fn set_review_state(next_hard: &mut Card, next_good: &mut Card, next_easy: &mut Card) {
        next_hard.state = ScheduleStatus::Review;
        next_hard.learning_steps = 0;
        next_good.state = ScheduleStatus::Review;
        next_good.learning_steps = 0;
        next_easy.state = ScheduleStatus::Review;
        next_easy.learning_steps = 0;
    }

    fn apply_learning_steps(&self, next: &mut Card, rating: Rating, to_state: ScheduleStatus) {
        let learning = learning_info(self.current.state, self.current.learning_steps, rating);
        if learning.scheduled_minutes > 0 && learning.scheduled_minutes < 1_440 {
            next.learning_steps = learning.next_step;
            next.scheduled_days = 0;
            next.state = to_state;
            next.due = self.now + learning.scheduled_minutes * MINUTE_MS;
        } else {
            next.state = ScheduleStatus::Review;
            if learning.scheduled_minutes >= 1_440 {
                next.learning_steps = learning.next_step;
                next.due = self.now + learning.scheduled_minutes * MINUTE_MS;
                next.scheduled_days = learning.scheduled_minutes / 1_440;
            } else {
                next.learning_steps = 0;
                let interval = next_interval(next.stability, self.elapsed_days);
                next.scheduled_days = interval;
                next.due = self.now + interval * DAY_MS;
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Card {
    due: i64,
    stability: f64,
    difficulty: f64,
    elapsed_days: i64,
    scheduled_days: i64,
    reps: u32,
    lapses: u32,
    state: ScheduleStatus,
    last_review: Option<i64>,
    learning_steps: usize,
}

impl Card {
    fn empty(now: i64) -> Self {
        Self {
            due: now,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: 0,
            state: ScheduleStatus::New,
            last_review: None,
            learning_steps: 0,
        }
    }
}

impl From<&ScheduleState> for Card {
    fn from(state: &ScheduleState) -> Self {
        Self {
            due: state.due,
            stability: state.stability,
            difficulty: state.difficulty,
            elapsed_days: state.elapsed_days,
            scheduled_days: state.scheduled_days,
            reps: state.reps,
            lapses: state.lapses,
            state: state.state,
            last_review: state.last_review,
            learning_steps: 0,
        }
    }
}

impl From<Card> for ScheduleState {
    fn from(card: Card) -> Self {
        Self {
            due: card.due,
            stability: card.stability,
            difficulty: card.difficulty,
            elapsed_days: card.elapsed_days,
            scheduled_days: card.scheduled_days,
            reps: card.reps,
            lapses: card.lapses,
            state: card.state,
            last_review: card.last_review,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LearningInfo {
    scheduled_minutes: i64,
    next_step: usize,
}

fn learning_info(state: ScheduleStatus, current_step: usize, rating: Rating) -> LearningInfo {
    let learning_steps = if matches!(state, ScheduleStatus::Review | ScheduleStatus::Relearning) {
        &RELEARNING_STEPS_MINUTES[..]
    } else {
        &LEARNING_STEPS_MINUTES[..]
    };
    let current_step =
        if state == ScheduleStatus::Learning && rating != Rating::Again && rating != Rating::Hard {
            current_step + 1
        } else {
            current_step
        };

    if learning_steps.is_empty() || current_step >= learning_steps.len() {
        return LearningInfo {
            scheduled_minutes: 0,
            next_step: 0,
        };
    }

    if state == ScheduleStatus::Review {
        return if rating == Rating::Again {
            LearningInfo {
                scheduled_minutes: learning_steps[current_step],
                next_step: 0,
            }
        } else {
            LearningInfo {
                scheduled_minutes: 0,
                next_step: 0,
            }
        };
    }

    match rating {
        Rating::Again => LearningInfo {
            scheduled_minutes: learning_steps[0],
            next_step: 0,
        },
        Rating::Hard => {
            let scheduled_minutes = if learning_steps.len() == 1 {
                (learning_steps[0] * 3 + 1) / 2
            } else {
                (learning_steps[0] + learning_steps[1] + 1) / 2
            };
            LearningInfo {
                scheduled_minutes,
                next_step: current_step,
            }
        }
        Rating::Good => learning_steps.get(current_step + 1).map_or(
            LearningInfo {
                scheduled_minutes: 0,
                next_step: 0,
            },
            |next_step| LearningInfo {
                scheduled_minutes: *next_step,
                next_step: current_step + 1,
            },
        ),
        Rating::Easy => LearningInfo {
            scheduled_minutes: 0,
            next_step: 0,
        },
    }
}

fn init_stability(rating: Rating) -> f64 {
    W[rating_index(rating)].max(0.1)
}

fn init_difficulty(rating: Rating) -> f64 {
    round8(W[4] - f64::exp((rating_value(rating) - 1.0) * W[5]) + 1.0)
}

fn next_interval(stability: f64, _elapsed_days: i64) -> i64 {
    let interval = (stability * interval_modifier()).round();
    rounded_interval(clamp(interval, 1.0, MAXIMUM_INTERVAL_F64))
}

const MAXIMUM_INTERVAL_F64: f64 = 36_500.0;

#[allow(clippy::cast_possible_truncation)]
fn rounded_interval(value: f64) -> i64 {
    value.round() as i64
}

fn linear_damping(delta_d: f64, old_d: f64) -> f64 {
    round8(delta_d * (10.0 - old_d) / 9.0)
}

fn next_difficulty(difficulty: f64, rating: Rating) -> f64 {
    let delta_d = -W[6] * (rating_value(rating) - 3.0);
    let next_d = difficulty + linear_damping(delta_d, difficulty);
    clamp(
        mean_reversion(init_difficulty(Rating::Easy), next_d),
        1.0,
        10.0,
    )
}

fn mean_reversion(init: f64, current: f64) -> f64 {
    round8(W[7] * init + (1.0 - W[7]) * current)
}

fn next_recall_stability(
    difficulty: f64,
    stability: f64,
    retrievability: f64,
    rating: Rating,
) -> f64 {
    let hard_penalty = if rating == Rating::Hard { W[15] } else { 1.0 };
    let easy_bound = if rating == Rating::Easy { W[16] } else { 1.0 };
    round8(clamp(
        stability
            * (1.0
                + f64::exp(W[8])
                    * (11.0 - difficulty)
                    * stability.powf(-W[9])
                    * (f64::exp((1.0 - retrievability) * W[10]) - 1.0)
                    * hard_penalty
                    * easy_bound),
        S_MIN,
        S_MAX,
    ))
}

fn next_forget_stability(difficulty: f64, stability: f64, retrievability: f64) -> f64 {
    round8(clamp(
        W[11]
            * difficulty.powf(-W[12])
            * ((stability + 1.0).powf(W[13]) - 1.0)
            * f64::exp((1.0 - retrievability) * W[14]),
        S_MIN,
        S_MAX,
    ))
}

fn next_short_term_stability(stability: f64, rating: Rating) -> f64 {
    let sinc = stability.powf(-W[19]) * f64::exp(W[17] * (rating_value(rating) - 3.0 + W[18]));
    let masked_sinc = if rating_value(rating) >= 3.0 {
        sinc.max(1.0)
    } else {
        sinc
    };

    round8(clamp(stability * masked_sinc, S_MIN, S_MAX))
}

fn forgetting_curve(elapsed_days: i64, stability: f64) -> f64 {
    let decay = -W[20];
    let factor = compute_decay_factor();
    round8((1.0 + factor * days_as_f64(elapsed_days) / stability).powf(decay))
}

#[allow(clippy::cast_precision_loss)]
fn days_as_f64(value: i64) -> f64 {
    value as f64
}

fn interval_modifier() -> f64 {
    let decay = -W[20];
    let factor = compute_decay_factor();
    round8((REQUEST_RETENTION.powf(1.0 / decay) - 1.0) / factor)
}

fn compute_decay_factor() -> f64 {
    let decay = -W[20];
    round8(f64::exp(decay.powf(-1.0) * f64::ln(0.9)) - 1.0)
}

fn date_diff_days(last_review: i64, now: i64) -> i64 {
    utc_day(now) - utc_day(last_review)
}

fn utc_day(timestamp: i64) -> i64 {
    timestamp.div_euclid(DAY_MS)
}

fn validate_timestamp(field: &'static str, value: i64) -> Result<(), SchedulerError> {
    // JavaScript Date accepts a finite IEEE-754 millisecond timestamp. Keep a
    // conservative check so invalid edge input fails before arithmetic.
    if value.abs() > 8_640_000_000_000_000 {
        Err(SchedulerError::InvalidTimestamp { field, value })
    } else {
        Ok(())
    }
}

fn rating_index(rating: Rating) -> usize {
    match rating {
        Rating::Again => 0,
        Rating::Hard => 1,
        Rating::Good => 2,
        Rating::Easy => 3,
    }
}

fn rating_value(rating: Rating) -> f64 {
    match rating {
        Rating::Again => 1.0,
        Rating::Hard => 2.0,
        Rating::Good => 3.0,
        Rating::Easy => 4.0,
    }
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

fn round8(value: f64) -> f64 {
    (value * 100_000_000.0).round() / 100_000_000.0
}

#[cfg(test)]
mod tests {
    use crate::{Rating, ScheduleState, ScheduleStatus};

    use super::next;

    #[test]
    fn matches_typescript_scheduler_fixtures() {
        for fixture in fixtures() {
            let actual = next(Some(&fixture.initial_state), fixture.rating, fixture.now)
                .expect("valid schedule");

            assert_schedule_eq(&actual, &fixture.expected);
        }
    }

    #[test]
    fn schedules_empty_state_like_typescript_create_empty_card() {
        let actual = next(None, Rating::Good, 0).expect("valid schedule");

        assert_schedule_eq(
            &actual,
            &ScheduleState {
                due: 600_000,
                stability: 2.3065,
                difficulty: 2.118_103_97,
                elapsed_days: 0,
                scheduled_days: 0,
                reps: 1,
                lapses: 0,
                state: ScheduleStatus::Learning,
                last_review: Some(0),
            },
        );
    }

    struct Fixture {
        initial_state: ScheduleState,
        rating: Rating,
        now: i64,
        expected: ScheduleState,
    }

    #[allow(clippy::too_many_lines)]
    fn fixtures() -> Vec<Fixture> {
        vec![
            Fixture {
                initial_state: ScheduleState {
                    due: 0,
                    stability: 0.0,
                    difficulty: 0.0,
                    elapsed_days: 0,
                    scheduled_days: 0,
                    reps: 0,
                    lapses: 0,
                    state: ScheduleStatus::New,
                    last_review: None,
                },
                rating: Rating::Good,
                now: 0,
                expected: ScheduleState {
                    due: 600_000,
                    stability: 2.3065,
                    difficulty: 2.118_103_97,
                    elapsed_days: 0,
                    scheduled_days: 0,
                    reps: 1,
                    lapses: 0,
                    state: ScheduleStatus::Learning,
                    last_review: Some(0),
                },
            },
            Fixture {
                initial_state: ScheduleState {
                    due: 600_000,
                    stability: 2.3065,
                    difficulty: 2.118_103_97,
                    elapsed_days: 0,
                    scheduled_days: 0,
                    reps: 1,
                    lapses: 0,
                    state: ScheduleStatus::Learning,
                    last_review: Some(0),
                },
                rating: Rating::Good,
                now: 600_000,
                expected: ScheduleState {
                    due: 173_400_000,
                    stability: 2.3065,
                    difficulty: 2.111_214_24,
                    elapsed_days: 0,
                    scheduled_days: 2,
                    reps: 2,
                    lapses: 0,
                    state: ScheduleStatus::Review,
                    last_review: Some(600_000),
                },
            },
            Fixture {
                initial_state: ScheduleState {
                    due: 173_400_000,
                    stability: 2.3065,
                    difficulty: 2.111_214_24,
                    elapsed_days: 0,
                    scheduled_days: 2,
                    reps: 2,
                    lapses: 0,
                    state: ScheduleStatus::Review,
                    last_review: Some(600_000),
                },
                rating: Rating::Again,
                now: 173_400_000,
                expected: ScheduleState {
                    due: 174_000_000,
                    stability: 0.607_701_66,
                    difficulty: 7.392_238_14,
                    elapsed_days: 2,
                    scheduled_days: 0,
                    reps: 3,
                    lapses: 1,
                    state: ScheduleStatus::Relearning,
                    last_review: Some(173_400_000),
                },
            },
            Fixture {
                initial_state: ScheduleState {
                    due: 174_000_000,
                    stability: 0.607_701_66,
                    difficulty: 7.392_238_14,
                    elapsed_days: 2,
                    scheduled_days: 0,
                    reps: 3,
                    lapses: 1,
                    state: ScheduleStatus::Relearning,
                    last_review: Some(173_400_000),
                },
                rating: Rating::Good,
                now: 174_000_000,
                expected: ScheduleState {
                    due: 260_400_000,
                    stability: 0.659_797_62,
                    difficulty: 7.380_074_27,
                    elapsed_days: 0,
                    scheduled_days: 1,
                    reps: 4,
                    lapses: 1,
                    state: ScheduleStatus::Review,
                    last_review: Some(174_000_000),
                },
            },
        ]
    }

    fn assert_schedule_eq(actual: &ScheduleState, expected: &ScheduleState) {
        assert_eq!(actual.due, expected.due);
        assert_float_eq(actual.stability, expected.stability);
        assert_float_eq(actual.difficulty, expected.difficulty);
        assert_eq!(actual.elapsed_days, expected.elapsed_days);
        assert_eq!(actual.scheduled_days, expected.scheduled_days);
        assert_eq!(actual.reps, expected.reps);
        assert_eq!(actual.lapses, expected.lapses);
        assert_eq!(actual.state, expected.state);
        assert_eq!(actual.last_review, expected.last_review);
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.000_000_01,
            "expected {actual} to equal {expected}",
        );
    }
}
