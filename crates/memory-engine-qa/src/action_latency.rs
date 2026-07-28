use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fmt::{self, Write as _},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{to_bytes, Body},
    http::{
        header::{HeaderMap, SET_COOKIE},
        Request, Response, StatusCode,
    },
    Router,
};
use memory_engine_api::{router, AccountRegistry, ApiState, AuthConfig};
use memory_engine_performance::{
    Action, AuthAccountAction, GenerationAction, MaterialAction, ReviewAction,
};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

const RECEIPT_SCHEMA: &str = "memory_engine.action_latency_receipt.v1";
const BUDGET_SCHEMA: &str = "memory_engine.action_latency_budgets.v1";
const DEFAULT_BUDGET_PATH: &str = "docs/perf/action-latency-budgets.v1.json";
const SOURCE_TITLE: &str = "Latency fixture";
const SOURCE_BODY: &str = "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.\n\nConcept: NATO CAT composition\nActivity: exercise\nStage: composition\nQuestion: Spell CAT over the phone using the NATO phonetic alphabet.\nAnswer: CHARLIE ALFA TANGO\nWorked Solution: C is CHARLIE, A is ALFA, and T is TANGO.\nReference: C is CHARLIE. A is ALFA. T is TANGO.";
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ITERATIONS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    File,
    Postgres,
}

impl Backend {
    fn parse(value: &str) -> Result<Self, LatencyError> {
        match value {
            "file" => Ok(Self::File),
            "postgres" => Ok(Self::Postgres),
            _ => Err(LatencyError::new("--backend must be file or postgres")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionKind {
    AuthHome,
    LoginRequest,
    Capture,
    GenerationEnqueue,
    ReviewNext,
    ReviewSubmit,
    ReviewContentFeedback,
    ReviewReveal,
    ReviewSkip,
    ReviewSnooze,
    Logout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActionSpec {
    taxonomy: Action,
    name: &'static str,
    family: &'static str,
    kind: ActionKind,
}

const ACTION_SPECS: &[ActionSpec] = &[
    ActionSpec {
        taxonomy: Action::AuthAccount(AuthAccountAction::AppHome),
        name: "auth.app_home",
        family: "auth",
        kind: ActionKind::AuthHome,
    },
    ActionSpec {
        taxonomy: Action::AuthAccount(AuthAccountAction::Login),
        name: "auth.login_request",
        family: "auth",
        kind: ActionKind::LoginRequest,
    },
    ActionSpec {
        taxonomy: Action::Material(MaterialAction::CaptureSource),
        name: "material.capture",
        family: "material",
        kind: ActionKind::Capture,
    },
    ActionSpec {
        taxonomy: Action::Generation(GenerationAction::Enqueue),
        name: "generation.enqueue",
        family: "generation",
        kind: ActionKind::GenerationEnqueue,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::Next),
        name: "review.next",
        family: "review",
        kind: ActionKind::ReviewNext,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::Submit),
        name: "review.submit",
        family: "review",
        kind: ActionKind::ReviewSubmit,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::ContentFeedback),
        name: "review.content_feedback",
        family: "review",
        kind: ActionKind::ReviewContentFeedback,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::Reveal),
        name: "review.reveal",
        family: "review",
        kind: ActionKind::ReviewReveal,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::Skip),
        name: "review.skip",
        family: "review",
        kind: ActionKind::ReviewSkip,
    },
    ActionSpec {
        taxonomy: Action::Review(ReviewAction::Snooze),
        name: "review.snooze",
        family: "review",
        kind: ActionKind::ReviewSnooze,
    },
    ActionSpec {
        taxonomy: Action::AuthAccount(AuthAccountAction::Logout),
        name: "auth.logout",
        family: "auth",
        kind: ActionKind::Logout,
    },
];

#[derive(Debug)]
struct LatencyError {
    message: String,
}

impl LatencyError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LatencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for LatencyError {}

impl From<std::io::Error> for LatencyError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("I/O failure: {error}"))
    }
}

impl From<serde_json::Error> for LatencyError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON failure: {error}"))
    }
}

#[derive(Clone, Debug)]
struct LatencyOptions {
    backend: Backend,
    iterations: usize,
    out: Option<PathBuf>,
    markdown: Option<PathBuf>,
    postgres_url: Option<String>,
}

