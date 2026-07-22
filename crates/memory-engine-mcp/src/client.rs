//! HTTP client over the deployed `memory-engine-api` v1 contract.
//!
//! This mirrors `memory-engine-review`'s `ReviewClient`: a Bearer-token
//! `ureq` client against the same v1 routes, kept deliberately thin. It adds
//! no new server surface — every method below maps to one existing v1 route.

use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRow {
    pub id: String,
    pub validation_status: String,
    #[serde(default)]
    pub learner_decision: Option<serde_json::Value>,
    #[serde(default)]
    pub source_spans: Vec<serde_json::Value>,
    #[serde(default)]
    pub provenance: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyView {
    #[serde(default)]
    pub drafts: Vec<DraftRow>,
    pub current: Option<StudyCurrent>,
    pub due_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyCurrent {
    pub review_unit_id: String,
    pub prompt: String,
    #[serde(default)]
    pub choices: Vec<String>,
    pub expected_answer: Option<String>,
    pub grade: Option<StudyGrade>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyGrade {
    pub verdict: String,
    pub rating: u8,
    pub is_correct: bool,
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

    /// Create a project-scoped deck, generate its review cards, and explicitly keep
    /// every undecided accepted draft, so the deck is immediately due for study. This
    /// composes three v1 calls (`project-decks`, `sources/{id}/generate`,
    /// `drafts/{id}/keep`) behind one agent-intent verb: an agent asking
    /// to "capture this as a deck" wants reviewable cards, not a bare saved
    /// source record.
    ///
    /// # Errors
    ///
    /// Returns an error when any of the composed HTTP calls fails.
    pub fn create_deck(
        &self,
        project_key: &str,
        title: &str,
        body: &str,
        ttl_expires_at: Option<i64>,
    ) -> Result<(ProjectDeckRecord, usize), String> {
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

        let view: StudyView = self.post_empty(&format!(
            "/v1/accounts/{}/sources/{}/generate",
            self.account_id, deck.source.source_id
        ))?;
        let pending = view
            .drafts
            .iter()
            .filter(|draft| {
                draft.validation_status == "accepted" && draft.learner_decision.is_none()
            })
            .map(|draft| draft.id.clone())
            .collect::<Vec<_>>();
        let kept_count = pending.len();
        for draft_id in pending {
            let _: StudyView = self.post_empty(&format!(
                "/v1/accounts/{}/drafts/{draft_id}/keep",
                self.account_id
            ))?;
        }

        Ok((deck, kept_count))
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
        .build()
        .into();
    let mut response = agent
        .post(endpoint(base_url, "/v1/accounts"))
        .send_json(json!({ "email": email }))
        .map_err(|error| transport_failure("create account", &error))?;
    read_json(&mut response, "create account")
}

fn read_json<T: DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
    action: &str,
) -> Result<T, String> {
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json()
        .map_err(|error| format!("{action} returned unreadable JSON: {error}"))
}

fn transport_failure(action: &str, error: &ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(status) => format!("{action} failed with HTTP {status}"),
        _ => format!("{action} transport failed: {error}"),
    }
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}
