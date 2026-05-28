use memory_engine_core::{
    AsyncGrader, ExactPrompt, ExactPromptKind, GradeContext, GradeablePrompt, Grader, GraderKind,
    Prompt, Rating, ReviewUnitId, RubricAssessment, RubricCriterion, RubricCriterionResult,
    RubricCriterionVerdict, RubricDefinition, RubricGradeError, RubricPrompt, StaticRubricGrader,
    Verdict,
};

#[test]
fn async_grader_preserves_deterministic_surface() {
    let prompt = deterministic_prompt();
    let context = GradeContext {
        response_time_ms: 5_100,
        prior_reps: 0,
    };

    let sync_result = Grader::new().grade(&prompt, "Punishment", context);
    let async_result = AsyncGrader::new()
        .grade_prompt(
            GradeablePrompt::Deterministic(&prompt),
            "Punishment",
            context,
        )
        .expect("deterministic grade");

    assert_eq!(async_result, sync_result);
    assert_eq!(async_result.criterion_results, []);
}

#[test]
fn rubric_prompt_requires_an_adapter_for_non_blank_answers() {
    let error = AsyncGrader::new()
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Some answer",
            default_context(),
        )
        .expect_err("rubric grader should be absent");

    assert!(matches!(error, RubricGradeError::RubricUnavailable));
    assert_eq!(error.to_string(), "Rubric grading is unavailable");
}

#[test]
fn blank_rubric_answer_short_circuits_without_adapter() {
    let result = AsyncGrader::new()
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "   ",
            default_context(),
        )
        .expect("blank answer is locally gradeable");

    assert_eq!(result.verdict, Verdict::Wrong);
    assert_eq!(result.rating, Rating::Again);
    assert!(!result.is_correct);
    assert_eq!(result.grader_kind, GraderKind::RubricLlm);
    assert_eq!(result.grader_model, None);
    assert_eq!(result.grader_confidence, Some(1.0));
    assert_eq!(result.feedback, "No answer submitted.");
    assert_eq!(
        result.criterion_results,
        vec![
            RubricCriterionResult {
                name: "speaker".to_owned(),
                verdict: RubricCriterionVerdict::Fail,
                evidence: "No answer submitted.".to_owned(),
            },
            RubricCriterionResult {
                name: "response".to_owned(),
                verdict: RubricCriterionVerdict::Fail,
                evidence: "No answer submitted.".to_owned(),
            },
        ]
    );
}

#[test]
fn static_adapter_resolves_correct_rubric_answers() {
    let grader = AsyncGrader::with_rubric_grader(StaticRubricGrader::new(RubricAssessment {
        model: Some("gpt-5.4-mini".to_owned()),
        confidence: 0.92,
        feedback: " Clear and complete. ".to_owned(),
        criterion_results: criterion_results([
            (
                " speaker ",
                RubricCriterionVerdict::Pass,
                "speaker evidence",
            ),
            (
                "RESPONSE",
                RubricCriterionVerdict::Pass,
                "response evidence",
            ),
        ]),
    }));

    let result = grader
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Glory to you, O Lord.",
            default_context(),
        )
        .expect("rubric grade");

    assert_eq!(result.verdict, Verdict::Correct);
    assert_eq!(result.rating, Rating::Good);
    assert!(result.is_correct);
    assert_eq!(result.grader_kind, GraderKind::RubricLlm);
    assert_eq!(result.grader_model, Some("gpt-5.4-mini".to_owned()));
    assert_eq!(result.grader_confidence, Some(0.92));
    assert_eq!(result.feedback, "Clear and complete.");
    assert_eq!(result.expected_answer, "Say \"Glory to you, O Lord.\"");
    assert_eq!(
        result.criterion_results,
        criterion_results([
            ("speaker", RubricCriterionVerdict::Pass, "speaker evidence"),
            (
                "response",
                RubricCriterionVerdict::Pass,
                "response evidence"
            ),
        ])
    );
}

#[test]
fn rubric_grade_downgrades_low_confidence_or_missing_required_criteria() {
    let low_confidence =
        AsyncGrader::with_rubric_grader(StaticRubricGrader::new(RubricAssessment {
            model: Some("gpt-5.4-mini".to_owned()),
            confidence: 0.62,
            feedback: "Mostly right.".to_owned(),
            criterion_results: criterion_results([
                ("speaker", RubricCriterionVerdict::Pass, "speaker evidence"),
                (
                    "response",
                    RubricCriterionVerdict::Pass,
                    "response evidence",
                ),
            ]),
        }))
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Something close.",
            default_context(),
        )
        .expect("rubric grade");

    assert_eq!(low_confidence.verdict, Verdict::Close);
    assert_eq!(low_confidence.rating, Rating::Hard);
    assert!(!low_confidence.is_correct);

    let missing_required =
        AsyncGrader::with_rubric_grader(StaticRubricGrader::new(RubricAssessment {
            model: Some("gpt-5.4-mini".to_owned()),
            confidence: 0.98,
            feedback: "Missed context.".to_owned(),
            criterion_results: criterion_results([
                ("speaker", RubricCriterionVerdict::Fail, "missing context"),
                ("response", RubricCriterionVerdict::Pass, "line supplied"),
            ]),
        }))
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Glory to you, O Lord.",
            default_context(),
        )
        .expect("rubric grade");

    assert_eq!(missing_required.verdict, Verdict::Close);
    assert_eq!(missing_required.rating, Rating::Hard);
    assert!(!missing_required.is_correct);
}

