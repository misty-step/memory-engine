---
shaping: true
ticket: 02-fsrs-scheduler
slice: 1
status: ready
priority: high
estimate: S
depends_on: [01-canonical-types]
oracles:
  - bun run ci
  - bun test tests/scheduler/
---

# FSRS scheduler wrapper — `next(state, rating, now)`

## Goal

Wrap `ts-fsrs` behind a single pure function `next(state, rating, now) →
state` that advances FSRS state deterministically and survives JSON
round-trip byte-identically.

## Non-Goals

- No queue selection (slice 2).
- No progression-aware filtering (slice 2).
- No custom retention parameters — hardcoded `request_retention: 0.9`,
  `maximum_interval: 36500`.
- No mocking `ts-fsrs` in tests. Exercise the real scheduler. The whole
  point of the round-trip oracle is to pin real behavior.
- No dependency on `src/grader.ts`. This module stands alone.

## Oracle

- [ ] `bun test tests/scheduler/roundtrip.test.ts` passes. Scenario:
      `next(null, Rating.Good, t0)` → serialize to JSON → `JSON.parse`
      → `next(parsed, Rating.Good, t0 + 1d)`. Compare byte-by-byte
      (`JSON.stringify`) against a control path that uses `ts-fsrs`
      directly (`createEmptyCard` → `fsrs().repeat(card, t0)[Good].card`
      → repeat). Must match exactly.
- [ ] `bun test tests/scheduler/serialize.test.ts` passes. For each FSRS
      state value (`0` new, `1` learning, `2` review, `3` relearning),
      build a representative `ScheduleState`, serialize → parse → assert
      `deepEqual`. Then call `next` on the original and on the parsed;
      assert `deepEqual` again.
- [ ] `bun run ci` exits 0.

## Notes

- Implementation: `src/scheduler.ts`. One exported function.
- Signature: `export function next(state: ScheduleState | null, rating:
  Rating, now: number): ScheduleState`.
- When `state === null`: call `createEmptyCard(new Date(now))` to produce
  the initial card.
- When `state !== null`: pass it directly to `scheduler.repeat` — because
  `ScheduleState === Card` (ticket 01), no translation.
- Instantiate the scheduler once at module scope: `const scheduler =
  fsrs({ request_retention: 0.9, maximum_interval: 36500 });`.
- Return value: `scheduler.repeat(card, new Date(now))[rating].card`.
  ts-fsrs keys its result object by `Rating` (1–4); the narrowing is safe
  because `Rating` is exactly `1|2|3|4`.
- Study `/Users/phaedrus/Development/caesar-in-a-year/lib/srs/fsrs.ts` for
  the minimal-wrapper pattern. Caesar's `scheduleReview` is what this
  ticket generalizes.

## Parallelism

Can proceed in parallel with 03 once 01 is merged. No cross-dependency.
