# MCP face: wrap the study/review API as a stdio MCP server

Priority: P1 · Status: shipped · Estimate: M

Origin: Powder card `memory-engine-071`. Closes a cell in the fleet-wide
"five faces" coverage matrix (skill / CLI / API / MCP / UI) — `memory-engine-api`
is the API face, `memory-engine-review` (070) is the CLI face, this is the
MCP face.

## Goal

An MCP-speaking agent can run a full study/review loop and manage project
decks purely over stdio JSON-RPC, with tools shaped as agent intents
(`create_deck`, `list_due`, `review_next`, `submit_answer`, ...), not 1:1
REST wrappers around the v1 contract — the discipline `powder-mcp` already
proved for this fleet.

## Non-Goals

- No new server-side surface. `crates/memory-engine-mcp` speaks the existing
  v1 HTTP contract (`ureq`, matching `memory-engine-review`'s idiom); it adds
  no route to `memory-engine-api`.
- No generation-provider tool surface (no raw `generate_source`/`approve_draft`
  tools). `create_deck` composes those calls internally so a deck is
  immediately due; exposing the intermediate steps as separate tools is
  deferred until real usage shows an agent needs to inspect drafts before
  approving them.
- No registration in the live `~/.harness-kit/factory-mcps.yaml` registry —
  that is a cross-repo, live agent-config change with its own review surface.
  `docs/dogfood/mcp-review-loop.md` documents the entry shape a follow-up
  would add.

## Oracle

- [x] `crates/memory-engine-mcp` is a standalone Rust binary+lib crate that
      speaks the v1 HTTP contract directly (`ureq`, matching
      `memory-engine-review`'s idiom), wired into the workspace like
      `memory-engine-cli`/`memory-engine-review`.
- [x] Six agent-intent tools: `create_deck`, `list_decks`, `invalidate_deck`,
      `list_due`, `review_next`, `submit_answer` — proved not to be
      REST-route echoes by a dedicated contract test.
- [x] `cargo test -p memory-engine-mcp` includes a real end-to-end test that
      spawns the compiled binary (not an in-process function call) against a
      local `memory-engine-api` axum server and drives
      create_deck -> list_decks -> list_due -> review_next -> submit_answer
      -> invalidate_deck -> list_due over its actual stdin/stdout pipes.
- [x] A dedicated test proves the server fails loudly (non-zero exit, no
      stdout) when no credential path (env vars, credentials file,
      bootstrap email) resolves — no silent in-memory fallback, matching
      `powder-mcp`'s own regression test for the same failure mode.
- [x] `docs/dogfood/mcp-review-loop.md` records the tool contract, the
      credential model, the falsifier, and a hand-run transcript from a real
      terminal (not test output) plus the factory-mcps registry entry shape.
- [x] `bun run ci` (fast gate) and `bun run ci:full` (Dagger gate) both pass.

## Falsifier

The claim under test: "a cold agent, given only `tools/list`'s descriptions,
can complete a full deck-create -> review -> invalidate loop over MCP stdio
without reaching into `memory-engine-api` internals or duplicating its
service logic."

Falsifying evidence would be: the loop cannot complete without extra
context beyond the tool descriptions, a tool silently drops server state
between calls, or a tool turns out to be a bare REST-route echo instead of
an agent intent.

## Notes

Reuses `memory-engine-review`'s exact credential env var names
(`MEMORY_ENGINE_ACCOUNT_ID`/`MEMORY_ENGINE_SESSION_TOKEN`, already documented
in `docs/runbook.md`) so the two dogfood clients are interchangeable on one
account. No interactive `login` subcommand exists here (stdin is the JSON-RPC
channel), so a brand-new local server bootstraps its own account
non-interactively from `MEMORY_ENGINE_MCP_EMAIL` instead, persisting to
`~/.memory-engine/mcp/credentials.json`.

## Shipped — 2026-07-04

PR #31 merged as `4db426f` with green CI. The committed dogfood transcript
drives the compiled stdio server through deck create, review, submit,
invalidation, and empty-queue proof against a real local API. Residual: live
harness registration and a production-account replay remain deliberately
outside this ticket.
