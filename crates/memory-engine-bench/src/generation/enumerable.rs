use memory_engine_generation::DraftCandidate;
use serde::Deserialize;

use super::normalize;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct EnumerableSetExpectation {
    direction: EnumerableDirection,
    members: Vec<EnumerableMember>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum EnumerableDirection {
    OrdinalToMember,
}

#[derive(Clone, Debug, Deserialize)]
struct EnumerableMember {
    ordinal: usize,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnumerableSetScore {
    pub expected: usize,
    pub observed: usize,
    pub covered: usize,
    pub missing: usize,
    pub duplicates: usize,
    pub invented: usize,
    pub misassigned: usize,
    pub reversed: usize,
    pub order_ok: bool,
    pub direction_ok: bool,
}

impl EnumerableSetScore {
    pub(crate) fn passes(&self) -> bool {
        self.observed == self.expected
            && self.covered == self.expected
            && self.missing == 0
            && self.duplicates == 0
            && self.invented == 0
            && self.misassigned == 0
            && self.reversed == 0
            && self.order_ok
            && self.direction_ok
    }
}

pub(super) fn score(
    expectation: Option<&EnumerableSetExpectation>,
    candidates: &[DraftCandidate],
) -> Option<EnumerableSetScore> {
    let expectation = expectation?;
    let expected_ordinals: Vec<usize> = expectation
        .members
        .iter()
        .map(|member| member.ordinal)
        .collect();
    let fixture_is_contiguous =
        expected_ordinals == (1..=expected_ordinals.len()).collect::<Vec<_>>();
    let mut observed_ordinals = Vec::new();
    let mut exact_ordinals = Vec::new();
    let mut duplicates = std::collections::BTreeSet::new();
    let mut invented = 0;
    let mut misassigned = 0;
    let mut reversed = 0;

    for candidate in candidates {
        let question_ordinal = expectation
            .members
            .iter()
            .find(|member| ordinal_in_text(member.ordinal, &candidate.question))
            .map(|member| member.ordinal);
        let question_member = expectation
            .members
            .iter()
            .find(|member| member_name_in_text(member, &candidate.question));
        let answer_ordinal = expectation
            .members
            .iter()
            .find(|member| ordinal_in_text(member.ordinal, &candidate.answer))
            .map(|member| member.ordinal);
        // The question ordinal is the identity key. Looking up the answer name
        // first misassigns repeated people such as Grover Cleveland (22/24)
        // and Donald J. Trump (45/47) to their first occurrence.
        let answer_member = question_ordinal
            .and_then(|ordinal| {
                expectation.members.iter().find(|member| {
                    member.ordinal == ordinal && member_name_in_text(member, &candidate.answer)
                })
            })
            .or_else(|| {
                expectation
                    .members
                    .iter()
                    .find(|member| member_name_in_text(member, &candidate.answer))
            });
        let answer_ordinal_matches_question =
            answer_ordinal.is_none_or(|ordinal| question_ordinal == Some(ordinal));

        match (
            question_ordinal,
            answer_member,
            answer_ordinal_matches_question,
        ) {
            (Some(ordinal), Some(member), true) => {
                observed_ordinals.push(ordinal);
                if member.ordinal == ordinal {
                    exact_ordinals.push(ordinal);
                } else {
                    misassigned += 1;
                }
                if !exact_ordinals.is_empty()
                    && exact_ordinals[..exact_ordinals.len() - 1].contains(&ordinal)
                {
                    duplicates.insert(ordinal);
                }
            }
            (Some(ordinal), Some(_), false) => {
                observed_ordinals.push(ordinal);
                misassigned += 1;
            }
            (None, _, _) if question_member.is_some() && answer_ordinal.is_some() => reversed += 1,
            _ => invented += 1,
        }
    }

    let covered = expected_ordinals
        .iter()
        .filter(|ordinal| exact_ordinals.contains(ordinal))
        .count();
    let missing = expected_ordinals
        .iter()
        .filter(|ordinal| !exact_ordinals.contains(ordinal))
        .count();

    Some(EnumerableSetScore {
        expected: expected_ordinals.len(),
        observed: candidates.len(),
        covered,
        missing,
        duplicates: duplicates.len(),
        invented,
        misassigned,
        reversed,
        order_ok: observed_ordinals == expected_ordinals,
        direction_ok: expectation.direction == EnumerableDirection::OrdinalToMember
            && fixture_is_contiguous
            && reversed == 0
            && misassigned == 0,
    })
}

fn member_name_in_text(member: &EnumerableMember, text: &str) -> bool {
    std::iter::once(&member.name)
        .chain(member.aliases.iter())
        .any(|name| contains_normalized_phrase(text, name))
}

fn ordinal_in_text(ordinal: usize, text: &str) -> bool {
    let ordinal = ordinal.to_string();
    let suffix = match ordinal.as_bytes().last().copied() {
        Some(b'1') if ordinal != "11" => "st",
        Some(b'2') if ordinal != "12" => "nd",
        Some(b'3') if ordinal != "13" => "rd",
        _ => "th",
    };
    let ordinal_word = ordinal_word(ordinal.parse().expect("ordinal fits usize"));
    [
        ordinal.clone(),
        format!("{ordinal}{suffix}"),
        format!("number {ordinal}"),
        format!("numbered {ordinal}"),
        format!("number {ordinal_word}"),
        ordinal_word.to_owned(),
    ]
    .iter()
    .any(|pattern| contains_normalized_phrase(text, pattern))
}

fn ordinal_word(ordinal: usize) -> &'static str {
    match ordinal {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        6 => "sixth",
        7 => "seventh",
        8 => "eighth",
        9 => "ninth",
        10 => "tenth",
        11 => "eleventh",
        12 => "twelfth",
        13 => "thirteenth",
        14 => "fourteenth",
        15 => "fifteenth",
        16 => "sixteenth",
        17 => "seventeenth",
        18 => "eighteenth",
        19 => "nineteenth",
        20 => "twentieth",
        21 => "twenty first",
        22 => "twenty second",
        23 => "twenty third",
        24 => "twenty fourth",
        25 => "twenty fifth",
        26 => "twenty sixth",
        27 => "twenty seventh",
        28 => "twenty eighth",
        29 => "twenty ninth",
        30 => "thirtieth",
        31 => "thirty first",
        32 => "thirty second",
        33 => "thirty third",
        34 => "thirty fourth",
        35 => "thirty fifth",
        36 => "thirty sixth",
        37 => "thirty seventh",
        38 => "thirty eighth",
        39 => "thirty ninth",
        40 => "fortieth",
        41 => "forty first",
        42 => "forty second",
        43 => "forty third",
        44 => "forty fourth",
        45 => "forty fifth",
        46 => "forty sixth",
        47 => "forty seventh",
        _ => "",
    }
}

fn contains_normalized_phrase(haystack: &str, needle: &str) -> bool {
    let haystack = normalize(haystack);
    let needle = normalize(needle);
    !needle.is_empty() && format!(" {haystack} ").contains(&format!(" {needle} "))
}

#[cfg(test)]
mod tests {
    use memory_engine_persistence::GeneratedLearningActivityKind;

    use super::*;

    fn member(ordinal: usize, name: &str) -> EnumerableMember {
        EnumerableMember {
            ordinal,
            name: name.to_owned(),
            aliases: Vec::new(),
        }
    }

    fn expectation() -> EnumerableSetExpectation {
        EnumerableSetExpectation {
            direction: EnumerableDirection::OrdinalToMember,
            members: vec![member(1, "George Washington"), member(2, "John Adams")],
        }
    }

    fn candidate(question: &str, answer: &str) -> DraftCandidate {
        DraftCandidate {
            index: 1,
            concept: "presidents".to_owned(),
            question: question.to_owned(),
            answer: answer.to_owned(),
            evidence: None,
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        }
    }

    #[test]
    fn complete_ordinal_fixture_passes_without_exact_prompt_wording() {
        let candidates = vec![
            candidate(
                "Name the first president.",
                "George Washington served as president.",
            ),
            candidate("Who held the office numbered 2?", "John Adams."),
        ];

        let score = score(Some(&expectation()), &candidates).expect("applicable");

        assert!(score.passes(), "{score:?}");
    }

    fn presidents_expectation() -> EnumerableSetExpectation {
        super::super::load_corpus()
            .expect("corpus")
            .into_iter()
            .find(|source| source.id == "us-presidents-ordinal")
            .and_then(|source| source.expect.enumerable_set)
            .expect("presidents enumerable expectation")
    }

    fn ordinal_candidate(ordinal: usize, answer: &str) -> DraftCandidate {
        candidate(&format!("Who was president number {ordinal}?"), answer)
    }

    #[test]
    fn fully_correct_47_card_set_passes_with_repeated_names() {
        let expectation = presidents_expectation();
        let candidates = expectation
            .members
            .iter()
            .map(|member| ordinal_candidate(member.ordinal, &member.name))
            .collect::<Vec<_>>();

        let score = score(Some(&expectation), &candidates).expect("applicable");

        assert_eq!(score.expected, 47);
        assert_eq!(score.covered, 47);
        assert_eq!(score.missing, 0);
        assert_eq!(score.misassigned, 0);
        assert!(score.passes(), "{score:?}");
    }

    #[test]
    fn rejects_an_answer_with_a_repeated_name_but_wrong_ordinal() {
        let expectation = presidents_expectation();
        let mut candidates = expectation
            .members
            .iter()
            .map(|member| ordinal_candidate(member.ordinal, &member.name))
            .collect::<Vec<_>>();
        candidates[23].answer = "Grover Cleveland was president number 22.".to_owned();

        let score = score(Some(&expectation), &candidates).expect("applicable");

        assert_eq!(score.misassigned, 1);
        assert!(!score.passes(), "an ordinal-misassignment mutant must fail");
    }

    #[test]
    fn detects_missing_duplicate_invented_reversed_and_order_corruption() {
        let candidates = vec![
            candidate("Who was the 2nd president?", "George Washington"),
            candidate("Who was president number 2?", "John Adams"),
            candidate("Who was the 2nd president?", "John Adams"),
            candidate("Who was the 99th president?", "A fictional president"),
            candidate("Which presidency did George Washington hold?", "1"),
        ];

        let score = score(Some(&expectation()), &candidates).expect("applicable");

        assert_eq!(score.missing, 1);
        assert_eq!(score.duplicates, 1);
        assert_eq!(score.invented, 1);
        assert_eq!(score.reversed, 1);
        assert!(!score.order_ok);
        assert!(!score.direction_ok);
        assert!(!score.passes());
    }

    #[test]
    fn rejects_an_exact_but_reordered_set() {
        let candidates = vec![
            candidate("Who was the 2nd president?", "John Adams"),
            candidate("Name the first president.", "George Washington"),
        ];

        let score = score(Some(&expectation()), &candidates).expect("applicable");

        assert_eq!(score.covered, 2);
        assert_eq!(score.missing, 0);
        assert_eq!(score.duplicates, 0);
        assert!(!score.order_ok);
        assert!(!score.passes());
    }

    #[test]
    fn conceptual_sources_are_not_applicable() {
        assert!(score(None, &[candidate("Explain mitochondria", "cell energy")]).is_none());
    }
}
