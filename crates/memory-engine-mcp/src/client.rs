//! HTTP client over the deployed `memory-engine-api` v1 contract.
//!
//! A Bearer-token `ureq` client against the same v1 routes
//! `memory-engine-review`'s `ReviewClient` and `memory-engine-contract`'s
//! `ContractClient` use, kept deliberately thin. It adds no new server
//! surface — every method below maps to one existing v1 route. Generation
//! goes through the durable job queue (`POST .../generation-jobs`, `GET
//! .../generation-jobs/{id}`) exclusively: the legacy synchronous `POST
//! .../generate` route is refused outright once a deployment has
//! `MEMORY_ENGINE_POSTGRES_URL` set (`ApiFailure::conflict`, HTTP 409 —
//! `registry.rs::generate_source`), which is every production deployment.

use std::{thread, time::Duration};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded poll for a queued generation job: 40 attempts at 500ms is a 20s
/// ceiling, comfortably above the ~1s-per-poll the production receipt
/// (`docs/qa/103-machine-generation-receipt-2026-07-17.md`) observed for a
/// one-card source, while still failing loudly instead of hanging an agent
/// call forever on a stuck job.
const GENERATION_POLL_MAX_ATTEMPTS: u32 = 40;
const GENERATION_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreated {
    pub account_id: String,
    pub session_token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub source_id: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub project_key: Option<String>,
    #[serde(default)]
    pub ttl_expires_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceList {
    pub sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeckRecord {
    pub deck_id: String,
    pub project_key: String,
    pub source: SourceRecord,
}

/// A queued generation job (`GenerationJob` in the `OpenAPI` contract).
/// `status` is one of `queued`, `running`, `retry`, `succeeded`, `failed` —
/// kept as a plain `String` rather than a closed enum so an added status
/// value degrades to a visible string instead of a deserialize failure that
/// would take down the whole poll.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationJob {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub status: String,
    pub card_count: usize,
    pub attempts: u32,
    pub retryable: bool,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl GenerationJob {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "succeeded" | "failed")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueuedGenerationJob {
    #[serde(flatten)]
    pub job: GenerationJob,
    /// `true` when this call joined a job already in flight for the same
    /// source rather than starting a new one.
    pub coalesced: bool,
}

/// The outcome of `create_deck`'s enqueue-then-poll composition: exactly one
/// declared shape per bounded-poll result, so a caller can match
/// exhaustively instead of guessing which fields are populated.
#[derive(Clone, Debug)]
pub enum GenerationOutcome {
    /// The job reached `succeeded`. `drafts` is every currently pending
    /// (accepted, unapproved) draft account-wide — usually empty, because
    /// the production job runner optimistically approves every accepted
    /// draft as part of running the job, so a normal `succeeded` job has
    /// nothing left to approve. A non-empty list means some accepted draft
    /// exists outside that path (a partially-committed job, or a
    /// legacy-origin draft) and is a genuine "still needs a decision" signal
    /// worth surfacing. A source whose material yielded no usable cards
    /// (`job.card_count == 0`) still lands here: zero cards is a valid
    /// terminal outcome, not an error.
    Succeeded {
        job: GenerationJob,
        coalesced: bool,
        drafts: Vec<DraftRow>,
    },
    /// The job reached `failed`. `job.retryable` tells the caller whether
    /// calling `create_deck` again for the same material has a chance of
    /// succeeding (the v1 contract has no dedicated retry route; enqueueing
    /// again is the supported retry path once the prior job is no longer
    /// active).
    Failed { job: GenerationJob, coalesced: bool },
    /// The job did not reach `succeeded` or `failed` within the bounded
    /// poll window. The job keeps running server-side; poll
    /// `generation_job(job.id)` later rather than assuming it died.
    TimedOut { job: GenerationJob, coalesced: bool },
}

/// One generated draft pending a learner-authority decision (`StudyDraft` in
/// the `OpenAPI` contract). `approved` distinguishes "already kept" from
/// "still pending"; there is no `rejected` flag — a draft an agent never
/// approves stays pending, unscheduled, forever (implicit rejection by
/// omission, not a mutation this contract has a dedicated route for).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRow {
    pub id: String,
    pub review_unit_id: String,
    pub activity_kind: String,
    pub activity_stage: String,
    pub prompt: String,
    pub validation_status: String,
    #[serde(default)]
    pub validation_reasons: Vec<String>,
    pub worked_solution: Option<String>,
    pub approved: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyView {
    #[serde(default)]
    pub drafts: Vec<DraftRow>,
    pub current: Option<StudyCurrent>,
    #[serde(default)]
    pub concept_progress: Vec<ConceptProgress>,
    #[serde(default)]
    pub summary: StudySummary,
    pub due_count: usize,
    #[serde(default)]
    pub generation_notices: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyCurrent {
    pub review_unit_id: String,
    pub concept_key: Option<String>,
    #[serde(default)]
    pub prompt_id: String,
    #[serde(default)]
    pub activity_kind: String,
    #[serde(default)]
    pub activity_stage: String,
    pub prompt: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub revision_expected_answer: String,
    pub expected_answer: Option<String>,
    #[serde(default)]
    pub reference_text: Option<String>,
    #[serde(default)]
    pub worked_solution: Option<String>,
    pub grade: Option<StudyGrade>,
    #[serde(default)]
    pub review_state: Option<ReviewState>,
    #[serde(default)]
    pub schedule_change: Option<ScheduleChange>,
    #[serde(default)]
    pub feedback: Option<StudyFeedback>,
    #[serde(default)]
    pub content_feedback_head_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyGrade {
    pub verdict: String,
    pub rating: u8,
    pub is_correct: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewState {
    pub due: i64,
    pub reps: u32,
    pub lapses: u32,
    pub state: u8,
    pub last_review: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleChange {
    pub before: Option<ReviewState>,
    pub after: ReviewState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyFeedback {
    pub verdict: String,
    pub expected_answer: String,
    pub item_history: StudyItemHistory,
    pub concept_progress: Option<ConceptProgress>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyItemHistory {
    pub attempts: u32,
    pub correct: u32,
    pub success_rate: String,
    pub trend: String,
    pub last_seen: Option<i64>,
    pub last_seen_summary: String,
    pub last_response_time_ms: Option<u32>,
    pub average_response_time_ms: Option<u32>,
    pub response_time_trend: String,
    pub stage: String,
    pub next_review: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptProgress {
    pub concept_key: String,
    pub concept_label: String,
    pub attempts: u32,
    pub correct: u32,
    pub success_rate: String,
    pub trend: String,
    pub average_response_time_ms: Option<u32>,
    pub response_time_trend: String,
    pub health: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySummary {
    #[serde(default)]
    pub source_count: usize,
    #[serde(default)]
    pub accepted_draft_count: usize,
    #[serde(default)]
    pub approved_review_unit_count: usize,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub last_outcome: Option<String>,
    #[serde(default)]
    pub next_review_unit_id: Option<String>,
}

/// A recorded content-feedback verdict (`kept`/`dropped`) on one review
/// unit — the "declare this generated card good or bad" signal, distinct
/// from grading an answer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentFeedback {
    pub id: String,
    pub review_unit_id: String,
    pub verdict: String,
    pub rationale: Option<String>,
    pub source: String,
    pub account_id: String,
    pub occurred_at: i64,
    pub supersedes_id: Option<String>,
}

/// Thin Bearer-token client over the v1 API, scoped to one account.
pub struct MemoryEngineClient {
    agent: ureq::Agent,
    base_url: String,
    account_id: String,
    session_token: String,
}

impl MemoryEngineClient {
    #[must_use]
    pub fn new(base_url: String, account_id: String, session_token: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            // Read the response body ourselves on 4xx/5xx instead of losing
            // it to a bare `StatusCode(u16)` error: the API's `ApiError`
            // body (`{"error": "..."}`) is the safe, agent-actionable
            // message ("Generation queue is full for this account...",
            // "Direct synchronous generation is disabled in production...")
            // that a raw status code discards.
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            base_url,
            account_id,
            session_token,
        }
    }

    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Save a project-scoped deck, enqueue its generation job on the durable
    /// queue, and poll to a bounded terminal state. Never calls the legacy
    /// synchronous generate route (refused with HTTP 409 in every production
    /// deployment). A `succeeded` job's accepted cards arrive already
    /// scheduled: the production job runner optimistically approves every
    /// accepted draft as part of the job itself
    /// (`registry.rs::run_generation_job`, unchanged by this client — that
    /// policy is shared with every other caller, including the browser, and
    /// is out of this ticket's scope). `approve_draft`/`list_drafts` exist
    /// for drafts that reach an unapproved state some other way (a
    /// partially-committed job, or a legacy-origin draft) — the caller
    /// should not expect this call's own drafts to need a follow-up
    /// approval in the common case.
    ///
    /// # Errors
    ///
    /// Returns an error when saving the deck fails, when the job cannot be
    /// enqueued (e.g. HTTP 409 "queue is full" / budget exhausted), or when
    /// polling the job's status fails.
    pub fn create_deck(
        &self,
        project_key: &str,
        title: &str,
        body: &str,
        ttl_expires_at: Option<i64>,
    ) -> Result<(ProjectDeckRecord, GenerationOutcome), String> {
        let mut request = json!({
            "projectKey": project_key,
            "title": title,
            "body": body,
        });
        if let Some(ttl) = ttl_expires_at {
            request["ttlExpiresAt"] = json!(ttl);
        }
        let deck: ProjectDeckRecord = self.post_json(
            &format!("/v1/accounts/{}/project-decks", self.account_id),
            &request,
        )?;

        let enqueued = self.enqueue_generation_job(&deck.source.source_id)?;
        let outcome = self.poll_generation_job(enqueued.job, enqueued.coalesced)?;

        Ok((deck, outcome))
    }

    /// Enqueue (or join an already-in-flight) generation job for a saved
    /// source. Composes `POST .../generation-jobs` — the durable queue every
    /// production deployment requires.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails, including the declared
    /// rejection/unavailable messages the queue returns (budget exhausted,
    /// queue full, generation temporarily disabled).
    pub fn enqueue_generation_job(&self, source_id: &str) -> Result<EnqueuedGenerationJob, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/sources/{source_id}/generation-jobs",
            self.account_id
        ))
    }

    /// Fetch one generation job's current status.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails (including "not found" for
    /// an unknown or cross-account job id).
    pub fn generation_job(&self, job_id: &str) -> Result<GenerationJob, String> {
        self.get(&format!(
            "/v1/accounts/{}/generation-jobs/{job_id}",
            self.account_id
        ))
    }

    /// Poll one job to a bounded terminal state (`succeeded`/`failed`), or
    /// declare `TimedOut` after `GENERATION_POLL_MAX_ATTEMPTS`. On success,
    /// fetches the pending (accepted, unapproved) drafts so the caller can
    /// decide what to approve — never approves anything itself.
    fn poll_generation_job(
        &self,
        mut job: GenerationJob,
        coalesced: bool,
    ) -> Result<GenerationOutcome, String> {
        for attempt in 0..GENERATION_POLL_MAX_ATTEMPTS {
            if job.is_terminal() {
                break;
            }
            if attempt + 1 == GENERATION_POLL_MAX_ATTEMPTS {
                return Ok(GenerationOutcome::TimedOut { job, coalesced });
            }
            thread::sleep(GENERATION_POLL_INTERVAL);
            job = self.generation_job(&job.id)?;
        }

        match job.status.as_str() {
            "succeeded" => {
                let drafts = self.pending_drafts()?;
                Ok(GenerationOutcome::Succeeded {
                    job,
                    coalesced,
                    drafts,
                })
            }
            "failed" => Ok(GenerationOutcome::Failed { job, coalesced }),
            _ => Ok(GenerationOutcome::TimedOut { job, coalesced }),
        }
    }

    /// Every currently pending (validator-accepted, not yet approved) draft
    /// across the account — the "inspect before you keep" read.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying study-view request fails.
    pub fn pending_drafts(&self) -> Result<Vec<DraftRow>, String> {
        let view = self.next_review()?;
        Ok(view
            .drafts
            .into_iter()
            .filter(|draft| draft.validation_status == "accepted" && !draft.approved)
            .collect())
    }

    /// Explicitly keep one generated draft: schedules it for review. This is
    /// the only mutation that turns a draft into a live review unit, so it
    /// is never called except in direct response to a caller's own
    /// `approve_draft` tool call — no composition calls it implicitly.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails (including approving an
    /// already-approved or unknown draft id).
    pub fn approve_draft(&self, draft_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/drafts/{draft_id}/approve",
            self.account_id
        ))
    }

    /// List saved sources that belong to a project deck (`project_key` set),
    /// optionally filtered to one `project_key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the list request fails.
    pub fn list_decks(&self, project_key: Option<&str>) -> Result<Vec<SourceRecord>, String> {
        let list: SourceList = self.get(&format!("/v1/accounts/{}/sources", self.account_id))?;
        Ok(list
            .sources
            .into_iter()
            .filter(|source| {
                source.project_key.is_some()
                    && project_key.is_none_or(|key| source.project_key.as_deref() == Some(key))
            })
            .collect())
    }

    /// Retire every card generated from a project deck.
    ///
    /// # Errors
    ///
    /// Returns an error when the invalidate request fails.
    pub fn invalidate_deck(&self, deck_id: &str, event: &str) -> Result<StudyView, String> {
        self.post_json(
            &format!(
                "/v1/accounts/{}/project-decks/{deck_id}/invalidate",
                self.account_id
            ),
            &json!({ "event": event }),
        )
    }

    /// Fetch the next due review card.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn next_review(&self) -> Result<StudyView, String> {
        self.post_empty(&format!("/v1/accounts/{}/review/next", self.account_id))
    }

    /// Submit a graded answer for one review card.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn submit_review(
        &self,
        review_unit_id: &str,
        answer: &str,
        response_time_ms: u32,
        idempotency_key: &str,
    ) -> Result<StudyView, String> {
        self.post_json(
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/submit",
                self.account_id
            ),
            &json!({
                "answer": answer,
                "responseTimeMs": response_time_ms,
                "idempotencyKey": idempotency_key,
            }),
        )
    }

    /// Reveal the current card's expected answer without grading it.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn reveal_review(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/reveal",
            self.account_id
        ))
    }

    /// Declared remediation: request extra reference material for the
    /// current card instead of grading it now.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn learn_more(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/reference",
            self.account_id
        ))
    }

    /// Declared remediation: skip the current card, leaving its schedule
    /// untouched, and advance to the next due card.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn skip_review(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/skip",
            self.account_id
        ))
    }

    /// Declared remediation: push just this card later in the due queue.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn snooze_review(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/snooze",
            self.account_id
        ))
    }

    /// Declared remediation: push every card for this card's concept later
    /// in the due queue.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn snooze_concept_review(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/snooze-concept",
            self.account_id
        ))
    }

    /// Declared remediation: request bridge (scaffold) material for a card
    /// the learner is consistently missing.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub fn bridge_review(&self, review_unit_id: &str) -> Result<StudyView, String> {
        self.post_empty(&format!(
            "/v1/accounts/{}/review/{review_unit_id}/bridge",
            self.account_id
        ))
    }

    /// Record a `kept`/`dropped` content-feedback verdict on one review
    /// unit's generated content — distinct from grading an answer.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails, including a declared
    /// conflict (HTTP 409) when `idempotency_key`/`supersedes_id` no longer
    /// match the current feedback revision.
    pub fn content_feedback(
        &self,
        review_unit_id: &str,
        verdict: &str,
        rationale: Option<&str>,
        idempotency_key: &str,
        supersedes_id: Option<&str>,
    ) -> Result<ContentFeedback, String> {
        let mut request = json!({
            "verdict": verdict,
            "idempotencyKey": idempotency_key,
        });
        if let Some(rationale) = rationale {
            request["rationale"] = json!(rationale);
        }
        if let Some(supersedes_id) = supersedes_id {
            request["supersedesId"] = json!(supersedes_id);
        }
        self.post_json(
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/content-feedback",
                self.account_id
            ),
            &request,
        )
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let mut response = self
            .agent
            .get(endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .call()
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let mut response = self
            .agent
            .post(endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_empty()
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, String> {
        let mut response = self
            .agent
            .post(endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_json(body)
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.session_token)
    }
}

/// Create an account against `base_url`, outside any existing `MemoryEngineClient`
/// (no account id/session token exist yet).
///
/// # Errors
///
/// Returns an error when the create-account request fails.
pub fn create_account(base_url: &str, email: &str) -> Result<AccountCreated, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(endpoint(base_url, "/v1/accounts"))
        .send_json(json!({ "email": email }))
        .map_err(|error| transport_failure("create account", &error))?;
    read_json(&mut response, "create account")
}

