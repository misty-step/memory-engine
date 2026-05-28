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
- `SLICE-*.md` files define shaped slice context.
- `backlog.d/` contains active work; `backlog.d/_done/` contains closed work.
- `exemplars.md` names consumer/source systems to lift from.
- `.dagger/src/index.ts` owns CI behavior.
- `Cargo.toml` owns the Rust workspace; `crates/memory-engine` owns the
  consumer-facing Rust facade and module exports.
- `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` record executable QA and dogfood evidence.

## Gate Contract

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and
runs Rust formatting, workspace tests, Clippy, rustdoc, and Gitleaks.

`bun run ci:local` is the inner loop only. `bun run qa` is the full QA sweep and ends with `bun run ci`, but it does not replace the gate. Delivery requires `bun run ci` plus any ticket-named proof oracle.

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

`/deliver` stops at merge-ready. `/settle` stops at ship-ready. `/ship` archives tickets, preserves closure trailers, merges, verifies archive state, writes final trace when available, and invokes bounded `/reflect`. `/groom` always reconciles tracker truth before strategy.

## Known Debt

- `backlog.d/28-mobile-beta-study-interface.md`: highest-priority active work. Mobile-first beta usefulness is the next high-risk product proof after persistence and generation.
- `backlog.d/29-service-contract-v0-hardening.md`: harden service DTO/reveal/failure semantics after beta pressure exists.
- `backlog.d/32-graduated-activity-ladder.md`: treat exercises/practice problems as first-class beta activities, not quiz variants only.
- `backlog.d/31-beta-extraction-decision.md`: decide promote, extract, keep experimenting, or reshape after enough beta ladder evidence.
- `backlog.d/16-system-visualization-workbench.md`: useful later; do not let it displace beta usefulness unless architecture confusion causes repeated defects.
