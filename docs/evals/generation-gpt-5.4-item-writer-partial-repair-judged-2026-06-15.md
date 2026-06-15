# Generation eval receipt

- Provider: openrouter/openai/gpt-5.4 (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1360/443 | $0.0000 | 708ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1315/255 | $0.0000 | 290ms |
| http-caching | technical-doc | 4 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1368/342 | $0.0000 | 711ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 2843/402 | $0.0000 | 2046ms |
| sourdough | how-to | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1380/535 | $0.0000 | 767ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1360/377 | $0.0000 | 292ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1299/288 | $0.0000 | 1219ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 2865/653 | $0.0000 | 1070ms |
| curie | biography | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1363/360 | $0.0000 | 283ms |
| git-branching | product-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1349/405 | $0.0000 | 351ms |
| spacing-effect | long-science-prose | 4 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 3211/548 | $0.0000 | 887ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1275/323 | $0.0000 | 284ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.5 | 3.5 | 75% | $0.0061 |
| nato-alphabet | 5.0 | 4.7 | 3.7 | 67% | $0.0054 |
| http-caching | 5.0 | 4.2 | 3.0 | 100% | $0.0061 |
| rubicon | 5.0 | 4.7 | 3.7 | 67% | $0.0052 |
| sourdough | 5.0 | 4.8 | 3.8 | 75% | $0.0069 |
| gdpr-basis | 4.7 | 4.0 | 3.3 | 67% | $0.0057 |
| hope-feathers | 5.0 | 2.0 | 3.0 | 0% | $0.0052 |
| pythagorean | 4.8 | 5.0 | 3.8 | 50% | $0.0069 |
| curie | 5.0 | 4.8 | 4.2 | 75% | $0.0066 |
| git-branching | 4.5 | 4.0 | 3.0 | 75% | $0.0061 |
| spacing-effect | 4.8 | 4.8 | 3.0 | 75% | $0.0070 |
| water-boiling | 5.0 | 5.0 | 3.0 | 33% | $0.0053 |

- Judge means: faithfulness 4.9 · question quality 4.4 · distractors 3.4 · keep rate 63% · judge cost $0.0723

Judge would not keep:

- mitochondria: draft 1: Compound question reduces focus; splitting into two atomic items would be stronger.
- nato-alphabet: draft 1: Distractor 'Alpha' is the very common misspelling the source explicitly warns against, making it too obvious as a wrong answer.
- rubicon: draft 2: Distractors are arbitrary legion numbers with no particular salience to a learner.
- sourdough: draft 4: Distractors are lazy round numbers rather than genuine learner confusions.
- gdpr-basis: draft 3: The answer conflates two conditions (balancing test and non-override) into one run-on, making it imprecise and slightly harder to retain as an atomic fact.
- hope-feathers: draft 1: The question relies on the prior line as a cue rather than standing alone as a meaningful prompt about the poem's content.
- hope-feathers: draft 2: The chained-line format requires memorizing sequence rather than testing meaningful content, making the question weak in isolation.
- hope-feathers: draft 3: Same chained-line weakness; question tests rote sequence recall rather than any meaningful poetic idea.
- pythagorean: draft 1: Short-answer format suits the item but the answer adds 'flat Euclidean geometry' which is not explicitly stated in the source.
- pythagorean: draft 3: Distractors are plausible but somewhat lazy—'angles sum to 180 degrees' describes all triangles and is a weak confuser.
- curie: draft 4: Short-answer format is defensible but the absence of distractors leaves the item weaker than a multiple-choice version would be for this memorable single fact.
- git-branching: draft 3: Answer is circular and redundant, restating the question rather than isolating the reason (rewriting commit hashes).
- spacing-effect: draft 3: The answer slightly conflates cause and effect — the source says recognition masquerades as recall causing leniency, not the reverse — making the causal chain mildly imprecise.
- water-boiling: draft 2: Distractors of 80 and 110 are implausibly far from the correct answer, reducing their effectiveness.
- water-boiling: draft 3: The distractor '0 degrees Celsius' is obviously wrong and no real learner would choose it.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 85% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0000 · mean per source: $0.0000
- Latency p50: 708ms · p95: 2046ms
