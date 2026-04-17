---
shaping: true
ticket: 08-queue-primitives
slice: 2
status: ready
priority: high
estimate: M
depends_on: [07-progression-primitives]
oracles:
  - bun run ci
  - bun test tests/queue/
---

# Queue primitives — due-first selection and anti-clumping

## Goal

Add shallow queue primitives for next-card selection: due-first ordering,
progression-aware eligibility, and anti-clumping over recent history, while
leaving app-owned burst/session choreography outside core.

## Non-Goals

- No full `SessionPlan` or session-builder abstraction.
- No Ruminatio `selectBurst()` migration in this ticket.
- No parser/content taxonomy work.
- No rubric capability beyond optional gating flags already carried in queue
  inputs.

## Oracle

- [ ] `bun test tests/queue/next.test.ts` passes. Cases are lifted from Vault's
      `pickNextDueCard()` suite: review-before-new, same-source anti-clumping,
      progression suppression, and locked-stage fallback.
- [ ] `bun test tests/queue/priority.test.ts` passes. It pins urgency ordering,
      review-state tie-breaks, and progression-family `stageOrder` behavior.
- [ ] `bun run ci` exits 0.

## Notes

- Implementation likely lives in `src/queue.ts`.
- Candidate shape should stay canonical and shallow: `reviewUnitId`,
  `scheduleState`, progression metadata, due timestamp, and optional concept /
  source / domain keys for anti-clumping.
- Anti-clumping windows and key selection belong in options, not in hardcoded
  knowledge of Vault's card documents.
- Do not absorb Ruminatio's `selectBurst()` into core yet. If the canaries later
  prove burst composition is genuinely shared, shape a follow-up from evidence.
- Study:
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/queue.ts`
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/queue.test.ts`
  - `/Users/phaedrus/Development/ruminatio/convex/scheduler.ts`
