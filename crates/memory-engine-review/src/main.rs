//! `memory-engine-review` — the morning-review dogfood client.
//!
//! A brutally thin CLI over the deployed `memory-engine-api` v1 contract: it
//! authenticates once, then drives `review/next` -> answer -> `review/submit`
//! in a loop until the account's due queue is empty. It keeps streak and
//! cold-recall evidence in a local NDJSON log; it does not add any new server
//! surface. See `docs/dogfood/morning-review-cli.md` for the falsifier this
//! client exists to serve.

use std::{
    env,
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use memory_engine_credentials::DEFAULT_BASE_URL;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Safety cap on cards reviewed in a single run. A real due queue should
/// empty out long before this; it exists only to stop a server bug (a queue
/// that never reaches zero) from looping the CLI forever.
const DEFAULT_MAX_CARDS: usize = 200;
const SECONDS_PER_DAY: i64 = 86_400;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let outcome = dispatch(&args, &mut io::stdin().lock(), &mut io::stdout());

    match outcome {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn dispatch(
    args: &[String],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
) -> Result<(), CliFailure> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }

    let (subcommand, rest) = match args.first().map(String::as_str) {
        Some("login") => ("login", &args[1..]),
        Some("streak") => ("streak", &args[1..]),
        Some("review") => ("review", &args[1..]),
        Some(other) if other.starts_with('-') => ("review", args),
        None => ("review", args),
        Some(_) => {
            return Err(CliFailure(format!(
                "unknown subcommand {:?}; expected login, review, or streak",
                args[0]
            )));
        }
    };

    match subcommand {
        "login" => run_login(rest),
        "streak" => run_streak(rest, stdout),
        _ => run_review(rest, stdin, stdout).map(|_receipt| ()),
    }
}

fn print_usage() {
    println!(
        "usage:\n  memory-engine-review login --email EMAIL [--base-url URL]\n  memory-engine-review login --account-id ID --session-token TOKEN [--base-url URL]\n  memory-engine-review [review] [--base-url URL] [--max-cards N]\n  memory-engine-review streak [--days N]\n\nCredentials resolve in order: MEMORY_ENGINE_ACCOUNT_ID/MEMORY_ENGINE_SESSION_TOKEN\nenv vars, then the file written by `login` (default {}).",
        default_credentials_path().display()
    );
}

// ---------------------------------------------------------------------------
// login
// ---------------------------------------------------------------------------

