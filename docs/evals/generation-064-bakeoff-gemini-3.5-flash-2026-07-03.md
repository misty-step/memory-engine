# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 13 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | yes | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 1471/581 | $0.0074 | 1161ms |
| nato-alphabet | enumerable-list | 27 | 1 | 0 | 96% | 100% | 100% | 0% | NO | 100% | yes | NO | 26/26 (100%) | NO | yes | 0% | 100% | 100% | 3181/2506 | $0.0273 | 1939ms |
| http-caching | technical-doc | 6 | 1 | 0 | 86% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 3140/1075 | $0.0144 | 1735ms |
| rubicon | narrative-history | 5 | 1 | 0 | 83% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 3088/721 | $0.0111 | 1792ms |
| sourdough | how-to | 6 | 1 | 0 | 86% | 100% | 100% | 0% | yes | 67% | NO | — | — | — | — | 0% | 100% | 100% | 3145/1078 | $0.0144 | 1814ms |
| gdpr-basis | regulatory | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 1489/652 | $0.0081 | 951ms |
| hope-feathers | verbatim-verse | 5 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 1432/479 | $0.0065 | 797ms |
| pythagorean | math-concept | 3 | 2 | 0 | 60% | 100% | 100% | 0% | yes | 67% | yes | — | — | — | — | 0% | 100% | 100% | 3127/809 | $0.0120 | 1720ms |
| curie | biography | 6 | 1 | 0 | 86% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 3115/1020 | $0.0139 | 2009ms |
| git-branching | product-doc | 7 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 1479/983 | $0.0111 | 893ms |
| spacing-effect | long-science-prose | 5 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 75% | yes | — | — | — | — | 0% | 100% | 100% | 1674/789 | $0.0096 | 889ms |
| water-boiling | tiny-fact | 2 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 1405/322 | $0.0050 | 979ms |
| apostles-creed | verbatim-sequential | 5 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | yes | yes | 0/6 (0%) | NO | yes | 0% | 100% | 100% | 1440/680 | $0.0083 | 861ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 5.0 | 3.8 | 75% | $0.0067 |
| nato-alphabet | 5.0 | 5.0 | 3.1 | 100% | $0.0275 |
| http-caching | 5.0 | 5.0 | 4.3 | 100% | $0.0090 |
| rubicon | 5.0 | 4.8 | 3.8 | 80% | $0.0074 |
| sourdough | 5.0 | 4.8 | 4.0 | 83% | $0.0091 |
| gdpr-basis | 5.0 | 5.0 | 3.8 | 100% | $0.0069 |
| hope-feathers | 4.6 | 4.6 | 3.2 | 40% | $0.0082 |
| pythagorean | 5.0 | 4.7 | 3.0 | 67% | $0.0059 |
| curie | 5.0 | 5.0 | 3.8 | 83% | $0.0084 |
| git-branching | 5.0 | 4.9 | 3.9 | 71% | $0.0093 |
| spacing-effect | 5.0 | 4.8 | 3.8 | 80% | $0.0085 |
| water-boiling | 5.0 | 5.0 | 5.0 | 100% | $0.0042 |
| apostles-creed | 5.0 | 5.0 | 3.8 | 60% | $0.0074 |

- Judge means: faithfulness 5.0 · question quality 4.9 · distractors 3.8 · keep rate 80% · judge cost $0.1185
- Keep rate (source-clustered, n=13): 80% (95% CI ±11pp)
- Power: ~13 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- mitochondria: draft 3: Distractors are weak — platelets and plasma cells are unlikely confusions, and white blood cells are obviously nucleated cells learners know have mitochondria.
- rubicon: draft 2: Question wording 'direct province' is awkward and slightly unclear.
- sourdough: draft 5: The float test is a common real-world sourdough test but the other two distractors ('acidity test', 'stretch test') feel invented and less grounded in learner confusion.
- hope-feathers: draft 2: Distractors are invented poetic phrases with no grounding in the poem or common confusions, making them obviously wrong to any careful reader.
- hope-feathers: draft 4: Distractors (Spring, Dark, Woods) are lazy generic nature words not drawn from the poem's actual contrasting imagery.
- hope-feathers: draft 5: The question misreads the poem: 'sore must be the storm' means the storm itself would need to be severe, not that it is sore because it is attempting to abash the bird, and the answer conflates what is 'sore' with what does the abashing.
- pythagorean: draft 3: 'A plane' and 'a flat surface' are essentially the same distractor, and both are obviously wrong given the question's framing.
- curie: draft 6: Only two distractors provided, making the multiple-choice set incomplete and the item feel unfinished.
- git-branching: draft 2: Distractors are arbitrary byte values with no grounding in learner misconceptions.
- git-branching: draft 6: Distractors are invented jargon not grounded in real learner misconceptions about rebase history.
- spacing-effect: draft 4: Distractors like 'retrieval effort bias' are invented jargon that no real learner would confuse with the correct answer.
- apostles-creed: draft 2: 'Saint Elizabeth' and 'Mary Magdalene' are outside-knowledge distractors with no grounding in the source text.
- apostles-creed: draft 3: All three distractors are drawn from outside the source, making them feel arbitrary rather than source-grounded confusions.

## Totals

- Provider failures: 0/13 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 93% · count-in-range: 11/13
- Intent shape matches: 9/13 sources
- Content fit matches: 0/3 sources · mean required-unit coverage 67%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.1490 · mean per source: $0.0115
- Latency p50: 1161ms · p95: 2009ms
