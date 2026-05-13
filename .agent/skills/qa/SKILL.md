---
name: qa
description: |
  Run library QA for memory-engine: executable oracles, package-export smoke checks, fixture/testkit validation, and consumer canary proof when touched. Trigger: /qa.
argument-hint: "[ticket|surface|canary]"
---

# /qa

This is a library package, not a browser app. QA means exercising the package surfaces and the consumer proof path, not walking UI routes.

Start from the active ticket and affected surface: `memory-engine`, `memory-engine/testkit`, `memory-engine/adapters`, scheduler, grader, progression, queue, Dagger, or harness docs. If there is no ticket for feature work, stop and shape/groom first.

## QA Paths

- Kernel smoke: `bun test tests/smoke.test.ts`.
- Exported package behavior: tests under `tests/testkit/`, `tests/adapters/`, and focused source suites.
- Scheduler/grader/progression/queue: run the relevant focused `bun test tests/<surface>/` command.
- Whole package confidence: `bun run ci:local` while iterating and `bun run ci` before handoff.
- Consumer proof: run Scry or Vault SRS canary commands only when the ticket touches the relevant shared contract or explicitly lists the canary oracle.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Evidence

A QA pass names exact commands, package surfaces exercised, fixture corpus exercised, and any consumer canary branch. A QA pass without `bun run ci` is not delivery evidence. A QA pass without a required canary must call that path unverified.
