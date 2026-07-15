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
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Resend");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        (0..expected_requests)
            .map(|_| {
                let (mut stream, _) = listener.accept().expect("accept Resend request");
                let request = read_http_request(&mut stream);
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                    )
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

fn run_mailer(resend_api_url: &str, reminder_key: Option<&str>, magic_link: bool) -> Output {
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
            .env("MEMORY_ENGINE_AUTH_EMAIL", "learner@example.test")
            .env(
                "MEMORY_ENGINE_AUTH_LINK",
                "/app/login/verify?token=magic-test",
            );
    } else {
        command
            .env(
                "MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL",
                "learner@example.test",
            )
            .env("MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT", "3")
            .env(
                "MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE",
                "/app/return-notifications?token=unsubscribe-test",
            );
    }
    command.output().expect("run bundled mailer")
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
}
