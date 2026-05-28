//! Rust web-shell dogfood host.
//!
//! This crate owns learner-facing session choreography, reveal state, compact
//! view DTOs, HTTP parsing, and static HTML delivery. The reusable service owns
//! grading, scheduling, and queue selection.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
};

use memory_engine_core::{
    Prompt, QueueCandidate, Rating, ReviewUnitId, ScheduleState, ScheduleStatus, Verdict,
};
use memory_engine_import::{compile_latin_prayer_fixture, CompiledImportProbe};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, NextQueueCommand, NextQueueOptions,
    ServiceAttemptRecord, ServiceError,
};
use serde::{Deserialize, Serialize};

const INDEX_HTML: &str = include_str!("../../../experiments/web-shell/index.html");
const NOW: i64 = 1_779_984_000_000;

const INTERFACE_PRESSURE: [&str; 3] = [
    "Reveal is UI-owned because the service has no first-class revealed review command.",
    "Review-state visibility needs a compact DTO; raw ScheduleState is too engine-shaped for UI copy.",
    "Prompt copy, confidence copy, and answer draft state remain client-owned.",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WebShellStatus {
    Answering,
    Revealed,
    Graded,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellView {
    pub fixture: String,
    pub status: WebShellStatus,
    pub current: Option<WebShellCurrent>,
    pub queue: Vec<WebShellQueueRow>,
    pub attempts: usize,
    pub commands: Vec<String>,
    pub interface_pressure: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellCurrent {
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: Option<String>,
    pub prompt: String,
    pub expected_answer: Option<String>,
    pub grade: Option<WebShellGrade>,
    pub review_state: Option<WebShellReviewState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellGrade {
    pub verdict: Verdict,
    pub rating: u8,
    pub is_correct: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellReviewState {
    pub due: i64,
    pub reps: u32,
    pub state: ScheduleStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellQueueRow {
    pub review_unit_id: ReviewUnitId,
    pub due: i64,
    pub reps: u32,
    pub state: Option<ScheduleStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebShellReceipt {
    pub fixture: String,
    pub commands: Vec<String>,
    pub submitted_answer: String,
    pub graded_verdict: Verdict,
    pub graded_rating: u8,
    pub scheduled_reps: u32,
    pub next_review_unit_id: Option<String>,
    pub interface_pressure: Vec<String>,
    pub extraction_recommendation: String,
}

#[derive(Clone, Debug)]
pub struct WebShellConfig {
    pub address: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebShellError {
    EmptyFixture,
    NoActiveReviewUnit,
    MissingQueueCandidate(ReviewUnitId),
    MissingPromptId(ReviewUnitId),
    UnknownReviewUnit(ReviewUnitId),
    Service(ServiceError<WebShellStoreError>),
    Io(String),
}

impl fmt::Display for WebShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFixture => formatter.write_str("web shell fixture must not be empty"),
            Self::NoActiveReviewUnit => formatter.write_str("web shell has no active review unit"),
            Self::MissingQueueCandidate(id) => write!(formatter, "Missing queue candidate: {id}"),
            Self::MissingPromptId(id) => write!(formatter, "Missing prompt id: {id}"),
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown web shell review unit: {id}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
        }
    }
}

impl Error for WebShellError {}

impl From<ServiceError<WebShellStoreError>> for WebShellError {
    fn from(error: ServiceError<WebShellStoreError>) -> Self {
        Self::Service(error)
    }
}

impl From<io::Error> for WebShellError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebShellStoreError {
    UnknownReviewUnit(ReviewUnitId),
}

impl fmt::Display for WebShellStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown web shell review unit: {id}"),
        }
    }
}

impl Error for WebShellStoreError {}

#[derive(Clone, Debug)]
struct WebShellUnit {
    prompt: Prompt,
    prompt_id: Option<String>,
    queue: QueueCandidate,
}

#[derive(Clone, Debug)]
struct WebShellStore {
    units: BTreeMap<ReviewUnitId, WebShellUnit>,
    attempts: Vec<ServiceAttemptRecord>,
    schedules: BTreeMap<ReviewUnitId, ScheduleState>,
}

impl WebShellStore {
    fn new(compiled: &CompiledImportProbe, units: &[WebShellUnit]) -> Self {
        Self {
            units: units
                .iter()
                .map(|unit| (prompt_review_unit_id(&unit.prompt).clone(), unit.clone()))
                .collect(),
            attempts: Vec::new(),
            schedules: compiled
                .schedules
                .iter()
                .map(|schedule| (schedule.review_unit_id.clone(), schedule.state.clone()))
                .collect(),
        }
    }

    fn attempt_count(&self) -> usize {
        self.attempts.len()
    }

    fn schedule_for(&self, review_unit_id: &ReviewUnitId) -> Option<&ScheduleState> {
        self.schedules.get(review_unit_id)
    }

    fn assert_known(&self, review_unit_id: &ReviewUnitId) -> Result<(), WebShellStoreError> {
        if self.units.contains_key(review_unit_id) {
            Ok(())
        } else {
            Err(WebShellStoreError::UnknownReviewUnit(
                review_unit_id.clone(),
            ))
        }
    }
}

impl MemoryServiceStore for WebShellStore {
    type Error = WebShellStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.assert_known(&attempt.review_unit_id)?;
        self.attempts.push(attempt);
        Ok(())
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        self.assert_known(review_unit_id)?;
        Ok(self.schedules.get(review_unit_id).cloned())
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        _expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        self.assert_known(review_unit_id)?;
        self.attempts.push(attempt);
        self.schedules
            .insert(review_unit_id.clone(), schedule_state);
        Ok(())
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self
            .units
            .values()
            .map(|unit| {
                let review_unit_id = prompt_review_unit_id(&unit.prompt);
                let schedule_state = self.schedules.get(review_unit_id).cloned();

                QueueCandidate {
                    review_unit_id: review_unit_id.clone(),
                    due: schedule_state
                        .as_ref()
                        .map_or(unit.queue.due, |state| state.due),
                    schedule_state,
                    progression: unit.queue.progression.clone(),
                    concept_key: unit.queue.concept_key.clone(),
                    source_key: unit.queue.source_key.clone(),
                    domain_key: unit.queue.domain_key.clone(),
                }
            })
            .collect())
    }
}

