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

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use memory_engine_generation::{
    enforce_content_policy, BridgeMaterial, BridgeMaterialProvider, BridgeMaterialRequest,
    DraftCandidate, DraftProvider, DraftRejection, LearningIntent, ProviderDrafts, ProviderFailure,
    ProviderUsage, ReferenceNoteDraft, ReferenceNoteProvider, ReferenceNoteRequest,
};
use memory_engine_persistence::{
    GeneratedLearningActivityKind, GeneratedPromptModel, SourceDocument, SourcePermission,
};
use serde::Deserialize;

/// Environment variable holding the `OpenRouter` API key.
pub const API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// One-run bearer capability for the trusted hosted-eval provider proxy.
pub const PROXY_TOKEN_ENV: &str = "OPENROUTER_PROXY_TOKEN";
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
/// Trusted hosted evaluation may replace the upstream with a local provider
/// proxy. The proxy owns the real key; target code receives only a one-run
/// capability token as `OPENROUTER_PROXY_TOKEN`.
pub const BASE_URL_ENV: &str = "OPENROUTER_BASE_URL";
/// Unix socket for the trusted hosted-eval proxy. The target gets a bounded
/// capability token, never the provider key or general network access.
pub const PROXY_SOCKET_ENV: &str = "OPENROUTER_PROXY_SOCKET";
const DEFAULT_TIMEOUT: Duration = Duration::from_mins(1);
/// Per-generation card ceiling. High enough that a finite enumerable set (an
/// alphabet, the 50 US states) is covered completely; the prompt restrains
/// open-ended material to a few high-value cards, so this is a ceiling, not a
/// target.
const DEFAULT_MAX_DRAFTS: usize = 60;
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// Total tries for one model call: the first attempt plus a single retry, taken
/// only when the failure is a transient transport error.
const MAX_REQUEST_ATTEMPTS: u32 = 2;
/// Pause before the lone retry, so a brief provider blip has a moment to clear
/// and we don't instantly re-hit a rate limit.
const RETRY_BACKOFF: Duration = Duration::from_millis(250);

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
    pub proxy_socket: Option<PathBuf>,
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
        let api_key = std::env::var(PROXY_TOKEN_ENV)
            .or_else(|_| std::env::var(API_KEY_ENV))
            .map_err(|_| format!("{API_KEY_ENV} is not set; model generation is unavailable"))?;

        Ok(Self {
            api_key,
            model: std::env::var(MODEL_ENV).unwrap_or_else(|_| DEFAULT_MODEL.to_owned()),
            base_url: std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned()),
            proxy_socket: std::env::var_os(PROXY_SOCKET_ENV).map(PathBuf::from),
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

        // One model call, retried once on a transient transport failure (the
        // provider unreachable or returning 5xx/429) so a single blip doesn't
        // surface as a failed generation. Permanent failures (4xx, malformed
        // body) return on the first attempt. This is per call: a generation that
        // also runs the repair pass can issue two retried calls, so worst-case
        // model spend per source doubles under sustained transient failure.
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = if let Some(proxy_socket) = &self.config.proxy_socket {
                #[cfg(unix)]
                {
                    self.attempt_proxy_structured(proxy_socket, &payload)
                }
                #[cfg(not(unix))]
                {
                    let _ = proxy_socket;
                    Err(ProviderFailure::new(
                        "The trusted provider proxy is unavailable on this platform.",
                    ))
                }
            } else {
                let url = format!("{}/chat/completions", self.config.base_url);
                self.attempt_structured(&url, &payload)
            };
            match result {
                Ok(response) => return Ok(response),
                Err(failure) if failure.is_transient() && attempt < MAX_REQUEST_ATTEMPTS => {
                    std::thread::sleep(RETRY_BACKOFF);
                }
                Err(failure) => return Err(failure),
            }
        }
    }

    /// One attempt at the structured completion: send the request, read the
    /// bounded response, and shape it. [`Self::complete_structured`] owns the
    /// retry policy around this.
    fn attempt_structured(
        &self,
        url: &str,
        payload: &serde_json::Value,
    ) -> Result<StructuredResponse, ProviderFailure> {
        let started = Instant::now();
        let mut response = self
            .agent
            .post(url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .send_json(payload)
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

    #[cfg(unix)]
    fn attempt_proxy_structured(
        &self,
        socket: &PathBuf,
        payload: &serde_json::Value,
    ) -> Result<StructuredResponse, ProviderFailure> {
        let started = Instant::now();
        let mut stream = UnixStream::connect(socket).map_err(|_| {
            ProviderFailure::transient("The trusted provider proxy could not be reached.")
        })?;
        let request = serde_json::json!({ "token": self.config.api_key, "payload": payload });
        let mut encoded = serde_json::to_vec(&request).map_err(|_| {
            ProviderFailure::new("The trusted provider request could not be encoded.")
        })?;
        encoded.push(b'\n');
        stream.write_all(&encoded).map_err(|_| {
            ProviderFailure::transient("The trusted provider proxy could not be reached.")
        })?;
        let mut line = String::new();
        BufReader::new(stream)
            .take(MAX_RESPONSE_BYTES + 1)
            .read_line(&mut line)
            .map_err(|_| {
                ProviderFailure::transient("The trusted provider proxy response could not be read.")
            })?;
        if line.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(ProviderFailure::new(
                "The trusted provider proxy response was too large.",
            ));
        }
        let response: ProxyResponse = serde_json::from_str(&line).map_err(|_| {
            ProviderFailure::new("The trusted provider proxy response was invalid.")
        })?;
        if !(200..300).contains(&response.status) {
            let failure = ProviderFailure::new(format!(
                "The model provider rejected the request (HTTP {}).",
                response.status
            ));
            return if response.status >= 500 || response.status == 429 {
                Err(ProviderFailure::transient(failure.to_string()))
            } else {
                Err(failure)
            };
        }
        let completion: Completion = serde_json::from_str(&response.body).map_err(|_| {
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
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }),
        })
    }
}

