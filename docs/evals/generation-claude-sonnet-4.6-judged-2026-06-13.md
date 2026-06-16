# Generation eval receipt

- Provider: openrouter/anthropic/claude-sonnet-4.6 (prompt-principled) · judge: openai/gpt-5.4
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1583/514 | $0.0125 | 6046ms |
| nato-alphabet | enumerable-list | 5 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1543/476 | $0.0118 | 1567ms |
| http-caching | technical-doc | 5 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1602/611 | $0.0140 | 1151ms |
| rubicon | narrative-history | 5 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1586/563 | $0.0132 | 1801ms |
| sourdough | how-to | 4 | 100% | 100% | 100% | 0% | yes | 33% | NO | 0% | 1617/482 | $0.0121 | 1883ms |
| gdpr-basis | regulatory | 5 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1587/607 | $0.0139 | 4117ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1530/438 | $0.0112 | 1531ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1593/502 | $0.0123 | 1959ms |
| curie | biography | 4 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1591/435 | $0.0113 | 1513ms |
| git-branching | product-doc | 5 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1575/568 | $0.0132 | 1626ms |
| spacing-effect | long-science-prose | 4 | 100% | 100% | 100% | 0% | yes | 75% | yes | 0% | 1801/553 | $0.0137 | 1105ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1499/354 | $0.0098 | 1551ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.5 | 3.2 | 75% | $0.0000 |
| nato-alphabet | 5.0 | 4.0 | 2.0 | 0% | $0.0000 |
| http-caching | 5.0 | 4.6 | 4.2 | 100% | $0.0000 |
| rubicon | 5.0 | 4.8 | 3.4 | 100% | $0.0000 |
| sourdough | 5.0 | 4.8 | 4.2 | 100% | $0.0000 |
| gdpr-basis | 5.0 | 4.8 | 3.4 | 100% | $0.0000 |
| hope-feathers | 5.0 | 3.0 | 3.0 | 33% | $0.0000 |
| pythagorean | 5.0 | 4.5 | 2.5 | 0% | $0.0000 |
| curie | 5.0 | 4.5 | 3.0 | 100% | $0.0000 |
| git-branching | 5.0 | 4.8 | 3.6 | 60% | $0.0000 |
| spacing-effect | 4.5 | 4.2 | 3.8 | 75% | $0.0000 |
| water-boiling | 5.0 | 5.0 | 3.7 | 100% | $0.0000 |

- Judge means: faithfulness 5.0 · question quality 4.5 · distractors 3.3 · keep rate 70% · judge cost $0.0000

Judge would not keep:

- mitochondria: draft 2: Distractors are mostly implausible and not realistic confusions.
- nato-alphabet: draft 1: Distractors are weak and mostly arbitrary B-words.
- nato-alphabet: draft 2: Distractors are weak and mostly arbitrary F-words.
- nato-alphabet: draft 3: Distractors are weak and mostly arbitrary D-words.
- nato-alphabet: draft 4: One distractor invokes outside standards not grounded in likely learner confusion.
- nato-alphabet: draft 5: Distractors are weak and mostly arbitrary G-words.
- hope-feathers: draft 1: This is mostly a line-recall prompt rather than a durable atomic concept.
- hope-feathers: draft 2: It depends on sequential line order instead of a meaningful takeaway.
- pythagorean: draft 1: Distractors are too artificial and not learner-plausible.
- pythagorean: draft 2: One distractor is awkwardly phrased rather than plausibly tempting.
- pythagorean: draft 3: Distractors are broad strawmen rather than close confusions.
- pythagorean: draft 4: The distractors include unsupported claims not grounded in likely confusion.
- git-branching: draft 2: Distractors are mostly weak reference-name fillers.
- git-branching: draft 5: Distractors are weak and mostly artificial rules.
- spacing-effect: draft 4: Answer has a tense/grammar error and is not cleanly phrased.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 81% · count-in-range: 12/12
- Intent shape matches: 9/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.1489 · mean per source: $0.0124
- Latency p50: 1567ms · p95: 6046ms
