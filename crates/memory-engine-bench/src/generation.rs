//! Generation eval: deterministic judges over the prose→quiz corpus.
//!
//! `cargo run -p memory-engine-bench -- generation --model <id>` runs every
//! corpus source through a draft provider and scores the output with judges
//! that keep models out of the loop (a `--judge <model-id>` lane adds rubric quality scoring): schema validity, provenance
//! (evidence-quote-actually-in-source, the same predicate the production
//! trust gate enforces), answerability, duplicates, expected draft counts,
//! and key-term coverage — alongside tokens, dollars, and latency.
//!
//! Receipts land in `docs/evals/`; CI never calls live models (the fake
//! provider is the default when no `--model` is given).

use std::{fmt::Write as _, fs, path::PathBuf};

use memory_engine_generation::{
    evidence_quote_matches, DraftCandidate, DraftProvider, FakeModelProvider,
};
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider, PromptVariant};
use memory_engine_persistence::{SourceDocument, SourceDocumentKind, SourcePermission};
use serde::Deserialize;

const NOW: i64 = 1_780_162_400_000;

#[derive(Clone, Debug, Deserialize)]
struct CorpusSource {
    id: String,
    title: String,
    category: String,
    body: String,
    expect: Expectations,
}

#[derive(Clone, Debug, Deserialize)]
struct Expectations {
    min_drafts: usize,
    max_drafts: usize,
    key_terms: Vec<String>,
}

/// Deterministic judge scores for one source's provider output.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceScore {
    pub source_id: String,
    pub category: String,
    pub drafts: usize,
    /// Candidates emitted vs candidates plus per-draft schema failures.
    pub schema_validity: f64,
    /// Fraction of drafts whose evidence quote is found in the source.
    pub provenance: f64,
    /// Fraction of drafts whose answer content appears in the source.
    pub answerability: f64,
    /// Fraction of drafts duplicating an earlier draft of the same source.
    pub duplicate_rate: f64,
    /// Draft count landed inside the annotated [min, max] expectation.
    pub count_in_range: bool,
    /// Fraction of annotated key terms covered by at least one draft.
    pub key_term_coverage: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
    /// Transport-level provider failure, recorded instead of scores.
    pub provider_error: Option<String>,
    /// Model-judge rubric aggregate when `--judge` is enabled.
    pub judge: Option<crate::judge::JudgeAggregate>,
    pub judge_error: Option<String>,
}

pub struct GenerationBenchArgs {
    pub model: Option<String>,
    pub prompt: PromptVariant,
    pub judge: Option<String>,
    pub out: Option<PathBuf>,
}

/// Parse `generation` subcommand arguments.
///
/// # Errors
///
/// Returns a usage message on unknown flags or missing values.
pub fn parse_args(arguments: &[String]) -> Result<GenerationBenchArgs, String> {
    let mut parsed = GenerationBenchArgs {
        model: None,
        prompt: PromptVariant::Principled,
        judge: None,
        out: None,
    };
    let mut iterator = arguments.iter();
    while let Some(flag) = iterator.next() {
        match flag.as_str() {
            "--model" => {
                parsed.model = Some(
                    iterator
                        .next()
                        .ok_or("--model requires a model id, e.g. deepseek/deepseek-v4-flash")?
                        .clone(),
                );
            }
            "--prompt" => {
                parsed.prompt = match iterator
                    .next()
                    .ok_or("--prompt requires `minimal` or `principled`")?
                    .as_str()
                {
                    "minimal" => PromptVariant::Minimal,
                    "principled" => PromptVariant::Principled,
                    other => return Err(format!("unknown prompt variant: {other}")),
                };
            }
            "--judge" => {
                parsed.judge = Some(
                    iterator
                        .next()
                        .ok_or("--judge requires a model id, e.g. openai/gpt-5.4")?
                        .clone(),
                );
            }
            "--out" => {
                parsed.out = Some(PathBuf::from(
                    iterator.next().ok_or("--out requires a file path")?,
                ));
            }
            other => {
                return Err(format!(
                    "unknown flag {other}; usage: generation [--model <id>] [--prompt minimal|principled] [--judge <id>] [--out <path>]"
                ))
            }
        }
    }

    Ok(parsed)
}

