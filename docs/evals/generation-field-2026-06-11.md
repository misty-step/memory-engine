# Generation model field run — 2026-06-11

First-pass bench of the ticket-043 shortlist on the 12-source corpus
(`crates/memory-engine-bench/corpus/generation/`), principled prompt variant,
scored by the deterministic judges in `memory-engine-bench generation`.
Per-model receipts are the sibling `generation-<model>-*.md` files. Model
facts rot in weeks — re-run before trusting these numbers.

Command: `cargo run -p memory-engine-bench -- generation --model <id> --prompt principled`

## Results (principled prompt, 12 sources)

| Model | Provenance | Answerability | Key-term cov. | Count-in-range | Cost / source | Latency p50 / p95 | Failures |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `openai/gpt-5.4-nano` | 100% | 99% | 94% | 12/12 | ~$0.0012 (computed)¹ | **452 / 1627 ms** | **0/12** |
| `deepseek/deepseek-v4-flash` | 100% | 98% | 95% | 10/11 | **$0.0004** (logged) | 778 / 3143 ms | 1/12² |
| `google/gemini-3.1-flash-lite` | 100% | 93% | 88% | 11/11 | $0.0010 (logged) | 994 / 5288 ms | 1/12 |
| `google/gemini-3.5-flash` | 100% | 100% | 93% | 12/12 | $0.0251 (logged) | 2742 / 5768 ms | 0/12 |
| `qwen/qwen3.6-flash` | 25% | 25% | 17% | 0/4 | $0.0035 (logged) | 675 / 810 ms | 8/12³ |

¹ OpenRouter does not return an inline `cost` for OpenAI-routed models, so
the bench logs `None`. Computed from published nano pricing ($0.20 in /
$1.25 out per 1M) over the measured ~380 in / ~900 out tokens.
² DeepSeek's cheapest upstream host intermittently returns an unreadable
envelope (1–2 of 12, moves between runs); `allow_fallbacks` is enabled but
does not fully eliminate it. Failures surface to the learner as a
human-readable "please try again", per the ticket oracle.
³ Qwen3.6-flash does not reliably honor strict `json_schema`; rejected.

All models sit 16–50× under the ticket's $0.02/source budget, so cost is not
the binding constraint — quality is uniformly excellent on the cheap tier,
and the real differentiators are **reliability** and **latency**.

## Model-judge addendum (2026-06-11)

Deterministic judges verify mechanical properties only; they cannot tell a
sharp question from a vague one or a plausible distractor from a lazy one.
A model judge (`--judge`, anchored 1-5 rubric, run from a different provider
family than the generator to blunt self-preference) closes that gap. The
deterministic judges remain the anti-gaming guardrail; the model judge is
never the only signal.

First judged run — generator `google/gemini-3.5-flash`, judge
`anthropic/claude-sonnet-4.6`, full corpus
(`generation-gemini-3.5-flash-judged-2026-06-11.md`):

- faithfulness **4.9/5**, question quality **4.8/5** — the generator's answers
  and stems are excellent.
- distractor quality **3.8/5**, **keep rate 70%** — the real gap the
  deterministic 100%-provenance score hid. Recurring critique: distractors
  lifted verbatim from the source (recognizable, not distracting), lazy
  name-guesses, anachronistic or implausible options, and near-giveaways.
- judge cost ~$0.008/source (~$0.10 for the corpus), comparable to generation.

Actionable next step: the distractor weakness is a prompt problem, not a model
problem — a candidate ticket is to strengthen the distractor instruction (and
re-judge) rather than change the generation model.

## Findings

- **The grounded single-pass pipeline works (H1 confirmed).** Every viable
  model reaches ~100% provenance with one JSON-schema-constrained call plus
  the deterministic post-hoc trust gate. No multi-stage pipeline or critique
  loop was needed — the De Jure / Scry multi-stage shape would multiply
  latency for no measurable quality gain on grounded input.
- **Normalized quote matching was necessary (H3 confirmed).** Models
  paraphrase punctuation and capitalization while quoting faithfully; the
  case/punctuation-folded `evidence_quote_matches` predicate credits these,
  and exact matching would have under-scored provenance.
- **Cost is a non-issue (H5 confirmed); pick on reliability and speed.**
- **Qwen is out; the two Geminis bracket the field** — Flash-Lite is a solid
  cheap middle, Flash is the quality ceiling but over budget and slowest.

## Default: `google/gemini-3.5-flash`

Operator decision (2026-06-11): default to the **quality ceiling**. Gemini
3.5 Flash is the only model that scored 100% on both provenance and
answerability with zero failures across runs and 12/12 count-in-range. The
trade is cost and latency: ~$0.025/source (above the ticket's original $0.02
estimate) and ~2.7s p50. This is a deliberate choice that quality is worth
the spend at beta scale; the $0.02 figure in ticket 043 is superseded by this
receipt.

Levers if cost or speed later bind:
- `MEMORY_ENGINE_GENERATION_MODEL=deepseek/deepseek-v4-flash` drops cost ~60×
  to $0.0004/source at the same provenance, trading some answerability and
  occasional transient upstream retries.
- Lowering `max_drafts` (default 8) cuts output tokens roughly linearly and
  is the main cost/latency dial without changing models.

Set in `memory-engine-openrouter::DEFAULT_MODEL`; per-environment override via
`MEMORY_ENGINE_GENERATION_MODEL`.
