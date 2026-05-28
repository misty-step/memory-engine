---
shaping: true
ticket: 21-cli-review-loop-dogfood
slice: 5
status: shipped
priority: high
estimate: M
depends_on: [18-modular-api-surface, 19-service-boundary-failure-semantics]
oracles:
  - bun run ci
  - bun test experiments/cli-review/cli-review.test.ts
  - bun run experiments:cli-review
  - test -f docs/dogfood/cli-review.md
---

# CLI review loop dogfood - first experimental client

## Goal

Build the first repo-local experimental client that consumes the modular API and
service boundary from outside `src/`: a thin Bun CLI review loop over a small
fixture, with executable evidence of attempt recording, grading, scheduling,
and next-queue selection.

## Non-Goals

- No production CLI distribution.
- No auth, user profiles, billing, analytics, streaks, or XP.
- No durable database; a file or in-memory store is enough if the contract is
  explicit.
- No content parser beyond a tiny fixture adapter owned by the experiment.
- No package export for the service boundary.

## Oracle

- [ ] `experiments/cli-review/` contains a CLI client that imports public API
      subpaths where possible and imports the repo-local service boundary only
      as an experiment boundary.
- [ ] `bun test experiments/cli-review/cli-review.test.ts` proves one review
      loop records an attempt, applies a review, updates schedule state, and
      selects the next queue candidate.
- [ ] `bun run experiments:cli-review` executes the same fixture non-
      interactively and prints a compact receipt.
- [ ] `docs/dogfood/cli-review.md` records commands run, fixture used, service
      commands exercised, and what stayed outside `src/`.
- [ ] `bun run ci` exits 0.

## Notes

This is not a product. It is an API pressure test. If the CLI needs a reusable
helper, first ask whether that helper belongs in `testkit`, the service
boundary, or the future extracted client app.

## Study

### Problem Diamond

User outcome: dogfood should reveal whether the API is ergonomic enough for
real learning-client work before building heavier interfaces.

Falsifying case: the CLI has to reach into internal files or duplicate service
logic to complete one review loop.

### Alternatives

CLI first is selected because it is fast, executable, and avoids UI complexity.

A local web shell first is deferred because it would mix API feedback with UX
and frontend decisions too early.
