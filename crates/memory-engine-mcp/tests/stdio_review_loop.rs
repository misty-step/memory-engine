//! Cold-agent evidence: spawns the real compiled `memory-engine-mcp` binary
//! (not a mocked transport) and drives a full deck-create -> inspect ->
//! keep -> review -> remediation -> feedback -> invalidate loop over its
//! actual stdin/stdout JSON-RPC pipes, against a real local
//! `memory-engine-api` instance (the same Rust binary that runs in
//! production, an in-process axum server with its background generation
//! worker started here — the same worker the deployed binary starts). A
//! hand-run transcript of this same flow, captured from a real terminal,
//! lives at `docs/dogfood/mcp-review-loop.md`.
//!
//! Generation goes through the durable job queue exclusively
//! (`create_deck` enqueues + polls `/generation-jobs`), proving the supported
//! production route contract even though this fixture's `ApiState` is an
//! ephemeral file store, not Postgres — the legacy synchronous `/generate`
//! route is never called regardless of backend (see
//! `create_deck_enqueues_and_polls_without_ever_requesting_generate` in
//! `src/client.rs` for a behavioral, HTTP-request-capture proof of that;
//! production additionally refuses `/generate` outright with HTTP 409 once
//! `MEMORY_ENGINE_POSTGRES_URL` is set —
//! `memory-engine-api-state::registry::generate_source`). A succeeded job's
//! accepted draft remains pending until this test explicitly calls
//! `keep_draft` — generation itself never schedules a card.

