use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use memory_engine_canary::{
    read_performance_timeline, CanaryConfig, CanaryReporter, ErrorEvent, PerformanceBatch,
    ReadbackConfig, Severity, PERFORMANCE_BATCH_SCHEMA, PERFORMANCE_EVENT_NAME,
};
use memory_engine_performance::{
    Action, CompletionMarker, CompletionPhase, GenerationAction, MachineRouteAction, Navigation,
    Outcome, ReviewAction, Viewport,
};
use serde_json::{json, Value};

#[test]
fn aggregates_observations_into_one_bounded_namespace_event() {
    let (endpoint, requests) = serve_responses(vec![(200, json!({"id": "EVT-1"}))]);
    let reporter = CanaryReporter::new(test_config(endpoint));
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .expect("marker");

    for duration_ms in [11, 23, 47] {
        assert!(reporter.report_performance(marker.observation(duration_ms).expect("observation")));
    }
    assert!(reporter.flush(Duration::from_secs(2)));

    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("performance request");
    assert!(request.starts_with("POST /api/v1/events"));
    let body = request_body(&request);
    assert_eq!(body["name"], PERFORMANCE_EVENT_NAME);
    assert_eq!(body["attributes"]["schema"], PERFORMANCE_BATCH_SCHEMA);
    assert_eq!(body["attributes"]["authority"], "non_authoritative_debug");
    let batch = PerformanceBatch::decode(&body["attributes"]).expect("closed batch");
    assert_eq!(batch.snapshots().len(), 1);
    assert_eq!(batch.snapshots()[0].count(), 3);
    assert_eq!(batch.snapshots()[0].sum_ms(), 81);
    assert_eq!(batch.delivery().batches_sent(), 1);
    assert_eq!(batch.delivery().batches_retried(), 0);
    let encoded = body.to_string();
    for forbidden in [
        "account_id",
        "session_id",
        "source_id",
        "review_unit_id",
        "job_id",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "leaked forbidden field {forbidden}"
        );
    }
}

#[test]
fn emits_no_more_than_one_batch_per_namespace_for_a_flush() {
    let responses = (0..3)
        .map(|index| (200, json!({"id": format!("EVT-{index}")})))
        .collect();
    let (endpoint, requests) = serve_responses(responses);
    let reporter = CanaryReporter::new(test_config(endpoint));
    let observations = [
        CompletionMarker::browser(
            Action::Review(ReviewAction::Submit),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
            Navigation::JavascriptEnhanced,
            Viewport::Mobile,
        )
        .expect("browser marker")
        .observation(20)
        .expect("browser observation"),
        CompletionMarker::server(
            Action::Machine(MachineRouteAction::OpenApi),
            CompletionPhase::ImmediateAck,
            Outcome::Succeeded,
        )
        .expect("server marker")
        .observation(30)
        .expect("server observation"),
        CompletionMarker::job(
            Action::Generation(GenerationAction::DurableTerminal),
            CompletionPhase::DurableGenerationTerminal,
            Outcome::Succeeded,
        )
        .expect("job marker")
        .observation(40)
        .expect("job observation"),
    ];
    for observation in observations {
        assert!(reporter.report_performance(observation));
    }
    assert!(reporter.flush(Duration::from_secs(2)));

    let mut namespaces = Vec::new();
    for _ in 0..3 {
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("namespace request");
        namespaces.push(
            request_body(&request)["attributes"]["namespace"]
                .as_str()
                .expect("namespace")
                .to_owned(),
        );
    }
    namespaces.sort_unstable();
    assert_eq!(namespaces, ["browser", "job", "server"]);
    assert!(reporter.flush(Duration::from_secs(2)));
    assert!(
        requests.try_recv().is_err(),
        "emitted more than three batches"
    );
}

