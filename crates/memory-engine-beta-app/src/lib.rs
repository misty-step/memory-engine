//! Rust HTTP host for the beta-study app.
//!
//! The host owns HTTP parsing, server-rendered HTML delivery, request
//! validation, and status-code mapping. Learning workflow state stays in
//! `memory-engine-study`.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
};

use memory_engine_generation::{FakeModelProvider, FallbackProvider, StructuredBlockProvider};
use memory_engine_study::{
    infer_capture_title, BetaStudyCurrent, BetaStudyOptions, BetaStudySession,
    BetaStudySourceInput, BetaStudyView,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct BetaAppConfig {
    pub address: String,
    pub study: BetaStudyOptions,
}

/// Ceiling for a plausible single-answer response time (ten minutes).
///
/// A local host cannot verify a client-reported duration. Missing, malformed,
/// non-positive, and implausibly large values therefore take the slow path so
/// they can never manufacture the fast-answer `Easy` rating.
const MAX_PLAUSIBLE_RESPONSE_TIME_MS: u32 = 600_000;

const HONEST_TIMING_SCRIPT: &str = r#"<script>
(function () {
  "use strict";
  var timingInput = document.querySelector('input[name="responseTimeMs"]');
  if (!timingInput) return;
  var monotonic = window.performance && typeof window.performance.now === "function";
  var now = function () { return monotonic ? window.performance.now() : Date.now(); };
  var shownAt = now();
  document.addEventListener("submit", function (event) {
    var form = event.target;
    if (!form || !form.querySelector) return;
    var input = form.querySelector('input[name="responseTimeMs"]');
    if (!input) return;
    var elapsed = now() - shownAt;
    input.value = String(Math.max(1, Math.round(elapsed)));
  });
})();
</script>"#;

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

    serve_connections(&listener, &mut session, None)
}

fn serve_connections(
    listener: &TcpListener,
    session: &mut BetaStudySession,
    max_connections: Option<usize>,
) -> Result<(), BetaAppError> {
    for (handled, stream) in listener.incoming().enumerate() {
        let mut stream = stream?;
        handle_stream(session, &mut stream)?;
        if max_connections.is_some_and(|max| handled + 1 >= max) {
            break;
        }
    }

    Ok(())
}

fn handle_stream(
    session: &mut BetaStudySession,
    stream: &mut TcpStream,
) -> Result<(), BetaAppError> {
    let request = match HttpRequest::read_from(stream) {
        Ok(request) => request,
        Err(error) if is_ignorable_client_error(&error) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let response = route(session, &request);
    response.write_to(stream)?;

    Ok(())
}

fn is_ignorable_client_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
    )
}