fn run_login(args: &[String]) -> Result<(), CliFailure> {
    let mut base_url = DEFAULT_BASE_URL.to_owned();
    let mut email = None;
    let mut account_id = None;
    let mut session_token = None;
    let mut credentials_path = default_credentials_path();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliFailure(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--base-url" => base_url.clone_from(value),
            "--email" => email = Some(value.clone()),
            "--account-id" => account_id = Some(value.clone()),
            "--session-token" => session_token = Some(value.clone()),
            "--credentials-path" => credentials_path = PathBuf::from(value),
            other => return Err(CliFailure(format!("unknown argument {other}"))),
        }
        index += 2;
    }

    let credentials = match (email, account_id, session_token) {
        (Some(email), None, None) => {
            let agent = build_agent();
            let created = create_account(&agent, &base_url, &email).map_err(|error| {
                CliFailure(format!(
                    "{error}\n\nIf this account already exists in production, fetch its \
                     existing session token (see docs/dogfood/morning-review-cli.md, \
                     \"Recovering an existing session token\") and run:\n  \
                     memory-engine-review login --account-id <id> --session-token <token>"
                ))
            })?;
            StoredCredentials {
                base_url,
                account_id: created.account_id,
                session_token: created.session_token,
            }
        }
        (None, Some(account_id), Some(session_token)) => StoredCredentials {
            base_url,
            account_id,
            session_token,
        },
        (None, None, None) => {
            return Err(CliFailure(
                "login requires --email, or --account-id and --session-token together".to_owned(),
            ));
        }
        _ => {
            return Err(CliFailure(
                "pass either --email alone, or --account-id and --session-token together"
                    .to_owned(),
            ));
        }
    };

    write_credentials(&credentials_path, &credentials)?;
    println!(
        "Saved credentials for account {} to {} (base url {}).",
        credentials.account_id,
        credentials_path.display(),
        credentials.base_url
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// review
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewSessionReceipt {
    reviewed_count: usize,
    due_count_at_start: usize,
    completed: bool,
    stopped_reason: Option<String>,
}

#[allow(clippy::too_many_lines)]
fn run_review(
    args: &[String],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
) -> Result<ReviewSessionReceipt, CliFailure> {
    let mut base_url_override = None;
    let mut credentials_path = default_credentials_path();
    let mut log_path = default_log_path();
    let mut max_cards = DEFAULT_MAX_CARDS;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliFailure(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--base-url" => base_url_override = Some(value.clone()),
            "--credentials-path" => credentials_path = PathBuf::from(value),
            "--log-path" => log_path = PathBuf::from(value),
            "--max-cards" => {
                max_cards = value.parse().map_err(|_| {
                    CliFailure(format!("--max-cards value {value} is not a number"))
                })?;
            }
            other => return Err(CliFailure(format!("unknown argument {other}"))),
        }
        index += 2;
    }

    let session = resolve_session(base_url_override, &credentials_path)?;
    let agent = build_agent();
    let client = ReviewClient::new(
        agent,
        session.base_url.clone(),
        session.account_id.clone(),
        session.token,
    );

    let mut receipt = ReviewSessionReceipt::default();
    let mut due_count_at_start = None;

    loop {
        let view = client.next_review()?;
        if due_count_at_start.is_none() {
            due_count_at_start = Some(view.due_count);
            receipt.due_count_at_start = view.due_count;
        }

        let Some(current) = view.current else {
            receipt.completed = true;
            break;
        };
        if view.due_count == 0 {
            receipt.completed = true;
            break;
        }
        if receipt.reviewed_count >= max_cards {
            receipt.stopped_reason = Some(format!(
                "reached the --max-cards safety cap ({max_cards}) before dueCount reached 0"
            ));
            break;
        }

        writeln!(stdout, "\n[{} due] {}", view.due_count, current.prompt).map_err(io_failure)?;
        if !current.choices.is_empty() {
            for (position, choice) in current.choices.iter().enumerate() {
                writeln!(stdout, "  {}. {choice}", position + 1).map_err(io_failure)?;
            }
        }
        write!(stdout, "> ").map_err(io_failure)?;
        stdout.flush().map_err(io_failure)?;

        let started_at = Instant::now();
        let mut answer = String::new();
        let bytes_read = stdin.read_line(&mut answer).map_err(io_failure)?;
        if bytes_read == 0 {
            receipt.stopped_reason = Some("stdin closed before the due queue reached 0".to_owned());
            break;
        }
        let answer = answer.trim().to_owned();
        let response_time_ms = clamp_response_time_ms(started_at.elapsed());

        let idempotency_key = format!(
            "review-cli-{}-{}-{}",
            current.review_unit_id,
            unique_suffix(),
            receipt.reviewed_count
        );
        let submitted = client.submit_review(
            &current.review_unit_id,
            &answer,
            response_time_ms,
            &idempotency_key,
        )?;
        let graded = submitted
            .current
            .as_ref()
            .and_then(|current| current.grade.clone())
            .ok_or_else(|| CliFailure("submit response carried no grade".to_owned()))?;
        let schedule_change = submitted
            .current
            .as_ref()
            .and_then(|current| current.schedule_change.clone());
        let expected_answer = submitted
            .current
            .as_ref()
            .and_then(|current| current.expected_answer.clone());

        writeln!(
            stdout,
            "  -> {} (rating {}){}",
            graded.verdict,
            graded.rating,
            if graded.is_correct {
                String::new()
            } else {
                expected_answer
                    .as_deref()
                    .map(|answer| format!(" — expected: {answer}"))
                    .unwrap_or_default()
            }
        )
        .map_err(io_failure)?;

        let cold = schedule_change
            .as_ref()
            .and_then(|change| change.before.as_ref())
            .is_some_and(|before| before.reps >= 1);
        append_streak_event(
            &log_path,
            &StreakEvent::Attempt {
                ts_ms: now_ms(),
                account_id: session.account_id.clone(),
                review_unit_id: current.review_unit_id.clone(),
                verdict: graded.verdict.clone(),
                rating: graded.rating,
                is_correct: graded.is_correct,
                cold,
                due_count_after: submitted.due_count,
            },
        )?;

        receipt.reviewed_count += 1;
    }

    if receipt.completed {
        append_streak_event(
            &log_path,
            &StreakEvent::Session {
                ts_ms: now_ms(),
                account_id: session.account_id.clone(),
                reviewed_count: receipt.reviewed_count,
                due_count_at_start: receipt.due_count_at_start,
            },
        )?;
        writeln!(
            stdout,
            "\nAll caught up. Reviewed {} card(s).",
            receipt.reviewed_count
        )
        .map_err(io_failure)?;
    } else if let Some(reason) = &receipt.stopped_reason {
        writeln!(
            stdout,
            "\nStopped after {} card(s) without reaching dueCount 0: {reason}",
            receipt.reviewed_count
        )
        .map_err(io_failure)?;
    }

    Ok(receipt)
}

