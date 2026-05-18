---
shaping: true
ticket: 25-repeatable-api-qa-system
slice: 5
status: ready
priority: high
estimate: M
depends_on: [18-modular-api-surface, 20-evals-and-benchmarks-baseline, 21-cli-review-loop-dogfood, 22-content-normalization-probe, 23-web-study-shell-dogfood]
oracles:
  - bun run qa:local
  - bun run qa
  - bun run ci
  - test -f docs/qa/system.md
  - test -f docs/qa/quality-register.md
---

# Repeatable API QA system

## Goal

Build a consistent, repeatable, executable QA system for `memory-engine` that
exercises the public API and actual learning-kernel behavior before handoff.
The system should go beyond bug finding by identifying quality-improvement
opportunities across API ergonomics, semantic coverage, fixtures, evals,
benchmarks, dogfood evidence, and consumer proof.

## Non-Goals

- No new runtime dependencies.
- No dashboard-only QA process.
- No lowering CI, coverage, Biome, typecheck, or secret-scan gates.
- No consumer migration or package extraction.
- No browser-only QA as the primary proof path.

## Oracle

- [ ] `bun run qa:local` runs the repeatable local QA harness and exits 0.
- [ ] `bun run qa` runs the full QA harness, including the canonical CI gate,
      and exits 0.
- [ ] `docs/qa/system.md` documents the QA lanes, quality model, commands, and
      when to use local versus full QA.
- [ ] `docs/qa/quality-register.md` records quality-improvement findings that
      are not necessarily bugs.
- [ ] `bun run ci` exits 0.

## Notes

Primary QA evidence should exercise package surfaces and learning semantics:
exports, scheduler, grader, progression, queue, service composition, testkit
fixtures, eval corpus, benchmarks, and dogfood clients.

The harness should be executable by an agent from a clean shell and should
produce lane-level pass/fail evidence. Benchmarks may remain non-gating, but the
command should still run them and preserve the receipt in output.