pub struct WebShellSession {
    fixture: String,
    units: Vec<WebShellUnit>,
    service: MemoryService<WebShellStore, fn(&ScheduleState) -> bool>,
    current: Option<WebShellUnit>,
    status: WebShellStatus,
    grade: Option<WebShellGrade>,
    expected_answer: Option<String>,
    commands: Vec<String>,
}

impl WebShellSession {
    /// Build the web-shell session from the built-in fixture.
    ///
    /// # Panics
    ///
    /// Panics only if the checked-in fixture is internally inconsistent. Use
    /// [`WebShellSession::try_new`] when callers need an error value.
    #[must_use]
    pub fn new() -> Self {
        Self::try_new().expect("built-in web-shell fixture is valid")
    }

    /// Build the web-shell session from the built-in Latin-prayer fixture.
    ///
    /// # Errors
    ///
    /// Returns [`WebShellError`] if the fixture cannot be assembled into
    /// service-owned units.
    pub fn try_new() -> Result<Self, WebShellError> {
        let compiled = compile_latin_prayer_fixture(NOW);
        let units = units_from_compiled(&compiled)?;
        let store = WebShellStore::new(&compiled, &units);
        let service = MemoryService::with_clock(
            store,
            mastered_after_four_reviews as fn(&ScheduleState) -> bool,
            || NOW,
        );

        Ok(Self {
            fixture: compiled.fixture,
            units,
            service,
            current: None,
            status: WebShellStatus::Answering,
            grade: None,
            expected_answer: None,
            commands: Vec::new(),
        })
    }

    /// Select the first due queue item.
    ///
    /// # Errors
    ///
    /// Returns [`WebShellError`] when queue selection fails.
    pub fn start(&mut self) -> Result<WebShellView, WebShellError> {
        self.select_next()
    }

    /// Reveal the current prompt answer without applying a scheduler review.
    ///
    /// # Errors
    ///
    /// Returns [`WebShellError::NoActiveReviewUnit`] when the shell has not
    /// selected a current prompt.
    pub fn reveal(&mut self) -> Result<WebShellView, WebShellError> {
        let active = self
            .current
            .as_ref()
            .ok_or(WebShellError::NoActiveReviewUnit)?;
        self.commands.push("reveal".to_owned());
        self.expected_answer = Some(prompt_expected_answer(&active.prompt));
        self.status = WebShellStatus::Revealed;

        Ok(self.view())
    }

