---
name: refactor
description: |
  Simplify memory-engine without widening scope: pure modules, public contracts, test fixtures, and Dagger/harness edges. Trigger: /refactor.
argument-hint: "[--base master] [--scope path] [--report-only|--apply]"
---

# /refactor

Reduce states and clarify invariants without changing the product promise. On feature branches, compare against `master` and simplify the diff unless the ticket explicitly scopes broader cleanup. On `master`, report opportunities and shape a ticket before editing.

Good targets: duplicated fixture setup, unclear type boundaries, shallow wrappers, confusing queue/progression conditionals, stale docs next to touched behavior, Dagger duplication, and harness drift. Bad targets: moving app-owned policy into `src/`, speculative package splits, hidden `ScheduleState` translation, public export churn without proof, or refactoring untouched working code for style.

Preserve pure core, behavior tests, package exports, fixture contracts, and dogfood proof. Use `ousterhout` for module-depth disputes, `grug` or `carmack` for scope cuts, and `beck`/`cooper` for test-design drift when needed.

Run focused tests for touched behavior, then `bun run ci`. If exports, fixtures, service, dogfood, or beta paths moved, run `/qa` and the ticket proof.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
