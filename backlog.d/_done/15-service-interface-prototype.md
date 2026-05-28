---
shaping: true
ticket: 15-service-interface-prototype
slice: 4
status: shipped
priority: high
estimate: M
depends_on: [14-consumer-canary-recertification]
oracles:
  - bun run ci
  - cargo test -p memory-engine-service
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

- [x] `crates/memory-engine-service/tests/command_contract.rs` pins the service command
      envelope for at least record-attempt, grade/apply-review, and next-queue
      behavior.
- [x] `crates/memory-engine-service/tests/command_contract.rs` proves the prototype keeps
      storage concerns outside the pure kernel.
- [x] `bun run ci` exits 0.
- [x] The implementation notes name what should stay in this repo versus what
      should move when the service/app is extracted.

## Notes

- Strategic correction on 2026-05-13: Scry and Vault FSRS are decommission
  targets, not consumers to merge.
- Use the historical Scry and Vault canaries as boundary evidence only.
- Prefer a thin prototype surface over a generic workflow engine. The goal is to
  discover the dedicated service shape, not to invent a broad orchestration DSL.
- 2026-05-14: Verified on `cx/deliver-15-service-interface-prototype-verified`.
  The former TypeScript prototype was later deleted during the Rust migration
  after `cargo test -p memory-engine-service` covered the command boundary.