#[test]
fn saturated_network_worker_never_blocks_request_path_and_accounts_drops() {
    let (endpoint, accepted, requests) = serve_delayed_error_then_event();
    let reporter = CanaryReporter::new(test_config(endpoint));
    reporter.report(&ErrorEvent {
        error_class: "SyntheticBlock".to_owned(),
        message: "hold worker".to_owned(),
        severity: Severity::Info,
        context: None,
        fingerprint: Vec::new(),
    });
    accepted
        .recv_timeout(Duration::from_secs(2))
        .expect("worker entered network call");

    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .expect("marker");
    let observation = marker.observation(7).expect("observation");
    let started = Instant::now();
    let mut rejected = 0_u64;
    for _ in 0..10_000 {
        if !reporter.report_performance(observation) {
            rejected += 1;
        }
    }
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(rejected > 0, "fixture did not saturate the bounded queue");
    assert!(reporter.flush(Duration::from_secs(5)));

    let event_request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("event request after delayed error");
    let batch = PerformanceBatch::decode(&request_body(&event_request)["attributes"])
        .expect("performance batch");
    assert_eq!(batch.delivery().observations_dropped(), rejected);
}

#[test]
fn retries_once_and_reports_retry_accounting_in_the_delivered_batch() {
    let (endpoint, requests) = serve_responses(vec![
        (503, json!({"error": "temporary"})),
        (200, json!({"id": "EVT-retry"})),
    ]);
    let reporter = CanaryReporter::new(test_config(endpoint));
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .expect("marker");
    assert!(reporter.report_performance(marker.observation(9).expect("observation")));
    assert!(reporter.flush(Duration::from_secs(3)));

    let _first_attempt = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("first attempt");
    let second_attempt = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("retry attempt");
    let batch = PerformanceBatch::decode(&request_body(&second_attempt)["attributes"])
        .expect("retry batch");
    assert_eq!(batch.delivery().batches_sent(), 1);
    assert_eq!(batch.delivery().batches_retried(), 1);
    assert_eq!(batch.delivery().batches_dropped(), 0);
}

#[test]
fn timeline_readback_pages_and_merges_two_instances_exactly() {
    let (ingest_endpoint, ingest_requests) = serve_responses(vec![
        (200, json!({"id": "EVT-instance-1"})),
        (200, json!({"id": "EVT-instance-2"})),
    ]);
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .expect("marker");
    for duration_ms in [13, 260] {
        let reporter = CanaryReporter::new(test_config(ingest_endpoint.clone()));
        assert!(reporter.report_performance(
            marker
                .observation(duration_ms)
                .expect("instance observation")
        ));
        assert!(reporter.shutdown(Duration::from_secs(2)));
    }
    let mut attributes = Vec::new();
    for _ in 0..2 {
        attributes.push(
            request_body(
                &ingest_requests
                    .recv_timeout(Duration::from_secs(2))
                    .expect("ingest request"),
            )["attributes"]
                .clone(),
        );
    }
    // Freeze both independent boots into the same deterministic window even
    // when the test happens to straddle a wall-clock minute boundary.
    let window = attributes[0]["window"].clone();
    attributes[1]["window"] = window.clone();
    attributes[1]["snapshots"][0]["window"] = window;

    let mut wrong_schema = attributes[0].clone();
    wrong_schema["schema"] = json!("memory_engine.performance_batch.v2");
    assert!(PerformanceBatch::decode(&wrong_schema).is_err());
    let mut wrong_buckets = attributes[0].clone();
    wrong_buckets["snapshots"][0]["histogram"]["bucket_counts"]
        .as_array_mut()
        .expect("bucket array")
        .pop();
    assert!(PerformanceBatch::decode(&wrong_buckets).is_err());

    let event = |attributes: Value| {
        json!({
            "event_type": "telemetry.event",
            "signal_name": PERFORMANCE_EVENT_NAME,
            "service": "memory-engine-api",
            "attributes": attributes,
        })
    };
    let (read_endpoint, _) = serve_responses(vec![
        (
            200,
            json!({"events": [event(attributes.remove(0))], "cursor": "page-2"}),
        ),
        (
            200,
            json!({"events": [event(attributes.remove(0))], "cursor": null}),
        ),
    ]);
    let result = read_performance_timeline(
        &ReadbackConfig::new(read_endpoint, "read-key", "memory-engine-api", "1h").expect("config"),
    )
    .expect("readback");

    assert_eq!(result.pages(), 2);
    assert_eq!(result.batches(), 2);
    assert_eq!(result.snapshots().len(), 1);
    let snapshot = &result.snapshots()[0];
    assert_eq!(snapshot.count(), 2);
    assert_eq!(snapshot.sum_ms(), 273);
    assert_eq!(snapshot.max_ms(), 260);
    let p95 = snapshot.percentile_bounds(95).expect("p95");
    assert_eq!((p95.lower_ms, p95.upper_ms), (201, 300));
}