/// Run the generation eval and print (and optionally write) the receipt.
///
/// # Errors
///
/// Returns a message when the corpus cannot be read or provider
/// configuration is invalid.
pub fn run(arguments: &[String]) -> Result<(), String> {
    let parsed = parse_args(arguments)?;
    let corpus = load_corpus()?;

    let (provider, label): (Box<dyn DraftProvider>, String) = match &parsed.model {
        None => (Box::new(FakeModelProvider), "fixture/fake-model".to_owned()),
        Some(model) => {
            let mut config = OpenRouterConfig::from_env()?;
            config.model.clone_from(model);
            config.prompt = parsed.prompt;
            let label = format!("openrouter/{model} ({})", parsed.prompt.label());
            (Box::new(OpenRouterProvider::new(config)), label)
        }
    };
    let judge = parsed
        .judge
        .as_ref()
        .map(|judge_model| -> Result<OpenRouterProvider, String> {
            let mut config = OpenRouterConfig::from_env()?;
            config.model.clone_from(judge_model);
            Ok(OpenRouterProvider::new(config))
        })
        .transpose()?;
    let mut label = label;
    if let Some(judge_model) = &parsed.judge {
        let _ = write!(label, " · judge: {judge_model}");
        let generator = parsed.model.as_deref().unwrap_or("fixture/fake-model");
        if crate::judge::same_model_family(generator, judge_model) {
            label.push_str(" ⚠ same model family as generator (self-preference risk)");
        }
    }

    let scores: Vec<SourceScore> = corpus
        .iter()
        .map(|source| score_source(provider.as_ref(), judge.as_ref(), source))
        .collect();
    let receipt = render_receipt(&label, &scores);
    println!("{receipt}");
    if let Some(path) = parsed.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &receipt).map_err(|error| error.to_string())?;
        println!("receipt written to {}", path.display());
    }

    Ok(())
}

fn load_corpus() -> Result<Vec<CorpusSource>, String> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/generation");
    let mut entries: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read corpus at {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    entries.sort();
    entries
        .iter()
        .map(|path| {
            let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))
        })
        .collect()
}

fn score_source(
    provider: &dyn DraftProvider,
    model_judge: Option<&OpenRouterProvider>,
    source: &CorpusSource,
) -> SourceScore {
    let document = SourceDocument {
        id: source.id.clone(),
        kind: SourceDocumentKind::Text,
        title: source.title.clone(),
        body: Some(source.body.clone()),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        created_at: NOW,
        archived_at: None,
    };

    match provider.generate_drafts(&document) {
        Ok(drafts) => {
            let mut score = deterministic_judges(&source.body, &source.expect, &drafts.candidates);
            score.source_id.clone_from(&source.id);
            score.category.clone_from(&source.category);
            let emitted = drafts.candidates.len() + drafts.failures.len();
            score.schema_validity = if emitted == 0 {
                1.0
            } else {
                fraction(drafts.candidates.len(), emitted)
            };
            if let Some(usage) = drafts.usage {
                score.input_tokens = usage.input_tokens;
                score.output_tokens = usage.output_tokens;
                score.cost_usd_micros = usage.cost_usd_micros;
                score.latency_ms = usage.latency_ms;
            }
            if let Some(judge) = model_judge {
                match crate::judge::judge_source(
                    judge,
                    &source.title,
                    &source.body,
                    &drafts.candidates,
                ) {
                    Ok(aggregate) => score.judge = aggregate,
                    Err(failure) => score.judge_error = Some(failure.to_string()),
                }
            }
            score
        }
        Err(failure) => SourceScore {
            source_id: source.id.clone(),
            category: source.category.clone(),
            drafts: 0,
            schema_validity: 0.0,
            provenance: 0.0,
            answerability: 0.0,
            duplicate_rate: 0.0,
            count_in_range: false,
            key_term_coverage: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd_micros: None,
            latency_ms: 0,
            provider_error: Some(failure.to_string()),
            judge: None,
            judge_error: None,
        },
    }
}

