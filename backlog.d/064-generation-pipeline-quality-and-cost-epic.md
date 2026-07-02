# Generation pipeline quality + cost — epic

Priority: P1 · Status: pending · Estimate: L

## Goal

Roll up the remaining work to make source-backed generation reliably produce
high-quality, content-appropriate material at an affordable per-generation
cost. This is an umbrella over already-shaped tickets plus one net-new
research item; it does not duplicate their oracles.

## Oracle

- [ ] 060 (content-type/coverage/shape/directionality evals) ships and is
      **green**, proving the eval suite catches the creed/NATO misclassification
      class of bug before generation changes, not after.
- [ ] 061 (verbatim/sequential/enumerable generation) ships and 060's suite
      goes green on real output, not fixtures.
- [ ] A live OpenRouter model-selection pass: pull current pricing for
      generation-suitable models via the OpenRouter models API, compare against
      the production default (`google/gemini-3.5-flash`, per
      `docs/runbook.md`) on cost-per-generation and 058's judged quality
      metrics (keep-rate CI, distractor cohesion), and record a decision
      (keep or switch) with the pricing snapshot as evidence. Do not assume a
      cheaper model exists without pricing evidence — none was found
      pre-verified in this repo or the fleet at investigation time.
- [ ] Production cost-per-generation is measured and recorded (tokens in/out ×
      current price) alongside the existing latency runbook procedure, so
      "affordable" has a number attached, not a feeling.
- [ ] No regression on the existing 055/058 bench gates.

## Notes

Operator's words (near-verbatim, binding), relayed via groom dispatch
2026-07-02: "needs substantial work to be actually usable — a lot of that is
building the AI content generation pipeline and evals to make sure it's
generating high-quality content in an affordable manner, then optimizing the
user experience."

**Repo-fit correction:** the generation pipeline and its evals are not
greenfield. `crates/memory-engine-generation` and `crates/memory-engine-bench`
already exist and are dogfooded in production (`memory-engine-api.fly.dev`).
Ticket history (043, 047, 055, 057, 058, 059) shows model-backed generation,
judge calibration with Cohen's kappa, keep-rate confidence intervals, a
durable job queue, and a production quality/latency pass already shipped. The
open, real gap is narrower than "build the pipeline": 060/061 (content-type
coverage — verbatim/enumerable material is misgenerated as conceptual
trivia) are the live quality hole, already shaped and sitting in backlog.
This epic exists to sequence and close them, not to originate net-new
generation-pipeline scope.

The model-cost research item is net new. No document in this repo, the
fleet checkouts under `~/Development`, or the daybook vault names a "cheap
tier" OpenRouter pick for content generation as of this investigation
(2026-07-02) — `bitterblossom/scripts/check-model-catalog.sh` validates agent
model configs against the live catalog but is not a cost-tier
recommendation list. Do this research live against the OpenRouter API rather
than citing a document that could not be found.

## Children

1. Land 060 (content-type/coverage evals) — already shaped, ready to deliver.
2. Land 061 (verbatim/sequential generation) — already shaped, blocked on 060.
3. Live OpenRouter pricing pull + cost-per-generation comparison vs current
   default model.
4. Production cost-per-generation measurement, recorded alongside latency.


## Lead groom review (2026-07-02, supervisor)

A July-2026 model/pricing research pass EXISTS:
`~/.factory-lanes/wave1/landmark-model-refresh.md` (OpenRouter-priced;
deepseek-v4-flash $0.089/$0.18 per 1M for high-volume structured work,
haiku-4.5 $1/$5 fallback, sonnet-5 $2/$10 for quality prose). Scoped to
landmark's stages but the price table transfers; start generation-cost work
from it rather than re-researching. The verbatim/enumerable misgeneration
(061 — creed/NATO rendered as conceptual trivia) is this epic's centerpiece:
it is a CORRECTNESS failure for memorization content, not a polish item —
sequence it first.
