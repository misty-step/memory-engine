use crate::{
    ExactPrompt, ExactPromptKind, GradeContext, GradeResult, GraderKind, Prompt, Rating, Verdict,
};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub type RatingPolicy = fn(Verdict, GradeContext) -> Rating;

#[derive(Clone, Debug)]
pub struct Grader {
    rating_policy: RatingPolicy,
}

impl Default for Grader {
    fn default() -> Self {
        Self {
            rating_policy: default_rating_policy,
        }
    }
}

impl Grader {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_rating_policy(rating_policy: RatingPolicy) -> Self {
        Self { rating_policy }
    }

    #[must_use]
    pub fn grade(&self, prompt: &Prompt, submitted: &str, context: GradeContext) -> GradeResult {
        let submitted_answer = submitted.trim();

        match prompt {
            Prompt::Mcq { correct_choice, .. } => {
                self.grade_mcq(submitted_answer, correct_choice, context)
            }
            Prompt::Boolean { correct_answer, .. } => {
                self.grade_boolean(submitted_answer, *correct_answer, context)
            }
            Prompt::Exact(prompt) => self.grade_exact(prompt, submitted_answer, context),
        }
    }

    fn grade_mcq(
        &self,
        submitted_answer: &str,
        correct_choice: &str,
        context: GradeContext,
    ) -> GradeResult {
        // Normalize both sides like exact grading (case-fold, strip diacritics
        // and punctuation, collapse whitespace). A clicked choice submits the
        // verbatim option and matches trivially; this is what makes a *typed*
        // answer ("charlie" vs the stored "Charlie") grade correctly instead of
        // failing on case alone — the bug a learner hits when they type the right
        // word and are told they're wrong.
        let is_correct = normalize_exact(submitted_answer) == normalize_exact(correct_choice);
        let verdict = if is_correct {
            Verdict::Correct
        } else {
            Verdict::Wrong
        };

        deterministic_grade(
            verdict,
            (self.rating_policy)(verdict, context),
            submitted_answer,
            correct_choice,
            is_correct,
        )
    }

    fn grade_boolean(
        &self,
        submitted_answer: &str,
        correct_answer: bool,
        context: GradeContext,
    ) -> GradeResult {
        let expected_answer = if correct_answer { "True" } else { "False" };
        let is_correct = normalize_exact(submitted_answer) == normalize_exact(expected_answer);
        let verdict = if is_correct {
            Verdict::Correct
        } else {
            Verdict::Wrong
        };

        deterministic_grade(
            verdict,
            (self.rating_policy)(verdict, context),
            submitted_answer,
            expected_answer,
            is_correct,
        )
    }

    fn grade_exact(
        &self,
        prompt: &ExactPrompt,
        submitted_answer: &str,
        context: GradeContext,
    ) -> GradeResult {
        match prompt.kind {
            ExactPromptKind::Recitation => self.grade_recitation(prompt, submitted_answer, context),
            ExactPromptKind::Cloze | ExactPromptKind::ShortAnswer => {
                self.grade_normalized_exact(prompt, submitted_answer, context)
            }
        }
    }

    fn grade_recitation(
        &self,
        prompt: &ExactPrompt,
        submitted_answer: &str,
        context: GradeContext,
    ) -> GradeResult {
        let submitted = submitted_answer.trim();
        let accepted = prompt
            .accepted_answers
            .iter()
            .map(|answer| answer.trim())
            .collect::<Vec<_>>();

        if accepted.contains(&submitted) {
            return deterministic_grade(
                Verdict::Correct,
                (self.rating_policy)(Verdict::Correct, context),
                submitted_answer,
                &expected_answer(prompt),
                true,
            );
        }

        let is_close = accepted.iter().any(|candidate| {
            !candidate.is_empty()
                && !submitted.eq_ignore_ascii_case(candidate)
                && (strip_punctuation_preserving_case(submitted)
                    == strip_punctuation_preserving_case(candidate)
                    || levenshtein(submitted, candidate)
                        <= near_miss_threshold(candidate.chars().count()))
        });
        let verdict = if is_close {
            Verdict::Close
        } else {
            Verdict::Wrong
        };

        deterministic_grade(
            verdict,
            (self.rating_policy)(verdict, context),
            submitted_answer,
            &expected_answer(prompt),
            false,
        )
    }

