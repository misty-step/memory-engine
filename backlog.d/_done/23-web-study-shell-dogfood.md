---
shaping: true
ticket: 23-web-study-shell-dogfood
slice: 5
status: shipped
priority: medium
estimate: M
depends_on: [21-cli-review-loop-dogfood, 22-content-normalization-probe]
oracles:
  - bun run ci
  - bun test experiments/web-shell/web-shell.test.ts
  - test -f docs/dogfood/web-shell.md
---

# Web study shell dogfood - local interface experiment

## Goal

Build a local, repo-contained web study shell that consumes the API through the
dogfood fixture path and reveals interaction pressure around answer submission,
reveal, queue transitions, and review-state visibility.

## Non-Goals

- No hosted product.
- No auth, billing, telemetry, persistence service, or deployment pipeline.
- No UI framework dependency unless the ticket branch justifies it.
- No generic workflow editor.
- No changes to `src/` for UI convenience.

## Oracle

- [x] `experiments/web-shell/` renders a local study loop over the dogfood
      fixture without importing private `src` internals.
- [x] `bun test experiments/web-shell/web-shell.test.ts` exercises the core
      interaction flow through code-level tests.
- [x] `docs/dogfood/web-shell.md` records the interface pressure, API friction,
      and extraction recommendation.
- [x] `bun run ci` exits 0.

## Notes

This ticket should happen after CLI and import dogfood. The web shell should
test interaction shape, not compensate for an unstable API.