#[derive(Clone, Debug)]
struct DiffOptions {
    base: PathBuf,
    head: PathBuf,
    budget: PathBuf,
    markdown: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct ServerTiming {
    total_ms: Option<u64>,
    pgconnect_ms: Option<u64>,
    pgop_ms: Option<u64>,
    render_ms: Option<u64>,
    connect_count: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct ActionReceipt {
    action: String,
    family: String,
    samples_ms: Vec<u64>,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    max_ms: Option<u64>,
    server_timing: Option<ServerTiming>,
    http_status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct LatencyReceipt {
    schema: String,
    git_sha: String,
    recorded_at_unix_ms: u64,
    backend: String,
    iterations: usize,
    actions: Vec<ActionReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct BudgetThresholds {
    soft: u64,
    hard: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct BudgetEntry {
    p95_ms: BudgetThresholds,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct BudgetsFile {
    schema: String,
    actions: BTreeMap<String, BudgetEntry>,
}

#[derive(Clone, Debug)]
struct Observation {
    elapsed_ms: u64,
    status: u16,
    server_timing: Option<ServerTiming>,
}

#[derive(Clone, Debug)]
struct Session {
    cookie: String,
    csrf_token: String,
    source_id: String,
}

struct Fixture {
    app: Router,
    state: ApiState,
    root: Option<PathBuf>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[derive(Clone, Debug)]
struct DiffRow {
    action: String,
    family: String,
    base_p50: Option<u64>,
    head_p50: Option<u64>,
    base_p95: Option<u64>,
    head_p95: Option<u64>,
    delta_percent: Option<i64>,
    budget: BudgetThresholds,
    hard_fail: bool,
    soft_regression: bool,
}

#[derive(Clone, Debug)]
struct DiffReport {
    rows: Vec<DiffRow>,
    hard_fail: bool,
}

pub fn run(command: &str, args: &[String]) -> i32 {
    let result = match command {
        "latency" => parse_latency_options(args).and_then(|options| run_latency(&options)),
        "diff" => parse_diff_options(args).and_then(|options| run_diff(&options)),
        _ => Err(LatencyError::new("unknown latency command")),
    };

    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("action latency failed: {error}");
            1
        }
    }
}

fn parse_latency_options(args: &[String]) -> Result<LatencyOptions, LatencyError> {
    let mut backend = Backend::File;
    let mut iterations = 5;
    let mut out = None;
    let mut markdown = None;
    let mut index = 0;

    while index < args.len() {
        let option = args[index].as_str();
        let value = |index: &mut usize| -> Result<&str, LatencyError> {
            *index += 1;
            args.get(*index)
                .map(String::as_str)
                .ok_or_else(|| LatencyError::new(format!("{option} requires a value")))
        };
        match option {
            "--backend" => backend = Backend::parse(value(&mut index)?)?,
            "--iterations" => {
                iterations = value(&mut index)?
                    .parse::<usize>()
                    .map_err(|_| LatencyError::new("--iterations must be a positive integer"))?;
            }
            "--out" => out = Some(PathBuf::from(value(&mut index)?)),
            "--markdown" => markdown = Some(PathBuf::from(value(&mut index)?)),
            "--help" | "-h" => {
                print_latency_help();
                return Err(LatencyError::new("help requested"));
            }
            _ => {
                return Err(LatencyError::new(format!(
                    "unknown latency option {option}"
                )))
            }
        }
        index += 1;
    }

    if !(1..=MAX_ITERATIONS).contains(&iterations) {
        return Err(LatencyError::new(format!(
            "--iterations must be between 1 and {MAX_ITERATIONS}"
        )));
    }

    let postgres_url = if backend == Backend::Postgres {
        Some(
            env::var("MEMORY_ENGINE_POSTGRES_TEST_URL")
                .map_err(|_| LatencyError::new("MEMORY_ENGINE_POSTGRES_TEST_URL is unset"))?,
        )
    } else {
        None
    };

    Ok(LatencyOptions {
        backend,
        iterations,
        out,
        markdown,
        postgres_url,
    })
}

fn parse_diff_options(args: &[String]) -> Result<DiffOptions, LatencyError> {
    let mut base = None;
    let mut head = None;
    let mut budget = PathBuf::from(DEFAULT_BUDGET_PATH);
    let mut markdown = None;
    let mut index = 0;

    while index < args.len() {
        let option = args[index].as_str();
        let value = |index: &mut usize| -> Result<&str, LatencyError> {
            *index += 1;
            args.get(*index)
                .map(String::as_str)
                .ok_or_else(|| LatencyError::new(format!("{option} requires a value")))
        };
        match option {
            "--base" => base = Some(PathBuf::from(value(&mut index)?)),
            "--head" => head = Some(PathBuf::from(value(&mut index)?)),
            "--budget" => budget = PathBuf::from(value(&mut index)?),
            "--markdown" => markdown = Some(PathBuf::from(value(&mut index)?)),
            "--help" | "-h" => {
                print_diff_help();
                return Err(LatencyError::new("help requested"));
            }
            _ => return Err(LatencyError::new(format!("unknown diff option {option}"))),
        }
        index += 1;
    }

    Ok(DiffOptions {
        base: base.ok_or_else(|| LatencyError::new("diff requires --base"))?,
        head: head.ok_or_else(|| LatencyError::new("diff requires --head"))?,
        budget,
        markdown,
    })
}

fn print_latency_help() {
    println!(
        "usage: cargo run -p memory-engine-qa -- latency [--backend file|postgres] [--iterations N] [--out path] [--markdown path]"
    );
}

fn print_diff_help() {
    println!(
        "usage: cargo run -p memory-engine-qa -- diff --base path.json --head path.json [--budget path.json] [--markdown path.md]"
    );
}

fn run_latency(options: &LatencyOptions) -> Result<(), LatencyError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| LatencyError::new("could not create Tokio runtime"))?;
    let receipt = runtime.block_on(run_latency_async(options))?;
    receipt.validate(true)?;

    let json = serde_json::to_string_pretty(&receipt)?;
    if let Some(path) = options.out.as_deref() {
        write_text(path, &format!("{json}\n"))?;
    }
    if let Some(path) = options.markdown.as_deref() {
        let budgets = load_budgets(Path::new(DEFAULT_BUDGET_PATH))?;
        write_text(path, &render_receipt_markdown(&receipt, &budgets))?;
    }
    println!("{json}");
    Ok(())
}

async fn run_latency_async(options: &LatencyOptions) -> Result<LatencyReceipt, LatencyError> {
    let specs = action_specs()?;
    let mut actions = Vec::with_capacity(specs.len());

    for spec in specs {
        let mut observations = Vec::with_capacity(options.iterations);
        for iteration in 0..options.iterations {
            let observation = run_action(spec, options, iteration).await?;
            if !(200..300).contains(&observation.status) {
                return Err(LatencyError::new(format!(
                    "{} returned HTTP {}",
                    spec.name, observation.status
                )));
            }
            if spec.kind == ActionKind::ReviewSubmit && options.backend == Backend::Postgres {
                let count = observation
                    .server_timing
                    .as_ref()
                    .and_then(|timing| timing.connect_count)
                    .ok_or_else(|| {
                        LatencyError::new(
                            "review.submit did not report Postgres connect_count in Server-Timing",
                        )
                    })?;
                if count > 1 {
                    return Err(LatencyError::new(format!(
                        "{} reported {count} Postgres connections",
                        spec.name
                    )));
                }
            }
            observations.push(observation);
        }
        actions.push(action_receipt(spec, &observations));
    }

    Ok(LatencyReceipt {
        schema: RECEIPT_SCHEMA.to_owned(),
        git_sha: git_sha()?,
        recorded_at_unix_ms: unix_now_ms(),
        backend: options.backend.as_str().to_owned(),
        iterations: options.iterations,
        actions,
    })
}

fn action_specs() -> Result<Vec<ActionSpec>, LatencyError> {
    let taxonomy = Action::all();
    if ACTION_SPECS
        .iter()
        .any(|spec| !taxonomy.contains(&spec.taxonomy))
    {
        return Err(LatencyError::new(
            "latency action list is out of sync with Action::all()",
        ));
    }
    Ok(ACTION_SPECS.to_vec())
}

fn action_receipt(spec: ActionSpec, observations: &[Observation]) -> ActionReceipt {
    let samples_ms = observations
        .iter()
        .map(|observation| observation.elapsed_ms)
        .collect::<Vec<_>>();
    let server_timing = observations
        .iter()
        .find_map(|observation| observation.server_timing.clone());
    let http_status = observations
        .first()
        .map_or(0, |observation| observation.status);

    ActionReceipt {
        action: spec.name.to_owned(),
        family: spec.family.to_owned(),
        p50_ms: percentile(&samples_ms, 50),
        p95_ms: percentile(&samples_ms, 95),
        max_ms: samples_ms.iter().copied().max(),
        samples_ms,
        server_timing,
        http_status,
        skip_reason: None,
    }
}

async fn run_action(
    spec: ActionSpec,
    options: &LatencyOptions,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    match spec.kind {
        ActionKind::AuthHome => run_auth_home(options, spec, iteration).await,
        ActionKind::LoginRequest => run_login_request(options, spec, iteration).await,
        ActionKind::Capture => run_capture(options, spec, iteration).await,
        ActionKind::GenerationEnqueue => run_generation_enqueue(options, spec, iteration).await,
        ActionKind::ReviewNext => run_review_next(options, spec, iteration).await,
        ActionKind::ReviewSubmit
        | ActionKind::ReviewContentFeedback
        | ActionKind::ReviewReveal
        | ActionKind::ReviewSkip
        | ActionKind::ReviewSnooze => run_review_mutation(options, spec, iteration).await,
        ActionKind::Logout => run_logout(options, spec, iteration).await,
    }
}

async fn run_auth_home(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    timed(
        &fixture.app,
        Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .map_err(|_| LatencyError::new("could not build home request"))?,
    )
    .await
}

async fn run_login_request(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    let email = fixture_email(iteration, spec.name);
    timed(
        &fixture.app,
        form_request("POST", "/app/account", None, &[("email", &email)])?,
    )
    .await
}

async fn run_capture(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    timed(
        &fixture.app,
        form_request(
            "POST",
            "/app/start",
            None,
            &[
                ("title", SOURCE_TITLE),
                ("body", SOURCE_BODY),
                ("permission", "local-only"),
            ],
        )?,
    )
    .await
}

async fn run_generation_enqueue(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    let session = start_session(&fixture).await?;
    timed(
        &fixture.app,
        form_request_with_cookie(
            "POST",
            "/app/generate",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("sourceId", &session.source_id),
            ],
        )?,
    )
    .await
}

async fn run_review_next(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    let session = seed_review(&fixture, false).await?;
    timed(
        &fixture.app,
        form_request_with_cookie(
            "POST",
            "/app/next",
            &session.cookie,
            &[("csrfToken", &session.csrf_token)],
        )?,
    )
    .await
}

async fn run_review_mutation(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    let session = seed_review(&fixture, true).await?;
    let review_unit_id = session.review_unit_id();
    let request = match spec.kind {
        ActionKind::ReviewSubmit => form_request_with_cookie(
            "POST",
            "/app/submit",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("reviewUnitId", review_unit_id),
                ("answer", "ALFA"),
                ("responseTimeMs", "1800"),
                ("idempotencyKey", &format!("latency-submit-{iteration}")),
            ],
        )?,
        ActionKind::ReviewContentFeedback => form_request_with_cookie(
            "POST",
            "/app/content-feedback",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("reviewUnitId", review_unit_id),
                ("verdict", "kept"),
                ("rationale", "fixture"),
                ("idempotencyKey", &format!("latency-feedback-{iteration}")),
            ],
        )?,
        ActionKind::ReviewReveal => form_request_with_cookie(
            "POST",
            "/app/reveal",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("reviewUnitId", review_unit_id),
            ],
        )?,
        ActionKind::ReviewSkip => form_request_with_cookie(
            "POST",
            "/app/skip",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("reviewUnitId", review_unit_id),
            ],
        )?,
        ActionKind::ReviewSnooze => form_request_with_cookie(
            "POST",
            "/app/snooze",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("reviewUnitId", review_unit_id),
            ],
        )?,
        _ => return Err(LatencyError::new("invalid review action")),
    };
    timed(&fixture.app, request).await
}

