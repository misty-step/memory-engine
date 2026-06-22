# Durable, bounded generation job queue

Priority: P3 · Status: done · Estimate: M

## Goal

The async generation job queue (shipped on `feat/055-gen-quality-latency`) is an
in-memory first cut: jobs live in a `Mutex<Vec<GenerationJob>>` that is never
pruned and is lost on restart. Make it durable and bounded so a long-lived
process neither grows without limit nor forgets job history across restarts.

## Oracle

- [x] Job history survives an API restart. The file-backed host mirrors the job
      Vec to `_jobs.json` under the store root (crash-durably: `write_atomic`
      fsyncs the temp file + parent dir) and restores it on boot; a job left
      in-flight by a crash comes back as a retryable `failed`. The postgres host
      keeps history in memory (a durable table behind the same `JobQueue`
      surface is the scale path). Unit-tested + an integration test that drives
      capture → restart → restore through the real router.
- [x] Per-account job retention is bounded: `enqueue` prunes terminal
      (succeeded/failed) history to the last `MAX_TERMINAL_JOBS_PER_ACCOUNT` (50)
      per account, so the Vec and `jobs_for`/`broadcast` cannot grow without
      bound. In-flight jobs are never pruned. Unit-tested (bounded + per-account).
- [x] A transient transport failure is auto-retried once before surfacing
      `failed`. The taxonomy lives at the provider boundary: `ProviderFailure`
      carries a `transient` bit (set for unreachable/timeout and 5xx/429, clear
      for 4xx/malformed), and `OpenRouterProvider::complete_structured` retries
      once on a transient failure before returning. Covers every model call
      (drafts, repair, reference, bridge); the learner-facing Retry remains for
      failures that persist. Mock-server tested (503→retry→succeed; 400→no retry).
- [x] The spawned worker drains a burst larger than `MAX_CONCURRENT_JOBS` with no
      lost/stuck job (`multi_thread` test). Peak concurrency is bounded by the
      `Semaphore` by construction; measuring the peak would need a
      blocking-provider seam (not worth it for a standard semaphore).

## Notes

Follow-on from the 055 async-generation work, surfaced by fresh-context review.
The in-memory cut was deliberate (cards are durably persisted on success; only
ephemeral job history is lost) — this ticket closes the durability / retention /
resilience gap once the feature has proven out. The same latent unbounded-growth
pattern exists in `submitted_reviews` and `browser_sessions`; fold those into the
retention pass if cheap.

## Progress — feat/057-bounded-job-queue (2026-06-21)

Shipped the high-impact half: per-account terminal-history retention (the
unbounded-growth fix flagged in the 055 review) + a burst-drain test on the real
spawned worker. Durable persistence (oracle 1) and auto-retry (oracle 3) are
reframed as lower-priority follow-ups — durable history is cosmetic (cards
persist on success), and auto-retry is a UX nicety needing a failure-taxonomy
pass. The `submitted_reviews` / `browser_sessions` growth note is a separate
retention pass, not done here.

## Progress — feat/057-durable-resilient-queue (2026-06-22)

Closed the two deferred oracles, so all four are met and the ticket is **done**:

- **Durable history (oracle 1):** `JobQueue::with_persistence` mirrors the job Vec
  to `_jobs.json` under the store root and restores it on boot; an in-flight job
  interrupted by a crash comes back as a retryable `failed`. `ApiState::new`
  enables it for the file-backed host only (postgres + tests stay in memory).
  Writes go through a single crash-durable `write_atomic` (fsync temp + parent
  dir), consolidated from the prior fsync-free duplicate.
- **Auto-retry (oracle 3):** a `transient` bit on `ProviderFailure` (set at
  OpenRouter's `transport_failure` for unreachable/timeout + 5xx/429) drives a
  one-shot retry inside `complete_structured`, covering every model call.
- **Verification:** unit tests (round-trip, interrupted-reset + durable-on-disk,
  in-memory isolation), an integration test (capture → restart → restore through
  the real router + worker), and openrouter mock-server retry tests. Thermo-nuclear
  review (1 MAJOR + 6 MINOR) addressed; fresh-context re-review returned APPROVE.
  Live binary confirmed to boot file-store mode and write `_jobs.json`. Receipt:
  `docs/qa/057-durable-resilient-queue-2026-06-22.md`.

The `submitted_reviews` / `browser_sessions` unbounded-growth note remains a
separate retention pass (not done here).
