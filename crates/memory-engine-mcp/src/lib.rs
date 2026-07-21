#![forbid(unsafe_code)]

//! Stdio MCP server wrapping the deployed `memory-engine-api` v1 study/review
//! contract. Tools are agent-intent-shaped (`create_deck`, `list_due`,
//! `review_next`, `submit_answer`, ...), not 1:1 REST wrappers — the same
//! discipline `powder-mcp` established for this fleet. It adds no new server
//! surface: every tool composes one or more existing v1 routes.
//!
//! Generation is queue-based end to end: `create_deck` enqueues a durable
//! generation job and polls it to a bounded terminal state, never the legacy
//! synchronous `/generate` route (refused with HTTP 409 in every production
//! deployment — `memory-engine-api-state::registry::generate_source`). A
//! `succeeded` job's cards arrive already scheduled — the production job
//! runner optimistically approves every accepted draft as part of the job,
//! a policy shared with every other caller (including the browser UI) that
//! this ticket leaves unchanged. `approve_draft`/`list_drafts` cover drafts
//! that reach an unapproved state some other way (a partially-committed
//! job, or a legacy-origin draft): approving one is always an explicit,
//! separately audited call, never composed automatically by this crate.

pub mod client;
pub mod session;

use client::{GenerationOutcome, MemoryEngineClient};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: &'static str,
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "create_deck",
        description: "Capture material as a project-scoped study deck: saves the text and enqueues its generation job on the durable production queue, polling to a bounded terminal state. Returns the deck plus the generation outcome (job status, and any draft still pending an explicit decision — usually none, since a succeeded job's accepted cards are already scheduled). Use project_key to group decks by the project/source they came from, so the whole deck can be invalidated later in one call when that material goes stale.",
        input_schema: r#"{"type":"object","required":["project_key","title","body"],"properties":{"project_key":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},"ttl_expires_at":{"type":"integer","description":"Optional epoch-ms expiry after which the deck is eligible for cleanup."}}}"#,
    },
    ToolDef {
        name: "list_decks",
        description: "List project-scoped decks (not general saved material), optionally filtered to one project_key. Use to check what is already captured, or to find a deck_id to invalidate.",
        input_schema: r#"{"type":"object","properties":{"project_key":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "invalidate_deck",
        description: "Retire every review card generated from one project deck after an external event (the source material changed, the project shipped or was cancelled), so stale cards stop surfacing in review. Returns the updated due count.",
        input_schema: r#"{"type":"object","required":["deck_id","event"],"properties":{"deck_id":{"type":"string"},"event":{"type":"string","description":"Free-text reason this deck is being invalidated, kept for audit."}}}"#,
    },
    ToolDef {
        name: "list_drafts",
        description: "Inspect every currently pending (validator-accepted, not yet approved) generated draft across the account, with its prompt, worked solution, and validation status. Usually empty: a normal successful create_deck job already schedules its accepted cards. A non-empty result means a draft is stuck without a decision (a partially-committed generation job, or a legacy-origin draft) — use approve_draft to keep one, or leave it pending to decline it (rejection by omission; there is no separate reject call).",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    ToolDef {
        name: "approve_draft",
        description: "Explicitly keep one generated draft, scheduling it as a live review card. Use it for a draft list_drafts surfaced as pending (a partially-committed job, or a legacy-origin draft) — a normal create_deck job has already scheduled its own accepted cards, so this is a recovery/completion action, not a required follow-up to every generation.",
        input_schema: r#"{"type":"object","required":["draft_id"],"properties":{"draft_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "list_due",
        description: "Check how many reviews are due right now, with a short teaser of the next prompt. A lightweight status check — call review_next instead when you are actually ready to answer.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    ToolDef {
        name: "review_next",
        description: "Fetch the next due review card in full: prompt, multiple-choice options if any, concept key, and its review_unit_id. Call submit_answer with that review_unit_id, then call review_next again to advance.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    ToolDef {
        name: "submit_answer",
        description: "Grade an answer for the review card identified by review_unit_id (from review_next) and advance its schedule. Returns the verdict, rating, schedule change (before/after review state), post-answer feedback (item history, concept health), and the due count after grading.",
        input_schema: r#"{"type":"object","required":["review_unit_id","answer"],"properties":{"review_unit_id":{"type":"string"},"answer":{"type":"string"},"response_time_ms":{"type":"integer","minimum":1,"description":"Defaults to 5000 when omitted."},"idempotency_key":{"type":"string","description":"Defaults to a fresh key when omitted; pass your own to make retried submits safe to repeat."}}}"#,
    },
    ToolDef {
        name: "reveal_answer",
        description: "Reveal the current review card's expected answer without grading it — use when the learner wants to see the answer instead of attempting one.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "learn_more",
        description: "Declared remediation: request extra reference material for the current review card instead of grading it now. Use when the learner needs more context before attempting an answer.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "skip_review",
        description: "Declared remediation: skip the current review card for this pass, leaving its schedule untouched, and advance to the next due card.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "snooze_review",
        description: "Declared remediation: push just this review card later in the due queue, without grading it.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "snooze_concept",
        description: "Declared remediation: push every review card for this card's concept later in the due queue — use when the whole concept needs a break, not just one card.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "bridge_review",
        description: "Declared remediation: request bridge (scaffold) material for a review card the learner keeps missing, to rebuild the prerequisite before re-attempting it.",
        input_schema: r#"{"type":"object","required":["review_unit_id"],"properties":{"review_unit_id":{"type":"string"}}}"#,
    },
    ToolDef {
        name: "record_content_feedback",
        description: "Record a kept/dropped verdict on one review card's generated content itself (is this card good or bad), distinct from grading an answer. Pass supersedes_id with the prior contentFeedbackHeadId (from review_next/submit_answer's current.contentFeedbackHeadId) when correcting an earlier verdict for the same card.",
        input_schema: r#"{"type":"object","required":["review_unit_id","verdict"],"properties":{"review_unit_id":{"type":"string"},"verdict":{"type":"string","enum":["kept","dropped"]},"rationale":{"type":"string"},"idempotency_key":{"type":"string","description":"Defaults to a fresh key when omitted."},"supersedes_id":{"type":"string","description":"The prior content-feedback id being corrected, if any."}}}"#,
    },
];

