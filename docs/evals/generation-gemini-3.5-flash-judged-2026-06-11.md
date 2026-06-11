# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 5 | 100% | 100% | 100% | 0% | yes | 100% | 642/2118 | $0.0200 | 3334ms |
| nato-alphabet | enumerable-list | 6 | 100% | 100% | 100% | 0% | yes | 100% | 610/3218 | $0.0299 | 2874ms |
| http-caching | technical-doc | 4 | 100% | 100% | 75% | 0% | yes | 100% | 671/2097 | $0.0199 | 2637ms |
| rubicon | narrative-history | 7 | 100% | 100% | 100% | 0% | yes | 100% | 650/2667 | $0.0250 | 3038ms |
| sourdough | how-to | 6 | 100% | 100% | 100% | 0% | yes | 67% | 673/2272 | $0.0215 | 2717ms |
| gdpr-basis | regulatory | 4 | 100% | 100% | 100% | 0% | yes | 100% | 660/1799 | $0.0172 | 2550ms |
| hope-feathers | verbatim-verse | 6 | 100% | 100% | 100% | 0% | yes | 100% | 603/2690 | $0.0251 | 2941ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 67% | 660/1849 | $0.0176 | 3675ms |
| curie | biography | 6 | 100% | 100% | 100% | 0% | yes | 100% | 661/1514 | $0.0146 | 2834ms |
| git-branching | product-doc | 6 | 100% | 100% | 100% | 0% | yes | 100% | 650/2217 | $0.0209 | 2914ms |
| spacing-effect | long-science-prose | 7 | 100% | 100% | 100% | 0% | yes | 100% | 845/2509 | $0.0238 | 3103ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | 576/1520 | $0.0145 | 2596ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.8 | 4.2 | 80% | $0.0079 |
| nato-alphabet | 5.0 | 5.0 | 3.5 | 50% | $0.0086 |
| http-caching | 4.8 | 4.5 | 3.8 | 50% | $0.0081 |
| rubicon | 5.0 | 5.0 | 3.9 | 71% | $0.0101 |
| sourdough | 5.0 | 4.8 | 3.7 | 67% | $0.0092 |
| gdpr-basis | 5.0 | 4.5 | 4.5 | 100% | $0.0068 |
| hope-feathers | 4.7 | 4.7 | 3.5 | 50% | $0.0088 |
| pythagorean | 4.8 | 4.8 | 3.5 | 75% | $0.0070 |
| curie | 4.8 | 4.7 | 3.5 | 50% | $0.0090 |
| git-branching | 5.0 | 5.0 | 3.5 | 67% | $0.0084 |
| spacing-effect | 5.0 | 5.0 | 4.0 | 86% | $0.0108 |
| water-boiling | 5.0 | 5.0 | 4.3 | 100% | $0.0053 |

- Judge means: faithfulness 4.9 · question quality 4.8 · distractors 3.8 · keep rate 70% · judge cost $0.0998

Judge would not keep:

- mitochondria: draft 4: The distractor 'More than a thousand' is lifted verbatim from the source as a liver-cell figure, making it recognizable rather than genuinely distracting.
- nato-alphabet: draft 4: Distractors like 'David' and 'Daniel' are lazy name-guesses rather than plausible confusions a learner would make.
- nato-alphabet: draft 5: Distractors are generic and weak; 'Fox' is decent but 'Frank' and 'Falcon' are not genuine learner confusions.
- nato-alphabet: draft 6: Distractors are weak; 'Henry' and 'How' are not plausible confusions for a learner who has studied the alphabet.
- http-caching: draft 3: The If-Modified-Since distractor is a real valid mechanism for the same purpose, making the answer ambiguously correct only because the source specifies ETag/If-None-Match.
- http-caching: draft 4: The answer adds 'intermediate and client caches' and 'unique cache key' phrasing not in the source, and distractors are implausible enough that a learner could eliminate them without knowing the answer.
- rubicon: draft 2: Distractors are numerically adjacent legions with no historical basis, making them feel arbitrary rather than genuinely confusing.
- rubicon: draft 6: 'They dispatched assassin guilds to Gaul' is anachronistic and implausible, breaking the realism of the distractor set.
- sourdough: draft 3: Distractor '8 to 12 hours' overlaps with the correct answer boundary (8 hours), making it potentially confusing and unfair.
- sourdough: draft 6: The distractors mix multiple plausible-sounding steps in confusing ways, and one distractor ('feed it twice') introduces information not in the source, reducing quality.
- hope-feathers: draft 2: Distractors are generic and not grounded in plausible misreadings of the poem.
- hope-feathers: draft 4: Distractors lack grounding in the poem's language and rely on generic nature imagery rather than plausible misreadings.
- hope-feathers: draft 5: The answer oversimplifies the poem's conditional phrasing—the poem says such a storm would have to be very sore, not that a 'sore storm' would succeed in abashing it.
- pythagorean: draft 4: The answer gives a symptom rather than the mechanism (curved geometry invalidates the flat-space relation), and distractors are weak or obviously wrong.
- curie: draft 3: Curium and berkelium are named after Marie Curie and Berkeley respectively, making that distractor a near-giveaway to informed learners.
- curie: draft 4: The question adds 'atomic radiation' which slightly editorializes beyond the source, and 'X-radiation' is not a real coined term, weakening distractors.
- curie: draft 6: The third distractor about chemical weapons is implausible enough to be easily eliminated, reducing distractor quality.
- git-branching: draft 2: Distractors (origin, master, index) are too obviously wrong for anyone with basic Git knowledge.
- git-branching: draft 6: Third distractor contains a grammatical error ('a permanent locks') making it obviously flawed.
- spacing-effect: draft 6: First distractor ('shows cards too frequently') is a plausible algorithmic consequence but the other two distractors are weak and not grounded in learner confusions.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 98% · mean key-term coverage: 94% · count-in-range: 12/12
- Total cost: $0.2501 · mean per source: $0.0208
- Latency p50: 2874ms · p95: 3675ms
