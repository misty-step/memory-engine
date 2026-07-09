# Multi-Client Beta Pressure

Refs-Powder: memory-engine-033

## Purpose

This receipt pressure-tested beta boundaries across two independent clients.
After the Rust cutover, the surviving executable pressure lives in
`crates/memory-engine-study` and `crates/memory-engine-beta-app`:

- mobile-first study/session flow in `memory-engine-study`, and
- local browser/API flow in `memory-engine-beta-app`.

Both clients use the same persistence spine (`BetaPersistenceStore`) and the
same service command surface (`next-queue`, `grade/apply-review`) while owning
client-specific reveal/session choreography.

## Executable Receipt

```sh
cargo test -p memory-engine-study -p memory-engine-beta-app
```

Rust session and HTTP tests prove source ingest, generation, approval, queue
selection, reveal, submit, and restart/resume behavior. Historical
multi-client TypeScript pressure was deleted after Rust parity landed.

## Repeated Cross-Client Signals

- Source ingest and generation stay app-owned and persist through the same
  beta store snapshot model.
- Draft approval promotes review units through the same store-backed boundary.
- Queue selection in both clients is driven by service `next-queue`, not
  client-local queue logic.
- Reveal is display-only state in both clients; no scheduling write occurs on
  reveal and attempt count remains unchanged.
- Submit flows through `grade/apply-review` and projects the same compact
  review-state DTO (`due`, `reps`, `lapses`, `state`, `last_review`).
- Duplicate submit after graded state is ignored in both clients; attempts and
  schedule projection remain stable.
- Restart/resume in both clients reloads persisted attempts, schedules, and
  approved units without regenerating content.

## Product-Owned Divergences

- Mobile study uses manual per-draft approval and browser-oriented screen state;
  coach workflow supports bulk approval for command-line style throughput.
- Mobile study emphasizes learner-facing composition (prompt pane + side
  panels); coach workflow emphasizes command traceability via `commands` logs.
- Both expose schedule-change projection, but each client shapes surrounding
  view DTOs for its own interaction model.

These divergences are intentional product behavior, not contract drift.

## Boundary Verdict

The current beta boundaries held under repeated pressure: persistence remains in
`crates/memory-engine-persistence`, service commands remain stable, reveal
remains UI-owned, and no new pure-kernel surface was required.
