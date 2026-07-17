//! Background generation jobs: asynchronous, non-blocking card generation.
//!
//! Creating material no longer blocks the request. A capture enqueues a
//! `GenerationJob`, the handler returns immediately, and an in-process worker
//! runs the (synchronous, ~20s) model call on a blocking thread, then
//! optimistically approves and schedules the resulting cards. Job status
//! (`queued → running → succeeded | failed`) is held in memory and pushed to
//! the browser over SSE; failed jobs can be retried.
//!
//! File-backed development queues mirror history to `_jobs.json`; production
//! queues use the Postgres job ledger with leases, retry state, and bounded
//! admission. A worker that stops mid-generation leaves a lease that a fresh
//! process can reclaim. Generation writes are replay-safe because draft and
//! review-unit identities are stable and persisted before the job is terminal.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use memory_engine_persistence_postgres::{
    PostgresEnqueueOutcome, PostgresGenerationJob, PostgresStudyStore,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::AccountRegistry;

/// Most generation jobs run at once; the rest wait. Bounds concurrent model
/// calls so a burst of captures can't open dozens of sockets at once.
const MAX_CONCURRENT_JOBS: usize = 4;
const MAX_QUEUE_DEPTH_PER_ACCOUNT: i64 = 8;
const MAX_QUEUE_DEPTH_GLOBAL: i64 = 64;
const MAX_ATTEMPTS: i32 = 3;
const JOB_LEASE_MS: i64 = 5 * 60 * 1_000;
// The grace exceeds one heartbeat interval, giving an old blocking provider
// call time to observe cancellation before another worker can reclaim it.
const JOB_RECLAIM_GRACE_MS: i64 = 2 * 60 * 1_000;
const RETRY_DELAY_MS: i64 = 1_000;
const ACCOUNT_MODEL_BUDGET_USD_MICROS: i64 = 100_000;
const MODEL_BUDGET_WINDOW_MS: i64 = 24 * 60 * 60 * 1_000;
/// Capacity of the SSE broadcast buffer. Events are full job snapshots, so a
/// slow subscriber that lags simply skips to the latest state.
const UPDATES_BUFFER: usize = 256;
/// Per-account cap on *terminal* (succeeded/failed) job history. In-flight jobs
/// are never pruned (the worker still owns them by id); this just keeps the
/// activity log and the in-memory Vec from growing without bound over a
/// long-lived process. The generated cards are durably persisted regardless.
const MAX_TERMINAL_JOBS_PER_ACCOUNT: usize = 50;

fn default_retryable() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Retry,
    Succeeded,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Retry => "retry",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Succeeded/failed jobs no longer change on their own, so they are the
    /// prunable history (see `MAX_TERMINAL_JOBS_PER_ACCOUNT`). This also governs
    /// crash-restore: a *non*-terminal job is reset to a retryable failure on
    /// restart, since no worker owns it after the restart (see `load_jobs`).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// A job-status update fanned out over the SSE broadcast channel. Carries the
/// owning `account_id` so the SSE handler can deliver a learner only their own
/// jobs; `payload` is the `GenerationJob` serialized for the browser.
#[derive(Clone, Debug)]
pub struct JobBroadcast {
    pub account_id: String,
    pub payload: String,
}

/// One generation job. The account/source ids drive the worker; the rest is
/// learner-facing status surfaced in the activity log and over SSE.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationJob {
    pub id: String,
    /// Authorization + store routing for the worker; never serialized to the UI.
    #[serde(skip)]
    pub account_id: String,
    #[serde(skip)]
    pub source_id: String,
    pub title: String,
    pub status: JobStatus,
    pub card_count: usize,
    pub attempts: u32,
    /// False once the bounded attempt budget is exhausted. This is sent over
    /// SSE so the browser never advertises a retry that the API must reject.
    pub retryable: bool,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip)]
    pub retry_at: Option<i64>,
    #[serde(skip)]
    pub lease_expires_at: Option<i64>,
}

/// The on-disk shape of a [`GenerationJob`]. Distinct from the UI serialization
/// (which is camelCase and `skip`s `account_id`/`source_id`): the worker needs
/// those ids to resume routing after a restart, so the disk record carries every
/// field. Keeping the two representations separate lets the wire format and the
/// storage format evolve independently.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedJob {
    id: String,
    account_id: String,
    source_id: String,
    title: String,
    status: JobStatus,
    card_count: usize,
    attempts: u32,
    #[serde(default = "default_retryable")]
    retryable: bool,
    error: Option<String>,
    created_at: i64,
    updated_at: i64,
    #[serde(default)]
    retry_at: Option<i64>,
    #[serde(default)]
    lease_expires_at: Option<i64>,
}

impl From<&GenerationJob> for PersistedJob {
    fn from(job: &GenerationJob) -> Self {
        // Destructured with no `..` on purpose: adding a field to `GenerationJob`
        // then fails to compile here until it is also persisted, so the on-disk
        // record can never silently drift out of sync with the in-memory one.
        let GenerationJob {
            id,
            account_id,
            source_id,
            title,
            status,
            card_count,
            attempts,
            retryable,
            error,
            created_at,
            updated_at,
            retry_at,
            lease_expires_at,
        } = job;
        Self {
            id: id.clone(),
            account_id: account_id.clone(),
            source_id: source_id.clone(),
            title: title.clone(),
            status: *status,
            card_count: *card_count,
            attempts: *attempts,
            retryable: *retryable,
            error: error.clone(),
            created_at: *created_at,
            updated_at: *updated_at,
            retry_at: *retry_at,
            lease_expires_at: *lease_expires_at,
        }
    }
}

impl From<PersistedJob> for GenerationJob {
    fn from(record: PersistedJob) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            source_id: record.source_id,
            title: record.title,
            status: record.status,
            card_count: record.card_count,
            attempts: record.attempts,
            retryable: record.retryable,
            error: record.error,
            created_at: record.created_at,
            updated_at: record.updated_at,
            retry_at: record.retry_at,
            lease_expires_at: record.lease_expires_at,
        }
    }
}

/// Outcome of [`JobQueue::enqueue_or_coalesce`]: a new or existing in-flight
/// job, a policy rejection, or a transient queue-store failure.
///
/// Carrying the job snapshot avoids a second Postgres read after a committed
/// enqueue and keeps callers from reporting a false "not found" response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnqueueOutcome {
    Started(GenerationJob),
    AlreadyInFlight(GenerationJob),
    Rejected(String),
    Unavailable(String),
}

/// An async queue of generation jobs plus the in-process worker that drains it.
///
/// Cheaply cloneable (`Arc` inside) so it can live in `ApiState`, be handed to
/// route handlers, and be owned by the spawned worker at once.
#[derive(Clone)]
pub struct JobQueue {
    inner: Arc<Inner>,
}