fn clamp_response_time_ms(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_millis())
        .unwrap_or(u32::MAX)
        .max(1)
}

// ---------------------------------------------------------------------------
// streak report
// ---------------------------------------------------------------------------

fn run_streak(args: &[String], stdout: &mut impl Write) -> Result<(), CliFailure> {
    let mut log_path = default_log_path();
    let mut days: i64 = 30;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliFailure(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--log-path" => log_path = PathBuf::from(value),
            "--days" => {
                days = value
                    .parse()
                    .map_err(|_| CliFailure(format!("--days value {value} is not a number")))?;
            }
            other => return Err(CliFailure(format!("unknown argument {other}"))),
        }
        index += 2;
    }

    let report = streak_report(&log_path, days, now_ms())?;
    let serialized =
        serde_json::to_string_pretty(&report).map_err(|error| CliFailure(error.to_string()))?;
    writeln!(stdout, "{serialized}").map_err(io_failure)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreakReport {
    window_days: i64,
    completed_days: i64,
    hit_rate: String,
    current_streak_days: i64,
    current_streak_as_of: String,
    cold_attempts: usize,
    cold_correct: usize,
    cold_recall_rate: String,
}

fn streak_report(
    log_path: &Path,
    window_days: i64,
    now_ms: i64,
) -> Result<StreakReport, CliFailure> {
    let today = day_index_from_ms(now_ms);
    let window_start = today - window_days + 1;

    // Unbounded: a real streak can run longer than the reporting window, so
    // the day set that feeds the backward walk must never be filtered by
    // `window_days` — only the window-scoped counters below are.
    let mut completed_by_day = std::collections::BTreeSet::new();
    let mut completed_days_in_window = 0i64;
    let mut cold_attempts = 0usize;
    let mut cold_correct = 0usize;

    for event in read_streak_events(log_path)? {
        let day = day_index_from_ms(event.ts_ms());
        match event {
            StreakEvent::Session { .. } => {
                if day <= today && completed_by_day.insert(day) && day >= window_start {
                    completed_days_in_window += 1;
                }
            }
            StreakEvent::Attempt {
                cold, is_correct, ..
            } => {
                if cold && day >= window_start && day <= today {
                    cold_attempts += 1;
                    if is_correct {
                        cold_correct += 1;
                    }
                }
            }
        }
    }

    let hit_rate = rate_string(completed_days_in_window, window_days);

    let (current_streak_days, current_streak_as_of) = {
        let anchor = if completed_by_day.contains(&today) {
            today
        } else if completed_by_day.contains(&(today - 1)) {
            today - 1
        } else {
            let (year, month, day) = civil_from_days(today);
            return Ok(StreakReport {
                window_days,
                completed_days: completed_days_in_window,
                hit_rate,
                current_streak_days: 0,
                current_streak_as_of: format_date(year, month, day),
                cold_attempts,
                cold_correct,
                cold_recall_rate: cold_recall_rate_string(cold_correct, cold_attempts),
            });
        };
        let mut streak = 0i64;
        let mut cursor = anchor;
        while completed_by_day.contains(&cursor) {
            streak += 1;
            cursor -= 1;
        }
        let (year, month, day) = civil_from_days(anchor);
        (streak, format_date(year, month, day))
    };

    Ok(StreakReport {
        window_days,
        completed_days: completed_days_in_window,
        hit_rate,
        current_streak_days,
        current_streak_as_of,
        cold_attempts,
        cold_correct,
        cold_recall_rate: cold_recall_rate_string(cold_correct, cold_attempts),
    })
}