/// Deterministic judges over one source's candidates. No model in the loop —
/// they verify mechanical properties only; rubric quality judgment is the
/// model judge's job (`crate::judge`).
fn deterministic_judges(
    body: &str,
    expect: &Expectations,
    candidates: &[DraftCandidate],
) -> SourceScore {
    let drafts = candidates.len();
    let provenance = fraction(
        candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .evidence
                    .as_deref()
                    .is_some_and(|quote| evidence_quote_matches(body, quote))
            })
            .count(),
        drafts,
    );
    let answerability = fraction(
        candidates
            .iter()
            .filter(|candidate| answer_supported(body, &candidate.answer))
            .count(),
        drafts,
    );
    let mut seen = std::collections::BTreeSet::new();
    let duplicates = candidates
        .iter()
        .filter(|candidate| {
            !seen.insert(format!(
                "{}\0{}",
                candidate.question.to_lowercase(),
                candidate.answer.to_lowercase()
            ))
        })
        .count();
    let covered_terms = expect
        .key_terms
        .iter()
        .filter(|term| {
            candidates.iter().any(|candidate| {
                let haystack = format!(
                    "{} {} {} {}",
                    candidate.concept,
                    candidate.question,
                    candidate.answer,
                    candidate.distractors.join(" ")
                );
                normalize(&haystack).contains(&normalize(term))
            })
        })
        .count();

    SourceScore {
        source_id: String::new(),
        category: String::new(),
        drafts,
        schema_validity: 1.0,
        provenance,
        answerability,
        duplicate_rate: fraction(duplicates, drafts),
        count_in_range: drafts >= expect.min_drafts && drafts <= expect.max_drafts,
        key_term_coverage: fraction(covered_terms, expect.key_terms.len()),
        input_tokens: 0,
        output_tokens: 0,
        cost_usd_micros: None,
        latency_ms: 0,
        provider_error: None,
        judge: None,
        judge_error: None,
    }
}

/// An answer is "supported" when at least 60% of its content words appear in
/// the source. String-level only; semantic paraphrase scores low by design.
fn answer_supported(body: &str, answer: &str) -> bool {
    let body = normalize(body);
    let body_words: std::collections::BTreeSet<&str> = body.split(' ').collect();
    let answer = normalize(answer);
    let mut words: Vec<&str> = answer.split(' ').filter(|word| word.len() > 2).collect();
    // Short answers (numbers, "to be") have no long content words; fall back to
    // matching the answer's actual words so they are not credited for free.
    if words.is_empty() {
        words = answer.split(' ').filter(|word| !word.is_empty()).collect();
    }
    if words.is_empty() {
        return false;
    }
    let found = words
        .iter()
        .filter(|word| body_words.contains(**word))
        .count();

    fraction(found, words.len()) >= 0.6
}

fn normalize(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space = true;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            previous_space = false;
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    while normalized.ends_with(' ') {
        normalized.pop();
    }

    normalized
}

