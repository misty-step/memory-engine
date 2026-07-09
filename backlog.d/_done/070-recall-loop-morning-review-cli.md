# Recall-loop dogfood client #1 — morning review CLI against the deployed API

Priority: P1 · Status: shipped · Estimate: M

Origin: Powder card `memory-engine-070` (recall-loop dogfood client #1, the T2
rung after the in-process `memory-engine-cli` fixture client).

## Goal

Prove the deployed `memory-engine-api` v1 contract is ergonomic enough for a
real daily habit: a brutally thin CLI that authenticates once, drives
`review/next` -> answer -> `review/submit` in a loop until the account's due
queue is empty, and leaves local evidence a later session can use to check
whether a 30-day streak and cold-recall accuracy are real.

This is not a product surface. It is the second dogfood pressure test on the
v1 contract, this time over the network against the live Fly deployment
(`memory-engine-contract`, already shipped, proved the contract shape works
end-to-end against a real HTTP server; this ticket proves it is usable enough
for a human's actual morning routine, not just a scripted fixture).

## Non-Goals

- No new server-side surface (no streak field, no cold-recall endpoint). The
  API already returns everything needed (`dueCount`, `grade`,
  `scheduleChange.before/after`); this client computes and logs derived
  metrics locally.
- No voice input (typed answers only; voice is a later rung).
- No production account self-service beyond what `POST /v1/accounts` already
  does; recovering an existing account's session token is a documented manual
  step (see `docs/dogfood/morning-review-cli.md`), not new code.

## Oracle

- [x] `crates/memory-engine-review` is a standalone Rust binary crate that
      speaks the v1 HTTP contract directly (`ureq`, matching
      `memory-engine-contract`'s idiom) — no dependency on
      `memory-engine-core`/`-service` (this is an external client, not an
      in-process dogfood harness like `memory-engine-cli`).
- [x] `cargo test -p memory-engine-review` includes a real end-to-end test
      that boots a local `memory-engine-api` axum server, creates an account,
      seeds a source, generates and approves a draft, then drives the review
      loop through piped stdin to a natural `dueCount == 0` completion.
- [x] The CLI has a documented one-time `login` step (`--email` for a new
      account, or `--account-id`/`--session-token` to import an existing
      one) that persists credentials locally and is never checked in.
- [x] The CLI loops `review/next` -> answer -> `review/submit` until
      `dueCount` reaches 0 (or a `--max-cards` safety cap, which is reported
      as an incomplete session, not silently swallowed).
- [x] Every graded attempt and every completed session is appended to a local
      NDJSON log (no new server surface) with enough fields to reconstruct a
      30-day streak and a cold-recall rate later.
- [x] `memory-engine-review streak` reads that log and reports: hit rate over
      a window (default 30 days), current consecutive-day streak, and
      cold-recall accuracy (attempts where the review unit had
      `scheduleChange.before.reps >= 1`, i.e. this was not the first exposure).
- [x] `docs/dogfood/morning-review-cli.md` records the falsifier, the
      self-run transcript, and residual risk.
- [x] `bun run rust:ci` (fast gate) and `bun run ci:full` (Dagger gate) both
      pass.

## Falsifier

The claim under test: "a human can run one command each morning, answer
typed prompts, and reach `dueCount == 0` in under two minutes, with local
evidence that later proves whether this became a real habit."

Falsifying evidence would be: the loop cannot reach 0 without reaching into
the API's internals or duplicating service logic (contract failure, same
falsifier as `memory-engine-contract`'s own ticket) — or the streak/
cold-recall numbers cannot be reconstructed from what the API already
returns (would mean new server surface is actually required, contradicting
this ticket's non-goal).

## Notes

Reuses the existing `.memory-engine/` local-state convention (already used
by the dev server's file-store and by prior QA receipts) for credentials and
the streak log, under `~/.memory-engine/review/`.

## Shipped — 2026-07-04

PR #30 merged as `b694b41` with green CI and fresh-context review. The real
binary completed the full loop against a local instance of the production Rust
server; `docs/dogfood/morning-review-cli.md` carries the transcript. Residual:
the operator has not yet run a real production morning or accumulated a cold
attempt, so this ships the client—not the 30-day habit claim now owned by 073.
