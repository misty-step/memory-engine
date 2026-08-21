# Legacy Work Hygiene

This historical receipt describes the retired work queue and ledger. A `ready` card must be
claimable with an executable oracle; a `done` card must not appear in the
ready query.

## Active Tickets

An active card must describe work that is not yet proved complete. If current
repo evidence already satisfies every oracle, complete the card with proof
instead of leaving it active. If implementation only satisfies part of the
oracle, reshape the card so the remaining work is explicit and executable.

Active card evidence must name:

- the files or executable surfaces that satisfy each oracle;
- the exact focused command for any ticket-specific proof;
- the fast gate, `bun run ci`, plus the full `bun run ci:full` handoff gate
  when the branch is ready for handoff.

## Completed Legacy Cards

Completed legacy cards are historical receipts. Attach closure evidence or a
commit/PR link concrete enough for a cold reviewer to re-run the proof.

## QA Receipts

For now, QA receipts remain stdout-first. `crates/memory-engine-qa` already
prints lane ids, surfaces, purposes, commands, pass/fail receipts, and a
summary. Adding a persisted `--report <path>` mode would create another
artifact format before review workflow has shown a durable need for persisted
machine-readable receipts.

Use persisted QA reports only when a future ticket names a consumer that needs
stable report files, such as release notes, a PR bot, dashboard import, or
cross-repo comparison. Until then, copy the relevant command receipts into the
handoff, trace, or commit message.

## Legacy Work Item 30 Delivery Receipt

This hygiene pass used stdout receipts rather than adding
`memory-engine-qa --report <path>`.

- `cargo test -p memory-engine-persistence` plus
  `cargo test -p memory-engine-generation`: persistence and deterministic
  beta-generation proof passed.
- `bun run check`: Biome checked 62 files with no fixes applied.
- `bun run qa`: 13 lanes passed, 0 warnings, 0 failures; canonical CI lane
  passed; coverage floor met at 93.06% funcs and 93.18% lines.
- `bun run ci`: Dagger check passed; typecheck, Biome, coverage, and Gitleaks
  completed with no leaks found.

## Current Slice 6 Queue

The prior ledger hygiene pass completed the beta persistence, beta generation,
and hygiene cards. The remaining historical Slice 6 path started with the
mobile beta interface as the highest-priority product proof:

- `28-mobile-beta-study-interface`
- `29-service-contract-v0-hardening`
- `31-beta-extraction-decision`
- `32-graduated-activity-ladder`

`16-system-visualization-workbench` remains later unless architecture confusion
causes repeated defects.
