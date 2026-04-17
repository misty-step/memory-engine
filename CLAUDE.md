# memory-engine

Shared learning engine kernel for four portfolio apps (Ruminatio, Vault SRS,
Caesar in a Year, Scry). Extracted from duplicated SRS/grading logic.
See `SPEC.md` for strategy, `SLICE-1-KERNEL.md` for the first buildable slice.

## What this repo is

- A single TypeScript package: canonical domain types + FSRS reference
  scheduler + deterministic grader.
- **Not** a monorepo yet. The 4-package split (`contracts`, `core`,
  `adapters`, `testkit`) is deferred to slice 2.
- **Not** a service. Consumers import it.

## Gate

`bun run ci` is the canonical gate. It shells out to
`dagger call check --source=.` and runs, in order:
`tsc --noEmit` → `biome check .` → coverage-enforced tests →
`gitleaks dir`. All four must exit 0.

`bun run ci:local` is the faster inner-loop variant:
`tsc --noEmit` → `biome check .` → coverage-enforced tests. Use it while
iterating, but never hand off without a green `bun run ci`.

## Invariants (load-bearing, do not violate)

1. **Core is pure.** No Convex, React, Hono, Node/Bun APIs in the runtime
   path under `src/`. Time, storage, and identity are injected.
2. **Consumer owns persistence.** Scheduler receives `ScheduleState` as
   argument and returns the next state. It never reads or writes storage.
3. **`ScheduleState` matches ts-fsrs `Card` exactly** (snake_case,
   `state: 0|1|2|3`, `last_review: number | null`). This leak is deliberate
   and documented — consumers must not assume it will survive a scheduler
   swap. See the ADR trail in git history before changing.
4. **Prompt ↔ Grader co-evolve.** Adding a `Prompt` union arm requires a
   concurrent grader dispatch update and an `assertNever` exhaustiveness
   check. Never ship one without the other.
5. **Verdict vocabulary is fixed**: `'correct' | 'close' | 'wrong' | 'revealed'`.
   Renames elsewhere (Ruminatio's `partial`, Caesar's SCREAMING case) map
   to these four; no new verdicts without a spec update.
6. **One grade call, one result.** `Grader.grade()` returns a `GradeResult`
   with `rating` already populated by the injected rating policy. No
   two-step verdict-then-rating protocol across the module boundary.
7. **Authority order:** tests > type system > code > docs > lore.

## Layout

- `src/` — production code (single package, subpath `memory-engine`).
- `testkit/` — shared fixtures + helpers (subpath `memory-engine/testkit`).
  Consumed by slice-1 tests and by future consumer contract tests.
- `tests/` — kernel test suite. One behavior per test. No mocking of
  ts-fsrs — exercise the real scheduler.
- `.dagger/` — CI pipeline (TypeScript SDK). Treat as owned code; changes
  require the same review as `src/`.
- `backlog.d/` — shaped tickets awaiting `/deliver`.
- `SPEC.md` / `SLICE-1-KERNEL.md` — authoritative on scope and shape.

## Conventions

- **TDD default.** Red → green → refactor. The exception is pure type-level
  scaffolding. Always write a test before implementing.
- **No mocks of ts-fsrs.** The FSRS round-trip oracle requires real behavior.
- **Lift, don't redesign.** Slice 1 lifts Vault SRS's grading thresholds
  and fixtures verbatim. See `exemplars.md` for what to study.
- **Style:** single quotes, semicolons, trailing commas, 2-space indent,
  100-wide (enforced by Biome). `type`-only imports required
  (`verbatimModuleSyntax`).
- **Strictness:** `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`
  are on. Array/object access can be `undefined`; optional properties are
  not the same as `| undefined`.

## Non-goals (this repo)

Progression graph (slice 2), queue selection (slice 2), rubric/LLM grading
(slice 3), session choreography (app-owned), content authoring (app-owned),
storage adapters (slice 2), npm publish (later).
