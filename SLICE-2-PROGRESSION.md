---
shaping: stub
slice: 2
parent: SPEC.md
status: awaiting-slice-1-completion
---

# Context Packet Stub: Memory Engine — Slice 2 (Progression + Queue + Adapter split)

**This is a strategic stub, not a shaped packet.** Full shaping happens
after slice 1 merges. `/groom` runs again at that point to decompose this
into tickets.

## Goal

Add progression-aware eligibility and queue-selection primitives to the
kernel, and split the single-package topology into `packages/{contracts,
core, adapters, testkit}` once the adapter boundary is concrete.

## Why this slice exists

Slice 1 proved the kernel's grading + scheduling boundary holds against
one consumer (Vault SRS). Slice 2 earns two things:

1. The richer semantics that make the kernel worth extracting in the
   first place — `requires`, `supersedes`, `stageOrder`, queue pacing.
   Without these, any app with real pedagogy has to reimplement them
   locally, and duplication reappears.
2. A second consumer. Slice 1 with one consumer is a library for one
   caller. Slice 2 proves the boundary is shared, not private.

## Non-Goals

- Rubric or AI-assisted grading (slice 3).
- Session choreography — stays app-owned forever.
- Content authoring pipelines.
- npm publish — defer until all three slices ship.
- Replace the slice-1 public API. Slice 1 stays backward compatible.

## Scope

### Progression primitives

- `ProgressionEdge` type (requires / supersedes / stageOrder).
- Eligibility computation: `isEligible(unit, graph, state) → boolean`.
- Mastery predicate: `isMastered(state) → boolean` (lifted from Ruminatio).
- No authoring concerns — the graph is given, not constructed.

### Queue primitives

- `pickNext(candidates, now, policy) → Candidate | null` — minimum
  useful queue primitive.
- Anti-clumping policy as injected function (lesson from bench: don't
  pre-abstract policies).
- Progression-aware eligibility gating (via slice-2 progression module).
- **Open:** does "select a burst" belong in core or in a consumer
  helper?

### Adapter split

- `packages/contracts` — pure types, no logic.
- `packages/core` — slice-1 logic (scheduler, grader) + slice-2 logic
  (progression, queue).
- `packages/adapters` — storage adapter interface + reference
  implementations (at least: in-memory; optionally: SQLite for Vault).
- `packages/testkit` — migrates out of the single package.
- Bun workspace as the monorepo driver. Top-level `package.json`
  becomes a workspace manifest.

## Constraints / Invariants

- Slice-1 public API stays backward compatible. Consumer imports do not
  change. Package split is transparent to the import path via workspace
  re-exports where needed.
- Progression fields do NOT live inside `Prompt`. They attach to the
  `ReviewUnit` via a separate graph structure (preserves `Prompt` as
  presentation-layer metadata only).
- Queue primitives work for BOTH concept-level (Scry) and
  phrasing-level (Ruminatio) consumers via `ReviewUnitId` opacity.
- Adapter interface MUST NOT force event sourcing. Mutable-snapshot is
  the default model (per SPEC.md §Storage And Adapter Model).

## Second-consumer question

Pick during shaping:
- **Caesar in a Year** — sentence-level, no progression today. Clean
  smoke test of queue primitives alone. Low risk.
- **Scry** — concept-level. Forces the opacity of `ReviewUnitId` to hold
  under review. Higher risk, higher confidence if it passes.

Recommend Caesar as the adoption path and Scry as the opacity stress
test. But real decision waits for shaping.

## Open Questions (resolved during slice-2 shaping)

- Exact shape of `ProgressionEdge` — typed relations vs typed edge list?
- Anti-clumping: core primitive with injected policy, or consumer-owned
  entirely?
- Does Scry's concept-level consumer need the queue primitive at all, or
  does it just use `scheduler.next` + custom eligibility?
- How does adapter storage-shape compose with slice-1's
  consumer-owns-persistence rule? (Adapter doesn't change the rule; it
  gives consumers a reusable implementation option.)
- ts-fsrs version bump policy once multiple consumers depend.

## Rough Implementation Sketch

Six to eight tickets, roughly:
1. Progression types (`ProgressionEdge`, `isMastered`, `isEligible`).
2. Queue primitives (`pickNext`, anti-clumping hooks).
3. Second-consumer canary (Caesar): smoke test of queue primitives.
4. Package split: workspace manifest, move files, redirect exports.
5. Storage adapter interface + in-memory reference.
6. Second-consumer canary (Scry) against progression + adapter.
7. Slice-2 testkit additions (progression/queue fixtures).
8. Documentation + migration guide for existing Vault adoption.

## Depends on

Slice 1 complete and merged. Vault canary green and promoted to main
migration PR.
