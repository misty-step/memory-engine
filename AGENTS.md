# Agent Operations

Harness-neutral guidance for agents working in `memory-engine`. Claude Code also reads `CLAUDE.md`; Codex, Pi, and other harnesses should treat this file as the router and `.spellbook/repo-brief.md` as the dense project spine.

## Stack & Boundaries

`memory-engine` is a Bun/TypeScript learning-kernel package plus repo-local dogfood experiments. Runtime code in `src/` must stay framework-free and persistence-free. No Convex, React, Hono, Node/Bun APIs, filesystem, network calls, logging, auth, analytics, UI state, or vendor SDKs belong in the published runtime path. Consumers and `experiments/` own storage, source ingestion, sessions, UI, identity, analytics, and model clients until repeated proof justifies promotion.

## Ground Truth

- `.spellbook/repo-brief.md` is the current tailored repo brief.
- `SPEC.md` is the strategy document.
- `SLICE-*.md` files define shaped slice context.
- `backlog.d/` contains active work; `backlog.d/_done/` contains closed work.
- `exemplars.md` names consumer/source systems to lift from.
- `.dagger/src/index.ts` owns CI behavior.
- `package.json` owns public exports: `memory-engine`, `memory-engine/testkit`, `memory-engine/adapters`, `memory-engine/grading`, `memory-engine/progression`, `memory-engine/queue`, `memory-engine/scheduling`, and `memory-engine/types`.
- `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` record executable QA and dogfood evidence.

## Gate Contract

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

`bun run ci:local` is the inner loop only. `bun run qa` is the full QA sweep and ends with `bun run ci`, but it does not replace the gate. Delivery requires `bun run ci` plus any ticket-named proof oracle.

## Invariants

- One ticket, one branch, one PR. Branch from `master`; use `cx/...` by default.
- Do not implement feature work without an active shaped `backlog.d/` ticket.
- TDD is the default. Test behavior, not implementation, and do not mock repo-owned pure collaborators.
- `ScheduleState` is the JSON-safe `ts-fsrs` card shape: snake_case, `state: 0 | 1 | 2 | 3`, `last_review: number | null`.
- `ReviewUnitId` is opaque. The kernel never inspects concept-vs-phrasing meaning.
- Prompt union changes and grader dispatch changes ship together with `assertNever` coverage.
- `Grader.grade()` returns one `GradeResult` envelope with `rating` populated.
- Verdicts remain `correct`, `close`, `wrong`, `revealed`.
- No `any`, non-null assertions, or `@ts-ignore`.
- Do not add runtime dependencies without shaped scope and docs updates.
- Do not lower gates, bypass hooks, or mark unrun canaries as proof.
- Historical Scry and Vault canary branches are deprecated. Use repo-local dogfood lanes and explicitly shaped current external proof instead.

## Backlog Lifecycle

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

`/deliver` stops at merge-ready. `/settle` stops at ship-ready. `/ship` archives tickets, preserves closure trailers, merges, verifies archive state, writes final trace when available, and invokes bounded `/reflect`. `/groom` always reconciles tracker truth before strategy.

## Known Debt

- `backlog.d/30-backlog-hygiene-and-qa-receipts.md`: reconcile active tickets whose oracles are already satisfied, including likely stale active `26` and `27`, and clean archived `status: ready` frontmatter.
- `backlog.d/28-mobile-beta-study-interface.md`: mobile-first beta usefulness is the next high-risk product proof after persistence and generation.
- `backlog.d/29-service-contract-v0-hardening.md`: harden service DTO/reveal/failure semantics after beta pressure exists.
- `backlog.d/32-graduated-activity-ladder.md`: treat exercises/practice problems as first-class beta activities, not quiz variants only.
- `backlog.d/31-beta-extraction-decision.md`: decide promote, extract, keep experimenting, or reshape after enough beta ladder evidence.
- `backlog.d/16-system-visualization-workbench.md`: useful later; do not let it displace beta usefulness unless architecture confusion causes repeated defects.

## Harness Index

| Skill | What it does here |
|---|---|
| `/research` | Grounds memory-engine shaping in official docs, repo docs, consumer exemplars, and prior art before moving kernel contracts. |
| `/groom` | Reconciles `backlog.d/` truth, stale archived status, shipped trailers, and Slice 6 priority before strategy. |
| `/shape` | Creates atomic `backlog.d/` tickets and slice packets with executable oracles and explicit kernel/application boundaries. |
| `/implement` | Executes one shaped ticket with TDD, pure-kernel discipline, focused tests, and `Refs-backlog` metadata. |
| `/qa` | Verifies package exports, fixtures, adapters, focused suites, dogfood lanes, beta receipts, and the canonical gate. |
| `/demo` | Produces reviewer evidence: command receipts, API import snippets, fixture examples, dogfood/beta receipts. |
| `/code-review` | Reviews diffs against shaped ticket, kernel invariants, no-internal-mock policy, strict TS/Biome, and proof evidence. |
| `/refactor` | Simplifies touched kernel, service, test, Dagger, or harness surfaces without widening package scope. |
| `/ci` | Runs or diagnoses the Dagger-backed `bun run ci` gate. |
| `/diagnose` | Reproduces exact failing oracles across Dagger, Bun tests, Biome, coverage, Gitleaks, exports, dogfood, or beta proof. |
| `/monitor` | Watches non-deploy signals: CI, QA, coverage, package exports, dogfood/beta evidence, benchmarks, and backlog lifecycle drift. |
| `/deliver` | Drives one active ticket to merge-ready code and stops before push, merge, archive, deploy, or reflection. |
| `/settle` | Polishes a branch until CI, review, refactor, QA, and backlog metadata are coherent and ship-ready. |
| `/ship` | Archives closing tickets, preserves closure trailers, merges to `master`, verifies archive state, traces, and reflects. |
| `/trace` | Writes local append-only work records under `.spellbook/traces/` with redacted evidence refs and final ship records. |
| `/yeet` | Slices intentional worktree changes into conventional commits with structured backlog trailers and pushes reviewable branch state. |
| `/flywheel` | Composes pick -> shape -> implement -> yeet -> settle -> ship -> monitor; `/ship` owns closure and reflection. |
| `/office-hours` | Interrogates raw ideas before shaping. |
| `/ceo-review` | Challenges plans and context packets before convergence. |
| `/reflect` | Captures session and cycle learnings without becoming the work-record store. |

## Agent Index

| Agent | What it does here |
|---|---|
| `planner` | Produces bounded implementation or harness plans from the repo brief and active ticket. |
| `builder` | Implements scoped code changes under the shaped ticket and repo invariants. |
| `critic` | Reviews acceptance, evidence, and cross-skill cohesion before handoff. |
| `beck` | Enforces TDD rhythm and behavior-first tests. |
| `cooper` | Challenges internal mocks and classicist test design. |
| `ousterhout` | Reviews module depth, information hiding, and shallow wrappers. |
| `carmack` | Cuts scope and speculative abstraction. |
| `grug` | Flags avoidable complexity and cleverness. |

Shared skills live in `.agent/skills/`; `.claude/skills/`, `.codex/skills/`, and `.pi/skills/` are bridges back to that root. Installed agents live in `.claude/agents/`.