struct Inner {
    registry: AccountRegistry,
    jobs: Mutex<Vec<GenerationJob>>,
    tx: mpsc::UnboundedSender<String>,
    /// Taken once by `spawn_worker`; `None` afterwards (single-consumer mpsc).
    rx: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
    updates: broadcast::Sender<JobBroadcast>,
    /// Where job history is mirrored, or `None` for an in-memory queue (tests,
    /// the postgres host). Every mutation writes the whole Vec through to here.
    persist_path: Option<PathBuf>,
    /// Serializes durable writes so two concurrent `finish`es can't race to
    /// write a stale snapshot. Held only across a snapshot+write, never with the
    /// jobs lock, so it cannot deadlock against it.
    persist_lock: Mutex<()>,
    postgres_url: Option<String>,
    worker_started: AtomicBool,
    worker_ready: AtomicBool,
    worker_id: String,
}

impl JobQueue {
    /// Build an in-memory queue bound to `registry`. History is lost on restart;
    /// use [`JobQueue::with_persistence`] for the durable host. The worker is not
    /// started until [`JobQueue::spawn_worker`] is called (so tests can drive
    /// jobs directly).
    #[must_use]
    pub fn new(registry: AccountRegistry) -> Self {
        Self::build(registry, Vec::new(), None, None)
    }

    /// Build a queue backed by the production Postgres job ledger.
    #[must_use]
    pub fn with_postgres(registry: AccountRegistry, database_url: impl Into<String>) -> Self {
        Self::build(registry, Vec::new(), None, Some(database_url.into()))
    }

    /// Build a queue whose history is mirrored to `path` and restored from it on
    /// construction. A job left non-terminal by a crash is restored as a
    /// retryable `failed` (see `load_jobs`).
    #[must_use]
    pub fn with_persistence(registry: AccountRegistry, path: PathBuf) -> Self {
        let restored = load_jobs(&path, registry.now());
        let queue = Self::build(registry, restored, Some(path), None);
        // Make the in-flight -> failed reset durable now, so a second crash
        // before the next mutation doesn't replay against stale on-disk state.
        // The worker isn't running yet, so this write is uncontended.
        queue.persist();
        queue
    }

    fn build(
        registry: AccountRegistry,
        jobs: Vec<GenerationJob>,
        persist_path: Option<PathBuf>,
        postgres_url: Option<String>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (updates, _keep_alive) = broadcast::channel(UPDATES_BUFFER);
        Self {
            inner: Arc::new(Inner {
                registry,
                jobs: Mutex::new(jobs),
                tx,
                rx: Mutex::new(Some(rx)),
                updates,
                persist_path,
                persist_lock: Mutex::new(()),
                postgres_url,
                worker_started: AtomicBool::new(false),
                worker_ready: AtomicBool::new(false),
                worker_id: format!("api-{}-{:032x}", std::process::id(), rand::random::<u128>()),
            }),
        }
    }

    /// Start the background worker. Must run inside a Tokio runtime. Idempotent:
    /// a second call is a no-op because the single receiver is already taken.
    pub fn spawn_worker(&self) {
        if !claim_worker_start(&self.inner.worker_started) {
            return;
        }
        if self.inner.postgres_url.is_some() {
            let worker = self.clone();
            tokio::spawn(worker.run_postgres());
            return;
        }
        self.inner.worker_ready.store(true, Ordering::Release);
        let Some(rx) = self.lock_rx().take() else {
            return;
        };
        let worker = self.clone();
        tokio::spawn(worker.run(rx));
    }

