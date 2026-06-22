# Durable, bounded generation job queue

Priority: P3 · Status: in progress · Estimate: M

## Goal

The async generation job queue (shipped on `feat/055-gen-quality-latency`) is an
in-memory first cut: jobs live in a `Mutex<Vec<GenerationJob>>` that is never
pruned and is lost on restart. Make it durable and bounded so a long-lived
process neither grows without limit nor forgets job history across restarts.

## Oracle

- [ ] Job history survives an API restart (a file `jobs.json` and/or a postgres
      table behind the `JobQueue` surface). **Deferred — lowest impact:** the
      generated cards already persist on success, so only ephemeral activity
      history is lost on restart. Bounding growth (below) is the real near-term
      fix; durable history can follow.
- [x] Per-account job retention is bounded: `enqueue` prunes terminal
      (succeeded/failed) history to the last `MAX_TERMINAL_JOBS_PER_ACCOUNT` (50)
      per account, so the Vec and `jobs_for`/`broadcast` cannot grow without
      bound. In-flight jobs are never pruned. Unit-tested (bounded + per-account).
- [ ] A transient transport failure is auto-retried once before surfacing
      `failed`. **Deferred — UX nicety:** the learner can already click Retry, and
      classifying transient-vs-real failures cleanly needs a failure-taxonomy pass
      on `run_generation_job`.
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
