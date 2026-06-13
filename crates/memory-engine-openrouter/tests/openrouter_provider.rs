use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

use memory_engine_core::ReviewUnitId;
use memory_engine_generation::{
    BridgeMaterialProvider, BridgeMaterialRequest, DraftProvider, DraftRejection, LearningIntent,
    ReferenceNoteProvider, ReferenceNoteRequest, ReviewPerformanceContext,
};
use memory_engine_openrouter::{OpenRouterConfig, OpenRouterProvider, PromptVariant};
use memory_engine_persistence::{SourceDocument, SourceDocumentKind, SourcePermission};

const NOW: i64 = 1_780_162_400_000;

#[test]
fn maps_model_json_to_grounded_draft_candidates_with_usage() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "learning_intent": "concept_understanding",
                    "drafts": [
                        {
                            "concept": "Mitochondria ATP production",
                            "question": "What do mitochondria generate most of?",
                            "answer": "The cell's supply of adenosine triphosphate",
                            "evidence_quote": "generate most of the cell's supply of adenosine triphosphate",
                            "distractors": ["Ribosomal RNA", "Chlorophyll"],
                            "activity_kind": "quiz",
                            "activity_stage": "free-recall",
                            "worked_solution": ""
                        },
                        {
                            "concept": "",
                            "question": "Malformed draft",
                            "answer": "",
                            "evidence_quote": "",
                            "distractors": [],
                            "activity_kind": "quiz",
                            "activity_stage": "recognition",
                            "worked_solution": ""
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

    assert_eq!(
        drafts.learning_intent,
        Some(LearningIntent::ConceptUnderstanding)
    );
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
    assert!(prompt.contains("learning_intent"));
    assert!(prompt.contains("verbatim_memorization"));
    assert!(prompt.contains("recitation-ladder exercise drafts only"));
    assert!(prompt.contains("semantically adjacent confusions"));
    assert!(prompt.contains("never format variants"));
}

#[test]
fn sends_repair_feedback_and_parses_repaired_drafts_with_usage() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "learning_intent": "concept_understanding",
                    "drafts": [{
                        "concept": "Mitochondria ATP production",
                        "question": "Why are mitochondria associated with ATP supply?",
                        "answer": "They generate most of the cell's supply of adenosine triphosphate.",
                        "evidence_quote": "generate most of the cell's supply of adenosine triphosphate",
                        "distractors": ["They package ribosomal RNA", "They absorb chlorophyll"],
                        "activity_kind": "quiz",
                        "activity_stage": "recognition",
                        "worked_solution": ""
                    }]
                }).to_string()
            }
        }],
        "usage": {
            "prompt_tokens": 900,
            "completion_tokens": 180,
            "cost": 0.000_31
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
    let repair = provider
        .repair_drafts(
            &prose_source(),
            &[DraftRejection {
                index: 1,
                concept: "Mitochondria ATP production".to_owned(),
                question: "What do mitochondria generate?".to_owned(),
                answer: "ATP".to_owned(),
                reasons: vec![
                    "Duplicate-ish generated draft".to_owned(),
                    "Evidence quote not found in cited source".to_owned(),
                ],
            }],
        )
        .expect("repair request")
        .expect("repair drafts");

    assert_eq!(repair.candidates.len(), 1);
    assert_eq!(repair.usage.expect("usage").cost_usd_micros, Some(310));

    let request = request
        .recv_timeout(Duration::from_secs(1))
        .expect("request");
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(
        payload["response_format"]["json_schema"]["name"],
        "quiz_draft_repair"
    );
    let prompt = payload["messages"][0]["content"].as_str().expect("prompt");
    assert!(prompt.contains("Repair pass:"));
    assert!(prompt.contains("Generate fresh replacements only"));
    assert!(prompt.contains("Duplicate-ish generated draft"));
    assert!(prompt.contains("Evidence quote not found in cited source"));
    assert!(prompt.contains("semantically adjacent confusions"));
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

#[test]
fn maps_model_json_to_reference_note() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "title": "NATO letter A",
                    "body": "The NATO code word for A is Alfa."
                }).to_string()
            }
        }]
    });
    let (base_url, request) = serve_once(200, &body.to_string());

    let provider = OpenRouterProvider::new(test_config(base_url));
    let note = provider
        .explain_concept(&ReferenceNoteRequest {
            concept_key: "nato-letter-a".to_owned(),
            concept_label: "NATO letter A".to_owned(),
            prompt: "What is the NATO phonetic alphabet word for A?".to_owned(),
            expected_answer: "ALFA".to_owned(),
            recent_performance: Vec::new(),
        })
        .expect("reference note");

    assert_eq!(note.title, "NATO letter A");
    assert!(note.body.contains("Alfa"));
    let request = request
        .recv_timeout(Duration::from_secs(1))
        .expect("request");
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(
        payload["response_format"]["json_schema"]["name"],
        "reference_note"
    );
    let prompt = payload["messages"][0]["content"].as_str().expect("prompt");
    assert!(prompt.contains("NATO letter A"));
    assert!(prompt.contains("ALFA"));
}