fn route(session: &mut BetaStudySession, request: &HttpRequest) -> HttpResponse {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => page_response(session.view()),
        ("GET", "/state") => view_response(session.view()),
        ("POST", "/source") => match read_source(&request.body) {
            Ok(source) => response_for(request, session.add_source(source)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/generate") => response_for(request, generate_all_sources(session)),
        ("POST", "/approve") => match read_required_string(&request.body, "draftId") {
            Ok(draft_id) => response_for(request, session.approve_draft(&draft_id)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/learn-more" | "/current/learn-more") => {
            response_for(request, session.learn_more())
        }
        ("POST", "/edit" | "/current/edit") => match read_revision(&request.body) {
            Ok(revision) => response_for(
                request,
                session.edit_current_prompt(revision.prompt, revision.expected_answer),
            ),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/delete" | "/current/delete") => response_for(request, session.archive_current()),
        ("POST", "/snooze" | "/current/snooze") => match read_i64(&request.body, "snoozedUntil") {
            Ok(snoozed_until) => response_for(request, session.snooze_current_until(snoozed_until)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/reveal") => response_for(request, session.reveal()),
        ("POST", "/answer") => match read_answer(&request.body) {
            Ok(answer) => response_for(
                request,
                session.submit_answer(
                    answer.answer,
                    sanitize_json_response_time_ms(answer.response_time_ms.as_ref()),
                ),
            ),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/next") => response_for(request, session.advance()),
        _ => HttpResponse::plain(404, "Not found"),
    }
}

fn generate_all_sources(
    session: &mut BetaStudySession,
) -> Result<BetaStudyView, memory_engine_study::BetaStudyError> {
    let structured = StructuredBlockProvider;
    let model = FakeModelProvider;
    let provider = FallbackProvider::new(&structured, &model);
    session.generate_with_provider(None, &provider)
}

fn response_for(
    request: &HttpRequest,
    result: Result<BetaStudyView, memory_engine_study::BetaStudyError>,
) -> HttpResponse {
    if request.is_form_post() {
        page_response(result)
    } else {
        view_response(result)
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

fn page_response(
    result: Result<BetaStudyView, memory_engine_study::BetaStudyError>,
) -> HttpResponse {
    match result {
        Ok(view) => HttpResponse::html(&render_page(&view, None)),
        Err(error) => HttpResponse::html(&render_page_error(&error.to_string())),
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    content_type: Option<String>,
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
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .collect::<Vec<_>>();
        let content_type = headers.iter().find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_owned())
        });
        let content_length = headers
            .iter()
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
            content_type,
            body: bytes[body_start..body_start + content_length].to_vec(),
        })
    }

    fn is_form_post(&self) -> bool {
        self.method == "POST"
            && self.content_type.as_deref().is_some_and(|content_type| {
                content_type
                    .split(';')
                    .next()
                    .is_some_and(|value| value.trim() == "application/x-www-form-urlencoded")
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
    id: Option<String>,
    title: Option<String>,
    body: Option<String>,
    capture: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerPayload {
    answer: String,
    response_time_ms: Option<serde_json::Value>,
}

struct RevisionPayload {
    prompt: String,
    expected_answer: String,
}

fn read_source(body: &[u8]) -> Result<BetaStudySourceInput, String> {
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        let body = form_optional(&fields, "capture")
            .or_else(|| form_optional(&fields, "body"))
            .ok_or_else(|| "capture must be a non-empty string".to_owned())?;
        let title = form_optional(&fields, "title").unwrap_or_default();
        let id = fields
            .iter()
            .find_map(|(key, value)| {
                (key == "id" && !value.trim().is_empty()).then(|| value.clone())
            })
            .unwrap_or_else(|| {
                format!("source-{}", slug_fragment(&source_slug_text(&title, &body)))
            });

        return Ok(BetaStudySourceInput {
            id,
            title,
            body,
            project_key: None,
            ttl_expires_at: None,
        });
    }

    let payload: SourcePayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be a source object: {error}"))?;
    let body = payload
        .capture
        .or(payload.body)
        .ok_or_else(|| "capture must be a non-empty string".to_owned())?;
    require_non_blank(&body, "capture")?;
    let title = payload.title.unwrap_or_default();
    let id = match payload.id {
        Some(id) => {
            require_non_blank(&id, "id")?;
            id
        }
        None => format!("source-{}", slug_fragment(&source_slug_text(&title, &body))),
    };

    Ok(BetaStudySourceInput {
        id,
        title,
        body,
        project_key: None,
        ttl_expires_at: None,
    })
}

fn read_answer(body: &[u8]) -> Result<AnswerPayload, String> {
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        let answer = form_required(&fields, "answer")?;
        let response_time_ms = fields
            .iter()
            .find_map(|(key, value)| (key == "responseTimeMs").then_some(value))
            .and_then(|value| value.trim().parse::<u32>().ok());
        return Ok(AnswerPayload {
            answer,
            response_time_ms: response_time_ms.map(serde_json::Value::from),
        });
    }

    let payload: AnswerPayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an answer object: {error}"))?;
    require_non_blank(&payload.answer, "answer")?;
    Ok(payload)
}

fn sanitize_response_time_ms(raw: Option<u32>) -> u32 {
    raw.filter(|&elapsed| elapsed > 0)
        .map_or(MAX_PLAUSIBLE_RESPONSE_TIME_MS, |elapsed| {
            elapsed.min(MAX_PLAUSIBLE_RESPONSE_TIME_MS)
        })
}

fn sanitize_json_response_time_ms(raw: Option<&serde_json::Value>) -> u32 {
    let elapsed = raw.and_then(|value| match value {
        serde_json::Value::String(value) => value.trim().parse::<u32>().ok(),
        serde_json::Value::Number(value) => {
            value.as_u64().and_then(|value| u32::try_from(value).ok())
        }
        _ => None,
    });
    sanitize_response_time_ms(elapsed)
}

fn read_required_string(body: &[u8], key: &str) -> Result<String, String> {
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        return form_required(&fields, key);
    }

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

fn read_revision(body: &[u8]) -> Result<RevisionPayload, String> {
    Ok(RevisionPayload {
        prompt: read_required_string(body, "prompt")?,
        expected_answer: read_required_string(body, "expectedAnswer")?,
    })
}

fn read_i64(body: &[u8], key: &str) -> Result<i64, String> {
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        let value = form_required(&fields, key)?;
        return value
            .parse::<i64>()
            .map_err(|_| format!("{key} must be an integer"));
    }

    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an object: {error}"))?;
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| format!("{key} must be an integer"))
}

fn looks_like_json(body: &[u8]) -> bool {
    body.iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| byte == b'{')
}

fn parse_form(body: &[u8]) -> Result<Vec<(String, String)>, String> {
    let text = std::str::from_utf8(body).map_err(|error| format!("invalid form body: {error}"))?;
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Ok((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn form_required(fields: &[(String, String)], key: &str) -> Result<String, String> {
    let value =
        form_optional(fields, key).ok_or_else(|| format!("{key} must be a non-empty string"))?;
    Ok(value)
}

fn form_optional(fields: &[(String, String)], key: &str) -> Option<String> {
    fields
        .iter()
        .find_map(|(field_key, value)| (field_key == key).then(|| value.clone()))
        .filter(|value| !value.trim().is_empty())
}

fn source_slug_text(title: &str, body: &str) -> String {
    if title.trim().is_empty() {
        infer_capture_title(body)
    } else {
        title.to_owned()
    }
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .map_err(|error| format!("invalid percent encoding: {error}"))?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("invalid percent encoding: %{hex}"))?;
                decoded.push(byte);
                index += 3;
            }
            b'%' => return Err("truncated percent encoding".to_owned()),
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|error| format!("invalid utf-8 form value: {error}"))
}

fn slug_fragment(value: &str) -> String {
    let slug = value
        .chars()
        .filter_map(|character| {
            character
                .is_ascii_alphanumeric()
                .then(|| character.to_ascii_lowercase())
                .or_else(|| character.is_ascii_whitespace().then_some('-'))
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    if slug.is_empty() {
        "manual".to_owned()
    } else {
        slug
    }
}

fn render_page_error(message: &str) -> String {
    format!(
        "{}<main><section><h1>Study error</h1><p>{}</p></section></main></body></html>",
        page_head("Memory Engine Beta Study"),
        escape_html(message)
    )
}

fn render_page(view: &BetaStudyView, error: Option<&str>) -> String {
    let mut html = page_head("Memory Engine Beta Study");
    html.push_str("<main><section class=\"study\"><header><strong>Beta Study</strong><span>");
    html.push_str(&escape_html(&format!("{:?}", view.status)).to_lowercase());
    html.push_str("</span></header>");
    render_current(&mut html, view.current.as_ref(), error);
    html.push_str("</section><aside>");
    render_summary(&mut html, view);
    render_generation_notices(&mut html, view);
    render_source_form(&mut html);
    render_drafts(&mut html, view);
    render_queue(&mut html, view);
    html.push_str("</aside></main>");
    if view.current.is_some() {
        html.push_str(HONEST_TIMING_SCRIPT);
    }
    html.push_str("</body></html>");
    html
}

fn page_head(title: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{}</style></head><body>",
        escape_html(title),
        CSS
    )
}

fn render_current(html: &mut String, current: Option<&BetaStudyCurrent>, error: Option<&str>) {
    html.push_str("<div class=\"prompt\"><div class=\"kind\"><span>");
    html.push_str(&current.map_or_else(
        || "activity".to_owned(),
        |current| escape_html(&format!("{:?}", current.activity_kind)),
    ));
    html.push_str("</span><span>");
    html.push_str(&current.map_or_else(
        || "stage".to_owned(),
        |current| escape_html(&current.activity_stage),
    ));
    html.push_str("</span></div><h1>");
    html.push_str(&current.map_or_else(
        || "Add source material to begin.".to_owned(),
        |current| escape_html(&current.prompt),
    ));
    html.push_str("</h1>");
    if let Some(error) = error {
        html.push_str("<p class=\"grade\">");
        html.push_str(&escape_html(error));
        html.push_str("</p>");
    }
    if let Some(current) = current {
        render_choices(html, current);
        html.push_str("<form method=\"post\" action=\"/answer\"><label for=\"answer\">Answer or worked solution</label><textarea id=\"answer\" name=\"answer\" autocomplete=\"off\" spellcheck=\"false\"></textarea><input type=\"hidden\" name=\"responseTimeMs\" value=\"\"><div class=\"actions\"><button type=\"submit\">Submit</button></form><form method=\"post\" action=\"/reveal\"><button type=\"submit\" class=\"secondary\">Reveal</button></form><form method=\"post\" action=\"/current/learn-more\"><button type=\"submit\" class=\"secondary\">Learn more</button></form><form method=\"post\" action=\"/current/snooze\"><input type=\"hidden\" name=\"snoozedUntil\" value=\"");
        html.push_str(&snooze_until().to_string());
        html.push_str("\"><button type=\"submit\" class=\"secondary\">Snooze</button></form><form method=\"post\" action=\"/current/delete\"><button type=\"submit\" class=\"secondary danger\">Delete</button></form><form method=\"post\" action=\"/next\"><button type=\"submit\" class=\"secondary\">Next</button></form></div>");
        html.push_str("<form class=\"edit\" method=\"post\" action=\"/current/edit\"><label for=\"prompt-edit\">Edit prompt</label><textarea id=\"prompt-edit\" name=\"prompt\">");
        html.push_str(&escape_html(&current.prompt));
        html.push_str("</textarea><label for=\"answer-edit\">Edit expected answer</label><input id=\"answer-edit\" name=\"expectedAnswer\" value=\"");
        html.push_str(&escape_html(&current.revision_expected_answer));
        html.push_str("\"><button type=\"submit\" class=\"secondary\">Save prompt</button></form>");
        if let Some(reference_text) = &current.reference_text {
            html.push_str("<div class=\"reference\">");
            html.push_str(&escape_html(reference_text));
            html.push_str("</div>");
        }
        if let Some(expected) = &current.expected_answer {
            html.push_str("<div class=\"answer\">");
            html.push_str(&escape_html(expected));
            html.push_str("</div>");
        }
        if let Some(solution) = &current.worked_solution {
            html.push_str("<div class=\"solution\">");
            html.push_str(&escape_html(solution));
            html.push_str("</div>");
        }
        if let Some(grade) = &current.grade {
            html.push_str("<div class=\"grade\">");
            html.push_str(&escape_html(&format!(
                "{:?} rating {:?}",
                grade.verdict, grade.rating
            )));
            html.push_str("</div>");
        }
        render_feedback(html, current);
    }
    html.push_str("</div>");
}

fn render_choices(html: &mut String, current: &BetaStudyCurrent) {
    if current.choices.is_empty() {
        return;
    }
    html.push_str("<ol class=\"choices\">");
    for choice in &current.choices {
        html.push_str("<li>");
        html.push_str(&escape_html(choice));
        html.push_str("</li>");
    }
    html.push_str("</ol>");
}

fn render_feedback(html: &mut String, current: &BetaStudyCurrent) {
    let Some(feedback) = &current.feedback else {
        return;
    };
    let item = &feedback.item_history;
    html.push_str("<div class=\"feedback\"><strong>Answer feedback</strong><p>");
    html.push_str(&escape_html(&format!(
        "{}; {}; {}; response time {}",
        item.success_rate, item.trend, item.last_seen_summary, item.response_time_trend
    )));
    html.push_str("</p>");
    if let Some(concept) = &feedback.concept_progress {
        html.push_str("<p>");
        html.push_str(&escape_html(&concept.summary));
        html.push_str("</p>");
    }
    html.push_str("</div>");
}

fn render_summary(html: &mut String, view: &BetaStudyView) {
    html.push_str("<section class=\"panel\"><h2>State</h2><dl><dt>Sources</dt><dd>");
    html.push_str(&view.summary.source_count.to_string());
    html.push_str("</dd><dt>Approved</dt><dd>");
    html.push_str(&view.summary.approved_review_unit_count.to_string());
    html.push_str("</dd><dt>Attempts</dt><dd>");
    html.push_str(&view.summary.attempt_count.to_string());
    html.push_str("</dd></dl></section>");
}

fn render_source_form(html: &mut String) {
    html.push_str("<section class=\"panel\"><h2>Add</h2><form class=\"composer\" method=\"post\" action=\"/source\"><label for=\"source-capture\">Paste anything</label><textarea id=\"source-capture\" name=\"capture\" placeholder=\"Word, phrase, notes, or article\"></textarea><button type=\"submit\">Save capture</button></form><form class=\"actions\" method=\"post\" action=\"/generate\"><button type=\"submit\" class=\"secondary\">Generate review items</button></form></section>");
}

fn render_generation_notices(html: &mut String, view: &BetaStudyView) {
    if view.generation_notices.is_empty() {
        return;
    }
    html.push_str("<section class=\"panel\"><h2>Generation notes</h2><ul>");
    for notice in &view.generation_notices {
        html.push_str("<li>");
        html.push_str(&escape_html(notice));
        html.push_str("</li>");
    }
    html.push_str("</ul></section>");
}

fn snooze_until() -> i64 {
    memory_engine_study::DEFAULT_BETA_STUDY_NOW + 86_400_000
}

fn render_drafts(html: &mut String, view: &BetaStudyView) {
    html.push_str("<section class=\"panel\"><h2>Drafts</h2><ul>");
    for draft in &view.drafts {
        html.push_str("<li><strong>");
        html.push_str(&escape_html(&draft.prompt));
        html.push_str("</strong>");
        html.push_str(&escape_html(&format!(
            "{:?} - {}",
            draft.activity_kind, draft.activity_stage
        )));
        html.push_str("<form method=\"post\" action=\"/approve\"><input type=\"hidden\" name=\"draftId\" value=\"");
        html.push_str(&escape_html(&draft.id));
        html.push_str(
            "\"><button type=\"submit\" class=\"secondary\">Approve</button></form></li>",
        );
    }
    html.push_str("</ul></section>");
}

fn render_queue(html: &mut String, view: &BetaStudyView) {
    html.push_str("<section class=\"panel\"><h2>Queue</h2><ol>");
    for row in &view.queue {
        html.push_str("<li><strong>");
        html.push_str(&escape_html(row.review_unit_id.as_str()));
        html.push_str("</strong>");
        html.push_str(&escape_html(&format!("due {}", row.due)));
        html.push_str("</li>");
    }
    html.push_str("</ol></section>");
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const CSS: &str = r"
body{margin:0;background:#f7f8f4;color:#20241f;font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}
*{box-sizing:border-box}
main{min-height:100vh;display:grid;grid-template-columns:minmax(0,1fr) minmax(280px,360px)}
.study{min-width:0;display:grid;grid-template-rows:auto minmax(0,1fr);padding:24px}
header,.actions{display:flex;align-items:center;justify-content:space-between;gap:12px}
header{border-bottom:1px solid #d9ded5;padding-bottom:16px;color:#586257;font-size:14px}
.prompt{width:min(100%,760px);align-self:center;padding:28px 0}
h1{margin:0 0 18px;font-size:34px;line-height:1.12;letter-spacing:0}
.kind,.actions{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:16px}
.kind span{border:1px solid #b9c4b5;border-radius:6px;padding:5px 8px;background:#fff;color:#485348;font-size:13px;font-weight:650}
label{display:block;margin-bottom:8px;color:#586257;font-size:14px;font-weight:650}
textarea,input{width:100%;border:1px solid #b9c4b5;border-radius:6px;padding:12px 14px;background:#fff;color:#20241f;font:inherit;line-height:1.45}
textarea{min-height:116px;resize:vertical}
button{min-height:40px;border:1px solid #24533d;border-radius:6px;padding:0 14px;background:#24533d;color:#fff;font:inherit;font-weight:700;cursor:pointer}
button.secondary{background:#fff;color:#24533d}
button.danger{border-color:#7a2e2a;color:#7a2e2a}
.edit{display:grid;gap:10px;margin-top:14px}
.choices{margin:0 0 16px;padding-left:22px;color:#20241f;font-weight:650}
.feedback{margin-top:12px;border-left:3px solid #d9ded5;padding-left:12px;color:#455048;font-size:15px}
.feedback p{margin:6px 0 0}
.answer,.grade,.solution,.reference{margin-top:12px;min-height:24px;overflow-wrap:anywhere;font-size:15px;font-weight:650}.answer{color:#24533d}.grade{color:#7a4322}.solution,.reference{color:#455048;font-weight:500}.reference{border-left:3px solid #b9c4b5;padding-left:12px;white-space:pre-wrap}
aside{min-width:0;border-left:1px solid #d9ded5;background:#fff}
section.panel{border-bottom:1px solid #d9ded5;padding:20px}
h2{margin:0 0 14px;font-size:15px;line-height:1.2;letter-spacing:0}
dl{display:grid;grid-template-columns:1fr auto;gap:9px 14px;margin:0;color:#586257;font-size:14px}
dd{margin:0;color:#20241f;font-weight:700}
ol,ul{display:grid;gap:9px;margin:0;padding-left:18px;color:#586257;font-size:14px}
li strong{display:block;color:#20241f;overflow-wrap:anywhere}
.composer{display:grid;gap:10px}
@media(max-width:760px){main{grid-template-columns:1fr}.study{display:block;min-height:auto;padding:18px}h1{font-size:28px}aside{border-left:0;border-top:1px solid #d9ded5}}
";

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
    use std::{
        fs,
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        path::PathBuf,
        thread,
    };

    use serde_json::{json, Value};

    use super::{
        looks_like_json, render_page, route, serve_connections, BetaStudyOptions, BetaStudySession,
        HttpRequest,
    };

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
                &json!({"answer": "ALFA", "responseTimeMs": 6500}).to_string(),
            ),
        );
        let answered: Value = serde_json::from_slice(&answered.body).expect("answered");
        assert_eq!(answered["status"], json!("graded"));
        assert_eq!(answered["current"]["grade"]["verdict"], json!("correct"));
        assert_eq!(answered["current"]["grade"]["rating"], json!(3));
        assert_eq!(answered["summary"]["attemptCount"], json!(1));
    }

    #[test]
    fn json_answers_sanitize_malformed_and_out_of_range_response_times() {
        for (index, response_time_ms) in [
            json!("6500"),
            json!("not-a-number"),
            json!(-250),
            json!(1.5),
            json!(u64::from(u32::MAX) + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let directory = TempDirectory::new(&format!("json-timing-{index}"));
            let mut session = session(directory.path().join("study.json"));
            seed_nato_source_and_generate(&mut session);
            approve_draft(&mut session, "study-run-1-draft-src-nato-1-nato-letter-a");

            let answered = route(
                &mut session,
                &request(
                    "POST",
                    "/answer",
                    &json!({
                        "answer": "ALFA",
                        "responseTimeMs": response_time_ms,
                    })
                    .to_string(),
                ),
            );
            assert_eq!(answered.status, 200, "timing case {index}");
            let answered: Value = serde_json::from_slice(&answered.body).expect("answered");
            assert_eq!(answered["current"]["grade"]["verdict"], json!("correct"));
            assert_eq!(answered["current"]["grade"]["rating"], json!(3));
        }
    }

    #[test]
    fn renders_honest_response_timing_for_review_forms() {
        let directory = TempDirectory::new("honest-timing-markup");
        let mut session = session(directory.path().join("study.json"));
        seed_nato_source_and_generate(&mut session);
        approve_draft(&mut session, "study-run-1-draft-src-nato-1-nato-letter-a");

        let html = String::from_utf8(route(&mut session, &request("GET", "/", "")).body)
            .expect("review html");
        assert!(html.contains(r#"name="responseTimeMs" value=""#));
        assert!(!html.contains(r#"name="responseTimeMs" value="2400"#));
        assert!(
            html.contains(r#"window.performance && typeof window.performance.now === "function""#)
        );
        assert!(html.contains("monotonic ? window.performance.now() : Date.now()"));
        assert!(!html.contains("typeof performance.now"));
        assert!(!html.contains("? performance.now()"));
        assert!(html.contains("Date.now"));
        assert!(html.contains("Math.max(1, Math.round(elapsed))"));
    }

    #[test]
    fn drives_item_lifecycle_routes_through_rust() {
        let directory = TempDirectory::new("lifecycle");
        let mut lifecycle_session = session(directory.path().join("study.json"));
        seed_nato_source_and_generate(&mut lifecycle_session);
        approve_draft(
            &mut lifecycle_session,
            "study-run-1-draft-src-nato-2-nato-cat-composition",
        );
        approve_draft(
            &mut lifecycle_session,
            "study-run-1-draft-src-nato-1-nato-letter-a",
        );

        let learned = route(
            &mut lifecycle_session,
            &request("POST", "/current/learn-more", "{}"),
        );
        let learned: Value = serde_json::from_slice(&learned.body).expect("learned");
        assert_eq!(learned["status"], json!("answering"));
        assert_eq!(learned["current"]["expectedAnswer"], json!(null));
        assert_eq!(
            learned["current"]["referenceText"],
            json!("The NATO phonetic alphabet word for A is ALFA.")
        );

        let edited = route(
            &mut lifecycle_session,
            &request(
                "POST",
                "/current/edit",
                &json!({
                    "prompt": "Name the NATO code word for A.",
                    "expectedAnswer": "ALFA"
                })
                .to_string(),
            ),
        );
        let edited: Value = serde_json::from_slice(&edited.body).expect("edited");
        assert_eq!(
            edited["current"]["prompt"],
            json!("Name the NATO code word for A.")
        );
        assert_eq!(edited["current"]["revisionExpectedAnswer"], json!("ALFA"));
        assert_eq!(edited["current"]["expectedAnswer"], json!(null));

        let deleted = route(
            &mut lifecycle_session,
            &request("POST", "/current/delete", "{}"),
        );
        let deleted: Value = serde_json::from_slice(&deleted.body).expect("deleted");
        assert_eq!(deleted["summary"]["approvedReviewUnitCount"], json!(0));
        assert_eq!(deleted["queue"], json!([]));

        let mut snooze_session = session(directory.path().join("snooze-study.json"));
        seed_nato_source_and_generate(&mut snooze_session);
        approve_draft(
            &mut snooze_session,
            "study-run-1-draft-src-nato-1-nato-letter-a",
        );
        let snoozed = route(
            &mut snooze_session,
            &request(
                "POST",
                "/current/snooze",
                &json!({"snoozedUntil": NOW + 86_400_000}).to_string(),
            ),
        );
        let snoozed: Value = serde_json::from_slice(&snoozed.body).expect("snoozed");
        assert_eq!(snoozed["status"], json!("drafting"));
        assert_eq!(snoozed["queue"][0]["due"], json!(NOW + 86_400_000));
    }

    #[test]
    fn drives_the_phone_form_flow_without_browser_javascript() {
        let directory = TempDirectory::new("form-flow");
        let mut session = session(directory.path().join("study.json"));

        let saved = route(
            &mut session,
            &form_request(
                "/source",
                &format!("capture={}", url_escape(&source_body())),
            ),
        );
        assert_eq!(saved.status, 200);
        assert_eq!(saved.content_type, "text/html; charset=utf-8");
        let saved_html = String::from_utf8(saved.body).expect("saved html");
        assert!(saved_html.contains("Beta Study"));
        assert!(saved_html.contains(r#"name="capture""#));
        assert!(!saved_html.contains(r#"name="title""#));
        assert!(!saved_html.contains(r#"name="body""#));
        assert!(!saved_html.contains("Source title"));
        assert!(!saved_html.contains("Source blocks"));
        assert!(!saved_html.contains("<script"));

        let generated = route(&mut session, &form_request("/generate", ""));
        let generated_html = String::from_utf8(generated.body).expect("generated html");
        assert!(generated_html.contains("What is the NATO phonetic alphabet word for A?"));
        assert!(!generated_html.contains("<script"));
    }

    #[test]
    fn renders_generation_notices_as_human_sentences() {
        let directory = TempDirectory::new("notice-flow");
        let session = session(directory.path().join("study.json"));
        let mut view = session.view().expect("view");
        view.generation_notices = vec!["No review items could be generated.".to_owned()];

        let generated_html = render_page(&view, None);

        assert!(generated_html.contains("Generation notes"));
        assert!(generated_html.contains("No review items could be generated"));
    }

    #[test]
    fn generates_review_items_for_arbitrary_capture_text() {
        let directory = TempDirectory::new("arbitrary-capture");
        let mut session = session(directory.path().join("study.json"));

        let saved = route(
            &mut session,
            &form_request(
                "/source",
                "capture=Mitochondria+are+organelles+because+cells+use+ATP+as+chemical+energy.",
            ),
        );
        assert_eq!(saved.status, 200);

        let generated = route(&mut session, &form_request("/generate", ""));
        let generated_html = String::from_utf8(generated.body).expect("generated html");

        assert!(generated_html.contains("Drafts"));
        assert!(generated_html.contains("Explain the idea"));
        assert!(!generated_html.contains("No review items could be generated"));
    }

    #[test]
    fn renders_provider_failures_as_generation_notices() {
        let directory = TempDirectory::new("provider-failure-notice");
        let session = session(directory.path().join("study.json"));
        let mut view = session.view().expect("view");
        view.generation_notices =
            vec!["src-prose: The model provider could not be reached.".to_owned()];

        let html = render_page(&view, None);

        assert!(html.contains("Generation notes"));
        assert!(html.contains("src-prose: The model provider could not be reached."));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn accepts_legacy_and_json_capture_payload_shapes() {
        let directory = TempDirectory::new("source-payload-shapes");
        let mut legacy_session = session(directory.path().join("legacy-study.json"));

        let legacy = route(
            &mut legacy_session,
            &form_request(
                "/source",
                &format!(
                    "title=NATO+practice+notes&body={}",
                    url_escape(&source_body())
                ),
            ),
        );

        assert_eq!(legacy.status, 200);
        assert_eq!(
            legacy_session.view().expect("view").sources[0].title,
            "NATO practice notes"
        );

        let mut capture_session = session(directory.path().join("capture-study.json"));
        let capture = route(
            &mut capture_session,
            &request(
                "POST",
                "/source",
                &json!({ "capture": source_body() }).to_string(),
            ),
        );

        assert_eq!(capture.status, 200);
        assert_eq!(
            capture_session.view().expect("view").sources[0].title,
            "Concept: NATO letter A"
        );
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

    #[test]
    fn unavailable_response_time_uses_conservative_rating() {
        let directory = TempDirectory::new("zero-response-time");
        let mut session = session(directory.path().join("study.json"));
        route(
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
        route(&mut session, &request("POST", "/generate", "{}"));
        route(
            &mut session,
            &request(
                "POST",
                "/approve",
                &json!({"draftId": "study-run-1-draft-src-nato-1-nato-letter-a"}).to_string(),
            ),
        );

        let answered = route(
            &mut session,
            &request(
                "POST",
                "/answer",
                &json!({"answer": "ALFA", "responseTimeMs": 0}).to_string(),
            ),
        );

        assert_eq!(answered.status, 200);
        let answered: Value = serde_json::from_slice(&answered.body).expect("answered");
        assert_eq!(answered["current"]["grade"]["verdict"], json!("correct"));
        assert_eq!(answered["current"]["grade"]["rating"], json!(3));
        assert_eq!(answered["summary"]["attemptCount"], json!(1));
    }

    #[test]
    fn ignores_an_incomplete_connection_before_serving_a_following_browser_form() {
        let directory = TempDirectory::new("disconnect");
        let study_path = directory.path().join("study.json");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = thread::spawn(move || {
            let mut session =
                BetaStudySession::open(BetaStudyOptions::new(study_path).with_clock(now))
                    .expect("open");
            session.start().expect("start");
            serve_connections(&listener, &mut session, Some(2)).expect("serve connections");
        });

        let mut incomplete = TcpStream::connect(address).expect("incomplete connection");
        incomplete
            .write_all(b"POST /source HTTP/1.1\r\nHost: localhost\r\n")
            .expect("write incomplete headers");
        incomplete
            .shutdown(Shutdown::Write)
            .expect("close incomplete connection");
        drop(incomplete);

        let body = b"capture=hello";
        let mut valid = TcpStream::connect(address).expect("valid connection");
        write!(
            valid,
            "POST /source HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .expect("write browser form headers");
        valid.write_all(body).expect("write browser form body");
        valid
            .shutdown(Shutdown::Write)
            .expect("finish browser form");
        let mut response = String::new();
        valid.read_to_string(&mut response).expect("read response");

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Sources</dt><dd>1</dd>"));
        server.join().expect("server completes");
    }

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            content_type: looks_like_json(body.as_bytes()).then(|| "application/json".to_owned()),
            body: body.as_bytes().to_vec(),
        }
    }

    fn form_request(path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: "POST".to_owned(),
            path: path.to_owned(),
            content_type: Some("application/x-www-form-urlencoded".to_owned()),
            body: body.as_bytes().to_vec(),
        }
    }

    fn session(path: PathBuf) -> BetaStudySession {
        let mut session =
            BetaStudySession::open(BetaStudyOptions::new(path).with_clock(now)).expect("open");
        session.start().expect("start");
        session
    }

    fn seed_nato_source_and_generate(session: &mut BetaStudySession) {
        route(
            session,
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
        route(session, &request("POST", "/generate", "{}"));
    }

    fn approve_draft(session: &mut BetaStudySession, draft_id: &str) {
        route(
            session,
            &request(
                "POST",
                "/approve",
                &json!({"draftId": draft_id}).to_string(),
            ),
        );
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

    fn url_escape(value: &str) -> String {
        value
            .replace('%', "%25")
            .replace('\n', "%0A")
            .replace(' ', "+")
            .replace(':', "%3A")
            .replace('?', "%3F")
            .replace(',', "%2C")
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
