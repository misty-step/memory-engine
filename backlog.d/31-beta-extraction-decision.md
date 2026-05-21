---
shaping: true
ticket: 31-beta-extraction-decision
slice: 6
status: ready
priority: medium
estimate: S
depends_on: [28-mobile-beta-study-interface, 29-service-contract-v0-hardening, 32-graduated-activity-ladder]
oracles:
  - bun run ci
  - bun run qa
  - test -f docs/beta/extraction-decision.md
---

# Beta extraction decision - promote, extract, or keep experimenting

## Goal

After the mobile beta interface, service-contract hardening, and graduated
activity ladder evidence exist, decide which parts should be extracted,
promoted, or kept private.

## Non-Goals

- No extraction in this ticket.
- No package split without explicit follow-up work.
- No public service export based on a single beta workflow.
- No database ownership change without evidence that multiple consumers need a
  shared hosted memory service.

## Oracle

- [ ] `docs/beta/extraction-decision.md` compares CLI, import, web shell, beta
      persistence, beta generation, mobile study, graduated activity ladder,
      and service-contract evidence.
- [ ] The decision chooses exactly one primary path: extract a beta app, promote
      a helper/API contract, keep experimenting, or reshape the kernel/service
      boundary.
- [ ] Follow-up tickets are created for the selected path and for any rejected
      high-value alternatives worth revisiting.
- [ ] `bun run qa` and `bun run ci` exit 0.

## Notes

Extraction requires repeated pressure. A database inside the beta interface is
not evidence that `memory-engine` should own all persistence; it is evidence
that the beta product needs durable state for dogfood learning sessions.

Likewise, successful exercises are not automatically evidence for kernel-owned
exercise generation. Promote only the stable substrate proven across the beta
ladder, such as progression metadata, queue behavior, grading contracts, or
activity DTOs that remain provider-, UI-, and persistence-neutral.
