---
shaping: true
ticket: 18-modular-api-surface
slice: 5
status: shipped
priority: high
estimate: M
depends_on: [15-service-interface-prototype, 17-service-scenario-fixtures]
oracles:
  - bun run ci
  - bun test tests/api/module-exports.test.ts
  - bun test tests/api/compatibility.test.ts
---

# Modular API surface - stable package entrypoints

## Goal

Turn the current barrel-first package into a modular API for building learning
and memorization applications: stable subpath exports for types, scheduling,
grading, progression, queue planning, adapters, and testkit, with executable
compatibility tests that lock the intended public surface.

## Non-Goals

- No physical monorepo split.
- No service export from `package.json`.
- No storage, HTTP, UI, auth, or content parser API.
- No breaking removal of existing root-barrel exports.
- No scheduler swap or `ScheduleState` shape change in this ticket.

## Oracle

- [ ] `package.json` exports additive module subpaths for the stable surfaces
      needed by client apps.
- [ ] `tests/api/module-exports.test.ts` imports each public subpath and proves
      the exported symbols are usable without importing the root barrel.
- [ ] `tests/api/compatibility.test.ts` locks root-barrel compatibility for
      existing consumers.
- [ ] README usage examples show the preferred modular imports.
- [ ] `bun run ci` exits 0.

## Notes

Keep the modules deep and boring. The API should expose cohesive capabilities,
not implementation files. The service prototype remains repo-local until
dogfood proves which command contract deserves extraction.

## Study

### Problem Diamond

User outcome: app builders should be able to discover and depend on the right
learning primitive without spelunking a giant root barrel or copying internal
paths.

Failure mode: adding subpaths that mirror the current file layout would freeze
implementation boundaries instead of API boundaries.

### Alternatives

Additive subpath exports are selected because they improve ergonomics without
breaking current consumers.

A physical `packages/*` split is deferred because versioning/runtime pressure
has not yet justified it.

Leaving the root barrel alone is insufficient because it keeps the API shallow
and makes dogfood clients depend on everything at once.