#[derive(Debug, Deserialize)]
struct ProxyResponse {
    status: u16,
    body: String,
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
        ensure_model_eligible(source)?;
        let response = self.complete_structured(
            &build_prompt(self.config.prompt, self.config.max_drafts, source),
            "quiz_drafts",
            &drafts_schema(),
        )?;
        parse_drafts_response(
            response,
            source,
            DraftProvider::model(self),
            self.config.max_drafts,
        )
    }

    fn repair_drafts(
        &self,
        source: &SourceDocument,
        rejections: &[DraftRejection],
    ) -> Result<Option<ProviderDrafts>, ProviderFailure> {
        ensure_model_eligible(source)?;
        if rejections.is_empty() {
            return Ok(None);
        }

        let response = self.complete_structured(
            &build_repair_prompt(
                self.config.prompt,
                self.config.max_drafts,
                source,
                rejections,
            ),
            "quiz_draft_repair",
            &drafts_schema(),
        )?;
        parse_drafts_response(
            response,
            source,
            DraftProvider::model(self),
            self.config.max_drafts,
        )
        .map(Some)
    }
}

fn ensure_model_eligible(source: &SourceDocument) -> Result<(), ProviderFailure> {
    if source.archived_at.is_some() {
        Err(ProviderFailure::archived_source(source.id.clone()))
    } else if source.permission == SourcePermission::LocalOnly {
        Err(ProviderFailure::local_only_source(source.id.clone()))
    } else {
        Ok(())
    }
}

fn ensure_authorized_request(
    authorization: &memory_engine_generation::SourceAuthorizationContext,
) -> Result<(), ProviderFailure> {
    if let Some(source_id) = authorization.local_only_source_id() {
        Err(ProviderFailure::local_only_source(source_id.to_owned()))
    } else {
        Ok(())
    }
}

fn parse_drafts_response(
    response: StructuredResponse,
    source: &SourceDocument,
    model: GeneratedPromptModel,
    max_drafts: usize,
) -> Result<ProviderDrafts, ProviderFailure> {
    let parsed: DraftsPayload = serde_json::from_str(&response.content).map_err(|_| {
        ProviderFailure::new("The model's drafts could not be read; please try again.")
    })?;
    let learning_intent = LearningIntent::from_label(&parsed.learning_intent).ok_or_else(|| {
        ProviderFailure::new("The model's learning intent could not be read; please try again.")
    })?;

    let mut candidates = Vec::new();
    let mut failures = Vec::new();
    for (position, draft) in parsed.drafts.into_iter().take(max_drafts).enumerate() {
        match draft.into_candidate(position + 1) {
            Ok(candidate) => candidates.push(candidate),
            Err(reason) => {
                failures.push(format!("{} draft {}: {reason}", source.id, position + 1));
            }
        }
    }

    Ok(enforce_content_policy(
        source,
        ProviderDrafts {
            model,
            learning_intent: Some(learning_intent),
            candidates,
            failures,
            usage: response.usage,
        },
    ))
}

