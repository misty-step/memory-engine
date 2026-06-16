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
| `google/gemini-3.5-flash` | production-gated bench, OpenAI judge | 3.0 | 54% | Proved the old bench bypassed production filtering/repair, but judged quality regressed. Artifact: `docs/evals/generation-gemini-3.5-flash-production-gated-judged-2026-06-13.md`. |
| `google/gemini-3.5-flash` | production-gated, max 5, atomic prompt, Anthropic judge | 3.3 | 76% | Keep improved, distractors still below baseline. Artifact: `docs/evals/generation-gemini-3.5-flash-judged-2026-06-13.md`. |
| `anthropic/claude-sonnet-4.6` | production-gated, max 5, atomic prompt, OpenAI judge | 3.3 | 70% | Model swap did not improve judged quality enough and had a 6.0s p95 outlier. Artifact: `docs/evals/generation-claude-sonnet-4.6-judged-2026-06-13.md`. |
| `google/gemini-3.5-flash` | one-pass expert item-writer prompt | 3.6 | 69% | Better distractors than the retained 3.3 path, but keep regressed and deterministic shape was 11/12. Failed experiment removed after review. |
| `google/gemini-3.5-flash` | one-pass item-writer + required audit fields | 3.5 | 61% | Structured rationale/self-check fields made output worse. Failed experiment removed after review. |
| `openai/gpt-5.4` | one-pass item-writer + required audit fields | 3.6 | 72% | Stronger model improved keep but still missed distractor baseline. Failed experiment removed after review. |
| `openai/gpt-5.4` | one-pass expert item-writer prompt | 3.5 | 64% | Removing audit fields did not improve distractors or keep. Failed experiment removed after review. |
| `openai/gpt-5.4` | one-pass item-writer + partial repair for rejected candidates | 3.4 | 63% | Repair-on-any-rejection plus safe MCQ gates did not lift judged quality. Failed experiment removed after review. |

The polish/editor experiments were reverted because they repeatedly lowered
distractor quality and caused deterministic intent-shape regressions. The
retained implementation work is limited to:

- production-gated generation bench scoring: deterministic and model judges now
  score accepted runtime drafts after duplicate filtering and repair instead of
  raw provider output;
- accepted-material near-duplicate filtering in source generation;
- one bounded repair request for rejected first-pass drafts, with usage merged
  into the run, even when the same source also produced accepted drafts;
- cheap MCQ trust-gate rejection for compound questions and distractors that
  duplicate the correct answer;
- a lower default model draft budget of 5, plus atomic-card and distractor
  guardrails in the principled prompt; this is a 055 follow-up to the June 11
  field note that identified `max_drafts` as the main cost/latency dial;
- explicit `--max-drafts` bench support for future field sweeps. The failed
  item-writer prompt experiment is summarized here as negative evidence; its
  live bench flag, provider prompt surface, and non-reproducible receipt files
  were removed after review because no judged receipt cleared the oracle.

The 2026-06-15 one-pass experiments were run because the likely failure could
have been prompt/context engineering rather than model capability. The retained
result does not support shipping a prompt-only/model-swap fix: the best
one-pass distractor score was 3.6, still below the 3.825 baseline, and the
strongest keep-rate result still missed distractor quality.

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

Current status: blocked. Infrastructure and evidence from this branch may land,
but do not archive backlog 055 or claim the ticket is shipped until both
required evidence gates are satisfied.
