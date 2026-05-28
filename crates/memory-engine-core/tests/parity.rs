use std::cmp::Ordering;

use memory_engine_core::{
    compare_queue_priority, default_rating_policy, pick_next_queue_candidate, GradeContext,
    ProgressionMetadata, QueueCandidate, QueueSelectionOptions, Rating, ReviewUnitId,
    ScheduleState, ScheduleStatus, Verdict,
};

const NOW: i64 = 1_775_650_400_000;

#[test]
fn rating_policy_matches_the_typescript_matrix() {
    for verdict in [
        Verdict::Correct,
        Verdict::Close,
        Verdict::Wrong,
        Verdict::Revealed,
    ] {
        for response_time_ms in [3_000, 10_000] {
            for prior_reps in [0, 3, 10] {
                let rating = default_rating_policy(
                    verdict,
                    GradeContext {
                        response_time_ms,
                        prior_reps,
                    },
                );
                let expected = match verdict {
                    Verdict::Correct if response_time_ms <= 4_000 && prior_reps >= 3 => {
                        Rating::Easy
                    }
                    Verdict::Correct => Rating::Good,
                    Verdict::Close => Rating::Hard,
                    Verdict::Wrong | Verdict::Revealed => Rating::Again,
                };

                assert_eq!(
                    rating, expected,
                    "unexpected rating for {verdict:?}, {response_time_ms}ms, {prior_reps} reps",
                );
            }
        }
    }
}

#[test]
fn queue_priority_matches_review_urgency_and_tie_break_contract() {
    let urgent = candidate(
        "urgent",
        Some(schedule(ScheduleStatus::Review, 3, 8, NOW - 4 * 86_400_000)),
        NOW - 4 * 86_400_000,
        None,
    );
    let less_urgent = candidate(
        "less-urgent",
        Some(schedule(ScheduleStatus::Review, 9, 8, NOW - 86_400_000)),
        NOW - 86_400_000,
        None,
    );

    assert_eq!(
        compare_queue_priority(&urgent, &less_urgent, NOW),
        Ordering::Less
    );

    let later_stage = candidate(
        "stage-3",
        None,
        NOW - 60_000,
        Some(progression("ladder", 3)),
    );
    let earlier_stage = candidate(
        "stage-1",
        None,
        NOW - 60_000,
        Some(progression("LADDER", 1)),
    );

    assert_eq!(
        compare_queue_priority(&later_stage, &earlier_stage, NOW),
        Ordering::Less
    );

    let lower_reps = candidate(
        "a",
        Some(schedule(ScheduleStatus::Learning, 2, 0, NOW - 60_000)),
        NOW - 60_000,
        None,
    );
    let higher_reps = candidate(
        "b",
        Some(schedule(ScheduleStatus::Learning, 5, 0, NOW - 60_000)),
        NOW - 60_000,
        None,
    );

    assert_eq!(
        compare_queue_priority(&lower_reps, &higher_reps, NOW),
        Ordering::Less,
    );
}

#[test]
fn queue_separation_normalizes_recent_keys_like_typescript() {
    let recent = [QueueCandidate {
        source_key: Some(" Abolition ".to_owned()),
        ..candidate(
            "recent",
            Some(schedule(ScheduleStatus::Review, 3, 8, NOW - 60_000)),
            NOW - 60_000,
            None,
        )
    }];
    let same_source = QueueCandidate {
        source_key: Some("abolition".to_owned()),
        ..candidate(
            "same",
            Some(schedule(ScheduleStatus::Review, 3, 8, NOW - 120_000)),
            NOW - 120_000,
            None,
        )
    };
    let alternative = QueueCandidate {
        source_key: Some("nato".to_owned()),
        ..candidate(
            "alternative",
            Some(schedule(ScheduleStatus::Review, 3, 8, NOW - 100_000)),
            NOW - 100_000,
            None,
        )
    };

    let next = pick_next_queue_candidate(
        &[same_source, alternative],
        |state| state.state == ScheduleStatus::Review && state.reps >= 3,
        &QueueSelectionOptions {
            now: NOW,
            recent_candidates: &recent,
            ..QueueSelectionOptions::default()
        },
    );

    assert_eq!(
        next.map(|entry| entry.review_unit_id),
        Some(ReviewUnitId::new("alternative")),
    );
}

fn schedule(state: ScheduleStatus, reps: u32, scheduled_days: i64, due: i64) -> ScheduleState {
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

fn progression(group: &str, stage_order: u32) -> ProgressionMetadata {
    ProgressionMetadata {
        progression_group: Some(group.to_owned()),
        stage_order,
        requires: Vec::new(),
        supersedes: Vec::new(),
    }
}

fn candidate(
    id: &str,
    schedule_state: Option<ScheduleState>,
    due: i64,
    progression: Option<ProgressionMetadata>,
) -> QueueCandidate {
    QueueCandidate {
        review_unit_id: ReviewUnitId::new(id),
        schedule_state,
        due,
        progression,
        concept_key: None,
        source_key: None,
        domain_key: None,
    }
}
