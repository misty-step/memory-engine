# Harden and collapse the production boundary

Priority: P0 · Status: pending · Estimate: XL

## Goal

Make the production learning loop deploy only proven code, survive restarts and
partial failure without losing generation work, protect learner credentials and
source privacy, and remove duplicate boundary surfaces after their behavior is
pinned.

## Oracle

- [ ] One canonical production plane is declared. The DigitalOcean primary
      deploys an immutable CI-cleared SHA, runs dependency-aware authenticated
      smoke, and has a tested prior-artifact rollback; the Fly standby is either
      a proven failover target or decommissioned.
- [ ] Generation jobs use Postgres leases/outbox state with bounded retries,
      restart recovery, idempotent completion, and a multi-instance oracle; no
      production job/history truth lives only in process memory.
- [ ] Postgres access uses a bounded connection pool and explicit versioned
      migrations with rehearsal and rollback evidence instead of fresh blocking
      connects plus first-use DDL.
- [ ] API/browser credentials are hashed at rest, expire, and can be rotated or
      reissued without raw-token SQL recovery instructions.
- [ ] Source permission is enforced before model egress; local-only material
      cannot reach OpenRouter, and the user-facing flow discloses the boundary.
- [ ] Query-shaped persistence keeps next-review and write paths bounded by the
      working set; fixtures demonstrate acceptable behavior at 10k and 100k
      concepts without whole-account snapshots.
- [ ] Superseded local hosts, file-store production wiring, facade dependencies,
      and duplicate pass-through interfaces are deleted only after live-diff or
      consumer proof pins their surviving behavior.
- [ ] The runbook carries DO-native logs, secrets, deploy, rollback, Neon
      backup/restore, RPO/RTO, and incident evidence; `bun run ci` and
      `bun run ci:full` stay green throughout.

## Verification System

- Claim: production is operationally boring and the application boundary is
  smaller after hardening.
- Falsifier: a red SHA begins replacing primary, a restart loses a queued job,
  a read-only database leak yields a usable bearer token, local-only content
  reaches a provider, or due-review latency grows with total account history.
- Driver: deployment rehearsal, restart/multi-instance fault injection,
  migration/restore drill, credential and source-permission request replay,
  account-size benchmark, and before/after consumer builds.
- Grader: exact post-state and latency/rollback assertions plus artifact-only
  security and architecture critics at each child milestone.
- Evidence packet: dated operational receipts under `docs/qa/`, migration and
  restore logs, benchmark raw output, live-diff results, and deletion ledger.
- Cadence: one independently shippable child per PR; live oracle + gates + fresh
  critic before advancing.

## Children

1. Canonical deploy plane, CI-gated DO primary, meaningful readiness, rollback.
2. Durable Postgres generation-job leases, retry, restart, multi-instance proof.
3. Connection pool and versioned migration runner.
4. Hashed/expiring/rotatable credentials.
5. Enforced source privacy and model-egress disclosure.
6. Query-shaped persistence and account-size performance proof.
7. Delete superseded hosts, file-production wiring, facade reach, and Fly
   standby after their replacement or removal oracles pass.

## Notes

This absorbs the deploy-shaped portion of stale epic 066 but deliberately does
not absorb its learner-experience work; 073 owns that outcome. It also names
the process-memory production job gap that the archived 057 ticket explicitly
left behind.
