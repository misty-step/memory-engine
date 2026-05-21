---
name: ship
description: |
  Final mile for memory-engine: verify merge-ready branch, archive backlog tickets, preserve closure trailers, merge, trace, and run reflection. Trigger: /ship.
argument-hint: "[branch-or-pr]"
---

# /ship

Ship assumes `/settle` has left the branch green and reviewed. It does not replace CI, code review, refactor, or QA.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

Before merge, source `scripts/lib/backlog.sh`, resolve closing IDs from `Closes-backlog:` or `Ships-backlog:`, and move each matching `backlog.d/NN-*.md` into `backlog.d/_done/` with `backlog_archive` unless already archived. Preserve the closing trailers in the landed commit or PR record so `/groom` can detect closure later. `Refs-backlog:` is reference-only.

Verify `bun run ci` evidence and any ticket-named proof oracles. After merge, verify active tickets were archived. Write or verify a `/trace final` record when transcript/evidence refs are available, then run `/reflect` with bounded scope: branch, merged SHA, closing IDs, and proof evidence. Harness edits from reflection go to a review branch, not directly to `master`.
