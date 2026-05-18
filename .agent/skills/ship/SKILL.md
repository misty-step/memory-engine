---
name: ship
description: |
  Final mile for memory-engine: verify merge-ready branch, archive backlog tickets, preserve closure trailers, merge, and run reflection. Trigger: /ship.
argument-hint: "[branch-or-pr]"
---

# /ship

Ship assumes /settle has left the branch green and reviewed. It does not replace CI or code review.

Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`). Before merge, source `scripts/lib/backlog.sh`, resolve closing IDs from `Closes-backlog:` or `Ships-backlog:`, and move each matching `backlog.d/NN-*.md` into `backlog.d/_done/` with `backlog_archive` unless already archived. Preserve the closing trailers into the landed commit or PR record so /groom can detect closure later. Use `Refs-backlog:` only for non-closing references.

Run or verify `bun run ci` evidence before merge. Verify any ticket-named current proof oracles. After merge, verify active tickets were archived, then run /reflect with bounded scope: branch, merged SHA, closing IDs, and proof evidence. Harness edits from reflection go to a review branch, not directly to `master`.
