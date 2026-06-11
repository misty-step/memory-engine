# Generation quality and latency pass

Priority: P2 · Status: pending · Estimate: M

## Goal

Close the quality gaps the 047 model-judge run exposed (distractor quality
3.8/5, keep rate 70%) and cut perceived generation latency, so a capture
reliably yields keep-worthy items fast enough that the user stays in flow.

## Oracle

- [ ] Distractor instructions in the generation prompt are strengthened
      (plausible, same-category, common-misconception-shaped) and a fresh
      judged field run shows distractor quality and keep rate improved over
      the 2026-06-11 baseline (docs/evals/generation-field-2026-06-11.md);
      receipt committed under docs/evals/.
- [ ] Duplicate/near-duplicate items are filtered before persistence with a
      cheap algorithmic similarity check (no extra model calls — see
      pdf-to-interactive-lesson's Jaccard approach), with unit tests on
      known-duplicate fixtures.
- [ ] A repair stage exists: items the trust gate or judges reject are
      regenerated once (bounded retry, cost-capped) before surfacing a
      zero-draft notice; cost accounting still lands in the usage record.
- [ ] Latency: generation for a typical article-sized source is measured
      end-to-end on production, the number is recorded, and at least one
      structural improvement ships if p50 exceeds ~10s (candidates:
      parallel per-chunk calls, streaming partial results to the UI).
- [ ] No quality regression: the full 12-source bench (deterministic +
      model judge) stays green and is the comparison artifact.

## Notes

Follow-on from the 043/047 eval work — the judge lane exists precisely to
make this ticket measurable. The pdf-to-interactive-lesson project
(first-contact report) validates the pipeline shape: parallel stages for
latency, algorithmic dedup, explicit repair of weak items. Keep the
provider boundary clean: dedup and repair orchestration live in
memory-engine-generation, not the OpenRouter crate. Prompt changes ride the
existing PromptVariant machinery so A/B runs stay one flag away.

## Children

1. Distractor prompt strengthening + judged re-run vs baseline.
2. Algorithmic dedup before persistence.
3. Bounded repair/regeneration stage with cost cap.
4. Production latency measurement + one structural improvement if needed.
