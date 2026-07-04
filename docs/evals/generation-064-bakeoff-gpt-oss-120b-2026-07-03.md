# Generation eval receipt

- Provider: openrouter/openai/gpt-oss-120b (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 13 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 8 | 0 | 0 | 100% | 100% | 88% | 0% | yes | 100% | NO | NO | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 1016/1966 | $0.0013 | 1246ms |
| nato-alphabet | enumerable-list | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | yes | NO | 0/26 (0%) | NO | yes | 0% | 100% | 100% | 1046/1362 | $0.0004 | 1263ms |
| http-caching | technical-doc | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 1019/1295 | $0.0002 | 302ms |
| rubicon | narrative-history | 9 | 3 | 0 | 75% | 100% | 100% | 0% | NO | 75% | yes | — | — | — | — | 0% | 100% | 100% | 2194/4517 | $0.0016 | 1350ms |
| sourdough | how-to | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | NO | — | — | — | — | 0% | 100% | 100% | 0/0 | — | 0ms |
| gdpr-basis | regulatory | 15 | 1 | 1 | 88% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 1010/3106 | $0.0005 | 4501ms |
| hope-feathers | verbatim-verse | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | NO | — | — | — | — | 0% | 100% | 100% | 952/1543 | $0.0011 | 353ms |
| pythagorean | math-concept | 8 | 1 | 0 | 89% | 100% | 75% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 2164/4826 | $0.0025 | 958ms |
| curie | biography | 0 | 1 | 0 | 0% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 2162/1264 | $0.0011 | 1696ms |
| git-branching | product-doc | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 1004/1372 | $0.0004 | 742ms |
| spacing-effect | long-science-prose | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 0/0 | — | 0ms |
| water-boiling | tiny-fact | 3 | 0 | 0 | 100% | 100% | 67% | 0% | yes | 50% | NO | — | — | — | — | 0% | 100% | 100% | 915/1151 | $0.0002 | 306ms |
| apostles-creed | verbatim-sequential | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | yes | yes | 0/6 (0%) | NO | yes | 0% | 100% | 100% | 969/672 | $0.0001 | 733ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.9 | 3.6 | 75% | $0.0106 |
| rubicon | 5.0 | 4.6 | 3.4 | 67% | $0.0107 |
| gdpr-basis | 4.9 | 3.1 | 3.4 | 40% | $0.0157 |
| pythagorean | 4.9 | 4.4 | 3.9 | 75% | $0.0103 |
| water-boiling | 5.0 | 5.0 | 4.0 | 67% | $0.0052 |

- Judge means: faithfulness 5.0 · question quality 4.4 · distractors 3.7 · keep rate 65% · judge cost $0.0525
- Keep rate (source-clustered, n=5): 65% (95% CI ±18pp)
- Paired vs baseline (5 sources): keep Δ -19.7pp (95% CI ±34.0pp) — **within noise** — CI includes 0 (not detected; not proof of no change)
- Power: ~5 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- mitochondria: draft 5: Distractors like 'spillover theory' and 'viral incorporation theory' are obscure or invented terms a learner would not genuinely confuse with the endosymbiotic theory.
- mitochondria: draft 8: Distractors (photosynthesis, cell division, protein synthesis) are obviously unrelated to cell death and would not fool any learner.
- rubicon: draft 2: Distractors are paraphrases of the correct answer rather than genuinely distinct alternatives.
- rubicon: draft 3: Distractors are implausible and don't represent real confusions a learner would have.
- rubicon: draft 8: Distractors are too numerically close and obvious; learners need conceptual foils, not adjacent numbers.
- gdpr-basis: draft 6: Distractors like 'impact assessment' and 'risk analysis' are not clearly wrong enough to challenge a knowledgeable learner, and 'documented' aspect from source is dropped.
- gdpr-basis: draft 7: Distractors are weak — 'national law' and 'contractual obligations' are not realistic confusions a learner of GDPR Article 6 would make.
- gdpr-basis: draft 8: Distractors are weak and unconvincing for a fill-in-the-blank item about a simple phrase.
- gdpr-basis: draft 10: Asking for ordinal position in a list tests rote memorization of ordering not emphasized in the source.
- gdpr-basis: draft 11: Ordinal-position question tests list order not meaningfully emphasized in the source.
- gdpr-basis: draft 12: Ordinal-position question tests list order not meaningfully emphasized in the source.
- gdpr-basis: draft 13: Ordinal-position question tests list order not meaningfully emphasized in the source.
- gdpr-basis: draft 14: Ordinal-position question tests list order not meaningfully emphasized in the source.
- gdpr-basis: draft 15: Ordinal-position question tests list order not meaningfully emphasized in the source.
- pythagorean: draft 6: Distractors 'flat surfaces' and 'planar surfaces' are synonyms and obviously wrong, making the item too easy.
- pythagorean: draft 8: Short-answer format is appropriate but this trivial recall item tests symbol manipulation rather than conceptual understanding.
- water-boiling: draft 3: Distractors are weak—'because temperature is higher' is nearly incoherent and would not fool a real learner.

## Totals

- Provider failures: 0/13 sources
- Mean provenance: 38% · mean answerability: 33% · mean key-term coverage: 33% · count-in-range: 3/13
- Intent shape matches: 9/13 sources
- Content fit matches: 0/3 sources · mean required-unit coverage 33%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0094 · mean per source: $0.0007
- Latency p50: 742ms · p95: 4501ms
