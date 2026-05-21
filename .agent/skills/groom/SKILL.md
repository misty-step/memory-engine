---
name: groom
description: |
  Maintain memory-engine's file-backed backlog, reconcile shipped tickets, challenge roadmap drift, and shape the next work. Trigger: /groom, /backlog, /rethink.
argument-hint: "[--emphasis tidy|shape|rethink]"
---

# /groom

Every run starts with tracker truth before strategy. Compare `backlog.d/`, `backlog.d/_done/`, recent git trailers, `SPEC.md`, slice docs, QA/dogfood receipts, and current code. Do not treat a stale active ticket as real priority just because the file is still under `backlog.d/`.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

Use `backlog_ids_from_commit`, `backlog_ids_from_range`, `backlog_file_for_id`, and `backlog_archive` from `scripts/lib/backlog.sh` when reconciling closure. If a shipped ticket remains active, archive it. If an archived ticket still says `status: ready`, fix it only under backlog hygiene scope or shape a concrete hygiene item.

## Current Strategy

Slice 6 is the active pressure path: beta persistence, source-grounded quiz/exercise generation, mobile study dogfood, service-contract hardening, graduated activity ladders, backlog/QA hygiene, and extraction decision work. Keep `backlog.d/16-system-visualization-workbench.md` later unless architecture confusion is causing repeated defects.

Reject stale Scry/Vault canary requirements. Current proof is repo-local dogfood lanes, beta receipts, package tests, and explicitly shaped external proof.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
