---
shaping: true
ticket: 11-rubric-grading-contract
slice: 3
status: ready
priority: high
estimate: M
depends_on: [10-scry-canary]
oracles:
  - bun run ci
  - bun test tests/grader/rubric-contract.test.ts
  - bun test tests/grader/async-surface.test.ts
---

# Rubric grading contract — additive async surface

## Goal

Extend the kernel with rubric prompt/result types and an additive async grading
surface that preserves the existing synchronous deterministic `Grader`.

## Non-Goals

- No recitation work. That is ticket 09.
- No vendor SDK or network client implementation.
- No package split.
- No retries, caching, or cost-control policy.

## Oracle

- [ ] `bun test tests/grader/rubric-contract.test.ts` passes. It pins Vault's
      required-criterion, passing-score, and confidence-threshold outcome
      mapping.
- [ ] `bun test tests/grader/async-surface.test.ts` passes. Deterministic calls
      remain synchronous; rubric grading uses the new async surface.
- [ ] `bun run ci` exits 0.

## Notes

- Prefer a new additive async surface over making `Grader.grade()` always return
  a `Promise`.
- Extend `GradeResult` additively with rubric metadata; do not redesign the
  deterministic envelope.
- Study:
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/types.ts`
  - `src/grader.ts`
  - `src/types.ts`