#[test]
fn maps_model_json_to_bridge_material_candidates() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "reference_note": {
                        "title": "NATO letter A",
                        "body": "The NATO code word for A is Alfa."
                    },
                    "drafts": [
                        {
                            "concept": "NATO letter A",
                            "question": "Which word cues the letter A?",
                            "answer": "Alfa",
                            "distractors": ["Bravo", "Charlie"],
                            "activity_kind": "quiz",
                            "activity_stage": "recognition-bridge",
                            "worked_solution": ""
                        },
                        {
                            "concept": "NATO letter A",
                            "question": "Use the cue Alfa to answer the original item.",
                            "answer": "Alfa",
                            "distractors": [],
                            "activity_kind": "exercise",
                            "activity_stage": "cued-recall-bridge",
                            "worked_solution": "Alfa maps to the letter A."
                        }
                    ]
                }).to_string()
            }
        }],
        "usage": {
            "prompt_tokens": 500,
            "completion_tokens": 125,
            "cost": 0.000_2
        }
    });
    let (base_url, request) = serve_once(200, &body.to_string());

    let provider = OpenRouterProvider::new(test_config(base_url));
    let material = provider
        .generate_bridge_material(&BridgeMaterialRequest {
            concept_key: "nato-letter-a".to_owned(),
            concept_label: "NATO letter A".to_owned(),
            parent_review_unit_id: ReviewUnitId::new("parent-nato-a"),
            parent_prompt: "What is the NATO phonetic alphabet word for A?".to_owned(),
            parent_expected_answer: "ALFA".to_owned(),
            parent_stage_order: 4,
            cached_reference_note: None,
            recent_performance: vec![ReviewPerformanceContext {
                review_unit_id: "parent-nato-a".to_owned(),
                submitted_answer: "BRAVO".to_owned(),
                verdict: Some("wrong".to_owned()),
            }],
        })
        .expect("bridge material");

    assert_eq!(material.reference_note.title, "NATO letter A");
    assert_eq!(material.candidates.len(), 2);
    assert_eq!(material.candidates[0].activity_stage, "recognition-bridge");
    assert_eq!(material.candidates[1].activity_stage, "cued-recall-bridge");
    assert_eq!(
        material.candidates[1].worked_solution.as_deref(),
        Some("Alfa maps to the letter A.")
    );
    assert_eq!(material.usage.expect("usage").cost_usd_micros, Some(200));
    let request = request
        .recv_timeout(Duration::from_secs(1))
        .expect("request");
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(
        payload["response_format"]["json_schema"]["name"],
        "bridge_material"
    );
    let prompt = payload["messages"][0]["content"].as_str().expect("prompt");
    assert!(prompt.contains("PARENT EXPECTED ANSWER: ALFA"));
    assert!(prompt.contains("RECENT PERFORMANCE:"));
    assert!(prompt.contains("parent-nato-a"));
    assert!(prompt.contains("BRAVO"));
    assert!(prompt.contains("Generate exactly 2 easier drafts"));
    assert!(prompt.contains("recognition-bridge"));
    assert!(prompt.contains("cued-recall-bridge"));
}

#[test]
fn rejects_bridge_material_without_explicit_bridge_stages() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": serde_json::json!({
                    "reference_note": {
                        "title": "NATO letter A",
                        "body": "The NATO code word for A is Alfa."
                    },
                    "drafts": [{
                        "concept": "NATO letter A",
                        "question": "Which word cues the letter A?",
                        "answer": "Alfa",
                        "distractors": ["Bravo", "Charlie"],
                        "activity_kind": "quiz",
                        "activity_stage": "0.3",
                        "worked_solution": ""
                    }]
                }).to_string()
            }
        }]
    });
    let (base_url, _request) = serve_once(200, &body.to_string());

    let provider = OpenRouterProvider::new(test_config(base_url));
    let failure = provider
        .generate_bridge_material(&BridgeMaterialRequest {
            concept_key: "nato-letter-a".to_owned(),
            concept_label: "NATO letter A".to_owned(),
            parent_review_unit_id: ReviewUnitId::new("parent-nato-a"),
            parent_prompt: "What is the NATO phonetic alphabet word for A?".to_owned(),
            parent_expected_answer: "ALFA".to_owned(),
            parent_stage_order: 4,
            cached_reference_note: None,
            recent_performance: Vec::new(),
        })
        .expect_err("numeric bridge stages must not be normalized into easier rungs");

    assert!(failure
        .to_string()
        .contains("bridge activity_stage must be recognition-bridge or cued-recall-bridge"));
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
