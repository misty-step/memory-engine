# QA: durable + resilient generation job queue (ticket 057)

Verification of the two delivered 057 oracles on `feat/057-durable-resilient-queue`:
durable job history across an API restart (oracle 1) and a one-shot auto-retry of
a transient transport failure (oracle 3).

- **Date:** 2026-06-22
- **Build:** branch `feat/057-durable-resilient-queue`, file-backed host.
- **Gate:** `cargo fmt --all --check`, `cargo +1.94 clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc
  --workspace --no-deps` — all green (1.94 = the Dagger-pinned toolchain).

## Oracle 1 — job history survives a restart

| # | Claim | Evidence |
|---|-------|----------|
| 1 | The production wiring restores history across a restart | New integration test `job_history_survives_a_restart_through_the_file_backed_host`: captures a source through the **real axum router + spawned worker**, waits for the durable `_jobs.json` write, then builds a **fresh `ApiState` on the same store dir** and asserts the succeeded job is restored by id (status `succeeded`, `card_count ≥ 1`, owning account intact, visible in `jobs_for`). Exercises `ApiState::new → with_persistence` end to end. |
| 2 | The real binary persists to disk in file-store mode | A live `memory-engine-api` (file store, `MEMORY_ENGINE_ENABLE_FILE_STORE=true`) boots and writes `_jobs.json` under the store root — the boot-time write (`[]`) confirms `with_persistence` is wired and the restart-reset is flushed immediately (the m5 fix), and a restart reads the file back. |
| 3 | An in-flight job interrupted by a crash comes back retryable | Unit test `interrupted_in_flight_jobs_restore_as_retryable_failed`: a history file with one job of each status restores `queued`/`running` → `failed` ("Interrupted by a server restart…"), leaves genuine `succeeded`/`failed` untouched, and **re-reads the file to confirm the reset is durable**, not just in memory. |
| 4 | A round-trip survives | Unit test `terminal_history_survives_a_restart`: enqueue + drain in one queue, then a fresh queue on the same path restores the terminal job with its title/account/source. |
| 5 | In-memory queues never touch disk | Unit test `new_queue_is_in_memory_and_ignores_history_on_disk`: `JobQueue::new` ignores an existing history file (tests + the postgres host stay in memory). |
| 6 | Writes are crash-durable | `write_atomic` fsyncs the temp file before the rename and best-effort fsyncs the parent dir, so a power loss leaves the old file or the new — never a truncated one. |

## Oracle 3 — transient transport failure auto-retried once

| # | Claim | Evidence |
|---|-------|----------|
| 7 | A transient 5xx is retried and the retry succeeds | Integration test `retries_once_on_transient_5xx_then_succeeds` against a real mock HTTP server: first connection returns 503, second returns 200 → `generate_drafts` succeeds, and the server confirms **exactly two connections**. |
| 8 | A permanent 4xx fails fast, no retry | `does_not_retry_on_permanent_client_error`: a 400 with a success queued behind it → `generate_drafts` errors with "rejected the request (HTTP 400)", and the server confirms **exactly one connection** (the queued success is never reached). |

## Residual / not covered here

- **Live binary capture-via-curl was not completed.** The app sets a `__Host-`
  `Secure` session cookie that curl won't hold over plain HTTP, and the
  anonymous `/app/start` session path is gated by the binary's required auth
  config. The capture → persist → restore path is instead proven by the
  integration test above, which drives the identical `router()` / `ApiState` /
  spawned-worker / `_jobs.json` code path and differs from the binary only at
  the OS-process boundary — the durability of which is covered by `write_atomic`'s
  fsync and the two-instances-on-one-path tests.
- **Durable persistence on the postgres host** stays in memory by design (a
  durable table behind the same `JobQueue` surface is the documented scale path);
  only the file-backed host mirrors to `_jobs.json`.
- **Auto-retry interaction with the repair pass**: a generation that also repairs
  can issue up to two retried model calls, doubling worst-case spend per source
  under sustained transient failure — documented at the retry site.