async fn run_logout(
    options: &LatencyOptions,
    spec: ActionSpec,
    iteration: usize,
) -> Result<Observation, LatencyError> {
    let fixture = Fixture::new(options, spec.name, iteration)?;
    let session = start_session(&fixture).await?;
    timed(
        &fixture.app,
        form_request_with_cookie(
            "POST",
            "/app/logout",
            &session.cookie,
            &[("csrfToken", &session.csrf_token)],
        )?,
    )
    .await
}

impl Session {
    fn review_unit_id(&self) -> &str {
        self.source_id.as_str()
    }
}

async fn start_session(fixture: &Fixture) -> Result<Session, LatencyError> {
    let response = send(
        &fixture.app,
        form_request(
            "POST",
            "/app/start",
            None,
            &[
                ("title", SOURCE_TITLE),
                ("body", SOURCE_BODY),
                ("permission", "local-only"),
            ],
        )?,
    )
    .await?;
    let (status, _timing, body) = finish(response).await?;
    if status != StatusCode::OK.as_u16() {
        return Err(LatencyError::new(format!(
            "fixture start returned HTTP {status}"
        )));
    }
    let cookie = cookie_from_headers(&body.0)?;
    let csrf_token = html_value(&body.1, "csrfToken")?;

    let library = send(&fixture.app, get_request("/app/library", Some(&cookie))?).await?;
    let (status, _timing, library_body) = finish(library).await?;
    if status != StatusCode::OK.as_u16() {
        return Err(LatencyError::new(format!(
            "fixture library returned HTTP {status}"
        )));
    }
    let source_id = html_value(&library_body.1, "sourceId")?;

    Ok(Session {
        cookie,
        csrf_token,
        source_id,
    })
}

