# Mitigate question memorization with retrieval variability

Priority: P2 · Status: pending · Estimate: M

## Goal

Users learn the concept, not the prompt string. Repeated reviews of the same
concept vary the retrieval demand — rephrased prompts, shuffled or
regenerated distractors, different item forms at the same difficulty — so
pattern-matching a memorized question stops working while concept knowledge
still does.

## Oracle

- [ ] Distractor order is never stable across reviews of the same
      multiple-choice item (cheapest win first; kernel/study test).
- [ ] Concepts carry item *variants*: generation can produce 2–3
      same-stage phrasings per concept, and the queue rotates among them on
      successive reviews; rotation behavior has a deterministic test.
- [ ] The mechanism is literature-grounded via 050: the docs/science entry
      for retrieval variability cites the evidence and states the design
      decision this ticket implements (or 050 documents why the literature
      rejects it, and this ticket is closed against that finding).
- [ ] An eval scenario checks variant quality: same concept, same stage,
      genuinely different surface form, no answer leakage between variants.
- [ ] Detection hook: per-item response-time + success trend is queryable
      (053's history query) so a future heuristic can flag
      "answers instantly, fails rephrasings" items.

## Notes

User intent from first contact: "do whatever we can to mitigate users sort
of just memorizing the question and not understanding the concept, which is
key." This is the engine-side moat (049 split: engine owns the science).
Sequence after 050's bibliography child so the implementation traces to
citations rather than vibes — retrieval variability and desirable difficulty
are exactly its scope. Variant generation rides 043's provider boundary and
051's intent-aware prompts; variant storage must keep `ScheduleState`
JSON-safe (invariant 3) — variants are item-level content, scheduling stays
per-review-unit.

## Children

1. Distractor shuffling (immediate, no model calls).
2. Variant generation + queue rotation + deterministic tests.
3. Literature alignment with 050; eval scenario for variant quality.
