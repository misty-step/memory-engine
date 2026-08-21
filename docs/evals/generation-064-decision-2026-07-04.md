# Generation model decision — KEEP `google/gemini-3.5-flash` (2026-07-04)

Decision for legacy work item `memory-engine-064` child 0, written by the lead from the five-model
bakeoff receipts (`generation-064-bakeoff-*-2026-07-03.md`) and the rollup
(`bench/results/memory-bakeoff-rerun-064-child-0.md`), honoring `058`'s
statistical-rigor disclosures.

## Verdict

**Keep the production default at `google/gemini-3.5-flash`**
(`crates/memory-engine-openrouter/src/lib.rs`). No candidate presents a
defensible switch signal; two present defensible regression signals; the cost
delta is immaterial at this product's scale.

## The reasoning, stated honestly

This is a "no reason to move" decision, not a "prod is proven best" decision.
The bench is underpowered (n=13 sources; ±11–18pp CIs) and the judge is
uncalibrated (058's κ-vs-human-labels run has not happened). What the data
*can* support:

1. **The only two statistically defensible signals in the batch are both
   regressions.** `deepseek-chat` −13.8pp±5.9pp (the tightest CI in the set)
   and `mistral-small-3.2` −21.1pp±16.0pp both clear the noise floor, against
   the cheapest candidates' favor.
2. **The "within noise" candidates fail on defects the keep-rate CI doesn't
   see.** `gemini-2.5-flash-lite` collapses count-in-range 11/13 → 4/13
   (wrong number of items generated — a correctness-adjacent defect for
   verbatim/enumerable content). `gpt-oss-120b` was judged on only 5/13
   sources (silent zero-acceptable-draft misses excluded from its own
   average) with the worst provenance/answerability grounding of the batch;
   the rollup itself flags its delta as not a promotion signal.
3. **The cost argument doesn't reach the bar.** Full-run generation spend:
   $0.149 (prod) vs $0.005–0.014 (cheapest). Generation runs at content-ingest
   time on a personal-scale product; the annualized delta is coffee money. A
   switch would trade a measurable quality floor for savings that don't
   register.

## What would change this decision (recorded triggers)

- A calibrated judge (058's human-label run, 30–50 labels, operator-gated —
  tooling exists) materially shifting the keep-rate picture.
- A candidate arriving that is within noise on keep-rate **and** matches prod
  on count-in-range and hard-failure count at ≥5x cost advantage.
- Generation volume growing to where the cost delta exceeds ~$10/month —
  then rerun at a sample size 058's power math says can resolve ~5pp.

## Follow-ups this decision creates (legacy work items, not blockers)

- `058`: the judge-calibration run is now the highest-leverage bench
  investment — every verdict above inherits its uncertainty.
- Count-in-range should graduate from receipt footnote to a first-class
  bench gate (it caught what the CI missed).
- Content-fit scored 0/3 across ALL five models including prod — a shared
  eval/prompt gap, not a model differentiator; deserves its own small ticket.
- 064 child 0's remaining oracle items (Fly secret plumbing for a model
  override, production capture on a switched model) are moot under KEEP but
  the override mechanism is still worth having before any future switch.
