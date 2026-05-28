---
id: 38
title: Rust migration
status: ready
priority: high
created: 2026-05-28
---

# Rust Migration

## Context

`memory-engine` is now a Rust kernel, service, persistence, generation, study,
and local app workspace. The target state is a Rust application and Rust
library substrate with the same product behavior, stronger type boundaries, and
parity evidence preserved from the former TypeScript implementation.

The migration must not flatten the design into shallow wrappers. Rust modules
should hide scheduling, grading, progression, queue, persistence, and interface
details behind deep APIs that carry the domain semantics.

## Scope

- Add a Rust workspace and cut the runtime over from the former TypeScript code.
- Port the pure kernel first: domain types, deterministic grading,
  progression eligibility, queue selection, and scheduler semantics.
- Keep TypeScript tests and experiments green as the migration oracle until a
  Rust replacement covers the same behavior, then delete them.
- Migrate service, persistence, beta generation, beta study UI/API, CLI
  experiments, QA, benchmarks, and package exports.
- Remove the TypeScript runtime only after Rust has executable parity for the
  published API and the beta-study application.

## Non-Goals

- No partial rewrite that changes learning semantics.
- No storage, network, UI, or model-client code in the Rust core crate.
- No runtime dependency additions without a Rust migration need and docs.
- No shallow compatibility shell that preserves TypeScript as the real engine.

## Design Constraints

- The Rust core stays pure and framework-free.
- `ReviewUnitId` remains opaque.
- Scheduling state stays JSON-safe and compatible with the current
  `ScheduleState` contract until the scheduler cutover deliberately versions it.
- Prompt union and grader dispatch changes move together.
- Queue and progression helpers accept caller policy where mastery differs by
  consumer.
- Service and persistence boundaries must use typed command/result enums rather
  than stringly request dispatch.

