# Generation eval receipt

- Provider: openrouter/google/gemini-2.5-flash-lite (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 13 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 9 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | NO | NO | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 966/1224 | $0.0006 | 586ms |
| nato-alphabet | enumerable-list | 28 | 1 | 0 | 97% | 100% | 100% | 0% | NO | 100% | yes | NO | 26/26 (100%) | NO | yes | 0% | 100% | 100% | 2168/6406 | $0.0028 | 1043ms |
| http-caching | technical-doc | 9 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 995/1437 | $0.0007 | 534ms |
| rubicon | narrative-history | 7 | 2 | 0 | 78% | 100% | 100% | 0% | yes | 75% | yes | — | — | — | — | 100% | 100% | 100% | 2133/2670 | $0.0013 | 982ms |
| sourdough | how-to | 10 | 1 | 0 | 91% | 100% | 100% | 0% | NO | 100% | NO | — | — | — | — | 100% | 100% | 100% | 2123/3168 | $0.0015 | 918ms |
| gdpr-basis | regulatory | 9 | 1 | 0 | 90% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 2102/2780 | $0.0013 | 1772ms |
| hope-feathers | verbatim-verse | 8 | 0 | 0 | 100% | 100% | 100% | 0% | NO | 100% | NO | — | — | — | — | 0% | 100% | 100% | 927/1024 | $0.0005 | 665ms |
| pythagorean | math-concept | 7 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | — | — | — | — | 0% | 100% | 100% | 984/1214 | $0.0006 | 453ms |
| curie | biography | 9 | 1 | 0 | 90% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 2096/2157 | $0.0011 | 1051ms |
| git-branching | product-doc | 11 | 0 | 0 | 100% | 100% | 91% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 974/1519 | $0.0007 | 523ms |
| spacing-effect | long-science-prose | 17 | 5 | 0 | 77% | 100% | 100% | 0% | NO | 100% | yes | — | — | — | — | 0% | 100% | 100% | 2639/4263 | $0.0020 | 2128ms |
| water-boiling | tiny-fact | 2 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | — | — | — | — | 0% | 100% | 100% | 900/331 | $0.0002 | 493ms |
| apostles-creed | verbatim-sequential | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | yes | 0/6 (0%) | NO | yes | 0% | 100% | 100% | 935/963 | $0.0005 | 599ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 4.9 | 4.9 | 3.7 | 89% | $0.0113 |
| nato-alphabet | 5.0 | 5.0 | 3.1 | 93% | $0.0274 |
| http-caching | 5.0 | 4.9 | 3.7 | 89% | $0.0120 |
| rubicon | 4.7 | 4.6 | 4.0 | 57% | $0.0096 |
| sourdough | 4.7 | 4.3 | 3.6 | 70% | $0.0133 |
| gdpr-basis | 4.9 | 4.0 | 3.4 | 56% | $0.0117 |
| hope-feathers | 5.0 | 4.0 | 3.0 | 100% | $0.0090 |
| pythagorean | 4.9 | 4.3 | 3.1 | 43% | $0.0101 |
| curie | 5.0 | 4.6 | 3.3 | 44% | $0.0126 |
| git-branching | 4.9 | 4.5 | 3.5 | 55% | $0.0133 |
| spacing-effect | 5.0 | 4.4 | 3.5 | 53% | $0.0200 |
| water-boiling | 5.0 | 5.0 | 4.5 | 100% | $0.0041 |
| apostles-creed | 5.0 | 4.5 | 3.0 | 33% | $0.0091 |

- Judge means: faithfulness 4.9 · question quality 4.5 · distractors 3.5 · keep rate 68% · judge cost $0.1636
- Keep rate (source-clustered, n=13): 68% (95% CI ±14pp)
- Paired vs baseline (13 sources): keep Δ -12.1pp (95% CI ±16.1pp) — **within noise** — CI includes 0 (not detected; not proof of no change)
- Power: ~13 sources resolves only large regressions; a ~3pp change needs ~1000 drafts (Miller 2411.00640). Read this suite as a large-regression guard.

Judge would not keep:

- mitochondria: draft 4: 'Binary fission' is essentially a synonym for dividing in two, making it a near-correct distractor that undermines the item.
- nato-alphabet: draft 27: Third distractor 'To provide a standardized international code' is partially true and too close to the correct answer, undermining discrimination.
- nato-alphabet: draft 28: Duplicate of draft 1; 'Apple' and 'Able' are weaker distractors than the first version's 'Atlanta' and 'Amber', making this an inferior duplicate.
- http-caching: draft 9: The first distractor ('Thinking it forces revalidation for every request') is actually correct behavior of no-cache, making it a factually problematic distractor.
- rubicon: draft 3: The answer conflates two separate facts from the source (treason + civil war inevitable) making it a compound answer rather than a single atom.
- rubicon: draft 6: Near-duplicate of draft 1 (same answer, nearly identical distractors); adds no new value and should be deduplicated.
- rubicon: draft 7: Near-duplicate of draft 2 with only minor distractor variation; redundant and should be deduplicated.
- sourdough: draft 2: 'Twice a day' is semantically equivalent to 'once every 12 hours', making two distractors redundant.
- sourdough: draft 5: 'Feed it more frequently' is essentially the correct answer rephrased, making the distractors ambiguous and the item unfair.
- sourdough: draft 10: Distractors 2 and 3 name non-organisms (dry yeast, flour, water) rather than plausible microbial confusions, and the item duplicates question 1.
- gdpr-basis: draft 1: Question is vague ('related to the data subject's agreement') and distractors are too few (only three listed but item references six bases).
- gdpr-basis: draft 4: 'Essential well-being' is a loose paraphrase of 'vital interests', introducing slight imprecision.
- gdpr-basis: draft 7: Distractors ('obtaining it', 'proving it', 'requesting it') are too similar and not meaningful confusions a learner would have.
- gdpr-basis: draft 9: Only two distractors provided and asking for 'first listed' tests trivial ordering rather than conceptual understanding.
- pythagorean: draft 3: First distractor is actually the forward theorem, not a wrong answer, making it a confusing near-correct option.
- pythagorean: draft 4: Distractors are obscure invented terms that no real learner would confuse with 'Pythagorean triples'.
- pythagorean: draft 5: Distractors include valid Pythagorean triples (6,8,10) and near-misses that require calculation to evaluate, making this a poor recognition item.
- pythagorean: draft 6: Distractors are nearly synonymous with the correct answer, making discrimination trivial.
- curie: draft 1: Distractors are plausible but the second one ('Only woman to win two Nobel Prizes') is actually a true statement about Curie, making it a flawed distractor.
- curie: draft 4: Distractors 'Polonium' and 'Radium' are not terms she coined and are obviously wrong as answers to 'what term did she coin', making them too easy to eliminate.
- curie: draft 6: 'Les Petits Curies' is a trivial language-variant distractor and 'Marie's Machines' and 'Radiant Vans' are implausibly informal inventions no learner would seriously consider.
- curie: draft 7: Question is ambiguous—it asks what 'remains highly contaminated' rather than what contaminates the notebooks, which is an odd framing; distractors are plausible radioactive elements but feel generic.
- curie: draft 9: This is a near-duplicate of draft 2 with only minor wording differences, making it redundant.
- git-branching: draft 2: 'Effectively free' is imprecise as the answer; the source ties it to the 41-byte file cost, and distractors are weak invented negatives.
- git-branching: draft 4: Distractors are lazy and obvious; 'To rebase commits' is too transparently wrong.
- git-branching: draft 8: Distractors are generic and weak; 'It merges histories' is too broad and 'It moves the HEAD pointer' is not clearly wrong.
- git-branching: draft 9: 'A rewritten history' is technically true of rebasing per the source, making it a faulty distractor.
- git-branching: draft 10: Distractors are implausible invented confusions rather than real learner misconceptions.
- spacing-effect: draft 4: Distractors like 'highlighting' and 'summarizing' are not real confusions a learner of this text would make.
- spacing-effect: draft 6: 'Cognitive load' and 'memory decay' are external terms not set up as confusions by this source text.
- spacing-effect: draft 7: Distractors are too obviously wrong and don't reflect real learner confusions about SRS design.
- spacing-effect: draft 11: 'Duration of study session' and 'ease of recalling' are weak distractors not grounded in source confusions.
- spacing-effect: draft 12: Distractors don't reflect real learner confusions—'prolong interval' and 'make material seem harder' are poorly motivated.
- spacing-effect: draft 15: This duplicates draft 1 and the distractors are trivially weak ('reviewed only once', 'studied over a short period').
- spacing-effect: draft 16: Short-answer format with no distractors is used for a compound two-part question that would work better split.
- spacing-effect: draft 17: This duplicates draft 10 almost exactly; redundancy makes it unpublishable.
- apostles-creed: draft 1: Distractors are plausible but somewhat generic and don't reflect common confusions a learner would realistically make about creeds.
- apostles-creed: draft 2: Distractors are too close in meaning to the correct answer and blur together without testing a meaningful distinction.
- apostles-creed: draft 5: 'On the day after he died' is logically equivalent to 'the second day' and exposes the answer by elimination, weakening distractor quality significantly.
- apostles-creed: draft 6: Distractors are paraphrases of the correct answer rather than genuinely distinct wrong answers, making them too transparent.

## Totals

- Provider failures: 0/13 sources
- Mean provenance: 100% · mean answerability: 99% · mean key-term coverage: 98% · count-in-range: 4/13
- Intent shape matches: 9/13 sources
- Content fit matches: 0/3 sources · mean required-unit coverage 67%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0137 · mean per source: $0.0010
- Latency p50: 665ms · p95: 2128ms