#[allow(clippy::cast_precision_loss)]
fn fraction(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Render the model-judge section when `--judge` ran: per-source rubric
/// means (1-5), keep rate, and the judge's notes on rejected drafts.
fn render_model_judge(receipt: &mut String, scores: &[SourceScore]) {
    if scores
        .iter()
        .all(|score| score.judge.is_none() && score.judge_error.is_none())
    {
        return;
    }

    let _ = writeln!(receipt, "## Model judge (rubric 1-5)");
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "| source | faithfulness | question quality | distractors | keep | judge cost |"
    );
    let _ = writeln!(receipt, "| --- | --- | --- | --- | --- | --- |");
    let mut judge_cost: i64 = 0;
    let mut reject_notes = Vec::new();
    for score in scores {
        if let Some(error) = &score.judge_error {
            let _ = writeln!(
                receipt,
                "| {} | — | — | — | — | JUDGE FAILED: {error} |",
                score.source_id
            );
            continue;
        }
        let Some(judge) = &score.judge else { continue };
        judge_cost += judge.cost_usd_micros.unwrap_or(0);
        reject_notes.extend(
            judge
                .reject_notes
                .iter()
                .map(|note| format!("{}: {note}", score.source_id)),
        );
        let _ = writeln!(
            receipt,
            "| {} | {:.1} | {:.1} | {:.1} | {:.0}% | {} |",
            score.source_id,
            judge.faithfulness,
            judge.question_quality,
            judge.distractor_quality,
            judge.keep_rate * 100.0,
            format_cost(judge.cost_usd_micros),
        );
    }
    let judged: Vec<_> = scores
        .iter()
        .filter_map(|score| score.judge.as_ref())
        .collect();
    if !judged.is_empty() {
        #[allow(clippy::cast_precision_loss)]
        let count = judged.len() as f64;
        let mean = |value: fn(&crate::judge::JudgeAggregate) -> f64| {
            judged.iter().map(|judge| value(judge)).sum::<f64>() / count
        };
        let _ = writeln!(receipt);
        let _ = writeln!(
            receipt,
            "- Judge means: faithfulness {:.1} · question quality {:.1} · distractors {:.1} · keep rate {:.0}% · judge cost {}",
            mean(|judge| judge.faithfulness),
            mean(|judge| judge.question_quality),
            mean(|judge| judge.distractor_quality),
            mean(|judge| judge.keep_rate) * 100.0,
            format_cost(Some(judge_cost)),
        );
    }
    if !reject_notes.is_empty() {
        let _ = writeln!(receipt);
        let _ = writeln!(receipt, "Judge would not keep:");
        let _ = writeln!(receipt);
        for note in reject_notes {
            let _ = writeln!(receipt, "- {note}");
        }
    }
    let _ = writeln!(receipt);
}

fn render_receipt(label: &str, scores: &[SourceScore]) -> String {
    let mut receipt = String::new();
    let _ = writeln!(receipt, "# Generation eval receipt");
    let _ = writeln!(receipt);
    let _ = writeln!(receipt, "- Provider: {label}");
    let _ = writeln!(receipt, "- Corpus: {} sources", scores.len());
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | tokens in/out | cost | latency |"
    );
    let _ = writeln!(
        receipt,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    );
    for score in scores {
        if let Some(error) = &score.provider_error {
            let _ = writeln!(
                receipt,
                "| {} | {} | — | — | — | — | — | — | — | — | — | FAILED: {error} |",
                score.source_id, score.category
            );
            continue;
        }
        let _ = writeln!(
            receipt,
            "| {} | {} | {} | {:.0}% | {:.0}% | {:.0}% | {:.0}% | {} | {:.0}% | {}/{} | {} | {}ms |",
            score.source_id,
            score.category,
            score.drafts,
            score.schema_validity * 100.0,
            score.provenance * 100.0,
            score.answerability * 100.0,
            score.duplicate_rate * 100.0,
            if score.count_in_range { "yes" } else { "NO" },
            score.key_term_coverage * 100.0,
            score.input_tokens,
            score.output_tokens,
            format_cost(score.cost_usd_micros),
            score.latency_ms,
        );
    }
    let _ = writeln!(receipt);
    render_model_judge(&mut receipt, scores);

    let judged: Vec<&SourceScore> = scores
        .iter()
        .filter(|score| score.provider_error.is_none())
        .collect();
    let failed = scores.len() - judged.len();
    let mut latencies: Vec<u64> = judged.iter().map(|score| score.latency_ms).collect();
    latencies.sort_unstable();
    let total_cost: i64 = judged
        .iter()
        .filter_map(|score| score.cost_usd_micros)
        .sum();
    let _ = writeln!(receipt, "## Totals");
    let _ = writeln!(receipt);
    let _ = writeln!(
        receipt,
        "- Provider failures: {failed}/{} sources",
        scores.len()
    );
    let _ = writeln!(
        receipt,
        "- Mean provenance: {:.0}% · mean answerability: {:.0}% · mean key-term coverage: {:.0}% · count-in-range: {}/{}",
        mean(&judged, |score| score.provenance) * 100.0,
        mean(&judged, |score| score.answerability) * 100.0,
        mean(&judged, |score| score.key_term_coverage) * 100.0,
        judged.iter().filter(|score| score.count_in_range).count(),
        judged.len(),
    );
    let _ = writeln!(
        receipt,
        "- Total cost: {} · mean per source: {}",
        format_cost(Some(total_cost)),
        format_cost(Some(if judged.is_empty() {
            0
        } else {
            total_cost / i64::try_from(judged.len()).unwrap_or(1)
        })),
    );
    let _ = writeln!(
        receipt,
        "- Latency p50: {}ms · p95: {}ms",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
    );

    receipt
}