impl ReferenceNoteProvider for OpenRouterProvider {
    fn model(&self) -> GeneratedPromptModel {
        DraftProvider::model(self)
    }

    fn explain_concept(
        &self,
        request: &ReferenceNoteRequest,
    ) -> Result<ReferenceNoteDraft, ProviderFailure> {
        ensure_authorized_request(request.authorization())?;
        let response = self.complete_structured(
            &build_reference_note_prompt(request),
            "reference_note",
            &reference_note_schema(),
        )?;
        let parsed: ReferenceNotePayload =
            serde_json::from_str(&response.content).map_err(|_| {
                ProviderFailure::new(
                    "The model's reference note could not be read; please try again.",
                )
            })?;
        parsed.into_note()
    }
}

impl BridgeMaterialProvider for OpenRouterProvider {
    fn generate_bridge_material(
        &self,
        request: &BridgeMaterialRequest,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        ensure_authorized_request(request.authorization())?;
        let response = self.complete_structured(
            &build_bridge_prompt(request),
            "bridge_material",
            &bridge_material_schema(),
        )?;
        let parsed: BridgeMaterialPayload =
            serde_json::from_str(&response.content).map_err(|_| {
                ProviderFailure::new(
                    "The model's bridge material could not be read; please try again.",
                )
            })?;
        parsed.into_bridge_material(DraftProvider::model(self), response.usage)
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
        // A 5xx or 429 is the provider faltering, not our request — worth one
        // retry. A 4xx (bad key, bad request) is permanent: retrying won't help.
        ureq::Error::StatusCode(code) => {
            let message = format!("The model provider rejected the request (HTTP {code}).");
            if *code >= 500 || *code == 429 {
                ProviderFailure::transient(message)
            } else {
                ProviderFailure::new(message)
            }
        }
        // Connection refused, DNS, TLS, timeout — never reached the provider, so
        // an identical retry may land.
        _ => ProviderFailure::transient("The model provider could not be reached."),
    }
}

