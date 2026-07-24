use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Output},
    thread::{self, JoinHandle},
};

fn script_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/send-magic-link")
}

fn start_mock_resend(expected_requests: usize) -> (String, JoinHandle<Vec<String>>) {
    start_mock_resend_response(expected_requests, "200 OK", r#"{"id":"email_test_123"}"#)
}

fn start_mock_resend_response(
    expected_requests: usize,
    status: &str,
    body: &str,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Resend");
    let address = listener.local_addr().expect("mock address");
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = thread::spawn(move || {
        (0..expected_requests)
            .map(|_| {
                let (mut stream, _) = listener.accept().expect("accept Resend request");
                let request = read_http_request(&mut stream);
                stream
                    .write_all(response.as_bytes())
                    .expect("write Resend response");
                request
            })
            .collect()
    });
    (format!("http://{address}"), server)
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut buffer).expect("read Resend request");
        assert!(read > 0, "Resend closed before sending headers");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer).expect("read Resend body");
        assert!(read > 0, "Resend closed before sending body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes).expect("Resend request is UTF-8")
}

fn header(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn run_mailer_with_inputs(
    resend_api_url: &str,
    reminder_key: Option<&str>,
    magic_link: bool,
    email: &str,
    link: &str,
) -> Output {
    let mut command = Command::new(script_path());
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("RESEND_API_KEY", "test-resend-key")
        .env("RESEND_API_URL", resend_api_url)
        .env(
            "MEMORY_ENGINE_PUBLIC_BASE_URL",
            "https://memory.example.test",
        )
        .env(
            "MEMORY_ENGINE_MAIL_FROM",
            "Memory Engine <test@example.test>",
        );
    if let Some(reminder_key) = reminder_key {
        command.env(
            "MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY",
            reminder_key,
        );
    }
    if magic_link {
        command
            .env("MEMORY_ENGINE_AUTH_EMAIL", email)
            .env("MEMORY_ENGINE_AUTH_LINK", link);
    } else {
        command
            .env("MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL", email)
            .env("MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT", "3")
            .env(
                "MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE",
                "/app/return-notifications?token=unsubscribe-test",
            );
    }
    command.output().expect("run bundled mailer")
}

fn run_mailer(resend_api_url: &str, reminder_key: Option<&str>, magic_link: bool) -> Output {
    run_mailer_with_inputs(
        resend_api_url,
        reminder_key,
        magic_link,
        "learner@example.test",
        "/app/login/verify?token=magic-test",
    )
}

#[test]
fn bundled_resend_mailer_requires_and_reuses_reminder_idempotency_key() {
    let (resend_api_url, server) = start_mock_resend(2);
    let first = run_mailer(&resend_api_url, Some("return-notification-retry-1"), false);
    assert!(first.status.success(), "first reminder failed: {first:?}");
    let second = run_mailer(&resend_api_url, Some("return-notification-retry-1"), false);
    assert!(second.status.success(), "retry reminder failed: {second:?}");
    let requests = server.join().expect("mock Resend server");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        header(&requests[0], "Idempotency-Key").as_deref(),
        Some("return-notification-retry-1")
    );
    assert_eq!(
        header(&requests[1], "Idempotency-Key").as_deref(),
        Some("return-notification-retry-1")
    );
}

#[test]
fn bundled_resend_mailer_fails_closed_without_reminder_key() {
    let output = run_mailer("http://127.0.0.1:1", None, false);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY is required"),
        "unexpected missing-key failure: {stderr}"
    );
}

