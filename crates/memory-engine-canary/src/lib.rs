//! Bounded Canary observability adapter for memory-engine.
//!
//! Errors and closed performance aggregates share one bounded worker queue.
//! Request paths never perform Canary I/O. Error events go to
//! `POST /api/v1/errors`; performance batches go to `POST /api/v1/events`
//! and are mirrored to stdout as explicitly non-authoritative debug evidence.

mod performance;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use memory_engine_performance::{Namespace, Observation};

use performance::{current_minute, namespace_index, Aggregator};
pub use performance::{
    read_performance_timeline, DeliveryAccounting, PerformanceBatch, PerformanceError,
    PerformanceReadback, ReadbackConfig, DEBUG_AUTHORITY, PERFORMANCE_BATCH_SCHEMA,
    PERFORMANCE_EVENT_NAME,
};

/// Environment variable naming the Canary base endpoint.
pub const ENDPOINT_ENV: &str = "CANARY_ENDPOINT";
/// Environment variable holding the ingest API key.
pub const API_KEY_ENV: &str = "CANARY_API_KEY";
const MAX_QUEUE_DEPTH: usize = 128;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_ATTEMPTS: usize = 2;
const WORKER_IDLE_POLL: Duration = Duration::from_millis(100);

/// Reporter configuration, normally loaded from the environment.
#[derive(Clone, Debug)]
pub struct CanaryConfig {
    /// Base URL, for example `https://canary.example.com`.
    pub endpoint: String,
    /// Ingest-scoped Canary API key.
    pub api_key: String,
    /// Stable service name.
    pub service: String,
    /// Deployment environment label.
    pub environment: String,
}

impl CanaryConfig {
    /// Build a configuration. Missing endpoint or key disables reporting.
    #[must_use]
    pub fn from_parts(endpoint: Option<String>, api_key: Option<String>) -> Option<Self> {
        let endpoint = endpoint?.trim_end_matches('/').to_owned();
        let api_key = api_key?;
        if endpoint.is_empty() || api_key.is_empty() {
            return None;
        }
        Some(Self {
            endpoint,
            api_key,
            service: "memory-engine-api".to_owned(),
            environment: "production".to_owned(),
        })
    }

    /// Load `CANARY_ENDPOINT` and `CANARY_API_KEY` from the process environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_parts(
            std::env::var(ENDPOINT_ENV).ok(),
            std::env::var(API_KEY_ENV).ok(),
        )
    }
}

/// Error severity accepted by Canary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// Non-fatal informational event.
    Info,
    /// Degraded behavior that needs attention.
    Warning,
    /// Failed behavior.
    Error,
}

impl Severity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Structured error event submitted to Canary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorEvent {
    /// Stable, bounded error classification.
    pub error_class: String,
    /// Human-readable error message.
    pub message: String,
    /// Event severity.
    pub severity: Severity,
    /// Optional structured context. Callers must not include secrets or learner content.
    pub context: Option<serde_json::Value>,
    /// Stable fingerprint components used by Canary grouping.
    pub fingerprint: Vec<String>,
}

/// A cheap process-liveness observation sent through Canary's check-in route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckInEvent {
    /// Stable monitor slug registered in Canary.
    pub monitor: String,
    /// Canary check-in status.
    pub status: CheckInStatus,
    /// Short bounded summary.
    pub summary: String,
    /// Monitor expiry requested from Canary.
    pub ttl_ms: u64,
    /// Optional structured, content-free monitor context.
    pub context: Option<serde_json::Value>,
}

/// Canary check-in state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckInStatus {
    Alive,
    Started,
    Completed,
    Failed,
}

impl CheckInStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
enum Command {
    Error(ErrorEvent),
    CheckIn(CheckInEvent),
    Performance(Observation),
    Flush(mpsc::Sender<()>),
    Shutdown(mpsc::Sender<()>),
}

struct ReporterInner {
    sender: SyncSender<Command>,
    performance_drops: Arc<[AtomicU64; 3]>,
    closed: AtomicBool,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

/// Non-blocking reporter backed by one bounded process worker.
#[derive(Clone)]
pub struct CanaryReporter {
    inner: Arc<ReporterInner>,
}

impl CanaryReporter {
    /// Construct a reporter and its single bounded worker.
    #[must_use]
    pub fn new(config: CanaryConfig) -> Self {
        let (sender, receiver) = mpsc::sync_channel(MAX_QUEUE_DEPTH);
        let performance_drops = Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let worker_drops = Arc::clone(&performance_drops);
        let worker = thread::Builder::new()
            .name("memory-engine-canary".to_owned())
            .spawn(move || worker_loop(&config, &receiver, worker_drops.as_ref()))
            .ok();
        Self {
            inner: Arc::new(ReporterInner {
                sender,
                performance_drops,
                closed: AtomicBool::new(false),
                worker: Mutex::new(worker),
            }),
        }
    }

