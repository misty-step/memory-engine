---
shaping: true
ticket: 04-testkit-fixtures
slice: 1
status: blocked
priority: medium
estimate: S
depends_on: [02-fsrs-scheduler, 03-deterministic-grader]
oracles:
  - bun run ci
  - bun test tests/testkit/
---

# Publish fixture corpus under `memory-engine/testkit`

## Goal

Expose memory-engine's grading parity fixtures and FSRS round-trip
fixtures under the `memory-engine/testkit` subpath so that consumer apps
can run contract tests against the kernel from their own test suites.

## Non-Goals

- No consumer-facing test runner. Consumers invoke fixtures through their
  own runner.
- No rubric fixtures (slice 3).
- No progression or queue fixtures (slice 2).
- No npm publish. Testkit is consumed via local path / Bun workspace.

## Oracle

- [ ] `bun test tests/testkit/fixtures.test.ts` passes. Imports
      `gradingFixtures` and `schedulerFixtures` from
      `memory-engine/testkit`, runs every grading fixture through the
      live `Grader` and every scheduler fixture through `next()`, and
      asserts each produces the fixture's `expected` field.
- [ ] `tests/testkit/fixtures.test.ts` also asserts non-empty arrays
      (guards against an accidentally empty export).
- [ ] Ticket 03's grader tests (`tests/grader/parity.test.ts`) migrate to
      import fixtures from `memory-engine/testkit` instead of the local
      copy, and still pass. Land this migration in the same PR as this
      ticket.
- [ ] `bun run ci` exits 0.

## Notes

- Implementation: `testkit/fixtures.ts` (data), `testkit/index.ts`
  (re-exports + types).
- Exports:
  - `gradingFixtures: GradingFixture[]` — the corpus promoted from
    ticket 03.
  - `schedulerFixtures: SchedulerFixture[]` — a compact set covering the
    four FSRS states, lifted from ticket 02's round-trip test.
  - `type GradingFixture = { name: string; prompt: Prompt; submitted:
    string; ctx: GradeCtx; expected: GradeResult }`.
  - `type SchedulerFixture = { name: string; initialState: ScheduleState
    | null; rating: Rating; now: number; expected: ScheduleState }`.
- Testkit has **no runtime dependencies on ts-fsrs**. Fixtures are pure
  data; the `expected` field is a pre-computed `ScheduleState`, not a
  call to ts-fsrs. This keeps testkit consumable by any project without
  pulling ts-fsrs.
- Subpath export already declared in `package.json`:
  `"./testkit": "./testkit/index.ts"`. No manifest change needed.
- If a fixture disagrees with live behavior, the live code is the
  source of truth — update the fixture, not the code.
