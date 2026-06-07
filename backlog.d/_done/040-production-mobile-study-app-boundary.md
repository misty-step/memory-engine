# 040 - Production Mobile Study App Boundary

## Status

Active

## Context

The Rust beta app has local mobile study proof, but production app requirements
now include account creation, generated study material, scheduled review, and
deployment to production infrastructure. The current repo has no active ticket
for that production boundary.

This ticket implements the first production-aligned slice from
`docs/architecture/production-mobile-study-app.md` and
`docs/architecture/adr-001-production-shell-boundary.md`.

## Goal

Create the production boundary for a simple mobile study app: account-scoped
source intake, generated study draft approval, scheduled review, and deployable
Rust service infrastructure.

## Product Requirements

- P0: A learner can create/save an account and resume account-scoped study
  state.
- P0: A learner can submit source material, generate cited study drafts, keep
  or edit drafts, and review kept material.
- P0: Review scheduling uses existing `memory-engine-service` grade/apply-review
  semantics.
- P0: Reveal remains display-only and does not mutate attempts, reps, or
  schedules.
- P0: Duplicate submit is idempotent and cannot double-count reps.
- P0: Production state uses managed durable storage, not the current single JSON
  file as source of truth.
- P0: Production deployment targets Fly.io Machines with managed Postgres as
  the selected architecture, while Railway and Render remain fallback options.
- P1: Mobile UX defers sign-up until the first useful study set exists unless
  the learner chooses to sign in first.

## Technical Requirements

- Add a Rust API boundary outside `memory-engine-core`.
- Add or shape a production persistence adapter with account/user scoping.
- Preserve deterministic generation mode and add the provider adapter seam
  without hardcoding vendor behavior into core/service crates.
- Add deployment configuration and a staging smoke path.
- Update QA evidence so product proof is not confused with package QA.

## Non-Goals

- Native app store distribution.
- Billing.
- Multi-region active-active writes.
- General tutoring chat.
- Moving auth, SQL, HTTP, provider clients, or UI state into
  `memory-engine-core`.
- Extracting the app into a separate repository.

## Acceptance Oracle

- [x] `docs/architecture/adr-001-production-shell-boundary.md` records the
  platform/database/API boundary decision.
- [x] Production API routes are covered by behavior tests that do not mock
  repo-owned service/session/persistence collaborators.
- [x] Account/user isolation test proves one account cannot read or mutate
  another account's source, draft, review, or schedule state.
- [x] Review idempotency test proves duplicate submit returns/rejects without
  double-counting attempts, reps, or schedule writes.
- [x] Generation tests prove source-backed drafts include citations, unsupported
  source fails visibly, and deterministic mode remains available.
- [x] Mobile browser smoke proves first source capture, generation, approval,
  account save, review, reveal, submit, next, restart/resume, and no horizontal
  overflow at 390 x 844.
- [x] Deployment smoke proves `/healthz` and one account-scoped source-to-review
  path on staging infrastructure.
- [x] `cargo run -p memory-engine-qa -- --local` passes.
- [x] `bun run ci` passes.

## Suggested Sequence

1. Add `/healthz` and deployment-shape scaffolding only after the API boundary
   is selected.
2. Add account-scoped API route tests first.
3. Add Postgres or production-store contract tests before wiring deployment.
4. Add generation provider adapter tests with deterministic mode preserved.
5. Add mobile render/smoke evidence.
6. Add staging deploy config and smoke receipt.

## Evidence Paths

- `docs/architecture/production-mobile-study-app.md`
- `docs/architecture/adr-001-production-shell-boundary.md`
- `docs/qa/system.md`
- `docs/qa/fly-staging.md`

## Closure

Use `Closes-backlog: 040` when this ticket is fully implemented, verified, and
ready for merge. Production credential setup or real deploy secrets that cannot
live in repo must be recorded as residual risk with exact missing state.