#[test]
fn strict_batch_decode_rejects_unknown_fields() {
    let mut attributes = json!({
        "schema": PERFORMANCE_BATCH_SCHEMA,
        "authority": "non_authoritative_debug",
        "namespace": "server",
        "window": {"start_minute": 1},
        "delivery": {
            "batches_sent": 0,
            "batches_retried": 0,
            "batches_dropped": 0,
            "observations_dropped": 0,
            "observations_invalid": 0,
            "series_dropped": 0
        },
        "snapshots": []
    });
    attributes["raw_path"] = json!("/app/submit");
    assert!(PerformanceBatch::decode(&attributes).is_err());
}

#[test]
fn timeline_readback_rejects_events_outside_service_authority() {
    let attributes = json!({
        "schema": PERFORMANCE_BATCH_SCHEMA,
        "authority": "non_authoritative_debug",
        "namespace": "server",
        "window": {"start_minute": 1},
        "delivery": {
            "batches_sent": 0,
            "batches_retried": 0,
            "batches_dropped": 0,
            "observations_dropped": 0,
            "observations_invalid": 0,
            "series_dropped": 0
        },
        "snapshots": []
    });
    let (endpoint, _) = serve_responses(vec![(
        200,
        json!({
            "events": [{
                "event_type": "telemetry.event",
                "signal_name": PERFORMANCE_EVENT_NAME,
                "service": "another-service",
                "attributes": attributes
            }],
            "cursor": null
        }),
    )]);
    let config =
        ReadbackConfig::new(endpoint, "read-key", "memory-engine-api", "1h").expect("config");
    assert!(read_performance_timeline(&config).is_err());
}

#[test]
fn shutdown_flushes_once_and_rejects_new_work() {
    let (endpoint, requests) = serve_responses(vec![(200, json!({"id": "EVT-1"}))]);
    let reporter = CanaryReporter::new(test_config(endpoint));
    let marker = CompletionMarker::server(
        Action::Machine(MachineRouteAction::OpenApi),
        CompletionPhase::ImmediateAck,
        Outcome::Succeeded,
    )
    .expect("marker");
    let observation = marker.observation(5).expect("observation");
    assert!(reporter.report_performance(observation));
    assert!(reporter.shutdown(Duration::from_secs(2)));
    requests
        .recv_timeout(Duration::from_secs(2))
        .expect("shutdown batch");
    assert!(!reporter.report_performance(observation));
    assert!(reporter.shutdown(Duration::from_millis(10)));
}

fn test_config(endpoint: String) -> CanaryConfig {
    let mut config =
        CanaryConfig::from_parts(Some(endpoint), Some("sk_test_key".to_owned())).expect("config");
    "test".clone_into(&mut config.environment);
    config
}

fn request_body(request: &str) -> Value {
    serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json")
}

fn serve_responses(responses: Vec<(u16, Value)>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream);
            let _ = sender.send(request);
            write_response(&mut stream, status, &body);
        }
    });
    (format!("http://{address}"), receiver)
}

fn serve_delayed_error_then_event() -> (String, mpsc::Receiver<()>, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (accepted_sender, accepted_receiver) = mpsc::channel();
    let (request_sender, request_receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut error_stream, _) = listener.accept().expect("error accept");
        let _ = read_request(&mut error_stream);
        accepted_sender.send(()).expect("accepted signal");
        thread::sleep(Duration::from_millis(750));
        write_response(&mut error_stream, 200, &json!({"id": "ERR-1"}));

        let (mut event_stream, _) = listener.accept().expect("event accept");
        let request = read_request(&mut event_stream);
        request_sender.send(request).expect("event request");
        write_response(&mut event_stream, 200, &json!({"id": "EVT-1"}));
    });
    (
        format!("http://{address}"),
        accepted_receiver,
        request_receiver,
    )
}

fn read_request(stream: &mut TcpStream) -> String {
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
    String::from_utf8_lossy(&request).into_owned()
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("write");
}
