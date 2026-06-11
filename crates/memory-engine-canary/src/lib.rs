//! Fire-and-forget error reporter for Canary.
//!
//! Canary is the self-hosted, agent-first observability service this stack
//! dogfoods (`misty-step/canary`). The wire contract mirrors Canary's
//! TypeScript SDK: `POST {endpoint}/api/v1/errors` with an ingest-scoped
//! bearer key and a JSON body of `service`, `environment`, `error_class`,
//! `message`, `severity`, optional `context`, and `fingerprint`.
//!
//! Reporting must never hurt the request path: events are sent from short-
//! lived background threads with a bounded in-flight count, a 2-second
//! timeout, one retry, and all failures swallowed. Without credentials the
//! reporter is a no-op, exactly like the model provider degrades without its
//! key.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Environment variable naming the Canary base endpoint.
pub const ENDPOINT_ENV: &str = "CANARY_ENDPOINT";
/// Environment variable holding the ingest API key.
pub const API_KEY_ENV: &str = "CANARY_API_KEY";
const MAX_IN_FLIGHT: usize = 10;
const SEND_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

/// One error occurrence to record in Canary.
#[derive(Clone, Debug)]
pub struct ErrorEvent {
    pub error_class: String,
    pub message: String,
    pub severity: Severity,
    pub context: Option<serde_json::Value>,
    /// Grouping hint; empty lets Canary group by its own heuristics.
    pub fingerprint: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CanaryConfig {
    pub endpoint: String,
    pub api_key: String,
    pub service: String,
    pub environment: String,
}

impl CanaryConfig {
    /// Build a config from `CANARY_ENDPOINT` and `CANARY_API_KEY`; `None`
    /// when either is unset, which callers treat as "reporting disabled".
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_parts(
            std::env::var(ENDPOINT_ENV).ok(),
            std::env::var(API_KEY_ENV).ok(),
        )
    }

    /// Build a config from explicit endpoint and key values.
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
}

/// Owns one unit of the bounded in-flight count; releases it on drop, on
/// every exit path including panics.
struct InFlightSlot(Arc<AtomicUsize>);

impl Drop for InFlightSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct CanaryReporter {
    config: CanaryConfig,
    agent: ureq::Agent,
    in_flight: Arc<AtomicUsize>,
}

impl CanaryReporter {
    #[must_use]
    pub fn new(config: CanaryConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(SEND_TIMEOUT))
            .build()
            .into();

        Self {
            config,
            agent,
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Send one event in the background. Never blocks the caller beyond
    /// thread spawn, never panics, never reports its own failure; events are
    /// dropped when the bounded in-flight queue is full.
    pub fn report(&self, event: &ErrorEvent) {
        if self.in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return;
        }

        let url = format!("{}/api/v1/errors", self.config.endpoint);
        let authorization = format!("Bearer {}", self.config.api_key);
        let body = serde_json::json!({
            "service": self.config.service,
            "environment": self.config.environment,
            "error_class": event.error_class,
            "message": event.message,
            "severity": event.severity.label(),
            "context": event.context,
            "fingerprint": event.fingerprint,
        });
        let agent = self.agent.clone();
        // Decrement on drop so a panic in the sender (or a failed spawn)
        // can never leak an in-flight slot and silently disable reporting.
        let slot = InFlightSlot(Arc::clone(&self.in_flight));
        let spawned = thread::Builder::new()
            .name("canary-report".to_owned())
            .spawn(move || {
                let _slot = slot;
                for _ in 0..2 {
                    let sent = agent
                        .post(&url)
                        .header("Authorization", &authorization)
                        .send_json(&body)
                        .is_ok();
                    if sent {
                        break;
                    }
                }
            });
        // Builder::spawn returns Err instead of panicking when the OS cannot
        // create a thread; the moved slot was dropped with the closure, so
        // accounting stays correct and the event is simply lost.
        drop(spawned);
    }

    /// Wait until in-flight sends settle or the deadline passes. For tests
    /// and graceful shutdown only; request paths never call this.
    pub fn drain(&self, deadline: Duration) {
        let started = Instant::now();
        while self.in_flight.load(Ordering::SeqCst) > 0 && started.elapsed() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }
}