/// The unified generation prompt. The input may be a topic to expand from world
/// knowledge, a passage to extract and quote from, or a mix; the model decides
/// learning intent, coverage, and — per card — grounding (quote the source when
/// the answer is in it, otherwise leave the quote empty). No deterministic
/// pre-classifier picks a mode: the model judges it, and the trust gate verifies
/// every quote a card claims.
fn build_prompt(variant: PromptVariant, max_drafts: usize, source: &SourceDocument) -> String {
    let principles = match variant {
        PromptVariant::Minimal => String::new(),
        PromptVariant::Principled => "\
Principles:
- One concept per card; never merge topics. Concept titles have at most 12 words; no vague labels like \"overview\" or \"basics\".
- Each question is standalone: it names its subject explicitly and never relies on the source, this prompt, or surrounding context.
- Test one durable atom a learner should retain, not punctuation, formatting, or trivia. Single-part: never ask for two facts, a list, or a causal chain in one card.
- Distractors are semantically adjacent confusions a real learner would make, matching the answer's category and granularity; never format variants of the answer, another true statement, an overlapping numeric range, a joke, an anachronism, or an impossible option.
- Distractors must share whatever surface feature the question keys on, so the answer is never identifiable by that feature alone. If the question fixes the letter C (\"which code word for the letter C?\"), EVERY option must start with C — answer Charlie, distractors like Cobra, Caesar, Casino — never Delta or Echo, which a learner eliminates without knowing the answer. The plausible same-feature options may come from your own knowledge, not only the source set.
- If you cannot write 2-3 distractors that share the answer's keyed feature, make the card short-answer with [] distractors rather than ship guessable options.
- Never invent or paraphrase a source quote. Fewer, better cards beat many shallow ones.

"
        .to_owned(),
    };

    format!(
        "You turn a learner's study input into spaced-repetition quiz cards. The input may be a TOPIC to expand from your own knowledge (for example \"NATO phonetic alphabet\"), a PASSAGE to extract and quote from, or a mix of both.

SOURCE TITLE: {title}
SOURCE TEXT:
{body}

{principles}First classify the input's learning_intent as exactly one of:
- verbatim_memorization: a specific text to reproduce exactly (a poem, an oath, a quote).
- enumerable_set: a finite set of independently recallable entries or mappings.
- concept_understanding: a mechanism, theory, cause, or idea to understand and apply.
- fact_recall: discrete facts, names, dates, definitions, or mappings.
- procedure_process: ordered steps, a workflow, a recipe, or commands.

Then generate cards, never exceeding {max_drafts}:
- Coverage: if the input names a finite, enumerable set (an alphabet, the planets, the months, a fixed list), write ONE card for EVERY element, in order — cover the whole set, never collapse it into a single card. Otherwise write the highest-value cards a learner should master first, one atomic fact or idea each, and stop early rather than pad with weak cards.
- For verbatim_memorization, write one exact recitation card for every source line or sentence in order; use the previous unit as the cue for the next unit.

Decide grounding for EACH card:
- If the answer is contained in the SOURCE TEXT above, set evidence_quote to the exact verbatim span from it that proves the answer. Never invent or paraphrase a quote.
- If the card comes from your own knowledge of the topic and the SOURCE TEXT does not state it, set evidence_quote to \"\".

Each card has:
- concept: short title naming the tested atom, at most 12 words (for example \"NATO alphabet: B\").
- question: a standalone question. The learner sees only the question, so name the subject inside it. Write \"In the NATO phonetic alphabet, which word stands for the letter B?\" — never \"What is the second item?\" or \"the subject of the text\". Never mention \"the source\", \"the passage\", \"the list above\", or \"the text\".
- answer: the exact correct answer, as short as it can correctly be.
- evidence_quote: a verbatim span copied from the SOURCE TEXT, or \"\" for a world-knowledge card (see grounding above).
- distractors: 2-3 plausible same-category wrong answers, or [] for a short-answer card.
- activity_kind: quiz or exercise.
- activity_stage: recognition, cued-recall, free-recall, or procedure-composition.
- worked_solution: required for exercises, otherwise \"\".

Return JSON only.",
        title = source.title,
        body = source.body.as_deref().unwrap_or_default(),
    )
}

fn build_repair_prompt(
    variant: PromptVariant,
    max_drafts: usize,
    source: &SourceDocument,
    rejections: &[DraftRejection],
) -> String {
    let rejected = rejections
        .iter()
        .map(|rejection| {
            format!(
                "- draft {} concept {:?}: question {:?}; answer {:?}; rejected because {}",
                rejection.index,
                rejection.concept,
                rejection.question,
                rejection.answer,
                rejection.reasons.join("; ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{base}

Repair pass:
- The drafts below were rejected by the trust gate. Generate fresh replacements only.
- Do not repeat rejected question wording or duplicate an accepted concept/question/answer surface.
- Fix every listed rejection reason; if you cannot fix one with source-grounded evidence, omit it.
- Return at most {repair_limit} repaired drafts.

REJECTED DRAFTS:
{rejected}",
        base = build_prompt(variant, max_drafts, source),
        repair_limit = rejections.len().min(max_drafts),
    )
}

fn build_reference_note_prompt(request: &ReferenceNoteRequest) -> String {
    format!(
        "Write a short reference note for a learner who is stuck on one review item.

CONCEPT KEY: {concept_key}
CONCEPT LABEL: {concept_label}
PROMPT: {prompt}
EXPECTED ANSWER: {answer}

Rules:
- Explain the underlying concept, not the UI.
- Keep it under 120 words.
- Do not reveal unrelated facts.
- Return JSON only.",
        concept_key = request.concept_key,
        concept_label = request.concept_label,
        prompt = request.prompt,
        answer = request.expected_answer,
    )
}

fn build_bridge_prompt(request: &BridgeMaterialRequest) -> String {
    let cached_note = request.cached_reference_note.as_deref().unwrap_or("");
    let recent_performance = bridge_performance_context(request);
    format!(
        "Generate bridge material for a learner struggling with a review item.

CONCEPT KEY: {concept_key}
CONCEPT LABEL: {concept_label}
PARENT STAGE ORDER: {stage_order}
PARENT PROMPT: {prompt}
PARENT EXPECTED ANSWER: {answer}
RECENT PERFORMANCE:
{recent_performance}
CACHED REFERENCE NOTE:
{cached_note}

Rules:
- Write or reuse a short reference note for the concept.
- Generate exactly 2 easier drafts than the parent.
- Draft 1 must be a recognition quiz with activity_stage recognition-bridge.
- Draft 2 must be a cued-recall exercise with activity_stage cued-recall-bridge and a worked_solution.
- Every draft must test the same concept and avoid duplicating the parent prompt.
- Return JSON only.",
        concept_key = request.concept_key,
        concept_label = request.concept_label,
        stage_order = request.parent_stage_order,
        prompt = request.parent_prompt,
        answer = request.parent_expected_answer,
        recent_performance = recent_performance,
    )
}

fn bridge_performance_context(request: &BridgeMaterialRequest) -> String {
    if request.recent_performance.is_empty() {
        return "No recent attempts recorded for this concept.".to_owned();
    }

    request
        .recent_performance
        .iter()
        .map(|attempt| {
            format!(
                "- {}: answer {:?}, verdict {}",
                attempt.review_unit_id,
                attempt.submitted_answer,
                attempt.verdict.as_deref().unwrap_or("ungraded")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn drafts_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "learning_intent": {
                "type": "string",
            "enum": ["verbatim_memorization", "enumerable_set", "concept_understanding", "fact_recall", "procedure_process"],
                "description": "The classified learning goal for this source."
            },
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
                            "description": "2-3 same-category plausible wrong answers, or empty for short-answer drafts."
                        },
                        "activity_kind": {
                            "type": "string",
                            "enum": ["quiz", "exercise"],
                            "description": "quiz for recognition/recall checks; exercise for recitation-ladder items."
                        },
                        "activity_stage": {
                            "type": "string",
                            "description": "recognition, cued-recall, free-recall, or procedure-composition."
                        },
                        "worked_solution": {
                            "type": "string",
                            "description": "Required human-readable solution for exercises; empty string for quizzes."
                        }
                    },
                    "required": ["concept", "question", "answer", "evidence_quote", "distractors", "activity_kind", "activity_stage", "worked_solution"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["learning_intent", "drafts"],
        "additionalProperties": false
    })
}

fn reference_note_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "body": { "type": "string" }
        },
        "required": ["title", "body"],
        "additionalProperties": false
    })
}

fn bridge_material_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "reference_note": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["title", "body"],
                "additionalProperties": false
            },
            "drafts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "concept": { "type": "string" },
                        "question": { "type": "string" },
                        "answer": { "type": "string" },
                        "distractors": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "activity_kind": {
                            "type": "string",
                            "enum": ["quiz", "exercise"]
                        },
                        "activity_stage": {
                            "type": "string",
                            "description": "Use recognition-bridge for the first quiz and cued-recall-bridge for the second exercise."
                        },
                        "worked_solution": { "type": "string" }
                    },
                    "required": ["concept", "question", "answer", "distractors", "activity_kind", "activity_stage", "worked_solution"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["reference_note", "drafts"],
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
    learning_intent: String,
    #[serde(default)]
    drafts: Vec<ModelDraft>,
}