    #[must_use]
    pub fn worker_started(&self) -> bool {
        self.inner.worker_started.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn worker_ready(&self) -> bool {
        self.inner.worker_ready.load(Ordering::Acquire)
    }

    /// Enqueue generation for an already-saved source. Returns the job id; the
    /// caller renders immediately while the worker runs in the background.
    ///
    /// Always starts a new job, even if one is already in flight for the same
    /// account+source — callers that must not duplicate a running job (any
    /// learner-triggered "generate" request) should call
    /// [`JobQueue::enqueue_or_coalesce`] instead.
    // Callers fire-and-forget (the id is incidental), so the return is routinely
    // discarded; `must_use` would only force noisy `let _ =` at every site.
    #[allow(clippy::must_use_candidate)]
    pub fn enqueue(&self, account_id: &str, source_id: &str, title: &str) -> EnqueueOutcome {
        if self.inner.postgres_url.is_some() {
            return self.enqueue_postgres(account_id, source_id, title);
        }
        let job = self.new_job(account_id, source_id, title);
        let snapshot = job.clone();
        // Commit to the Vec first, then broadcast — so an SSE subscriber that
        // reacts to the event can always find the job via `jobs_for`/`job`. This
        // matches the commit-then-broadcast order of the other mutators.
        {
            let mut jobs = self.lock_jobs();
            jobs.push(job);
            enforce_terminal_retention(&mut jobs, account_id);
        }
        self.start(&snapshot);
        EnqueueOutcome::Started(snapshot)
    }

    /// Enqueue generation for an already-saved source, coalescing onto an
    /// existing in-flight (queued/running) job for the same account+source
    /// instead of starting a second one. A repeat "Create review" press while
    /// generation is still running must not enqueue a duplicate job (082) —
    /// the check-and-insert happens under a single lock acquisition so two
    /// racing requests can't both observe "no active job" and both enqueue.
    #[must_use]
    pub fn enqueue_or_coalesce(
        &self,
        account_id: &str,
        source_id: &str,
        title: &str,
    ) -> EnqueueOutcome {
        if self.inner.postgres_url.is_some() {
            return self.enqueue_postgres(account_id, source_id, title);
        }
        let candidate = self.new_job(account_id, source_id, title);
        let outcome = {
            let mut jobs = self.lock_jobs();
            let active = jobs
                .iter()
                .find(|job| {
                    job.account_id == account_id
                        && job.source_id == source_id
                        && !job.status.is_terminal()
                })
                .cloned();
            if let Some(job) = active {
                Err(job)
            } else {
                let snapshot = candidate.clone();
                jobs.push(candidate);
                enforce_terminal_retention(&mut jobs, account_id);
                Ok(snapshot)
            }
        };
        match outcome {
            Ok(snapshot) => {
                self.start(&snapshot);
                EnqueueOutcome::Started(snapshot)
            }
            Err(job) => EnqueueOutcome::AlreadyInFlight(job),
        }
    }

    fn enqueue_postgres(&self, account_id: &str, source_id: &str, title: &str) -> EnqueueOutcome {
        let Some(database_url) = self.inner.postgres_url.as_deref() else {
            return EnqueueOutcome::Unavailable(
                "Generation is temporarily unavailable. Please try again.".to_owned(),
            );
        };
        let model_key = std::env::var("MEMORY_ENGINE_GENERATION_MODEL")
            .unwrap_or_else(|_| "deterministic".to_owned());
        let job_id = format!("job-{:032x}", rand::random::<u128>());
        let result = with_postgres_store(database_url, |store| {
            store.enqueue_generation_job(
                account_id,
                &job_id,
                source_id,
                title,
                &model_key,
                self.inner.registry.now(),
                MAX_QUEUE_DEPTH_PER_ACCOUNT,
                MAX_QUEUE_DEPTH_GLOBAL,
                ACCOUNT_MODEL_BUDGET_USD_MICROS,
                MODEL_BUDGET_WINDOW_MS,
            )
        });
        match result {
            Ok(PostgresEnqueueOutcome::Started(job)) => {
                self.broadcast_postgres(&job);
                EnqueueOutcome::Started(job.into())
            }
            Ok(PostgresEnqueueOutcome::AlreadyInFlight(job)) => {
                EnqueueOutcome::AlreadyInFlight(job.into())
            }
            Ok(PostgresEnqueueOutcome::Rejected(reason)) => EnqueueOutcome::Rejected(reason),
            Err(error) => {
                eprintln!("memory-engine: generation enqueue failed: {error}");
                EnqueueOutcome::Unavailable(
                    "Generation is temporarily unavailable. Please try again.".to_owned(),
                )
            }
        }
    }

    /// Build a fresh queued job. Shared by `enqueue` and `enqueue_or_coalesce`
    /// so the two entry points can never drift on the job's initial shape.
    fn new_job(&self, account_id: &str, source_id: &str, title: &str) -> GenerationJob {
        let now = self.inner.registry.now();
        GenerationJob {
            id: format!("job-{:032x}", rand::random::<u128>()),
            account_id: account_id.to_owned(),
            source_id: source_id.to_owned(),
            title: title.to_owned(),
            status: JobStatus::Queued,
            card_count: 0,
            attempts: 0,
            retryable: true,
            error: None,
            created_at: now,
            updated_at: now,
            retry_at: None,
            lease_expires_at: None,
        }
    }

    /// Broadcast, persist, and wake the worker for a freshly pushed job.
    /// Shared tail of `enqueue` and `enqueue_or_coalesce`'s "started" path.
    fn start(&self, job: &GenerationJob) {
        self.broadcast(job);
        self.persist();
        // Send never fails while the queue is alive (the receiver lives in the
        // worker); if the worker was never started the job simply stays queued.
        let _ = self.inner.tx.send(job.id.clone());
    }

    /// Re-queue a failed job (the learner pressed Retry). Scoped to the owning
    /// account so one learner cannot touch another's jobs.
    // The handler branches on the bool inline; other callers ignore it. Marking
    // it `must_use` would force a `let _ =` at the fire-and-forget sites.
    #[allow(clippy::must_use_candidate)]
    pub fn retry(&self, account_id: &str, job_id: &str) -> bool {
        if let Some(database_url) = self.inner.postgres_url.as_deref() {
            return with_postgres_store(database_url, |store| {
                store.retry_generation_job(
                    account_id,
                    job_id,
                    self.inner.registry.now(),
                    MAX_ATTEMPTS,
                )
            })
            .unwrap_or(false);
        }
        let requeued = {
            let mut jobs = self.lock_jobs();
            match jobs
                .iter_mut()
                .find(|job| job.id == job_id && job.account_id == account_id)
            {
                Some(job) if job.status == JobStatus::Failed => {
                    if !job.retryable {
                        return false;
                    }
                    job.status = JobStatus::Queued;
                    job.error = None;
                    job.updated_at = self.inner.registry.now();
                    Some(job.clone())
                }
                _ => None,
            }
        };
        if let Some(job) = requeued {
            self.broadcast(&job);
            self.persist();
            let _ = self.inner.tx.send(job_id.to_owned());
            true
        } else {
            false
        }
    }

    /// Current jobs for an account, newest first — the server-authoritative
    /// activity log rendered on every page load.
    #[must_use]
    pub fn jobs_for(&self, account_id: &str) -> Vec<GenerationJob> {
        if let Some(database_url) = self.inner.postgres_url.as_deref() {
            return with_postgres_store(database_url, |store| {
                store.list_generation_jobs(account_id, 50)
            })
            .map(|jobs| jobs.into_iter().map(GenerationJob::from).collect())
            .unwrap_or_default();
        }
        let mut jobs = self
            .lock_jobs()
            .iter()
            .filter(|job| job.account_id == account_id)
            .cloned()
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.created_at));
        jobs
    }

    /// Subscribe to job-status updates for SSE. Each item is a [`JobBroadcast`]
    /// carrying the owning account id, so the handler can filter to one learner.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<JobBroadcast> {
        self.inner.updates.subscribe()
    }

    /// Look up a single account-scoped job.
    ///
    /// Postgres read failures remain distinct from a missing job so machine
    /// pollers can retry transient outages instead of discarding active work.
    ///
    /// # Errors
    ///
    /// Returns a retryable message when the Postgres job ledger is unavailable.
    pub fn job_for_account(
        &self,
        account_id: &str,
        job_id: &str,
    ) -> Result<Option<GenerationJob>, String> {
        if let Some(database_url) = self.inner.postgres_url.as_deref() {
            return with_postgres_store(database_url, |store| {
                store.generation_job(account_id, job_id)
            })
            .map(|job| job.map(GenerationJob::from))
            .map_err(|_| {
                "Generation status is temporarily unavailable. Please try again.".to_owned()
            });
        }
        Ok(self
            .lock_jobs()
            .iter()
            .find(|job| job.id == job_id && job.account_id == account_id)
            .cloned())
    }

    /// Test-only lookup for file-backed queues. Production reads must carry an
    /// authenticated account id; a Postgres queue refuses an unscoped lookup.
    #[must_use]
    pub fn job(&self, job_id: &str) -> Option<GenerationJob> {
        if self.inner.postgres_url.is_some() {
            return None;
        }
        self.lock_jobs()
            .iter()
            .find(|job| job.id == job_id)
            .cloned()
    }

    /// Synchronously run every queued job to completion, in order. The spawned
    /// worker drains the queue asynchronously in production; this is the
    /// deterministic path for tests (and any host without an async runtime).
    pub fn run_pending_blocking(&self) {
        if self.inner.postgres_url.is_some() {
            while let Ok(Some(job)) = self.claim_postgres() {
                self.broadcast_postgres(&job);
                self.run_claimed_blocking(&job);
            }
            return;
        }
        while let Some(job_id) = self.next_queued() {
            if let Some((account_id, source_id)) = self.mark_running(&job_id) {
                let result = self
                    .inner
                    .registry
                    .run_generation_job(
                        &account_id,
                        &source_id,
                        &format!("file-job-{job_id}"),
                        0,
                        "",
                        || true,
                    )
                    .map_err(|failure| failure.message);
                self.finish(&job_id, result);
            }
        }
    }

    fn run_claimed_blocking(&self, job: &PostgresGenerationJob) {
        let run_id = format!("job:{}:attempt:{}", job.id, job.attempts);
        let Some(database_url) = self.inner.postgres_url.as_deref() else {
            return;
        };
        let bound = with_postgres_store(database_url, |store| {
            store.bind_generation_job_attempt_run(
                &job.account_id,
                &job.id,
                job.attempts,
                job.lease_token.as_deref().unwrap_or_default(),
                &run_id,
            )
        })
        .unwrap_or(false);
        if !bound {
            return;
        }
        let fence_database_url = database_url.to_owned();
        let fence_account_id = job.account_id.clone();
        let fence_job_id = job.id.clone();
        let fence_token = job.lease_token.clone().unwrap_or_default();
        let fence_reservation = job.reserved_cost_usd_micros;
        let fence_run_id = run_id.clone();
        let fence_registry = self.inner.registry.clone();
        let generation_attempt = i32::try_from(job.attempts).unwrap_or(i32::MAX);
        let generation_lease_token = job.lease_token.clone().unwrap_or_default();
        let result = self
            .inner
            .registry
            .run_generation_job(
                &job.account_id,
                &job.source_id,
                &run_id,
                generation_attempt,
                &generation_lease_token,
                move || {
                    with_postgres_store(&fence_database_url, |store| {
                        let current = store.generation_job(&fence_account_id, &fence_job_id)?;
                        let cost =
                            store.generation_cost_for_run(&fence_account_id, &fence_run_id)?;
                        Ok(current.is_some_and(|current| {
                            current.status == "running"
                                && current.lease_token.as_deref() == Some(fence_token.as_str())
                                && current
                                    .lease_expires_at
                                    .is_some_and(|expires| expires > fence_registry.now())
                                && cost <= fence_reservation
                        }))
                    })
                    .ok()
                    .unwrap_or(false)
                },
            )
            .and_then(|card_count| {
                self.inner
                    .registry
                    .generation_cost_for_run(&job.account_id, &run_id)
                    .map(|cost| (card_count, cost))
            })
            .map_err(|failure| failure.message);
        let _ = with_postgres_store(database_url, |store| {
            store.finish_generation_job(
                &job.account_id,
                &job.id,
                job.lease_token.as_deref().unwrap_or_default(),
                self.inner.registry.now(),
                result,
                MAX_ATTEMPTS,
                RETRY_DELAY_MS,
            )
        });
    }

    fn next_queued(&self) -> Option<String> {
        self.lock_jobs()
            .iter()
            .find(|job| job.status == JobStatus::Queued)
            .map(|job| job.id.clone())
    }

    async fn run(self, mut rx: mpsc::UnboundedReceiver<String>) {
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS));
        while let Some(job_id) = rx.recv().await {
            // Bound concurrency: acquire before spawning so at most
            // MAX_CONCURRENT_JOBS model calls run at once.
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };
            let worker = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                worker.run_job(&job_id).await;
            });
        }
    }

    async fn run_postgres(self) {
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS));
        loop {
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };
            let worker = self.clone();
            let Ok(claimed) = tokio::task::spawn_blocking(move || worker.claim_postgres()).await
            else {
                self.inner.worker_ready.store(false, Ordering::Release);
                drop(permit);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            };
            let job = match claimed {
                Ok(Some(job)) => job,
                Ok(None) => {
                    self.inner.worker_ready.store(true, Ordering::Release);
                    drop(permit);
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
                Err(_) => {
                    self.inner.worker_ready.store(false, Ordering::Release);
                    drop(permit);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }
            };
            self.inner.worker_ready.store(true, Ordering::Release);
            self.broadcast_postgres(&job);
            let worker = self.clone();
            tokio::spawn(async move {
                let _permit = permit;
                worker.run_claimed_postgres(job).await;
            });
        }
    }

    fn claim_postgres(
        &self,
    ) -> Result<Option<PostgresGenerationJob>, memory_engine_persistence_postgres::PostgresStoreError>
    {
        let Some(database_url) = self.inner.postgres_url.as_deref() else {
            return Ok(None);
        };
        with_postgres_store(database_url, |store| {
            store.claim_generation_job(
                &self.inner.worker_id,
                self.inner.registry.now(),
                JOB_LEASE_MS,
                JOB_RECLAIM_GRACE_MS,
                i64::try_from(MAX_CONCURRENT_JOBS).unwrap_or(i64::MAX),
                MAX_ATTEMPTS,
            )
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn run_claimed_postgres(&self, job: PostgresGenerationJob) {
        let registry = self.inner.registry.clone();
        let account_id = job.account_id.clone();
        let source_id = job.source_id.clone();
        let run_id = format!("job:{}:attempt:{}", job.id, job.attempts);
        let cancelled = Arc::new(AtomicBool::new(false));
        let generation_cancelled = cancelled.clone();
        let generation_run_id = run_id.clone();
        let Some(database_url) = self.inner.postgres_url.as_deref() else {
            return;
        };
        let fence_database_url = database_url.to_owned();
        let fence_account_id = job.account_id.clone();
        let fence_job_id = job.id.clone();
        let fence_token = job.lease_token.clone().unwrap_or_default();
        let fence_reservation = job.reserved_cost_usd_micros;
        let fence_run_id = run_id.clone();
        let fence_registry = registry.clone();
        let generation_attempt = i32::try_from(job.attempts).unwrap_or(i32::MAX);
        let generation_lease_token = job.lease_token.clone().unwrap_or_default();
        let bound = with_postgres_store(database_url, |store| {
            store.bind_generation_job_attempt_run(
                &job.account_id,
                &job.id,
                job.attempts,
                job.lease_token.as_deref().unwrap_or_default(),
                &run_id,
            )
        })
        .unwrap_or(false);
        if !bound {
            return;
        }
        let generation = tokio::task::spawn_blocking(move || {
            registry
                .run_generation_job(
                    &account_id,
                    &source_id,
                    &generation_run_id,
                    generation_attempt,
                    &generation_lease_token,
                    move || {
                        if generation_cancelled.load(Ordering::Acquire) {
                            return false;
                        }
                        with_postgres_store(&fence_database_url, |store| {
                            let current = store.generation_job(&fence_account_id, &fence_job_id)?;
                            let cost =
                                store.generation_cost_for_run(&fence_account_id, &fence_run_id)?;
                            Ok(current.is_some_and(|current| {
                                current.status == "running"
                                    && current.lease_token.as_deref() == Some(fence_token.as_str())
                                    && current
                                        .lease_expires_at
                                        .is_some_and(|expires| expires > fence_registry.now())
                                    && cost <= fence_reservation
                            }))
                        })
                        .ok()
                        .unwrap_or(false)
                    },
                )
                .and_then(|card_count| {
                    registry
                        .generation_cost_for_run(&account_id, &generation_run_id)
                        .map(|cost| (card_count, cost))
                })
                .map_err(|failure| failure.message)
        });
        tokio::pin!(generation);
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
            u64::try_from((JOB_LEASE_MS / 3).max(1)).unwrap_or(1),
        ));
        let outcome = loop {
            tokio::select! {
                result = &mut generation => {
                    break result.unwrap_or_else(|_| Err("Generation crashed unexpectedly.".to_owned()));
                }
                _ = heartbeat.tick() => {
                    let Some(database_url) = self.inner.postgres_url.clone() else {
                        return;
                    };
                    let account_id = job.account_id.clone();
                    let job_id = job.id.clone();
                    let lease_token = job.lease_token.clone().unwrap_or_default();
                    let now_ms = self.inner.registry.now();
                    let renewed = tokio::task::spawn_blocking(move || {
                        with_postgres_store(&database_url, |store| {
                            store.renew_generation_job(
                                &account_id,
                                &job_id,
                                &lease_token,
                                now_ms,
                                JOB_LEASE_MS,
                            )
                        })
                    }).await;
                    if !matches!(renewed, Ok(Ok(true))) {
                        cancelled.store(true, Ordering::Release);
                        let outcome = tokio::time::timeout(
                            std::time::Duration::from_millis(JOB_RECLAIM_GRACE_MS as u64),
                            &mut generation,
                        )
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or_else(|| Err("Generation lease was lost.".to_owned()));
                        let _ = self.finish_postgres_attempt(&job, outcome);
                        return;
                    }
                }
            }
        };
        let _ = self.finish_postgres_attempt(&job, outcome);
        if let Ok(Some(updated)) = with_postgres_store(database_url, |store| {
            store.generation_job(&job.account_id, &job.id)
        }) {
            self.broadcast_postgres(&updated);
        }
    }

    fn finish_postgres_attempt(
        &self,
        job: &PostgresGenerationJob,
        outcome: Result<(usize, i64), String>,
    ) -> Result<bool, memory_engine_persistence_postgres::PostgresStoreError> {
        let Some(database_url) = self.inner.postgres_url.as_deref() else {
            return Ok(false);
        };
        with_postgres_store(database_url, |store| {
            store.finish_generation_job(
                &job.account_id,
                &job.id,
                job.lease_token.as_deref().unwrap_or_default(),
                self.inner.registry.now(),
                outcome,
                MAX_ATTEMPTS,
                RETRY_DELAY_MS,
            )
        })
    }

    async fn run_job(&self, job_id: &str) {
        let Some((account_id, source_id)) = self.mark_running(job_id) else {
            return;
        };
        let registry = self.inner.registry.clone();
        let run_id = format!("file-job-{job_id}");
        // The model call is synchronous (`ureq`); run it off the async runtime
        // on the blocking pool so it never parks a Tokio worker thread.
        let outcome = tokio::task::spawn_blocking(move || {
            registry.run_generation_job(&account_id, &source_id, &run_id, 0, "", || true)
        })
        .await;

        match outcome {
            Ok(Ok(card_count)) => self.finish(job_id, Ok(card_count)),
            Ok(Err(failure)) => self.finish(job_id, Err(failure.message)),
            Err(_join) => self.finish(job_id, Err("Generation crashed unexpectedly.".to_owned())),
        }
    }

    fn mark_running(&self, job_id: &str) -> Option<(String, String)> {
        let now = self.inner.registry.now();
        let snapshot = {
            let mut jobs = self.lock_jobs();
            let job = jobs.iter_mut().find(|job| job.id == job_id)?;
            job.status = JobStatus::Running;
            job.attempts += 1;
            job.error = None;
            job.updated_at = now;
            job.clone()
        };
        let input = (snapshot.account_id.clone(), snapshot.source_id.clone());
        self.broadcast(&snapshot);
        self.persist();
        Some(input)
    }

    fn finish(&self, job_id: &str, result: Result<usize, String>) {
        let now = self.inner.registry.now();
        let snapshot = {
            let mut jobs = self.lock_jobs();
            let Some(job) = jobs.iter_mut().find(|job| job.id == job_id) else {
                return;
            };
            match result {
                Ok(card_count) => {
                    job.status = JobStatus::Succeeded;
                    job.card_count = card_count;
                    job.error = None;
                }
                Err(message) => {
                    job.status = JobStatus::Failed;
                    job.retryable = job.attempts < MAX_ATTEMPTS as u32;
                    job.error = Some(message);
                }
            }
            job.updated_at = now;
            let snapshot = job.clone();
            // Now that this job is terminal, prune the account's terminal history
            // so the cap holds continuously, not only at enqueue time.
            enforce_terminal_retention(&mut jobs, &snapshot.account_id);
            snapshot
        };
        self.broadcast(&snapshot);
        self.persist();
    }

    /// Mirror the whole job Vec to disk, when a `persist_path` is configured.
    ///
    /// Best-effort: a write failure logs and is swallowed (the in-memory Vec
    /// stays authoritative and the generated cards are durable regardless). The
    /// `persist_lock` serializes writers so a slow write cannot land after a
    /// newer one and leave stale history on disk; each writer re-snapshots the
    /// latest state under the jobs lock, then writes outside it.
    fn persist(&self) {
        let Some(path) = self.inner.persist_path.as_ref() else {
            return;
        };
        let _writing = self
            .inner
            .persist_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let serialized = {
            let jobs = self.lock_jobs();
            let records = jobs.iter().map(PersistedJob::from).collect::<Vec<_>>();
            serde_json::to_vec_pretty(&records)
        };
        let Ok(bytes) = serialized else {
            return;
        };
        if let Err(error) = crate::write_atomic(path, &bytes) {
            eprintln!(
                "memory-engine: failed to persist job history to {}: {error}",
                path.display()
            );
        }
    }

    fn broadcast(&self, job: &GenerationJob) {
        if let Ok(payload) = serde_json::to_string(job) {
            // Err only when there are no subscribers; that is fine.
            let _ = self.inner.updates.send(JobBroadcast {
                account_id: job.account_id.clone(),
                payload,
            });
        }
    }

    fn broadcast_postgres(&self, job: &PostgresGenerationJob) {
        let job = postgres_job_payload(job);
        self.broadcast(&job);
    }

    fn lock_jobs(&self) -> std::sync::MutexGuard<'_, Vec<GenerationJob>> {
        self.inner
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_rx(&self) -> std::sync::MutexGuard<'_, Option<mpsc::UnboundedReceiver<String>>> {
        self.inner
            .rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn postgres_store(
    database_url: &str,
) -> Result<PostgresStudyStore, memory_engine_persistence_postgres::PostgresStoreError> {
    static MIGRATED_URLS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let mut store = PostgresStudyStore::connect(database_url)?;
    let migrated = MIGRATED_URLS.get_or_init(|| Mutex::new(BTreeSet::new()));
    let mut migrated = migrated
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !migrated.contains(database_url) {
        store.migrate()?;
        migrated.insert(database_url.to_owned());
    }
    Ok(store)
}

fn claim_worker_start(started: &AtomicBool) -> bool {
    !started.swap(true, Ordering::AcqRel)
}

fn with_postgres_store<R>(
    database_url: &str,
    operation: impl FnOnce(
        &mut PostgresStudyStore,
    ) -> Result<R, memory_engine_persistence_postgres::PostgresStoreError>,
) -> Result<R, memory_engine_persistence_postgres::PostgresStoreError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| {
            let mut store = postgres_store(database_url)?;
            operation(&mut store)
        })
    } else {
        let mut store = postgres_store(database_url)?;
        operation(&mut store)
    }
}

