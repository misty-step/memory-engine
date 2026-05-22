---
shaping: true
ticket: 19-service-boundary-failure-semantics
slice: 5
status: shipped
priority: high
estimate: M
depends_on: [15-service-interface-prototype, 17-service-scenario-fixtures]
oracles:
  - bun run ci
  - bun test tests/service/failure-semantics.test.ts
  - bun test tests/service/interface-contract.test.ts
---

# Service boundary failure semantics - safe command contracts

## Goal

Pin the failure, validation, and atomicity expectations for the repo-local
service command boundary before any experimental client treats it as a stable
integration point.

## Non-Goals

- No durable database implementation.
- No retry queue, idempotency key store, or distributed transaction machinery.
- No HTTP/RPC transport.
- No production error taxonomy beyond the contracts needed by callers and fake
  stores.
- No changes to pure `src/` kernel behavior unless a test exposes a real shared
  contract gap.

## Oracle

- [ ] `tests/service/failure-semantics.test.ts` proves `record-attempt` does
      not report success when `MemoryServiceStore.recordAttempt` rejects.
- [ ] The same suite proves `grade/apply-review` propagates read/apply failures
      without silently swallowing or remapping them to grading verdicts.
- [ ] The same suite proves validation belongs at the store boundary by using a
      realistic fake that rejects unknown review units, blank answers, invalid
      response times, and mismatched applied review units.
- [ ] `docs/service-prototype.md` documents the atomic write expectation for
      `applyReview` and what remains app-owned.
- [ ] `bun run ci` exits 0.

## Notes

This ticket hardens the command contract; it should not turn the prototype into
a production service. The useful invariant is that clients can rely on honest
success/failure signals while storage remains outside the kernel.

## Study

### Problem Diamond

User outcome: dogfood clients should be able to trust service command results
without corrupting schedules or attempts when a store fails.

Falsifying case: a partial review application looks successful to the client
after the schedule write failed.

### Alternatives

Minimal fake-store tests are selected because they pin the boundary while
keeping persistence out of this repo.

Adding a real SQLite or file store now is rejected because it would answer a
storage-product question before the API contract is stable.