fn mean(scores: &[&SourceScore], value: impl Fn(&SourceScore) -> f64) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = scores.len() as f64;

    scores.iter().map(|score| value(score)).sum::<f64>() / count
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * percent).div_ceil(100);

    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn format_cost(micros: Option<i64>) -> String {
    match micros {
        None => "—".to_owned(),
        #[allow(clippy::cast_precision_loss)]
        Some(micros) => format!("${:.4}", micros as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
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

    fn expectations() -> Expectations {
        Expectations {
            min_drafts: 1,
            max_drafts: 3,
            key_terms: vec!["Alfa".to_owned(), "Bravo".to_owned()],
        }
    }

    const BODY: &str = "A is Alfa. B is Bravo. C is Charlie.";

    #[test]
    fn grounded_output_scores_perfect_provenance_and_coverage() {
        let candidates = vec![
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is B?", "Bravo", "B is Bravo."),
        ];

        let score = deterministic_judges(BODY, &expectations(), &candidates);

        assert!((score.provenance - 1.0).abs() < f64::EPSILON);
        assert!((score.answerability - 1.0).abs() < f64::EPSILON);
        assert!((score.key_term_coverage - 1.0).abs() < f64::EPSILON);
        assert!(score.duplicate_rate.abs() < f64::EPSILON);
        assert!(score.count_in_range);
    }

    #[test]
    fn fabricated_evidence_and_duplicates_are_caught() {
        let candidates = vec![
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is A?", "Alfa", "A is Alfa"),
            candidate("What is D?", "Delta", "D is Delta."),
        ];

        let score = deterministic_judges(BODY, &expectations(), &candidates);

        assert!((score.provenance - 2.0 / 3.0).abs() < 0.01);
        assert!((score.duplicate_rate - 1.0 / 3.0).abs() < 0.01);
        assert!(score.count_in_range, "3 drafts sits inside [1, 3]");
    }

    #[test]
    fn provenance_matching_tolerates_case_and_punctuation() {
        let candidates = vec![candidate("What is C?", "Charlie", "c IS charlie")];

        let score = deterministic_judges(BODY, &expectations(), &candidates);

        assert!((score.provenance - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn paraphrased_answers_score_low_on_answerability() {
        let candidates = vec![candidate(
            "What does B stand for?",
            "Bravissimo encore fortissimo",
            "B is Bravo",
        )];

        let score = deterministic_judges(BODY, &expectations(), &candidates);

        assert!(score.answerability.abs() < f64::EPSILON);
    }

    #[test]
    fn count_in_range_respects_annotations() {
        let none = deterministic_judges(BODY, &expectations(), &[]);
        assert!(!none.count_in_range);
        assert!(none.provenance.abs() < f64::EPSILON);
    }

    #[test]
    fn percentile_picks_nearest_rank() {
        assert_eq!(percentile(&[10, 20, 30, 40], 50), 20);
        assert_eq!(percentile(&[10, 20, 30, 40], 95), 40);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn corpus_loads_and_fake_provider_scores_clean() {
        let corpus = load_corpus().expect("corpus");
        assert!(corpus.len() >= 10, "047 requires ≥10 sources");

        for source in &corpus {
            let score = score_source(&FakeModelProvider, None, source);
            assert!(score.provider_error.is_none());
            assert!(
                (score.provenance - 1.0).abs() < f64::EPSILON,
                "fake provider quotes verbatim; {} scored {}",
                source.id,
                score.provenance
            );
        }
    }
}