    /// Queue an error without blocking the caller. Saturated or closed queues
    /// drop the event rather than affecting application behavior.
    pub fn report(&self, event: &ErrorEvent) {
        self.try_send(Command::Error(event.clone()), None);
    }

    /// Queue a check-in without blocking the caller.
    pub fn check_in(&self, event: &CheckInEvent) {
        self.try_send(Command::CheckIn(event.clone()), None);
    }

    /// Queue one validated performance observation without blocking the caller.
    ///
    /// Returns `true` when the bounded worker accepted the observation. A
    /// `false` result is accounted in the next batch for the observation's
    /// trusted namespace.
    #[must_use]
    pub fn report_performance(&self, observation: Observation) -> bool {
        let namespace = observation.marker().namespace();
        self.try_send(Command::Performance(observation), Some(namespace))
    }

    /// Flush queued work and the active aggregate, bounded by `deadline`.
    /// Intended for tests; live request paths do not call this because
    /// flushing closes the active minute. Delivery failure remains isolated
    /// from application behavior.
    pub fn drain(&self, deadline: Duration) {
        let _ = self.flush(deadline);
    }

    /// Flush queued work and report whether it settled before `deadline`.
    #[must_use]
    pub fn flush(&self, deadline: Duration) -> bool {
        if self.inner.closed.load(Ordering::Acquire) {
            return false;
        }
        let started = Instant::now();
        let (acknowledge, received) = mpsc::channel();
        if !send_control_until(
            &self.inner.sender,
            Command::Flush(acknowledge),
            started,
            deadline,
        ) {
            return false;
        }
        let remaining = deadline.saturating_sub(started.elapsed());
        received.recv_timeout(remaining).is_ok()
    }

    /// Flush and stop the worker within `deadline`.
    ///
    /// Returns `false` when the deadline expires. Repeated calls are harmless.
    #[must_use]
    pub fn shutdown(&self, deadline: Duration) -> bool {
        let started = Instant::now();
        self.inner.closed.store(true, Ordering::Release);
        if self
            .inner
            .worker
            .lock()
            .map_or(true, |worker| worker.is_none())
        {
            return true;
        }

        let (acknowledge, received) = mpsc::channel();
        if send_control_until(
            &self.inner.sender,
            Command::Shutdown(acknowledge),
            started,
            deadline,
        ) {
            let remaining = deadline.saturating_sub(started.elapsed());
            let _ = received.recv_timeout(remaining);
        }

        loop {
            let finished = self.inner.worker.lock().map_or(true, |worker| {
                worker
                    .as_ref()
                    .is_none_or(std::thread::JoinHandle::is_finished)
            });
            if finished {
                break;
            }
            if started.elapsed() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }

        self.inner
            .worker
            .lock()
            .ok()
            .and_then(|mut worker| worker.take())
            .is_none_or(|worker| worker.join().is_ok())
    }

