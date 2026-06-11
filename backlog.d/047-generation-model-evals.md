# Build a generation eval harness and bench the cheap-model field

Priority: P1 · Status: pending · Estimate: M

## Goal

Model choice for prose→quiz generation is decided by receipts: a repeatable
eval that scores candidate models on quality, faithfulness, and cost over a
fixed corpus, runnable on demand as models churn.

## Oracle

- [ ] An eval corpus of ≥10 real-world source texts (prose, lists, technical
      doc, narrative; varied length) lives in the repo with expected-property
      annotations.
- [ ] Deterministic judges score each run with no model in the loop:
      JSON-schema validity rate, provenance-quote-actually-in-source rate,
      duplicate rate, question-answerable-from-source rate (string-level
      checks), draft count distribution.
- [ ] `cargo run -p memory-engine-bench -- generation --model <id>` (or
      equivalent) produces a per-model receipt: scores + tokens + dollars.
- [ ] At least 4 models from the 043 shortlist benched, results committed
      under docs/evals/ with dates; a default model is named with evidence.
- [ ] CI never calls live models; eval runs are explicit and local.

## Notes

The repo already has memory-engine-bench and docs/evals.md as the receipts
convention — extend them, don't invent a new harness. Deterministic judges
first; an LLM-judge lane can be added later but must never be the only
signal. Re-run cadence: model facts rot in weeks; receipts carry dates.
Depends on 043 child 3 (the HTTP provider) for live runs, but the corpus and
judges can land first against the fake provider.

## Children

1. Corpus + annotations.
2. Deterministic judge suite (tested against hand-built good/bad outputs).
3. Bench command with per-model cost/quality receipt output.
4. First field run across ≥4 shortlist models; commit receipts; pick default.
