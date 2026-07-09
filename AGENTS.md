# Agent Operations

`memory-engine` is a Rust human-learning engine and repo-local dogfood
workspace for spaced repetition, grading, progression, queueing,
source-backed generation, and beta study workflows — the Anki-killer product
direction described in `VISION.md`.

## Stack & Boundaries

- A Rust workspace. `Cargo.toml` owns the crate graph.
- `crates/memory-engine-core` is the pure framework-free learning kernel.
- `crates/memory-engine` is the consumer-facing facade.
- Boundary crates own service orchestration, persistence, generation, study
  sessions, local app hosts, dogfood receipts, benchmarks, and QA.
- `.dagger/src/index.ts` is the only TypeScript surface retained after the
  Rust cutover because it owns the Dagger CI module.

Runtime code in `crates/memory-engine-core` must stay framework-free and
persistence-free. No Convex, React, Hono, Node/Bun APIs, filesystem, network
calls, logging, auth, analytics, UI state, or vendor SDKs belong in the pure
kernel path. Boundary crates own storage, source ingestion, sessions, UI,
identity, analytics, and model clients until repeated proof justifies
promotion.

## Ground Truth

- `VISION.md` is the north star for the human learning product premise, Rust
  kernel boundary, dogfood product surface, and application/service boundary.
- `SPEC.md` is the older strategy document; `docs/rust-migration.md` records
  cutover state. Use them for technical history and boundary context;
  `VISION.md` governs when product positioning conflicts.
- `SLICE-*.md` files and `exemplars.md` are historical extraction context,
  not current delivery oracles.
- `backlog.d/` contains active work; `backlog.d/_done/` contains closed work.
- `.dagger/src/index.ts` owns CI behavior.
- `Cargo.toml` owns the Rust workspace; `crates/memory-engine` owns the
  consumer-facing Rust facade and module exports.
- `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` record executable QA
  and dogfood evidence.
- `docs/runbook.md` documents the DigitalOcean primary, temporary Fly standby,
  and production smoke contract for agents.
- Authority order overall: tests > type system > code > docs > lore.

## Gate Contract

`bun run ci` IS the default fast gate. It runs directly on the host through
Cargo: Rust formatting, workspace tests, Clippy, and rustdoc. It is the
pre-push and day-to-day agent loop.

`bun run ci:full` is the Dagger-backed full/ship parity gate. Keep it when the
containerized Postgres service, pinned Rust image, and Gitleaks scan matter.
Hosted CI calls this repo-owned script instead of raw Dagger.

`bun run ci:local` and `bun run rust:ci` remain compatibility aliases for the
fast gate while iterating. `bun run qa` is the full QA sweep and ends with
`bun run ci:full`, but it does not replace the fast gate. Delivery requires
`bun run ci`, `bun run ci:full` before handoff, and any ticket-named proof
oracle.

## Invariants

1. One ticket, one branch, one PR. Branch from `master`; use `cx/...` by
   default.
2. Do not implement feature work without an active shaped `backlog.d/`
   ticket.
3. TDD is the default — test behavior, not implementation; do not mock
   repo-owned pure collaborators (see Conventions for the full statement).
4. Core is pure: no Convex, React, Hono, Node/Bun APIs, filesystem,
   networking, logging, auth, analytics, UI state, or model clients belong in
   `crates/memory-engine-core` (see Stack & Boundaries for the exact
   exclusion list).
5. Consumer owns persistence. The scheduler receives `ScheduleState` as an
   argument and returns the next state; it never reads or writes storage.
6. `ScheduleState` is the JSON-safe scheduler card shape: snake_case,
   `state: 0 | 1 | 2 | 3`, `last_review: number | null`. `ReviewUnitId` is
   opaque — the kernel never inspects concept-vs-phrasing meaning, and
   consumers must not infer meaning from it.
7. Prompt enum changes and grader dispatch changes ship together with
   exhaustive Rust match coverage and grader tests in the same change.
