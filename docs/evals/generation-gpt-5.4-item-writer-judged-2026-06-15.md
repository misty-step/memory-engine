# Generation eval receipt

- Provider: openrouter/openai/gpt-5.4 (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1360/503 | $0.0000 | 692ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1315/251 | $0.0000 | 301ms |
| http-caching | technical-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1368/433 | $0.0000 | 294ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1361/323 | $0.0000 | 293ms |
| sourdough | how-to | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1380/445 | $0.0000 | 396ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1360/357 | $0.0000 | 306ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1299/275 | $0.0000 | 2345ms |
| pythagorean | math-concept | 5 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1367/649 | $0.0000 | 296ms |
| curie | biography | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1363/261 | $0.0000 | 419ms |
| git-branching | product-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1349/472 | $0.0000 | 455ms |
| spacing-effect | long-science-prose | 4 | 100% | 100% | 100% | 0% | yes | 25% | yes | 0% | 1541/469 | $0.0000 | 384ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1275/288 | $0.0000 | 353ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 4.8 | 4.0 | 3.8 | 25% | $0.0065 |
| nato-alphabet | 5.0 | 4.7 | 3.3 | 67% | $0.0056 |
| http-caching | 5.0 | 4.2 | 3.0 | 100% | $0.0069 |
| rubicon | 4.3 | 5.0 | 3.7 | 33% | $0.0056 |
| sourdough | 5.0 | 5.0 | 4.0 | 100% | $0.0060 |
| gdpr-basis | 4.7 | 4.3 | 3.7 | 67% | $0.0056 |
| hope-feathers | 5.0 | 3.3 | 3.0 | 67% | $0.0052 |
| pythagorean | 5.0 | 4.6 | 3.4 | 80% | $0.0079 |
| curie | 5.0 | 5.0 | 3.3 | 67% | $0.0055 |
| git-branching | 4.8 | 4.2 | 4.0 | 50% | $0.0064 |
| spacing-effect | 5.0 | 4.8 | 3.8 | 50% | $0.0076 |
| water-boiling | 5.0 | 3.7 | 3.7 | 67% | $0.0053 |

- Judge means: faithfulness 4.9 · question quality 4.4 · distractors 3.5 · keep rate 64% · judge cost $0.0740

Judge would not keep:

- mitochondria: draft 1: Short-answer format is acceptable but the question is compound, testing two separate pieces of evidence at once.
- mitochondria: draft 2: Answer is a verb phrase fragment rather than a clean noun answer, which weakens usability.
- mitochondria: draft 4: Distractors (mitosis, osmosis, glycolysis) are too loosely related to apoptosis to challenge a real learner.
- nato-alphabet: draft 2: Distractors share the 'Fox-' prefix or F-sound but feel lazy and unlikely to be genuine learner confusions.
- rubicon: draft 2: Distractors are adjacent numerals with no conceptual grounding, making them feel arbitrary rather than meaningful confusions.
- rubicon: draft 3: The answer echoes the awkward phrasing 'civil war inevitable' rather than a clean answer like 'civil war'.
- gdpr-basis: draft 3: Question is compound and slightly imprecise—the balancing test limitation conflates two separate constraints from the source.
- hope-feathers: draft 3: Asking to 'recite exactly' is a rote transcription task with low pedagogical value and unclear standalone context.
- pythagorean: draft 3: First distractor ('any right triangle has three equal sides') is too obviously wrong to challenge a real learner.
- curie: draft 3: Distractor 'radiation' is too close semantically and 'nuclear fission' is anachronistic but obvious; distractors need refinement.
- git-branching: draft 1: Short-answer format is acceptable but the question is compound, testing two concepts at once.
- git-branching: draft 4: The answer redundantly restates the question stem rather than giving a concise, standalone explanation.
- spacing-effect: draft 1: Short-answer format is acceptable but no distractors means no discrimination between common confusions like 'more practice time' vs. the delay-and-forgetting mechanism.
- spacing-effect: draft 3: Two of the three distractors are implausible fabrications not grounded in the source, reducing their value as genuine learner confusions.
- water-boiling: draft 2: Question phrasing is awkward and grammatically broken ('boils at a what kind of temperature').

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 81% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0000 · mean per source: $0.0000
- Latency p50: 353ms · p95: 2345ms