    /// Submit an answer through the Rust service grade/apply-review workflow.
    ///
    /// # Errors
    ///
    /// Returns [`WebShellError`] when no prompt is active or the service store
    /// rejects the review.
    pub fn submit_answer(
        &mut self,
        answer: String,
        response_time_ms: u32,
    ) -> Result<WebShellView, WebShellError> {
        let active = self
            .current
            .as_ref()
            .ok_or(WebShellError::NoActiveReviewUnit)?;
        self.commands.push("grade/apply-review".to_owned());
        let review = self.service.grade_apply_review(GradeApplyReviewCommand {
            prompt: active.prompt.clone(),
            submitted_answer: answer,
            response_time_ms,
            prompt_id: active.prompt_id.clone(),
            occurred_at: Some(NOW),
            idempotency_key: None,
        })?;
        self.grade = Some(WebShellGrade {
            verdict: review.grade.verdict,
            rating: rating_value(review.grade.rating),
            is_correct: review.grade.is_correct,
        });
        self.expected_answer = Some(review.grade.expected_answer);
        self.status = WebShellStatus::Graded;

        Ok(self.view())
    }

    /// Advance to the next due queue item.
    ///
    /// # Errors
    ///
    /// Returns [`WebShellError`] when queue selection fails.
    pub fn advance(&mut self) -> Result<WebShellView, WebShellError> {
        self.select_next()
    }

    #[must_use]
    pub fn view(&self) -> WebShellView {
        WebShellView {
            fixture: self.fixture.clone(),
            status: self.status.clone(),
            current: self.current.as_ref().map(|unit| self.current_view(unit)),
            queue: self.queue_rows(),
            attempts: self.service.store().attempt_count(),
            commands: self.commands.clone(),
            interface_pressure: interface_pressure(),
        }
    }

    fn select_next(&mut self) -> Result<WebShellView, WebShellError> {
        self.commands.push("next-queue".to_owned());
        let selected = self.service.next_queue(NextQueueCommand {
            options: NextQueueOptions::default(),
        })?;
        self.current = selected
            .candidate
            .as_ref()
            .and_then(|candidate| self.unit_for(&candidate.review_unit_id).cloned());
        self.status = WebShellStatus::Answering;
        self.grade = None;
        self.expected_answer = None;

        Ok(self.view())
    }

    fn unit_for(&self, review_unit_id: &ReviewUnitId) -> Option<&WebShellUnit> {
        self.units
            .iter()
            .find(|unit| prompt_review_unit_id(&unit.prompt) == review_unit_id)
    }

    fn current_view(&self, unit: &WebShellUnit) -> WebShellCurrent {
        let review_unit_id = prompt_review_unit_id(&unit.prompt);
        let schedule = self.service.store().schedule_for(review_unit_id);

        WebShellCurrent {
            review_unit_id: review_unit_id.clone(),
            prompt_id: unit.prompt_id.clone(),
            prompt: prompt_text(&unit.prompt).to_owned(),
            expected_answer: self.expected_answer.clone(),
            grade: self.grade.clone(),
            review_state: schedule.map(review_state),
        }
    }

