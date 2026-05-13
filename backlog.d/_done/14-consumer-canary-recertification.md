---
shaping: true
ticket: 14-consumer-canary-recertification
slice: 4
status: shipped
priority: high
estimate: M
depends_on: [10-scry-canary, 13-vault-rubric-canary]
oracles:
  - bun run ci
  - (cd /Users/phaedrus/Development/scry && git switch memory-engine-canary && ASDF_NODEJS_VERSION=22.22.0 corepack pnpm@10.12.1 tsc --noEmit && ASDF_NODEJS_VERSION=22.22.0 corepack pnpm@10.12.1 exec vitest --run tests/convex/memory-engine-adapter.test.ts tests/convex/fsrs.test.ts convex/fsrs/conceptScheduler.test.ts)
  - (cd /Users/phaedrus/Documents/daybook && git worktree add /tmp/vault-srs-memory-engine-rubric-canary memory-engine-rubric-canary && cd /tmp/vault-srs-memory-engine-rubric-canary && bun test && cd /Users/phaedrus/Documents/daybook && git worktree remove /tmp/vault-srs-memory-engine-rubric-canary)
---

# Consumer canary recertification — adoption decision gate

## Goal

Re-run the Scry and Vault canary branches against the current memory-engine
package, then decide whether each branch is ready to merge, needs a small
consumer update, or reveals a real kernel contract gap that deserves a shaped
follow-up ticket.

## Non-Goals

- No new memory-engine primitives.
- No package split.
- No additional consumer migrations beyond Scry and Vault.
- No dashboard or manual-only verification path.

## Oracle

- [x] `bun run ci` exits 0 in memory-engine.
- [x] Scry's `memory-engine-canary` oracle exits 0 from a clean Scry worktree.
- [x] Vault's `memory-engine-rubric-canary` oracle exits 0 from a clean Vault
      worktree.
- [x] The adoption decision for each canary is recorded in this ticket or a
      follow-up closure note before the ticket is archived.

## Notes

- 2026-05-13: Scry passed with `ASDF_NODEJS_VERSION=22.22.0` because the
  repo's default asdf Node was `22.15.0` and `promptfoo` requires
  `^20.20.0 || >=22.22.0`.
- 2026-05-13: Vault passed from `/tmp/vault-srs-memory-engine-rubric-canary`,
  a temporary clean worktree for `memory-engine-rubric-canary`.
- Superseding decision on 2026-05-13: Scry and the Vault FSRS app are
  decommission targets. These canaries remain contract evidence, but their
  branches are not the adoption path. The next work is a dedicated service and
  interface prototype in this repo.
- The Vault repo currently carries unrelated daybook work in its main worktree;
  use a clean worktree before switching to `memory-engine-rubric-canary`.
- Treat canary failures as contract evidence. Fix memory-engine only when the
  failure shows a shared kernel boundary problem, not consumer-local drift.