async fn seed_review(fixture: &Fixture, open_review: bool) -> Result<Session, LatencyError> {
    let mut session = start_session(fixture).await?;
    let generated = send(
        &fixture.app,
        form_request_with_cookie(
            "POST",
            "/app/generate",
            &session.cookie,
            &[
                ("csrfToken", &session.csrf_token),
                ("sourceId", &session.source_id),
            ],
        )?,
    )
    .await?;
    let (status, _timing, _body) = finish(generated).await?;
    if status != StatusCode::OK.as_u16() {
        return Err(LatencyError::new(format!(
            "fixture generation returned HTTP {status}"
        )));
    }
    fixture.state.run_pending_jobs_blocking();
    // Local-only generation leaves deterministic drafts pending. Keep them
    // through the browser form so the review queue has real due cards.
    let workspace = send(&fixture.app, get_request("/", Some(&session.cookie))?).await?;
    let (workspace_status, _timing, workspace_body) = finish(workspace).await?;
    if workspace_status != StatusCode::OK.as_u16() {
        return Err(LatencyError::new(format!(
            "fixture workspace returned HTTP {workspace_status}"
        )));
    }
    for draft_id in html_values(&workspace_body.1, "draftId") {
        let kept = send(
            &fixture.app,
            form_request_with_cookie(
                "POST",
                "/app/draft/keep",
                &session.cookie,
                &[("csrfToken", &session.csrf_token), ("draftId", &draft_id)],
            )?,
        )
        .await?;
        let (kept_status, _timing, _body) = finish(kept).await?;
        if kept_status != StatusCode::OK.as_u16() {
            return Err(LatencyError::new(format!(
                "fixture draft keep returned HTTP {kept_status}"
            )));
        }
    }

    if open_review {
        let next = send(
            &fixture.app,
            form_request_with_cookie(
                "POST",
                "/app/next",
                &session.cookie,
                &[("csrfToken", &session.csrf_token)],
            )?,
        )
        .await?;
        let (status, _timing, body) = finish(next).await?;
        if status != StatusCode::OK.as_u16() {
            return Err(LatencyError::new(format!(
                "fixture review next returned HTTP {status}"
            )));
        }
        session.source_id = html_value(&body.1, "reviewUnitId")?;
    }

    Ok(session)
}

