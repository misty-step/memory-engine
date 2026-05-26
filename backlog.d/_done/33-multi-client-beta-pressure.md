---
shaping: true
ticket: 33-multi-client-beta-pressure
slice: 6
status: ready
priority: high
estimate: L
depends_on: [31-beta-extraction-decision]
oracles:
  - bun run ci
  - bun test experiments/beta-study/
  - test -f docs/beta/multi-client-pressure.md
---

# Multi-client beta pressure - prove repeated interface needs before promotion

## Goal

Keep experimenting by adding a second independent beta workflow/client that
uses the same persistence and service boundaries, then document what command
semantics and DTO projections repeat across both workflows.

## Non-Goals

- No repo extraction.
- No new public export from `src/`.
- No hosted deployment.
- No provider-specific contracts in kernel surfaces.

## Oracle

- [ ] A second beta workflow/client runs through source ingest, approval,
      queue, reveal, submit, and restart/resume against the existing beta
      persistence and service boundaries.
- [ ] Shared behavior evidence is captured across both workflows, including
      reveal semantics, duplicate-submit handling, and review-state projection.
- [ ] Any divergences are documented as explicit product-owned behavior rather
      than accidental contract drift.
- [ ] `docs/beta/multi-client-pressure.md` records repeated signals versus
      client-specific behavior.
- [ ] `bun run ci` exits 0.

## Notes

This is the selected follow-up for ticket 31. Extraction, promotion, and
boundary reshaping should wait until repeated pressure exists across at least
two independent workflows.
