# Statistical rigor for the generation bench

Priority: P2 · Status: pending · Estimate: M

## Goal

The generation bench reports judge scores as bare point estimates (a 1–5 Likert
for distractors/quality, a keep %) with no confidence interval, no paired
comparison against a baseline, and a model judge that has never been calibrated
against human labels. So a receipt can read "distractors 3.8 → 4.1, improved"
when the delta is pure noise at n≈12 sources. Make the bench's numbers honest,
per the eval-rigor doctrine now in the harness
(`harness-kit/harnesses/shared/references/verification-system-first.md`,
"Eval & Benchmark Rigor").

## Oracle

- [ ] Every rate in the receipt (keep rate) carries a confidence interval —
      `SE = sqrt(p*(1-p)/n)`, clustered by source — printed in the receipt.
- [ ] Version comparison is paired per source against a named baseline
      (McNemar / paired bootstrap), and the receipt labels any delta inside the
      CI as "within noise" instead of reporting it as a change.
- [ ] Distractor / question-quality judging is binary pass/fail per atomic
      criterion (not a 1–5 Likert), with the judge writing its rationale before
      the verdict.
- [ ] The judge is calibrated against a small human-labeled holdout (target
      Cohen's κ ≈ 0.80; report TPR/TNR, then bias-correct the rate), with the
      calibration receipt committed under `docs/evals/`.
- [ ] The suite's power is documented: ~12 sources detects only large
      regressions; note the n needed for a ~3% delta and treat the suite as a
      large-regression guard, not a small-improvement detector.

## Notes

Surfaced while codifying eval/benchmark design principles into the harness.
Already done right: the bench enforces judge ≠ generator family
(`same_model_family`) and keeps deterministic judges as the model-free first
line. The gap is purely in how the *numbers* are sized and read. This ticket
either gives the bench the teeth to support 055 oracle item 1 (a real
distractor/keep-rate improvement claim) or reframes that oracle as
"no large regression" — see the 2026-06-21 rigor receipt under docs/evals/.
