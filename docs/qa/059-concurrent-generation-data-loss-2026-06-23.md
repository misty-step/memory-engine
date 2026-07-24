# QA: concurrent-generation data loss (ticket 059)

Verification that concurrent generation jobs for one account no longer clobber
each other's cards on the file-backed store.

- **Date:** 2026-06-23
- **Branch:** `feat/059-concurrent-generation-data-loss`
- **Gate:** `cargo fmt --all --check`, `cargo +1.94 clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc
  --workspace --no-deps` — all green.

## The bug, reproduced from live data

The dogfood store (`~/.memory-engine/store`) showed the smoking gun: the NATO
job reported `card_count=26`, but `study-run-1` (NATO) had `draftIds: []` /
`completedAt: null` while `study-run-2` (Apostles) owned all 7 drafts + 6 review
units. The two jobs started 15s apart with ~20s model calls, so they overlapped.
Root cause confirmed in code: the persistence layer rewrites the whole
`study.json` per save (`serde_json::to_string_pretty` → atomic `fs::rename`) with
no lock, and 055's worker runs up to 4 generations at once — a lost-update race.

## Verification — live surface, deterministic

The regression test
`concurrent_generations_for_one_account_do_not_clobber_each_other` **is** the
live-surface check: it drives the **real axum router + the real spawned Tokio
worker + the real file-backed store** through the exact concurrent path. It
captures 6 sources sequentially (worker off, so the *source* writes don't race),
then starts the worker so all 6 generate at once (semaphore = 4), and asserts the
data-integrity invariant `sum(card_count over jobs) == reviewUnits on disk`.

- **Red (unfixed):** `8 reported across jobs, 5 on disk` — the clobber.
- **Green (fixed):** deterministic across 5 consecutive runs; `sum == persisted`.
- **Offline + hermetic:** structured-block fixtures route through the
  deterministic provider (`FallbackProvider` primary), so no model call — verified
  **0.22s with `OPENROUTER_API_KEY` set** in the env (a real call would be ~20s).

### Why not a manual binary walk

The race needs concurrent generations to *overlap*. With the deterministic
offline provider, generation is sub-millisecond, so a casual binary capture
wouldn't reliably overlap; forcing it via real ~20s model calls would cost API
spend and be flaky. The integration test forces the overlap deterministically
(worker-off-then-all-at-once) — strictly better evidence for a concurrency bug
than a hand walk, and it exercises the identical router/worker/store code the
binary runs.

## Production exposure

Prod is **Postgres** (`MEMORY_ENGINE_POSTGRES_URL` deployed) — DB transactions
already covered concurrency, so production did not lose data. The clobber was
specific to the **file-backed local dogfood host**. The per-account lock lives in
`run_generation_job`, so it is backend-agnostic: it fixes the file host and is
harmless belt-and-suspenders on Postgres.

## Residual

The lock guards `run_generation_job` only. A concurrent user mutation
(review/keep) on the file host can still race a background generation — a
narrower window (one user, rare overlap) than the worker's 4-way concurrency,
noted in the ticket as a follow-up.

## Review

Fresh-context thermo-nuclear review: **APPROVE, no blockers** (lock is
deadlock-free, `std::sync::Mutex` correct under `spawn_blocking`, residual
honestly framed, dropped "scope the approve" sub-task reasoning verified
correct). Two MINORs (test red-guarantee; env hermeticity) **rejected with
reason** — the test is a deterministic-green regression guard that caught the
real bug, and the structured-block format keeps it offline (verified with the key
set), so no `unsafe` env mutation is warranted.