#[allow(clippy::cast_precision_loss)]
fn rate_string(numerator: i64, denominator: i64) -> String {
    if denominator <= 0 {
        return "n/a".to_owned();
    }
    format!("{:.0}%", (numerator as f64 / denominator as f64) * 100.0)
}

fn cold_recall_rate_string(correct: usize, attempts: usize) -> String {
    rate_string(
        i64::try_from(correct).unwrap_or(i64::MAX),
        i64::try_from(attempts).unwrap_or(i64::MAX),
    )
}

// ---------------------------------------------------------------------------
// streak log (NDJSON)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum StreakEvent {
    Attempt {
        ts_ms: i64,
        account_id: String,
        review_unit_id: String,
        verdict: String,
        rating: u8,
        is_correct: bool,
        cold: bool,
        due_count_after: usize,
    },
    Session {
        ts_ms: i64,
        account_id: String,
        reviewed_count: usize,
        due_count_at_start: usize,
    },
}

impl StreakEvent {
    fn ts_ms(&self) -> i64 {
        match self {
            Self::Attempt { ts_ms, .. } | Self::Session { ts_ms, .. } => *ts_ms,
        }
    }
}

fn append_streak_event(log_path: &Path, event: &StreakEvent) -> Result<(), CliFailure> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliFailure(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    let line = serde_json::to_string(event).map_err(|error| CliFailure(error.to_string()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| CliFailure(format!("could not open {}: {error}", log_path.display())))?;
    writeln!(file, "{line}")
        .map_err(|error| CliFailure(format!("could not write {}: {error}", log_path.display())))
}

fn read_streak_events(log_path: &Path) -> Result<Vec<StreakEvent>, CliFailure> {
    let contents = match fs::read_to_string(log_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CliFailure(format!(
                "could not read {}: {error}",
                log_path.display()
            )))
        }
    };
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| CliFailure(format!("malformed streak log line: {error}")))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// credentials
// ---------------------------------------------------------------------------

use memory_engine_credentials::StoredCredentials;

struct Session {
    base_url: String,
    account_id: String,
    token: String,
}

fn resolve_session(
    base_url_override: Option<String>,
    credentials_path: &Path,
) -> Result<Session, CliFailure> {
    if let Some(session) =
        memory_engine_credentials::env_session(base_url_override.clone(), DEFAULT_BASE_URL)
    {
        return Ok(Session {
            base_url: session.base_url,
            account_id: session.account_id,
            token: session.session_token,
        });
    }

    let stored = read_credentials(credentials_path)?.ok_or_else(|| {
        CliFailure(format!(
            "no credentials found (checked MEMORY_ENGINE_ACCOUNT_ID/MEMORY_ENGINE_SESSION_TOKEN \
             and {}). Run `memory-engine-review login --email you@example.com` first.",
            credentials_path.display()
        ))
    })?;

    Ok(Session {
        base_url: base_url_override.unwrap_or(stored.base_url),
        account_id: stored.account_id,
        token: stored.session_token,
    })
}

