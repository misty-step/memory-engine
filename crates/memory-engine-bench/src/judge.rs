//! Model judge for generated drafts.
//!
//! Deterministic judges verify mechanical properties (quote-in-source,
//! duplicates, schema); they cannot assess whether a question is well-posed,
//! whether distractors are plausible confusions, or whether a draft is worth
//! keeping. This lane has a strong model grade each draft against an anchored
//! rubric. Per ticket 047, it is never the only signal — the deterministic
//! judges stay as the anti-gaming guardrail — and CI never runs it (live
//! judge runs are explicit via `--judge <model-id>`).
//!
//! Self-preference bias is real: a model rates its own outputs higher. Pick a
//! judge from a different provider family than the generator; the receipt
//! carries a warning when the families match.

use std::fmt::Write as _;

use memory_engine_generation::{DraftCandidate, ProviderFailure};
use memory_engine_openrouter::OpenRouterProvider;
use serde::Deserialize;

/// One draft's rubric scores from the judge, 1 (unusable) to 5 (excellent).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DraftVerdict {
    /// 1-based index matching the candidate order sent to the judge.
    pub index: usize,
    /// Is the answer actually correct given the source (5) or contradicted /
    /// unsupported by it (1)?
    pub faithfulness: u8,
    /// Standalone, unambiguous, tests a retention-worthy atom (5) vs vague,
    /// context-dependent, or trivia (1).
    pub question_quality: u8,
    /// Distractors are plausible adjacent confusions (5) vs absent when
    /// needed, obviously wrong, or format variants (1).
    pub distractor_quality: u8,
    /// Would a learning-science-literate editor keep this draft as-is?
    pub keep: bool,
    /// One short sentence justifying the weakest score.
    pub note: String,
}

/// Aggregated judge scores for one source's drafts.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgeAggregate {
    pub faithfulness: f64,
    pub question_quality: f64,
    pub distractor_quality: f64,
    /// Fraction of drafts the judge would keep as-is.
    pub keep_rate: f64,
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
    /// Notes for drafts the judge would not keep, for the receipt appendix.
    pub reject_notes: Vec<String>,
}

/// Judge one source's candidates. Returns `None` when there is nothing to
/// judge (zero candidates).
///
/// # Errors
///
/// Propagates the judge model's transport/parse failure.
pub fn judge_source(
    judge: &OpenRouterProvider,
    source_title: &str,
    source_body: &str,
    candidates: &[DraftCandidate],
) -> Result<Option<JudgeAggregate>, ProviderFailure> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let response = judge.complete_structured(
        &judge_prompt(source_title, source_body, candidates),
        "draft_verdicts",
        &verdicts_schema(),
    )?;
    let verdicts = parse_verdicts(&response.content, candidates.len())?;
    let mut aggregate = aggregate(&verdicts);
    if let Some(usage) = response.usage {
        aggregate.cost_usd_micros = usage.cost_usd_micros;
        aggregate.latency_ms = usage.latency_ms;
    }

    Ok(Some(aggregate))
}

/// Whether generator and judge share a provider family (e.g. both
/// `google/...`), which invites self-preference bias.
#[must_use]
pub fn same_model_family(generator_model: &str, judge_model: &str) -> bool {
    let family = |model: &str| model.split('/').next().unwrap_or(model).to_owned();

    family(generator_model) == family(judge_model)
}

fn judge_prompt(title: &str, body: &str, candidates: &[DraftCandidate]) -> String {
    let mut drafts = String::new();
    for candidate in candidates {
        let _ = write!(
            drafts,
            "DRAFT {index}\nquestion: {question}\nanswer: {answer}\ndistractors: {distractors}\n\n",
            index = candidate.index,
            question = candidate.question,
            answer = candidate.answer,
            distractors = if candidate.distractors.is_empty() {
                "(short answer, none)".to_owned()
            } else {
                candidate.distractors.join(" | ")
            },
        );
    }

    format!(
        "You are a strict editor of spaced-repetition quiz items, grading drafts \
generated from a source document.

SOURCE TITLE: {title}
SOURCE TEXT:
{body}

{drafts}For every draft, score each dimension 1-5 against these anchors:
- faithfulness: 5 = the answer is exactly what the source says; 3 = roughly right \
but imprecise; 1 = contradicted by or absent from the source.
- question_quality: 5 = standalone, unambiguous, names its topic, tests an atom a \
learner should retain; 3 = answerable but vague or compound; 1 = unanswerable \
without hidden context, or punctuation/format trivia.
- distractor_quality: 5 = distractors are confusions a real learner would make; \
3 = plausible but lazy; 1 = obviously wrong or format variants. For short-answer \
drafts with no distractors, score 3 if short-answer suits the item, lower if \
multiple-choice was clearly needed.
- keep: true only if you would publish the draft as-is, with no edits.

Judge every draft independently. Be harsh: a 5 is rare. The note must be one \
short sentence naming the weakest aspect. Return JSON only.",
    )
}

