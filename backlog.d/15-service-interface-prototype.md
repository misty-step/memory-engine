---
shaping: true
ticket: 15-service-interface-prototype
slice: 4
status: ready
priority: high
estimate: M
depends_on: [14-consumer-canary-recertification]
oracles:
  - bun run ci
  - bun test tests/service/interface-contract.test.ts
  - bun test tests/service/persistence-boundary.test.ts
---

# Service interface prototype — dedicated memory microservice

## Goal

Prototype the first focused memory-service interface inside this repo, using the
existing kernel as the domain core and producing executable contract tests that
can guide a later extraction into a separate service/application repository.

## Non-Goals

- No Scry adoption work.
- No Vault FSRS adoption work.
- No extraction to a new repository in this ticket.
- No framework/runtime imports in `src/`.
- No production auth, billing, hosting, or deployment surface.

## Oracle

- [ ] `tests/service/interface-contract.test.ts` pins the service command
      envelope for at least record-attempt, grade/apply-review, and next-queue
      behavior.
- [ ] `tests/service/persistence-boundary.test.ts` proves the prototype keeps
      storage concerns outside the pure kernel.
- [ ] `bun run ci` exits 0.
- [ ] The implementation notes name what should stay in this repo versus what
      should move when the service/app is extracted.

## Notes

- Strategic correction on 2026-05-13: Scry and Vault FSRS are decommission
  targets, not consumers to merge.
- Use the historical Scry and Vault canaries as boundary evidence only.
- Prefer a thin prototype surface over a generic workflow engine. The goal is to
  discover the dedicated service shape, not to invent a broad orchestration DSL.
