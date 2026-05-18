---
name: code-review
description: |
  Repo-specific review workflow for memory-engine. Use for PR, branch, or diff review against the shaped ticket, kernel invariants, tests, dogfood evidence, and shaped proof evidence. Trigger: /code-review, /review, /critique.
argument-hint: "[branch|diff|files]"
---

# /code-review

Review `memory-engine`, a pure TypeScript learning kernel for scheduling, grading, progression, queue selection, adapter contracts, and test fixtures. Review the diff against `master` by default. Findings lead; summaries are secondary.

Read `.spellbook/repo-brief.md`, `AGENTS.md`, `CLAUDE.md`, the active `backlog.d/` ticket, and touched source/tests. If feature work has no shaped ticket, that is a blocking process finding.

## Review Lenses

- Pure core boundary: no framework, storage, network, filesystem, logging, auth, UI, product choreography, or vendor SDK imports in `src/`.
- `ScheduleState`: snake_case ts-fsrs-native JSON shape, `state: 0 | 1 | 2 | 3`, `last_review: number | null`; no camelCase translation or hidden Date objects.
- One-envelope grader: no public verdict-then-rating two-step protocol; verdicts stay fixed.
- Exhaustiveness: any `Prompt` union change updates grader dispatch and `assertNever` tests.
- Product proof: local package tests are not product proof when the work claims beta/application behavior. Require the ticket's current dogfood, beta, or external proof oracle, or call out the gap. Historical Scry and Vault canaries are deprecated.
- Strict TS/Biome: no `any`, non-null assertions, `@ts-ignore`, unused imports, or value imports used only as types.
- Tests mock only true external boundaries; do not mock repo-owned pure modules or `ts-fsrs`.

## Verification

Delivery-ready review requires `bun run ci`. `bun run ci:local` is inner-loop evidence only. For any changed executable path, name the exact command or Dagger gate that exercised it.

## Output

Return findings first, ordered by severity, with file/line references. If there are no findings, say so and list residual risk, especially any ticket-required proof oracle not run.
