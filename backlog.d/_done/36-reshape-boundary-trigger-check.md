---
shaping: true
ticket: 36-reshape-boundary-trigger-check
slice: 6
status: ready
priority: low
estimate: S
depends_on: [33-multi-client-beta-pressure]
oracles:
  - bun run ci
  - test -f docs/beta/boundary-reshape-trigger-check.md
---

# Reshape boundary trigger check - explicit gate for kernel/service ownership changes

## Goal

Define concrete triggers for reshaping kernel/service boundaries (for example,
shared hosted memory-service needs across multiple consumers) and verify
whether those triggers are actually present.

## Non-Goals

- No immediate boundary reshaping.
- No persistence/provider/UI ownership move into `src/` without proof.
- No hosted service implementation.

## Oracle

- [ ] Trigger criteria are explicit, measurable, and tied to repeated
      multi-consumer evidence.
- [ ] Current beta evidence is evaluated against those criteria.
- [ ] The outcome is explicit: keep boundary as-is or shape a boundary-change
      ticket with exact scope.
- [ ] `docs/beta/boundary-reshape-trigger-check.md` records the criteria,
      evidence, and outcome.
- [ ] `bun run ci` exits 0.

## Notes

Ticket 31 rejected immediate boundary reshaping. This follow-up keeps the
alternative alive with a concrete proof threshold.
