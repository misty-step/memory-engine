---
name: settle
description: |
  Polish a memory-engine branch until it is lean, green, reviewed, and ship-ready. Stops before merge and archival. Trigger: /settle, /pr-polish.
argument-hint: "[branch|PR]"
---

# /settle

Settle takes a branch that already has code and makes it ship-ready. It does not merge, archive tickets, write final trace, deploy, or reflect; `/ship` owns the final mile.

Loop: `/ci` -> `/code-review` -> `/refactor` -> `/qa` -> final diff read. Exit only when `bun run ci` is green, blocking review findings are fixed, unnecessary complexity is removed, and ticket-required proof evidence is present or explicitly marked missing.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

Before declaring ship-ready, verify the branch has a resolvable backlog ID through branch name, PR body, or trailers. If `/ship` cannot connect the work to `backlog.d/`, the branch is not settled.
