---
name: qa
description: |
  Run library QA for memory-engine: executable oracles, package-export smoke checks, fixture/testkit validation, dogfood lanes, and shaped proof when touched. Trigger: /qa.
argument-hint: "[ticket|surface|proof]"
---

# /qa

This is a library package, not a deployed browser app. QA means exercising the package surfaces, repo-local dogfood clients, and any current proof oracle named by the ticket.

Start from the active ticket and affected surface: `memory-engine`, `memory-engine/testkit`, `memory-engine/adapters`, scheduler, grader, progression, queue, Dagger, or harness docs. If there is no ticket for feature work, stop and shape/groom first.

## QA Paths

- Kernel smoke: `bun test tests/smoke.test.ts`.
- Exported package behavior: tests under `tests/testkit/`, `tests/adapters/`, and focused source suites.
- Scheduler/grader/progression/queue: run the relevant focused `bun test tests/<surface>/` command.
- Whole package confidence: `bun run ci:local` while iterating and `bun run ci` before handoff.
- Product proof: run current dogfood/beta/external proof commands only when the ticket explicitly lists them. Historical Scry and Vault canaries are deprecated and are not required harness proof.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Evidence

A QA pass names exact commands, package surfaces exercised, fixture corpus exercised, dogfood lanes, and any required current proof oracle. A QA pass without `bun run ci` is not delivery evidence. A QA pass without a ticket-required proof oracle must call that path unverified.
