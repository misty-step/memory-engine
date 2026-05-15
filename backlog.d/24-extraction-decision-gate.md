---
shaping: true
ticket: 24-extraction-decision-gate
slice: 5
status: ready
priority: medium
estimate: S
depends_on: [20-evals-and-benchmarks-baseline, 21-cli-review-loop-dogfood, 23-web-study-shell-dogfood]
oracles:
  - bun run ci
  - test -f docs/dogfood/extraction-decision.md
---

# Extraction decision gate - choose winners from dogfood evidence

## Goal

Compare dogfood evidence from experimental clients and decide which interface,
if any, should be extracted into its own application repository, which helpers
belong in `testkit`, and which service/API contracts should remain private.

## Non-Goals

- No extraction in this ticket.
- No production app build.
- No package split.
- No service export without evidence from at least two clients.

## Oracle

- [ ] `docs/dogfood/extraction-decision.md` compares CLI, import, and web-shell
      evidence against API ergonomics, repeated boundary needs, eval coverage,
      and benchmark receipts.
- [ ] The decision names one of: extract a client, keep experimenting, promote a
      helper to testkit, or reshape the API.
- [ ] Follow-up tickets are created for any selected extraction or API change.
- [ ] `bun run ci` exits 0.

## Notes

Extraction requires repeated pressure. A helper used by one experiment is
client-local; a helper independently needed by two experiments may deserve a
stable package surface or a future application repository.
