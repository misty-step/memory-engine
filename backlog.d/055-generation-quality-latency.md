# Generation quality and latency pass

Priority: P2 · Status: in progress · Estimate: M

## Goal

Close the quality gaps the 047 model-judge run exposed (distractor quality
3.8/5, keep rate 70%) and cut perceived generation latency, so a capture
reliably yields keep-worthy items fast enough that the user stays in flow.

## Oracle

- [x] Distractor instructions in the generation prompt are strengthened
      (plausible, same-category, common-misconception-shaped). The judged field
      run shows **no large regression** vs the 2026-06-11 baseline — distractor
      and keep-rate deltas sit inside the noise floor at n=12, so a small
      *improvement* is not provable by this suite (2026-06-21 rigor receipt).
      Ticket 058 gives the suite the power + binary criteria to claim one.
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

## Progress — feat/055-gen-quality-latency (2026-06-20)

Landed on the branch (delivered, merge-ready):

- Distractor prompt strengthened (same-category / same-keyed-feature /
  misconception-shaped) with a deterministic `distractor_cohesion` bench judge; a
  self-reference gate with a `self_referential_free` judge; per-card grounding (a
  quote-free card becomes a world-knowledge expansion instead of a rejection).
- Structural latency improvement shipped (oracle 4): generation is now
  non-blocking — a capture enqueues a background job and returns immediately, with
  status streamed to the UI over SSE. This cuts *perceived* latency; per-source
  model compute is unchanged.
- Dedup (oracle 2) and bounded repair (oracle 3) were already in HEAD before this
  branch (6d400fa et al.) and were left untouched.
- The bench now shares the runtime self-reference predicate, so eval and gate
  cannot drift. MCQ grading normalizes case/punctuation.
- Live QA receipt: docs/qa/055-async-generation-live-2026-06-20.md.

Still open (operational receipts, not code):

- Oracle 1: a fresh *judged field run* (scored batch) showing distractor quality
  and keep rate beat the 2026-06-11 baseline, committed under docs/evals/. The
  live walk showed one clean MCQ (Alfa/Amber/Atlas/Apollo), not a scored run.
- Oracle 4: record a production p50; if it still exceeds ~10s, the deeper
  candidate (parallel per-chunk model calls) is the next lever — deferred here.
- Oracle 5: confirm the full 12-source bench (deterministic + model judge) as the
  comparison artifact.
- Durability/retention of the new job queue is tracked separately in 057.
