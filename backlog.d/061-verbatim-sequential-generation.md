# Verbatim / sequential / enumerable card generation

Priority: P2 · Status: pending · Estimate: L

## Goal

Generate content-appropriate, comprehensive cards instead of sampled conceptual
trivia. A creed or poem → overlapping-cloze "next-line" cards covering **every**
line; an enumerable set → complete recall cards in the non-derivable direction;
conceptual prose unchanged ("fewer, better"). Driven by 060's eval suite.

## Background (diagnosed)

The capability is half-present: `LearningIntent::VerbatimMemorization` exists and
the deterministic provider has crude `verbatim_candidates` ("recite the next line
after…"). But the **model path** — which real captures use — emits only
`quiz`/`exercise` Q&A with distractors. There is no cloze card type, no coverage
invariant, and no set/sequence branch, and the model misclassified the creed as
conceptual to begin with. Research brief (Wozniak's 20 Rules, Anki LPCG,
SuperMemo) summarized in the 2026-06-23 thread; the consensus shape for verbatim
text is overlapping cloze, one line per card, ~2 lines of context.

## Oracle

- [ ] **Creed / poem** → per-line overlapping-cloze ("show ~2 lines, recall the
      next"), covering every line; stanza/section bridge cards where structure
      exists.
- [ ] **Enumerable set (NATO)** → 26 letter→word recall cards (one per letter).
      **No** word→letter cards — that direction is derivable from the answer's
      first letter, so it is bloat (the load-bearing principle: emit only
      mappings the learner can't trivially derive from the answer).
- [ ] **Classification fixed**: the creed is recognized as verbatim/sequential,
      not conceptual.
- [ ] **Coverage invariant** enforced in `memory-engine-generation` — asserted in
      code, not left to model judgment (the model is told to be comprehensive
      *and* the runner verifies `[1..N]`).
- [ ] **Conceptual prose unchanged** — regression: still fewer-better cards; the
      coverage rule does not bleed into conceptual material.
- [ ] **Invariant 4 honored**: any new prompt variant or card type co-evolves
      with exhaustive Rust match coverage + grader tests in the same change; if a
      cloze card shape is added, the review-UI rendering and grading land with it.
- [ ] 060's suite goes **green** — that is the proof, not eyeballing.

## Notes

**Open scope decision (defer to delivery):** full redesign (a true
overlapping-cloze card type end to end — schema, grader, review UI) vs.
incremental, eval-gated (comprehensiveness + correct classification +
directionality first, using existing recall Q&A per line; add the cloze type
only when 060 shows recall Q&A isn't enough). Recommend the incremental path:
smaller first step, the eval tells us when to escalate. Keep dedup/repair
orchestration in `memory-engine-generation`, not the OpenRouter crate (per 055).

## Children

1. Content-type classifier: verbatim/sequential vs enumerable-set vs conceptual.
2. Coverage invariant in the generation runner.
3. Next-line / overlapping-cloze card shape (scope per the decision above).
4. Directional set cards (non-derivable direction only).
5. Prompt ↔ grader co-evolution; review-UI for any new card shape.
6. 060 suite green as the acceptance artifact.
