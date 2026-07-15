# Card 097 scheduled return reminders QA receipt

Date: 2026-07-15  
Branch: `cx/097-scheduled-reminders`  
Implementation commit: `e8a7e521c720906c6d61010eae8f12e638db8a21`  
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
- Hosted GitHub CI [run 29442439565](https://github.com/misty-step/memory-engine/actions/runs/29442439565)
  — passed on `b23e013`; hosted Cerberus review [run
  29442440681](https://github.com/misty-step/memory-engine/actions/runs/29442440681)
  — passed. CodeRabbit reported an explicit draft skip, not a review result.
- Focused API coverage includes pending-retry fairness, active-claim exclusion,
  malformed preference surfacing, exact capped backoff, slow-provider clock
  anchoring, health failure semantics, graceful scheduler shutdown, blocked
  sender versus responsive health, descriptor-lock owner safety, and
  lease-expiry file-outbox deduplication.
- Same-load branch/base A/B for the one observed route-retry failure:
  `e8a7e52` branch `cargo test -p memory-engine-api --lib` — 96 passed, 1
  ignored; clean base `85db8aff` with the identical command — 86 passed, 1
  ignored. Neither reproduced the failure, so no branch-specific delta was
  observed under this oracle; the exact initial failure remains retained in
  the run log and was not treated as acceptance by a lucky rerun.

## Residuals

This is merge-ready but intentionally not merged or marked complete. The final
receipt-doc synchronization will produce one more hosted CI run. Production
scheduled-reminder evidence awaits deployment. Card092 remains open
for child056 real inbox deliverability; its merged/deployed proof and the
097 relation are recorded in Powder.