## Acceptance

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`
- `bun run ci:local`
- `bun run ci`
- Rust parity docs identify what was migrated, what was deleted, and the
  hardening evidence required after cutover.
- The beta-study app remains locally runnable during the migration.

## Migration Slices

1. Rust core crate with domain, grading, progression, and queue parity.
2. Rust scheduler boundary with fixture parity against current `ts-fsrs` output.
3. Rust service crate with typed command envelope and storage trait.
4. Rust persistence crate for the beta store.
5. Rust beta generation and study API, keeping deterministic fixtures first.
6. Rust web/app delivery path or extracted app host.
7. TypeScript removal after parity gates and dogfood receipt pass.

## Progress

- 2026-05-28: Added `memory-engine-core` for pure domain, grading,
  progression, queue, and scheduler semantics with Rust and TypeScript parity
  tests.
- 2026-05-28: Added `memory-engine-service` with typed command/result
  envelopes, store trait, deep grade/apply-review orchestration, and service
  contract tests. Persistence and beta app cutover remain future slices.
- 2026-05-28: Added `memory-engine-persistence` with a file-backed beta
  store, typed validation and conflict errors, atomic commit behavior, queue
  projection, and service-store integration tests. Beta generation and app
  server cutover remain future slices.
- 2026-05-28: Added `memory-engine-generation` with deterministic source-block
  parsing, generated prompt drafts, generation-run bookkeeping, and accepted /
  rejected draft parity tests. Also tightened core serde for TypeScript-shaped
  prompt, grade, rating, schedule, progression, queue, and beta-store wire
  contracts. Beta-study session/server cutover remains a future slice.
- 2026-05-28: Added `memory-engine-study` with Rust beta-study session/API
  orchestration for source intake, deterministic generation, draft approval,
  reveal, grade/apply-review, queue advancement, resume, duplicate-submit
  idempotence, and mobile API JSON shape. HTTP server and web UI cutover remain
  future slices.
- 2026-05-28: Added `memory-engine-beta-app`, a Rust HTTP host for the local
  beta-study app. It serves the existing mobile HTML and ports `/state`,
  `/source`, `/generate`, `/approve`, `/reveal`, `/answer`, and `/next` onto
  the Rust study session with route tests. Shared HTML and final TypeScript
  package/API removal remain future slices.
- 2026-05-28: Added `memory-engine-cli` with a Rust CLI review dogfood client,
  in-memory service-store fixture, calibration receipt output, and boundary
  tests. The TypeScript CLI experiment remains only as a migration oracle.
- 2026-05-28: Added `memory-engine-import` with a Rust authored-content import
  probe, canonical prompt / queue / schedule compilation, service-loop receipt,
  and boundary tests. The TypeScript import experiment was later deleted after
  Rust parity landed.
- 2026-05-28: Added `memory-engine-web-shell` with Rust learner-facing session
  choreography, compact view DTOs, reveal handling, web-shell receipt output,
  and HTTP route tests over the Rust service boundary. The TypeScript web-shell
  experiment was later deleted after Rust parity landed.
- 2026-05-28: Added the consumer-facing `memory-engine` Rust facade crate,
  with root and modular API tests that mirror the former TypeScript package
  export ergonomics while preserving deep ownership in the existing Rust crates.
  The TypeScript package exports were later deleted after Rust facade parity
  landed.
- 2026-05-28: Added `fixtures/service-command-scenarios.json` and replay tests
  for service record-attempt, grade/apply-review, next-queue, and
  progression-unlock scenarios. Rust `NextQueueOptions` now deserializes
  missing option fields through explicit defaults to match the former
  TypeScript command ergonomics.
- 2026-05-28: Added the Rust facade `testkit` module with grading, recitation,
  scheduler, progression, and queue fixtures replayed through the public Rust
  surfaces. The TypeScript `memory-engine/testkit` surface now has a cutover
  counterpart instead of remaining a TypeScript-only consumer contract.
- 2026-05-28: Added Rust rubric grading and adapter parity. The pure core now
  owns rubric prompt/assessment types, the async-grader facade, no-adapter
  failure semantics, blank-answer handling, criterion/result normalization, and
  a static adapter double; the consumer facade exposes matching root,
  `grading`, `types`, and `adapters` paths without adding model-client or
  runtime dependencies to the kernel.
- 2026-05-28: Replaced the TypeScript benchmark script with
  `memory-engine-bench`, a Rust receipt generator that exercises facade
  grading, scheduler advancement, queue selection, and service composition.
  `bun run bench` now runs the Rust crate while remaining non-gating until
  historical performance budgets exist.
- 2026-05-28: Replaced the TypeScript QA runner with `memory-engine-qa`, a Rust
  receipt runner that preserves local/full lane selection, command receipts,
  gating versus warning behavior, and the canonical Dagger lane.
- 2026-05-28: Retired the TypeScript/Bun coverage gate after the oracle tests
  were deleted; Rust workspace tests, Clippy, rustdoc, QA, and Dagger are now
  the executable confidence path.
- 2026-05-28: Deleted the TypeScript CLI review, import probe, and web-shell
  dogfood oracles after their Rust crates owned receipt, service-loop, session,
  and HTTP route parity. The web-shell HTML asset remains shared by the Rust
  host.
- 2026-05-28: Deleted the TypeScript beta-study session and server oracle after
  `memory-engine-study` and `memory-engine-beta-app` owned source, generation,
  approval, reveal, review, restart/resume, duplicate-submit, and HTTP route
  parity. The mobile HTML asset remains shared by the Rust host.
- 2026-05-28: Deleted the TypeScript beta-generation oracle after
  `memory-engine-generation` owned source-block parsing, provenance/reference
  persistence, accepted/rejected draft validation, promotion, and source-error
  fixture parity.
- 2026-05-28: Deleted the TypeScript beta-store oracle after
  `memory-engine-persistence` owned durable snapshot IO, atomic commit,
  duplicate/stale review protection, queue projection, generated-draft
  validation, and camelCase wire-shape parity.
- 2026-05-28: Deleted the TypeScript service prototype and `tests/service`
  oracle after `memory-engine-service` owned typed command/result envelopes,
  shared service fixtures, store failure propagation, queue selection, and JSON
  kind-tag parity.
- 2026-05-28: Deleted the remaining TypeScript package facade, testkit, and Bun
  oracle tests after the Rust facade, core, testkit, service, persistence,
  generation, study, app, dogfood, benchmark, QA, and Dagger gates covered the
  migration surfaces. Root package scripts now dispatch to Rust commands.
