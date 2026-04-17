---
shaping: true
ticket: 09-recitation-and-slice2-fixtures
slice: 2
status: ready
priority: medium
estimate: M
depends_on: [07-progression-primitives, 08-queue-primitives]
oracles:
  - bun run ci
  - bun test tests/grader/recitation.test.ts
  - bun test tests/testkit/slice2-fixtures.test.ts
---

# Deterministic recitation + slice-2 fixture corpus

## Goal

Add `recitation` as the remaining deterministic prompt arm and publish slice-2
fixture corpora for progression, queue, and recitation through
`memory-engine/testkit`.

## Non-Goals

- No rubric grading or adapter work.
- No audio transcription, voice input, or speech scoring.
- No consumer canary wiring in this ticket.

## Oracle

- [ ] `bun test tests/grader/recitation.test.ts` passes using cases lifted from
      Ruminatio's existing study-system tests.
- [ ] `bun test tests/testkit/slice2-fixtures.test.ts` passes. It imports
      progression, queue, and recitation fixtures from `memory-engine/testkit`
      and validates each against live kernel behavior.
- [ ] Existing deterministic grader and queue suites stay green.
- [ ] `bun run ci` exits 0.

## Notes

- `recitation` is deterministic long-form recall. It reuses the exact-answer /
  accepted-variant / near-miss path; it is explicitly not a rubric prompt.
- If fixture data disagrees with live kernel behavior, update the fixture only
  after proving the live behavior matches the source-system oracle.
- Study:
  - `/Users/phaedrus/Development/ruminatio/convex/lib/grading.ts`
  - `/Users/phaedrus/Development/ruminatio/apps/web/app/study-system.test.ts`
  - `testkit/fixtures.ts`
