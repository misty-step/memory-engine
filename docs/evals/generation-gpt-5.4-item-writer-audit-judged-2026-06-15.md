# Generation eval receipt

- Provider: openrouter/openai/gpt-5.4 (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1522/633 | $0.0000 | 471ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1477/444 | $0.0000 | 322ms |
| http-caching | technical-doc | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1530/772 | $0.0000 | 327ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1523/583 | $0.0000 | 250ms |
| sourdough | how-to | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1542/771 | $0.0000 | 399ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1522/660 | $0.0000 | 877ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1461/418 | $0.0000 | 358ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1529/801 | $0.0000 | 346ms |
| curie | biography | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1525/526 | $0.0000 | 360ms |
| git-branching | product-doc | 3 | 100% | 100% | 67% | 0% | yes | 100% | yes | 0% | 1511/570 | $0.0000 | 272ms |
| spacing-effect | long-science-prose | 4 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1703/869 | $0.0000 | 1667ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1437/539 | $0.0000 | 304ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.7 | 3.3 | 33% | $0.0056 |
| nato-alphabet | 5.0 | 4.7 | 4.3 | 100% | $0.0050 |
| http-caching | 5.0 | 5.0 | 4.0 | 100% | $0.0054 |
| rubicon | 5.0 | 4.7 | 3.7 | 67% | $0.0054 |
| sourdough | 5.0 | 5.0 | 4.3 | 100% | $0.0062 |
| gdpr-basis | 4.0 | 3.3 | 3.0 | 33% | $0.0055 |
| hope-feathers | 5.0 | 3.7 | 3.0 | 67% | $0.0054 |
| pythagorean | 5.0 | 5.0 | 4.0 | 100% | $0.0066 |
| curie | 5.0 | 5.0 | 3.7 | 67% | $0.0056 |
| git-branching | 5.0 | 4.7 | 3.7 | 100% | $0.0052 |
| spacing-effect | 5.0 | 4.8 | 3.0 | 25% | $0.0075 |
| water-boiling | 5.0 | 4.7 | 3.7 | 67% | $0.0054 |

- Judge means: faithfulness 4.9 · question quality 4.6 · distractors 3.6 · keep rate 72% · judge cost $0.0688

Judge would not keep:

- mitochondria: draft 2: Distractors include skin cells and muscle cells which are not mentioned in the source, making them weakly motivated placeholders.
- mitochondria: draft 3: Short-answer format is acceptable but the answer is a verbose restatement that could be tightened to two distinct factual atoms.
- rubicon: draft 3: Campania is a region not a city, making it a weaker distractor that breaks the parallel structure.
- gdpr-basis: draft 1: One distractor ('performance of a task in the public interest') is actually a correct answer, making the question misleading.
- gdpr-basis: draft 3: The compound question tests two atoms at once, weakening its utility as a spaced-repetition item.
- hope-feathers: draft 3: 'Recite' framing makes the question feel like a dictation exercise rather than a meaningful recall prompt, reducing its pedagogical clarity.
- curie: draft 3: Distractors 'radiation' and 'nuclear physics' are too easily eliminated as one is a broader concept and the other clearly not a coined term.
- spacing-effect: draft 2: Atkinson and Shiffrin are absent from the source, making that distractor obviously wrong to anyone familiar with the text's scope.
- spacing-effect: draft 3: Short-answer format is acceptable but the absence of distractors reduces utility for a concept that has common misconceptions worth testing.
- spacing-effect: draft 4: Third distractor directly contradicts the source's logic (lenient grading makes retrieval less effortful, not more), making it an obviously wrong implausible option.
- water-boiling: draft 3: 'A much hotter temperature' is obviously wrong and not a confusion a real learner would make, weakening distractor quality.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 97% · mean key-term coverage: 83% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.0000 · mean per source: $0.0000
- Latency p50: 346ms · p95: 1667ms
