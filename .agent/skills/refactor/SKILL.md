---
name: refactor
description: |
  Simplify memory-engine without widening scope: pure modules, public contracts, test fixtures, and Dagger/harness edges. Trigger: /refactor.
argument-hint: "[--base master] [--scope path] [--report-only|--apply]"
---

# /refactor

Refactor to reduce states and clarify invariants, not to invent architecture. In this repo, the danger signs are speculative package splits, service drift, pass-through wrappers around simple functions, hidden ScheduleState translation, and consumer-specific flags leaking into core.

On feature branches, compare against `master` and simplify only the diff unless the active ticket explicitly allows broader cleanup. On `master`, report opportunities and shape a ticket before editing.

## Targets

Good targets: duplicated fixture setup, unclear type boundaries, shallow wrappers, confusing queue/progression conditionals, stale docs next to touched behavior, and Dagger/harness duplication. Bad targets: changing behavior without oracle updates, moving app-owned policy into core, package splitting before a shaped need, or refactoring untouched working code because it looks different.

Keep tests behavior-focused and do not mock internal collaborators. Preserve `ScheduleState`, prompt/grader exhaustiveness, one-envelope grading, and pure `src/`.

## Verification

Run the focused tests for touched behavior, then `bun run ci`. If refactoring changes exports or fixtures, run /qa and any ticket-required current proof oracle.
