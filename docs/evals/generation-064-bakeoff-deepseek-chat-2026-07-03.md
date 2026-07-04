# Generation eval receipt

- Provider: openrouter/deepseek/deepseek-chat (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 13 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | NO | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 950/563 | $0.0008 | 216ms |
| nato-alphabet | enumerable-list | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | yes | NO | 0/26 (0%) | NO | yes | 0% | 100% | 100% | 0/0 | — | 0ms |
| http-caching | technical-doc | 7 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 1449/1004 | $0.0011 | 1158ms |
| rubicon | narrative-history | 0 | 0 | 0 | 100% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 964/19 | $0.0003 | 163ms |
| sourdough | how-to | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | NO | — | — | — | — | 0% | 100% | 100% | 0/0 | — | 0ms |
| gdpr-basis | regulatory | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 67% | yes | — | — | — | — | 0% | 100% | 100% | 965/661 | $0.0009 | 320ms |
| hope-feathers | verbatim-verse | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 67% | NO | — | — | — | — | 0% | 100% | 100% | 906/468 | $0.0007 | 294ms |
| pythagorean | math-concept | 5 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 67% | yes | — | — | — | — | 0% | 100% | 100% | 1435/809 | $0.0009 | 740ms |
| curie | biography | 7 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 1439/1021 | $0.0011 | 824ms |
| git-branching | product-doc | 5 | 1 | 0 | 83% | 100% | 100% | 0% | yes | 75% | yes | — | — | — | — | 0% | 100% | 100% | 2508/801 | $0.0013 | 1031ms |
| spacing-effect | long-science-prose | 9 | 1 | 0 | 90% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 3367/1467 | $0.0018 | 1686ms |
| water-boiling | tiny-fact | 2 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 1351/254 | $0.0005 | 933ms |
| apostles-creed | verbatim-sequential | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 67% | yes | NO | 0/6 (0%) | NO | yes | 0% | 100% | 100% | 921/658 | $0.0009 | 169ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 4.5 | 4.5 | 3.5 | 50% | $0.0072 |
| http-caching | 5.0 | 4.6 | 4.0 | 71% | $0.0097 |
| gdpr-basis | 4.8 | 3.8 | 4.0 | 83% | $0.0077 |
| hope-feathers | 4.5 | 4.5 | 2.8 | 25% | $0.0064 |
| pythagorean | 4.6 | 4.4 | 3.6 | 60% | $0.0083 |
| curie | 5.0 | 5.0 | 3.7 | 71% | $0.0099 |
| git-branching | 5.0 | 5.0 | 3.2 | 60% | $0.0070 |
| spacing-effect | 5.0 | 4.2 | 3.1 | 67% | $0.0120 |
| water-boiling | 4.5 | 5.0 | 4.0 | 100% | $0.0044 |
| apostles-creed | 4.7 | 4.2 | 3.3 | 50% | $0.0081 |

- Judge means: faithfulness 4.8 · question quality 4.5 · distractors 3.5 · keep rate 64% · judge cost $0.0807
- Keep rate (source-clustered, n=10): 64% (95% CI ±15pp)
- Paired vs baseline (10 sources): keep Δ -13.8pp (95% CI ±5.9pp) — **detectable** — CI excludes 0
- Power: ~10 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- mitochondria: draft 2: Distractors like 'Evolutionary synthesis' are too obscure to be realistic learner confusions.
- mitochondria: draft 3: Source says liver cells 'can contain more than a thousand' but doesn't claim they have the highest number of any human cell; muscle cells and neurons are well-known high-mitochondria cells, making the answer imprecise and the distractor 'muscle cells' arguably more correct.
- http-caching: draft 5: The answer is long and compound, making it awkward as a single-answer multiple-choice item.
- http-caching: draft 6: Distractors are generic and unlikely to represent real learner confusions about the Vary header specifically.
- gdpr-basis: draft 6: Answer oversimplifies: legitimate interests only applies when controller interests are NOT overridden, making the framing misleading.
- hope-feathers: draft 2: Distractors are invented phrases not grounded in any real confusion a learner would have about this poem.
- hope-feathers: draft 3: The answer paraphrases rather than quotes the source, and distractors are generic filler phrases unlikely to cause real confusion.
- hope-feathers: draft 4: 'The storm' is nearly synonymous with 'the Gale' making it a poor distractor, and 'the whisper' is too obviously wrong.
- pythagorean: draft 3: Distractors are generic made-up terms that no learner would seriously confuse with the correct answer.
- pythagorean: draft 4: The source says the theorem 'fails' on curved surfaces but notes angles summing beyond 180°; 'only for small triangles' hints at a nuance the source does not actually state, and yes/no format with weak distractors is poor.
- curie: draft 4: Distractor 'Discovering radioactivity' is partially true per the source (she coined the term), making it a confusing distractor.
- curie: draft 6: Distractors like 'Curie Mobiles' and 'Radium Rovers' are obviously invented and implausible, not genuine confusions a learner would make.
- git-branching: draft 2: Distractors TAIL and ROOT are nonsensical Git terms that no real learner would confuse with HEAD.
- git-branching: draft 5: Only two distractors provided and cherry-picking is not mentioned in the source, making it a weak distractor set.
- spacing-effect: draft 5: Distractors are trivially implausible and no real learner would confuse 'undesirable difficulty' or 'effortless learning' as coined terms.
- spacing-effect: draft 6: Distractors are weak combinations that no learner familiar with the topic would seriously consider.
- spacing-effect: draft 9: Short-answer format suits it but 'forgetting speed' is not a real term and 'retention probability' is also estimated, making distractors imprecise.
- apostles-creed: draft 2: 'The Virgin Mary' distractor is too obviously wrong since she is already named as the mother, not the conceiver.
- apostles-creed: draft 4: Distractors 'Elizabeth' and 'Leah' are too obscure or unrelated, making them easy to eliminate.
- apostles-creed: draft 6: The source also mentions being seated at the right hand of the Father, making 'ascended into heaven' an incomplete answer to what occurred after the resurrection.

## Totals

- Provider failures: 0/13 sources
- Mean provenance: 77% · mean answerability: 77% · mean key-term coverage: 65% · count-in-range: 9/13
- Intent shape matches: 9/13 sources
- Content fit matches: 0/3 sources · mean required-unit coverage 33%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0104 · mean per source: $0.0008
- Latency p50: 320ms · p95: 1686ms
