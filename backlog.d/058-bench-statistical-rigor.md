# Statistical rigor for the generation bench

Priority: P2 · Status: in progress · Estimate: M

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

- [x] The keep rate carries a 95% confidence interval, source-clustered
      (Student-t, df = n-1), printed in the receipt.
- [x] `--baseline <receipt>` pairs per source against a prior run and prints the
      mean keep-rate delta with its CI, labelling a delta inside the CI as
      "within noise" instead of a change.
- [ ] Distractor / question-quality judging is binary pass/fail per atomic
      criterion (not a 1–5 Likert), with the judge writing its rationale before
      the verdict. **Coupled to the next item** — a binary judge you cannot
      validate is no more trustworthy than the Likert one.
- [ ] The judge is calibrated against a small human-labeled holdout (target
      Cohen's κ ≈ 0.80; report TPR/TNR, then bias-correct the rate), with the
      calibration receipt committed under `docs/evals/`. **User-gated**: needs
      ~30–50 operator pass/fail labels on generated cards.
- [x] The suite's power is documented: the receipt prints a power note (~12
      sources detects only large regressions; a ~3pp delta needs ~1000 drafts;
      read it as a large-regression guard).

## Notes

Surfaced while codifying eval/benchmark design principles into the harness.
Already done right: the bench enforces judge ≠ generator family
(`same_model_family`) and keeps deterministic judges as the model-free first
line. The gap is purely in how the *numbers* are sized and read. This ticket
either gives the bench the teeth to support 055 oracle item 1 (a real
distractor/keep-rate improvement claim) or reframes that oracle as
"no large regression" — see the 2026-06-21 rigor receipt under docs/evals/.

## Progress — feat/058-bench-statistical-rigor (2026-06-21)

Shipped: a `stats` module (source-clustered Student-t CI, paired-vs-baseline
verdict, receipt keep-rate parser) wired into the generation receipt behind
`--baseline`. The receipt now prints the keep-rate CI, the paired verdict
(within noise / detectable), and a power note. Oracles 1, 2, 5 met.

Remaining (oracles 3 + 4, a coupled pair): the binary-criteria judge and its
human calibration. Deferred because an uncalibrated binary judge violates the
rigor it implements, and calibration needs ~30–50 operator labels.
