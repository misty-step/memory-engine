# Durable, bounded generation job queue

Priority: P3 · Status: pending · Estimate: M

## Goal

The async generation job queue (shipped on `feat/055-gen-quality-latency`) is an
in-memory first cut: jobs live in a `Mutex<Vec<GenerationJob>>` that is never
pruned and is lost on restart. Make it durable and bounded so a long-lived
process neither grows without limit nor forgets job history across restarts.

## Oracle

- [ ] Job history survives an API restart (a file `jobs.json` and/or a postgres
      table behind the existing `JobQueue` surface — `jobs.rs` already names this
      as the planned shape).
- [ ] Per-account job retention is bounded (e.g. keep the last N), so
      `jobs_for` / `broadcast` stay O(N) and memory cannot grow unboundedly over
      a long-lived process.
- [ ] A transient transport failure (the 60s `ureq` timeout, a 5xx) is
      auto-retried once before the job surfaces as `failed`, so a learner is not
      forced to click Retry for a blip. A real generation rejection (zero
      keepable drafts) still surfaces immediately.
- [ ] The spawned worker's concurrency bound (`Semaphore(MAX_CONCURRENT_JOBS)`)
      is covered by a burst test (enqueue > N, assert at most N run at once).

## Notes

Follow-on from the 055 async-generation work, surfaced by fresh-context review.
The in-memory cut was deliberate (cards are durably persisted on success; only
ephemeral job history is lost) — this ticket closes the durability / retention /
resilience gap once the feature has proven out. The same latent unbounded-growth
pattern exists in `submitted_reviews` and `browser_sessions`; fold those into the
retention pass if cheap.
