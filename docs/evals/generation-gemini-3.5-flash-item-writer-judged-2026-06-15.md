# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1673/1748 | $0.0182 | 3227ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1640/1282 | $0.0140 | 3080ms |
| http-caching | technical-doc | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1701/2273 | $0.0230 | 2850ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1681/1836 | $0.0190 | 2678ms |
| sourdough | how-to | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1703/2597 | $0.0259 | 2589ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1691/2645 | $0.0263 | 2298ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1632/1228 | $0.0135 | 2616ms |
| pythagorean | math-concept | 3 | 100% | 100% | 100% | 0% | yes | 33% | yes | 0% | 1690/2142 | $0.0218 | 2274ms |
| curie | biography | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1691/1459 | $0.0157 | 2758ms |
| git-branching | product-doc | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1680/2706 | $0.0269 | 3013ms |
| spacing-effect | long-science-prose | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1875/2548 | $0.0257 | 2772ms |
| water-boiling | tiny-fact | 2 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1607/1360 | $0.0147 | 3933ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.3 | 4.0 | 67% | $0.0057 |
| nato-alphabet | 5.0 | 5.0 | 3.0 | 33% | $0.0054 |
| http-caching | 5.0 | 4.3 | 4.0 | 67% | $0.0065 |
| rubicon | 5.0 | 5.0 | 4.0 | 67% | $0.0060 |
| sourdough | 5.0 | 5.0 | 4.3 | 100% | $0.0059 |
| gdpr-basis | 5.0 | 5.0 | 3.7 | 67% | $0.0059 |
| hope-feathers | 5.0 | 4.0 | 3.0 | 100% | $0.0055 |
| pythagorean | 5.0 | 4.3 | 3.7 | 67% | $0.0056 |
| curie | 5.0 | 4.7 | 3.0 | 67% | $0.0063 |
| git-branching | 5.0 | 4.0 | 3.7 | 67% | $0.0056 |
| spacing-effect | 5.0 | 5.0 | 3.3 | 33% | $0.0069 |
| water-boiling | 5.0 | 5.0 | 3.5 | 100% | $0.0042 |

- Judge means: faithfulness 5.0 · question quality 4.6 · distractors 3.6 · keep rate 69% · judge cost $0.0695

Judge would not keep:

- mitochondria: draft 1: The compound question tests two atoms at once, making it better split into separate cards.
- nato-alphabet: draft 1: Distractors are implausible fabrications no learner would genuinely consider.
- nato-alphabet: draft 2: Distractors like Henry and Hector are too obviously non-standard and feel arbitrary rather than tempting confusions.
- http-caching: draft 3: Short-answer format is reasonable here, but the question is compound and somewhat verbose, reducing its focus as a discrete learning atom.
- rubicon: draft 3: Distractors like 'et tu, Brute' and 'memento mori' are not Latin phrases Caesar would plausibly have said at the Rubicon, making them too easy to eliminate.
- gdpr-basis: draft 3: Short-answer format is acceptable but the absence of distractors limits its utility for spaced-repetition multiple-choice practice.
- pythagorean: draft 3: Short-answer format is appropriate but the question asks for explanation rather than testing a discrete memorable fact.
- curie: draft 2: Distractors are weak: 'Mobiles de Curie' and 'Radios de guerre' feel invented rather than genuinely confusing alternatives a learner would consider.
- git-branching: draft 1: Short-answer format is acceptable but the question is compound and verbose, testing two things at once.
- spacing-effect: draft 1: Short-answer format is acceptable here, but the lack of distractors misses an opportunity to test against plausible confusions like 'easy recall' or 'massed practice'.
- spacing-effect: draft 2: Two of the three distractors are absurd non-starters ('physically demanding', 'cognitive strain halts consolidation') rather than genuine learner confusions.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 76% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.2448 · mean per source: $0.0204
- Latency p50: 2758ms · p95: 3933ms
