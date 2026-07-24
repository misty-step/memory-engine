# Generation 061 live comparison receipt — 2026-07-15

## Status

The deterministic proof is green, but the required live model-backed comparison
was not run. A credential-safe environment check found
`OPENROUTER_API_KEY=absent` and no `MEMORY_ENGINE_GENERATION_MODEL` override.
The configured default model is `google/gemini-3.5-flash`; no credential value
was printed or persisted.

## Exact commands

Deterministic corpus receipt:

```sh
cargo run --quiet -p memory-engine-bench -- generation \
  --out /tmp/memory-engine-061-deterministic-2026-07-15.md
```

Required live comparison once provider authority exists:

```sh
cargo run --quiet -p memory-engine-bench -- generation \
  --model google/gemini-3.5-flash \
  --prompt principled \
  --out docs/evals/generation-061-live-comparison-2026-07-15.md
```

## Deterministic corpus cases

Provider: `fixture/fake-model` · corpus: 14 sources · provider failures: 0.
The runtime runner, trust gate, repair path, and source-owned coverage policy
were exercised.

| case | classification | accepted | coverage | direction | provenance |
| --- | --- | ---: | --- | --- | --- |
| `mitochondria` | concept_understanding | 3 | N/A | N/A | 100% |
| `nato-alphabet` | enumerable_set | 26 | 26/26 | pass | 100% |
| `sourdough` | procedure_process | 6 | N/A | N/A | 100% |
| `hope-feathers` | verbatim_memorization | 8 | N/A | N/A | 100% |
| `apostles-creed` | verbatim_memorization | 6 | 6/6 | pass | 100% |
| `us-presidents-ordinal` | fact_recall with ordinal finite-set policy | 47 | 47/47 | pass | 100% |

The 47-president scorer reports expected `47`, observed `47`, covered `47`,
missing `0`, duplicates `0`, invented `0`, misassigned `0`, reversed `0`,
ordered `yes`, direction `yes`, pass `yes`. Repeated Cleveland and Trump
names remain ordinally disambiguated.

## Red residuals

- Required live comparison is blocked by the absent `OPENROUTER_API_KEY`; no
  model-backed classification, coverage, or direction result is claimed.
- The deterministic corpus receipt still reports `count-in-range: NO` for
  `hope-feathers` and `spacing-effect`; these are existing draft-count oracle
  residuals, not suppressed or reclassified here.
- The OpenRouter live test remains intentionally ignored without credentials.
