---
name: flywheel
description: |
  Compose memory-engine's outer loop: pick, shape, implement, yeet, settle, ship, then stop because this package has no deploy/monitor target. Trigger: /flywheel.
argument-hint: "[--max-cycles N]"
---

# /flywheel

Run the product loop without absorbing leaf logic. For `memory-engine`: pick a backlog item, /shape if needed, /implement, /yeet, /settle, /ship, then stop. There is no /deploy or /monitor leaf today because the repo has no deploy target or health signal surface.

Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`). /flywheel does not archive tickets or invoke /reflect directly. /ship owns closure, archival, merge, and reflection.

Each cycle must preserve kernel boundaries: pure `src/`, executable oracles, `bun run ci`, and ticket-required dogfood/beta/external proof when a shared contract moves. If no active ticket exists, run /groom rather than inventing work.
