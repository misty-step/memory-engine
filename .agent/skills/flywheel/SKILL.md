---
name: flywheel
description: |
  Compose memory-engine's outer loop: pick, shape, implement, yeet, settle, ship, monitor, then stop because this package has no production release surface. Trigger: /flywheel.
argument-hint: "[--max-cycles N]"
---

# /flywheel

Run the product loop without absorbing leaf logic: pick a backlog item -> `/shape` if needed -> `/implement` -> `/yeet` -> `/settle` -> `/ship` -> `/monitor` -> loop or stop.

There is no release leaf today because this repo has no production release surface. `/monitor` still runs as a repository signal watch over CI, QA, dogfood/beta evidence, benchmark drift, and backlog lifecycle contradictions.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

`/flywheel` does not archive tickets or invoke `/reflect` directly. `/ship` owns closure, archival, merge, final trace, and reflection. If no active ticket exists, run `/groom` rather than inventing work.

Each cycle must preserve pure `src/`, executable oracles, `bun run ci`, and ticket-required dogfood/beta/external proof.