fn verdicts_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "verdicts": {
                "type": "array",
                "items": {
                    "type": "object",
                    // Anthropic structured outputs reject minimum/maximum on
                    // integers, so the 1-5 range lives in descriptions and is
                    // enforced deterministically by parse_verdicts.
                    "properties": {
                        "index": { "type": "integer" },
                        "faithfulness": { "type": "integer", "description": "1 to 5" },
                        "question_quality": { "type": "integer", "description": "1 to 5" },
                        "distractor_quality": { "type": "integer", "description": "1 to 5" },
                        "keep": { "type": "boolean" },
                        "note": { "type": "string" }
                    },
                    "required": [
                        "index", "faithfulness", "question_quality",
                        "distractor_quality", "keep", "note"
                    ],
                    "additionalProperties": false
                }
            }
        },
        "required": ["verdicts"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
struct VerdictsPayload {
    #[serde(default)]
    verdicts: Vec<DraftVerdict>,
}

/// Parse and sanity-check the judge's verdicts: one per draft, scores in
/// range. A judge that skips drafts or invents indexes is itself unreliable,
/// so that surfaces as a failure rather than silently partial scores.
fn parse_verdicts(content: &str, expected: usize) -> Result<Vec<DraftVerdict>, ProviderFailure> {
    let parsed: VerdictsPayload = serde_json::from_str(content)
        .map_err(|_| ProviderFailure::new("The judge's verdicts could not be read."))?;
    if parsed.verdicts.len() != expected {
        return Err(ProviderFailure::new(format!(
            "The judge returned {} verdicts for {expected} drafts.",
            parsed.verdicts.len()
        )));
    }
    for verdict in &parsed.verdicts {
        let scores = [
            verdict.faithfulness,
            verdict.question_quality,
            verdict.distractor_quality,
        ];
        if scores.iter().any(|score| !(1..=5).contains(score)) {
            return Err(ProviderFailure::new(format!(
                "The judge scored draft {} outside the 1-5 rubric.",
                verdict.index
            )));
        }
    }

    Ok(parsed.verdicts)
}

fn aggregate(verdicts: &[DraftVerdict]) -> JudgeAggregate {
    let mean = |value: fn(&DraftVerdict) -> u8| {
        #[allow(clippy::cast_precision_loss)]
        let count = verdicts.len() as f64;
        verdicts
            .iter()
            .map(|verdict| f64::from(value(verdict)))
            .sum::<f64>()
            / count
    };
    let kept = verdicts.iter().filter(|verdict| verdict.keep).count();
    #[allow(clippy::cast_precision_loss)]
    let keep_rate = kept as f64 / verdicts.len() as f64;

    JudgeAggregate {
        faithfulness: mean(|verdict| verdict.faithfulness),
        question_quality: mean(|verdict| verdict.question_quality),
        distractor_quality: mean(|verdict| verdict.distractor_quality),
        keep_rate,
        cost_usd_micros: None,
        latency_ms: 0,
        reject_notes: verdicts
            .iter()
            .filter(|verdict| !verdict.keep)
            .map(|verdict| format!("draft {}: {}", verdict.index, verdict.note))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict_json(index: usize, faith: u8, keep: bool) -> serde_json::Value {
        serde_json::json!({
            "index": index,
            "faithfulness": faith,
            "question_quality": 4,
            "distractor_quality": 3,
            "keep": keep,
            "note": "distractors are lazy"
        })
    }

    #[test]
    fn parses_and_aggregates_verdicts() {
        let content = serde_json::json!({
            "verdicts": [verdict_json(1, 5, true), verdict_json(2, 3, false)]
        })
        .to_string();

        let verdicts = parse_verdicts(&content, 2).expect("verdicts");
        let aggregate = aggregate(&verdicts);

        assert!((aggregate.faithfulness - 4.0).abs() < f64::EPSILON);
        assert!((aggregate.keep_rate - 0.5).abs() < f64::EPSILON);
        assert_eq!(aggregate.reject_notes, ["draft 2: distractors are lazy"]);
    }

    #[test]
    fn wrong_verdict_count_is_a_failure_not_partial_scores() {
        let content = serde_json::json!({ "verdicts": [verdict_json(1, 5, true)] }).to_string();

        let failure = parse_verdicts(&content, 2).expect_err("must fail");
        assert!(failure.to_string().contains("1 verdicts for 2 drafts"));
    }

    #[test]
    fn out_of_range_scores_are_rejected() {
        let content = serde_json::json!({ "verdicts": [verdict_json(1, 9, true)] }).to_string();

        assert!(parse_verdicts(&content, 1).is_err());
    }

    #[test]
    fn unparseable_judge_output_is_a_human_readable_failure() {
        let failure = parse_verdicts("not json", 1).expect_err("must fail");
        assert!(failure.to_string().contains("could not be read"));
    }

    #[test]
    fn same_family_detection_flags_self_preference() {
        assert!(same_model_family(
            "google/gemini-3.5-flash",
            "google/gemini-3.1-pro-preview"
        ));
        assert!(!same_model_family(
            "google/gemini-3.5-flash",
            "openai/gpt-5.4"
        ));
    }
}
