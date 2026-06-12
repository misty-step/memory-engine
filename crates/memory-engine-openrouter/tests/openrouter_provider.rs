use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

use memory_engine_generation::DraftProvider;
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider, PromptVariant};
use memory_engine_persistence::{SourceDocument, SourceDocumentKind, SourcePermission};

const NOW: i64 = 1_780_162_400_000;

#[test]
fn maps_model_json_to_grounded_draft_candidates_with_usage() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "drafts": [
                        {
                            "concept": "Mitochondria ATP production",
                            "question": "What do mitochondria generate most of?",
                            "answer": "The cell's supply of adenosine triphosphate",
                            "evidence_quote": "generate most of the cell's supply of adenosine triphosphate",
                            "distractors": ["Ribosomal RNA", "Chlorophyll"]
                        },
                        {
                            "concept": "",
                            "question": "Malformed draft",
                            "answer": "",
                            "evidence_quote": "",
                            "distractors": []
                        }
                    ]
                }).to_string()
            }
        }],
        "usage": {
            "prompt_tokens": 850,
            "completion_tokens": 220,
            "cost": 0.000_295
        }
    });
    let (base_url, request) = serve_once(200, &body.to_string());

    let provider = OpenRouterProvider::new(OpenRouterConfig {
        api_key: "test-key".to_owned(),
        model: "deepseek/deepseek-v4-flash".to_owned(),
        base_url,
        timeout: Duration::from_secs(5),
        prompt: PromptVariant::Principled,
        max_drafts: 8,
    });
    let drafts = provider
        .generate_drafts(&prose_source())
        .expect("provider output");

    assert_eq!(drafts.candidates.len(), 1);
    let candidate = &drafts.candidates[0];
    assert_eq!(candidate.index, 1);
    assert_eq!(candidate.concept, "Mitochondria ATP production");
    assert_eq!(
        candidate.evidence.as_deref(),
        Some("generate most of the cell's supply of adenosine triphosphate")
    );
    assert_eq!(candidate.distractors.len(), 2);
    assert_eq!(drafts.failures.len(), 1, "malformed draft must be reported");

    let usage = drafts.usage.expect("usage");
    assert_eq!(usage.input_tokens, 850);
    assert_eq!(usage.output_tokens, 220);
    assert_eq!(usage.cost_usd_micros, Some(295));

    assert_eq!(drafts.model.provider, "openrouter");
    assert_eq!(drafts.model.name, "deepseek/deepseek-v4-flash");

    let request = request
        .recv_timeout(Duration::from_secs(1))
        .expect("request");
    assert!(request.starts_with("POST /api/v1/chat/completions"));
    assert!(request.contains("Bearer test-key"));
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(payload["model"], "deepseek/deepseek-v4-flash");
    assert_eq!(payload["response_format"]["type"], "json_schema");
    assert_eq!(payload["response_format"]["json_schema"]["strict"], true);
    assert_eq!(payload["usage"]["include"], true);
    let prompt = payload["messages"][0]["content"].as_str().expect("prompt");
    assert!(
        prompt.contains("Mitochondria are organelles"),
        "prompt must carry the source text"
    );
}

#[test]
fn unparseable_model_payload_is_a_human_readable_failure() {
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "not json at all" } }]
    });
    let (base_url, _request) = serve_once(200, &body.to_string());

    let provider = OpenRouterProvider::new(test_config(base_url));
    let failure = provider
        .generate_drafts(&prose_source())
        .expect_err("must fail");

    let message = failure.to_string();
    assert!(
        message.contains("could not be read"),
        "expected human-readable message, got: {message}"
    );
}

#[test]
fn http_error_status_is_a_human_readable_failure() {
    let (base_url, _request) = serve_once(401, r#"{"error":{"message":"bad key"}}"#);

    let provider = OpenRouterProvider::new(test_config(base_url));
    let failure = provider
        .generate_drafts(&prose_source())
        .expect_err("must fail");

    let message = failure.to_string();
    assert!(
        message.contains("The model provider rejected the request"),
        "expected human-readable message, got: {message}"
    );
}

fn test_config(base_url: String) -> OpenRouterConfig {
    OpenRouterConfig {
        api_key: "test-key".to_owned(),
        model: "deepseek/deepseek-v4-flash".to_owned(),
        base_url,
        timeout: Duration::from_secs(5),
        prompt: PromptVariant::Minimal,
        max_drafts: 8,
    }
}

fn prose_source() -> SourceDocument {
    SourceDocument {
        id: "src-prose".to_owned(),
        kind: SourceDocumentKind::Text,
        title: "Mitochondria notes".to_owned(),
        body: Some(
            "Mitochondria are organelles that generate most of the cell's supply of \
             adenosine triphosphate."
                .to_owned(),
        ),
        uri: None,
        permission: SourcePermission::ModelEligible,
        freshness: Some(NOW),
        created_at: NOW,
        archived_at: None,
    }
}

/// Serve exactly one HTTP request with a canned response; returns the base
/// URL and a channel yielding the raw request for assertions.
fn serve_once(status: u16, body: &str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let body = body.to_owned();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).expect("read");
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            if let Some(header_end) = text.find("\r\n\r\n") {
                let content_length = text
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().expect("length"))
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            if read == 0 {
                break;
            }
        }
        sender
            .send(String::from_utf8_lossy(&request).into_owned())
            .expect("send request");
        let reason = if status == 200 { "OK" } else { "Error" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("write");
    });

    (format!("http://{address}/api/v1"), receiver)
}
