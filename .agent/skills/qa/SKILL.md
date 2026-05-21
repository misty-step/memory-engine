---
name: qa
description: |
  Run library QA for memory-engine: executable oracles, package-export smoke checks, fixture/testkit validation, dogfood lanes, and shaped proof when touched. Trigger: /qa.
argument-hint: "[ticket|surface|proof]"
---

# /qa

QA for `memory-engine` means executable package and dogfood evidence, not a deployed-browser checklist. Start from the active ticket and affected surface: public exports, scheduler, grader, progression, queue, testkit, adapters, service prototype, dogfood clients, beta experiments, Dagger, or harness docs.

## Standard Paths

- Local QA: `bun run qa:local`.
- Full QA: `bun run qa`.
- Kernel smoke: `bun test tests/smoke.test.ts`.
- Package exports: `bun test tests/api/module-exports.test.ts tests/api/compatibility.test.ts`.
- Focused surfaces: `bun test tests/grader/`, `bun test tests/queue/`, `bun test tests/progression/`, `bun test tests/service/`, `bun test experiments/<name>/`.
- Canonical gate: `bun run ci`.

`bun run qa` exercises package exports, types/scheduler, grading, progression/queue, testkit/adapters, service prototype, regression corpus, dogfood experiments, coverage, benchmarks, and then the canonical Dagger gate.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Evidence

Report exact commands, pass/fail status, surfaces exercised, fixture corpus touched, dogfood/beta receipts, and ticket-required proof oracles. Historical Scry/Vault canaries are deprecated; do not claim them as current proof.
