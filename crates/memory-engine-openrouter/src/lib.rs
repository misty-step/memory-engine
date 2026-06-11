//! OpenRouter-dialect HTTP draft provider.
//!
//! Speaks the OpenAI-compatible `chat/completions` dialect with
//! JSON-schema-constrained output, so one implementation covers every
//! candidate model behind `OpenRouter` (and any OpenAI-compatible endpoint via
//! `base_url`). Returns the provider boundary types from
//! `memory-engine-generation`; the provenance trust gate stays there.
//!
//! Failure messages are written for learners: transport errors, HTTP
//! rejections, and unreadable model payloads each map to one human sentence.

use std::time::{Duration, Instant};

use memory_engine_generation::{
    DraftCandidate, DraftProvider, ProviderDrafts, ProviderFailure, ProviderUsage,
};
use memory_engine_persistence::{
    GeneratedLearningActivityKind, GeneratedPromptModel, SourceDocument,
};
use serde::Deserialize;

/// Environment variable holding the `OpenRouter` API key.
pub const API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Environment variable overriding the generation model id.
pub const MODEL_ENV: &str = "MEMORY_ENGINE_GENERATION_MODEL";
/// Default model, chosen from the 2026-06-11 field run in
/// `docs/evals/generation-field-2026-06-11.md`: the quality ceiling (100%
/// provenance and answerability, zero failures across runs). It costs
/// ~$0.025/source, a deliberate trade above the original $0.02 estimate in
/// favor of output quality; `MEMORY_ENGINE_GENERATION_MODEL` overrides it,
/// and `deepseek/deepseek-v4-flash` is the documented budget alternative at
/// ~$0.0004/source.
pub const DEFAULT_MODEL: &str = "google/gemini-3.5-flash";
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_DRAFTS: usize = 8;
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Prompt strategy under evaluation; see
/// `docs/research/prose-to-quiz-generation.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptVariant {
    /// Direct task description only.
    Minimal,
    /// Adds principle-based rules distilled from Scry's production prompts.
    Principled,
}

impl PromptVariant {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimal => "prompt-minimal",
            Self::Principled => "prompt-principled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub timeout: Duration,
    pub prompt: PromptVariant,
    pub max_drafts: usize,
}

impl OpenRouterConfig {
    /// Build a config from the environment.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when `OPENROUTER_API_KEY` is unset.
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var(API_KEY_ENV)
            .map_err(|_| format!("{API_KEY_ENV} is not set; model generation is unavailable"))?;

        Ok(Self {
            api_key,
            model: std::env::var(MODEL_ENV).unwrap_or_else(|_| DEFAULT_MODEL.to_owned()),
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            prompt: PromptVariant::Principled,
            max_drafts: DEFAULT_MAX_DRAFTS,
        })
    }
}

pub struct OpenRouterProvider {
    config: OpenRouterConfig,
    agent: ureq::Agent,
}

/// One structured-output completion: the extracted JSON object text plus
/// usage accounting when the provider reported it.
#[derive(Clone, Debug)]
pub struct StructuredResponse {
    pub content: String,
    pub usage: Option<ProviderUsage>,
}

impl OpenRouterProvider {
    #[must_use]
    pub fn new(config: OpenRouterConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .build()
            .into();

        Self { config, agent }
    }

    /// Run one JSON-schema-constrained completion against the configured
    /// model and return the extracted JSON object text.
    ///
    /// This is the crate's single HTTP path; draft generation and the eval
    /// harness's model judge both go through it, so transport behavior
    /// (timeouts, body bound, failure wording) stays identical.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderFailure`] with a human-readable sentence on
    /// transport failure, HTTP rejection, or an unreadable response.
    pub fn complete_structured(
        &self,
        prompt: &str,
        schema_name: &str,
        schema: &serde_json::Value,
    ) -> Result<StructuredResponse, ProviderFailure> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let payload = serde_json::json!({
            "model": self.config.model,
            "messages": [{ "role": "user", "content": prompt }],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": schema_name,
                    "strict": true,
                    "schema": schema,
                },
            },
            "provider": { "require_parameters": true, "allow_fallbacks": true },
            "usage": { "include": true },
        });

        let started = Instant::now();
        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .send_json(&payload)
            .map_err(|error| transport_failure(&error))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Bound the response read explicitly: OpenRouter is a third party and
        // a hung-but-streaming upstream could otherwise pour data for the full
        // timeout window. Structured payloads are kilobytes; 16 MiB is ample.
        let completion: Completion = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_json()
            .map_err(|_| {
                ProviderFailure::new("The model provider's response could not be read.")
            })?;
        let content = completion
            .choices
            .first()
            .map(|choice| choice.message.content.as_str())
            .unwrap_or_default();

        Ok(StructuredResponse {
            content: extract_json_object(content).to_owned(),
            usage: completion.usage.map(|usage| ProviderUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cost_usd_micros: usage.cost.map(cost_to_micros),
                latency_ms,
            }),
        })
    }
}

impl DraftProvider for OpenRouterProvider {
    fn model(&self) -> GeneratedPromptModel {
        GeneratedPromptModel {
            provider: "openrouter".to_owned(),
            name: self.config.model.clone(),
            version: self.config.prompt.label().to_owned(),
        }
    }