#[derive(Deserialize)]
struct ApiError {
    error: String,
}

/// Deserialize a successful response, or surface the server's safe
/// `{"error": "..."}` message on a non-2xx status instead of a bare status
/// code (the agent built with `http_status_as_error(false)` above never
/// turns a 4xx/5xx into a transport error, so every call reaches here).
fn read_json<T: DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
    action: &str,
) -> Result<T, String> {
    let status = response.status();
    if status.is_success() {
        return response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_json()
            .map_err(|error| format!("{action} returned unreadable JSON: {error}"));
    }

    let body: Result<ApiError, _> = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json();
    match body {
        Ok(ApiError { error }) => Err(format!("{action} failed: {error} (HTTP {status})")),
        Err(_) => Err(format!("{action} failed with HTTP {status}")),
    }
}

fn transport_failure(action: &str, error: &ureq::Error) -> String {
    format!("{action} transport failed: {error}")
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::{Request, State},
        middleware::{self, Next},
        response::Response,
    };

    use super::*;

    type RequestLog = Arc<Mutex<Vec<(String, String)>>>;

    async fn spawn_local_api() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local API listener");
        let address = listener.local_addr().expect("local address");
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                memory_engine_api::router(memory_engine_api::ApiState::default()),
            )
            .await
            .expect("serve local API");
        });
        (format!("http://{address}"), handle)
    }

    /// Same real local API as `spawn_local_api`, but with its generation-jobs
    /// worker started (so a queued job actually progresses to a terminal
    /// state) and a request-capture layer recording every method+path this
    /// crate's client sends it — the evidence behind
    /// `create_deck_enqueues_and_polls_without_ever_requesting_generate`
    /// below, which asserts on the capture instead of scanning source text.
    async fn spawn_local_api_with_capture() -> (String, tokio::task::JoinHandle<()>, RequestLog) {
        let state = memory_engine_api::ApiState::default();
        state.start_worker();

        let requests: RequestLog = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local API listener");
        let address = listener.local_addr().expect("local address");
        let router = memory_engine_api::router(state).layer(middleware::from_fn_with_state(
            requests.clone(),
            capture_request,
        ));
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve local API");
        });
        (format!("http://{address}"), handle, requests)
    }

    async fn capture_request(State(log): State<RequestLog>, req: Request, next: Next) -> Response {
        log.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((req.method().to_string(), req.uri().path().to_owned()));
        next.run(req).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_legacy_origin_unapproved_draft_can_be_inspected_and_approved() {
        let (base_url, server) = spawn_local_api().await;

        let created = create_account(&base_url, "mcp-client-recovery-test@example.com")
            .expect("create account");
        let client = MemoryEngineClient::new(
            base_url.clone(),
            created.account_id.clone(),
            created.session_token.clone(),
        );

        // Seed an accepted-but-unapproved draft the way a pre-existing or
        // interrupted lineage would: directly against the legacy synchronous
        // route, bypassing this crate's own client entirely (this ApiState
        // has no `MEMORY_ENGINE_POSTGRES_URL`, so the legacy route is not
        // yet refused with 409 the way every production deployment refuses
        // it — this fixture only needs a real unapproved draft to exist).
        let source: serde_json::Value = ureq::post(endpoint(&base_url, &format!(
            "/v1/accounts/{}/sources",
            created.account_id
        )))
        .header("Authorization", &format!("Bearer {}", created.session_token))
        .send_json(json!({
            "title": "legacy-origin fixture",
            "body": "Concept: NATO letter B\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for B?\nAnswer: BRAVO\nDistractors: ALFA, CHARLIE\nReference: The NATO phonetic alphabet word for B is BRAVO.",
        }))
        .expect("create source")
        .body_mut()
        .read_json()
        .expect("source json");
        let source_id = source["sourceId"].as_str().expect("sourceId").to_owned();
        ureq::post(endpoint(
            &base_url,
            &format!(
                "/v1/accounts/{}/sources/{source_id}/generate",
                created.account_id
            ),
        ))
        .header(
            "Authorization",
            &format!("Bearer {}", created.session_token),
        )
        .send_empty()
        .expect("legacy generate");

        let pending = client.pending_drafts().expect("pending drafts");
        assert_eq!(
            pending.len(),
            1,
            "the legacy route must leave its draft unapproved"
        );
        assert!(!pending[0].approved);
        assert_eq!(pending[0].validation_status, "accepted");

        let due_before = client.next_review().expect("study view before approval");
        assert_eq!(
            due_before.due_count, 0,
            "an unapproved draft must not be due"
        );

        let approved_view = client
            .approve_draft(&pending[0].id)
            .expect("approve the pending draft");
        assert_eq!(
            approved_view.due_count, 1,
            "approval must schedule the card"
        );

        let remaining = client
            .pending_drafts()
            .expect("pending drafts after approval");
        assert!(
            remaining.is_empty(),
            "the approved draft must no longer be pending"
        );

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn errors_surface_the_servers_safe_message_not_a_bare_status_code() {
        let (base_url, server) = spawn_local_api().await;
        let created = create_account(&base_url, "mcp-client-safe-error-test@example.com")
            .expect("create account");
        let client = MemoryEngineClient::new(base_url, created.account_id, created.session_token);

        let error = client
            .approve_draft("draft-does-not-exist")
            .expect_err("an unknown draft id must fail");
        assert!(
            error.contains("Unknown generated prompt draft: draft-does-not-exist"),
            "error must carry the server's safe message, not a bare status code: {error}"
        );

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_deck_enqueues_and_polls_without_ever_requesting_generate() {
        let (base_url, server, requests) = spawn_local_api_with_capture().await;

        let created = create_account(&base_url, "mcp-client-generate-guard-test@example.com")
            .expect("create account");
        let client = MemoryEngineClient::new(
            base_url,
            created.account_id.clone(),
            created.session_token.clone(),
        );

        let deck_body = "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\n\
            Question: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\n\
            Distractors: BRAVO, CHARLIE\n\
            Reference: The NATO phonetic alphabet word for A is ALFA.";

        let (_deck, outcome) = client
            .create_deck("nato-onboarding", "NATO letter A fixture", deck_body, None)
            .expect("create_deck reaches a terminal outcome against a real worker");
        assert!(
            matches!(outcome, GenerationOutcome::Succeeded { .. }),
            "the job must reach succeeded against a real running worker, not time out: {outcome:?}"
        );

        server.abort();

        let captured = requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !captured.iter().any(|(_, path)| path.ends_with("/generate")),
            "create_deck must never request the legacy synchronous /generate route; captured: {captured:?}"
        );
        assert!(
            captured
                .iter()
                .any(|(method, path)| method == "POST" && path.ends_with("/generation-jobs")),
            "create_deck must enqueue on the durable generation-jobs queue; captured: {captured:?}"
        );
        assert!(
            captured
                .iter()
                .any(|(method, path)| method == "GET" && path.contains("/generation-jobs/")),
            "create_deck must poll the enqueued job's status; captured: {captured:?}"
        );
    }
}
