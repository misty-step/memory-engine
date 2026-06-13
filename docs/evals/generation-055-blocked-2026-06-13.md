# Generation 055 blocked receipt — 2026-06-13

Backlog 055 requires a fresh judged field run that improves both distractor
quality and keep rate over `docs/evals/generation-field-2026-06-11.md`, plus a
production end-to-end latency receipt.

Baseline judged receipt:

- Command: `cargo run -p memory-engine-bench -- generation --model google/gemini-3.5-flash --prompt principled --judge anthropic/claude-sonnet-4.6`
- Artifact: `docs/evals/generation-gemini-3.5-flash-judged-2026-06-11.md`
- Judge means: distractors 3.825, keep rate 70.5%.

Field attempts run on 2026-06-13 did not clear both judged metrics:

| Generator | Variant | Distractors | Keep rate | Outcome |
| --- | --- | ---: | ---: | --- |
| `google/gemini-3.5-flash` | prompt-strengthened experiment | 3.35 | 75.6% | Keep improved, distractors regressed. |
| `google/gemini-3.5-flash` | prompt-strengthened + `--max-drafts 4` | 3.46 | 72.3% | Keep improved, distractors regressed. |
| `openai/gpt-5.4-nano` | prompt-strengthened | 3.81 | 67.7% | Distractors near baseline, keep regressed. |
| `openai/gpt-5.4-nano` | prompt-strengthened rerun | 3.75 | 72.3% | Keep improved, distractors regressed. |
| `openai/gpt-5.4-mini` | prompt-strengthened | 3.53 | 65.5% | Both judged metrics regressed. |
| `openai/gpt-5.4` | stable prompt | 3.40 | 81.0% | Keep improved, distractors regressed. |
| `google/gemini-3.5-flash` | same-model full polish experiment | 3.5 | 82% | Keep improved, distractors and deterministic shape regressed. |
| `google/gemini-3.5-flash` | same-model distractor-only merge experiment | 3.2 | 71% | Keep barely improved, distractors and deterministic shape regressed. |
| `google/gemini-3.5-flash` + `openai/gpt-5.4` editor | cross-model distractor-only experiment | 3.5 | 78% | Keep improved, distractors and deterministic shape regressed. |
| `google/gemini-3.5-flash` + `openai/gpt-5.4` editor | cross-model full-editor experiment | 3.3 | 78% | Keep improved, distractors and deterministic shape regressed. |
| `google/gemini-3.5-flash` | retained code path after reverting polish/editor experiments | 3.4 | 78% | Keep improved, distractors and deterministic shape regressed. |

The prompt-strengthened experiments were reverted because they did not satisfy
the judged distractor oracle. The polish/editor experiments were also reverted
because they repeatedly lowered distractor quality and caused deterministic
intent-shape regressions. The retained implementation work is limited to:

- accepted-material near-duplicate filtering in source generation;
- one bounded repair request after a source produces zero accepted first-pass
  drafts, with usage merged into the run;
- an explicit `--max-drafts` bench flag for future field sweeps.

Production latency attempt:

```sh
base="https://memory-engine-api.fly.dev"
curl -fsS --max-time 20 \
  -H 'content-type: application/json' \
  -d '{"email":"latency-<stamp>@memory-engine.local"}' \
  "$base/v1/accounts"
```

Result: HTTP 403, so no production generation latency number was recorded.
The deployed v1 account-creation path is allowlist-protected; an allowlisted
account/session token or a production-safe latency harness is required before
this ticket can close.

The runbook has been corrected to require `MEMORY_ENGINE_ACCOUNT_ID` and
`MEMORY_ENGINE_SESSION_TOKEN` instead of creating a throwaway production
account. This shell did not have those credentials exported, so no production
latency receipt was generated.

Current status: blocked. Do not archive backlog 055 or merge this branch until
both required evidence gates are satisfied.
