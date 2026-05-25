---
shaping: true
ticket: 35-extract-beta-app-readiness-gate
slice: 6
status: ready
priority: medium
estimate: M
depends_on: [33-multi-client-beta-pressure, 34-promote-cross-client-contract-candidate]
oracles:
  - bun run ci
  - bun test experiments/beta-study/
  - test -f docs/beta/extract-beta-app-readiness.md
---

# Extract beta app readiness gate - defer split until proof is repeated

## Goal

Define and execute an explicit extraction-readiness gate for moving the beta
study interface into its own repository, only if evidence shows sustained value
and stable boundaries.

## Non-Goals

- No extraction by default.
- No package split that breaks current repo-local QA receipts.
- No hosted production rollout.

## Oracle

- [ ] Readiness criteria compare in-repo operation versus extracted-repo cost,
      including QA coverage, release friction, ownership clarity, and boundary
      stability.
- [ ] Multi-session beta receipts show sustained usage value, not one-off
      fixture success.
- [ ] Contract dependencies between beta app and kernel are explicit and
      minimal.
- [ ] `docs/beta/extract-beta-app-readiness.md` records extract/hold decision
      and evidence.
- [ ] `bun run ci` exits 0.

## Notes

Ticket 31 rejected immediate extraction. This ticket keeps extraction as a
high-value option with explicit acceptance criteria.
