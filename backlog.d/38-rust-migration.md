---
id: 38
title: Rust migration
status: ready
priority: high
created: 2026-05-28
---

# Rust Migration

## Context

`memory-engine` is still a Bun/TypeScript kernel plus repo-local dogfood
experiments. The target state is a Rust application and Rust library substrate
with the same product behavior, stronger type boundaries, and parity evidence
against the current TypeScript implementation until cutover.

The migration must not flatten the design into shallow wrappers. Rust modules
should hide scheduling, grading, progression, queue, persistence, and interface
details behind deep APIs that carry the domain semantics.

## Scope

- Add a Rust workspace beside the existing TypeScript code.
- Port the pure kernel first: domain types, deterministic grading,
  progression eligibility, queue selection, and scheduler semantics.
- Keep TypeScript tests and experiments green as the migration oracle until a
  Rust replacement covers the same behavior.
- Migrate service, persistence, beta generation, beta study UI/API, CLI
  experiments, and package exports in later slices.
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
- Rust parity docs identify what is migrated, what is still TypeScript-owned,
  and the cutover evidence required before deleting TypeScript code.
- The beta-study app remains locally runnable during the migration.

## Migration Slices

1. Rust core crate with domain, grading, progression, and queue parity.
2. Rust scheduler wrapper with fixture parity against current `ts-fsrs` output.
3. Rust service crate with typed command envelope and storage trait.
4. Rust persistence crate for the beta store.
5. Rust beta generation and study API, keeping deterministic fixtures first.
6. Rust web/app delivery path or extracted app host.
7. TypeScript removal after parity gates and dogfood receipt pass.
