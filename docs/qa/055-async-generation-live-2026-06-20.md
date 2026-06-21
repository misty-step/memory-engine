# QA: async generation + study flow (ticket 055)

Live request-replay walk of the non-blocking generation pipeline and study
surface on `feat/055-gen-quality-latency`, driven against a real running
`memory-engine-api` host with a real OpenRouter model call.

- **Date:** 2026-06-20
- **Build:** branch `feat/055-gen-quality-latency`, file store, debug-link auth.
- **Host:** `cargo run -p memory-engine-api` on `127.0.0.1:8099`
  (`MEMORY_ENGINE_ENABLE_FILE_STORE=true`, real `OPENROUTER_API_KEY`).
- **Method:** curl with a cookie jar (the Chrome extension was unauthenticated,
  so the browser walk fell back to HTTP replay; the app is server-rendered, so
  the activity log is authoritative on every page load).

## What was verified

| # | Claim | Evidence |
|---|-------|----------|
| 1 | Capture is non-blocking (055 latency fix) | `POST /app/capture` returned in **0.01s real** — it did not wait on the ~20s model call. |
| 2 | The background worker runs real generation | Activity-log job `job-11e7…` transitioned to `data-status="succeeded"`, meta **"26 cards · scheduled for review"**. |
| 3 | Cards auto-schedule and become due | Workspace then showed due cards / a "Start review" entry. |
| 4 | Distractor cohesion (055 quality) | The generated "NATO word for A?" MCQ offered **Alfa / Amber / Atlas / Apollo** — all same-category A-words, the exact shape the strengthened distractor prompt targets. |
| 5 | Review + graded reveal | Submitting the correct choice rendered verdict **"Correct"**, marked `me-graded-choice-correct`, and scheduled "again in ~1 hour". |
| 6 | Session-derived CSRF (refactor) | The same `csrf_…` token was stable across the workspace, capture, and review forms and was accepted on every mutation. |
| 7 | Every UI state renders | `design_preview::emit_preview_pages` wrote 11 states (signed-out, capture-queued, activity, due, review choices/open, graded correct/wrong, revealed, check-email); the `conformance_*` tests guard the design Law. |
| 8 | Retry transition | `mobile_retry_requeues_and_reruns_a_failed_job` drives the real `/app/jobs/retry` endpoint and asserts the worker re-runs (attempts 1→2). |

## Residual / not covered here

- **p50 latency not formally measured** (055 oracle item 4 remainder). The fix
  makes the *request* non-blocking; per-source model compute is unchanged.
- **SSE live-streaming** (`/app/jobs/events` + `app.js`) was only smoke-reached;
  the server-authoritative log was the verified path. The real Tokio worker's
  concurrency bound / broadcast fan-out is exercised in production but not
  load-tested (the deterministic `run_pending_blocking` shim covers the logic).
- **Judged field run** for the distractor/keep-rate deltas (055 oracle item 1)
  is owed as a `docs/evals/` receipt; this walk shows one good MCQ, not a scored
  batch.
