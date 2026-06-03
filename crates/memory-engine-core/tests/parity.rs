use std::cmp::Ordering;

use memory_engine_core::{
    compare_queue_priority, default_rating_policy, pick_next_queue_candidate, ExactPrompt,
    ExactPromptKind, GradeContext, GradeResult, GraderKind, ProgressionMetadata, Prompt,
    QueueCandidate, QueueSelectionOptions, Rating, ReviewUnitId, ScheduleState, ScheduleStatus,
    Verdict,
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

#[test]
fn beta_wire_serialization_matches_typescript_prompt_and_grade_shapes() {
    let prompt = Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: ReviewUnitId::new("unit-1"),
        prompt: "Translate: Pater noster".to_owned(),
        accepted_answers: vec!["Our Father".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: vec![".".to_owned()],
    });
    let encoded = serde_json::to_value(&prompt).expect("prompt json");

    assert_eq!(
        encoded,
        serde_json::json!({
            "kind": "shortAnswer",
            "reviewUnitId": "unit-1",
            "prompt": "Translate: Pater noster",
            "acceptedAnswers": ["Our Father"],
            "equivalenceGroups": [],
            "ignoredTokens": ["."]
        })
    );
    assert_eq!(
        serde_json::from_value::<Prompt>(encoded).expect("prompt decode"),
        prompt
    );

    let grade = GradeResult {
        verdict: Verdict::Correct,
        rating: Rating::Good,
        is_correct: true,
        submitted_answer: "Our Father".to_owned(),
        expected_answer: "Our Father".to_owned(),
        grader_kind: GraderKind::Deterministic,
        grader_model: None,
        grader_confidence: None,
        feedback: String::new(),
        criterion_results: Vec::new(),
    };
    let encoded = serde_json::to_value(&grade).expect("grade json");

    assert_eq!(encoded["verdict"], "correct");
    assert_eq!(encoded["rating"], 3);
    assert_eq!(encoded["isCorrect"], true);
    assert_eq!(encoded["graderKind"], "deterministic");
    assert_eq!(encoded["submittedAnswer"], "Our Father");
}

#[test]
fn schedule_wire_keeps_ts_fsrs_numeric_state_and_snake_case_fields() {
    let encoded = serde_json::to_value(ScheduleState {
        last_review: Some(NOW - 86_400_000),
        ..schedule(ScheduleStatus::Review, 3, 2, NOW)
    })
    .expect("schedule json");

    assert_eq!(encoded["state"], 2);
    assert_eq!(encoded["last_review"], NOW - 86_400_000);
    assert!(encoded.get("lastReview").is_none());
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
