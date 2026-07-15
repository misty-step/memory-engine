# QA receipt — durable production generation jobs (093)

## Contract

Production generation uses `memory_engine_generation_jobs` in Postgres. The
versioned migration ledger applies the existing account schema as migration 1,
the job/lease schema as migration 2, additive lease-token/reservation
compatibility as migration 3, and the per-attempt receipt ledger as migration
4. Queue admission is atomic under a Postgres advisory transaction lock: eight
active jobs per account, 64 active jobs globally, three attempts, four running
jobs globally, one running job per account, and 100,000 micro-USD per
account/model over 24 hours. Each attempt carries a durable reservation and an
exact `(job_id, attempt, lease_token, generation_run_id)` usage receipt. Source
bodies are limited to 256 KiB and titles to 512 bytes.

## Automated restart and partial-outage oracle

The production-shaped test creates an isolated Postgres schema, enqueues a
job, reads it through a fresh store instance, claims it, rejects renewal and
finish after lease expiry, reclaims it with another worker, records a retry,
recreates the next reservation atomically, reclaims the retry after its
backoff, and completes it. It also proves same-account/source coalescing,
per-attempt receipt recovery after provider completion before job finish,
deployed-v2 migration row preservation, exact-run cost lookup, and rejection
of spent 75 plus a proposed reservation 50 against a budget of 100:

```sh
MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test?sslmode=disable \
  cargo test -p memory-engine-persistence-postgres \
  live_generation_jobs_are_durable_leased_and_bounded -- --nocapture
```

The API-state idempotency oracle runs the same generation job twice through the
real file-backed study adapter. The first pass schedules material; the replay
returns zero new scheduled cards, proving a worker restart cannot duplicate
material after an interrupted approval loop:

```sh
cargo test -p memory-engine-api-state \
  rerunning_a_generation_job_is_idempotent_after_an_interrupted_schedule -- --nocapture
```

Readiness is distinct from process liveness:

```sh
curl -i http://127.0.0.1:3000/healthz  # liveness
curl -i http://127.0.0.1:3000/readyz   # Postgres + worker readiness
```

`/readyz` returns `503` when the worker has not started or Postgres cannot be
probed; `/healthz` remains the process liveness contract.

## Local run note

The complete live persistence proof was rerun against a fresh disposable
`pgvector/pgvector:pg15` container on `127.0.0.1:55434`. The target role and
database were verified with `psql` before running:

```sh
MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://memory_engine:<disposable-password>@127.0.0.1:55434/memory_engine \
  cargo test -p memory-engine-persistence-postgres --lib -- --nocapture
```

Result: **16 passed, 0 failed**. This includes
`live_generation_jobs_are_durable_leased_and_bounded`, which proves fresh-store
restart readback, expired-owner renewal/finish fencing, stale reclaim, durable
attempt reservation and usage recovery, retry backoff and reservation
recreation, retry completion, exact generation-run cost lookup, and the 75 +
50 > 100 admission boundary. The same run passed the deployed-v2 upgrade
proof with the pre-v2 job row preserved, all live return-notification and rate
limit Postgres contracts, the migration contract, and both generation-job SQL
typing contracts.

The API restart-level proof also passed against that disposable URL:

```sh
MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://memory_engine:<disposable-password>@127.0.0.1:55434/memory_engine \
  cargo test -p memory-engine-api --lib \
  postgres_backend_browser_session_resumes_after_restart -- --nocapture
```

Result: **1 passed, 0 failed**. A fresh `ApiState` and worker queue resumed
against the same Postgres store and completed the durable generation after
process construction. Hosted full/review URLs and the final amended SHA will
be added after this coherent repair head is pushed; no earlier cfe7bcd or
docs-only result is acceptance evidence for this repair.

## Not covered

This receipt does not deploy or restart DigitalOcean production. The production
deployment smoke remains the runbook health/home/anonymous-mutation contract;
controlled production restart proof requires an operator-approved deployment
window after merge.