#[derive(Deserialize)]
struct ReferenceNotePayload {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
}

impl ReferenceNotePayload {
    fn into_note(self) -> Result<ReferenceNoteDraft, ProviderFailure> {
        if self.title.trim().is_empty() || self.body.trim().is_empty() {
            return Err(ProviderFailure::new(
                "The model's reference note was incomplete; please try again.",
            ));
        }

        Ok(ReferenceNoteDraft {
            title: self.title,
            body: self.body,
        })
    }
}

#[derive(Deserialize)]
struct BridgeMaterialPayload {
    reference_note: ReferenceNotePayload,
    #[serde(default)]
    drafts: Vec<ModelBridgeDraft>,
}

impl BridgeMaterialPayload {
    fn into_bridge_material(
        self,
        model: GeneratedPromptModel,
        usage: Option<ProviderUsage>,
    ) -> Result<BridgeMaterial, ProviderFailure> {
        let mut candidates = Vec::new();
        for (position, draft) in self.drafts.into_iter().enumerate() {
            candidates.push(draft.into_candidate(position + 1)?);
        }
        if candidates.is_empty() {
            return Err(ProviderFailure::new(
                "The model did not produce bridge items; please try again.",
            ));
        }

        Ok(BridgeMaterial {
            model,
            reference_note: self.reference_note.into_note()?,
            candidates,
            usage,
        })
    }
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
    #[serde(default)]
    activity_kind: String,
    #[serde(default)]
    activity_stage: String,
    #[serde(default)]
    worked_solution: String,
}

