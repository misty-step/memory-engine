---
shaping: true
ticket: 10-scry-canary
slice: 2
status: ready
priority: high
estimate: M
depends_on: [07-progression-primitives, 08-queue-primitives, 09-recitation-and-slice2-fixtures]
oracles:
  - bun run ci
  - (cd /Users/phaedrus/Development/scry && git switch memory-engine-canary && bun test)
---

# Scry concept-level canary — slice-2 acceptance gate

## Goal

Prove the slice-2 boundary works for a concept-level consumer by wiring Scry's
review-state path through memory-engine behind a consumer-owned mapping layer,
without forcing concept-specific assumptions into core.

## Non-Goals

- No phrasing generation or wider Scry product work.
- No storage schema migration in Scry.
- No rubric grading.
- No attempt to force Scry onto queue primitives it does not need yet.

## Oracle

- [ ] `cd /Users/phaedrus/Development/scry && git switch
      memory-engine-canary && bun test` exits 0.
- [ ] Scry keeps its own persisted FSRS state shape and maps to/from
      `ScheduleState` at the consumer edge.
- [ ] Any adopted progression or queue helper imports come from
      `memory-engine` rather than re-implementing local equivalents.
- [ ] `bun run ci` in `memory-engine` still exits 0 after canary-driven
      kernel changes.

## Notes

- The point of this ticket is `ReviewUnitId` opacity. Do not add concept-only
  fields to core types to make the canary easier.
- If Scry only adopts scheduler/progression in the canary, that is acceptable.
  The canary exists to prove the boundary, not to inflate scope.
- Study:
  - `/Users/phaedrus/Development/scry/convex/fsrs/engine.ts`
  - `src/scheduler.ts`
  - `src/types.ts`