#[must_use]
pub fn tools() -> &'static [ToolDef] {
    TOOLS
}

/// Render every tool's JSON Schema for a `tools/list` response.
///
/// # Panics
///
/// Panics if a `TOOLS` entry's `input_schema` literal is not valid JSON —
/// that is a compile-time-constant bug in this crate, never runtime input.
#[must_use]
pub fn tool_defs_json() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": serde_json::from_str::<Value>(tool.input_schema)
                        .expect("tool schema is valid json"),
                })
            })
            .collect(),
    )
}

/// Handle one JSON-RPC request line against `client`. Returns `None` for
/// notifications (no `id`), matching the JSON-RPC 2.0 contract.
pub fn handle_json_rpc(client: &MemoryEngineClient, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2024-11-05"),
            "serverInfo": {"name": "memory-engine", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {"listChanged": false}},
        })),
        "tools/list" => Ok(json!({ "tools": tool_defs_json() })),
        "tools/call" => {
            let params = &request["params"];
            let name = params["name"].as_str().unwrap_or("");
            let args = &params["arguments"];
            call_tool(client, name, args)
        }
        "ping" => Ok(json!({})),
        other => Err(format!("method not found: {other}")),
    };

    id.map(|id| match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32603, "message": message},
        }),
    })
}

/// Dispatch one `tools/call` to `client`.
///
/// # Errors
///
/// Returns an error when `name` is unknown, a required argument is missing,
/// or the underlying HTTP call to `memory-engine-api` fails.
#[allow(clippy::too_many_lines)]
pub fn call_tool(client: &MemoryEngineClient, name: &str, args: &Value) -> Result<Value, String> {
    let payload = match name {
        "create_deck" => {
            let project_key = required_str(args, "project_key")?;
            let title = required_str(args, "title")?;
            let body = required_str(args, "body")?;
            let ttl_expires_at = args["ttl_expires_at"].as_i64();
            let (deck, outcome) = client.create_deck(project_key, title, body, ttl_expires_at)?;
            json!({ "deck": deck, "generation": generation_outcome_json(&outcome) })
        }
        "list_decks" => {
            let project_key = args["project_key"].as_str();
            json!(client.list_decks(project_key)?)
        }
        "invalidate_deck" => {
            let deck_id = required_str(args, "deck_id")?;
            let event = required_str(args, "event")?;
            json!(client.invalidate_deck(deck_id, event)?)
        }
        "list_drafts" => json!(client.pending_drafts()?),
        "approve_draft" => {
            let draft_id = required_str(args, "draft_id")?;
            json!(client.approve_draft(draft_id)?)
        }
        "list_due" => {
            let view = client.next_review()?;
            json!({
                "dueCount": view.due_count,
                "nextPrompt": view.current.as_ref().map(|current| current.prompt.clone()),
            })
        }
        "review_next" => json!(client.next_review()?),
        "submit_answer" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            let answer = required_str(args, "answer")?;
            let response_time_ms = args["response_time_ms"].as_u64().unwrap_or(5000);
            let response_time_ms = u32::try_from(response_time_ms).unwrap_or(u32::MAX);
            let idempotency_key = args["idempotency_key"]
                .as_str()
                .map_or_else(|| default_idempotency_key(review_unit_id), str::to_owned);
            json!(client.submit_review(
                review_unit_id,
                answer,
                response_time_ms,
                &idempotency_key
            )?)
        }
        "reveal_answer" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.reveal_review(review_unit_id)?)
        }
        "learn_more" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.learn_more(review_unit_id)?)
        }
        "skip_review" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.skip_review(review_unit_id)?)
        }
        "snooze_review" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.snooze_review(review_unit_id)?)
        }
        "snooze_concept" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.snooze_concept_review(review_unit_id)?)
        }
        "bridge_review" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            json!(client.bridge_review(review_unit_id)?)
        }
        "record_content_feedback" => {
            let review_unit_id = required_str(args, "review_unit_id")?;
            let verdict = required_str(args, "verdict")?;
            if verdict != "kept" && verdict != "dropped" {
                return Err(format!(
                    "verdict must be \"kept\" or \"dropped\", got {verdict:?}"
                ));
            }
            let rationale = args["rationale"].as_str();
            let idempotency_key = args["idempotency_key"]
                .as_str()
                .map_or_else(|| default_idempotency_key(review_unit_id), str::to_owned);
            let supersedes_id = args["supersedes_id"].as_str();
            json!(client.content_feedback(
                review_unit_id,
                verdict,
                rationale,
                &idempotency_key,
                supersedes_id
            )?)
        }
        other => return Err(format!("unknown tool: {other}")),
    };

    let text = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    Ok(json!({"content": [{"type": "text", "text": text}]}))
}

