---
name: implement
description: |
  Execute a shaped memory-engine backlog ticket with TDD, pure-kernel discipline, exact oracle verification, and coherent backlog closure metadata. Trigger: /implement.
argument-hint: "[backlog.d ticket]"
---

# /implement

Use this for one shaped `memory-engine` ticket. Before editing, read `AGENTS.md`, `.spellbook/repo-brief.md`, `SPEC.md`, relevant `SLICE-*.md`, the target `backlog.d/` ticket, and touched tests/source. If there is no active shaped ticket for feature work, stop and run `/shape` or `/groom`.

Branch from `master` with `cx/...` unless directed otherwise. Record `Refs-backlog: NN` during implementation so `/ship` can later resolve closure.

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

## TDD Loop

Write the failing behavior test first, implement the smallest pure change, then refactor. Use the repo's real collaborators; do not mock repo-owned pure modules, adapters, or `ts-fsrs`. Mock only external network, clock/random, third-party SDKs, or filesystem when file content is irrelevant.

Test surfaces: `tests/types/`, `tests/scheduler/`, `tests/grader/`, `tests/progression/`, `tests/queue/`, `tests/testkit/`, `tests/adapters/`, `tests/service/`, `tests/evals/`, and focused `experiments/*/*.test.ts` when the ticket names beta/dogfood proof.

Keep `src/` pure: no framework, Bun/Node API, filesystem, network, persistence, logging, UI, auth, analytics, or provider SDK imports.

## Verification

Run the ticket's exact oracles. Use focused tests while iterating, then finish with `bun run ci`. If the ticket names dogfood, beta, or external proof commands, run them and report them separately.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