async fn timed(app: &Router, request: Request<Body>) -> Result<Observation, LatencyError> {
    let started = Instant::now();
    let response = send(app, request).await?;
    let (status, server_timing, _body) = finish(response).await?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(Observation {
        elapsed_ms,
        status,
        server_timing,
    })
}

async fn send(app: &Router, request: Request<Body>) -> Result<Response<Body>, LatencyError> {
    app.clone()
        .oneshot(request)
        .await
        .map_err(|_| LatencyError::new("in-process router request failed"))
}

async fn finish(
    response: Response<Body>,
) -> Result<(u16, Option<ServerTiming>, (HeaderMap, String)), LatencyError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let server_timing = parse_server_timing(&headers)?;
    let body = to_bytes(response.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            LatencyError::new("router response body exceeded the latency harness limit")
        })?;
    let body = String::from_utf8(body.to_vec())
        .map_err(|_| LatencyError::new("router response was not UTF-8 HTML"))?;
    Ok((status, server_timing, (headers, body)))
}

fn parse_server_timing(headers: &HeaderMap) -> Result<Option<ServerTiming>, LatencyError> {
    let Some(value) = headers.get("server-timing") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| LatencyError::new("server-timing header was not valid ASCII"))?;
    let mut timing = ServerTiming {
        total_ms: None,
        pgconnect_ms: None,
        pgop_ms: None,
        render_ms: None,
        connect_count: None,
    };
    for entry in value.split(',') {
        let mut parts = entry.trim().split(';');
        let name = parts.next().unwrap_or_default().trim();
        for parameter in parts {
            let Some((key, raw_value)) = parameter.trim().split_once('=') else {
                continue;
            };
            let parsed = raw_value.trim().trim_matches('"').parse::<u64>().ok();
            match (name, key.trim(), parsed) {
                ("total", "dur", Some(value)) => timing.total_ms = Some(value),
                ("pgconnect", "dur", Some(value)) => timing.pgconnect_ms = Some(value),
                ("pgop", "dur", Some(value)) => timing.pgop_ms = Some(value),
                ("render", "dur", Some(value)) => timing.render_ms = Some(value),
                ("pgconn", "desc", Some(value)) => timing.connect_count = Some(value),
                _ => {}
            }
        }
    }
    Ok(Some(timing))
}

fn get_request(uri: &str, cookie: Option<&str>) -> Result<Request<Body>, LatencyError> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    builder
        .body(Body::empty())
        .map_err(|_| LatencyError::new("could not build GET request"))
}

fn form_request(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    fields: &[(&str, &str)],
) -> Result<Request<Body>, LatencyError> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    builder
        .body(Body::from(form_body(fields)))
        .map_err(|_| LatencyError::new("could not build form request"))
}

fn form_request_with_cookie(
    method: &str,
    uri: &str,
    cookie: &str,
    fields: &[(&str, &str)],
) -> Result<Request<Body>, LatencyError> {
    form_request(method, uri, Some(cookie), fields)
}

fn form_body(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", form_escape(name), form_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn form_escape(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![char::from(byte)]
            }
            b' ' => vec!['+'],
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn cookie_from_headers(headers: &HeaderMap) -> Result<String, LatencyError> {
    headers
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_owned)
        .ok_or_else(|| LatencyError::new("fixture response did not set a browser session cookie"))
}