    fn grade_normalized_exact(
        &self,
        prompt: &ExactPrompt,
        submitted_answer: &str,
        context: GradeContext,
    ) -> GradeResult {
        let normalized_submitted = normalize_for_comparison(
            submitted_answer,
            &prompt.equivalence_groups,
            &prompt.ignored_tokens,
        );
        let normalized_accepted = prompt
            .accepted_answers
            .iter()
            .map(|answer| {
                normalize_for_comparison(answer, &prompt.equivalence_groups, &prompt.ignored_tokens)
            })
            .collect::<Vec<_>>();

        if normalized_accepted.contains(&normalized_submitted) {
            return deterministic_grade(
                Verdict::Correct,
                (self.rating_policy)(Verdict::Correct, context),
                submitted_answer,
                &expected_answer(prompt),
                true,
            );
        }

        let is_close = !normalized_submitted.is_empty()
            && normalized_accepted.iter().any(|candidate| {
                !candidate.is_empty()
                    && levenshtein(&normalized_submitted, candidate)
                        <= near_miss_threshold(candidate.chars().count())
            });
        let verdict = if is_close {
            Verdict::Close
        } else {
            Verdict::Wrong
        };

        deterministic_grade(
            verdict,
            (self.rating_policy)(verdict, context),
            submitted_answer,
            &expected_answer(prompt),
            false,
        )
    }
}

