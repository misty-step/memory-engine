use memory_engine_core::{next, Rating, ScheduleState, ScheduleStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerFixture {
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

#[test]
fn replays_shared_typescript_scheduler_fixtures() {
    let fixtures: Vec<SchedulerFixture> =
        serde_json::from_str(include_str!("../../../fixtures/scheduler.json"))
            .expect("valid scheduler fixtures");

    for fixture in fixtures {
        let actual = next(
            Some(&fixture.initial_state.into_schedule_state()),
            into_rating(fixture.rating),
            fixture.now,
        )
        .unwrap_or_else(|error| panic!("{} should schedule: {error}", fixture.name));
        let expected = fixture.expected.into_schedule_state();

        assert_schedule_eq(&actual, &expected, &fixture.name);
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

fn into_status(value: u8) -> ScheduleStatus {
    match value {
        0 => ScheduleStatus::New,
        1 => ScheduleStatus::Learning,
        2 => ScheduleStatus::Review,
        3 => ScheduleStatus::Relearning,
        _ => panic!("invalid fixture schedule state: {value}"),
    }
}

fn assert_schedule_eq(actual: &ScheduleState, expected: &ScheduleState, name: &str) {
    assert_eq!(actual.due, expected.due, "{name}: due");
    assert_float_eq(actual.stability, expected.stability, name, "stability");
    assert_float_eq(actual.difficulty, expected.difficulty, name, "difficulty");
    assert_eq!(
        actual.elapsed_days, expected.elapsed_days,
        "{name}: elapsed_days"
    );
    assert_eq!(
        actual.scheduled_days, expected.scheduled_days,
        "{name}: scheduled_days"
    );
    assert_eq!(actual.reps, expected.reps, "{name}: reps");
    assert_eq!(actual.lapses, expected.lapses, "{name}: lapses");
    assert_eq!(actual.state, expected.state, "{name}: state");
    assert_eq!(
        actual.last_review, expected.last_review,
        "{name}: last_review"
    );
}

fn assert_float_eq(actual: f64, expected: f64, name: &str, field: &str) {
    assert!(
        (actual - expected).abs() < 0.000_000_01,
        "{name}: expected {field} {actual} to equal {expected}",
    );
}
