# Memory Engine Vision

Status: Canonical root vision for the Rust memory-engine workspace. Revise when
the reusable kernel boundary, dogfood product surface, or extraction strategy
materially changes.

## What Memory Engine Is

Memory Engine is a Rust learning-engine workspace for spaced repetition,
answer grading, source ingestion, content generation, study sessions, dogfood
clients, QA, and benchmark loops. It is the durable memory product boundary
after older TypeScript-era and app-specific surfaces have been retired or
demoted to evidence.

It serves operators and agents building reliable learning workflows where
generated study content, scheduling state, grading verdicts, source evidence,
and production receipts must stay explainable.

## North Star

A dogfood-proven memory engine: high-quality generation and review loops backed
by a framework-free core, explicit boundary crates, reproducible benches,
production smoke, and persistence owned by consuming layers.

## What Must Stay True

- `crates/memory-engine-core` stays framework-free and persistence-free: no
  Convex, React, Hono, Node/Bun APIs, filesystem, network, logging, auth,
  analytics, UI state, or vendor SDKs.
- Rust crates own durable domain and service boundaries. Boundary crates own
  storage, source ingestion, sessions, UI, identity, analytics, and model
  clients until repeated proof justifies promotion.
- Consumers own persistence. Modules exchange explicit state, verdicts, and
  envelopes rather than hiding database writes.
- Generation quality, latency, coverage, and statistical rigor are product
  concerns, not optional benchmark decorations.
- Dogfood receipts matter more than architectural appetite. Beta app extraction
  or reusable API promotion needs repeated evidence from the Rust app.
- Historical Scry and Vault material is boundary evidence, not current product
  direction.

## What Memory Engine Refuses

- Reviving decommissioned Scry/Vault surfaces as the primary direction.
- Unstable library extraction before dogfood evidence.
- Runtime dependencies in the pure kernel.
- Green aggregate tests without ticket-specific proof, live QA, bench evidence,
  or production smoke where the change calls for it.
- Prompt or grader enum drift without exhaustive Rust match coverage.

## Current Bets

1. Improve content-fit, coverage, latency, and post-answer feedback loops until
   generation is useful in real study.
2. Keep magic-link and study-session behavior reliable enough for production
   dogfood.
3. Use the Fly-hosted `memory-engine-api` as the living proof surface.
4. Preserve extraction packets as history while letting current dogfood decide
   what becomes reusable.
5. Keep `bun run ci` as the canonical gate and add ticket-specific QA when the
   aggregate gate cannot prove the behavior.

## Where The Depth Lives

- `AGENTS.md` is the repo operating contract and kernel boundary map.
- `README.md` explains the Rust workspace, status, usage, and current docs.
- `SPEC.md` is the strategy document.
- `docs/runbook.md` is the production Fly/API runbook and smoke contract.
- `docs/qa/system.md`, `docs/qa/quality-register.md`, `docs/dogfood/`, and
  `docs/beta/` hold executable QA and dogfood evidence.
- `backlog.d/` is the active shaped-work queue; `backlog.d/_done/` is closed
  history.
- `bun run ci` shells to Dagger and is the closeout gate.
