//! Provider boundary for draft generation.
//!
//! A [`DraftProvider`] turns one source document into draft candidates plus
//! human-readable failures. Providers own *candidate production* only; the
//! trust gate (provenance verification, duplicate suppression, persistence)
//! stays in [`run_beta_generation_with_provider`](crate::run_beta_generation_with_provider)
//! so every provider's output passes through the same validation. Model-backed
//! providers live in boundary crates outside this one; this crate ships the
//! deterministic providers used by tests, CI, and the structured-block flow.

use std::{error::Error, fmt};

use memory_engine_persistence::{
    GeneratedLearningActivityKind, GeneratedPromptModel, SourceDocument,
};

/// One generated draft candidate before validation.
///
/// `index` is a stable 1-based position within the source (structured blocks
/// use their block number) and participates in generated draft ids.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftCandidate {
    pub index: usize,
    pub concept: String,
    pub question: String,
    pub answer: String,
    /// Verbatim quote from the source backing this draft. Drafts without
    /// evidence are dropped; drafts whose evidence cannot be found in the
    /// source are persisted as rejected.
    pub evidence: Option<String>,
    pub distractors: Vec<String>,
    pub worked_solution: Option<String>,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub unsupported: bool,
}

/// Token and cost accounting reported by a provider for one source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost in integer micro-USD; `None` when the provider is free/local.
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
}

/// Provider output for one source document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDrafts {
    pub model: GeneratedPromptModel,
    pub learning_intent: Option<LearningIntent>,
    pub candidates: Vec<DraftCandidate>,
    /// Human-readable, per-candidate problems the provider already detected
    /// (for example malformed blocks). Surfaced to the learner alongside
    /// validation failures.
    pub failures: Vec<String>,
    pub usage: Option<ProviderUsage>,
}

/// A transport- or provider-level failure that prevented draft generation.
///
/// The message is shown to learners verbatim, so providers must phrase it as
/// a human sentence, never a debug dump.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    message: String,
}

impl ProviderFailure {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ProviderFailure {}

/// Produces draft candidates from one source document.
pub trait DraftProvider {
    /// Identity recorded on generation runs, available before any generation
    /// happens so failed runs still carry a model receipt.
    fn model(&self) -> GeneratedPromptModel;

    /// Generate draft candidates for `source`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] when the provider cannot produce any
    /// output for the source (transport failure, malformed response). The
    /// message is surfaced to the learner.
    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearningIntent {
    VerbatimMemorization,
    ConceptUnderstanding,
    FactRecall,
    ProcedureProcess,
}

impl LearningIntent {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::VerbatimMemorization => "verbatim_memorization",
            Self::ConceptUnderstanding => "concept_understanding",
            Self::FactRecall => "fact_recall",
            Self::ProcedureProcess => "procedure_process",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label.trim() {
            "verbatim_memorization" => Some(Self::VerbatimMemorization),
            "concept_understanding" => Some(Self::ConceptUnderstanding),
            "fact_recall" => Some(Self::FactRecall),
            "procedure_process" => Some(Self::ProcedureProcess),
            _ => None,
        }
    }
}

impl fmt::Display for LearningIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningIntentClassification {
    pub intent: LearningIntent,
    pub rationale: String,
}

#[must_use]
pub fn classify_learning_intent(source: &SourceDocument) -> LearningIntentClassification {
    let body = source.body.as_deref().unwrap_or_default();
    let normalized = body.to_lowercase();
    let lines = non_empty_lines(body);
    let list_facts = count_fact_sentences(body);
    if looks_verbatim(&normalized, &lines) {
        return LearningIntentClassification {
            intent: LearningIntent::VerbatimMemorization,
            rationale: "source reads like a quoted passage or line-broken text".to_owned(),
        };
    }
    if looks_process(&normalized) {
        return LearningIntentClassification {
            intent: LearningIntent::ProcedureProcess,
            rationale: "source describes ordered actions or conditional steps".to_owned(),
        };
    }
    if looks_concept(&normalized) {
        return LearningIntentClassification {
            intent: LearningIntent::ConceptUnderstanding,
            rationale: "source emphasizes causes, mechanisms, or explanatory ideas".to_owned(),
        };
    }
    if list_facts >= 2 || looks_like_tiny_fact(&normalized) {
        return LearningIntentClassification {
            intent: LearningIntent::FactRecall,
            rationale: "source is dominated by discrete facts".to_owned(),
        };
    }

    LearningIntentClassification {
        intent: LearningIntent::ConceptUnderstanding,
        rationale: "source asks for explanation or conceptual retention".to_owned(),
    }
}

/// Deterministic provider that parses `Concept:/Question:/Answer:` blocks.
///
/// This is the original beta generation behavior, now one provider among
/// several.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructuredBlockProvider;

impl DraftProvider for StructuredBlockProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "deterministic-beta-generator".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let mut failures = Vec::new();
        let candidates = parse_source_document(source, &mut failures);