use std::{
    io::{BufRead, BufReader, Write},
    process::{ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
};

use serde_json::{json, Value};

#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_agent_completes_a_full_review_loop_over_stdio() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local API listener");
    let address = listener.local_addr().expect("local address");
    let email = format!("memory-engine-mcp-test-{}@example.com", unique_suffix());
    let store_root =
        std::env::temp_dir().join(format!("memory-engine-mcp-api-{}", unique_suffix()));
    let state = memory_engine_api::ApiState::new(
        memory_engine_api::AccountRegistry::with_store_root(&store_root).with_auth_config(
            memory_engine_api::AuthConfig::allow_emails([email.clone()])
                .with_anonymous_account_creation(true),
        ),
    );
    // Matches production's `main.rs`: generation is job-queue-based end to
    // end, so the worker must actually run for a queued job to ever reach
    // `succeeded`.
    state.start_worker();
    let created = state.create_account(&email).expect("pre-provision account");
    let account_id = created.account_id.clone();
    let session_token = created.session_token.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, memory_engine_api::router(state))
            .await
            .expect("serve local API");
    });

    let base_url = format!("http://{address}");

    let binary = env!("CARGO_BIN_EXE_memory-engine-mcp");
    let mut child = Command::new(binary)
        .env("MEMORY_ENGINE_MCP_BASE_URL", &base_url)
        .env("MEMORY_ENGINE_ACCOUNT_ID", &account_id)
        .env("MEMORY_ENGINE_SESSION_TOKEN", &session_token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn memory-engine-mcp");

    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut transcript = Vec::new();
    let mut id = 0_u64;
    let mut next_id = || {
        id += 1;
        id
    };

    // 1. initialize
    let init = call(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "initialize",
        &json!({}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "memory-engine");

    // 2. tools/list — seventeen agent-intent tools, not REST-route echoes.
    let list = rpc(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "tools/list",
        &json!({}),
    );
    let tool_names = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(tool_names.len(), 17);
    for expected in [
        "create_deck",
        "keep_draft",
        "edit_draft",
        "reject_draft",
        "list_decks",
        "invalidate_deck",
        "list_drafts",
        "reveal_answer",
        "record_content_feedback",
    ] {
        assert!(
            tool_names.contains(&expected.to_owned()),
            "missing {expected}"
        );
    }

    // 3. create_deck — saves the source and drives it through the durable
    //    generation-jobs queue (enqueue + bounded poll), never the legacy
    //    synchronous /generate route. Generation never auto-schedules
    //    (`registry.rs::run_generation_job` always returns a zero card
    //    count), so the accepted draft comes back pending for an explicit
    //    decision.
    let deck_body = "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\n\
         Question: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\n\
         Distractors: BRAVO, CHARLIE\n\
         Reference: The NATO phonetic alphabet word for A is ALFA.";
    let created_deck = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "create_deck",
        &json!({
            "project_key": "nato-onboarding",
            "title": "NATO letter A fixture",
            "body": deck_body,
        }),
    );
    let deck_payload = tool_payload(&created_deck);
    assert_eq!(
        deck_payload["generation"]["status"], "succeeded",
        "queued generation must reach succeeded: {deck_payload}"
    );
    assert_eq!(
        deck_payload["generation"]["job"]["cardCount"], 0,
        "generation never auto-schedules a card: {deck_payload}"
    );
    let pending_draft = deck_payload["generation"]["pendingDrafts"]
        .as_array()
        .and_then(|drafts| drafts.first())
        .expect("pending draft");
    assert!(pending_draft["sourceSpans"].is_array());
    assert!(pending_draft["provenance"].is_object());
    let pending_draft_id = pending_draft["id"]
        .as_str()
        .expect("pending draft id")
        .to_owned();
    let deck_id = deck_payload["deck"]["deckId"]
        .as_str()
        .expect("deckId")
        .to_owned();

    // 4. list_drafts — the same pending draft is visible account-wide,
    //    independent of the create_deck response above.
    let drafts = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "list_drafts",
        &json!({}),
    );
    let drafts_payload = tool_payload(&drafts).as_array().cloned().unwrap_or_default();
    assert_eq!(
        drafts_payload.len(),
        1,
        "the fixture's accepted draft must be pending until decided: {drafts_payload:?}"
    );
    assert_eq!(drafts_payload[0]["id"], pending_draft_id);

    // 5. keep_draft — only the explicit keep promotes the accepted draft.
    let kept = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "keep_draft",
        &json!({"draft_id": pending_draft_id}),
    );
    assert!(tool_payload(&kept)["drafts"].is_array());

    // 6. list_decks — the new deck is visible, scoped to its project_key.
    let listed_decks = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "list_decks",
        &json!({"project_key": "nato-onboarding"}),
    );
    assert_eq!(
        tool_payload(&listed_decks).as_array().map(Vec::len),
        Some(1)
    );

    // 7. list_due — the card kept above is now due.
    let due = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "list_due",
        &json!({}),
    );
    assert_eq!(tool_payload(&due)["dueCount"], 1);

    // 8. review_next — full detail needed to actually answer.
    let next = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "review_next",
        &json!({}),
    );
    let next_payload = tool_payload(&next);
    let review_unit_id = next_payload["current"]["reviewUnitId"]
        .as_str()
        .expect("reviewUnitId")
        .to_owned();
    assert!(next_payload["current"]["prompt"]
        .as_str()
        .unwrap_or_default()
        .contains("NATO phonetic alphabet word for A"));

    // 9. reveal_answer — declared remediation: show the answer without
    //    grading, and the queue stays on the same card.
    let revealed = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "reveal_answer",
        &json!({"review_unit_id": review_unit_id}),
    );
    let revealed_payload = tool_payload(&revealed);
    assert_eq!(revealed_payload["current"]["reviewUnitId"], review_unit_id);
    assert_eq!(revealed_payload["current"]["expectedAnswer"], "ALFA");

    // 10. submit_answer — grades the card and advances the schedule.
    let submitted = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "submit_answer",
        &json!({"review_unit_id": review_unit_id, "answer": "ALFA"}),
    );
    let submitted_payload = tool_payload(&submitted);
    assert_eq!(submitted_payload["current"]["grade"]["verdict"], "correct");
    assert_eq!(submitted_payload["dueCount"], 0);

    // 11. record_content_feedback — a kept/dropped verdict on the content
    //     itself, distinct from grading the answer.
    let feedback = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "record_content_feedback",
        &json!({"review_unit_id": review_unit_id, "verdict": "kept", "rationale": "clear and correct"}),
    );
    let feedback_payload = tool_payload(&feedback);
    assert_eq!(feedback_payload["verdict"], "kept");
    assert_eq!(feedback_payload["reviewUnitId"], review_unit_id);

    // 12. invalidate_deck — retires the deck; due count stays at 0.
    let invalidated = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        next_id(),
        "invalidate_deck",
        &json!({"deck_id": deck_id, "event": "onboarding project closed"}),
    );
    assert_eq!(tool_payload(&invalidated)["dueCount"], 0);

    drop(stdin);
    let _ = child.wait();
    let _ = reader.join();
    server.abort();

    assert!(
        transcript.len() >= 12,
        "expected at least 12 request/response pairs in the transcript, got {}",
        transcript.len()
    );
}

fn rpc(
    stdin: &mut ChildStdin,
    rx: &mpsc::Receiver<String>,
    transcript: &mut Vec<(Value, Value)>,
    id: u64,
    method: &str,
    params: &Value,
) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let line = serde_json::to_string(&request).expect("serialize request");
    writeln!(stdin, "{line}").expect("write request");
    stdin.flush().expect("flush request");

    let response_line = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap_or_else(|error| panic!("no response to {method} within timeout: {error}"));
    let response: Value = serde_json::from_str(&response_line).expect("response is valid json");
    transcript.push((request, response.clone()));
    response
}

fn call(
    stdin: &mut ChildStdin,
    rx: &mpsc::Receiver<String>,
    transcript: &mut Vec<(Value, Value)>,
    id: u64,
    method: &str,
    params: &Value,
) -> Value {
    rpc(stdin, rx, transcript, id, method, params)
}

fn call_tool(
    stdin: &mut ChildStdin,
    rx: &mpsc::Receiver<String>,
    transcript: &mut Vec<(Value, Value)>,
    id: u64,
    name: &str,
    arguments: &Value,
) -> Value {
    rpc(
        stdin,
        rx,
        transcript,
        id,
        "tools/call",
        &json!({"name": name, "arguments": arguments}),
    )
}

fn tool_payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool response carried no text content: {response}"));
    serde_json::from_str(text).expect("tool text payload is valid json")
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{millis}-{counter}", std::process::id())
}