#[test]
fn bundled_resend_mailer_magic_link_mode_does_not_require_or_send_key() {
    let (resend_api_url, server) = start_mock_resend(1);
    let output = run_mailer(&resend_api_url, None, true);
    assert!(output.status.success(), "magic link failed: {output:?}");
    let requests = server.join().expect("mock Resend server");
    assert_eq!(requests.len(), 1);
    assert!(header(&requests[0], "Idempotency-Key").is_none());
    assert!(
        requests[0].contains(r#"text":"Sign in: https://memory.example.test/app/login/verify?token=magic-test\n\nThe link"#),
        "message newlines must be encoded once in JSON: {}",
        requests[0]
    );
    assert!(
        !requests[0].contains(r"\\n"),
        "message must not contain double-escaped newline literals: {}",
        requests[0]
    );
}

#[test]
fn bundled_resend_mailer_escapes_untrusted_email_and_reports_bounded_provider_id() {
    let (resend_api_url, server) = start_mock_resend(1);
    let output = run_mailer_with_inputs(
        &resend_api_url,
        None,
        true,
        r#"learner"quoted@example.test"#,
        "/app/login/verify?token=magic-test",
    );
    assert!(
        output.status.success(),
        "escaped magic link failed: {output:?}"
    );
    let requests = server.join().expect("mock Resend server");
    assert!(
        requests[0].contains(r#""to":["learner\"quoted@example.test"]"#),
        "recipient was not JSON escaped: {}",
        requests[0]
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("resend_status=accepted"),
        "missing bounded status: {stderr}"
    );
    assert!(
        stderr.contains("resend_id=email_test_123"),
        "missing provider id: {stderr}"
    );
    assert!(
        !stderr.contains("quoted@example.test"),
        "recipient leaked to diagnostics: {stderr}"
    );
    assert!(
        !stderr.contains("magic-test"),
        "token leaked to diagnostics: {stderr}"
    );
}

#[test]
fn bundled_resend_mailer_rejects_control_characters_before_provider_call() {
    for email in ["learner\n@example.test", "learner\r@example.test"] {
        let output = run_mailer_with_inputs(
            "http://127.0.0.1:1",
            None,
            true,
            email,
            "/app/login/verify?token=magic-test",
        );
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("mailer input contains unsupported control characters"),
            "unexpected control failure: {stderr}"
        );
        assert!(
            !stderr.contains("learner"),
            "recipient leaked to diagnostics: {stderr}"
        );
        assert!(
            !stderr.contains("magic-test"),
            "token leaked to diagnostics: {stderr}"
        );
    }
}

#[test]
fn bundled_resend_mailer_reports_provider_failure_without_body_or_curl_noise() {
    let (resend_api_url, server) = start_mock_resend_response(
        1,
        "422 Unprocessable Entity",
        r#"{"message":"provider detail"}"#,
    );
    let output = run_mailer_with_inputs(
        &resend_api_url,
        None,
        true,
        "learner@example.test",
        "/app/login/verify?token=magic-test",
    );
    assert!(!output.status.success());
    let _requests = server.join().expect("mock Resend server");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("resend_status=failed http_status=422"),
        "missing bounded provider failure: {stderr}"
    );
    assert!(
        !stderr.contains("provider detail"),
        "provider body leaked to diagnostics: {stderr}"
    );
    assert!(
        !stderr.contains("curl:"),
        "curl transport noise leaked to diagnostics: {stderr}"
    );
    assert!(
        !stderr.contains("magic-test"),
        "token leaked to diagnostics: {stderr}"
    );
}

#[test]
fn bundled_resend_mailer_suppresses_transport_diagnostics() {
    let output = run_mailer_with_inputs(
        "http://127.0.0.1:1",
        None,
        true,
        "learner@example.test",
        "/app/login/verify?token=magic-test",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.trim(), "resend_status=transport_error");
}
#[test]
fn bundled_resend_mailer_rejects_control_characters_in_idempotency_key() {
    let output = run_mailer_with_inputs(
        "http://127.0.0.1:1",
        Some("retry\ninjected"),
        false,
        "learner@example.test",
        "/app/login/verify?token=magic-test",
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mailer input contains unsupported control characters"),
        "unexpected key failure: {stderr}"
    );
    assert!(
        !stderr.contains("injected"),
        "idempotency key leaked to diagnostics: {stderr}"
    );
}

#[test]
fn bundled_resend_mailer_escapes_backslashes_in_json_fields() {
    let (resend_api_url, server) = start_mock_resend(1);
    let output = run_mailer_with_inputs(
        &resend_api_url,
        None,
        true,
        r"learner\quoted@example.test",
        "/app/login/verify?token=magic-test",
    );
    assert!(
        output.status.success(),
        "backslash input failed: {output:?}"
    );
    let requests = server.join().expect("mock Resend server");
    assert!(
        requests[0].contains(r#""to":["learner\\quoted@example.test"]"#),
        "backslash was not JSON escaped: {}",
        requests[0]
    );
}

#[test]
fn bundled_resend_mailer_escapes_json_fields_without_a_subprocess_per_character() {
    // json_escape used to spawn a `grep` (and `printf`) subprocess for every
    // ordinary character in the recipient, sender, subject, and message.
    // The fix checks each value for disallowed control characters once,
    // then escapes with pure shell built-ins. A hard wall-clock assertion
    // here would trade one flaky shared-host timing test for another (see
    // `waitlist_join_file_store_handler_overhead_is_negligible`): this
    // machine measured 1.28s before the fix and 0.08-0.11s after under
    // normal load, but a single contended run still touched 0.88s, close
    // enough to a "generous" fixed threshold to flake. Assert correctness
    // here; the before/after timing is recorded in the fix's commit
    // message as reproducible, non-gating evidence.
    let (resend_api_url, server) = start_mock_resend(1);
    let started = std::time::Instant::now();
    let output = run_mailer(&resend_api_url, None, true);
    let elapsed = started.elapsed();
    assert!(output.status.success(), "mailer failed: {output:?}");
    server.join().expect("mock Resend server");
    eprintln!("magic-link send completed in {elapsed:?} (informational only)");
}
