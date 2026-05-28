//! Rust HTTP host for the beta-study app.
//!
//! The host owns HTTP parsing, static HTML delivery, request validation, and
//! status-code mapping. Learning workflow state stays in `memory-engine-study`.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
};

use memory_engine_study::{
    BetaStudyOptions, BetaStudySession, BetaStudySourceInput, BetaStudyView,
};
use serde::{Deserialize, Serialize};

const INDEX_HTML: &str = include_str!("../../../experiments/beta-study/index.html");

#[derive(Clone, Debug)]
pub struct BetaAppConfig {
    pub address: String,
    pub study: BetaStudyOptions,
}

#[derive(Debug)]
pub enum BetaAppError {
    Io(io::Error),
    Study(memory_engine_study::BetaStudyError),
}

impl fmt::Display for BetaAppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Study(error) => write!(formatter, "study error: {error}"),
        }
    }
}

impl Error for BetaAppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Study(error) => Some(error),
        }
    }
}

impl From<io::Error> for BetaAppError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<memory_engine_study::BetaStudyError> for BetaAppError {
    fn from(error: memory_engine_study::BetaStudyError) -> Self {
        Self::Study(error)
    }
}

/// Run the blocking beta-study HTTP server.
///
/// # Errors
///
/// Returns [`BetaAppError`] when the socket cannot bind, the study session
/// cannot open, or a connection write fails.
pub fn serve(config: BetaAppConfig) -> Result<(), BetaAppError> {
    let listener = TcpListener::bind(&config.address)?;
    let mut session = BetaStudySession::open(config.study)?;
    let _ = session.start()?;

    for stream in listener.incoming() {
        let mut stream = stream?;
        handle_stream(&mut session, &mut stream)?;
    }

    Ok(())
}

fn handle_stream(
    session: &mut BetaStudySession,
    stream: &mut TcpStream,
) -> Result<(), BetaAppError> {
    let request = HttpRequest::read_from(stream)?;
    let response = route(session, &request);
    response.write_to(stream)?;

    Ok(())
}