#[must_use]
pub fn default_rating_policy(verdict: Verdict, context: GradeContext) -> Rating {
    match verdict {
        Verdict::Correct if context.prior_reps >= 3 && context.response_time_ms <= 4_000 => {
            Rating::Easy
        }
        Verdict::Correct => Rating::Good,
        Verdict::Close => Rating::Hard,
        Verdict::Wrong | Verdict::Revealed => Rating::Again,
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

fn expected_answer(prompt: &ExactPrompt) -> String {
    prompt.accepted_answers.join(" / ")
}

fn normalize_exact(value: &str) -> String {
    normalize_for_comparison(value, &[], &[])
}

fn normalize_for_comparison(
    value: &str,
    equivalence_groups: &[Vec<String>],
    ignored_tokens: &[String],
) -> String {
    let mut normalized = base_normalize(value);
    if normalized.is_empty() {
        return normalized;
    }

    normalized = apply_equivalence_groups(&normalized, equivalence_groups);
    normalized = strip_ignored_tokens(&normalized, ignored_tokens);

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn base_normalize(value: &str) -> String {
    let words = value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();

    words.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_punctuation_preserving_case(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn apply_equivalence_groups(value: &str, groups: &[Vec<String>]) -> String {
    let mut replacements = groups
        .iter()
        .map(|group| {
            group
                .iter()
                .map(|entry| base_normalize(entry))
                .collect::<Vec<_>>()
        })
        .filter(|group| group.len() >= 2)
        .flat_map(|group| {
            let canonical = group[0].clone();
            group
                .into_iter()
                .skip(1)
                .map(move |alias| (alias, canonical.clone()))
        })
        .filter(|(alias, canonical)| !alias.is_empty() && alias != canonical)
        .collect::<Vec<_>>();

    replacements.sort_by_key(|(alias, _)| std::cmp::Reverse(alias.len()));

    replace_whole_tokens(value, &replacements)
}

fn strip_ignored_tokens(value: &str, ignored_tokens: &[String]) -> String {
    let replacements = ignored_tokens
        .iter()
        .map(|token| base_normalize(token))
        .filter(|token| !token.is_empty())
        .map(|token| (token, String::new()))
        .collect::<Vec<_>>();

    replace_whole_tokens(value, &replacements)
}

fn replace_whole_tokens(value: &str, replacements: &[(String, String)]) -> String {
    value
        .split_whitespace()
        .map(|token| {
            replacements
                .iter()
                .find_map(|(from, to)| (token == from).then_some(to.as_str()))
                .unwrap_or(token)
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn near_miss_threshold(length: usize) -> usize {
    match length {
        0..=4 => 0,
        5..=8 => 1,
        _ => 2,
    }
}

fn levenshtein(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();

    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];

        for (right_index, right_char) in right_chars.iter().enumerate() {
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            current.push(insertion.min(deletion).min(substitution));
        }

        previous = current;
    }

    previous[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use crate::{ExactPromptKind, ReviewUnitId};

    use super::*;

    fn context(response_time_ms: u32, prior_reps: u32) -> GradeContext {
        GradeContext {
            response_time_ms,
            prior_reps,
        }
    }

    fn short_answer(accepted_answers: Vec<&str>) -> Prompt {
        Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::ShortAnswer,
            review_unit_id: ReviewUnitId::new("short-answer"),
            prompt: "Answer?".to_owned(),
            accepted_answers: accepted_answers.into_iter().map(str::to_owned).collect(),
            equivalence_groups: Vec::new(),
            ignored_tokens: Vec::new(),
        })
    }

    fn recitation(accepted_answer: &str) -> Prompt {
        Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::Recitation,
            review_unit_id: ReviewUnitId::new("recitation"),
            prompt: "Recite the accepted unit exactly.".to_owned(),
            accepted_answers: vec![accepted_answer.to_owned()],
            equivalence_groups: vec![vec!["colour".to_owned(), "color".to_owned()]],
            ignored_tokens: vec!["please".to_owned(), ".".to_owned()],
        })
    }

    #[test]
    fn default_rating_policy_matches_current_typescript_contract() {
        assert_eq!(
            default_rating_policy(Verdict::Correct, context(3_000, 3)),
            Rating::Easy
        );
        assert_eq!(
            default_rating_policy(Verdict::Correct, context(5_100, 3)),
            Rating::Good
        );
        assert_eq!(
            default_rating_policy(Verdict::Close, context(5_100, 0)),
            Rating::Hard
        );
        assert_eq!(
            default_rating_policy(Verdict::Wrong, context(5_100, 0)),
            Rating::Again
        );
        assert_eq!(
            default_rating_policy(Verdict::Revealed, context(5_100, 0)),
            Rating::Again
        );
    }

    #[test]
    fn trims_and_grades_mcq_choices() {
        let prompt = Prompt::Mcq {
            review_unit_id: ReviewUnitId::new("mcq"),
            prompt: "Pick one".to_owned(),
            choices: vec!["Alpha".to_owned(), "Beta".to_owned()],
            correct_choice: "Alpha".to_owned(),
        };

        let grade = Grader::new().grade(&prompt, " Alpha ", context(5_100, 0));

        assert_eq!(grade.verdict, Verdict::Correct);
        assert_eq!(grade.rating, Rating::Good);
        assert_eq!(grade.submitted_answer, "Alpha");
    }

    #[test]
    fn mcq_grading_is_case_and_punctuation_insensitive() {
        // Regression: a learner typed the right word and was marked wrong because
        // MCQ grading compared raw strings. The stored choice is "Charlie"; the
        // learner's "charlie" (and "charlie!") must grade correct.
        let prompt = Prompt::Mcq {
            review_unit_id: ReviewUnitId::new("mcq"),
            prompt: "Which code word represents the letter C?".to_owned(),
            choices: vec![
                "Charlie".to_owned(),
                "Delta".to_owned(),
                "Bravo".to_owned(),
                "Echo".to_owned(),
            ],
            correct_choice: "Charlie".to_owned(),
        };

        for submission in ["charlie", "Charlie", "  CHARLIE ", "charlie!"] {
            let grade = Grader::new().grade(&prompt, submission, context(5_100, 0));
            assert_eq!(
                grade.verdict,
                Verdict::Correct,
                "submission {submission:?} should grade correct"
            );
        }

        let wrong = Grader::new().grade(&prompt, "Delta", context(5_100, 0));
        assert_eq!(wrong.verdict, Verdict::Wrong);
    }

    #[test]
    fn exact_grading_handles_equivalence_ignored_tokens_and_near_misses() {
        let prompt = Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::Cloze,
            review_unit_id: ReviewUnitId::new("cloze"),
            prompt: "Respond".to_owned(),
            accepted_answers: vec!["Glory to you, O Lord".to_owned()],
            equivalence_groups: vec![vec!["o".to_owned(), "oh".to_owned()]],
            ignored_tokens: vec!["please".to_owned()],
        });

        let correct =
            Grader::new().grade(&prompt, "Please glory to you oh lord", context(5_100, 0));
        assert_eq!(correct.verdict, Verdict::Correct);

        let close = Grader::new().grade(
            &short_answer(vec!["punishment"]),
            "punishmant",
            context(5_100, 0),
        );
        assert_eq!(close.verdict, Verdict::Close);
        assert_eq!(close.rating, Rating::Hard);
    }

    #[test]
    fn exact_grading_strips_diacritics_like_the_typescript_oracle() {
        let prompt = short_answer(vec!["credo"]);

        let grade = Grader::new().grade(&prompt, "crédō", context(5_100, 0));

        assert_eq!(grade.verdict, Verdict::Correct);
    }

    #[test]
    fn recitation_requires_the_exact_accepted_unit_and_has_explicit_near_misses() {
        let prompt = recitation("The colour of sky.");

        let exact = Grader::new().grade(&prompt, "The colour of sky.", context(5_100, 0));
        assert_eq!(exact.verdict, Verdict::Correct);

        let outer_whitespace =
            Grader::new().grade(&prompt, "  The colour of sky.  ", context(5_100, 0));
        assert_eq!(outer_whitespace.verdict, Verdict::Correct);

        let case_variant = Grader::new().grade(&prompt, "the colour of sky.", context(5_100, 0));
        assert_eq!(case_variant.verdict, Verdict::Wrong);

        let punctuation_variant =
            Grader::new().grade(&prompt, "The colour of sky", context(5_100, 0));
        assert_eq!(punctuation_variant.verdict, Verdict::Close);

        let word_near_miss = Grader::new().grade(&prompt, "The colour of say.", context(5_100, 0));
        assert_eq!(word_near_miss.verdict, Verdict::Close);

        let normalized_variant =
            Grader::new().grade(&prompt, "Please the color of sky.", context(5_100, 0));
        assert_eq!(normalized_variant.verdict, Verdict::Wrong);
    }
}