        Ok(ProviderDrafts {
            model: self.model(),
            learning_intent: Some(classify_learning_intent(source).intent),
            candidates,
            failures,
            usage: None,
        })
    }
}

/// Routes each source to a primary provider, falling back to a secondary
/// provider only when the primary produces no candidates.
///
/// Wiring `StructuredBlockProvider` as primary and a model provider as
/// fallback keeps the deterministic, free path for hand-written
/// `Concept:/Question:/Answer:` blocks while sending arbitrary prose — which
/// the structured parser cannot handle — to the model. Primary parse
/// failures are dropped when the fallback runs, so prose is not reported as
/// malformed blocks.
pub struct FallbackProvider<'a> {
    primary: &'a dyn DraftProvider,
    fallback: &'a dyn DraftProvider,
}

impl<'a> FallbackProvider<'a> {
    #[must_use]
    pub fn new(primary: &'a dyn DraftProvider, fallback: &'a dyn DraftProvider) -> Self {
        Self { primary, fallback }
    }
}

impl DraftProvider for FallbackProvider<'_> {
    fn model(&self) -> GeneratedPromptModel {
        self.fallback.model()
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let primary = self.primary.generate_drafts(source)?;
        if primary.candidates.is_empty() {
            self.fallback.generate_drafts(source)
        } else {
            Ok(primary)
        }
    }
}

/// Deterministic stand-in for a model provider, used by CI and tests.
///
/// Emits one short-answer draft per leading sentence (up to three) with the
/// sentence itself as verbatim evidence, so drafts always pass the
/// provenance gate without any network access.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeModelProvider;

impl DraftProvider for FakeModelProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "fixture".to_owned(),
            name: "fake-model".to_owned(),
            version: "v1".to_owned(),
        }
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let body = source.body.clone().unwrap_or_default();
        let classification = classify_learning_intent(source);
        let candidates = match classification.intent {
            LearningIntent::VerbatimMemorization => verbatim_candidates(source, &body),
            LearningIntent::ConceptUnderstanding => concept_candidates(source, &body),
            LearningIntent::FactRecall => fact_candidates(source, &body),
            LearningIntent::ProcedureProcess => procedure_candidates(source, &body),
        };

        Ok(ProviderDrafts {
            model: self.model(),
            learning_intent: Some(classification.intent),
            candidates,
            failures: Vec::new(),
            usage: None,
        })
    }
}

fn looks_verbatim(normalized: &str, lines: &[String]) -> bool {
    normalized.contains("recite")
        || normalized.contains("memorize")
        || normalized.contains("verbatim")
        || normalized.contains("poem")
        || (lines.len() >= 3
            && lines
                .iter()
                .filter(|line| line.chars().count() <= 96)
                .count()
                >= 3)
}