    fn try_send(&self, command: Command, namespace: Option<Namespace>) -> bool {
        if self.inner.closed.load(Ordering::Acquire) {
            self.note_performance_drop(namespace);
            return false;
        }
        match self.inner.sender.try_send(command) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.note_performance_drop(namespace);
                false
            }
        }
    }

    fn note_performance_drop(&self, namespace: Option<Namespace>) {
        if let Some(namespace) = namespace {
            self.inner.performance_drops[namespace_index(namespace)]
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn send_control_until(
    sender: &SyncSender<Command>,
    mut command: Command,
    started: Instant,
    deadline: Duration,
) -> bool {
    loop {
        match sender.try_send(command) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                if started.elapsed() >= deadline {
                    return false;
                }
                command = returned;
                thread::sleep(Duration::from_millis(5));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

fn worker_loop(
    config: &CanaryConfig,
    receiver: &Receiver<Command>,
    performance_drops: &[AtomicU64; 3],
) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    let mut aggregator = Aggregator::new();
    loop {
        match receiver.recv_timeout(WORKER_IDLE_POLL) {
            Ok(Command::Error(event)) => send_error(&agent, config, &event),
            Ok(Command::CheckIn(event)) => send_check_in(&agent, config, &event),
            Ok(Command::Performance(observation)) => {
                let minute = current_minute();
                if aggregator
                    .active_minute()
                    .is_some_and(|active| active != minute)
                {
                    flush_performance(&agent, config, &mut aggregator, performance_drops);
                }
                aggregator.record(minute, observation);
            }
            Ok(Command::Flush(acknowledge)) => {
                flush_performance(&agent, config, &mut aggregator, performance_drops);
                let _ = acknowledge.send(());
            }
            Ok(Command::Shutdown(acknowledge)) => {
                flush_performance(&agent, config, &mut aggregator, performance_drops);
                let _ = acknowledge.send(());
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if aggregator
                    .active_minute()
                    .is_some_and(|active| active != current_minute())
                {
                    flush_performance(&agent, config, &mut aggregator, performance_drops);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush_performance(&agent, config, &mut aggregator, performance_drops);
                break;
            }
        }
    }
}

fn send_error(agent: &ureq::Agent, config: &CanaryConfig, event: &ErrorEvent) {
    let body = serde_json::json!({
        "service": config.service,
        "environment": config.environment,
        "error_class": event.error_class,
        "message": event.message,
        "severity": event.severity.as_str(),
        "context": event.context,
        "fingerprint": event.fingerprint,
    });
    let url = format!("{}/api/v1/errors", config.endpoint);
    let authorization = format!("Bearer {}", config.api_key);
    let _ = send_json_with_retry(agent, &url, &authorization, &body);
}

fn send_check_in(agent: &ureq::Agent, config: &CanaryConfig, event: &CheckInEvent) {
    let body = serde_json::json!({
        "monitor": event.monitor,
        "status": event.status.as_str(),
        "summary": event.summary,
        "ttl_ms": event.ttl_ms,
        "context": event.context,
    });
    let url = format!("{}/api/v1/check-ins", config.endpoint);
    let authorization = format!("Bearer {}", config.api_key);
    let _ = send_json_with_retry(agent, &url, &authorization, &body);
}

fn flush_performance(
    agent: &ureq::Agent,
    config: &CanaryConfig,
    aggregator: &mut Aggregator,
    performance_drops: &[AtomicU64; 3],
) {
    for namespace in Namespace::all() {
        let dropped = performance_drops[namespace_index(*namespace)].swap(0, Ordering::AcqRel);
        aggregator.note_queue_drop(*namespace, dropped);
    }
    for batch in aggregator.take_batches() {
        let (sent, attempts, body) = send_performance_with_retry(agent, config, &batch);
        println!("{body}");
        if !sent {
            aggregator.note_batch_result(
                batch.namespace(),
                attempts.saturating_sub(1) as u64,
                false,
            );
        }
    }
}

fn send_performance_with_retry(
    agent: &ureq::Agent,
    config: &CanaryConfig,
    batch: &PerformanceBatch,
) -> (bool, usize, serde_json::Value) {
    let url = format!("{}/api/v1/events", config.endpoint);
    let authorization = format!("Bearer {}", config.api_key);
    for attempt in 1..=RETRY_ATTEMPTS {
        let body = performance_body(config, batch, attempt, true);
        if agent
            .post(&url)
            .header("Authorization", &authorization)
            .send_json(&body)
            .is_ok_and(|response| response.status().is_success())
        {
            return (true, attempt, body);
        }
    }
    (
        false,
        RETRY_ATTEMPTS,
        performance_body(config, batch, RETRY_ATTEMPTS, false),
    )
}

fn performance_body(
    config: &CanaryConfig,
    batch: &PerformanceBatch,
    attempts: usize,
    sent: bool,
) -> serde_json::Value {
    serde_json::json!({
        "service": config.service,
        "name": PERFORMANCE_EVENT_NAME,
        "summary": "Bounded memory-engine performance aggregate",
        "severity": "info",
        "attributes": batch.attributes_for_attempt(attempts, sent),
        "sampling_policy": "unsampled",
        "retention_class": "standard",
        "privacy_policy": "redacted",
    })
}

fn send_json_with_retry(
    agent: &ureq::Agent,
    url: &str,
    authorization: &str,
    body: &serde_json::Value,
) -> (bool, usize) {
    for attempt in 1..=RETRY_ATTEMPTS {
        if agent
            .post(url)
            .header("Authorization", authorization)
            .send_json(body)
            .is_ok_and(|response| response.status().is_success())
        {
            return (true, attempt);
        }
    }
    (false, RETRY_ATTEMPTS)
}
