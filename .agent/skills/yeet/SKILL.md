---
name: yeet
description: |
  Turn memory-engine worktree changes into focused conventional commits and push a reviewable branch. Trigger: /yeet, /ship-local.
argument-hint: "[--dry-run|--single-commit|--no-push]"
---

# /yeet

Yeet is the commit-and-push judgment layer. It reads the worktree, separates intentional ticket work from debris, stages only what belongs, commits in reviewable chunks, and pushes.

Branch from `master`; use `cx/...` unless the operator specified another branch. Commit subjects use Conventional Commits. Bodies explain why and include structured backlog references: `Refs-backlog: NN` for in-progress work, `Closes-backlog: NN` or `Ships-backlog: NN` only when the ticket is archived or will be archived by /ship.

Do not commit unrelated user changes. Do not hide red gates. If package contracts moved, include the exact oracle and canary evidence in the commit or PR body.