#[test]
fn rubric_grade_normalizes_missing_evidence_feedback_and_confidence() {
    let grader = AsyncGrader::with_rubric_grader(StaticRubricGrader::new(RubricAssessment {
        model: None,
        confidence: f64::INFINITY,
        feedback: "   ".to_owned(),
        criterion_results: vec![RubricCriterionResult {
            name: "speaker".to_owned(),
            verdict: RubricCriterionVerdict::Pass,
            evidence: "   ".to_owned(),
        }],
    }));

    let result = grader
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Partial answer.",
            default_context(),
        )
        .expect("rubric grade");

    assert_eq!(result.verdict, Verdict::Close);
    assert_eq!(result.grader_confidence, Some(0.0));
    assert_eq!(
        result.feedback,
        "Answer did not clearly satisfy the rubric."
    );
    assert_eq!(
        result.criterion_results,
        vec![
            RubricCriterionResult {
                name: "speaker".to_owned(),
                verdict: RubricCriterionVerdict::Pass,
                evidence: "No clear evidence supplied.".to_owned(),
            },
            RubricCriterionResult {
                name: "response".to_owned(),
                verdict: RubricCriterionVerdict::Fail,
                evidence: "No clear evidence supplied.".to_owned(),
            },
        ]
    );
}

#[test]
fn rubric_adapter_uses_injected_rating_policy() {
    let grader = AsyncGrader::with_rubric_options(
        StaticRubricGrader::new(RubricAssessment {
            model: Some("gpt-5.4-mini".to_owned()),
            confidence: 0.91,
            feedback: "Strong answer.".to_owned(),
            criterion_results: criterion_results([
                ("speaker", RubricCriterionVerdict::Pass, "speaker evidence"),
                (
                    "response",
                    RubricCriterionVerdict::Pass,
                    "response evidence",
                ),
            ]),
        }),
        |verdict, _context| {
            if verdict == Verdict::Correct {
                Rating::Easy
            } else {
                Rating::Again
            }
        },
        0.85,
    );

    let result = grader
        .grade_prompt(
            GradeablePrompt::Rubric(&rubric_prompt()),
            "Glory to you, O Lord.",
            GradeContext {
                response_time_ms: 6_000,
                prior_reps: 4,
            },
        )
        .expect("rubric grade");

    assert_eq!(result.verdict, Verdict::Correct);
    assert_eq!(result.rating, Rating::Easy);
}

#[test]
fn rubric_wire_shape_matches_typescript_contract() {
    let encoded = serde_json::to_value(rubric_prompt()).expect("prompt json");

    assert_eq!(
        encoded,
        serde_json::json!({
            "reviewUnitId": "mass-rubric-01",
            "prompt": "At Mass, before the Gospel, the deacon announces the reading. Respond.",
            "rubric": {
                "answerGuide": ["Say \"Glory to you, O Lord.\""],
                "passingScore": 2,
                "criteria": [
                    {
                        "name": "speaker",
                        "description": "Understands this is the Gospel response.",
                        "required": true
                    },
                    {
                        "name": "response",
                        "description": "Supplies the correct response.",
                        "required": true
                    }
                ]
            }
        })
    );
}

fn deterministic_prompt() -> Prompt {
    Prompt::Exact(ExactPrompt {
        kind: ExactPromptKind::ShortAnswer,
        review_unit_id: ReviewUnitId::new("short-answer-01"),
        prompt: "Answer?".to_owned(),
        accepted_answers: vec!["punishment".to_owned()],
        equivalence_groups: Vec::new(),
        ignored_tokens: Vec::new(),
    })
}

fn rubric_prompt() -> RubricPrompt {
    RubricPrompt {
        review_unit_id: ReviewUnitId::new("mass-rubric-01"),
        prompt: "At Mass, before the Gospel, the deacon announces the reading. Respond.".to_owned(),
        rubric: RubricDefinition {
            answer_guide: vec!["Say \"Glory to you, O Lord.\"".to_owned()],
            passing_score: 2,
            criteria: vec![
                RubricCriterion {
                    name: "speaker".to_owned(),
                    description: "Understands this is the Gospel response.".to_owned(),
                    required: true,
                },
                RubricCriterion {
                    name: "response".to_owned(),
                    description: "Supplies the correct response.".to_owned(),
                    required: true,
                },
            ],
        },
    }
}

fn criterion_results<const COUNT: usize>(
    results: [(&str, RubricCriterionVerdict, &str); COUNT],
) -> Vec<RubricCriterionResult> {
    results
        .into_iter()
        .map(|(name, verdict, evidence)| RubricCriterionResult {
            name: name.to_owned(),
            verdict,
            evidence: evidence.to_owned(),
        })
        .collect()
}

fn default_context() -> GradeContext {
    GradeContext {
        response_time_ms: 6_000,
        prior_reps: 0,
    }
}
