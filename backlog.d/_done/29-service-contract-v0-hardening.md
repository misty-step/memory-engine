---
shaping: true
ticket: 29-service-contract-v0-hardening
slice: 6
status: ready
priority: medium
estimate: M
depends_on: [28-mobile-beta-study-interface]
oracles:
  - bun run ci
  - cargo test -p memory-engine-service
  - cargo test -p memory-engine-study -p memory-engine-beta-app
  - test -f docs/beta/service-contract-v0.md
---

# Service contract v0 hardening - DTOs, reveal, and failures

## Goal

Harden the repo-local service boundary after the beta interface has produced
real pressure. Decide what remains client-owned and what should become stable
service behavior.

## Non-Goals

- No public package export unless the beta evidence requires it.
- No hosted service.
- No persistence implementation in `src`.
- No API promotion based on one interface alone.

## Oracle

- [ ] Service-facing tests use public package subpaths where possible instead
      of private `src` imports.
- [ ] A documented service contract names the command lifecycle, learner-facing
      DTO decision, reveal policy, activity-kind metadata decision, and typed
      failure envelope.
- [ ] Reveal is explicitly classified as either display-only UI state or a
      review event with scheduling consequences, with tests proving the chosen
      behavior.
- [ ] `grade/apply-review` has a documented retry/idempotency and
      compare-and-apply story for durable stores.
- [ ] A shared validating in-memory store/test harness removes duplicated
      ad-hoc `MemoryServiceStore` scaffolding across service tests and
      experiments.
- [ ] Beta-study tests still pass through the hardened service boundary.
- [ ] `docs/beta/service-contract-v0.md` records what is stable, what remains
      private, and what evidence would justify extraction.
- [ ] `bun run ci` exits 0.

## Notes

- The current web shell shows pressure for a compact review-state projection
  and reveal semantics, but one client is insufficient for promotion.
- Keep the module deep: stable commands should hide engine-shaped details from
  product UI without hiding schedule/review evidence from tests.
- Treat activity kind, ladder stage, variant group, worked solution, and
  exercise-specific feedback as beta pressure until the graduated ladder proves
  a provider-neutral service shape.
