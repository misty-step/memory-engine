---
name: shape
description: |
  Shape memory-engine work into a repo-specific context packet and backlog ticket before implementation. Use when a feature, boundary change, consumer adoption step, or canary needs definition before code. Trigger: /shape, /spec, /plan.
argument-hint: "[idea|ticket|slice]"
---

# /shape

Shape first. In `memory-engine`, feature work starts only after there is a shaped `backlog.d/` ticket; slice-sized work also gets a context packet in the local `SLICE-*.md` style. This protects the pure kernel boundary from product-specific drift.

Read `.spellbook/repo-brief.md`, `SPEC.md`, current `SLICE-*.md`, `exemplars.md`, nearby tickets in `backlog.d/` and `backlog.d/_done/`, and any consumer files named by the work. If the user asks for implementation with no ticket, stop and shape.

## Problem Diamond

Name the user outcome and boundary pressure before proposing code. Ask which consumer proves the behavior is shared; whether it is a stable primitive or app-owned pedagogy/session choreography; whether a shallow canonical input/output is enough; and what canary would falsify the boundary. Produce at least two approaches: a minimal kernel change and a stricter consumer-owned alternative.

## Output

Slice packets use: frontmatter, Goal, Non-Goals, Constraints / Invariants, Authority Order, Repo Anchors, Prior Art or Exemplar Techniques, executable Oracle, Implementation Sequence, Risk + Rollout.

Tickets use `backlog.d/NN-name.md` with frontmatter fields `shaping`, `ticket`, `slice`, `status: ready`, `priority`, `estimate`, `depends_on`, and `oracles`, followed by Goal, Non-Goals, Oracle, Notes, and Study. Tickets must be atomic enough for one branch and one PR.

## Required Invariants

Core stays pure; consumers own persistence, time, identity mapping, sessions, parsing, SDKs, and pedagogy. `ScheduleState` remains ts-fsrs-native. `ReviewUnitId` remains opaque. `Grader.grade()` keeps one result envelope. Runtime dependencies require shaped scope. `bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Oracles

Every shaped item needs commands, not prose. Include focused Bun tests, `bun run ci`, and any consumer canary needed to prove the boundary. If no canary is needed, state why fixtures or testkit contracts are enough.
