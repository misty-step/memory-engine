---
name: yeet
description: |
  Turn memory-engine worktree changes into focused conventional commits and push a reviewable branch. Trigger: /yeet, /ship-local.
argument-hint: "[--dry-run|--single-commit|--no-push]"
---

# /yeet

`/yeet` is the commit-and-push judgment layer. It reads the worktree, separates intentional ticket work from debris, stages only what belongs, commits in reviewable chunks, and pushes branch state when appropriate.

Branch from `master`; use `cx/...` unless the operator specified another branch. Commit subjects use Conventional Commits. Bodies explain why and include structured backlog references: `Refs-backlog: NN` for in-progress work, `Closes-backlog: NN` or `Ships-backlog: NN` only when the ticket is being closed or `/ship` will preserve that closure.

Do not commit unrelated user changes. Do not hide red gates. If package contracts, fixtures, dogfood, beta paths, or harness behavior moved, include exact oracle evidence in the commit or PR body.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
