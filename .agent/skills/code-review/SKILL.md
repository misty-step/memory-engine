---
name: code-review
description: |
  Repo-specific review workflow for memory-engine. Use for PR, branch, or diff review against the shaped ticket, kernel invariants, tests, dogfood evidence, and shaped proof evidence. Trigger: /code-review, /review, /critique.
argument-hint: "[branch|diff|files]"
---

# /code-review

Review `memory-engine` against `master` by default. Findings lead, ordered by severity, with file/line references. Summaries are secondary.

Read `.spellbook/repo-brief.md`, `AGENTS.md`, the active ticket, touched source/tests/docs, and proof receipts. Feature work without a shaped active ticket is a blocking process finding.

## Review Lenses

- Pure core boundary: no framework, storage, network, filesystem, logging, auth, UI, analytics, product choreography, or provider SDK imports in `src/`.
- `ScheduleState`: snake_case `ts-fsrs` shape with `state: 0 | 1 | 2 | 3` and `last_review: number | null`.
- `ReviewUnitId` remains opaque.
- Prompt union changes update grader dispatch and `assertNever` tests.
- `Grader.grade()` remains one envelope with populated `rating`; verdicts stay fixed.
- Tests mock external boundaries only; do not mock repo-owned pure collaborators.
- Strict TS/Biome: no `any`, non-null assertions, `@ts-ignore`, stale value imports, or unused code.
- Product claims require ticket-named dogfood, beta, or external proof. Historical Scry/Vault canaries are deprecated.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

If no issues are found, say so and name residual risks, especially unrun ticket proof.