/// Shared with `memory-engine-mcp`: logging in here is enough for a
/// freshly started MCP server to pick up the same account too, no manual
/// copying between the two clients' credential files.
fn default_credentials_path() -> PathBuf {
    memory_engine_credentials::default_credentials_path()
}

fn default_log_path() -> PathBuf {
    memory_engine_credentials::memory_engine_home()
        .join("review")
        .join("streak.ndjson")
}

fn read_credentials(path: &Path) -> Result<Option<StoredCredentials>, CliFailure> {
    memory_engine_credentials::read_credentials(path).map_err(CliFailure)
}

fn write_credentials(path: &Path, credentials: &StoredCredentials) -> Result<(), CliFailure> {
    memory_engine_credentials::write_credentials(path, credentials).map_err(CliFailure)
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

struct ReviewClient {
    agent: ureq::Agent,
    base_url: String,
    account_id: String,
    session_token: String,
}

impl ReviewClient {
    fn new(
        agent: ureq::Agent,
        base_url: String,
        account_id: String,
        session_token: String,
    ) -> Self {
        Self {
            agent,
            base_url,
            account_id,
            session_token,
        }
    }

    fn next_review(&self) -> Result<StudyView, CliFailure> {
        self.post_empty(&format!("/v1/accounts/{}/review/next", self.account_id))
    }

    fn submit_review(
        &self,
        review_unit_id: &str,
        answer: &str,
        response_time_ms: u32,
        idempotency_key: &str,
    ) -> Result<StudyView, CliFailure> {
        self.post_json(
            &format!(
                "/v1/accounts/{}/review/{review_unit_id}/submit",
                self.account_id
            ),
            &json!({
                "answer": answer,
                "responseTimeMs": response_time_ms,
                "idempotencyKey": idempotency_key,
            }),
        )
    }

    fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliFailure> {
        let mut response = self
            .agent
            .post(&endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_empty()
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, CliFailure> {
        let mut response = self
            .agent
            .post(&endpoint(&self.base_url, path))
            .header("Authorization", &self.authorization())
            .send_json(body)
            .map_err(|error| transport_failure(path, &error))?;
        read_json(&mut response, path)
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.session_token)
    }
}

fn build_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into()
}

fn create_account(
    agent: &ureq::Agent,
    base_url: &str,
    email: &str,
) -> Result<AccountCreated, CliFailure> {
    let mut response = agent
        .post(&endpoint(base_url, "/v1/accounts"))
        .send_json(json!({ "email": email }))
        .map_err(|error| transport_failure("create account", &error))?;
    read_json(&mut response, "create account")
}

fn read_json<T: DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
    action: &str,
) -> Result<T, CliFailure> {
    response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json()
        .map_err(|error| CliFailure(format!("{action} returned unreadable JSON: {error}")))
}

