# Crucible eval integration — epic

Priority: P2 · Status: pending · Estimate: M

## Goal

Decide and spec how Memory Engine's generation-quality evals relate to
Crucible (the fleet's eval/benchmark workbench), so quality measurement work
doesn't fork into two competing systems. This ticket is a decision +
spec-writing epic, not an implementation migration — Crucible does not yet
have the ingestion surface to receive it.

## Oracle

- [ ] A written recommendation exists (this ticket, refined during delivery)
      on the split: what stays in `memory-engine-bench` vs what Crucible
      should eventually own, with the reasoning below either confirmed or
      revised against Crucible's actual state at delivery time.
- [ ] If a near-term action is in scope, it is export-format alignment only:
      naming memory-engine's judged-run fields to a Harbor-importable shape
      (task definition, fixture refs, grader manifest, rubric, baselines, run
      records, labels, aggregate scores, uncertainty, provenance) so a future
      Crucible import adapter is additive, not a rewrite — mirroring the
      OTel-GenAI-naming hedge already applied in 062.
- [ ] No memory-engine bench functionality is removed or paused waiting on
      Crucible; 058/060/061 ship on their own timeline regardless of this
      ticket's resolution.

## Notes

Operator's words, relayed via groom dispatch 2026-07-02: evals "should
compose with crucible" and "the eval work belongs there as specs, with
memory-engine as the customer."

**Recommendation, based on reading both repos' current state (2026-07-02):**
keep the near-term generation-quality graders in `memory-engine-bench`, do
not migrate now. Reasoning, from Crucible's own `VISION.md`:

- Crucible's first eval family is explicitly agentic code review (the
  Cerberus wedge); "Product behavior evals for Memory Engine" is named as a
  **future** family, after that wedge lands.
- Crucible states it has "no live model-call engine yet" as of 2026-07-01 —
  it cannot yet run a generation eval end to end.
- Memory Engine's classification/coverage/directionality graders (060) need
  tight coupling to `ReviewUnit`/`Draft`/`GenerationRun` domain types that
  live in `crates/memory-engine-core` and `-generation`. Crucible owning that
  grader logic today would mean either duplicating those types across repos
  or Crucible reaching into memory-engine's crate graph — both violate
  Crucible's own "own the eval artifact, borrow the engine" model, since
  there is no borrowed engine here to plug behind an adapter.

So Memory Engine is the right owner of *domain-specific* graders now, and the
right customer of Crucible's run database, calibration/trust layer, and
human-judgment UI *once those exist* — which is exactly the role Crucible's
vision assigns Memory Engine ("Product behavior evals for Memory Engine or
Allie" is explicitly listed as a next family after code review). Building the
export-format alignment now is the correct-sized bet: cheap, and it means the
eventual Crucible import is additive per Crucible's own stated intent to
"import eval and benchmark definitions authored elsewhere... through
adapters" rather than a rewrite.

**Open call for the operator:** should this ticket also *file* the "Memory
Engine eval family" scoping work into Crucible's own `backlog.d/` now (as a
forward-looking placeholder Crucible can pick up when it gets to that
family), or wait until Crucible's code-review wedge actually ships? Filing
early risks a stale spec; waiting risks the two systems drifting further
apart on vocabulary. This ticket does not resolve that call — it's flagged
for the operator.

## Children

1. Confirm/revise the stay-vs-migrate recommendation against Crucible's state
   at delivery time (re-read Crucible's `backlog.d/` — it moves fast).
2. Harbor-shape field-naming pass over memory-engine-bench's judged-run
   output (058's calibration records, 060's grader outputs).
3. Decide (with the operator) whether to file a placeholder "Memory Engine
   eval family" ticket in Crucible's own backlog now or defer.