fn html_value(html: &str, name: &str) -> Result<String, LatencyError> {
    let marker = format!(r#"name="{name}" value=""#);
    let start = html
        .find(&marker)
        .map(|offset| offset + marker.len())
        .ok_or_else(|| LatencyError::new(format!("fixture HTML omitted {name}")))?;
    let end = html[start..]
        .find('"')
        .map(|offset| offset + start)
        .ok_or_else(|| LatencyError::new(format!("fixture HTML truncated {name}")))?;
    Ok(html[start..end].to_owned())
}

fn html_values(html: &str, name: &str) -> Vec<String> {
    let marker = format!(r#"name="{name}" value=""#);
    let mut values = Vec::new();
    let mut remaining = html;
    while let Some(start) = remaining.find(&marker) {
        let value_start = start + marker.len();
        let Some(end) = remaining[value_start..].find('"') else {
            break;
        };
        let value = &remaining[value_start..value_start + end];
        if !value.is_empty() && !values.iter().any(|existing| existing == value) {
            values.push(value.to_owned());
        }
        remaining = &remaining[value_start + end + 1..];
    }
    values
}

impl Fixture {
    fn new(options: &LatencyOptions, action: &str, iteration: usize) -> Result<Self, LatencyError> {
        let auth = AuthConfig::for_local_tests().with_debug_links(true);
        match options.backend {
            Backend::File => {
                let root = temp_store_root(action, iteration);
                fs::create_dir_all(&root)?;
                let state =
                    ApiState::new(AccountRegistry::with_store_root(&root).with_auth_config(auth));
                let app = router(state.clone());
                Ok(Self {
                    app,
                    state,
                    root: Some(root),
                })
            }
            Backend::Postgres => {
                let url = options
                    .postgres_url
                    .as_deref()
                    .ok_or_else(|| LatencyError::new("Postgres URL is unavailable"))?;
                let state =
                    ApiState::new(AccountRegistry::with_postgres_url(url).with_auth_config(auth));
                let app = router(state.clone());
                Ok(Self {
                    app,
                    state,
                    root: None,
                })
            }
        }
    }
}

fn temp_store_root(action: &str, iteration: usize) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    env::temp_dir().join(format!(
        "memory-engine-action-latency-{}-{action}-{iteration}-{nonce}",
        std::process::id()
    ))
}

fn fixture_email(iteration: usize, action: &str) -> String {
    let safe_action = action.replace('.', "-");
    format!("latency-{safe_action}-{iteration}@example.test")
}

fn percentile(samples: &[u64], percentile: usize) -> Option<u64> {
    if samples.is_empty() || !(1..=100).contains(&percentile) {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * percentile).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

fn git_sha() -> Result<String, LatencyError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|_| LatencyError::new("could not read git SHA"))?;
    if !output.status.success() {
        return Err(LatencyError::new("git rev-parse HEAD failed"));
    }
    let sha = String::from_utf8(output.stdout)
        .map_err(|_| LatencyError::new("git SHA was not UTF-8"))?
        .trim()
        .to_owned();
    if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LatencyError::new(
            "git SHA was not a full hexadecimal commit",
        ));
    }
    Ok(sha)
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn write_text(path: &Path, content: &str) -> Result<(), LatencyError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

impl LatencyReceipt {
    fn validate(&self, require_actions: bool) -> Result<(), LatencyError> {
        if self.schema != RECEIPT_SCHEMA {
            return Err(LatencyError::new(
                "receipt schema is not action_latency_receipt.v1",
            ));
        }
        if self.git_sha.len() != 40 || !self.git_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(LatencyError::new(
                "receipt git_sha is not a full hexadecimal commit",
            ));
        }
        if !matches!(self.backend.as_str(), "file" | "postgres") {
            return Err(LatencyError::new("receipt backend is not file or postgres"));
        }
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return Err(LatencyError::new(
                "receipt iterations is outside the supported range",
            ));
        }

        let known = action_specs()?
            .into_iter()
            .map(|spec| spec.name.to_owned())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        for action in &self.actions {
            if !known.contains(&action.action) {
                return Err(LatencyError::new(format!(
                    "receipt contains unknown action {}",
                    action.action
                )));
            }
            if !seen.insert(action.action.clone()) {
                return Err(LatencyError::new(format!(
                    "receipt contains duplicate action {}",
                    action.action
                )));
            }
            if action.samples_ms.is_empty() {
                if action.skip_reason.as_deref() != Some("generation fixture unavailable") {
                    return Err(LatencyError::new(
                        "empty action samples require the fixed generation skip reason",
                    ));
                }
                if action.p50_ms.is_some() || action.p95_ms.is_some() || action.max_ms.is_some() {
                    return Err(LatencyError::new(
                        "skipped action cannot carry percentile values",
                    ));
                }
                continue;
            }
            if action.samples_ms.len() != self.iterations {
                return Err(LatencyError::new(format!(
                    "{} has {} samples, expected {}",
                    action.action,
                    action.samples_ms.len(),
                    self.iterations
                )));
            }
            if action.skip_reason.is_some() {
                return Err(LatencyError::new(
                    "completed action cannot carry a skip reason",
                ));
            }
            if action.http_status < 200 || action.http_status >= 600 {
                return Err(LatencyError::new(format!(
                    "{} has an invalid HTTP status",
                    action.action
                )));
            }
            if action.p50_ms != percentile(&action.samples_ms, 50)
                || action.p95_ms != percentile(&action.samples_ms, 95)
                || action.max_ms != action.samples_ms.iter().copied().max()
            {
                return Err(LatencyError::new(format!(
                    "{} percentile values do not match samples",
                    action.action
                )));
            }
            if action.family.is_empty() {
                return Err(LatencyError::new(format!(
                    "{} has no action family",
                    action.action
                )));
            }
        }
        if require_actions && seen != known {
            return Err(LatencyError::new(
                "receipt does not cover the required action subset",
            ));
        }
        Ok(())
    }
}

