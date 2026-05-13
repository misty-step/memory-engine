---
name: groom
description: |
  Maintain memory-engine's file-backed backlog, reconcile shipped tickets, challenge roadmap drift, and shape the next work. Trigger: /groom, /backlog, /rethink.
argument-hint: "[--emphasis tidy|shape|rethink]"
---

# /groom

Grooming keeps `backlog.d/` truthful. Every run tidies before strategy: compare active tickets, `backlog.d/_done/`, `SPEC.md`, slice docs, recent git history, and canary branches.

Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`). The detector is `scripts/lib/backlog.sh`: use `backlog_ids_from_commit`, `backlog_ids_from_range`, and `backlog_archive` when reconciling closure trailers against active files. If a shipped ticket remains active, archive it. If an archived ticket still says `status: ready`, fix the status or shape a hygiene ticket.

## Strategic Work

After tidy, challenge whether the next item belongs in the kernel or a consumer. Require a consumer proof story for shared behavior. Update `SPEC.md` when roadmap status drifts from code. Shape atomic tickets with executable oracles and canary commands where needed.

Current known grooming targets: reconcile `SPEC.md` immediate-next-work with implemented slices 2/3; fix stale frontmatter for tickets 10-13; record or re-run Scry and Vault rubric canaries.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
