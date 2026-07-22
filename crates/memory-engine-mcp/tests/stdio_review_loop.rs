//! Cold-agent evidence: spawns the real compiled `memory-engine-mcp` binary
//! (not a mocked transport) and drives a full deck-create -> review ->
//! invalidate loop over its actual stdin/stdout JSON-RPC pipes, against a
//! real local `memory-engine-api` instance (the same Rust binary that runs
//! in production, an in-process axum server here). A hand-run transcript of
//! this same flow, captured from a real terminal, lives at
//! `docs/dogfood/mcp-review-loop.md`.

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
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            memory_engine_api::router(memory_engine_api::ApiState::default()),
        )
        .await
        .expect("serve local API");
    });

    let base_url = format!("http://{address}");
    let email = format!("memory-engine-mcp-test-{}@example.com", unique_suffix());
    let created: Value = ureq::post(format!("{base_url}/v1/accounts"))
        .send_json(json!({ "email": email }))
        .expect("create account")
        .body_mut()
        .read_json()
        .expect("account json");
    let account_id = created["accountId"].as_str().expect("accountId").to_owned();
    let session_token = created["sessionToken"]
        .as_str()
        .expect("sessionToken")
        .to_owned();

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

    // 1. initialize
    let init = call(
        &mut stdin,
        &rx,
        &mut transcript,
        1,
        "initialize",
        &json!({}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "memory-engine");

    // 2. tools/list — six agent-intent tools, not REST-route echoes.
    let list = rpc(
        &mut stdin,
        &rx,
        &mut transcript,
        2,
        "tools/list",
        &json!({}),
    );
    let tool_names = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(tool_names.len(), 6);
    assert!(tool_names.contains(&"create_deck".to_owned()));

    // 3. create_deck — one call composes save+generate+keep; the deck is
    //    immediately due, proving the tool is intent-shaped, not a 1:1 wrapper
    //    around POST /project-decks.
    let deck_body = "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\n\
         Question: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\n\
         Distractors: BRAVO, CHARLIE\n\
         Reference: The NATO phonetic alphabet word for A is ALFA.";
    let created_deck = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        3,
        "create_deck",
        &json!({
            "project_key": "nato-onboarding",
            "title": "NATO letter A fixture",
            "body": deck_body,
        }),
    );
    let deck_payload = tool_payload(&created_deck);
    assert!(
        deck_payload["approvedCardCount"].as_u64().unwrap_or(0) >= 1,
        "create_deck must generate and keep at least one card: {deck_payload}"
    );
    let deck_id = deck_payload["deck"]["deckId"]
        .as_str()
        .expect("deckId")
        .to_owned();

    // 4. list_decks — the new deck is visible, scoped to its project_key.
    let listed_decks = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        4,
        "list_decks",
        &json!({"project_key": "nato-onboarding"}),
    );
    let listed_decks_payload = tool_payload(&listed_decks);
    assert_eq!(listed_decks_payload.as_array().map(Vec::len), Some(1));

    // 5. list_due — a lightweight status check before committing to review.
    let due = call_tool(&mut stdin, &rx, &mut transcript, 5, "list_due", &json!({}));
    let due_payload = tool_payload(&due);
    assert_eq!(due_payload["dueCount"], 1);

    // 6. review_next — full detail needed to actually answer.
    let next = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        6,
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

    // 7. submit_answer — grades the card and advances the schedule.
    let submitted = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        7,
        "submit_answer",
        &json!({"review_unit_id": review_unit_id, "answer": "ALFA"}),
    );
    let submitted_payload = tool_payload(&submitted);
    assert_eq!(submitted_payload["current"]["grade"]["verdict"], "correct");
    assert_eq!(submitted_payload["dueCount"], 0);

    // 8. invalidate_deck — retires the deck; due count stays at 0.
    let invalidated = call_tool(
        &mut stdin,
        &rx,
        &mut transcript,
        8,
        "invalidate_deck",
        &json!({"deck_id": deck_id, "event": "onboarding project closed"}),
    );
    let invalidated_payload = tool_payload(&invalidated);
    assert_eq!(invalidated_payload["dueCount"], 0);

    drop(stdin);
    let _ = child.wait();
    let _ = reader.join();
    server.abort();

    assert!(
        transcript.len() >= 8,
        "expected at least 8 request/response pairs in the transcript, got {}",
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
    writeln!(stdin, "{request}").expect("write request line");
    stdin.flush().expect("flush stdin");
    let line = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("read response line before timeout");
    let response: Value = serde_json::from_str(&line).expect("response is valid json");
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