    fn queue_rows(&self) -> Vec<WebShellQueueRow> {
        let mut rows = self
            .units
            .iter()
            .map(|unit| {
                let review_unit_id = prompt_review_unit_id(&unit.prompt);
                let schedule = self.service.store().schedule_for(review_unit_id);

                WebShellQueueRow {
                    review_unit_id: review_unit_id.clone(),
                    due: schedule.map_or(unit.queue.due, |state| state.due),
                    reps: schedule.map_or(0, |state| state.reps),
                    state: schedule.map(|state| state.state),
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.due.cmp(&right.due));
        rows
    }
}

impl Default for WebShellSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the canned web-shell dogfood flow and emit an extraction receipt.
///
/// # Errors
///
/// Returns [`WebShellError`] if fixture assembly, queue selection, reveal, or
/// review application fails.
pub fn run_web_shell_flow() -> Result<WebShellReceipt, WebShellError> {
    let mut shell = WebShellSession::try_new()?;
    shell.start()?;
    shell.reveal()?;
    let reviewed = shell.submit_answer("I believe in one God".to_owned(), 2_400)?;
    let next = shell.advance()?;
    let reviewed_current = reviewed
        .current
        .as_ref()
        .ok_or(WebShellError::NoActiveReviewUnit)?;
    let grade = reviewed_current
        .grade
        .as_ref()
        .ok_or(WebShellError::NoActiveReviewUnit)?;
    let review_state = reviewed_current
        .review_state
        .as_ref()
        .ok_or(WebShellError::NoActiveReviewUnit)?;

    Ok(WebShellReceipt {
        fixture: reviewed.fixture,
        commands: next.commands,
        submitted_answer: "I believe in one God".to_owned(),
        graded_verdict: grade.verdict,
        graded_rating: grade.rating,
        scheduled_reps: review_state.reps,
        next_review_unit_id: next
            .current
            .as_ref()
            .map(|current| current.review_unit_id.as_str().to_owned()),
        interface_pressure: interface_pressure(),
        extraction_recommendation: "keep experimenting".to_owned(),
    })
}

/// Run the blocking web-shell HTTP server.
///
/// # Errors
///
/// Returns [`WebShellError`] when the socket cannot bind or a connection write
/// fails.
pub fn serve(config: &WebShellConfig) -> Result<(), WebShellError> {
    let listener = TcpListener::bind(&config.address)?;
    let mut session = WebShellSession::try_new()?;
    let _ = session.start()?;

    for stream in listener.incoming() {
        let mut stream = stream?;
        handle_stream(&mut session, &mut stream)?;
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

pub fn route(session: &mut WebShellSession, request: &HttpRequest) -> HttpResponse {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => HttpResponse::html(INDEX_HTML),
        ("GET", "/state") => HttpResponse::json(200, &session.view()),
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
#[serde(rename_all = "camelCase")]
struct AnswerPayload {
    answer: String,
    response_time_ms: u32,
}

fn handle_stream(
    session: &mut WebShellSession,
    stream: &mut TcpStream,
) -> Result<(), WebShellError> {
    let request = HttpRequest::read_from(stream)?;
    let response = route(session, &request);
    response.write_to(stream)?;

    Ok(())
}

fn view_response(result: Result<WebShellView, WebShellError>) -> HttpResponse {
    match result {
        Ok(view) => HttpResponse::json(200, &view),
        Err(error) => HttpResponse::plain(500, &error.to_string()),
    }
}

fn read_answer(body: &[u8]) -> Result<AnswerPayload, String> {
    let payload: AnswerPayload = serde_json::from_slice(body)
        .map_err(|error| format!("Request body must be an answer object: {error}"))?;
    if payload.answer.trim().is_empty() {
        return Err("answer must be a non-empty string".to_owned());
    }
    if payload.response_time_ms == 0 {
        return Err("responseTimeMs must be a positive integer".to_owned());
    }

    Ok(payload)
}

fn units_from_compiled(compiled: &CompiledImportProbe) -> Result<Vec<WebShellUnit>, WebShellError> {
    compiled
        .prompts
        .iter()
        .map(|prompt| {
            let review_unit_id = prompt_review_unit_id(prompt);
            let queue = compiled
                .queue
                .iter()
                .find(|candidate| &candidate.review_unit_id == review_unit_id)
                .ok_or_else(|| WebShellError::MissingQueueCandidate(review_unit_id.clone()))?;
            let prompt_id = compiled
                .prompt_ids
                .iter()
                .find_map(|(id, prompt_id)| (id == review_unit_id).then(|| prompt_id.clone()))
                .ok_or_else(|| WebShellError::MissingPromptId(review_unit_id.clone()))?;

            Ok(WebShellUnit {
                prompt: prompt.clone(),
                prompt_id: Some(prompt_id),
                queue: queue.clone(),
            })
        })
        .collect()
}

fn prompt_review_unit_id(prompt: &Prompt) -> &ReviewUnitId {
    match prompt {
        Prompt::Mcq { review_unit_id, .. } | Prompt::Boolean { review_unit_id, .. } => {
            review_unit_id
        }
        Prompt::Exact(prompt) => &prompt.review_unit_id,
    }
}

fn prompt_text(prompt: &Prompt) -> &str {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => prompt,
        Prompt::Exact(prompt) => &prompt.prompt,
    }
}

fn prompt_expected_answer(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Mcq { correct_choice, .. } => correct_choice.clone(),
        Prompt::Boolean { correct_answer, .. } => {
            if *correct_answer {
                "True".to_owned()
            } else {
                "False".to_owned()
            }
        }
        Prompt::Exact(prompt) => prompt.accepted_answers.join(" / "),
    }
}

fn review_state(schedule: &ScheduleState) -> WebShellReviewState {
    WebShellReviewState {
        due: schedule.due,
        reps: schedule.reps,
        state: schedule.state,
    }
}

fn mastered_after_four_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 4
}

fn rating_value(rating: Rating) -> u8 {
    match rating {
        Rating::Again => 1,
        Rating::Hard => 2,
        Rating::Good => 3,
        Rating::Easy => 4,
    }
}

fn interface_pressure() -> Vec<String> {
    INTERFACE_PRESSURE
        .iter()
        .map(|pressure| (*pressure).to_owned())
        .collect()
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
    use serde_json::{json, Value};

    use super::{route, run_web_shell_flow, HttpRequest, WebShellSession, WebShellStatus};

    #[test]
    fn drives_reveal_review_and_queue_choreography_through_the_service_boundary() {
        let mut shell = WebShellSession::new();

        let initial = shell.start().expect("start");
        assert_eq!(initial.fixture, "latin-prayer-authored-v1");
        assert_eq!(initial.status, WebShellStatus::Answering);
        assert_eq!(
            initial
                .current
                .as_ref()
                .map(|current| current.review_unit_id.as_str()),
            Some("import-credo-in-unum-deum")
        );
        assert_eq!(
            initial
                .current
                .as_ref()
                .map(|current| current.prompt.as_str()),
            Some("Translate: Credo in unum Deum")
        );
        assert_eq!(
            initial
                .queue
                .iter()
                .map(|row| row.review_unit_id.as_str())
                .collect::<Vec<_>>(),
            ["import-credo-in-unum-deum", "import-pater-noster"]
        );

        let revealed = shell.reveal().expect("reveal");
        assert_eq!(revealed.status, WebShellStatus::Revealed);
        assert_eq!(
            revealed
                .current
                .as_ref()
                .and_then(|current| current.expected_answer.as_deref()),
            Some("I believe in one God")
        );

        let reviewed = shell
            .submit_answer("I believe in one God".to_owned(), 2_400)
            .expect("review");
        assert_eq!(reviewed.status, WebShellStatus::Graded);
        let reviewed_current = reviewed.current.as_ref().expect("current");
        assert_eq!(
            reviewed_current.grade.as_ref().map(|grade| grade.rating),
            Some(4)
        );
        assert_eq!(
            reviewed_current
                .review_state
                .as_ref()
                .map(|state| state.reps),
            Some(4)
        );
        assert_eq!(
            reviewed.commands,
            ["next-queue", "reveal", "grade/apply-review"]
        );

        let next = shell.advance().expect("next");
        assert_eq!(next.status, WebShellStatus::Answering);
        assert_eq!(
            next.current
                .as_ref()
                .map(|current| current.review_unit_id.as_str()),
            Some("import-pater-noster")
        );
    }

    #[test]
    fn emits_web_shell_receipt_for_extraction_review() {
        let receipt = run_web_shell_flow().expect("receipt");

        assert_eq!(receipt.fixture, "latin-prayer-authored-v1");
        assert_eq!(
            receipt.commands,
            ["next-queue", "reveal", "grade/apply-review", "next-queue"]
        );
        assert_eq!(receipt.submitted_answer, "I believe in one God");
        assert_eq!(receipt.graded_rating, 4);
        assert_eq!(receipt.scheduled_reps, 4);
        assert_eq!(
            receipt.next_review_unit_id.as_deref(),
            Some("import-pater-noster")
        );
        assert_eq!(receipt.extraction_recommendation, "keep experimenting");
        assert!(receipt
            .interface_pressure
            .contains(&"Review-state visibility needs a compact DTO; raw ScheduleState is too engine-shaped for UI copy.".to_owned()));
    }

    #[test]
    fn serves_html_state_and_validates_answer_payloads() {
        let mut shell = WebShellSession::new();
        shell.start().expect("start");

        let html = route(&mut shell, &request("GET", "/", ""));
        assert_eq!(html.status, 200);
        assert_eq!(html.content_type, "text/html; charset=utf-8");
        assert!(String::from_utf8(html.body)
            .expect("html")
            .contains("Memory Engine Web Shell"));

        let state = route(&mut shell, &request("GET", "/state", ""));
        assert_eq!(state.status, 200);
        let state: Value = serde_json::from_slice(&state.body).expect("state");
        assert_eq!(state["status"], json!("answering"));
        assert_eq!(
            state["current"]["reviewUnitId"],
            json!("import-credo-in-unum-deum")
        );

        let bad_answer = route(&mut shell, &request("POST", "/answer", r#"{"answer":""}"#));
        assert_eq!(bad_answer.status, 400);

        let reveal = route(&mut shell, &request("POST", "/reveal", "{}"));
        assert_eq!(reveal.status, 200);
        let reveal: Value = serde_json::from_slice(&reveal.body).expect("reveal");
        assert_eq!(
            reveal["current"]["expectedAnswer"],
            json!("I believe in one God")
        );
    }

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            body: body.as_bytes().to_vec(),
        }
    }
}