impl From<PostgresGenerationJob> for GenerationJob {
    fn from(job: PostgresGenerationJob) -> Self {
        let status = match job.status.as_str() {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "retry" => JobStatus::Retry,
            "succeeded" => JobStatus::Succeeded,
            _ => JobStatus::Failed,
        };
        Self {
            id: job.id,
            account_id: job.account_id,
            source_id: job.source_id,
            title: job.title,
            status,
            card_count: job.card_count,
            attempts: job.attempts,
            retryable: job.attempts < MAX_ATTEMPTS as u32,
            error: job.error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            retry_at: job.retry_at,
            lease_expires_at: job.lease_expires_at,
        }
    }
}

fn postgres_job_payload(job: &PostgresGenerationJob) -> GenerationJob {
    GenerationJob::from(job.clone())
}

/// Keep at most `MAX_TERMINAL_JOBS_PER_ACCOUNT` terminal jobs for `account_id`,
/// dropping the oldest first. In-flight (queued/running) jobs are untouched —
/// the worker still owns them by id. Jobs are pushed in creation order, so
/// `retain` walks oldest-to-newest and drops the leading excess.
fn enforce_terminal_retention(jobs: &mut Vec<GenerationJob>, account_id: &str) {
    let terminal = jobs
        .iter()
        .filter(|job| job.account_id == account_id && job.status.is_terminal())
        .count();
    let mut excess = terminal.saturating_sub(MAX_TERMINAL_JOBS_PER_ACCOUNT);
    if excess == 0 {
        return;
    }
    jobs.retain(|job| {
        let drop = excess > 0 && job.account_id == account_id && job.status.is_terminal();
        if drop {
            excess -= 1;
        }
        !drop
    });
}

