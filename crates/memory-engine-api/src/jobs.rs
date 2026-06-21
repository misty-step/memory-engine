//! Background generation jobs: asynchronous, non-blocking card generation.
//!
//! Creating material no longer blocks the request. A capture enqueues a
//! `GenerationJob`, the handler returns immediately, and an in-process worker
//! runs the (synchronous, ~20s) model call on a blocking thread, then
//! optimistically approves and schedules the resulting cards. Job status
//! (`queued → running → succeeded | failed`) is held in memory and pushed to
//! the browser over SSE; failed jobs can be retried.
//!
//! In-memory is the deliberate first cut: the *cards* are durably persisted by
//! the study store on success, so only ephemeral job history is lost on
//! restart. Durable job persistence (a file `jobs.json` and a postgres table)
//! is a planned follow-up behind the same `JobQueue` surface.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::AccountRegistry;

/// Most generation jobs run at once; the rest wait. Bounds concurrent model
/// calls so a burst of captures can't open dozens of sockets at once.
const MAX_CONCURRENT_JOBS: usize = 4;
/// Capacity of the SSE broadcast buffer. Events are full job snapshots, so a
/// slow subscriber that lags simply skips to the latest state.
const UPDATES_BUFFER: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
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
#[derive(Clone, Debug, Serialize)]
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
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
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
}

impl JobQueue {
    /// Build a queue bound to `registry`. The worker is not started until
    /// [`JobQueue::spawn_worker`] is called (so tests can drive jobs directly).
    #[must_use]
    pub fn new(registry: AccountRegistry) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (updates, _keep_alive) = broadcast::channel(UPDATES_BUFFER);
        Self {
            inner: Arc::new(Inner {
                registry,
                jobs: Mutex::new(Vec::new()),
                tx,
                rx: Mutex::new(Some(rx)),
                updates,
            }),
        }
    }

    /// Start the background worker. Must run inside a Tokio runtime. Idempotent:
    /// a second call is a no-op because the single receiver is already taken.
    pub fn spawn_worker(&self) {
        let Some(rx) = self.lock_rx().take() else {
            return;
        };
        let worker = self.clone();
        tokio::spawn(worker.run(rx));
    }

    /// Enqueue generation for an already-saved source. Returns the job id; the
    /// caller renders immediately while the worker runs in the background.
    // Callers fire-and-forget (the id is incidental), so the return is routinely
    // discarded; `must_use` would only force noisy `let _ =` at every site.
    #[allow(clippy::must_use_candidate)]
    pub fn enqueue(&self, account_id: &str, source_id: &str, title: &str) -> String {
        let now = self.inner.registry.now();
        let id = format!("job-{:032x}", rand::random::<u128>());
        let job = GenerationJob {
            id: id.clone(),
            account_id: account_id.to_owned(),
            source_id: source_id.to_owned(),
            title: title.to_owned(),
            status: JobStatus::Queued,
            card_count: 0,
            attempts: 0,
            error: None,
            created_at: now,
            updated_at: now,
        };
        self.broadcast(&job);
        self.lock_jobs().push(job);
        // Send never fails while the queue is alive (the receiver lives in the
        // worker); if the worker was never started the job simply stays queued.
        let _ = self.inner.tx.send(id.clone());
        id
    }

    /// Re-queue a failed job (the learner pressed Retry). Scoped to the owning
    /// account so one learner cannot touch another's jobs.
    // The handler branches on the bool inline; other callers ignore it. Marking
    // it `must_use` would force a `let _ =` at the fire-and-forget sites.
    #[allow(clippy::must_use_candidate)]
    pub fn retry(&self, account_id: &str, job_id: &str) -> bool {
        let requeued = {
            let mut jobs = self.lock_jobs();
            match jobs
                .iter_mut()
                .find(|job| job.id == job_id && job.account_id == account_id)
            {
                Some(job) if job.status == JobStatus::Failed => {
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

    /// Look up a single job (test + handler convenience).
    #[must_use]
    pub fn job(&self, job_id: &str) -> Option<GenerationJob> {
        self.lock_jobs()
            .iter()
            .find(|job| job.id == job_id)
            .cloned()
    }

    /// Synchronously run every queued job to completion, in order. The spawned
    /// worker drains the queue asynchronously in production; this is the
    /// deterministic path for tests (and any host without an async runtime).
    pub fn run_pending_blocking(&self) {
        while let Some(job_id) = self.next_queued() {
            if let Some((account_id, source_id)) = self.mark_running(&job_id) {
                let result = self
                    .inner
                    .registry
                    .run_generation_job(&account_id, &source_id)
                    .map_err(|failure| failure.message);
                self.finish(&job_id, result);
            }
        }
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

    async fn run_job(&self, job_id: &str) {
        let Some((account_id, source_id)) = self.mark_running(job_id) else {
            return;
        };
        let registry = self.inner.registry.clone();
        // The model call is synchronous (`ureq`); run it off the async runtime
        // on the blocking pool so it never parks a Tokio worker thread.
        let outcome = tokio::task::spawn_blocking(move || {
            registry.run_generation_job(&account_id, &source_id)
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
                    job.error = Some(message);
                }
            }
            job.updated_at = now;
            job.clone()
        };
        self.broadcast(&snapshot);
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
