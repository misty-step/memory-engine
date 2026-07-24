# ADR-001: Production Shell Boundary

Status: superseded by the 2026-07-08 DigitalOcean cutover; retained as a
historical decision record
Date: 2026-06-06
Refs-Powder: memory-engine-040

Current runtime authority is `docs/runbook.md`. The provider-specific decision
and commands below describe the original deployment only and are not an active
rollback or recreation path.

## Context

The Rust beta app proves local source intake, deterministic generation, draft
approval, reveal, grading, schedule mutation, duplicate-submit handling, and
restart/resume. It does not provide account creation, account-scoped
production storage, provider-backed generation, or cloud deployment proof.

The kernel contract is explicit: `crates/memory-engine-core` owns learning
semantics only. Auth, identity, persistence, provider clients, HTTP, UI, and
deployment belong outside the pure kernel.

## Decision

Build the production mobile study app as a cohesive Rust HTTP service deployed
on Fly.io Machines, backed by managed Postgres, with auth/account scoping at
the HTTP boundary and production storage behind a dedicated Postgres adapter.

The first production service boundary should be a new crate, tentatively
`memory-engine-api`, that composes existing service, generation, and
persistence contracts without moving production concerns into
`memory-engine-core`.

## Accepted Architecture

- Runtime: long-running Rust HTTP service.
- Deployment: Fly.io Machines with Docker/Fly config in repo.
- Storage: managed Postgres for account-backed source, generation, review, and
  schedule state.
- Auth: account/session boundary in the API shell; all mutable state is scoped
  before invoking study/service operations.
- Generation: provider adapter behind `memory-engine-generation`, preserving
  deterministic fixture mode for tests and dogfood receipts.
- Mobile UX: phone-first web app or server-rendered shell before native app.

## Rejected Alternatives

- Vercel as backend: Rust Functions are a poor fit for one deep service
  boundary and durable scheduled-review state. Vercel remains possible for
  future static/marketing surfaces only.
- Fly volume plus JSON store as production: useful for dogfood smoke but not
  account-backed production state.
- Native mobile first: premature until backend/account/generation/schedule
  proof exists.
- VPS first: portable but too much early operational burden.

## Consequences

- New production crates may use HTTP, async, database, auth, logging, and model
  client dependencies.
- `memory-engine-core` must remain free of those dependencies.
- Dagger should remain the full/ship parity CI gate and later gain an explicit
  staging deploy/smoke lane.
- Product proof must include rendered/mobile and deployed-route evidence, not
  only package tests.

## Verification

- Focused tests for account scoping, idempotent review submission, provider
  generation receipts, and Postgres compare-and-apply.
- Mobile browser smoke at 390 x 844.
- Staging deployment smoke against `/healthz` and one account-scoped
  source-to-review path.
- Fast `bun run ci` plus full `bun run ci:full`.

2026-06-06 staging verification deployed `memory-engine-api` to Fly Machines
with Fly Managed Postgres cluster `memory-engine-api-pg` in `ord`, using
`MEMORY_ENGINE_POSTGRES_URL` as the app secret. The deployed service passed
`/healthz`, JSON account/source/generate/keep/reveal/submit, Machine
restart/resume, and mobile 390 x 844 browser smoke.
