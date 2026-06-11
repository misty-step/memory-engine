use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

use memory_engine_canary::{CanaryConfig, CanaryReporter, ErrorEvent, Severity};

#[test]
fn posts_the_canary_error_contract() {
    let (endpoint, request) = serve_once(
        200,
        r#"{"id":"ERR-1","group_hash":"h","is_new_class":true}"#,
    );
    let reporter = CanaryReporter::new(test_config(endpoint));

    reporter.report(&ErrorEvent {
        error_class: "ApiFailure".to_owned(),
        message: "store error: connection refused".to_owned(),
        severity: Severity::Error,
        context: Some(serde_json::json!({ "route": "/app/generate" })),
        fingerprint: vec!["api-internal".to_owned()],
    });
    reporter.drain(Duration::from_secs(2));

    let request = request
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert!(request.starts_with("POST /api/v1/errors"));
    assert!(request.contains("Bearer sk_test_key"));
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(payload["service"], "memory-engine-api");
    assert_eq!(payload["environment"], "test");
    assert_eq!(payload["error_class"], "ApiFailure");
    assert_eq!(payload["message"], "store error: connection refused");
    assert_eq!(payload["severity"], "error");
    assert_eq!(payload["context"]["route"], "/app/generate");
    assert_eq!(payload["fingerprint"][0], "api-internal");
}

#[test]
fn reporting_failures_never_reach_the_caller() {
    // Nothing listening on this endpoint: report must not panic or block long.
    let reporter = CanaryReporter::new(test_config("http://127.0.0.1:9".to_owned()));

    reporter.report(&simple_event("unreachable"));
    reporter.drain(Duration::from_secs(3));
}

#[test]
fn from_env_is_none_without_credentials() {
    // Run in-process with the vars guaranteed absent by using bogus names is
    // not possible; instead assert the constructor contract directly.
    assert!(CanaryConfig::from_parts(None, Some("k".to_owned())).is_none());
    assert!(CanaryConfig::from_parts(Some("http://x".to_owned()), None).is_none());
    assert!(CanaryConfig::from_parts(Some("http://x".to_owned()), Some("k".to_owned())).is_some());
}

fn simple_event(message: &str) -> ErrorEvent {
    ErrorEvent {
        error_class: "Test".to_owned(),
        message: message.to_owned(),
        severity: Severity::Error,
        context: None,
        fingerprint: Vec::new(),
    }
}

fn test_config(endpoint: String) -> CanaryConfig {
    let mut config =
        CanaryConfig::from_parts(Some(endpoint), Some("sk_test_key".to_owned())).expect("config");
    "test".clone_into(&mut config.environment);
    config
}

/// Serve exactly one HTTP request with a canned response; returns the
/// endpoint and a channel yielding the raw request for assertions.
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
        let response = format!(
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("write");
    });

    (format!("http://{address}"), receiver)
}
