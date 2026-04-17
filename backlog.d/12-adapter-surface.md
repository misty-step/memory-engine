---
shaping: true
ticket: 12-adapter-surface
slice: 3
status: ready
priority: high
estimate: S
depends_on: [11-rubric-grading-contract]
oracles:
  - bun run ci
  - bun test tests/adapters/rubric-adapter.test.ts
  - bun run typecheck
---

# Vendor-neutral rubric adapter surface

## Goal

Publish a vendor-neutral rubric adapter interface and test doubles under a
separate memory-engine surface, while keeping the repo as a single package.

## Non-Goals

- No OpenAI/Anthropic client in this repo.
- No storage adapters.
- No monorepo conversion.
- No streaming interface.

## Oracle

- [ ] `bun test tests/adapters/rubric-adapter.test.ts` passes.
- [ ] `package.json` exports a dedicated adapter subpath without breaking
      consumers that only import `memory-engine` or `memory-engine/testkit`.
- [ ] `bun run typecheck` and `bun run ci` exit 0.

## Notes

- The reference implementation can be a no-op or canned test adapter only. Real
  model calls stay in consumers.
- Adapter failure should throw. Do not map transport/model errors to
  `verdict: 'revealed'`.
- Study:
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/rubric-grader.ts`
  - `package.json`
  - `src/index.ts`
