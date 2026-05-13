# Agent Operations

Harness-neutral guidance for agents working in `memory-engine`. Claude Code
also reads `CLAUDE.md`; Codex and other harnesses should treat this file as the
router and `.spellbook/repo-brief.md` as the dense project spine.

## Stack & Boundaries

`memory-engine` is a pure TypeScript/Bun package for shared learning-kernel
behavior: canonical types, FSRS scheduling, deterministic and rubric grading
contracts, progression, queue primitives, adapters, and fixture corpora.

Runtime code in `src/` must stay framework-free. No Convex, React, Hono,
Node/Bun APIs, filesystem access, network calls, logging, persistence, or
vendor SDKs belong in the core runtime path. Consumers own storage, UI, session
choreography, content parsing, auth, analytics, and model clients.

## Ground Truth

- `.spellbook/repo-brief.md` is the current repo brief for tailored skills.
- `SPEC.md` is the strategy document.
- `SLICE-*.md` files define shaped slice context.
- `backlog.d/` contains active work; `backlog.d/_done/` contains closed work.
- `exemplars.md` names consumer/source patterns to lift.
- `.dagger/src/index.ts` owns CI behavior.
- `package.json` owns public exports: `memory-engine`,
  `memory-engine/testkit`, and `memory-engine/adapters`.

## Gate Contract

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and
runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

`bun run ci:local` is the inner loop only. It runs typecheck, Biome check, and
coverage. Delivery requires a green `bun run ci`.

## Invariants

- One ticket, one branch, one PR. Branch from `master`; use `cx/...` by default.
- Do not implement feature work without an active shaped `backlog.d/` ticket.
- TDD is the default. Write behavior tests before implementation.
- `ScheduleState` is the JSON-safe `ts-fsrs` card shape: snake_case,
  `state: 0 | 1 | 2 | 3`, `last_review: number | null`.
- `ReviewUnitId` is opaque. The kernel never inspects concept-vs-phrasing
  meaning.
- Prompt union changes and grader dispatch changes ship together with
  `assertNever` coverage.
- `Grader.grade()` returns one `GradeResult` envelope with `rating` populated.
- Verdicts remain `correct`, `close`, `wrong`, `revealed`.
- No `any`, non-null assertions, or `@ts-ignore`.
- Do not add runtime dependencies without shaped scope and docs updates.
- Do not lower gates, bypass hooks, or mark unrun canaries as proof.

## Backlog Lifecycle

Active tracker: `backlog.d/`.

Closed tracker: `backlog.d/_done/`.

Structured signals:

- `Refs-backlog: NN` references work without closing it.
- `Closes-backlog: NN` and `Ships-backlog: NN` close work.

Archive operation: source `scripts/lib/backlog.sh` and use `backlog_archive`.
Review verdict helpers live in `scripts/lib/verdicts.sh`.

## Known Debt

- `SPEC.md` still describes slices 2 and 3 as immediate next work even though
  code for progression, queue, recitation, rubric contracts, and adapters is on
  `master`.
- Tickets `10` through `13` are archived under `_done/` but still carry
  `status: ready` in frontmatter.
- External canary branches exist for Scry and Vault SRS; re-run or record their
  oracles before claiming consumer adoption is complete.

## Harness Index

| Skill | What it does here |
|---|---|
| `/shape` | Creates slice packets and atomic `backlog.d/` tickets with executable oracles. |
| `/implement` | Executes one shaped ticket with TDD and pure-kernel discipline. |
| `/ci` | Runs or diagnoses the Dagger-backed `bun run ci` gate. |
| `/code-review` | Reviews diffs against kernel invariants and canary evidence. |
| `/qa` | Verifies package exports, fixtures, focused tests, and named consumer canaries. |
| `/refactor` | Simplifies without widening kernel scope or inventing package splits. |
| `/groom` | Reconciles backlog status, roadmap drift, and next shaped work. |
| `/deliver` | Drives one ticket to merge-ready code; does not ship. |
| `/settle` | Polishes a branch until CI, review, refactor, and QA are clean. |
| `/ship` | Archives tickets, preserves closing trailers, merges, and reflects. |
| `/yeet` | Slices worktree changes into conventional commits and pushes. |
| `/flywheel` | Composes pick → shape → implement → yeet → settle → ship; no deploy leaf here. |
| `/diagnose` | Reproduces failing oracles and fixes root causes. |
| `/research` | Grounds shaping in official docs, consumer exemplars, and prior art. |
| `/demo` | Produces command, API, fixture, or canary evidence for reviewers. |
| `/office-hours` | Interrogates raw ideas before shaping. |
| `/ceo-review` | Challenges plans and context packets before convergence. |
| `/reflect` | Captures learnings and harness follow-ups after shipping. |

Installed agents live in `.claude/agents/`. Shared skills live in
`.agent/skills/`; `.claude/skills/`, `.codex/skills/`, and `.pi/skills/` are
bridges back to that shared root.
