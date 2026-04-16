---
shaping: true
ticket: 05-vault-canary
slice: 1
status: blocked
priority: high
estimate: M
depends_on: [02-fsrs-scheduler, 03-deterministic-grader]
oracles:
  - bun run ci
  - (cd /Users/phaedrus/Documents/daybook/tools/vault-srs && git switch memory-engine-canary && bun test)
---

# Vault SRS canary branch — slice-1 acceptance gate

## Goal

On a `memory-engine-canary` branch of Vault SRS, replace Vault's local
grading and FSRS-scheduling internals with imports from memory-engine
(via local path dependency), and keep every existing Vault test green
without modification. This is the slice-1 acceptance gate — if the
canary is red, slice 1 is not done.

## Non-Goals

- Do NOT modify Vault's test files to accommodate the kernel. If tests
  fail, the kernel is wrong — fix the kernel in a separate commit.
- Do NOT replace Vault's `short-answer-rubric` path. It stays on Vault's
  current code until slice 3 merges.
- Do NOT merge the canary branch to Vault's master. Land as draft PR only.
- Do NOT publish memory-engine to npm. Use `file:` path dependency or a
  Bun workspace reference.
- Do NOT remove Vault's `src/grading.ts` or FSRS wrapper — keep them as
  a fallback path until slice 2 migration lands. This canary is a
  proof-of-concept branch, not a migration.

## Oracle

- [ ] `cd /Users/phaedrus/Documents/daybook/tools/vault-srs && git switch
      memory-engine-canary && bun test` exits 0. Every Vault test that
      was green on `master` stays green on this branch.
- [ ] Vault's `src/grading.ts` imports `Grader` (or its `grade` helper)
      from `memory-engine` and routes the four deterministic
      `ItemType`s (`mcq`, `boolean`, `cloze-exact`, `short-answer-exact`)
      through it. `short-answer-rubric` stays on existing code.
- [ ] Vault's FSRS scheduling calls `next()` from `memory-engine` instead
      of its local `ts-fsrs` wrapper (if any dedicated wrapper exists;
      otherwise, the raw ts-fsrs calls in review application code move
      behind `next()`).
- [ ] `package.json` of Vault contains `"memory-engine": "file:../../../
      Development/memory-engine"` (path from vault-srs to memory-engine
      root).
- [ ] `bun run ci` in memory-engine still exits 0 after any fixture/impl
      tweaks made during canary iteration.

## Notes

- Two repos are touched. Keep commits per-repo and push each branch
  separately. Reference the canary branch in the memory-engine PR
  description.
- Branch name in Vault: `memory-engine-canary`.
- Expected iteration loop: run Vault tests → fixture divergence → diff
  Vault's grader behavior vs memory-engine's behavior → pick one:
  - If Vault's behavior is correct and memory-engine drifted: fix
    memory-engine (add fixture, fix impl, run oracle 03 tests).
  - If Vault's fixture encoded a bug: note in canary PR description and
    update the Vault fixture. Do this only if the bug is clearly a bug
    (e.g., relying on a Levenshtein threshold that was wrong in Vault).
- Once the canary is green, slice 1 is DONE. Stop here. Do not start
  migrating other Vault modules or other consumer apps — slice 2 and 3
  handle that.
- This ticket does NOT include any schema migration for Vault's database.
  `ScheduleState` is JSON-compatible with Vault's existing
  `FsrsCardState` (both are ts-fsrs `Card` shape).

## Risks

- ts-fsrs version skew: memory-engine pins 5.3.2 (matches Vault at audit
  time). If Vault's pin has drifted, align to memory-engine's pin on the
  canary branch first.
- Bun workspace vs `file:` dependency: `file:` copies into node_modules
  and requires re-install on every memory-engine change. Bun workspace
  uses a symlink but requires a root-level workspace declaration. Start
  with `file:`; upgrade to workspace if iteration friction bites.
