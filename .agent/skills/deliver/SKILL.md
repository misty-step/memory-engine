---
name: deliver
description: |
  Take one shaped memory-engine backlog ticket to merge-ready code by composing /shape, /implement, /ci, /code-review, /refactor, and /qa. Stops before push or merge. Trigger: /deliver.
argument-hint: "[backlog.d/NN-slug.md]"
---

# /deliver

One active `backlog.d/` ticket becomes merge-ready code. Delivered is not shipped: this skill does not push, merge, archive, deploy, or reflect.

If no ticket is specified, select the highest-priority ready active ticket by reading `backlog.d/` and dependencies. If the tracker is contradictory, run `/groom` first instead of guessing.

## Loop

1. Confirm the ticket Goal, Non-Goals, and executable Oracle.
2. Run `/shape` first if the work is unshaped or too broad.
3. Run `/implement` with TDD and `Refs-backlog: NN` recorded.
4. Run `/ci` until `bun run ci` is green.
5. Run `/code-review` and fix blocking findings.
6. Run `/refactor` only for simplification inside ticket scope.
7. Run `/qa` for package surfaces and ticket-named dogfood/beta/external proof.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

Output ticket ID, branch, changed surfaces, commands run, proof evidence, and what remains for `/yeet`, `/settle`, or `/ship`.
