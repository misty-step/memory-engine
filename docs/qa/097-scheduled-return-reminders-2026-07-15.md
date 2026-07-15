# Card 097 scheduled return reminders QA receipt

Date: 2026-07-15  
Branch: `cx/097-scheduled-reminders`  
Implementation commit: `b400ac47c429b48267f7c41289cc6c23d0bd6c20`  
Base: `85db8aff01446dc6ab2a627376623848e3ea2b1e`  
Draft PR: [#57](https://github.com/misty-step/memory-engine/pull/57)

## Contract exercised

The production scheduler is an owned background task. It enumerates only
enabled, due, retry-ready accounts without an unexpired claim, claims through
the file or Postgres adapter, and sends the signed card092 retry envelope with
the persisted delivery key. A successful completion samples the clock after
the provider returns; failures release with a post-provider retry timestamp.
The file outbox enforces the same delivery-key uniqueness while holding the
descriptor-owned nonblocking libc lock.

The operator-only manual endpoint is:

```text
POST /internal/scheduler/return-notifications
x-scheduler-token: $MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN
```

An absent or incorrect token returns `403`; an authorized request returns the
bounded report (`examined`, `sent`, `skipped`, `failed`). `GET` routes do not
start or mutate a send. `GET /healthz` returns `status: "ok"`,
`service: "memory-engine-api"`, and `returnNotificationScheduler` with
`enabled`, `running`, `lastRunAtMs`, `lastSuccessAtMs`, and `failureCount`.
`lastSuccessAtMs` advances only after a zero-failure sweep.

## Evidence

- `MEMORY_ENGINE_POSTGRES_TEST_URL=postgresql://127.0.0.1:55433/postgres cargo test -p memory-engine-persistence-postgres --lib`
  — 12 passed, including durable retry expiry, atomic claims, fencing, and
  live account persistence.
- `MEMORY_ENGINE_POSTGRES_TEST_URL=postgresql://127.0.0.1:55433/postgres cargo test -p memory-engine-api postgres_scheduler_retries_after_restart_and_contends_across_instances --lib`
  — 1 passed, 96 filtered; proves retry after a fresh state/store restart and
  one durable send across two independent scheduler instances.
- `bun run ci` — passed: format, workspace tests, clippy, and rustdoc.
- `bun run ci:full` — passed through the repo-owned Dagger pipeline: pinned
  workspace tests (including 97 API tests), clippy, rustdoc, and secrets scan.
- Focused API coverage includes pending-retry fairness, active-claim exclusion,
  malformed preference surfacing, exact capped backoff, slow-provider clock
  anchoring, health failure semantics, graceful scheduler shutdown, blocked
  sender versus responsive health, descriptor-lock owner safety, and
  lease-expiry file-outbox deduplication.

## Residuals

This is merge-ready but intentionally not merged or marked complete. Hosted
checks/review evidence will be appended after the pushed draft PR finishes.
Production scheduled-reminder evidence awaits deployment. Card092 remains open
for child056 real inbox deliverability; its merged/deployed proof and the
097 relation are recorded in Powder.
