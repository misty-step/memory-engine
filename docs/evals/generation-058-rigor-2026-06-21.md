# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 5 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1471/2234 | $0.0223 | 1502ms |
| nato-alphabet | enumerable-list | 9 | 1 | 0 | 90% | 100% | 100% | 0% | NO | 100% | yes | 0% | 89% | 100% | 3021/4415 | $0.0443 | 2866ms |
| http-caching | technical-doc | 6 | 2 | 0 | 75% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3135/3950 | $0.0403 | 2868ms |
| rubicon | narrative-history | 7 | 2 | 0 | 78% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3140/4873 | $0.0486 | 3029ms |
| sourdough | how-to | 6 | 1 | 0 | 86% | 100% | 100% | 0% | yes | 33% | NO | 100% | 100% | 100% | 3138/3905 | $0.0399 | 3542ms |
| gdpr-basis | regulatory | 5 | 2 | 0 | 71% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3164/5258 | $0.0521 | 2890ms |
| hope-feathers | verbatim-verse | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1432/2174 | $0.0217 | 1423ms |
| pythagorean | math-concept | 4 | 3 | 0 | 57% | 100% | 100% | 0% | yes | 67% | yes | 0% | 100% | 100% | 3249/5478 | $0.0542 | 2725ms |
| curie | biography | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 1490/3134 | $0.0304 | 1815ms |
| git-branching | product-doc | 6 | 0 | 0 | 100% | 100% | 83% | 0% | yes | 100% | yes | 0% | 100% | 100% | 1479/3088 | $0.0300 | 1520ms |
| spacing-effect | long-science-prose | 5 | 2 | 0 | 71% | 100% | 100% | 0% | yes | 75% | yes | 0% | 100% | 100% | 3495/4452 | $0.0453 | 3155ms |
| water-boiling | tiny-fact | 3 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1405/1702 | $0.0174 | 1586ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 5.0 | 4.4 | 100% | $0.0073 |
| nato-alphabet | 5.0 | 4.9 | 3.3 | 89% | $0.0116 |
| http-caching | 5.0 | 5.0 | 4.5 | 100% | $0.0084 |
| rubicon | 4.9 | 4.7 | 3.9 | 71% | $0.0096 |
| sourdough | 5.0 | 4.8 | 3.7 | 50% | $0.0090 |
| gdpr-basis | 4.8 | 4.4 | 4.4 | 80% | $0.0076 |
| hope-feathers | 4.8 | 4.0 | 3.2 | 17% | $0.0098 |
| pythagorean | 4.8 | 4.8 | 3.2 | 25% | $0.0075 |
| curie | 5.0 | 5.0 | 3.8 | 67% | $0.0078 |
| git-branching | 5.0 | 4.8 | 4.0 | 83% | $0.0088 |
| spacing-effect | 4.8 | 5.0 | 4.4 | 80% | $0.0082 |
| water-boiling | 5.0 | 4.7 | 4.3 | 67% | $0.0050 |

- Judge means: faithfulness 4.9 · question quality 4.8 · distractors 3.9 · keep rate 69% · judge cost $0.1004
- Keep rate (source-clustered, n=12): 69% (95% CI ±17pp)
- Paired vs baseline (12 sources): keep Δ -1.4pp (95% CI ±19.5pp) — **within noise** (indistinguishable from a no-op at this n)
- Power: ~12 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- nato-alphabet: draft 9: The distractors are implausible fabrications that no learner would genuinely consider as real reasons.
- rubicon: draft 5: Distractors are common English idioms unrelated to Roman history, making them too easy to eliminate by topic.
- rubicon: draft 7: Crassus was dead by 49 BC and Sulla died decades earlier, making those distractors anachronistically implausible to any informed learner.
- sourdough: draft 3: 'Contaminated with yeast mold' is not a realistic learner confusion and weakens the distractor set.
- sourdough: draft 5: 'Float test' and 'windowpane test' are bread-baking terms but are not confusions specifically tied to sourdough starter readiness, making them feel borrowed rather than organic.
- sourdough: draft 6: This item is a near-duplicate of Draft 1 and should be removed to avoid redundancy.
- gdpr-basis: draft 4: This is a near-duplicate of draft 2 and should be removed to avoid redundancy.
- hope-feathers: draft 1: Distractors (wings, talons, claws) are all bird-body-part synonyms that feel too obviously wrong and interchangeable rather than genuine confusions.
- hope-feathers: draft 3: Third distractor has a grammatical error ('without a end') which disqualifies it from publication.
- hope-feathers: draft 4: Distractors ('stops when night falls', 'stops during the rain', 'stops when winters come') are invented conditions not from the poem and feel arbitrary rather than tempting confusions.
- hope-feathers: draft 5: Distractors (Breeze, Tempest, Blizzard) are plausible weather words but 'Tempest' is nearly synonymous with Gale, making the set inconsistently calibrated.
- hope-feathers: draft 6: The answer 'the storm' slightly undersells faithfulness since the poem says the storm must be 'sore', but more critically the question awkwardly describes 'storm' as a 'meteorological event' when the poem uses it more figuratively.
- pythagorean: draft 1: The correct answer only partially explains the failure; the source also states the relation no longer applies, and the distractor about whole-number sides is a non-sequitur confusing learners.
- pythagorean: draft 2: Distractors 'adjacent side' and 'opposite side' are informal/ambiguous terms that could confuse without being genuine learner misconceptions, and 'altitude' is not a side at all.
- pythagorean: draft 4: Short-answer format is acceptable here, but scoring 3 because multiple-choice distractors (e.g., Pythagorean pairs, Pythagorean primes) would reinforce the specific term more effectively.
- curie: draft 2: Only two distractors and one is the answer to a sister question, making it too easy by elimination.
- curie: draft 3: Only two distractors and one is the answer to a sister question, making elimination trivial.
- git-branching: draft 5: 'A linear rebase' and 'a cherry-pick merge' are not real merge types, making them weak distractors a learner could easily eliminate.
- spacing-effect: draft 2: Source says 'nonsense syllables' not 'vocabulary-like nonsense syllables,' introducing a minor but unnecessary qualifier not in the source.
- water-boiling: draft 3: Only two distractors and one is almost trivially wrong ('exactly the same'), weakening distractor quality.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 99% · mean key-term coverage: 90% · count-in-range: 11/12
- Intent shape matches: 8/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.4464 · mean per source: $0.0372
- Latency p50: 2725ms · p95: 3542ms
