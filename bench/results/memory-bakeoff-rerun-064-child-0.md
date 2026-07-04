# Ticket 064 child 0 model bakeoff rerun

Date: 2026-07-03
Refs-backlog: 064

## Verdict

Numbers only. No production model default or Fly secret was changed in this
run.

The current production default, `google/gemini-3.5-flash`, remains the safest
choice from this 13-source bench pass. `google/gemini-2.5-flash-lite` is the
only cheap candidate that completed all 13 judged source comparisons with a
keep-rate delta still inside the paired 95% CI, but it had a lower point keep
rate, lower question/distractor means, and a large count-in-range regression
(4/13 vs 11/13). `mistralai/mistral-small-3.2-24b-instruct` and
`deepseek/deepseek-chat` were detectable keep-rate regressions. `openai/gpt-oss-120b`
judged only 5 sources and had poor provenance/answerability, so its "within
noise" keep delta is not a promotion signal.

## Comparison

All runs used:

```sh
cargo run -p memory-engine-bench -- generation \
  --model <model> \
  --judge anthropic/claude-sonnet-4.6 \
  --baseline docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md \
  --out docs/evals/generation-064-bakeoff-<model>-2026-07-03.md
```

The prod-default baseline was run without `--baseline` first, then every
candidate was paired against that receipt by source id.

| model | catalog price per 1M input/output | judged n | keep rate, 95% CI | paired keep delta vs prod, 95% CI | judge means F/Q/D | provider cost | p50/p95 latency | count in range | content fit | decision read |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `google/gemini-3.5-flash` | $1.50 / $9.00 | 13 | 80% +/- 11pp | baseline | 5.0 / 4.9 / 3.8 | $0.1490 | 1161ms / 2009ms | 11/13 | 0/3, 67% coverage | production baseline |
| `openai/gpt-oss-120b` | $0.03 / $0.15 | 5 | 65% +/- 18pp | -19.7pp +/- 34.0pp, within noise | 5.0 / 4.4 / 3.7 | $0.0094 | 742ms / 4501ms | 3/13 | 0/3, 33% coverage | reject: too few judged sources; 38% provenance, 33% answerability |
| `mistralai/mistral-small-3.2-24b-instruct` | $0.075 / $0.20 | 11 | 61% +/- 12pp | -21.1pp +/- 16.0pp, detectable regression | 4.8 / 4.6 / 3.6 | $0.0051 | 445ms / 3350ms | 7/13 | 0/3, 40% coverage | reject: detectable keep-rate regression |
| `google/gemini-2.5-flash-lite` | $0.10 / $0.40 | 13 | 68% +/- 14pp | -12.1pp +/- 16.1pp, within noise | 4.9 / 4.5 / 3.5 | $0.0137 | 665ms / 2128ms | 4/13 | 0/3, 67% coverage | cheapest complete candidate inside keep-rate noise; not an automatic swap |
| `deepseek/deepseek-chat` | $0.2002 / $0.8001 | 10 | 64% +/- 15pp | -13.8pp +/- 5.9pp, detectable regression | 4.8 / 4.5 / 3.5 | $0.0104 | 320ms / 1686ms | 9/13 | 0/3, 33% coverage | reject: detectable keep-rate regression |

## Evidence

Repo receipts:

- `docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md`
- `docs/evals/generation-064-bakeoff-gpt-oss-120b-2026-07-03.md`
- `docs/evals/generation-064-bakeoff-mistral-small-3.2-24b-2026-07-03.md`
- `docs/evals/generation-064-bakeoff-gemini-2.5-flash-lite-2026-07-03.md`
- `docs/evals/generation-064-bakeoff-deepseek-chat-2026-07-03.md`

External evidence copies:

- `~/.factory-lanes/wave2/bakeoff-evidence/openrouter-models-2026-07-03.json`
- `~/.factory-lanes/wave2/bakeoff-evidence/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md`
- `~/.factory-lanes/wave2/bakeoff-evidence/generation-064-bakeoff-gpt-oss-120b-2026-07-03.md`
- `~/.factory-lanes/wave2/bakeoff-evidence/generation-064-bakeoff-mistral-small-3.2-24b-2026-07-03.md`
- `~/.factory-lanes/wave2/bakeoff-evidence/generation-064-bakeoff-gemini-2.5-flash-lite-2026-07-03.md`
- `~/.factory-lanes/wave2/bakeoff-evidence/generation-064-bakeoff-deepseek-chat-2026-07-03.md`

## Commands

Catalog snapshot:

```sh
curl -fsS https://openrouter.ai/api/v1/models \
  -H "Authorization: Bearer ${OPENROUTER_API_KEY}" \
  -o ~/.factory-lanes/wave2/bakeoff-evidence/openrouter-models-2026-07-03.json
```

Bench commands:

```sh
cargo run -p memory-engine-bench -- generation \
  --model google/gemini-3.5-flash \
  --judge anthropic/claude-sonnet-4.6 \
  --out docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md

timeout 900 cargo run -p memory-engine-bench -- generation \
  --model openai/gpt-oss-120b \
  --judge anthropic/claude-sonnet-4.6 \
  --baseline docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md \
  --out docs/evals/generation-064-bakeoff-gpt-oss-120b-2026-07-03.md

timeout 900 cargo run -p memory-engine-bench -- generation \
  --model mistralai/mistral-small-3.2-24b-instruct \
  --judge anthropic/claude-sonnet-4.6 \
  --baseline docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md \
  --out docs/evals/generation-064-bakeoff-mistral-small-3.2-24b-2026-07-03.md

cargo run -p memory-engine-bench -- generation \
  --model google/gemini-2.5-flash-lite \
  --judge anthropic/claude-sonnet-4.6 \
  --baseline docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md \
  --out docs/evals/generation-064-bakeoff-gemini-2.5-flash-lite-2026-07-03.md

timeout 900 cargo run -p memory-engine-bench -- generation \
  --model deepseek/deepseek-chat \
  --judge anthropic/claude-sonnet-4.6 \
  --baseline docs/evals/generation-064-bakeoff-gemini-3.5-flash-2026-07-03.md \
  --out docs/evals/generation-064-bakeoff-deepseek-chat-2026-07-03.md
```

## Notes

- The bench corpus is currently 13 sources, including `apostles-creed`; older
  receipts in `docs/evals/` used 12 sources.
- `openai/gpt-oss-120b` returned many zero-accepted source rows and only 5
  judged source rows. Its paired CI includes 0 only because the judged sample is
  sparse; the full receipt metrics do not support promotion.
- No production default, runtime code, Fly secret, or model pin was changed.
