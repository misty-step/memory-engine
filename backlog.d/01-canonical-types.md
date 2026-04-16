---
shaping: true
ticket: 01-canonical-types
slice: 1
status: ready
priority: high
estimate: S
depends_on: []
oracles:
  - bun run ci
  - bun test tests/types/
---

# Canonical domain types + assertNever

## Goal

Publish the slice-1 canonical domain types — `Prompt`, `Verdict`, `Rating`,
`ScheduleState`, `GradeResult`, `ReviewUnitId`, `GraderKind` — plus an
`assertNever` exhaustiveness helper, as the type substrate every other
slice-1 module consumes.

## Non-Goals

- No runtime logic beyond `assertNever`. This ticket is types + one helper.
- No extra fields beyond what `SLICE-1-KERNEL.md` enumerates. Progression
  fields (`progressionGroup`, `requires`, `supersedes`, `stageOrder`) are
  explicitly out — slice 2.
- No camelCase translation of `ScheduleState`. It must match ts-fsrs `Card`
  byte-exact (snake_case, numeric `state: 0|1|2|3`, `last_review: number |
  null`). Alias, don't rebuild.
- No rubric or recitation Prompt arms. Only `mcq | boolean | cloze |
  shortAnswer`.

## Oracle

- [ ] `bun run ci` exits 0 (typecheck + Biome + tests).
- [ ] `bun test tests/types/exhaustiveness.test.ts` passes, and contains a
      switch over `Prompt['kind']` where each arm is narrowed and the
      default branch calls `assertNever(prompt)`. A `// @ts-expect-error`
      next to a commented-out arm must still error, proving the union is
      exhaustive.
- [ ] `bun test tests/types/schedule-state-shape.test.ts` passes — imports
      ts-fsrs's `Card` and asserts `ScheduleState` is structurally
      identical at the type level (`expectType<Card>` helper or equivalent).
- [ ] `src/index.ts` re-exports every type plus `assertNever`.

## Notes

- Implementation: `src/types.ts` (types) + `src/assert.ts` (single function).
- Discriminator field on `Prompt` is `kind` (not `type` — avoids the
  TypeScript-keyword confusion that bites code review).
- Study `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/types.ts`.
  Lift `GradeResult`-adjacent shapes (`criterionResults` etc. are slice-3;
  omit). `FsrsCardState` there IS `ScheduleState` — alias from ts-fsrs
  directly rather than transcribing.
- `assertNever(value: never): never` throws `new Error(\`Unreachable:
  \${JSON.stringify(value)}\`)`. No side channels, no logging — pure throw.
- Every downstream slice-1 ticket's dispatcher uses `assertNever`. This
  ticket is the blocker for 02 and 03.
