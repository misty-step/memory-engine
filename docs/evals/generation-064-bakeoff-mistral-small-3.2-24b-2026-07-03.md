# Generation eval receipt

- Provider: openrouter/mistralai/mistral-small-3.2-24b-instruct (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 13 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 8 | 1 | 0 | 89% | 100% | 100% | 0% | yes | 100% | NO | yes | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 2052/1030 | $0.0005 | 3350ms |
| nato-alphabet | enumerable-list | 6 | 25 | 0 | 19% | 100% | 83% | 0% | NO | 33% | yes | NO | 5/26 (19%) | NO | yes | 0% | 100% | 100% | 2284/3191 | $0.0011 | 565ms |
| http-caching | technical-doc | 7 | 2 | 2 | 64% | 100% | 100% | 0% | yes | 75% | yes | — | — | — | — | 0% | 100% | 100% | 2157/1319 | $0.0005 | 1258ms |
| rubicon | narrative-history | 10 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 978/1201 | $0.0004 | 410ms |
| sourdough | how-to | 8 | 2 | 0 | 80% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 2145/1344 | $0.0005 | 1090ms |
| gdpr-basis | regulatory | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 67% | yes | — | — | — | — | 0% | 100% | 100% | 983/734 | $0.0003 | 315ms |
| hope-feathers | verbatim-verse | 7 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | NO | — | — | — | — | 0% | 100% | 100% | 919/799 | $0.0003 | 400ms |
| pythagorean | math-concept | 6 | 0 | 0 | 100% | 100% | 83% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 989/849 | $0.0003 | 309ms |
| curie | biography | 9 | 2 | 0 | 82% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 2095/1336 | $0.0005 | 1951ms |
| git-branching | product-doc | 8 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 100% | 100% | 100% | 972/1089 | $0.0004 | 445ms |
| spacing-effect | long-science-prose | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | yes | — | — | — | — | 0% | 100% | 100% | 0/0 | — | 0ms |
| water-boiling | tiny-fact | 3 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 900/400 | $0.0002 | 872ms |
| apostles-creed | verbatim-sequential | 0 | 0 | 1 | 0% | 0% | 0% | 0% | NO | 0% | yes | NO | 0/6 (0%) | NO | yes | 0% | 100% | 100% | 0/0 | — | 0ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 4.9 | 4.9 | 3.9 | 88% | $0.0101 |
| nato-alphabet | 5.0 | 5.0 | 3.7 | 67% | $0.0086 |
| http-caching | 4.7 | 4.1 | 3.7 | 43% | $0.0096 |
| rubicon | 5.0 | 4.8 | 3.4 | 90% | $0.0123 |
| sourdough | 4.9 | 4.8 | 3.6 | 50% | $0.0108 |
| gdpr-basis | 4.7 | 4.3 | 3.5 | 67% | $0.0085 |
| hope-feathers | 4.6 | 4.4 | 3.1 | 57% | $0.0097 |
| pythagorean | 4.8 | 4.5 | 3.5 | 33% | $0.0086 |
| curie | 4.9 | 4.6 | 3.6 | 56% | $0.0110 |
| git-branching | 3.9 | 4.2 | 3.4 | 50% | $0.0101 |
| water-boiling | 5.0 | 4.7 | 4.0 | 67% | $0.0050 |

- Judge means: faithfulness 4.8 · question quality 4.6 · distractors 3.6 · keep rate 61% · judge cost $0.1042
- Keep rate (source-clustered, n=11): 61% (95% CI ±12pp)
- Paired vs baseline (11 sources): keep Δ -21.1pp (95% CI ±16.0pp) — **detectable** — CI excludes 0
- Power: ~11 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- mitochondria: draft 6: 'A few thousand' overlaps with the correct answer range, making the item ambiguous.
- nato-alphabet: draft 2: Distractors like 'Caesar' and 'Casino' are not plausible confusions a learner studying NATO alphabet would make.
- nato-alphabet: draft 3: All three distractors are US city names with no phonetic-alphabet plausibility, making them too easy to eliminate.
- http-caching: draft 4: Source also mentions ETag as the mechanism but doesn't explicitly exclude If-Modified-Since, making the answer slightly incomplete given common HTTP knowledge.
- http-caching: draft 5: Distractors like ETag and Last-Modified are response headers, not Cache-Control directives, making them categorically inconsistent with the question's implied domain.
- http-caching: draft 6: This item nearly duplicates draft 3 and the 'public' distractor is not a real learner confusion in this context.
- http-caching: draft 7: The source mentions ETag but doesn't explicitly define it as an 'entity tag,' and the question conflates the tag itself with the header, reducing precision.
- rubicon: draft 5: Distractors (patriotism, heroism, loyalty) are obviously absurd antonyms rather than plausible legal or political confusions.
- sourdough: draft 1: Distractors 'bacteria and mold' and 'yeast and bacteria' are too close to the correct answer and not confusing enough to be useful.
- sourdough: draft 3: 'Every 12 hours' and 'twice a day' are the same interval, making two distractors redundant.
- sourdough: draft 4: Answer omits the time frame ('within 4 to 8 hours') which is an important part of the source detail.
- sourdough: draft 8: 'Let it sit for 24 hours' is partially correct and too close to the right answer to be a fair distractor.
- gdpr-basis: draft 1: Distractors are also correct lawful bases from the source, making this question misleading and unanswerable as a quiz item.
- gdpr-basis: draft 6: Distractors are weak — 'fulfilling a marketing campaign' is obviously wrong and 'industry standard' is too easy to dismiss.
- hope-feathers: draft 1: The answer 'feathers' is imprecise — hope IS the thing with feathers, not that it resembles feathers; also distractors 'wings' and 'songs' are too few and obvious.
- hope-feathers: draft 5: Distractors 'Breeze,' 'Calm,' and 'Night' are plausible but lazy weather/time terms rather than carefully crafted confusions.
- hope-feathers: draft 6: Distractors are specific bird names that no attentive reader would confuse with the poem's generic 'little Bird,' making them obviously wrong.
- pythagorean: draft 2: Distractors like 'adjacent side' and 'opposite side' are informal or ambiguous geometry terms that could confuse rather than test the specific concept.
- pythagorean: draft 3: Distractors are plausible but lazy name-drops with no real connection to number triples.
- pythagorean: draft 4: The source says the theorem fails on curved surfaces generally and uses a sphere as an example, but 'cube' is not a curved surface, making one distractor obviously wrong.
- pythagorean: draft 5: Short-answer is acceptable here, but the question is weak because '5, 12, 13' is equally correct and the stem doesn't constrain which example to give.
- curie: draft 1: Two distractors are near-identical name variants that are too obvious as wrong answers.
- curie: draft 5: Distractors are too obscure (berkelium/californium) or too obvious (uranium/thorium as red herrings a learner might actually believe).
- curie: draft 6: 'Radiation' is too close to 'radioactivity' and risks confusing learners about what exactly was coined versus what already existed.
- curie: draft 9: Distractors like 'glass cases' are obviously implausible for radioactive materials, lowering distractor quality.
- git-branching: draft 2: Distractors BRANCH, COMMIT, and MERGE are not real Git pointers/concepts at the same level, making them too obviously wrong.
- git-branching: draft 6: The source never mentions specific Git commands, so the answer is not supported by the source text.
- git-branching: draft 7: The source never mentions specific Git commands, making this item unfaithful to the source material.
- git-branching: draft 8: The source never mentions specific Git commands, so the answer is not supported by the source text.
- water-boiling: draft 2: Distractors like 'inconsistent' are implausible and lazy for a factual physics question.

## Totals

- Provider failures: 0/13 sources
- Mean provenance: 85% · mean answerability: 82% · mean key-term coverage: 75% · count-in-range: 7/13
- Intent shape matches: 9/13 sources
- Content fit matches: 0/3 sources · mean required-unit coverage 40%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0051 · mean per source: $0.0004
- Latency p50: 445ms · p95: 3350ms