8. `Grader::grade()` returns one `GradeResult` envelope with `rating` already
   populated by the injected rating policy — no two-step
   verdict-then-rating protocol across the module boundary.
9. Verdicts remain `correct`, `close`, `wrong`, `revealed`. Renames elsewhere
   (Ruminatio's `partial`, Caesar's SCREAMING case) map to these four; no new
   verdicts without a spec update.
10. Do not add runtime dependencies without shaped scope and docs updates.
11. Do not lower gates, bypass hooks, or mark unrun canaries as proof.
12. Historical Scry and Vault canary branches are deprecated. Use repo-local
    dogfood lanes and explicitly shaped current external proof instead.

## Layout

- `crates/memory-engine-core` — pure domain, grading, scheduling,
  progression, queue, and rubric logic.
- `crates/memory-engine` — facade exports and testkit surface.
- `crates/memory-engine-service` — typed service command boundary.
- `crates/memory-engine-persistence` — local beta persistence.
- `crates/memory-engine-generation` — source-backed generation behind a
  `DraftProvider` boundary, with deterministic structured-block and fake
  providers and the provenance trust gate.
- `crates/memory-engine-openrouter` — OpenRouter-dialect HTTP draft provider
  (model-backed generation); the only crate that talks to a model network.
- `crates/memory-engine-study` — beta session/API boundary.
- `crates/memory-engine-api` — production-facing HTTP route registration, request
  handlers, static assets, and binary entrypoint.
- `crates/memory-engine-api-state` — API account/session state, auth, storage
  adapters, and background generation jobs.
- `crates/memory-engine-api-render` — server-rendered study UI and design
  preview conformance fixtures.
- `crates/memory-engine-beta-app` and `crates/memory-engine-web-shell` —
  local Rust HTTP dogfood hosts.
- `crates/memory-engine-cli`, `crates/memory-engine-import`,
  `crates/memory-engine-bench`, and `crates/memory-engine-qa` — receipts,
  import, benchmark, and QA tooling.
- `.dagger/` — CI pipeline (TypeScript SDK). Treat as owned code; changes
  require the same review as Rust runtime changes.
- `backlog.d/` — shaped tickets awaiting `/deliver`.
- `SPEC.md` / `docs/rust-migration.md` — authoritative on strategy and
  cutover state.

## Conventions

- **TDD default.** Red -> green -> refactor. Always write behavior tests
  before implementation for non-mechanical changes. Test behavior, not
  implementation.
- **No internal mocks.** Exercise real repo-owned collaborators; mock only
  external boundaries such as network, clock, and model providers.
- **Style:** `cargo fmt --all --check`; `cargo clippy --workspace
  --all-targets -- -D warnings`.
- **Docs:** `cargo doc --workspace --no-deps` must pass.
- **No non-Dagger TypeScript runtime.** The QA crate has a regression test
  for this (see Known Debt for cutover completion status).

## Backlog Lifecycle

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`.
Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or
`Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using
`backlog_archive`.

`/deliver` stops at merge-ready. `/ship` archives tickets, preserves closure
trailers, merges, verifies archive state, writes final trace when available,
and invokes bounded `/reflect`. `/groom` always reconciles tracker truth
before strategy.

## Known Debt

- Keep the Rust cutover complete: no non-Dagger TypeScript runtime/test files
  should return, and operator docs must point at Rust crates, Cargo
  commands, Dagger CI, and the production runbook.
- After cutover, prioritize repeated phone-sized dogfood receipts over new
  abstractions. Beta app extraction or promotion needs repeated evidence
  from the Rust app, not archived TypeScript-era tickets.
- The largest current simplification pressure is in boundary crates,
  especially local HTTP hosts and persistence. Do not move that complexity
  into `memory-engine-core`.

## Non-goals

General-purpose hosting or auth frameworks, billing, chat tutoring, generalized
content import, and extracting the beta app into another repository.
