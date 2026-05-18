---
name: deliver
description: |
  Take one shaped memory-engine backlog ticket to merge-ready code by composing /shape, /implement, /ci, /code-review, /refactor, and /qa. Stops before push or merge. Trigger: /deliver.
argument-hint: "[backlog.d/NN-slug.md]"
---

# /deliver

One active `backlog.d/` ticket becomes merge-ready code. Delivered is not shipped: this skill does not push, merge, archive, deploy, or reflect.

Read `.spellbook/repo-brief.md`, the ticket, relevant slice docs, and shipped precedents in `backlog.d/_done/`. If the ticket is missing or unshaped, run /shape first. Create or use one branch from `master` with `cx/...` naming unless directed otherwise.

## Loop

1. Confirm the ticket's Goal, Non-Goals, and executable Oracle.
2. Run /implement with TDD and `Refs-backlog: NN` recorded for later closure.
3. Run /ci until `bun run ci` is green.
4. Run /code-review and fix blocking findings.
5. Run /refactor only for simplification that preserves the ticket scope.
6. Run /qa for package surfaces and any named current proof oracles.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`). /deliver only prepares the branch; /ship performs archival and closing trailers.

## Output

Report ticket ID, branch, changed surfaces, commands run, proof evidence, and what remains for /yeet, /settle, or /ship.