fn load_receipt(path: &Path) -> Result<LatencyReceipt, LatencyError> {
    let receipt: LatencyReceipt = serde_json::from_str(&fs::read_to_string(path)?)?;
    receipt.validate(true)?;
    Ok(receipt)
}

fn load_budgets(path: &Path) -> Result<BudgetsFile, LatencyError> {
    let budgets: BudgetsFile = serde_json::from_str(&fs::read_to_string(path)?)?;
    if budgets.schema != BUDGET_SCHEMA {
        return Err(LatencyError::new(
            "budget schema is not action_latency_budgets.v1",
        ));
    }
    let expected = action_specs()?
        .into_iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    let actual = budgets
        .actions
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(LatencyError::new(
            "budgets do not cover the required action subset",
        ));
    }
    for (action, budget) in &budgets.actions {
        if budget.p95_ms.soft == 0 || budget.p95_ms.hard < budget.p95_ms.soft {
            return Err(LatencyError::new(format!(
                "invalid p95 budget for {action}"
            )));
        }
    }
    Ok(budgets)
}

fn run_diff(options: &DiffOptions) -> Result<(), LatencyError> {
    let base = load_receipt(&options.base)?;
    let head = load_receipt(&options.head)?;
    if base.backend != head.backend {
        return Err(LatencyError::new("base and head backends differ"));
    }
    let budgets = load_budgets(&options.budget)?;
    let report = compare_receipts(&base, &head, &budgets)?;
    let markdown = render_diff_markdown(&base, &head, &report);
    if let Some(path) = options.markdown.as_deref() {
        write_text(path, &markdown)?;
    }
    print!("{markdown}");
    if report.hard_fail {
        return Err(LatencyError::new(
            "one or more action latency hard budgets failed",
        ));
    }
    Ok(())
}

fn percent_delta(base: Option<u64>, head: Option<u64>) -> Option<i64> {
    match (base, head) {
        (Some(0), Some(head)) if head > 0 => Some(100),
        (Some(base), Some(head)) if base > 0 => {
            let ratio = u128::from(head)
                .saturating_mul(100)
                .checked_div(u128::from(base))
                .unwrap_or(u128::MAX);
            i64::try_from(ratio).map_or(Some(i64::MAX), |value| Some(value.saturating_sub(100)))
        }
        _ => None,
    }
}

