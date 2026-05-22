# Backlog Hygiene

The backlog is a work queue, not a historical archive. Active tickets live in
`backlog.d/`; completed tickets live in `backlog.d/_done/`. A ticket in
`_done/` must not carry `status: ready`, because that makes the next-work
selector treat closed work as available scope.

## Active Tickets

An active ticket must describe work that is not yet proved complete. If current
repo evidence already satisfies every oracle, archive the ticket instead of
leaving it active. If the implementation only satisfies part of the oracle,
reshape the ticket so the remaining work is explicit and executable.

Active ticket evidence must name:

- the files or executable surfaces that satisfy each oracle;
- the exact focused command for any ticket-specific proof;
- the canonical gate, `bun run ci`, when the branch is ready for handoff.

## Archived Tickets

Archived tickets are historical receipts. Their frontmatter should use
`status: shipped` when the ticket was delivered and moved into `_done/`.
Do not leave archived tickets with `status: ready`.

When an active ticket is archived because existing code already satisfies it,
add closure evidence to the ticket or preserve the verification commands in the
commit message. The evidence should be concrete enough for a cold reviewer to
re-run the proof without reading the old conversation.

## QA Receipts

For now, QA receipts remain stdout-first. `scripts/qa.ts` already prints lane
ids, surfaces, purposes, commands, pass/fail receipts, and a summary. Adding
`scripts/qa.ts --report <path>` would create another artifact format before
review workflow has shown a durable need for persisted machine-readable
receipts.

Use persisted QA reports only when a future ticket names a consumer that needs
stable report files, such as release notes, a PR bot, dashboard import, or
cross-repo comparison. Until then, copy the relevant command receipts into the
handoff, trace, or commit message.

## Backlog 30 Delivery Receipt

This hygiene pass used stdout receipts rather than adding
`scripts/qa.ts --report <path>`.

- `bun test experiments/beta-store/ experiments/beta-generation/`: 8 pass,
  0 fail.
- `bun run check`: Biome checked 62 files with no fixes applied.
- `bun run qa`: 13 lanes passed, 0 warnings, 0 failures; canonical CI lane
  passed; coverage floor met at 93.06% funcs and 93.18% lines.
- `bun run ci`: Dagger check passed; typecheck, Biome, coverage, and Gitleaks
  completed with no leaks found.

## Current Slice 6 Queue

Backlog hygiene archived the completed beta persistence, beta generation, and
backlog hygiene tickets. The remaining active Slice 6 path starts with the
mobile beta interface as the highest-priority product proof:

- `28-mobile-beta-study-interface`
- `29-service-contract-v0-hardening`
- `31-beta-extraction-decision`
- `32-graduated-activity-ladder`

`16-system-visualization-workbench` remains later unless architecture confusion
causes repeated defects.
