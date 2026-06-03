use std::{convert::Infallible, error::Error, fmt};

use crate::{
    default_rating_policy, GradeContext, GradeResult, Grader, GraderKind, Prompt, Rating,
    RatingPolicy, RubricAssessment, RubricCriterionResult, RubricCriterionVerdict, RubricPrompt,
    Verdict,
};

pub const DEFAULT_RUBRIC_CONFIDENCE_FLOOR: f64 = 0.85;

#[derive(Clone, Copy, Debug)]
pub enum GradeablePrompt<'a> {
    Deterministic(&'a Prompt),
    Rubric(&'a RubricPrompt),
}

pub trait RubricGraderAdapter {
    type Error;

    /// Grade a rubric prompt through a caller-owned adapter.
    ///
    /// # Errors
    ///
    /// Returns the adapter's error when the caller-owned grading boundary
    /// cannot produce a rubric assessment.
    fn grade(&self, prompt: &RubricPrompt, answer: &str) -> Result<RubricAssessment, Self::Error>;
}

#[derive(Clone, Debug)]
pub struct StaticRubricGrader {
    assessment: RubricAssessment,
}

impl StaticRubricGrader {
    #[must_use]
    pub fn new(assessment: RubricAssessment) -> Self {
        Self { assessment }
    }
}

impl RubricGraderAdapter for StaticRubricGrader {
    type Error = Infallible;

    fn grade(
        &self,
        _prompt: &RubricPrompt,
        _answer: &str,
    ) -> Result<RubricAssessment, Self::Error> {
        Ok(self.assessment.clone())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoRubricGrader;

#[derive(Clone, Debug)]
pub struct AsyncGrader<TAdapter = NoRubricGrader> {
    deterministic_grader: Grader,
    rating_policy: RatingPolicy,
    rubric_grader: Option<TAdapter>,
    rubric_confidence_floor: f64,
}

impl Default for AsyncGrader<NoRubricGrader> {
    fn default() -> Self {
        Self {
            deterministic_grader: Grader::new(),
            rating_policy: default_rating_policy,
            rubric_grader: None,
            rubric_confidence_floor: DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
        }
    }
}

impl AsyncGrader<NoRubricGrader> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_rating_policy(rating_policy: RatingPolicy) -> Self {
        Self {
            deterministic_grader: Grader::with_rating_policy(rating_policy),
            rating_policy,
            rubric_grader: None,
            rubric_confidence_floor: DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
        }
    }
}

impl<TAdapter> AsyncGrader<TAdapter>
where
    TAdapter: RubricGraderAdapter,
{
    #[must_use]
    pub fn with_rubric_grader(rubric_grader: TAdapter) -> Self {
        Self::with_rubric_options(
            rubric_grader,
            default_rating_policy,
            DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
        )
    }

    #[must_use]
    pub fn with_rubric_options(
        rubric_grader: TAdapter,
        rating_policy: RatingPolicy,
        rubric_confidence_floor: f64,
    ) -> Self {
        Self {
            deterministic_grader: Grader::with_rating_policy(rating_policy),
            rating_policy,
            rubric_grader: Some(rubric_grader),
            rubric_confidence_floor,
        }
    }

    #[must_use]
    pub fn grade(&self, prompt: &Prompt, submitted: &str, context: GradeContext) -> GradeResult {
        self.deterministic_grader.grade(prompt, submitted, context)
    }

    /// Grade either a deterministic prompt or a rubric prompt.
    ///
    /// # Errors
    ///
    /// Returns [`RubricGradeError::RubricUnavailable`] when a non-blank rubric
    /// answer is submitted without an adapter, or [`RubricGradeError::Adapter`]
    /// when the injected adapter fails.
    pub fn grade_prompt(
        &self,
        prompt: GradeablePrompt<'_>,
        submitted: &str,
        context: GradeContext,
    ) -> Result<GradeResult, RubricGradeError<TAdapter::Error>> {
        match prompt {
            GradeablePrompt::Deterministic(prompt) => Ok(self.grade(prompt, submitted, context)),
            GradeablePrompt::Rubric(prompt) => self.grade_rubric(prompt, submitted, context),
        }
    }

    /// Grade a rubric prompt through the injected adapter.
    ///
    /// # Errors
    ///
    /// Returns [`RubricGradeError::RubricUnavailable`] when no adapter is
    /// configured for a non-blank answer, or [`RubricGradeError::Adapter`] when
    /// the adapter fails.
    pub fn grade_rubric(
        &self,
        prompt: &RubricPrompt,
        submitted: &str,
        context: GradeContext,
    ) -> Result<GradeResult, RubricGradeError<TAdapter::Error>> {
        let submitted_answer = submitted.trim();

        if submitted_answer.is_empty() {
            return Ok(blank_rubric_grade(prompt));
        }

        let rubric_grader = self
            .rubric_grader
            .as_ref()
            .ok_or(RubricGradeError::RubricUnavailable)?;
        let assessment = rubric_grader
            .grade(prompt, submitted_answer)
            .map_err(RubricGradeError::Adapter)?;

        Ok(resolve_rubric_grade(
            prompt,
            submitted_answer,
            &assessment,
            self.rubric_confidence_floor,
            self.rating_policy,
            context,
        ))
    }
}

impl AsyncGrader<NoRubricGrader> {
    #[must_use]
    pub fn grade(&self, prompt: &Prompt, submitted: &str, context: GradeContext) -> GradeResult {
        self.deterministic_grader.grade(prompt, submitted, context)
    }

    /// Grade either a deterministic prompt or a rubric prompt without an adapter.
    ///
    /// # Errors
    ///
    /// Returns [`RubricGradeError::RubricUnavailable`] when a non-blank rubric
    /// answer is submitted. Blank rubric answers are graded locally.
    pub fn grade_prompt(
        &self,
        prompt: GradeablePrompt<'_>,
        submitted: &str,
        context: GradeContext,
    ) -> Result<GradeResult, RubricGradeError<Infallible>> {
        match prompt {
            GradeablePrompt::Deterministic(prompt) => Ok(self.grade(prompt, submitted, context)),
            GradeablePrompt::Rubric(prompt) => self.grade_rubric(prompt, submitted, context),
        }
    }

    /// Grade a rubric prompt without an adapter.
    ///
    /// # Errors
    ///
    /// Returns [`RubricGradeError::RubricUnavailable`] when a non-blank rubric
    /// answer is submitted. Blank answers are graded locally.
    pub fn grade_rubric(
        &self,
        prompt: &RubricPrompt,
        submitted: &str,
        _context: GradeContext,
    ) -> Result<GradeResult, RubricGradeError<Infallible>> {
        let submitted_answer = submitted.trim();

        if submitted_answer.is_empty() {
            Ok(blank_rubric_grade(prompt))
        } else {
            Err(RubricGradeError::RubricUnavailable)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RubricGradeError<TError> {
    RubricUnavailable,
    Adapter(TError),
}

impl<TError> fmt::Display for RubricGradeError<TError>
where
    TError: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RubricUnavailable => formatter.write_str("Rubric grading is unavailable"),
            Self::Adapter(error) => write!(formatter, "{error}"),
        }
    }
}

impl<TError> Error for RubricGradeError<TError> where TError: Error + 'static {}

#[must_use]
pub fn resolve_rubric_grade(
    prompt: &RubricPrompt,
    submitted_answer: &str,
    assessment: &RubricAssessment,
    confidence_floor: f64,
    rating_policy: RatingPolicy,
    context: GradeContext,
) -> GradeResult {
    let criterion_results = normalize_criterion_results(prompt, assessment);
    let passed_count = criterion_results
        .iter()
        .filter(|criterion| criterion.verdict == RubricCriterionVerdict::Pass)
        .count();
    let required_satisfied = prompt.rubric.criteria.iter().all(|criterion| {
        !criterion.required
            || criterion_results.iter().any(|result| {
                result.name == criterion.name && result.verdict == RubricCriterionVerdict::Pass
            })
    });
    let has_passing_score = passed_count >= prompt.rubric.passing_score;
    let confident_enough = assessment.confidence >= confidence_floor;
    let verdict = if required_satisfied && has_passing_score && confident_enough {
        Verdict::Correct
    } else if passed_count > 0 {
        Verdict::Close
    } else {
        Verdict::Wrong
    };

    rubric_grade(RubricGradeParts {
        verdict,
        rating: rating_policy(verdict, context),
        submitted_answer,
        expected_answer: &rubric_expected_answer(prompt),
        is_correct: verdict == Verdict::Correct,
        grader_model: assessment.model.clone(),
        grader_confidence: Some(clamp_confidence(assessment.confidence)),
        feedback: normalized_feedback(&assessment.feedback),
        criterion_results,
    })
}

fn blank_rubric_grade(prompt: &RubricPrompt) -> GradeResult {
    rubric_grade(RubricGradeParts {
        verdict: Verdict::Wrong,
        rating: Rating::Again,
        submitted_answer: "",
        expected_answer: &rubric_expected_answer(prompt),
        is_correct: false,
        grader_model: None,
        grader_confidence: Some(1.0),
        feedback: "No answer submitted.".to_owned(),
        criterion_results: prompt
            .rubric
            .criteria
            .iter()
            .map(|criterion| RubricCriterionResult {
                name: criterion.name.clone(),
                verdict: RubricCriterionVerdict::Fail,
                evidence: "No answer submitted.".to_owned(),
            })
            .collect(),
    })
}

fn normalize_criterion_results(
    prompt: &RubricPrompt,
    assessment: &RubricAssessment,
) -> Vec<RubricCriterionResult> {
    prompt
        .rubric
        .criteria
        .iter()
        .map(|criterion| {
            let matching_result = assessment
                .criterion_results
                .iter()
                .find(|result| normalize_key(&result.name) == normalize_key(&criterion.name));

            RubricCriterionResult {
                name: criterion.name.clone(),
                verdict: matching_result
                    .filter(|result| result.verdict == RubricCriterionVerdict::Pass)
                    .map_or(RubricCriterionVerdict::Fail, |_| {
                        RubricCriterionVerdict::Pass
                    }),
                evidence: matching_result
                    .map(|result| result.evidence.trim())
                    .filter(|evidence| !evidence.is_empty())
                    .unwrap_or("No clear evidence supplied.")
                    .to_owned(),
            }
        })
        .collect()
}

fn normalize_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn rubric_expected_answer(prompt: &RubricPrompt) -> String {
    let answer_guide = prompt.rubric.answer_guide.join(" / ");
    if answer_guide.is_empty() {
        prompt
            .rubric
            .criteria
            .iter()
            .map(|criterion| criterion.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        answer_guide
    }
}

fn clamp_confidence(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn normalized_feedback(feedback: &str) -> String {
    let trimmed = feedback.trim();
    if trimmed.is_empty() {
        "Answer did not clearly satisfy the rubric.".to_owned()
    } else {
        trimmed.to_owned()
    }
}

struct RubricGradeParts<'a> {
    verdict: Verdict,
    rating: Rating,
    submitted_answer: &'a str,
    expected_answer: &'a str,
    is_correct: bool,
    grader_model: Option<String>,
    grader_confidence: Option<f64>,
    feedback: String,
    criterion_results: Vec<RubricCriterionResult>,
}

fn rubric_grade(parts: RubricGradeParts<'_>) -> GradeResult {
    GradeResult {
        verdict: parts.verdict,
        rating: parts.rating,
        is_correct: parts.is_correct,
        submitted_answer: parts.submitted_answer.to_owned(),
        expected_answer: parts.expected_answer.to_owned(),
        grader_kind: GraderKind::RubricLlm,
        grader_model: parts.grader_model,
        grader_confidence: parts.grader_confidence,
        feedback: parts.feedback,
        criterion_results: parts.criterion_results,
    }
}
