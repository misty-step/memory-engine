# Capture the memory-science literature in the engine

Priority: P2 · Status: pending · Estimate: XL

## Goal

The engine earns "memory engine": its scheduling, grading, and item-design
decisions trace to cognitive-science findings (testing effect, desirable
difficulty, interleaving, retrieval variability), each captured as cited
doctrine plus an executable behavior with an eval.

## Oracle

- [ ] docs/science/ holds a living annotated bibliography: each adopted
      principle has citations, the design decision it drives, and the eval
      or test that enforces it (no cargo-cult features).
- [ ] Interleaving: queue anti-clumping is justified or revised against the
      literature, with a bench scenario proving the behavior.
- [ ] Desirable difficulty: generation produces items across retrieval
      depths (recognition → cued recall → free recall/composition) and the
      progression ladder is literature-aligned; eval distinguishes the tiers.
- [ ] FSRS parameters: a documented stance on default vs per-user
      optimization, with a simulation bench over synthetic review histories.
- [ ] At least one principle is deliberately rejected with cited reasoning
      (proof the bibliography is a filter, not a collection).

## Notes

This is the moat for the Scry split (049): Scry owns experience, this engine
owns the science. Existing assets to build on: pure FSRS (Scry doctrine: no
comfort features), progression stages, recitation grading, queue
anti-clumping, memory-engine-bench. Use /research lanes for literature
sweeps; every claim lands with a citation or it doesn't land.

## Children

1. Annotated bibliography v1 (testing effect, spacing, interleaving,
   desirable difficulty, feedback timing).
2. Item-depth ladder alignment + generation guidance for 043 prompts.
3. FSRS parameter stance + simulation bench.
4. One rejected-principle writeup.