fn looks_process(normalized: &str) -> bool {
    [
        "to maintain",
        "first",
        "then",
        "finally",
        "step",
        "process",
        "procedure",
        "always ",
        " once every ",
        "if it ",
        "before ",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn looks_concept(normalized: &str) -> bool {
    [
        " because ",
        " theory",
        " effect",
        " why ",
        "mechanism",
        "regulate",
        "descend",
        "principle",
        "algorithm",
        "system design",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn looks_like_tiny_fact(normalized: &str) -> bool {
    normalized.split_whitespace().count() <= 28
        || normalized.contains(" is ")
        || normalized.contains(" are ")
        || normalized.contains(" means ")
}

fn count_fact_sentences(body: &str) -> usize {
    split_sentences(body)
        .iter()
        .filter(|sentence| {
            let normalized = sentence.to_lowercase();
            normalized.contains(" is ")
                || normalized.contains(" are ")
                || normalized.contains(" means ")
                || normalized.contains(" assigns ")
        })
        .count()
}

fn verbatim_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let lines = non_empty_lines(body);
    if lines.len() >= 2 {
        let cued_answer = lines[1].clone();
        let cued_evidence = lines.iter().take(2).cloned().collect::<Vec<_>>().join("\n");
        let free_answer = lines.iter().take(4).cloned().collect::<Vec<_>>().join("\n");
        return vec![
            DraftCandidate {
                index: 1,
                concept: format!("{} cued recitation", source.title),
                question: format!("Recite the next line after: {}", lines[0]),
                answer: cued_answer.clone(),
                evidence: Some(cued_evidence),
                distractors: Vec::new(),
                worked_solution: Some(format!("The source line is: {cued_answer}")),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "cued-recall".to_owned(),
                unsupported: false,
            },
            DraftCandidate {
                index: 2,
                concept: format!("{} free recitation", source.title),
                question: format!("Recite the opening passage of {}.", source.title),
                answer: free_answer.clone(),
                evidence: Some(free_answer.clone()),
                distractors: Vec::new(),
                worked_solution: Some(format!("Compare your recitation against:\n{free_answer}")),
                activity_kind: GeneratedLearningActivityKind::Exercise,
                activity_stage: "free-recall".to_owned(),
                unsupported: false,
            },
        ];
    }

    let sentence = split_sentences(body).into_iter().next().unwrap_or_default();
    vec![DraftCandidate {
        index: 1,
        concept: format!("{} recitation", source.title),
        question: format!("Recite {} exactly.", source.title),
        answer: sentence.clone(),
        evidence: Some(sentence.clone()),
        distractors: Vec::new(),
        worked_solution: Some(format!("Compare your answer against: {sentence}")),
        activity_kind: GeneratedLearningActivityKind::Exercise,
        activity_stage: "free-recall".to_owned(),
        unsupported: false,
    }]
}

fn concept_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    split_sentences(body)
        .into_iter()
        .filter(|sentence| sentence.split_whitespace().count() >= 6)
        .take(3)
        .enumerate()
        .map(|(position, sentence)| DraftCandidate {
            index: position + 1,
            concept: format!("{} concept {}", source.title, position + 1),
            question: format!(
                "Explain the idea from \"{}\" in your own words: {}",
                source.title,
                lead_words(&sentence, 8)
            ),
            answer: sentence.clone(),
            evidence: Some(sentence),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "free-recall".to_owned(),
            unsupported: false,
        })
        .collect()
}

fn fact_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let facts = split_sentences(body)
        .into_iter()
        .filter(|sentence| {
            let normalized = sentence.to_lowercase();
            normalized.contains(" is ")
                || normalized.contains(" are ")
                || normalized.contains(" means ")
                || normalized.contains(" assigns ")
        })
        .take(6)
        .collect::<Vec<_>>();
    let facts = if facts.is_empty() {
        let sentences = split_sentences(body)
            .into_iter()
            .take(3)
            .collect::<Vec<_>>();
        if sentences.is_empty() && !body.trim().is_empty() {
            vec![body.trim().to_owned()]
        } else {
            sentences
        }
    } else {
        facts
    };

    facts
        .into_iter()
        .enumerate()
        .map(|(position, sentence)| {
            let (question, answer) = fact_question_answer(source, &sentence);
            DraftCandidate {
                index: position + 1,
                concept: format!("{} fact {}", source.title, position + 1),
                question,
                answer,
                evidence: Some(sentence),
                distractors: vec![
                    "A nearby but different source detail".to_owned(),
                    "A plausible unrelated detail".to_owned(),
                ],
                worked_solution: None,
                activity_kind: GeneratedLearningActivityKind::Quiz,
                activity_stage: "recognition".to_owned(),
                unsupported: false,
            }
        })
        .collect()
}

fn procedure_candidates(source: &SourceDocument, body: &str) -> Vec<DraftCandidate> {
    let sentences = split_sentences(body);
    let evidence = sentences
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(". ");
    let evidence = if evidence.is_empty() {
        body.trim().to_owned()
    } else {
        evidence
    };
    let mut candidates = vec![DraftCandidate {
        index: 1,
        concept: format!("{} procedure", source.title),
        question: format!("Describe the process in \"{}\" in order.", source.title),
        answer: evidence.clone(),
        evidence: Some(evidence),
        distractors: Vec::new(),
        worked_solution: None,
        activity_kind: GeneratedLearningActivityKind::Quiz,
        activity_stage: "procedure-composition".to_owned(),
        unsupported: false,
    }];
    if let Some(check_sentence) = sentences.get(2).or_else(|| sentences.get(1)) {
        candidates.push(DraftCandidate {
            index: 2,
            concept: format!("{} procedure check", source.title),
            question: format!("What condition or check matters in \"{}\"?", source.title),
            answer: check_sentence.clone(),
            evidence: Some(check_sentence.clone()),
            distractors: Vec::new(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "procedure-check".to_owned(),
            unsupported: false,
        });
    }
    candidates
}

fn fact_question_answer(source: &SourceDocument, sentence: &str) -> (String, String) {
    for separator in [" is ", " are ", " means "] {
        if let Some((left, right)) = sentence.split_once(separator) {
            let subject = left.trim();
            let answer = right.trim().to_owned();
            return (
                format!("According to \"{}\", what is {subject}?", source.title),
                answer,
            );
        }
    }
    (
        format!("What fact should you recall from \"{}\"?", source.title),
        sentence.to_owned(),
    )
}

fn split_sentences(body: &str) -> Vec<String> {
    body.split(['.', '?', '!'])
        .map(str::trim)
        .filter(|sentence| sentence.split_whitespace().count() >= 3)
        .map(str::to_owned)
        .collect()
}

fn non_empty_lines(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn lead_words(sentence: &str, count: usize) -> String {
    sentence
        .split_whitespace()
        .take(count)
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_source_document(
    source: &SourceDocument,
    validation_failures: &mut Vec<String>,
) -> Vec<DraftCandidate> {
    source
        .body
        .as_deref()
        .unwrap_or_default()
        .split("\n\n")
        .enumerate()
        .filter_map(|(index, block)| {
            parse_candidate_block(source, block, index + 1, validation_failures)
        })
        .collect()
}

fn parse_candidate_block(
    source: &SourceDocument,
    block: &str,
    block_index: usize,
    validation_failures: &mut Vec<String>,
) -> Option<DraftCandidate> {
    let fields = parse_fields(block);
    let concept = fields.iter().find_value("concept");
    let question = fields.iter().find_value("question");
    let answer = fields.iter().find_value("answer");

    let (Some(concept), Some(question), Some(answer)) = (concept, question, answer) else {
        validation_failures.push(format!(
            "{} block {block_index}: concept, question, and answer are required",
            source.id
        ));
        return None;
    };

    Some(DraftCandidate {
        index: block_index,
        activity_kind: parse_activity_kind(fields.iter().find_value("activity")),
        activity_stage: fields
            .iter()
            .find_value("stage")
            .unwrap_or("recognition")
            .to_owned(),
        concept: concept.to_owned(),
        question: question.to_owned(),
        answer: answer.to_owned(),
        evidence: fields.iter().find_value("reference").map(str::to_owned),
        distractors: split_list(fields.iter().find_value("distractors")),
        worked_solution: fields
            .iter()
            .find_value("worked solution")
            .or_else(|| fields.iter().find_value("worked"))
            .map(str::to_owned),
        unsupported: parse_boolean(fields.iter().find_value("unsupported")).unwrap_or(false)
            || parse_boolean(fields.iter().find_value("supported")) == Some(false),
    })
}

fn parse_fields(block: &str) -> Vec<(String, String)> {
    block
        .lines()
        .filter_map(|raw_line| {
            let line = raw_line.trim();
            let separator = line.find(':')?;
            let key = line[..separator].trim().to_lowercase();
            let value = line[separator + 1..].trim().to_owned();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

trait FindField<'a> {
    fn find_value(self, key: &str) -> Option<&'a str>;
}

impl<'a, T> FindField<'a> for T
where
    T: Iterator<Item = &'a (String, String)>,
{
    fn find_value(self, key: &str) -> Option<&'a str> {
        self.filter_map(|(candidate_key, value)| {
            if candidate_key == key {
                Some(value.as_str())
            } else {
                None
            }
        })
        .last()
    }
}

fn parse_activity_kind(value: Option<&str>) -> GeneratedLearningActivityKind {
    if value.is_some_and(|value| value.eq_ignore_ascii_case("exercise")) {
        GeneratedLearningActivityKind::Exercise
    } else {
        GeneratedLearningActivityKind::Quiz
    }
}

fn parse_boolean(value: Option<&str>) -> Option<bool> {
    match value?.to_lowercase().as_str() {
        "true" | "yes" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

fn split_list(value: Option<&str>) -> Vec<String> {
    value.map_or_else(Vec::new, |value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    })
}
