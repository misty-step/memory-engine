---
shaping: true
ticket: 20-evals-and-benchmarks-baseline
slice: 5
status: ready
priority: high
estimate: M
depends_on: [18-modular-api-surface]
oracles:
  - bun run ci
  - bun test tests/evals/regression-corpus.test.ts
  - bun run bench
---

# Evals and benchmarks baseline - regression corpus and speed receipts

## Goal

Add a small, executable eval and benchmark baseline so API changes can be judged
by behavioral quality and performance, not just typecheck and unit-test
coverage.

## Non-Goals

- No model-provider calls.
- No flaky timing thresholds in CI.
- No broad simulation framework beyond the first useful baseline.
- No product analytics or learner telemetry.
- No runtime dependency unless it is explicitly justified in the ticket branch.

## Oracle

- [ ] `tests/evals/regression-corpus.test.ts` runs named grading, scheduling,
      progression, and queue scenarios from stable fixture data and emits clear
      failure diffs.
- [ ] A `bench` script runs deterministic benchmark cases for grading,
      scheduling, queue selection, and service command composition.
- [ ] Benchmark output records operation counts and elapsed time without
      enforcing brittle machine-specific thresholds.
- [ ] README or `docs/evals.md` explains how to add eval cases and how to read
      benchmark receipts.
- [ ] `bun run ci` exits 0.

## Notes

The first eval suite should reuse existing fixture corpora where possible. Add
new fixture shape only when the eval needs an end-to-end scenario the current
testkit cannot express.

## Study

### Problem Diamond

User outcome: a world-class learning API needs quality gates that catch semantic
drift in learning behavior and obvious performance regressions before clients
dogfood them.

Falsifying case: a queue heuristic refactor passes local unit tests but changes
the selected candidate for a representative learning loop.

### Alternatives

Behavioral eval tests plus non-gating benchmarks are selected for the baseline.

CI-enforced performance thresholds are deferred until enough benchmark history
exists to set meaningful budgets.