fn generation_outcome_json(outcome: &GenerationOutcome) -> Value {
    match outcome {
        GenerationOutcome::Succeeded {
            job,
            coalesced,
            drafts,
        } => json!({
            "status": "succeeded",
            "coalesced": coalesced,
            "job": job,
            "pendingDrafts": drafts,
        }),
        GenerationOutcome::Failed { job, coalesced } => json!({
            "status": "failed",
            "coalesced": coalesced,
            "job": job,
        }),
        GenerationOutcome::TimedOut { job, coalesced } => json!({
            "status": "timed_out",
            "coalesced": coalesced,
            "job": job,
        }),
    }
}

fn required_str<'a>(args: &'a Value, key: &'static str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn default_idempotency_key(review_unit_id: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("mcp-{review_unit_id}-{millis}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tools_are_agent_intents_not_rest_routes() {
        let names = TOOLS.iter().map(|tool| tool.name).collect::<Vec<_>>();

        assert_eq!(TOOLS.len(), 15);
        for expected in [
            "create_deck",
            "list_decks",
            "invalidate_deck",
            "list_drafts",
            "approve_draft",
            "list_due",
            "review_next",
            "submit_answer",
            "reveal_answer",
            "learn_more",
            "skip_review",
            "snooze_review",
            "snooze_concept",
            "bridge_review",
            "record_content_feedback",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }

        // No tool name is a REST-route echo (verb_noun, not noun/verb-http).
        for tool in TOOLS {
            assert!(
                !tool.name.contains('/'),
                "tool {} looks like a route, not an intent",
                tool.name
            );
        }
    }

    #[test]
    fn tool_defs_json_serializes_every_tool_with_a_valid_schema() {
        let value = tool_defs_json();
        let array = value.as_array().expect("array");
        assert_eq!(array.len(), TOOLS.len());
        for entry in array {
            assert!(entry["name"].is_string());
            assert!(entry["description"].is_string());
            assert!(entry["inputSchema"]["type"] == "object");
        }
    }

    #[test]
    fn call_tool_rejects_unknown_tool_names() {
        let client = MemoryEngineClient::new(
            "http://127.0.0.1:1".to_owned(),
            "acct_test".to_owned(),
            "token".to_owned(),
        );
        let error = call_tool(&client, "delete_everything", &json!({})).unwrap_err();
        assert!(error.contains("unknown tool"));
    }

    #[test]
    fn call_tool_requires_required_arguments() {
        let client = MemoryEngineClient::new(
            "http://127.0.0.1:1".to_owned(),
            "acct_test".to_owned(),
            "token".to_owned(),
        );
        let error = call_tool(&client, "create_deck", &json!({"title": "t"})).unwrap_err();
        assert!(error.contains("project_key"));
    }

    #[test]
    fn call_tool_rejects_an_invalid_content_feedback_verdict() {
        let client = MemoryEngineClient::new(
            "http://127.0.0.1:1".to_owned(),
            "acct_test".to_owned(),
            "token".to_owned(),
        );
        let error = call_tool(
            &client,
            "record_content_feedback",
            &json!({"review_unit_id": "ru_1", "verdict": "maybe"}),
        )
        .unwrap_err();
        assert!(error.contains("kept") && error.contains("dropped"));
    }

    #[test]
    fn create_deck_never_calls_the_legacy_synchronous_generate_route() {
        // `create_deck`'s only generation-shaped composition target is
        // `MemoryEngineClient::create_deck`, which — per client.rs — enqueues
        // on `/generation-jobs` and polls `/generation-jobs/{id}`; there is
        // no code path in this crate's production source that calls
        // `/generate`. This is a structural regression guard: the legacy
        // route name must not appear in this crate's non-test, non-doc
        // source. `client.rs`'s own `#[cfg(test)] mod tests` intentionally
        // calls the legacy route directly (bypassing `MemoryEngineClient`)
        // to seed a pre-existing unapproved draft fixture — that is a test
        // fixture, not a composition this crate ships, so it is excluded
        // from the scan rather than the guard weakened to allow it broadly.
        let source = include_str!("client.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("client.rs source");
        let legacy_calls = production_source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .any(|line| line.contains("/generate\"") || line.contains("/generate\","));
        assert!(
            !legacy_calls,
            "memory-engine-mcp must never call the legacy synchronous /generate route"
        );
    }
}