fn compare_receipts(
    base: &LatencyReceipt,
    head: &LatencyReceipt,
    budgets: &BudgetsFile,
) -> Result<DiffReport, LatencyError> {
    let base_actions = base
        .actions
        .iter()
        .map(|action| (action.action.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::with_capacity(head.actions.len());
    let mut hard_fail = false;

    for action in &head.actions {
        let Some(base_action) = base_actions.get(action.action.as_str()) else {
            return Err(LatencyError::new(format!(
                "baseline is missing {}",
                action.action
            )));
        };
        let budget = budgets
            .actions
            .get(&action.action)
            .ok_or_else(|| LatencyError::new(format!("budget is missing {}", action.action)))?
            .p95_ms
            .clone();
        let head_p95 = action.p95_ms;
        let base_p95 = base_action.p95_ms;
        let delta_percent = percent_delta(base_p95, head_p95);
        let hard = head_p95.is_some_and(|value| value > budget.hard);
        let soft = match (base_p95, head_p95) {
            (Some(base), Some(head)) if base > 0 => {
                head.saturating_mul(100) > base.saturating_mul(120)
            }
            (Some(0), Some(head)) => head > 0,
            _ => false,
        };
        hard_fail |= hard;
        rows.push(DiffRow {
            action: action.action.clone(),
            family: action.family.clone(),
            base_p50: base_action.p50_ms,
            head_p50: action.p50_ms,
            base_p95,
            head_p95,
            delta_percent,
            budget,
            hard_fail: hard,
            soft_regression: soft,
        });
    }
    Ok(DiffReport { rows, hard_fail })
}

fn render_receipt_markdown(receipt: &LatencyReceipt, budgets: &BudgetsFile) -> String {
    let mut markdown = format!(
        "# Core action latency\n\n- Schema: `{}`\n- Git SHA: `{}`\n- Backend: `{}`\n- Iterations: `{}`\n\n| Action | Family | p50 (ms) | p95 (ms) | Max (ms) | Soft p95 | Hard p95 | Status |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |\n",
        receipt.schema, receipt.git_sha, receipt.backend, receipt.iterations
    );
    for action in &receipt.actions {
        let budget = budgets.actions.get(&action.action);
        let status = if action.skip_reason.is_some() {
            "SKIPPED"
        } else if budget
            .is_some_and(|budget| action.p95_ms.is_some_and(|p95| p95 > budget.p95_ms.hard))
        {
            "HARD FAIL"
        } else if budget
            .is_some_and(|budget| action.p95_ms.is_some_and(|p95| p95 > budget.p95_ms.soft))
        {
            "SOFT WARN"
        } else {
            "PASS"
        };
        let _ = writeln!(
            markdown,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
            action.action,
            action.family,
            display_number(action.p50_ms),
            display_number(action.p95_ms),
            display_number(action.max_ms),
            budget.map_or_else(|| "-".to_owned(), |value| value.p95_ms.soft.to_string()),
            budget.map_or_else(|| "-".to_owned(), |value| value.p95_ms.hard.to_string()),
            status
        );
    }
    markdown
}

fn render_diff_markdown(
    base: &LatencyReceipt,
    head: &LatencyReceipt,
    report: &DiffReport,
) -> String {
    let mut markdown = format!(
        "# Core action latency diff\n\n- Base: `{}`\n- Head: `{}`\n- Backend: `{}`\n\n| Action | Family | Base p50 | Head p50 | Base p95 | Head p95 | p95 delta | Hard budget | Result |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
        base.git_sha, head.git_sha, head.backend
    );
    for row in &report.rows {
        let result = if row.hard_fail {
            "HARD FAIL"
        } else if row.soft_regression {
            "SOFT WARN"
        } else {
            "PASS"
        };
        let delta = row
            .delta_percent
            .map_or_else(|| "-".to_owned(), |value| format!("{value:+}%"));
        let _ = writeln!(
            markdown,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} |",
            row.action,
            row.family,
            display_number(row.base_p50),
            display_number(row.head_p50),
            display_number(row.base_p95),
            display_number(row.head_p95),
            delta,
            row.budget.hard,
            result
        );
    }
    markdown
}

fn display_number(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_receipt(p95: u64) -> LatencyReceipt {
        let actions = ACTION_SPECS
            .iter()
            .map(|spec| ActionReceipt {
                action: spec.name.to_owned(),
                family: spec.family.to_owned(),
                samples_ms: vec![10, 12, p95],
                p50_ms: Some(12),
                p95_ms: Some(p95),
                max_ms: Some(p95),
                server_timing: None,
                http_status: 200,
                skip_reason: None,
            })
            .collect();
        LatencyReceipt {
            schema: RECEIPT_SCHEMA.to_owned(),
            git_sha: "a".repeat(40),
            recorded_at_unix_ms: 1,
            backend: "file".to_owned(),
            iterations: 3,
            actions,
        }
    }

    fn sample_budgets(hard: u64) -> BudgetsFile {
        BudgetsFile {
            schema: BUDGET_SCHEMA.to_owned(),
            actions: ACTION_SPECS
                .iter()
                .map(|spec| {
                    (
                        spec.name.to_owned(),
                        BudgetEntry {
                            p95_ms: BudgetThresholds { soft: 10, hard },
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn action_specs_are_a_closed_subset_of_action_all() {
        let specs = action_specs().expect("taxonomy");
        assert_eq!(specs.len(), ACTION_SPECS.len());
        assert!(specs
            .iter()
            .all(|spec| Action::all().contains(&spec.taxonomy)));
    }

    #[test]
    fn percentile_uses_nearest_rank_for_small_samples() {
        assert_eq!(percentile(&[12, 11, 13], 50), Some(12));
        assert_eq!(percentile(&[12, 11, 13], 95), Some(13));
        assert_eq!(percentile(&[12, 11, 13], 100), Some(13));
    }

    #[test]
    fn receipt_round_trip_validates_schema_and_safe_fields() {
        let receipt = sample_receipt(13);
        receipt.validate(true).expect("receipt validates");
        let json = serde_json::to_string(&receipt).expect("receipt JSON");
        assert!(!json.contains("email"));
        assert!(!json.contains("token"));
        assert!(!json.contains("ALFA"));
        let decoded: LatencyReceipt = serde_json::from_str(&json).expect("receipt round trip");
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn diff_reports_soft_regression_without_hard_failure() {
        let base = sample_receipt(100);
        let head = sample_receipt(125);
        let budgets = sample_budgets(200);
        let report = compare_receipts(&base, &head, &budgets).expect("diff");
        assert!(!report.hard_fail);
        assert!(report.rows.iter().all(|row| row.soft_regression));
        assert!(render_diff_markdown(&base, &head, &report).contains("SOFT WARN"));
    }

    #[test]
    fn diff_reports_hard_budget_failure() {
        let base = sample_receipt(100);
        let head = sample_receipt(201);
        let budgets = sample_budgets(200);
        let report = compare_receipts(&base, &head, &budgets).expect("diff");
        assert!(report.hard_fail);
        assert!(report.rows.iter().all(|row| row.hard_fail));
    }

    #[test]
    fn server_timing_parses_connect_count_without_raw_identifiers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "server-timing",
            "request;desc=\"req_opaque\", total;dur=10, pgconnect;dur=3, pgop;dur=5, pgconn;desc=\"1\", render;dur=1"
                .parse()
                .expect("header"),
        );
        let timing = parse_server_timing(&headers)
            .expect("timing")
            .expect("timing present");
        assert_eq!(timing.total_ms, Some(10));
        assert_eq!(timing.pgconnect_ms, Some(3));
        assert_eq!(timing.pgop_ms, Some(5));
        assert_eq!(timing.render_ms, Some(1));
        assert_eq!(timing.connect_count, Some(1));
    }
}