/// Restore job history from `path`, or an empty list when the file is absent or
/// unreadable (a corrupt history must not stop the server from booting). Any job
/// still non-terminal (queued/running) when the process stopped is restored as a
/// retryable `failed`: no worker owns it after the restart, so leaving it
/// "running" would strand it forever. The learner can press Retry. `now` stamps
/// the reset so the activity log shows the restart time.
fn load_jobs(path: &Path, now: i64) -> Vec<GenerationJob> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let records = match serde_json::from_slice::<Vec<PersistedJob>>(&bytes) {
        Ok(records) => records,
        Err(error) => {
            eprintln!(
                "memory-engine: job history at {} is unreadable ({error}); starting empty",
                path.display()
            );
            return Vec::new();
        }
    };
    records
        .into_iter()
        .map(|record| {
            let mut job = GenerationJob::from(record);
            if !job.status.is_terminal() {
                job.status = JobStatus::Failed;
                job.retryable = job.attempts < MAX_ATTEMPTS as u32;
                job.error = Some(
                    "Interrupted by a server restart. Press Retry to generate again.".to_owned(),
                );
                job.updated_at = now;
            }
            job
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AccountRegistry;

    // A ghost source fails fast in the worker (no model call), so every job
    // becomes terminal (failed) without touching the network.
    const GHOST: &str = "ghost-source";
    static TEST_CLOCK_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

    fn enqueue_id(outcome: EnqueueOutcome) -> String {
        match outcome {
            EnqueueOutcome::Started(job) | EnqueueOutcome::AlreadyInFlight(job) => job.id,
            EnqueueOutcome::Rejected(reason) | EnqueueOutcome::Unavailable(reason) => {
                panic!("test enqueue rejected: {reason}")
            }
        }
    }

    fn test_clock_ms() -> i64 {
        TEST_CLOCK_MS.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn postgres_queue_failures_are_reported_as_unavailable() {
        let queue = JobQueue::with_postgres(
            AccountRegistry::default(),
            "postgresql://invalid:invalid@127.0.0.1:1/unreachable",
        );

        let outcome = queue.enqueue("acct", GHOST, "unreachable queue");

        assert!(matches!(outcome, EnqueueOutcome::Unavailable(_)));
        assert!(queue.job_for_account("acct", "job-missing").is_err());
    }

    #[test]
    fn terminal_history_is_bounded_per_account() {
        let queue = JobQueue::new(AccountRegistry::default());
        for i in 0..(MAX_TERMINAL_JOBS_PER_ACCOUNT + 12) {
            let _ = queue.enqueue("acct", GHOST, &format!("job {i}"));
        }
        queue.run_pending_blocking();
        let newest = enqueue_id(queue.enqueue("acct", GHOST, "newest")); // triggers the prune

        let jobs = queue.jobs_for("acct");
        let terminal = jobs.iter().filter(|job| job.status.is_terminal()).count();
        assert!(
            terminal <= MAX_TERMINAL_JOBS_PER_ACCOUNT,
            "terminal history must stay bounded, got {terminal}"
        );
        assert!(
            jobs.len() <= MAX_TERMINAL_JOBS_PER_ACCOUNT + 1,
            "total grew unbounded: {}",
            jobs.len()
        );
        assert!(
            queue.job(&newest).is_some(),
            "the newest enqueue must survive pruning"
        );
        // Oldest-first: the very first job is gone, a recent one remains.
        // (Newest-first pruning would invert both of these.)
        let titles: Vec<&str> = jobs.iter().map(|job| job.title.as_str()).collect();
        assert!(
            !titles.contains(&"job 0"),
            "the oldest terminal job must be pruned first"
        );
        let recent = format!("job {}", MAX_TERMINAL_JOBS_PER_ACCOUNT + 11);
        assert!(
            titles.contains(&recent.as_str()),
            "a recent terminal job must survive"
        );
    }

    #[test]
    fn retention_is_per_account() {
        let queue = JobQueue::new(AccountRegistry::default());
        for i in 0..(MAX_TERMINAL_JOBS_PER_ACCOUNT + 5) {
            let _ = queue.enqueue("noisy", GHOST, &format!("n {i}"));
        }
        let quiet = enqueue_id(queue.enqueue("quiet", GHOST, "quiet-one"));
        queue.run_pending_blocking();
        let _ = queue.enqueue("noisy", GHOST, "trigger"); // prunes the noisy account only

        let noisy_terminal = queue
            .jobs_for("noisy")
            .iter()
            .filter(|job| job.status.is_terminal())
            .count();
        assert!(noisy_terminal <= MAX_TERMINAL_JOBS_PER_ACCOUNT);
        assert!(
            queue.job(&quiet).is_some(),
            "another account's job must survive the noisy account's pruning"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_drains_a_burst_without_losing_jobs() {
        // The semaphore bounds concurrency to MAX_CONCURRENT_JOBS; this proves a
        // burst several times larger still fully drains — no lost or stuck job.
        let queue = JobQueue::new(AccountRegistry::default());
        queue.spawn_worker();
        let ids: Vec<String> = (0..(MAX_CONCURRENT_JOBS * 4))
            .map(|i| enqueue_id(queue.enqueue("acct", GHOST, &format!("burst {i}"))))
            .collect();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let done = ids
                    .iter()
                    .filter(|id| queue.job(id).is_some_and(|job| job.status.is_terminal()))
                    .count();
                if done == ids.len() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the whole burst must drain within 10s");
    }

    #[test]
    fn worker_start_claim_is_idempotent() {
        let started = AtomicBool::new(false);

        assert!(claim_worker_start(&started));
        assert!(!claim_worker_start(&started));
        assert!(!claim_worker_start(&started));
    }

    /// A unique temp dir that cleans itself up, so durability tests never share
    /// state or leave litter. Dependency-free (no `tempfile` in the workspace).
    struct TempStore(PathBuf);

    impl TempStore {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("me-jobs-{tag}-{:032x}", rand::random::<u128>()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn jobs_path(&self) -> PathBuf {
            self.0.join("_jobs.json")
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn terminal_history_survives_a_restart() {
        let store = TempStore::new("restart");
        let path = store.jobs_path();

        // First "process": enqueue and drain to terminal, then drop the queue.
        let succeeded_id;
        {
            let queue = JobQueue::with_persistence(AccountRegistry::default(), path.clone());
            succeeded_id = enqueue_id(queue.enqueue("acct", GHOST, "first run"));
            queue.run_pending_blocking(); // GHOST fails fast -> terminal
        }

        // Second "process": a fresh queue over the same file restores history.
        let queue = JobQueue::with_persistence(AccountRegistry::default(), path);
        let restored = queue.jobs_for("acct");
        assert_eq!(restored.len(), 1, "the job must survive the restart");
        let job = queue.job(&succeeded_id).expect("job restored by id");
        assert_eq!(job.title, "first run");
        assert!(job.status.is_terminal(), "terminal status must persist");
        // The owning account is restored too (skipped on the wire, kept on disk).
        assert_eq!(job.account_id, "acct");
        assert_eq!(job.source_id, GHOST);
    }

    #[test]
    fn interrupted_in_flight_jobs_restore_as_retryable_failed() {
        let store = TempStore::new("interrupted");
        let path = store.jobs_path();
        // A history file as a crash would leave it: one job of every status.
        let history = serde_json::json!([
            { "id": "q", "account_id": "acct", "source_id": "s", "title": "queued",
              "status": "queued", "card_count": 0, "attempts": 0, "error": null,
              "created_at": 1, "updated_at": 1 },
            { "id": "r", "account_id": "acct", "source_id": "s", "title": "running",
              "status": "running", "card_count": 0, "attempts": 1, "error": null,
              "created_at": 2, "updated_at": 2 },
            { "id": "ok", "account_id": "acct", "source_id": "s", "title": "done",
              "status": "succeeded", "card_count": 3, "attempts": 1, "error": null,
              "created_at": 3, "updated_at": 3 },
            { "id": "no", "account_id": "acct", "source_id": "s", "title": "broke",
              "status": "failed", "card_count": 0, "attempts": 1, "error": "boom",
              "created_at": 4, "updated_at": 4 },
        ]);
        std::fs::write(&path, serde_json::to_vec(&history).unwrap()).unwrap();

        let queue = JobQueue::with_persistence(AccountRegistry::default(), path.clone());

        // In-flight (queued/running) reset to a retryable failure...
        for id in ["q", "r"] {
            let job = queue.job(id).expect("restored");
            assert_eq!(job.status, JobStatus::Failed, "{id} must reset to failed");
            assert!(
                job.error.as_deref().is_some_and(|e| e.contains("restart")),
                "{id} must carry the interrupted-by-restart notice"
            );
        }
        // ...while genuinely terminal jobs are untouched.
        assert_eq!(queue.job("ok").unwrap().status, JobStatus::Succeeded);
        assert_eq!(queue.job("ok").unwrap().card_count, 3);
        assert_eq!(queue.job("no").unwrap().error.as_deref(), Some("boom"));

        // The reset is durable immediately, not just in memory: re-reading the
        // file shows no surviving in-flight status, so a second crash before any
        // mutation can't resurrect a "running" job.
        let on_disk: Vec<PersistedJob> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            on_disk.iter().all(|job| job.status.is_terminal()),
            "the in-flight -> failed reset must be written back to disk"
        );
    }

    #[test]
    fn new_queue_is_in_memory_and_ignores_history_on_disk() {
        let store = TempStore::new("inmem");
        let path = store.jobs_path();
        // Leave durable history at the path a persistent queue would use.
        {
            let durable = JobQueue::with_persistence(AccountRegistry::default(), path.clone());
            let _ = durable.enqueue("acct", GHOST, "persisted");
        }
        assert!(
            path.exists(),
            "the persistent queue must have written history"
        );

        // An in-memory queue never reads it back — restart loses the history.
        let volatile = JobQueue::new(AccountRegistry::default());
        assert!(
            volatile.jobs_for("acct").is_empty(),
            "JobQueue::new must not restore history from disk"
        );
    }

    #[test]
    fn rerunning_a_generation_job_is_idempotent_after_an_interrupted_schedule() {
        let store = TempStore::new("idempotent-generation");
        let registry = AccountRegistry::with_store_root(store.0.clone());
        let account = registry
            .create_account("idempotent@example.com")
            .expect("test account");
        let source = registry
            .save_source(
                &account.account_id,
                &account.session_token,
                &crate::CreateSourceRequest {
                    title: "Idempotent source".to_owned(),
                    body: "Concept: Stable generation\nQuestion: What stays stable?\nAnswer: The job identity."
                        .to_owned(),
                },
            )
            .expect("source");

        let first = registry
            .run_generation_job(
                &account.account_id,
                &source.source_id,
                "test-run-1",
                0,
                "",
                || true,
            )
            .expect("first generation");
        let second = registry
            .run_generation_job(
                &account.account_id,
                &source.source_id,
                "test-run-2",
                0,
                "",
                || true,
            )
            .expect("replayed generation");
        assert!(first > 0, "fixture must produce scheduled material");
        assert_eq!(second, 0, "a replay must not schedule duplicate material");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn reclaimed_postgres_generation_cannot_commit_after_the_lease_turns_stale() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!(
                "skipping live Postgres fence regression; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
            );
            return;
        };
        let now_ms = 1_779_465_600_000_i64;
        let schema = format!("memory_engine_test_fence_{}_{}", std::process::id(), now_ms);
        let scoped_url = format!(
            "{}{}options=-csearch_path%3D{}",
            database_url,
            if database_url.contains('?') { '&' } else { '?' },
            schema
        );
        let mut admin = memory_engine_persistence_postgres::connect_client(&database_url)
            .expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let result = (|| -> Result<(), String> {
            let registry = AccountRegistry::with_postgres_url(scoped_url.clone());
            let account = registry
                .create_account("fence@example.com")
                .map_err(|error| error.message.clone())?;
            let source = registry
                .save_source(
                    &account.account_id,
                    &account.session_token,
                    &crate::CreateSourceRequest {
                        title: "Fence source".to_owned(),
                        body: "Concept: Fence\nQuestion: What keeps stale work out?\nAnswer: The durable attempt ledger."
                            .to_owned(),
                    },
                )
                .map_err(|error| error.message.clone())?;
            let mut ledger =
                PostgresStudyStore::connect(&scoped_url).map_err(|error| error.to_string())?;
            ledger.migrate().map_err(|error| error.to_string())?;
            let started = ledger
                .enqueue_generation_job(
                    &account.account_id,
                    "job-fence",
                    &source.source_id,
                    "Fence source",
                    "model-fence",
                    now_ms,
                    2,
                    4,
                    100,
                    86_400_000,
                )
                .map_err(|error| error.to_string())?;
            let job = match started {
                memory_engine_persistence_postgres::PostgresEnqueueOutcome::Started(job) => job,
                other => return Err(format!("unexpected enqueue outcome: {other:?}")),
            };
            let run_id = format!("job:{}:attempt:{}", job.id, 1);
            let claimed = ledger
                .claim_generation_job("worker-a", now_ms, 10, 0, 1, 3)
                .map_err(|error| error.to_string())?
                .expect("claim job");
            assert_eq!(claimed.id, "job-fence");
            assert!(ledger
                .bind_generation_job_attempt_run(
                    &account.account_id,
                    &claimed.id,
                    claimed.attempts,
                    claimed.lease_token.as_deref().expect("lease token"),
                    &run_id,
                )
                .map_err(|error| error.to_string())?);

            let (reclaim_tx, reclaim_rx) = std::sync::mpsc::channel::<()>();
            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
            let reclaim_triggered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reclaim_url = scoped_url.clone();
            let reclaim_account_id = account.account_id.clone();
            let reclaim_job_id = claimed.id.clone();
            let reclaim_run_id = run_id.clone();
            let worker = std::thread::spawn(move || {
                reclaim_rx.recv().expect("reclaim signal");
                let mut store =
                    PostgresStudyStore::connect(&reclaim_url).expect("connect reclaim store");
                store.migrate().expect("migrate reclaim store");
                let _ = store
                    .claim_generation_job("worker-b", now_ms + 16, 10, 0, 1, 3)
                    .expect("reclaim stale job");
                let _ = reclaim_account_id;
                let _ = reclaim_job_id;
                let _ = reclaim_run_id;
                done_tx.send(()).expect("done signal");
            });
            let approval_gate = {
                let reclaim_tx = reclaim_tx.clone();
                let done_rx = std::sync::Arc::new(std::sync::Mutex::new(done_rx));
                let reclaim_triggered = std::sync::Arc::clone(&reclaim_triggered);
                move || {
                    if !reclaim_triggered.swap(true, std::sync::atomic::Ordering::AcqRel) {
                        reclaim_tx.send(()).expect("send reclaim signal");
                        done_rx.lock().expect("done lock").recv().expect("done ack");
                    }
                    true
                }
            };
            let outcome = registry.run_generation_job(
                &account.account_id,
                &source.source_id,
                &run_id,
                i32::try_from(claimed.attempts).unwrap_or(i32::MAX),
                claimed.lease_token.as_deref().unwrap_or_default(),
                approval_gate,
            );
            worker.join().expect("worker thread");
            let scope =
                memory_engine_persistence_postgres::AccountScope::new(account.account_id.clone())
                    .map_err(|error| error.to_string())?;
            let account = ledger.for_account(scope);
            let snapshot = account.snapshot().map_err(|error| error.to_string())?;
            assert!(
                snapshot.review_units.is_empty(),
                "stale worker must not commit any review units after reclaim"
            );
            match outcome {
                Ok(_) => Err("stale worker was able to commit review units".to_owned()),
                Err(error) => {
                    assert!(
                        error.message.contains("Generation lease lost")
                            || error.message.contains("committed"),
                        "unexpected fence error: {}",
                        error.message
                    );
                    Ok(())
                }
            }
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("postgres generation fence");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn postgres_generation_cannot_commit_after_live_lease_expiry_without_reclaim() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!(
                "skipping live Postgres expiry regression; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
            );
            return;
        };
        let start_ms = 1_779_465_600_000_i64;
        TEST_CLOCK_MS.store(start_ms, std::sync::atomic::Ordering::SeqCst);
        let schema = format!(
            "memory_engine_test_live_expiry_{}_{}",
            std::process::id(),
            start_ms
        );
        let scoped_url = format!(
            "{}{}options=-csearch_path%3D{}",
            database_url,
            if database_url.contains('?') { '&' } else { '?' },
            schema
        );
        let mut admin = memory_engine_persistence_postgres::connect_client(&database_url)
            .expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let result = (|| -> Result<(), String> {
            let registry =
                AccountRegistry::with_postgres_url(scoped_url.clone()).with_clock(test_clock_ms);
            let account = registry
                .create_account("expiry@example.com")
                .map_err(|error| error.message.clone())?;
            let source = registry
                .save_source(
                    &account.account_id,
                    &account.session_token,
                    &crate::CreateSourceRequest {
                        title: "Expiry source".to_owned(),
                        body: "Concept: Expiry\nQuestion: What keeps stale work out?\nAnswer: The durable attempt ledger."
                            .to_owned(),
                    },
                )
                .map_err(|error| error.message.clone())?;
            let mut ledger =
                PostgresStudyStore::connect(&scoped_url).map_err(|error| error.to_string())?;
            ledger.migrate().map_err(|error| error.to_string())?;
            let started = ledger
                .enqueue_generation_job(
                    &account.account_id,
                    "job-expiry",
                    &source.source_id,
                    "Expiry source",
                    "model-expiry",
                    start_ms,
                    2,
                    4,
                    100,
                    5_000,
                )
                .map_err(|error| error.to_string())?;
            let job = match started {
                memory_engine_persistence_postgres::PostgresEnqueueOutcome::Started(job) => job,
                other => return Err(format!("unexpected enqueue outcome: {other:?}")),
            };
            let run_id = format!("job:{}:attempt:{}", job.id, 1);
            let claimed = ledger
                .claim_generation_job("worker-a", start_ms, 10, 0, 1, 3)
                .map_err(|error| error.to_string())?
                .expect("claim job");
            assert_eq!(claimed.id, "job-expiry");
            assert!(ledger
                .bind_generation_job_attempt_run(
                    &account.account_id,
                    &claimed.id,
                    claimed.attempts,
                    claimed.lease_token.as_deref().expect("lease token"),
                    &run_id,
                )
                .map_err(|error| error.to_string())?);

            let generation_attempt = i32::try_from(claimed.attempts).unwrap_or(i32::MAX);
            let generation_lease_token = claimed.lease_token.clone().unwrap_or_default();
            let outcome = registry.run_generation_job(
                &account.account_id,
                &source.source_id,
                &run_id,
                generation_attempt,
                &generation_lease_token,
                move || {
                    TEST_CLOCK_MS.store(start_ms + 16, std::sync::atomic::Ordering::SeqCst);
                    true
                },
            );
            let err = outcome.expect_err("expired lease must block commit");
            assert!(
                err.message.contains("Generation lease lost") || err.message.contains("committed"),
                "unexpected fence error: {}",
                err.message
            );
            let scope =
                memory_engine_persistence_postgres::AccountScope::new(account.account_id.clone())
                    .map_err(|error| error.to_string())?;
            let account = ledger.for_account(scope);
            let snapshot = account.snapshot().map_err(|error| error.to_string())?;
            assert!(
                snapshot.review_units.is_empty(),
                "expired lease must not commit review units"
            );
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        TEST_CLOCK_MS.store(0, std::sync::atomic::Ordering::SeqCst);
        result.expect("live lease expiry fence");
    }
}