fn route(session: &mut BetaStudySession, request: &HttpRequest) -> HttpResponse {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => HttpResponse::html(INDEX_HTML),
        ("GET", "/state") => view_response(session.view()),
        ("POST", "/source") => match read_source(&request.body) {
            Ok(source) => view_response(session.add_source(source)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/generate") => view_response(session.generate(None)),
        ("POST", "/approve") => match read_required_string(&request.body, "draftId") {
            Ok(draft_id) => view_response(session.approve_draft(&draft_id)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/reveal") => view_response(session.reveal()),
        ("POST", "/answer") => match read_answer(&request.body) {
            Ok(answer) => {
                view_response(session.submit_answer(answer.answer, answer.response_time_ms))
            }
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/next") => view_response(session.advance()),
        _ => HttpResponse::plain(404, "Not found"),
    }
}

fn view_response(
    result: Result<BetaStudyView, memory_engine_study::BetaStudyError>,
) -> HttpResponse {
    match result {
        Ok(view) => HttpResponse::json(200, &view),
        Err(error) => HttpResponse::plain(500, &error.to_string()),
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

impl HttpRequest {
    fn read_from(stream: &mut TcpStream) -> io::Result<Self> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        let header_end;
        loop {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed before headers",
                ));
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = find_header_end(&bytes) {
                header_end = index;
                break;
            }
        }

        let header_text = std::str::from_utf8(&bytes[..header_end]).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid headers: {error}"),
            )
        })?;
        let mut lines = header_text.split("\r\n");
        let request_line = lines
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
            .to_owned();
        let target = request_parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?;
        let path = target.split('?').next().unwrap_or(target).to_owned();
        let content_length = lines
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);

        let body_start = header_end + 4;
        while bytes.len() < body_start + content_length {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed before body",
                ));
            }
            bytes.extend_from_slice(&buffer[..read]);
        }

        Ok(Self {
            method,
            path,
            body: bytes[body_start..body_start + content_length].to_vec(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl HttpResponse {
    fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn json<T: Serialize>(status: u16, value: &T) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(value).expect("serializable response"),
        }
    }

    fn bad_request(message: &str) -> Self {
        Self::plain(400, message)
    }

    fn plain(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn write_to(&self, stream: &mut TcpStream) -> io::Result<()> {
        let reason = reason_phrase(self.status);
        write!(
            stream,
            "HTTP/1.1 {} {reason}\r\ncontent-type: {}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
            self.status,
            self.content_type,
            self.body.len()
        )?;
        stream.write_all(&self.body)?;
        stream.flush()
    }
}

#[derive(Deserialize)]
struct SourcePayload {
    id: String,
    title: String,
    body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerPayload {
    answer: String,
    response_time_ms: u32,
}

fn read_source(body: &[u8]) -> Result<BetaStudySourceInput, String> {
    let payload: SourcePayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be a source object: {error}"))?;
    require_non_blank(&payload.id, "id")?;
    require_non_blank(&payload.title, "title")?;
    require_non_blank(&payload.body, "body")?;

    Ok(BetaStudySourceInput {
        id: payload.id,
        title: payload.title,
        body: payload.body,
    })
}

fn read_answer(body: &[u8]) -> Result<AnswerPayload, String> {
    let payload: AnswerPayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an answer object: {error}"))?;
    require_non_blank(&payload.answer, "answer")?;
    if payload.response_time_ms == 0 {
        return Err("responseTimeMs must be a positive integer".to_owned());
    }

    Ok(payload)
}

fn read_required_string(body: &[u8], key: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an object: {error}"))?;
    let value = value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{key} must be a non-empty string"))?
        .to_owned();
    require_non_blank(&value, key)?;

    Ok(value)
}

fn require_non_blank(value: &str, key: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{key} must be a non-empty string"))
    } else {
        Ok(())
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::{json, Value};

    use super::{route, BetaStudyOptions, BetaStudySession, HttpRequest};

    const NOW: i64 = 1_779_984_000_000;

    #[test]
    fn serves_html_and_state_from_the_rust_session() {
        let directory = TempDirectory::new("state");
        let mut session = session(directory.path().join("study.json"));

        let html = route(&mut session, &request("GET", "/", ""));
        assert_eq!(html.status, 200);
        assert_eq!(html.content_type, "text/html; charset=utf-8");
        assert!(String::from_utf8(html.body)
            .expect("html")
            .contains("Memory Engine Beta Study"));

        let state = route(&mut session, &request("GET", "/state", ""));
        assert_eq!(state.status, 200);
        let encoded: Value = serde_json::from_slice(&state.body).expect("state json");
        assert_eq!(encoded["status"], json!("drafting"));
        assert_eq!(encoded["summary"]["sourceCount"], json!(0));
    }

    #[test]
    fn drives_the_mobile_beta_http_flow_through_rust() {
        let directory = TempDirectory::new("flow");
        let mut session = session(directory.path().join("study.json"));

        let source = route(
            &mut session,
            &request(
                "POST",
                "/source",
                &json!({
                    "id": "src-nato",
                    "title": "NATO practice notes",
                    "body": source_body()
                })
                .to_string(),
            ),
        );
        assert_eq!(source.status, 200);

        let generated = route(&mut session, &request("POST", "/generate", "{}"));
        let generated: Value = serde_json::from_slice(&generated.body).expect("generated");
        assert_eq!(
            generated["drafts"][0]["id"],
            json!("study-run-1-draft-src-nato-1-nato-letter-a")
        );

        let approved = route(
            &mut session,
            &request(
                "POST",
                "/approve",
                &json!({"draftId": "study-run-1-draft-src-nato-1-nato-letter-a"}).to_string(),
            ),
        );
        let approved: Value = serde_json::from_slice(&approved.body).expect("approved");
        assert_eq!(approved["status"], json!("answering"));

        let revealed = route(&mut session, &request("POST", "/reveal", "{}"));
        let revealed: Value = serde_json::from_slice(&revealed.body).expect("revealed");
        assert_eq!(revealed["current"]["expectedAnswer"], json!("ALFA"));

        let answered = route(
            &mut session,
            &request(
                "POST",
                "/answer",
                &json!({"answer": "ALFA", "responseTimeMs": 1800}).to_string(),
            ),
        );
        let answered: Value = serde_json::from_slice(&answered.body).expect("answered");
        assert_eq!(answered["status"], json!("graded"));
        assert_eq!(answered["current"]["grade"]["verdict"], json!("correct"));
        assert_eq!(answered["summary"]["attemptCount"], json!(1));
    }

    #[test]
    fn rejects_malformed_http_payloads_before_touching_the_session() {
        let directory = TempDirectory::new("bad-request");
        let mut session = session(directory.path().join("study.json"));

        let response = route(
            &mut session,
            &request("POST", "/source", r#"{"id":"","title":"x","body":"y"}"#),
        );

        assert_eq!(response.status, 400);
        assert!(String::from_utf8(response.body)
            .expect("body")
            .contains("id must be"));
        assert_eq!(session.view().expect("view").summary.source_count, 0);
    }

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn session(path: PathBuf) -> BetaStudySession {
        let mut session =
            BetaStudySession::open(BetaStudyOptions::new(path).with_clock(now)).expect("open");
        session.start().expect("start");
        session
    }

    fn now() -> i64 {
        NOW
    }

    fn source_body() -> String {
        [
            "Concept: NATO letter A",
            "Activity: quiz",
            "Stage: recognition-3",
            "Question: What is the NATO phonetic alphabet word for A?",
            "Answer: ALFA",
            "Distractors: BRAVO, CHARLIE",
            "Reference: The NATO phonetic alphabet word for A is ALFA.",
        ]
        .join("\n")
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "memory-engine-rust-beta-app-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp directory");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
