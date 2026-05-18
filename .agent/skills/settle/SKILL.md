---
name: settle
description: |
  Polish a memory-engine branch until it is lean, green, reviewed, and ship-ready. Stops before merge and archival. Trigger: /settle, /pr-polish.
argument-hint: "[branch|PR]"
---

# /settle

Settle takes a branch that already has code and makes it merge-ready. It does not merge, archive tickets, or reflect; /ship owns the final mile.

Loop: /ci, /code-review, /refactor, /qa, then one final diff read. The loop exits only when `bun run ci` is green, blocking review findings are fixed, unnecessary complexity is removed, and ticket-required proof evidence is present or explicitly marked missing.

Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`). Before declaring ship-ready, verify the branch has a resolvable backlog ID through branch name, PR body, or trailers. If the repo's detector cannot connect the work to `backlog.d/`, do not hand off to /ship as ready.
