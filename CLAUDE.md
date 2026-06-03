# memory-engine

Rust learning engine kernel and local dogfood workspace for spaced repetition,
grading, progression, queueing, source-backed generation, and beta study
workflows.

## What this repo is

- A Rust workspace. `Cargo.toml` owns the crate graph.
- `crates/memory-engine-core` is the pure framework-free learning kernel.
- `crates/memory-engine` is the consumer-facing facade.
- Boundary crates own service orchestration, persistence, generation, study
  sessions, local app hosts, dogfood receipts, benchmarks, and QA.
- `.dagger/src/index.ts` is the only TypeScript surface retained after the Rust
  cutover because it owns the Dagger CI module.

## Gate

`bun run ci` is the canonical gate. It shells out to
`dagger call check --source=.` and runs Rust formatting, workspace tests,
Clippy, rustdoc, and Gitleaks.

Use `bun run ci:local` / `bun run rust:ci` while iterating, but never hand off
without a green `bun run ci`.

## Invariants (load-bearing, do not violate)

1. **Core is pure.** No Convex, React, Hono, Node/Bun APIs, filesystem,
   networking, logging, auth, analytics, UI state, or model clients belong in
   `crates/memory-engine-core`.
2. **Consumer owns persistence.** Scheduler receives `ScheduleState` as
   argument and returns the next state. It never reads or writes storage.
3. **`ScheduleState` remains JSON-safe** (snake_case,
   `state: 0|1|2|3`, `last_review: number | null`). Consumers must not infer
   meaning from `ReviewUnitId`.
4. **Prompt enum ↔ grader dispatch co-evolve.** Adding a prompt variant requires
   exhaustive Rust match coverage and grader tests in the same change.
5. **Verdict vocabulary is fixed**: `correct`, `close`, `wrong`, `revealed`.
   Renames elsewhere (Ruminatio's `partial`, Caesar's SCREAMING case) map
   to these four; no new verdicts without a spec update.
6. **One grade call, one result.** `Grader::grade()` returns a `GradeResult`
   with `rating` already populated by the injected rating policy. No
   two-step verdict-then-rating protocol across the module boundary.
7. **Authority order:** tests > type system > code > docs > lore.

## Layout

- `crates/memory-engine-core` — pure domain, grading, scheduling,
  progression, queue, and rubric logic.
- `crates/memory-engine` — facade exports and testkit surface.
- `crates/memory-engine-service` — typed service command boundary.
- `crates/memory-engine-persistence` — local beta persistence.
- `crates/memory-engine-generation` — deterministic source-backed generation.
- `crates/memory-engine-study` — beta session/API boundary.
- `crates/memory-engine-beta-app` and `crates/memory-engine-web-shell` — local
  Rust HTTP dogfood hosts.
- `crates/memory-engine-cli`, `crates/memory-engine-import`,
  `crates/memory-engine-bench`, and `crates/memory-engine-qa` — receipts,
  import, benchmark, and QA tooling.
- `.dagger/` — CI pipeline (TypeScript SDK). Treat as owned code; changes
  require the same review as Rust runtime changes.
- `backlog.d/` — shaped tickets awaiting `/deliver`.
- `SPEC.md` / `docs/rust-migration.md` — authoritative on strategy and cutover
  state.

## Conventions

- **TDD default.** Red -> green -> refactor. Always write behavior tests before
  implementation for non-mechanical changes.
- **No internal mocks.** Exercise real repo-owned collaborators; mock only
  external boundaries such as network, clock, and model providers.
- **Style:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets
  -- -D warnings`.
- **Docs:** `cargo doc --workspace --no-deps` must pass.
- **No non-Dagger TypeScript runtime.** The QA crate has a regression test for
  this.

## Non-goals (this repo)

Production hosting, auth, billing, chat tutoring, generalized content import,
and extracting the beta app into another repository.
