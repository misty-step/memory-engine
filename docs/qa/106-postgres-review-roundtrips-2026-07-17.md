# 106 — Postgres review-loop round-trip receipt

Ticket: memory-engine-106. Follow-up to the account snapshot diagnosis in
`docs/qa/082-latency-investigation-2026-07-09.md` and the request-scoped cache
in memory-engine-085. The user-visible failure was a five-to-ten-second pause
for both starting the next quiz and submitting an answer on the DigitalOcean
production app.

## Root cause

The request-scoped cache removed repeated snapshots, but each remaining
Postgres snapshot still performed ten sequential table reads. Queue selection
then read each review unit's schedule separately, and active-review rendering
opened another connection for job history that ordinary next/submit responses
do not use. The production path pays network latency for every one of those
round trips.

## Change

- `AccountStudyStore::snapshot` reads all ten ordered account collections with
  one account-scoped `UNION ALL` statement. It returns one JSON record per row,
  rather than aggregating a whole account table into one `jsonb` container, so
  it does not introduce PostgreSQL's per-container size ceiling.
- `list_queue_candidates` reads review units and optional schedules with one
  account-scoped `LEFT JOIN` instead of one schedule query per review unit.
- Active-review rendering skips source and job-history reads unless a
  `Generating…` notice needs the memory-engine-081 live-job truth check.
  Workspace rendering and non-generating notices retain their prior behavior.

The snapshot query preserves every former per-table ordering key. Attempts use
a fixed-width representation of their positive `BIGSERIAL` id only as the
shared union sort key, preserving numeric tie order when multiple attempts have
the same millisecond timestamp.

## Exact route receipt

The ignored latency fixture drives the real Rust HTTP routes and a live
Postgres 17 instance with `log_statement=all`: it creates a browser session,
generates and approves a two-card source, calls `/app/next`, then submits the
correct answer to `/app/submit`. The response contract requires HTTP 200 and a
rendered `correct` verdict.

```sh
cargo test -p memory-engine-api \
  postgres_review_actions_emit_latency_receipt -- --ignored --nocapture
```

The same route fixture and source were used before and after the change. Counts
below are prepared statements on each route's Postgres connection; the submit
transaction row also shows all commands including transaction boundaries.

| Route | Before | After | Reduction |
|---|---:|---:|---:|
| `/app/next` | 38 statements | 5 statements | 86.8% |
| `/app/submit` | 60 statements | 18 statements | 70.0% |
| `/app/submit`, including transaction boundaries | 70 commands | 20 commands | 71.4% |

Final loopback timing: `/app/next` **57.485 ms** and `/app/submit` **87.618
ms**. These timings isolate application work from production TLS and network
latency; the statement counts are the durable performance contract. The
remaining submit reads and writes belong to grading, schedule-state validation,
idempotency, and the atomic apply transaction.

## Behavioral proof

- The live Postgres account/snapshot/idempotent-review contract passes with all
  ten snapshot collections, account isolation, queue state, and applied-review
  receipts intact.
- That contract inserts eleven extra attempts at one timestamp, crossing the
  `BIGSERIAL` 9-to-10 boundary, and verifies snapshot order remains numeric.
- Render loader coverage pins ordinary active review, `Generating…`,
  non-generating notice, and workspace branches.
- Fresh-context review found and drove fixes for the initial `jsonb_agg` size
  ceiling, duplicated notice predicate, and lexicographic attempt tie order.

Verification commands:

```sh
cargo test -p memory-engine-persistence-postgres \
  live_postgres_store_scopes_accounts_and_persists_idempotent_reviews -- --nocapture
cargo test -p memory-engine-api-render \
  active_review_loads_only_data_used_by_the_rendered_branch -- --nocapture
bun run ci
bun run ci:full
```

The model-backed generation brain was not exercised; generation behavior did
not change. The fixture uses deterministic generated material so it can isolate
the Postgres review loop.