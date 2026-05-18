---
name: implement
description: |
  Execute a shaped memory-engine backlog ticket with TDD, pure-kernel discipline, exact oracle verification, and coherent backlog closure metadata. Trigger: /implement.
argument-hint: "[backlog.d ticket]"
---

# /implement

Use this for a shaped `memory-engine` ticket. Before changing code, read `AGENTS.md`, `CLAUDE.md`, `.spellbook/repo-brief.md`, `SPEC.md`, relevant `SLICE-*.md`, and the target ticket. If there is no active ticket, stop and run /shape or /groom first.

One ticket maps to one branch and one PR. Branch from `master`, using `cx/...` unless the operator specifies otherwise. Include a structured reference that /ship can resolve later: `Refs-backlog: NN` during implementation, then `Closes-backlog: NN` or `Ships-backlog: NN` when closing. Lifecycle contract: active work lives in `backlog.d/`, closed work lives in `backlog.d/_done/`, closure trailers are `Closes-backlog:` or `Ships-backlog:`, references use `Refs-backlog:`, and archival uses `scripts/lib/backlog.sh` (`backlog_archive`).

## TDD Loop

Write the failing behavior test first, implement the smallest pure change, then refactor. Test locations are part of the contract: `tests/types/`, `tests/scheduler/`, `tests/grader/`, `tests/progression/`, `tests/queue/`, `tests/testkit/`, and `tests/adapters/`.

Keep `src/` pure. Runtime code must not import frameworks, Node/Bun APIs, filesystem, network, persistence, logging, or vendor SDKs. Consumers own storage, UI, sessions, identity, analytics, and model clients.

Preserve invariants: `ScheduleState` is ts-fsrs-native and JSON-safe; prompt union changes update grader dispatch and exhaustiveness tests; `Grader.grade()` returns one envelope with `rating`; verdicts remain `correct | close | wrong | revealed`. Do not introduce `any`, non-null assertions, or `@ts-ignore`.

## Verification

Run the ticket's exact oracles. Use focused suites while iterating, then finish with `bun run ci`. If the ticket names current dogfood, beta, or external proof commands, run them exactly and report them separately. Historical Scry and Vault canaries are deprecated and are not required unless a future ticket explicitly replaces them with a current oracle.

Close with ticket ID, changed surfaces, exact commands run, proof evidence if applicable, and residual unverified paths.
