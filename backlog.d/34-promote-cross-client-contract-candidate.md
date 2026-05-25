---
shaping: true
ticket: 34-promote-cross-client-contract-candidate
slice: 6
status: ready
priority: medium
estimate: M
depends_on: [33-multi-client-beta-pressure]
oracles:
  - bun run ci
  - bun test tests/service/
  - test -f docs/beta/contract-promotion-candidate.md
---

# Promote cross-client contract candidate - only after repeated pressure

## Goal

Evaluate whether one service/helper contract should be promoted from
experiment-owned code into a stable package-facing contract after multi-client
proof exists.

## Non-Goals

- No extraction of the beta app.
- No promotion of persistence, UI state, or provider metadata into `src/`.
- No speculative contract promotion without explicit repeated evidence.

## Oracle

- [ ] A candidate contract is identified from repeated multi-client pressure
      (for example, review-state projection DTO semantics or retry/idempotency
      envelope shape).
- [ ] Evidence shows the same semantics are required by at least two
      independent workflows without client-specific fields.
- [ ] Service tests cover accepted and rejected contract paths for the chosen
      candidate.
- [ ] `docs/beta/contract-promotion-candidate.md` records the candidate,
      evidence, and rejection reasons for non-promoted alternatives.
- [ ] `bun run ci` exits 0.

## Notes

Ticket 31 rejected immediate promotion. This follow-up is the revisit gate, not
a guarantee that promotion will happen.
