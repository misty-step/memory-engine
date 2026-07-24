# Generation 061 content-fit receipt — 2026-07-15

## Oracle

The source policy must classify finite mappings as `enumerable_set`, emit every
source entry in the non-derivable cue→answer direction, classify ordered quoted
text as `verbatim_memorization`, emit one exact recitation exercise per line or
sentence, preserve source evidence, and leave conceptual prose on the
fewer-better path.

## Deterministic result

Provider: `fixture/fake-model`
Corpus: 13 generation fixtures
Runtime trust gate: exercised through `run_beta_generation_with_provider`

| source | intent | accepted | coverage | shape | direction | provenance |
| --- | --- | ---: | --- | --- | --- | --- |
| `nato-alphabet` | enumerable_set | 26 | 26/26 (100%) | pass | pass | 100% |
| `apostles-creed` | verbatim_memorization | 6 | 6/6 (100%) | pass | pass | 100% |
| `mitochondria` | concept_understanding | 3 | not applicable | pass | pass | 100% |

All 13 sources had zero provider failures, zero rejected drafts, and 100%
runtime acceptance. The full receipt is reproducible with the command below;
the three rows above are the card-specific content-fit oracles.

## Exact commands

Red baseline reproduced before implementation:

```sh
cargo run --quiet -p memory-engine-bench -- generation
```

Observed before the change: NATO `6` accepted with `5/26` coverage and creed
`2` accepted with `0/6` coverage; conceptual mitochondria content fit passed.

Repository gates:

```sh
bun run ci
bun run ci:full
```

`bun run ci` passed on the final rerun: format, workspace tests, Clippy with
`-D warnings`, and rustdoc. `bun run ci:full` was attempted twice; both
attempts stopped before repository stages because the local Dagger engine
returned `buildctl dial-stdio ... input/output error`. Restarting that
disposable engine was also blocked by Docker returning the same I/O error while
reading its container metadata. Full container parity remains a handoff
blocker, not a claimed pass.

Focused deterministic and provider-boundary proof:

```sh
cargo test -p memory-engine-generation --test provider_generation
cargo test -p memory-engine-openrouter --test openrouter_provider
cargo test -p memory-engine-bench
cargo run --quiet -p memory-engine-bench -- generation
```

## Honest uncertainty

No live OpenRouter generation was run in this receipt; the live test remains
ignored unless `OPENROUTER_API_KEY` is present. The OpenRouter adapter is
covered with an external-boundary HTTP fixture that supplies incomplete or
misclassified model output and verifies deterministic source policy repairs it.
This proves the model path's policy seam, not the quality of any particular
model's classification of an unstructured topic. A finite set that is not
actually present in the captured source still relies on model knowledge and
human review. The 47-president child eval (memory-engine-084) was not claimed or
closed by this delivery.

The incremental card shape uses existing exact recitation/short-answer prompts;
it does not add a new cloze schema or review-UI card type.
