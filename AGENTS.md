# Agent Operations

Scry is the consumer product. The Rust workspace is Scry's engine and current
production workspace. The canonical product promise is **Remember everything.**
Quiz-driven memorization is the heart of the product; the five faces are PWA, CLI,
skill, MCP, and API over one capability system. See `VISION.md` for the full
product contract.

## Product Topology

- The phone-first PWA is the primary human surface.
- CLI, skill, MCP, and API are additional faces over the same typed capabilities;
  they must not grow separate learning or grading semantics.
- Human beta access is invite-gated with a visible waitlist and magic-link sign-in
  only; there is no OAuth path. Machine faces use operator-gated service sessions.
  Subscription is intended; public signup waits for bounded costs, privacy,
  reliability, and Stripe proof.
- The current production proof surface is the native Rust `memory-engine-api`
  process on Misty Step's isolated DigitalOcean public application host,
  backed by Neon Postgres and served at `https://scry.study`.
- Crate names, Postgres identifiers, wire and telemetry literals, and
  `MEMORY_ENGINE_*` environment variables deliberately retain the old name.
  Do not rename a storage, network, deployment, or compatibility boundary
  without a shaped GitHub issue and migration proof.

## Stack & Boundaries

- A Rust workspace. `Cargo.toml` owns the crate graph.
- `crates/memory-engine-core` is the pure framework-free learning kernel.
- `crates/memory-engine` is the consumer-facing facade.
- Boundary crates own service orchestration, persistence, generation, study
  sessions, local app hosts, dogfood receipts, benchmarks, and QA.
- `.dagger/src/index.ts` is the only TypeScript surface retained after the
  Rust cutover because it owns the Dagger CI module. Browser-delivered JS
  assets under `crates/memory-engine-api/assets/` — the PWA service worker
  and the page bootstrap — are a separate and permitted exception: a browser
  executes only JS, so these cannot move to Rust. They ship as plain `.js`
  with no build step, and their behavior is gated by Rust route/render tests
  plus the `app.js` contract test.

Runtime code in `crates/memory-engine-core` must stay framework-free and
persistence-free. No Convex, React, Hono, Node/Bun APIs, filesystem, network
calls, logging, auth, analytics, UI state, or vendor SDKs belong in the pure
kernel path. Boundary crates own storage, source ingestion, sessions, UI,
identity, analytics, and model clients until repeated proof justifies
promotion.

## Ground Truth

- `VISION.md` is the canonical Scry consumer-product vision: it governs the Remember
  everything promise, quiz-first learning loop, five faces, product bars, Rust
  engine boundary and production surface.
- `SPEC.md` is the older strategy document; `docs/rust-migration.md` records
  cutover state. Use them for technical history and boundary context;
  `VISION.md` governs when product positioning conflicts.
- `SLICE-*.md` files and `exemplars.md` are historical extraction context,
  not current delivery oracles.
- GitHub Issues is the sole work ledger: issues hold shaped work, status,
  relations, proof, and closure. Git history holds archived source history.
- `.dagger/src/index.ts` owns CI behavior.
- `Cargo.toml` owns the Rust workspace; `crates/memory-engine` owns the
  consumer-facing Rust facade and module exports.
- `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` record executable QA
  and dogfood evidence.
- `docs/runbook.md` documents the sole DigitalOcean production runtime and its
  production smoke contract for agents.
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
2. Do not implement feature work without an active shaped GitHub issue.
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
12. Use the current Scry product surface, repo-local dogfood lanes, and explicitly
    shaped external proof for live-product decisions.

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
- GitHub Issues — shaped work, assignment, status, relations, proof, and closure.
- `SPEC.md` / `docs/rust-migration.md` — technical strategy and cutover context;
  `VISION.md` governs product positioning.

## Conventions

- **TDD default.** Red -> green -> refactor. Always write behavior tests
  before implementation for non-mechanical changes. Test behavior, not
  implementation.
- **No internal mocks.** Exercise real repo-owned collaborators; mock only
  external boundaries such as network, clock, and model providers.
- **Style:** `cargo fmt --all --check`; `cargo clippy --workspace
  --all-targets -- -D warnings`.
- **Docs:** `cargo doc --workspace --no-deps` must pass.
- **No non-Dagger TypeScript runtime.** `crates/memory-engine-qa` enforces
  this by file extension (`ts`, `tsx`, `mts`, `cts`) everywhere outside
  `.dagger/`. Plain `.js` browser assets are deliberately outside that scope;
  see Stack & Boundaries for why the service worker and bootstrap stay JS.

## Work Lifecycle

GitHub Issues is authoritative. `status:backlog` is groomed but not claimable;
`status:ready` has executable acceptance and proof with no unresolved blocker;
`status:blocked` names an external dependency. Assign the issue and apply
`status:in-progress` before implementation; record progress and proof as issue
comments. Use `Refs #<issue>` in commits and pull requests. Use
`Closes #<issue>` only when the merge satisfies every acceptance criterion and
no post-merge proof remains. `/deliver` stops at merge-ready; `/ship` links the
landed commit and production proof, then closes the issue and invokes bounded
`/reflect`. `/groom` reconciles open GitHub issues before strategy.

## Known Debt

- Keep the Rust cutover complete: no non-Dagger TypeScript runtime/test files
  should return, and operator docs must point at Rust crates, Cargo
  commands, Dagger CI, and the production runbook.
- Prioritize repeated phone-sized Scry dogfood receipts over new abstractions.
  Beta app extraction or promotion needs repeated evidence from the Rust app, not
  speculative client architecture.
- The largest current simplification pressure is in boundary crates,
  especially local HTTP hosts and persistence. Do not move that complexity
  into `memory-engine-core`.

## Non-goals

General-purpose hosting or auth frameworks, chat tutoring, and generalized content
import.

Organization root context: @~/Development/misty-step/AGENTS.md
