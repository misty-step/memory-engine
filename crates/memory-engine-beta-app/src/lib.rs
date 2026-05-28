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

use memory_engine_study::{
    BetaStudyCurrent, BetaStudyOptions, BetaStudySession, BetaStudySourceInput, BetaStudyView,
};
use serde::{Deserialize, Serialize};

const DEFAULT_SOURCE_BODY: &str = "\
Concept: NATO letter A
Activity: quiz
Stage: recognition-3
Question: What is the NATO phonetic alphabet word for A?
Answer: ALFA
Distractors: BRAVO, CHARLIE
Reference: The NATO phonetic alphabet word for A is ALFA.";

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
        ("GET", "/") => page_response(session.view()),
        ("GET", "/state") => view_response(session.view()),
        ("POST", "/source") => match read_source(&request.body) {
            Ok(source) => response_for(request, session.add_source(source)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/generate") => response_for(request, session.generate(None)),
        ("POST", "/approve") => match read_required_string(&request.body, "draftId") {
            Ok(draft_id) => response_for(request, session.approve_draft(&draft_id)),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/reveal") => response_for(request, session.reveal()),
        ("POST", "/answer") => match read_answer(&request.body) {
            Ok(answer) => response_for(
                request,
                session.submit_answer(answer.answer, answer.response_time_ms),
            ),
            Err(error) => HttpResponse::bad_request(&error),
        },
        ("POST", "/next") => response_for(request, session.advance()),
        _ => HttpResponse::plain(404, "Not found"),
    }
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
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        let title = form_required(&fields, "title")?;
        let body = form_required(&fields, "body")?;
        let id = fields
            .iter()
            .find_map(|(key, value)| {
                (key == "id" && !value.trim().is_empty()).then(|| value.clone())
            })
            .unwrap_or_else(|| format!("source-{}", slug_fragment(&title)));

        return Ok(BetaStudySourceInput { id, title, body });
    }

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
    if !looks_like_json(body) {
        let fields = parse_form(body)?;
        let answer = form_required(&fields, "answer")?;
        let response_time_ms = fields
            .iter()
            .find_map(|(key, value)| (key == "responseTimeMs").then_some(value))
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| "responseTimeMs must be a positive integer".to_owned())?;
        if response_time_ms == 0 {
            return Err("responseTimeMs must be a positive integer".to_owned());
        }
        return Ok(AnswerPayload {
            answer,
            response_time_ms,
        });
    }

    let payload: AnswerPayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an answer object: {error}"))?;
    require_non_blank(&payload.answer, "answer")?;
    if payload.response_time_ms == 0 {
        return Err("responseTimeMs must be a positive integer".to_owned());
    }

    Ok(payload)
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
    let value = fields
        .iter()
        .find_map(|(field_key, value)| (field_key == key).then(|| value.clone()))
        .ok_or_else(|| format!("{key} must be a non-empty string"))?;
    require_non_blank(&value, key)?;
    Ok(value)
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
    render_source_form(&mut html);
    render_drafts(&mut html, view);
    render_queue(&mut html, view);
    html.push_str("</aside></main></body></html>");
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
        html.push_str("<form method=\"post\" action=\"/answer\"><label for=\"answer\">Answer or worked solution</label><textarea id=\"answer\" name=\"answer\" autocomplete=\"off\" spellcheck=\"false\"></textarea><input type=\"hidden\" name=\"responseTimeMs\" value=\"2400\"><div class=\"actions\"><button type=\"submit\">Submit</button></form><form method=\"post\" action=\"/reveal\"><button type=\"submit\" class=\"secondary\">Reveal</button></form><form method=\"post\" action=\"/next\"><button type=\"submit\" class=\"secondary\">Next</button></form></div>");
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
    html.push_str("<section class=\"panel\"><h2>Source</h2><form class=\"composer\" method=\"post\" action=\"/source\"><label for=\"source-title\">Source title</label><input id=\"source-title\" name=\"title\" value=\"NATO practice notes\"><label for=\"source-body\">Source blocks</label><textarea id=\"source-body\" name=\"body\">");
    html.push_str(&escape_html(DEFAULT_SOURCE_BODY));
    html.push_str("</textarea><button type=\"submit\">Save source</button></form><form class=\"actions\" method=\"post\" action=\"/generate\"><button type=\"submit\" class=\"secondary\">Generate</button></form></section>");
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
.answer,.grade,.solution{margin-top:12px;min-height:24px;overflow-wrap:anywhere;font-size:15px;font-weight:650}.answer{color:#24533d}.grade{color:#7a4322}.solution{color:#455048;font-weight:500}
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
    use std::{fs, path::PathBuf};

    use serde_json::{json, Value};

    use super::{looks_like_json, route, BetaStudyOptions, BetaStudySession, HttpRequest};

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
    fn drives_the_phone_form_flow_without_browser_javascript() {
        let directory = TempDirectory::new("form-flow");
        let mut session = session(directory.path().join("study.json"));

        let saved = route(
            &mut session,
            &form_request(
                "/source",
                &format!(
                    "title=NATO+practice+notes&body={}",
                    url_escape(&source_body())
                ),
            ),
        );
        assert_eq!(saved.status, 200);
        assert_eq!(saved.content_type, "text/html; charset=utf-8");
        let saved_html = String::from_utf8(saved.body).expect("saved html");
        assert!(saved_html.contains("Beta Study"));
        assert!(!saved_html.contains("<script"));

        let generated = route(&mut session, &form_request("/generate", ""));
        let generated_html = String::from_utf8(generated.body).expect("generated html");
        assert!(generated_html.contains("What is the NATO phonetic alphabet word for A?"));
        assert!(!generated_html.contains("<script"));
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
    fn rejects_zero_response_time_before_touching_the_session() {
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

        assert_eq!(answered.status, 400);
        assert!(String::from_utf8(answered.body)
            .expect("body")
            .contains("responseTimeMs must be"));
        assert_eq!(session.view().expect("view").summary.attempt_count, 0);
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
