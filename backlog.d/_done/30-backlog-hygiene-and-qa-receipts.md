---
shaping: true
ticket: 30-backlog-hygiene-and-qa-receipts
slice: 6
status: shipped
priority: medium
estimate: S
depends_on: [24-extraction-decision-gate]
oracles:
  - bun run ci
  - bun run qa
  - test -f docs/qa/backlog-hygiene.md
---

# Backlog hygiene and QA receipts - reduce review ambiguity

## Goal

Remove process drag from stale backlog metadata and make QA receipts easier to
compare across longer beta cycles.

## Non-Goals

- No weakening CI, coverage, Biome, typecheck, or secret scanning.
- No dashboard-only QA.
- No external proof automation unless an exported-contract ticket requires it.

## Oracle

- [ ] Archived tickets no longer retain misleading `status: ready` frontmatter
      when they are already in `backlog.d/_done/`.
- [ ] Active tickets whose oracles are satisfied are archived with closure
      notes or explicitly reshaped if more work remains.
- [ ] `docs/qa/backlog-hygiene.md` documents active versus archived ticket
      invariants and closure evidence expectations.
- [ ] QA docs decide whether to add `scripts/qa.ts --report <path>` now or keep
      stdout receipts until review workflow needs persisted artifacts.
- [ ] `bun run qa` and `bun run ci` exit 0.

## Notes

- `scripts/qa.ts` currently duplicates local lanes and canonical Dagger CI in
  full mode. That is acceptable for confidence, but a future optimization can
  split "diagnostic full sweep" from "canonical handoff gate."
- Product proof remains separate from package QA. Add exact dogfood, beta, or
  external proof lanes only when a ticket changes exported contracts that must
  be validated outside the local package harness.

## Closure Evidence

- Archived completed active tickets `26` and `27` after focused beta-store and
  beta-generation proof passed.
- Normalized archived tickets so `_done/` no longer contains `status: ready`.
- Added `docs/qa/backlog-hygiene.md` to document active/archive invariants,
  closure evidence expectations, and the stdout-first QA receipt decision.
- Verified with `bun run qa` and `bun run ci`.
