# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1256/2031 | $0.0202 | 2458ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1224/1445 | $0.0148 | 2543ms |
| http-caching | technical-doc | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1285/2137 | $0.0212 | 2398ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1264/1504 | $0.0154 | 2393ms |
| sourdough | how-to | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1287/3896 | $0.0370 | 2512ms |
| gdpr-basis | regulatory | 2 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1274/2058 | $0.0204 | 2863ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | NO | 0% | 1217/1498 | $0.0153 | 2765ms |
| pythagorean | math-concept | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1274/1671 | $0.0169 | 2312ms |
| curie | biography | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1275/2260 | $0.0223 | 2433ms |
| git-branching | product-doc | 3 | 100% | 100% | 67% | 0% | yes | 50% | yes | 0% | 1264/2130 | $0.0211 | 2489ms |
| spacing-effect | long-science-prose | 3 | 100% | 100% | 67% | 0% | yes | 50% | yes | 0% | 1459/2574 | $0.0254 | 2936ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1190/2344 | $0.0229 | 2908ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.8 | 4.2 | 100% | $0.0062 |
| nato-alphabet | 5.0 | 4.3 | 2.7 | 33% | $0.0054 |
| http-caching | 5.0 | 4.7 | 3.0 | 100% | $0.0056 |
| rubicon | 5.0 | 5.0 | 3.0 | 33% | $0.0061 |
| sourdough | 5.0 | 4.0 | 3.5 | 75% | $0.0065 |
| gdpr-basis | 5.0 | 5.0 | 3.0 | 100% | $0.0043 |
| hope-feathers | 5.0 | 4.0 | 3.0 | 100% | $0.0053 |
| pythagorean | 5.0 | 4.7 | 3.7 | 67% | $0.0060 |
| curie | 5.0 | 5.0 | 4.0 | 67% | $0.0058 |
| git-branching | 4.3 | 5.0 | 3.0 | 67% | $0.0054 |
| spacing-effect | 5.0 | 5.0 | 3.0 | 100% | $0.0059 |
| water-boiling | 5.0 | 4.7 | 4.0 | 67% | $0.0050 |

- Judge means: faithfulness 4.9 · question quality 4.7 · distractors 3.3 · keep rate 76% · judge cost $0.0676

Judge would not keep:

- nato-alphabet: draft 2: Distractors are too obviously wrong — no real learner would confuse Foxtrot with Fox, Falcon, or Florida.
- nato-alphabet: draft 3: Distractors are weak — Henry, Home, and Hawk are not credible confusions for Hotel in the NATO alphabet context.
- rubicon: draft 2: Distractors reference other famous dates (Caesar's assassination, Actium, Philippi) that a knowledgeable learner might recognize as wrong too easily, making them lazy rather than genuinely confusing.
- rubicon: draft 3: Distractors are sequential numbers (X, XI, XII vs. XIII) that feel mechanical and arbitrary rather than plausible alternatives a learner would genuinely confuse.
- sourdough: draft 1: Compound question tests two facts at once, making it a weak atom for spaced repetition.
- pythagorean: draft 2: Distractors are low-plausibility invented terms that a learner would easily eliminate.
- curie: draft 3: Distractors are weak inventions that no real learner would confuse with 'petites Curies'.
- git-branching: draft 2: The answer has the merge direction inverted: a fast-forward occurs when the target branch is a direct ancestor of the branch being merged in, not a descendant.
- water-boiling: draft 3: Only two distractors and 'it remains the same' is a weak, lazy choice.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 94% · mean key-term coverage: 73% · count-in-range: 12/12
- Intent shape matches: 9/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.2528 · mean per source: $0.0211
- Latency p50: 2489ms · p95: 2936ms
