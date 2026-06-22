//! Background generation jobs: asynchronous, non-blocking card generation.
//!
//! Creating material no longer blocks the request. A capture enqueues a
//! `GenerationJob`, the handler returns immediately, and an in-process worker
//! runs the (synchronous, ~20s) model call on a blocking thread, then
//! optimistically approves and schedules the resulting cards. Job status
//! (`queued → running → succeeded | failed`) is held in memory and pushed to
//! the browser over SSE; failed jobs can be retried.
//!
//! Job history is held in memory and, on the file-backed host, mirrored to a
//! `_jobs.json` file under the store root so the activity log survives a
//! restart (the *cards* were already durable — the study store persists them on
//! success). A job still running when the process stops is restored as a
//! retryable `failed` ("interrupted by a restart"), since no worker owns it
//! after the restart. The postgres host keeps history in memory for now (a
//! durable table behind the same `JobQueue` surface is the scale path).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::AccountRegistry;

/// Most generation jobs run at once; the rest wait. Bounds concurrent model
/// calls so a burst of captures can't open dozens of sockets at once.
const MAX_CONCURRENT_JOBS: usize = 4;
/// Capacity of the SSE broadcast buffer. Events are full job snapshots, so a
/// slow subscriber that lags simply skips to the latest state.
const UPDATES_BUFFER: usize = 256;
/// Per-account cap on *terminal* (succeeded/failed) job history. In-flight jobs
/// are never pruned (the worker still owns them by id); this just keeps the
/// activity log and the in-memory Vec from growing without bound over a
/// long-lived process. The generated cards are durably persisted regardless.
const MAX_TERMINAL_JOBS_PER_ACCOUNT: usize = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

    /// Succeeded/failed jobs no longer change on their own, so they are the
    /// prunable history (see `MAX_TERMINAL_JOBS_PER_ACCOUNT`). This also governs
    /// crash-restore: a *non*-terminal job is reset to a retryable failure on
    /// restart, since no worker owns it after the restart (see [`load_jobs`]).
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
    error: Option<String>,
    created_at: i64,
    updated_at: i64,
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
            error,
            created_at,
            updated_at,
        } = job;
        Self {
            id: id.clone(),
            account_id: account_id.clone(),
            source_id: source_id.clone(),
            title: title.clone(),
            status: *status,
            card_count: *card_count,
            attempts: *attempts,
            error: error.clone(),
            created_at: *created_at,
            updated_at: *updated_at,
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
            error: record.error,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
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
}

impl JobQueue {
    /// Build an in-memory queue bound to `registry`. History is lost on restart;
    /// use [`JobQueue::with_persistence`] for the durable host. The worker is not
    /// started until [`JobQueue::spawn_worker`] is called (so tests can drive
    /// jobs directly).
    #[must_use]
    pub fn new(registry: AccountRegistry) -> Self {
        Self::build(registry, Vec::new(), None)
    }

    /// Build a queue whose history is mirrored to `path` and restored from it on
    /// construction. A job left non-terminal by a crash is restored as a
    /// retryable `failed` (see [`load_jobs`]).
    #[must_use]
    pub fn with_persistence(registry: AccountRegistry, path: PathBuf) -> Self {
        let restored = load_jobs(&path, registry.now());
        let queue = Self::build(registry, restored, Some(path));
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
        // Commit to the Vec first, then broadcast — so an SSE subscriber that
        // reacts to the event can always find the job via `jobs_for`/`job`. This
        // matches the commit-then-broadcast order of the other mutators.
        let snapshot = job.clone();
        {
            let mut jobs = self.lock_jobs();
            jobs.push(job);
            enforce_terminal_retention(&mut jobs, account_id);
        }
        self.broadcast(&snapshot);
        self.persist();
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

    #[test]
    fn terminal_history_is_bounded_per_account() {
        let queue = JobQueue::new(AccountRegistry::default());
        for i in 0..(MAX_TERMINAL_JOBS_PER_ACCOUNT + 12) {
            queue.enqueue("acct", GHOST, &format!("job {i}"));
        }
        queue.run_pending_blocking();
        let newest = queue.enqueue("acct", GHOST, "newest"); // triggers the prune

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
            queue.enqueue("noisy", GHOST, &format!("n {i}"));
        }
        let quiet = queue.enqueue("quiet", GHOST, "quiet-one");
        queue.run_pending_blocking();
        queue.enqueue("noisy", GHOST, "trigger"); // prunes the noisy account only

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
            .map(|i| queue.enqueue("acct", GHOST, &format!("burst {i}")))
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
            succeeded_id = queue.enqueue("acct", GHOST, "first run");
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
            durable.enqueue("acct", GHOST, "persisted");
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
}
