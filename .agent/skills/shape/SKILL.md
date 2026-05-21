---
name: shape
description: |
  Shape memory-engine work into a repo-specific context packet and backlog ticket before implementation. Use when a feature, boundary change, beta/application adoption step, or proof oracle needs definition before code. Trigger: /shape, /spec, /plan.
argument-hint: "[idea|ticket|slice]"
---

# /shape

Shape before implementation. In `memory-engine`, feature work starts from a shaped `backlog.d/` ticket; larger direction can also update `SLICE-*.md` or docs under `docs/research/`, `docs/beta/`, or `docs/dogfood/`.

Read `.spellbook/repo-brief.md`, `SPEC.md`, relevant `SLICE-*.md`, `exemplars.md`, nearby active and archived tickets, touched package exports, and current beta/dogfood evidence.

## Problem Diamond

Name the user outcome and the boundary pressure. Ask whether the behavior belongs in the pure kernel, service prototype, beta experiment, testkit fixture, adapter contract, or consumer app. Produce at least two approaches: the smallest stable shared contract and an application-owned probe that keeps `src/` narrower.

## Ticket Shape

Use `backlog.d/NN-slug.md` with frontmatter: `shaping`, `ticket`, `slice`, `status`, `priority`, `estimate`, `depends_on`, and `oracles`. Body sections: Goal, Non-Goals, Oracle, Notes, and Study when useful. Each ticket must be small enough for one branch and one PR.

Oracles must be executable: focused `bun test ...`, `bun run qa`, `bun run ci`, beta/dogfood proof commands, and docs existence checks when documentation is part of the behavior. Prose-only proof is not enough.

## Invariants

Core stays pure. `ScheduleState`, `ReviewUnitId`, prompt/grader exhaustiveness, one-envelope grading, verdict vocabulary, and runtime dependency discipline remain load-bearing.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