    fn generate_drafts(&self, source: &SourceDocument) -> Result<ProviderDrafts, ProviderFailure> {
        let response = self.complete_structured(
            &build_prompt(self.config.prompt, self.config.max_drafts, source),
            "quiz_drafts",
            &drafts_schema(),
        )?;
        let parsed: DraftsPayload = serde_json::from_str(&response.content).map_err(|_| {
            ProviderFailure::new("The model's drafts could not be read; please try again.")
        })?;

        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        for (position, draft) in parsed
            .drafts
            .into_iter()
            .take(self.config.max_drafts)
            .enumerate()
        {
            match draft.into_candidate(position + 1) {
                Ok(candidate) => candidates.push(candidate),
                Err(reason) => {
                    failures.push(format!("{} draft {}: {reason}", source.id, position + 1));
                }
            }
        }

        Ok(ProviderDrafts {
            model: self.model(),
            candidates,
            failures,
            usage: response.usage,
        })
    }
}

/// Extract the outermost JSON object from model content.
///
/// Even under strict `json_schema` output, flash-tier models intermittently
/// wrap the object in Markdown fences or prepend reasoning prose. Slicing
/// from the first `{` to the last `}` recovers the object; if neither is
/// present the original text is returned so the caller's parse error stands.
fn extract_json_object(content: &str) -> &str {
    match (content.find('{'), content.rfind('}')) {
        (Some(start), Some(end)) if end > start => &content[start..=end],
        _ => content,
    }
}

/// Per-request generation costs are fractions of a cent; an f64→i64 cast
/// after rounding cannot truncate at any realistic magnitude.
#[allow(clippy::cast_possible_truncation)]
fn cost_to_micros(cost: f64) -> i64 {
    (cost * 1_000_000.0).round() as i64
}

fn transport_failure(error: &ureq::Error) -> ProviderFailure {
    match error {
        ureq::Error::StatusCode(code) => ProviderFailure::new(format!(
            "The model provider rejected the request (HTTP {code})."
        )),
        _ => ProviderFailure::new("The model provider could not be reached."),
    }
}

fn build_prompt(variant: PromptVariant, max_drafts: usize, source: &SourceDocument) -> String {
    let principles = match variant {
        PromptVariant::Minimal => String::new(),
        PromptVariant::Principled => "\
Principles:
- One concept per draft; never merge topics. Concept titles have at most 12 words; no vague labels like \"overview\" or \"basics\".
- Each question is standalone: it names its topic explicitly and never relies on surrounding context.
- Test the ideas a learner should retain, not punctuation, formatting, or trivia.
- Distractors are semantically adjacent confusions a real learner would make, never format variants of the answer.
- If you cannot quote verbatim support for a draft from the source text, do not emit that draft.
- Fewer, better drafts beat many shallow ones.

"
        .to_owned(),
    };

    format!(
        "You generate spaced-repetition quiz drafts from a source document.

SOURCE TITLE: {title}
SOURCE TEXT:
{body}

{principles}Generate up to {max_drafts} quiz drafts testing the most valuable facts in the source.
Each draft has:
- concept: short title naming the tested atom
- question: a standalone question answerable from the source alone
- answer: the correct answer
- evidence_quote: a quote copied verbatim from the source text that proves the answer
- distractors: 2-3 plausible wrong answers, or [] for a short-answer draft

Return JSON only.",
        title = source.title,
        body = source.body.as_deref().unwrap_or_default(),
    )
}

fn drafts_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "drafts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "concept": { "type": "string", "description": "Short title naming the tested atom." },
                        "question": { "type": "string", "description": "Standalone question answerable from the source alone." },
                        "answer": { "type": "string", "description": "The correct answer." },
                        "evidence_quote": { "type": "string", "description": "Verbatim quote from the source text proving the answer." },
                        "distractors": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "2-3 semantically adjacent wrong answers, or empty for short-answer drafts."
                        }
                    },
                    "required": ["concept", "question", "answer", "evidence_quote", "distractors"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["drafts"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
struct Completion {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct DraftsPayload {
    #[serde(default)]
    drafts: Vec<ModelDraft>,
}

#[derive(Deserialize)]
struct ModelDraft {
    #[serde(default)]
    concept: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    evidence_quote: String,
    #[serde(default)]
    distractors: Vec<String>,
}

impl ModelDraft {
    fn into_candidate(self, index: usize) -> Result<DraftCandidate, String> {
        for (field, value) in [
            ("concept", &self.concept),
            ("question", &self.question),
            ("answer", &self.answer),
            ("evidence_quote", &self.evidence_quote),
        ] {
            if value.trim().is_empty() {
                return Err(format!("the model omitted the {field}"));
            }
        }

        Ok(DraftCandidate {
            index,
            concept: self.concept,
            question: self.question,
            answer: self.answer,
            evidence: Some(self.evidence_quote),
            distractors: self
                .distractors
                .into_iter()
                .filter(|distractor| !distractor.trim().is_empty())
                .collect(),
            worked_solution: None,
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "recognition".to_owned(),
            unsupported: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::extract_json_object;

    #[test]
    fn unwraps_markdown_fenced_json() {
        let fenced = "```json\n{\"drafts\": []}\n```";
        assert_eq!(extract_json_object(fenced), "{\"drafts\": []}");
    }

    #[test]
    fn strips_leading_reasoning_prose() {
        let prefixed = "Here are the drafts:\n{\"drafts\": [{\"a\": 1}]}";
        assert_eq!(extract_json_object(prefixed), "{\"drafts\": [{\"a\": 1}]}");
    }

    #[test]
    fn passes_bare_object_through() {
        assert_eq!(extract_json_object("{\"drafts\": []}"), "{\"drafts\": []}");
    }

    #[test]
    fn returns_original_when_no_object_present() {
        assert_eq!(extract_json_object("not json"), "not json");
    }
}