#[derive(Deserialize)]
struct ModelBridgeDraft {
    #[serde(default)]
    concept: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    distractors: Vec<String>,
    #[serde(default)]
    activity_kind: String,
    #[serde(default)]
    activity_stage: String,
    #[serde(default)]
    worked_solution: String,
}

impl ModelBridgeDraft {
    fn into_candidate(self, index: usize) -> Result<DraftCandidate, ProviderFailure> {
        for (field, value) in [
            ("concept", &self.concept),
            ("question", &self.question),
            ("answer", &self.answer),
            ("activity_kind", &self.activity_kind),
            ("activity_stage", &self.activity_stage),
        ] {
            if value.trim().is_empty() {
                return Err(ProviderFailure::new(format!(
                    "The model omitted {field} from a bridge item; please try again."
                )));
            }
        }
        let activity_kind = parse_activity_kind(&self.activity_kind)
            .map_err(|reason| ProviderFailure::new(format!("{reason}; please try again.")))?;
        let activity_stage = parse_bridge_stage(&self.activity_stage)
            .map_err(|reason| ProviderFailure::new(format!("{reason}; please try again.")))?;

        Ok(DraftCandidate {
            index,
            concept: self.concept,
            question: self.question,
            answer: self.answer,
            evidence: None,
            distractors: self.distractors,
            worked_solution: non_empty(&self.worked_solution),
            activity_kind,
            activity_stage,
            unsupported: false,
        })
    }
}

fn parse_bridge_stage(stage: &str) -> Result<String, String> {
    let normalized = stage.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "recognition-bridge" | "recognition bridge" => Ok("recognition-bridge".to_owned()),
        "cued-recall-bridge" | "cued recall bridge" => Ok("cued-recall-bridge".to_owned()),
        other => Err(format!(
            "bridge activity_stage must be recognition-bridge or cued-recall-bridge, got {other}"
        )),
    }
}

impl ModelDraft {
    fn into_candidate(self, index: usize) -> Result<DraftCandidate, String> {
        // evidence_quote is intentionally optional: a world-knowledge card
        // leaves it empty, and the generation trust gate decides grounding per
        // card from its presence (and verifies any quote that is present).
        for (field, value) in [
            ("concept", &self.concept),
            ("question", &self.question),
            ("answer", &self.answer),
            ("activity_kind", &self.activity_kind),
            ("activity_stage", &self.activity_stage),
        ] {
            if value.trim().is_empty() {
                return Err(format!("the model omitted the {field}"));
            }
        }
        let activity_kind = parse_activity_kind(&self.activity_kind)?;

        Ok(DraftCandidate {
            index,
            concept: self.concept,
            question: self.question,
            answer: self.answer,
            evidence: non_empty(&self.evidence_quote),
            distractors: self
                .distractors
                .into_iter()
                .filter(|distractor| !distractor.trim().is_empty())
                .collect(),
            worked_solution: non_empty(&self.worked_solution),
            activity_kind,
            activity_stage: self.activity_stage,
            unsupported: false,
        })
    }
}

fn parse_activity_kind(value: &str) -> Result<GeneratedLearningActivityKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "quiz" => Ok(GeneratedLearningActivityKind::Quiz),
        "exercise" => Ok(GeneratedLearningActivityKind::Exercise),
        other => Err(format!("unknown activity_kind {other}")),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_json_object, parse_bridge_stage};

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

    #[test]
    fn parses_only_explicit_bridge_stage_rungs() {
        assert_eq!(
            parse_bridge_stage("recognition-bridge").expect("recognition"),
            "recognition-bridge"
        );
        assert_eq!(
            parse_bridge_stage("cued recall bridge").expect("cued"),
            "cued-recall-bridge"
        );
        assert!(parse_bridge_stage("0.3").is_err());
        assert!(parse_bridge_stage("composition").is_err());
    }
}
