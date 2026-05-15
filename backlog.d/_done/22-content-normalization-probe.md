---
shaping: true
ticket: 22-content-normalization-probe
slice: 5
status: ready
priority: medium
estimate: M
depends_on: [21-cli-review-loop-dogfood]
oracles:
  - bun run ci
  - bun test experiments/import-probe/import-probe.test.ts
  - test -f docs/dogfood/import-probe.md
---

# Content normalization probe - authored material into API inputs

## Goal

Create a small experimental import probe that converts one intentionally tiny
authored fixture into canonical `Prompt`, `QueueCandidate`, and `ScheduleState`
inputs for the dogfood clients, without moving content parsing into the core
API.

## Non-Goals

- No general markdown parser.
- No product taxonomy standardization.
- No shared content authoring system.
- No changes to `src/` unless the probe exposes a missing primitive.
- No production migration from Ruminatio, Scry, Caesar, or Vault.

## Oracle

- [ ] `experiments/import-probe/` owns a tiny authored fixture and adapter.
- [ ] `bun test experiments/import-probe/import-probe.test.ts` proves the
      adapter produces canonical API inputs consumed by the existing service
      scenario or CLI dogfood loop.
- [ ] `docs/dogfood/import-probe.md` records which authored fields were
      essential, which stayed product-owned, and whether any API gap surfaced.
- [ ] `bun run ci` exits 0.

## Notes

The point is to discover input pressure, not to create a parser framework. If a
second client needs the same fixture shape, shape a testkit or extracted-client
follow-up then.
