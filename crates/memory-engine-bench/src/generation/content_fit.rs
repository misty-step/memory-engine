use memory_engine_generation::{DraftCandidate, LearningIntent};
use memory_engine_persistence::GeneratedLearningActivityKind;
use serde::Deserialize;

use super::{fraction, normalize};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ContentFitExpectation {
    #[serde(default)]
    expected_type: Option<ContentKind>,
    #[serde(default)]
    coverage_units: Vec<ContentCoverageUnit>,
    #[serde(default)]
    required_shape: Option<RequiredShape>,
    #[serde(default)]
    forbidden_directions: Vec<ForbiddenDirection>,
    #[serde(default)]
    required_direction: PairDirection,
    #[serde(default)]
    pairs: Vec<PairedAssociate>,
}

impl ContentFitExpectation {
    fn all_coverage_units(&self) -> Vec<ContentCoverageUnit> {
        let mut units = self.coverage_units.clone();
        units.extend(
            self.pairs
                .iter()
                .map(|pair| self.required_direction.coverage_unit(pair)),
        );
        units
    }

    fn all_forbidden_directions(&self) -> Vec<ForbiddenDirection> {
        let mut directions = self.forbidden_directions.clone();
        directions.extend(
            self.pairs
                .iter()
                .map(|pair| self.required_direction.forbidden_reverse(pair)),
        );
        directions
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ContentKind {
    VerbatimSequential,
    Conceptual,
    EnumerableSet,
    FactRecall,
    ProcedureProcess,
}

impl ContentKind {
    fn matches_learning_intent(self, learning_intent: Option<LearningIntent>) -> bool {
        matches!(
            (self, learning_intent),
            (
                Self::VerbatimSequential,
                Some(LearningIntent::VerbatimMemorization)
            ) | (Self::Conceptual, Some(LearningIntent::ConceptUnderstanding))
                | (Self::EnumerableSet, Some(LearningIntent::EnumerableSet))
                | (Self::FactRecall, Some(LearningIntent::FactRecall))
                | (
                    Self::ProcedureProcess,
                    Some(LearningIntent::ProcedureProcess)
                )
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum RequiredShape {
    #[serde(rename = "next_line_recall")]
    NextLine,
    #[serde(rename = "production_recall")]
    Production,
    #[serde(rename = "conceptual_free_recall")]
    ConceptualFree,
}

#[derive(Clone, Debug, Deserialize)]
struct ContentCoverageUnit {
    id: String,
    #[serde(default)]
    prompt_terms: Vec<String>,
    answer_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ForbiddenDirection {
    #[serde(default)]
    prompt_terms: Vec<String>,
    answer_terms: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PairDirection {
    #[default]
    CueToAnswer,
    AnswerToCue,
}

impl PairDirection {
    fn coverage_unit(self, pair: &PairedAssociate) -> ContentCoverageUnit {
        let (cue, answer) = self.ordered(pair);
        ContentCoverageUnit {
            id: format!("pair-{}-to-{}", id_fragment(cue), id_fragment(answer)),
            prompt_terms: cue_prompt_terms(cue),
            answer_terms: vec![answer.to_owned()],
        }
    }

    fn forbidden_reverse(self, pair: &PairedAssociate) -> ForbiddenDirection {
        let (cue, answer) = self.ordered(pair);
        ForbiddenDirection {
            prompt_terms: cue_prompt_terms(answer),
            answer_terms: vec![cue.to_owned()],
        }
    }

    fn ordered(self, pair: &PairedAssociate) -> (&str, &str) {
        match self {
            Self::CueToAnswer => (&pair.cue, &pair.answer),
            Self::AnswerToCue => (&pair.answer, &pair.cue),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PairedAssociate {
    cue: String,
    answer: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentFitScore {
    pub classification: ContentFitCheck,
    pub covered_units: usize,
    pub required_units: usize,
    pub coverage: f64,
    pub shape: ContentFitCheck,
    pub directionality: ContentFitCheck,
}

impl ContentFitScore {
    pub fn passes(&self) -> bool {
        self.classification.passes()
            && (self.coverage - 1.0).abs() < f64::EPSILON
            && self.shape.passes()
            && self.directionality.passes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentFitCheck {
    Pass,
    Fail,
}

impl ContentFitCheck {
    fn from_bool(pass: bool) -> Self {
        if pass {
            Self::Pass
        } else {
            Self::Fail
        }
    }

    fn passes(self) -> bool {
        self == Self::Pass
    }
}

pub(super) fn score(
    expectation: Option<&ContentFitExpectation>,
    learning_intent: Option<LearningIntent>,
    candidates: &[DraftCandidate],
) -> Option<ContentFitScore> {
    let content = expectation?;
    let coverage_units = content.all_coverage_units();
    let forbidden_directions = content.all_forbidden_directions();
    let classification = ContentFitCheck::from_bool(
        content
            .expected_type
            .is_none_or(|expected| expected.matches_learning_intent(learning_intent)),
    );
    let covered_units = coverage_units
        .iter()
        .filter(|unit| !unit.id.trim().is_empty() && coverage_unit_covered(unit, candidates))
        .count();
    let required_units = coverage_units.len();
    let coverage = if required_units == 0 {
        1.0
    } else {
        fraction(covered_units, required_units)
    };
    let shape = ContentFitCheck::from_bool(
        content
            .required_shape
            .is_none_or(|shape| shape.matches(candidates, &coverage_units)),
    );
    let directionality = ContentFitCheck::from_bool(forbidden_directions_absent(
        &forbidden_directions,
        candidates,
    ));

    Some(ContentFitScore {
        classification,
        covered_units,
        required_units,
        coverage,
        shape,
        directionality,
    })
}

pub(super) fn cells(score: Option<&ContentFitScore>) -> (String, String, String, String) {
    match score {
        Some(score) => (
            check_cell(score.classification).to_owned(),
            format!(
                "{}/{} ({:.0}%)",
                score.covered_units,
                score.required_units,
                score.coverage * 100.0
            ),
            check_cell(score.shape).to_owned(),
            check_cell(score.directionality).to_owned(),
        ),
        None => (
            "N/A".to_owned(),
            "N/A".to_owned(),
            "N/A".to_owned(),
            "N/A".to_owned(),
        ),
    }
}

impl RequiredShape {
    fn matches(
        self,
        candidates: &[DraftCandidate],
        coverage_units: &[ContentCoverageUnit],
    ) -> bool {
        match self {
            Self::NextLine => {
                let covered = content_coverage_candidates(candidates, coverage_units);
                !covered.is_empty()
                    && candidates.iter().all(|candidate| {
                        candidate.distractors.is_empty()
                            && candidate.activity_kind == GeneratedLearningActivityKind::Exercise
                            && (candidate.activity_stage.to_lowercase().contains("recall")
                                || candidate.activity_stage.to_lowercase().contains("cloze")
                                || contains_normalized_phrase(&candidate.question, "next line")
                                || contains_normalized_phrase(&candidate.question, "recite"))
                    })
            }
            Self::Production => {
                let covered = content_coverage_candidates(candidates, coverage_units);
                !covered.is_empty()
                    && candidates
                        .iter()
                        .all(|candidate| candidate.distractors.is_empty())
            }
            Self::ConceptualFree => {
                !candidates.is_empty()
                    && candidates.iter().all(|candidate| {
                        candidate.activity_stage.to_lowercase().contains("free")
                            && candidate.distractors.is_empty()
                    })
            }
        }
    }
}

fn coverage_unit_covered(unit: &ContentCoverageUnit, candidates: &[DraftCandidate]) -> bool {
    candidates.iter().any(|candidate| {
        let prompt_haystack = format!("{} {}", candidate.concept, candidate.question);
        let prompt_matches = unit.prompt_terms.is_empty()
            || unit
                .prompt_terms
                .iter()
                .any(|term| contains_normalized_phrase(&prompt_haystack, term));
        let answer_matches = unit
            .answer_terms
            .iter()
            .any(|term| normalized_equals(&candidate.answer, term));

        prompt_matches && answer_matches
    })
}

fn content_coverage_candidates<'a>(
    candidates: &'a [DraftCandidate],
    coverage_units: &[ContentCoverageUnit],
) -> Vec<&'a DraftCandidate> {
    if coverage_units.is_empty() {
        return candidates.iter().collect();
    }
    candidates
        .iter()
        .filter(|candidate| {
            coverage_units.iter().any(|unit| {
                !unit.id.trim().is_empty()
                    && coverage_unit_covered(unit, std::slice::from_ref(candidate))
            })
        })
        .collect()
}

fn forbidden_directions_absent(
    directions: &[ForbiddenDirection],
    candidates: &[DraftCandidate],
) -> bool {
    directions.iter().all(|direction| {
        candidates.iter().all(|candidate| {
            let prompt_haystack = format!("{} {}", candidate.concept, candidate.question);
            let prompt_matches = direction
                .prompt_terms
                .iter()
                .any(|term| contains_normalized_phrase(&prompt_haystack, term));
            let answer_matches = direction
                .answer_terms
                .iter()
                .any(|term| forbidden_answer_matches(&candidate.answer, term));

            !(prompt_matches && answer_matches)
        })
    })
}

fn forbidden_answer_matches(answer: &str, term: &str) -> bool {
    let normalized_term = normalize(term);
    if normalized_term.chars().count() == 1 {
        return normalized_equals(answer, &normalized_term)
            || contains_normalized_phrase(answer, &format!("letter {normalized_term}"))
            || contains_normalized_phrase(answer, &format!("the letter {normalized_term}"));
    }

    contains_normalized_phrase(answer, term)
}

fn cue_prompt_terms(cue: &str) -> Vec<String> {
    let normalized = normalize(cue);
    if normalized.chars().count() == 1 {
        vec![
            format!("what is {normalized}"),
            format!("letter {normalized}"),
        ]
    } else {
        vec![cue.to_owned()]
    }
}

fn id_fragment(value: &str) -> String {
    let normalized = normalize(value);
    if normalized.is_empty() {
        "blank".to_owned()
    } else {
        normalized.replace(' ', "-")
    }
}

fn contains_normalized_phrase(haystack: &str, needle: &str) -> bool {
    let haystack = normalize(haystack);
    let needle = normalize(needle);
    if needle.is_empty() {
        return false;
    }
    format!(" {haystack} ").contains(&format!(" {needle} "))
}

fn normalized_equals(left: &str, right: &str) -> bool {
    normalize(left) == normalize(right)
}

fn check_cell(check: ContentFitCheck) -> &'static str {
    match check {
        ContentFitCheck::Pass => "yes",
        ContentFitCheck::Fail => "NO",
    }
}

#[cfg(test)]
mod tests {
    use memory_engine_generation::{DraftCandidate, FakeModelProvider, LearningIntent};
    use memory_engine_persistence::GeneratedLearningActivityKind;

    use super::*;

    fn candidate(question: &str, answer: &str, evidence: &str) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: question.to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: Some(evidence.to_owned()),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        }
    }

    fn mcq(question: &str, answer: &str, distractors: &[&str]) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: question.to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: None,
            distractors: distractors
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        }
    }

    fn expectation() -> ContentFitExpectation {
        ContentFitExpectation {
            expected_type: None,
            coverage_units: Vec::new(),
            required_shape: None,
            forbidden_directions: Vec::new(),
            required_direction: PairDirection::CueToAnswer,
            pairs: Vec::new(),
        }
    }

    fn coverage_unit(
        id: &str,
        prompt_terms: &[&str],
        answer_terms: &[&str],
    ) -> ContentCoverageUnit {
        ContentCoverageUnit {
            id: id.to_owned(),
            prompt_terms: prompt_terms.iter().map(|term| (*term).to_owned()).collect(),
            answer_terms: answer_terms.iter().map(|term| (*term).to_owned()).collect(),
        }
    }

    fn pair(cue: &str, answer: &str) -> PairedAssociate {
        PairedAssociate {
            cue: cue.to_owned(),
            answer: answer.to_owned(),
        }
    }

    #[test]
    fn flags_current_verbatim_and_enumerable_failures() {
        let creed_expect = ContentFitExpectation {
            expected_type: Some(ContentKind::VerbatimSequential),
            coverage_units: vec![
                coverage_unit("creed-1", &[], &["I believe in God the Father almighty"]),
                coverage_unit("creed-2", &[], &["creator of heaven and earth"]),
            ],
            required_shape: Some(RequiredShape::NextLine),
            ..expectation()
        };
        let creed_candidates = vec![mcq(
            "God the Father is creator of which two realms?",
            "heaven and earth",
            &["sea and sky", "time and space"],
        )];

        let creed = score(
            Some(&creed_expect),
            Some(LearningIntent::ConceptUnderstanding),
            &creed_candidates,
        )
        .expect("creed content fit");

        assert_eq!(creed.classification, ContentFitCheck::Fail);
        assert!(creed.coverage < 1.0);
        assert_eq!(creed.shape, ContentFitCheck::Fail);
        assert!(!creed.passes());

        let nato_expect = ContentFitExpectation {
            expected_type: Some(ContentKind::EnumerableSet),
            required_shape: Some(RequiredShape::Production),
            pairs: vec![pair("A", "Alfa"), pair("B", "Bravo"), pair("C", "Charlie")],
            ..expectation()
        };
        let nato_candidates = vec![
            mcq(
                "According to \"NATO alphabet\", what is A?",
                "Alfa",
                &["Bravo", "Charlie"],
            ),
            mcq(
                "According to \"NATO alphabet\", what is B?",
                "Bravo",
                &["Alfa", "Charlie"],
            ),
        ];

        let nato = score(
            Some(&nato_expect),
            Some(LearningIntent::EnumerableSet),
            &nato_candidates,
        )
        .expect("nato content fit");

        assert_eq!(nato.classification, ContentFitCheck::Pass);
        assert_eq!(nato.covered_units, 2);
        assert_eq!(nato.required_units, 3);
        assert!((nato.coverage - (2.0 / 3.0)).abs() < 0.01);
        assert_eq!(nato.shape, ContentFitCheck::Fail);
        assert!(!nato.passes());
    }

    #[test]
    fn paired_associates_derive_forbidden_reverse_direction() {
        let expect = ContentFitExpectation {
            expected_type: Some(ContentKind::EnumerableSet),
            required_shape: Some(RequiredShape::Production),
            pairs: vec![pair("B", "Bravo")],
            ..expectation()
        };
        let reverse_card = vec![DraftCandidate {
            activity_kind: GeneratedLearningActivityKind::Exercise,
            activity_stage: "cued-recall".to_owned(),
            distractors: Vec::new(),
            ..candidate(
                "What letter does Bravo represent?",
                "the letter B",
                "B is Bravo.",
            )
        }];

        let fit = score(
            Some(&expect),
            Some(LearningIntent::FactRecall),
            &reverse_card,
        )
        .expect("content fit");

        assert_eq!(fit.directionality, ContentFitCheck::Fail);
        assert!(!fit.passes());
    }

    #[test]
    fn answer_to_cue_direction_derives_specific_single_letter_reverse_prompts() {
        let expect = ContentFitExpectation {
            expected_type: Some(ContentKind::EnumerableSet),
            required_shape: Some(RequiredShape::Production),
            required_direction: PairDirection::AnswerToCue,
            pairs: vec![pair("A", "Alfa")],
            ..expectation()
        };
        let article_only = vec![DraftCandidate {
            activity_kind: GeneratedLearningActivityKind::Exercise,
            activity_stage: "cued-recall".to_owned(),
            distractors: Vec::new(),
            ..candidate(
                "Explain why Alfa is spelled with a final a.",
                "The spelling aids pronunciation.",
                "Alfa is deliberately spelled that way.",
            )
        }];

        let fit = score(
            Some(&expect),
            Some(LearningIntent::FactRecall),
            &article_only,
        )
        .expect("content fit");

        assert_eq!(fit.directionality, ContentFitCheck::Pass);
    }

    #[test]
    fn direction_ignores_single_letter_articles() {
        let expect = ContentFitExpectation {
            forbidden_directions: vec![ForbiddenDirection {
                prompt_terms: vec!["alfa".to_owned()],
                answer_terms: vec!["A".to_owned()],
            }],
            ..expectation()
        };
        let not_a_letter_answer = vec![DraftCandidate {
            activity_kind: GeneratedLearningActivityKind::Exercise,
            activity_stage: "free-recall".to_owned(),
            distractors: Vec::new(),
            ..candidate(
                "Explain why Alfa is spelled that way.",
                "Alfa is a deliberate spelling for multilingual pronunciation.",
                "The spelling Alfa prevents mispronunciation.",
            )
        }];

        let fit = score(
            Some(&expect),
            Some(LearningIntent::FactRecall),
            &not_a_letter_answer,
        )
        .expect("content fit");

        assert_eq!(fit.directionality, ContentFitCheck::Pass);
        assert!(fit.passes());
    }

    #[test]
    fn production_shape_rejects_mixed_recognition_cards() {
        let expect = ContentFitExpectation {
            expected_type: Some(ContentKind::EnumerableSet),
            coverage_units: vec![coverage_unit("nato-a", &["letter a"], &["Alfa"])],
            required_shape: Some(RequiredShape::Production),
            ..expectation()
        };
        let mixed = vec![
            DraftCandidate {
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "cued-recall".to_owned(),
                distractors: Vec::new(),
                ..candidate("What is the code word for letter A?", "Alfa", "A is Alfa.")
            },
            mcq(
                "In the NATO phonetic alphabet, what code word represents the letter B?",
                "Bravo",
                &["Alfa", "Charlie"],
            ),
        ];

        let fit =
            score(Some(&expect), Some(LearningIntent::FactRecall), &mixed).expect("content fit");

        assert!((fit.coverage - 1.0).abs() < f64::EPSILON);
        assert_eq!(fit.shape, ContentFitCheck::Fail);
        assert!(!fit.passes());
    }

    #[test]
    fn conceptual_free_recall_requires_every_candidate_to_be_free_recall() {
        let expect = ContentFitExpectation {
            expected_type: Some(ContentKind::Conceptual),
            required_shape: Some(RequiredShape::ConceptualFree),
            ..expectation()
        };
        let mixed = vec![
            DraftCandidate {
                activity_stage: "free-recall".to_owned(),
                distractors: Vec::new(),
                ..candidate(
                    "Explain why mitochondria matter.",
                    "Mitochondria generate ATP for cells.",
                    "Mitochondria generate ATP.",
                )
            },
            mcq(
                "What molecule do mitochondria help generate?",
                "ATP",
                &["DNA", "RNA"],
            ),
        ];

        let fit = score(
            Some(&expect),
            Some(LearningIntent::ConceptUnderstanding),
            &mixed,
        )
        .expect("content fit");

        assert_eq!(fit.classification, ContentFitCheck::Pass);
        assert_eq!(fit.shape, ContentFitCheck::Fail);
        assert!(!fit.passes());
    }

    #[test]
    fn allows_conceptual_free_recall_without_coverage_bloat() {
        let expect = ContentFitExpectation {
            expected_type: Some(ContentKind::Conceptual),
            required_shape: Some(RequiredShape::ConceptualFree),
            ..expectation()
        };
        let conceptual = vec![DraftCandidate {
            activity_stage: "free-recall".to_owned(),
            distractors: Vec::new(),
            ..candidate(
                "Explain why mitochondria matter.",
                "Mitochondria generate ATP for cells.",
                "Mitochondria generate ATP.",
            )
        }];

        let fit = score(
            Some(&expect),
            Some(LearningIntent::ConceptUnderstanding),
            &conceptual,
        )
        .expect("content fit");

        assert_eq!(fit.classification, ContentFitCheck::Pass);
        assert!((fit.coverage - 1.0).abs() < f64::EPSILON);
        assert_eq!(fit.shape, ContentFitCheck::Pass);
        assert_eq!(fit.directionality, ContentFitCheck::Pass);
        assert!(fit.passes());
    }

    #[test]
    fn corpus_records_current_red_cases() {
        let corpus = super::super::load_corpus().expect("corpus");
        let scores = corpus
            .iter()
            .map(|source| super::super::score_source(&FakeModelProvider, None, source))
            .filter_map(|score| score.content_fit.map(|fit| (score.source_id, fit)))
            .collect::<std::collections::BTreeMap<_, _>>();

        let scored_sources = scores.keys().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            scored_sources,
            ["apostles-creed", "mitochondria", "nato-alphabet"]
        );

        let mitochondria = scores.get("mitochondria").expect("mitochondria fit");
        assert!(
            mitochondria.passes(),
            "conceptual prose is the regression guard"
        );

        let nato = scores.get("nato-alphabet").expect("nato fit");
        assert_eq!(nato.classification, ContentFitCheck::Pass);
        assert_eq!(nato.covered_units, 26);
        assert_eq!(nato.required_units, 26);
        assert_eq!(nato.shape, ContentFitCheck::Pass);
        assert_eq!(nato.directionality, ContentFitCheck::Pass);
        assert!(nato.passes());

        let creed = scores.get("apostles-creed").expect("creed fit");
        assert_eq!(creed.classification, ContentFitCheck::Pass);
        assert_eq!(creed.covered_units, 6);
        assert_eq!(creed.required_units, 6);
        assert_eq!(creed.shape, ContentFitCheck::Pass);
        assert!(creed.passes());
    }
}
