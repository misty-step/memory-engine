# Generation eval receipt

- Provider: openrouter/openai/gpt-5.4 (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1360/465 | $0.0000 | 788ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1315/254 | $0.0000 | 437ms |
| http-caching | technical-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1368/438 | $0.0000 | 494ms |
| rubicon | narrative-history | 2 | 100% | 100% | 100% | 0% | yes | 0% | yes | 0% | 1361/342 | $0.0000 | 488ms |
| sourdough | how-to | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1380/555 | $0.0000 | 2397ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1360/348 | $0.0000 | 457ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1299/279 | $0.0000 | 421ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1367/479 | $0.0000 | 669ms |
| curie | biography | 2 | 100% | 100% | 100% | 0% | yes | 25% | yes | 0% | 1363/274 | $0.0000 | 334ms |
| git-branching | product-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1349/415 | $0.0000 | 763ms |
| spacing-effect | long-science-prose | 3 | 100% | 100% | 100% | 0% | yes | 25% | yes | 0% | 1541/437 | $0.0000 | 249ms |
| water-boiling | tiny-fact | 2 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1275/291 | $0.0000 | 334ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 3.7 | 3.3 | 67% | $0.0056 |
| nato-alphabet | 5.0 | 4.7 | 3.7 | 67% | $0.0055 |
| http-caching | 5.0 | 4.5 | 3.0 | 75% | $0.0063 |
| rubicon | 5.0 | 4.5 | 3.5 | 50% | $0.0049 |
| sourdough | 5.0 | 4.8 | 4.0 | 75% | $0.0072 |
| gdpr-basis | 5.0 | 4.0 | 3.3 | 67% | $0.0057 |
| hope-feathers | 5.0 | 2.0 | 3.0 | 0% | $0.0051 |
| pythagorean | 4.8 | 4.8 | 3.0 | 75% | $0.0065 |
| curie | 5.0 | 5.0 | 3.5 | 50% | $0.0047 |
| git-branching | 4.8 | 4.0 | 3.0 | 75% | $0.0060 |
| spacing-effect | 5.0 | 5.0 | 3.0 | 33% | $0.0066 |
| water-boiling | 5.0 | 4.5 | 3.5 | 100% | $0.0044 |

- Judge means: faithfulness 5.0 · question quality 4.3 · distractors 3.3 · keep rate 61% · judge cost $0.0683

Judge would not keep:

- mitochondria: draft 2: Compound question tests two atoms at once, weakening its value as a focused spaced-repetition item.
- nato-alphabet: draft 1: The distractor 'Alpha' is excellent but 'Bravo' and 'Echo' are too obviously wrong since they are different letters entirely.
- http-caching: draft 4: Answer omits the concrete example (Vary: Accept-Encoding) that makes the concept memorable and testable.
- rubicon: draft 1: Distractors are weak because 'Brundisium' appears verbatim in the text, making the third distractor an obvious near-copy rather than a genuine learner confusion.
- sourdough: draft 2: Distractors are weak—'bake immediately' and 'use straight from fridge' are near-identical confusions rather than distinct plausible errors.
- gdpr-basis: draft 3: The question is compound, testing two distinct atoms simultaneously, which weakens its utility as a spaced-repetition item.
- hope-feathers: draft 1: The question relies on reciting an adjacent line rather than testing conceptual understanding of the poem.
- hope-feathers: draft 2: The question is pure rote sequence recall with no standalone context about the poem's meaning or topic.
- hope-feathers: draft 3: The question tests only line-order memorization and lacks any conceptual or thematic value.
- pythagorean: draft 4: Answer conflates two related but distinct points (curved surfaces generally vs. sphere specifically), making it slightly imprecise relative to the source's structure.
- curie: draft 1: Distractors are lazy; 'Medicine and Chemistry' and 'Chemistry and Peace' are implausible combinations that no serious learner would confuse with the correct answer.
- git-branching: draft 3: The answer echoes the question too closely and is phrased as a clause rather than a clean standalone fact.
- spacing-effect: draft 1: Short-answer format is acceptable but the absence of distractors leaves easy guessing room for a factual causal claim that would benefit from MCQ.
- spacing-effect: draft 3: Distractors are clearly fabricated and not genuine confusions a learner would hold—none are plausible misreadings of the source.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 71% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0000 · mean per source: $0.0000
- Latency p50: 457ms · p95: 2397ms
