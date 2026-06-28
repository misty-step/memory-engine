# Agent Operations


## Stack & Boundaries

`memory-engine` is a Rust learning-kernel and repo-local dogfood workspace.
Runtime code in `crates/memory-engine-core` must stay framework-free and
persistence-free. No Convex, React, Hono, Node/Bun APIs, filesystem, network
calls, logging, auth, analytics, UI state, or vendor SDKs belong in the pure
kernel path. Boundary crates own storage, source ingestion, sessions, UI,
identity, analytics, and model clients until repeated proof justifies promotion.

## Ground Truth

- `SPEC.md` is the strategy document.
- `SLICE-*.md` files and `exemplars.md` are historical extraction context,
  not current delivery oracles.
- `backlog.d/` contains active work; `backlog.d/_done/` contains closed work.
- `.dagger/src/index.ts` owns CI behavior.
- `Cargo.toml` owns the Rust workspace; `crates/memory-engine` owns the
  consumer-facing Rust facade and module exports.
- `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` record executable QA and dogfood evidence.
- `docs/runbook.md` documents the deployed Fly surface and production smoke
  contract for agents.

## Gate Contract

`bun run ci` IS the default fast gate. It runs directly on the host through
Cargo: Rust formatting, workspace tests, Clippy, and rustdoc. It is the
pre-push and day-to-day agent loop.

`bun run ci:full` is the Dagger-backed full/ship parity gate. Keep it when the
containerized Postgres service, pinned Rust image, and Gitleaks scan matter.
Hosted CI calls this repo-owned script instead of raw Dagger.

`bun run ci:local` remains a compatibility alias for the fast gate. `bun run qa`
is the full QA sweep and ends with `bun run ci:full`, but it does not replace
the fast gate. Delivery requires `bun run ci`, `bun run ci:full` before handoff,
and any ticket-named proof oracle.

## Invariants

- One ticket, one branch, one PR. Branch from `master`; use `cx/...` by default.
- Do not implement feature work without an active shaped `backlog.d/` ticket.
- TDD is the default. Test behavior, not implementation, and do not mock repo-owned pure collaborators.
- `ScheduleState` is the JSON-safe scheduler card shape: snake_case,
  `state: 0 | 1 | 2 | 3`, `last_review: number | null`.
- `ReviewUnitId` is opaque. The kernel never inspects concept-vs-phrasing meaning.
- Prompt enum changes and grader dispatch changes ship together with exhaustive
  Rust match coverage.
- `Grader::grade()` returns one `GradeResult` envelope with `rating` populated.
- Verdicts remain `correct`, `close`, `wrong`, `revealed`.
- Do not add runtime dependencies without shaped scope and docs updates.
- Do not lower gates, bypass hooks, or mark unrun canaries as proof.
- Historical Scry and Vault canary branches are deprecated. Use repo-local dogfood lanes and explicitly shaped current external proof instead.

## Backlog Lifecycle

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

`/deliver` stops at merge-ready. `/ship` archives tickets, preserves closure trailers, merges, verifies archive state, writes final trace when available, and invokes bounded `/reflect`. `/groom` always reconciles tracker truth before strategy.

## Known Debt

- Keep the Rust cutover complete: no non-Dagger TypeScript runtime/test files
  should return, and operator docs must point at Rust crates, Cargo commands,
  Dagger CI, and the Fly runbook.
- After cutover, prioritize repeated phone-sized dogfood receipts over new
  abstractions. Beta app extraction or promotion needs repeated evidence from
  the Rust app, not archived TypeScript-era tickets.
- The largest current simplification pressure is in boundary crates, especially
  local HTTP hosts and persistence. Do not move that complexity into
  `memory-engine-core`.
