# Legacy Card 097 scheduled return reminders QA receipt

Date: 2026-07-15  
Branch: `cx/097-scheduled-reminders`  
Implementation commit: `a659af85d4c1e014b592a8f90c17bcbd62990da8`  
Base: `85db8aff01446dc6ab2a627376623848e3ea2b1e`  
Draft PR: [#57](https://github.com/misty-step/memory-engine/pull/57)

## Contract exercised

The scheduler implementation used an owned background task. It enumerated only
enabled, due, retry-ready accounts without an unexpired claim, claimed through
the file or Postgres adapter, and sent the persisted retry envelope with its
delivery key. A successful completion sampled the clock after the provider
returned; failures released with a post-provider retry timestamp. The file
outbox enforced the same delivery-key uniqueness while holding the
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
- Hosted GitHub CI [run 29443803062](https://github.com/misty-step/memory-engine/actions/runs/29443803062)
  — passed on `a659af8`; hosted Cerberus review [run
  29443803383](https://github.com/misty-step/memory-engine/actions/runs/29443803383)
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

This historical implementation evidence predates merge and deployment.
Production scheduled-reminder delivery was not captured. The 2026-08-16
grooming pass retired the reminder card rather than migrating it. GitHub issue
[#98](https://github.com/misty-step/scry/issues/98) owns the current,
narrower invite-link delivery proof; this receipt is not an active work item.