fn transport_failure(action: &str, error: &ureq::Error) -> CliFailure {
    match error {
        ureq::Error::StatusCode(status) => {
            CliFailure(format!("{action} failed with HTTP {status}"))
        }
        _ => CliFailure(format!("{action} transport failed: {error}")),
    }
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{millis}-{counter}", std::process::id())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

// Takes ownership (rather than `&io::Error`) so it can be used directly as
// `.map_err(io_failure)` at every call site instead of a wrapping closure.
#[allow(clippy::needless_pass_by_value)]
fn io_failure(error: io::Error) -> CliFailure {
    CliFailure(format!("io error: {error}"))
}

// ---------------------------------------------------------------------------
// calendar days (no date/time dependency)
// ---------------------------------------------------------------------------

fn day_index_from_ms(ms: i64) -> i64 {
    ms.div_euclid(1000).div_euclid(SECONDS_PER_DAY)
}

/// Converts days since the Unix epoch (1970-01-01) into a `(year, month, day)`
/// UTC proleptic-Gregorian civil date. Ported from Howard Hinnant's public
/// domain `civil_from_days` algorithm so this crate does not need a date
/// dependency for one streak report.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097).cast_unsigned();
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe.cast_signed() + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

fn format_date(year: i64, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------------------------------------------------------------------------
// API response shapes (thin subset of docs/api/openapi.v1.json)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AccountCreated {
    account_id: String,
    session_token: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StudyView {
    current: Option<StudyCurrent>,
    due_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StudyCurrent {
    review_unit_id: String,
    prompt: String,
    #[serde(default)]
    choices: Vec<String>,
    expected_answer: Option<String>,
    grade: Option<StudyGrade>,
    schedule_change: Option<ScheduleChange>,
}

#[derive(Clone, Debug, Deserialize)]
struct StudyGrade {
    verdict: String,
    rating: u8,
    #[serde(rename = "isCorrect")]
    is_correct: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ScheduleChange {
    before: Option<ReviewState>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReviewState {
    reps: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliFailure(String);

impl fmt::Display for CliFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CliFailure {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn civil_from_days_matches_known_reference_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }

    #[test]
    fn day_index_from_ms_floors_to_the_utc_calendar_day() {
        let start_of_2000 = 946_684_800_000; // 2000-01-01T00:00:00Z
        assert_eq!(day_index_from_ms(start_of_2000), 10_957);
        assert_eq!(day_index_from_ms(start_of_2000 + 1_000), 10_957);
    }

    #[test]
    fn streak_report_counts_consecutive_completed_days_ending_today() {
        let dir = tempdir();
        let log_path = dir.join("streak.ndjson");
        let today_ms = 1_000 * SECONDS_PER_DAY * 20_000; // arbitrary anchor day
        let day_ms = 1_000 * SECONDS_PER_DAY;

        for offset in 0..3 {
            append_streak_event(
                &log_path,
                &StreakEvent::Session {
                    ts_ms: today_ms - offset * day_ms,
                    account_id: "acct_test".to_owned(),
                    reviewed_count: 2,
                    due_count_at_start: 2,
                },
            )
            .expect("append session event");
        }
        // A gap: no event 4 days ago, so the streak must not extend past it.
        append_streak_event(
            &log_path,
            &StreakEvent::Session {
                ts_ms: today_ms - 5 * day_ms,
                account_id: "acct_test".to_owned(),
                reviewed_count: 1,
                due_count_at_start: 1,
            },
        )
        .expect("append older session event");

        let report = streak_report(&log_path, 30, today_ms).expect("streak report");
        assert_eq!(report.current_streak_days, 3);
        assert_eq!(report.completed_days, 4);
    }

    #[test]
    fn current_streak_is_not_truncated_by_the_reporting_window() {
        // A real streak can run longer than the `--days` window used for the
        // hit-rate figure; the two must not be conflated.
        let dir = tempdir();
        let log_path = dir.join("streak.ndjson");
        let today_ms = 4_000 * SECONDS_PER_DAY * 20_000;
        let day_ms = 1_000 * SECONDS_PER_DAY;
        let unbroken_days = 45;

        for offset in 0..unbroken_days {
            append_streak_event(
                &log_path,
                &StreakEvent::Session {
                    ts_ms: today_ms - offset * day_ms,
                    account_id: "acct_test".to_owned(),
                    reviewed_count: 1,
                    due_count_at_start: 1,
                },
            )
            .expect("append session event");
        }

        let report = streak_report(&log_path, 30, today_ms).expect("streak report");
        assert_eq!(
            report.current_streak_days, unbroken_days,
            "the true streak must not be capped at the 30-day report window"
        );
        assert_eq!(
            report.completed_days, 30,
            "the window-scoped hit-rate count is still bounded by --days"
        );
        assert_eq!(report.hit_rate, "100%");
    }

    #[test]
    fn streak_report_reports_zero_when_today_and_yesterday_are_both_missed() {
        let dir = tempdir();
        let log_path = dir.join("streak.ndjson");
        let today_ms = 2_000 * SECONDS_PER_DAY * 20_000;
        let day_ms = 1_000 * SECONDS_PER_DAY;

        append_streak_event(
            &log_path,
            &StreakEvent::Session {
                ts_ms: today_ms - 3 * day_ms,
                account_id: "acct_test".to_owned(),
                reviewed_count: 1,
                due_count_at_start: 1,
            },
        )
        .expect("append session event");

        let report = streak_report(&log_path, 30, today_ms).expect("streak report");
        assert_eq!(report.current_streak_days, 0);
    }

    #[test]
    fn streak_report_computes_cold_recall_rate_from_cold_attempts_only() {
        let dir = tempdir();
        let log_path = dir.join("streak.ndjson");
        let today_ms = 3_000 * SECONDS_PER_DAY * 20_000;

        append_streak_event(
            &log_path,
            &StreakEvent::Attempt {
                ts_ms: today_ms,
                account_id: "acct_test".to_owned(),
                review_unit_id: "ru-1".to_owned(),
                verdict: "correct".to_owned(),
                rating: 3,
                is_correct: true,
                cold: true,
                due_count_after: 1,
            },
        )
        .expect("append cold attempt");
        append_streak_event(
            &log_path,
            &StreakEvent::Attempt {
                ts_ms: today_ms,
                account_id: "acct_test".to_owned(),
                review_unit_id: "ru-2".to_owned(),
                verdict: "wrong".to_owned(),
                rating: 1,
                is_correct: false,
                cold: true,
                due_count_after: 0,
            },
        )
        .expect("append cold attempt");
        append_streak_event(
            &log_path,
            &StreakEvent::Attempt {
                ts_ms: today_ms,
                account_id: "acct_test".to_owned(),
                review_unit_id: "ru-3".to_owned(),
                verdict: "correct".to_owned(),
                rating: 3,
                is_correct: true,
                cold: false,
                due_count_after: 0,
            },
        )
        .expect("append non-cold attempt");

        let report = streak_report(&log_path, 30, today_ms).expect("streak report");
        assert_eq!(report.cold_attempts, 2);
        assert_eq!(report.cold_correct, 1);
        assert_eq!(report.cold_recall_rate, "50%");
    }

    #[test]
    fn dispatch_rejects_unknown_subcommands() {
        let mut stdin = Cursor::new(Vec::new());
        let mut stdout = Vec::new();
        let error = dispatch(&["bogus".to_owned()], &mut stdin, &mut stdout).expect_err("error");
        assert!(error.to_string().contains("unknown subcommand"));
    }

    #[test]
    fn login_rejects_partial_existing_credentials() {
        let error = run_login(&["--account-id".to_owned(), "acct_demo".to_owned()])
            .expect_err("partial credentials should fail");
        assert!(error
            .to_string()
            .contains("--account-id and --session-token together"));
    }

    #[test]
    fn resolve_session_prefers_env_vars_over_credentials_file() {
        let dir = tempdir();
        let credentials_path = dir.join("credentials.json");
        write_credentials(
            &credentials_path,
            &StoredCredentials {
                base_url: "https://file.example.com".to_owned(),
                account_id: "acct_file".to_owned(),
                session_token: "file-token".to_owned(),
            },
        )
        .expect("write credentials");

        // SAFETY: this test process does not run other tests concurrently
        // that read these two variable names.
        std::env::set_var("MEMORY_ENGINE_ACCOUNT_ID", "acct_env");
        std::env::set_var("MEMORY_ENGINE_SESSION_TOKEN", "env-token");
        let session = resolve_session(None, &credentials_path).expect("session");
        std::env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        std::env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");

        assert_eq!(session.account_id, "acct_env");
        assert_eq!(session.token, "env-token");
    }

    #[test]
    fn round_trip_credentials_file_restricts_permissions_on_unix() {
        let dir = tempdir();
        let path = dir.join("credentials.json");
        let credentials = StoredCredentials {
            base_url: "https://memory-engine-api-i2xcr.ondigitalocean.app".to_owned(),
            account_id: "acct_demo".to_owned(),
            session_token: "secret".to_owned(),
        };
        write_credentials(&path, &credentials).expect("write credentials");
        let read_back = read_credentials(&path)
            .expect("read credentials")
            .expect("present");
        assert_eq!(read_back, credentials);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "memory-engine-review-test-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn review_loop_drives_a_real_local_api_end_to_end() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local API listener");
        let address = listener.local_addr().expect("local address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                memory_engine_api::router(memory_engine_api::ApiState::default()),
            )
            .await
            .expect("serve local API");
        });

        let agent = build_agent();
        let base_url = format!("http://{address}");
        let email = format!("memory-engine-review-test-{}@example.com", unique_suffix());
        let created = create_account(&agent, &base_url, &email).expect("create account");
        let client = ReviewClient::new(
            agent,
            base_url.clone(),
            created.account_id.clone(),
            created.session_token.clone(),
        );

        let fixtures = [
            (
                "NATO letter A fixture",
                "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.",
                "ALFA",
            ),
            (
                "NATO letter B fixture",
                "Concept: NATO letter B\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for B?\nAnswer: BRAVO\nDistractors: ALFA, CHARLIE\nReference: The NATO phonetic alphabet word for B is BRAVO.",
                "BRAVO",
            ),
        ];
        for (title, body, _answer) in &fixtures {
            let source: serde_json::Value = client
                .post_json(
                    &format!("/v1/accounts/{}/sources", created.account_id),
                    &json!({ "title": title, "body": body }),
                )
                .expect("create source");
            let source_id = source["sourceId"].as_str().expect("source id").to_owned();
            let generated: serde_json::Value = client
                .post_empty(&format!(
                    "/v1/accounts/{}/sources/{source_id}/generate",
                    created.account_id
                ))
                .expect("generate");
            let draft_id = generated["drafts"]
                .as_array()
                .expect("drafts array")
                .iter()
                .find(|draft| !draft["approved"].as_bool().unwrap_or(true))
                .expect("one unapproved draft for this source")["id"]
                .as_str()
                .expect("draft id")
                .to_owned();
            client
                .post_empty::<serde_json::Value>(&format!(
                    "/v1/accounts/{}/drafts/{draft_id}/approve",
                    created.account_id
                ))
                .expect("approve draft");
        }

        // SAFETY: no other test in this process depends on these two names.
        std::env::set_var("MEMORY_ENGINE_ACCOUNT_ID", &created.account_id);
        std::env::set_var("MEMORY_ENGINE_SESSION_TOKEN", &created.session_token);

        let dir = tempdir();
        let log_path = dir.join("streak.ndjson");
        // Two due cards, both answered correctly: proves the loop iterates
        // (not just handles one card), accumulates reviewed_count across
        // iterations, and still reaches a natural dueCount == 0 completion.
        let mut stdin = Cursor::new(b"ALFA\nBRAVO\n".to_vec());
        let mut stdout = Vec::new();
        let receipt = run_review(
            &[
                "--base-url".to_owned(),
                base_url,
                "--log-path".to_owned(),
                log_path.display().to_string(),
            ],
            &mut stdin,
            &mut stdout,
        )
        .expect("review run");

        std::env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        std::env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        server.abort();

        assert_eq!(receipt.reviewed_count, 2);
        assert_eq!(receipt.due_count_at_start, 2);
        assert!(receipt.completed);

        let transcript = String::from_utf8(stdout).expect("utf8 transcript");
        assert!(transcript.contains("What is the NATO phonetic alphabet word for A?"));
        assert!(transcript.contains("What is the NATO phonetic alphabet word for B?"));
        assert!(transcript.contains("All caught up. Reviewed 2 card(s)."));

        let events = read_streak_events(&log_path).expect("streak events");
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], StreakEvent::Attempt { .. }));
        assert!(matches!(events[1], StreakEvent::Attempt { .. }));
        assert!(matches!(
            events[2],
            StreakEvent::Session {
                reviewed_count: 2,
                due_count_at_start: 2,
                ..
            }
        ));
    }
}
