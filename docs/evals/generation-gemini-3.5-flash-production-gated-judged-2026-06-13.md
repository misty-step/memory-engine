# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: openai/gpt-5.4
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 5 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1088/1925 | $0.0190 | 2869ms |
| nato-alphabet | enumerable-list | 8 | 100% | 100% | 100% | 0% | yes | 33% | yes | 0% | 1056/2619 | $0.0252 | 2753ms |
| http-caching | technical-doc | 4 | 100% | 100% | 50% | 0% | yes | 100% | yes | 0% | 1117/4494 | $0.0421 | 2490ms |
| rubicon | narrative-history | 6 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1096/2102 | $0.0206 | 2580ms |
| sourdough | how-to | 4 | 100% | 100% | 100% | 0% | yes | 67% | NO | 0% | 1119/3832 | $0.0362 | 2548ms |
| gdpr-basis | regulatory | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1106/1677 | $0.0168 | 2447ms |
| hope-feathers | verbatim-verse | 7 | 100% | 100% | 100% | 0% | NO | 100% | yes | 0% | 1049/5399 | $0.0502 | 2512ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1106/3860 | $0.0364 | 2534ms |
| curie | biography | 6 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 1107/2270 | $0.0221 | 2713ms |
| git-branching | product-doc | 4 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1096/2370 | $0.0230 | 2859ms |
| spacing-effect | long-science-prose | 5 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1291/3833 | $0.0364 | 2699ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1022/1974 | $0.0193 | 2430ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 5.0 | 3.4 | 60% | $0.0000 |
| nato-alphabet | 5.0 | 4.8 | 2.9 | 88% | $0.0000 |
| http-caching | 5.0 | 4.0 | 3.0 | 100% | $0.0000 |
| rubicon | 5.0 | 5.0 | 3.8 | 100% | $0.0000 |
| sourdough | 4.5 | 3.5 | 2.8 | 75% | $0.0000 |
| gdpr-basis | 4.7 | 3.3 | 3.0 | 33% | $0.0000 |
| hope-feathers | 5.0 | 2.4 | 2.9 | 0% | $0.0000 |
| pythagorean | 5.0 | 3.8 | 3.0 | 75% | $0.0000 |
| curie | 5.0 | 4.3 | 3.0 | 67% | $0.0000 |
| git-branching | 4.5 | 4.0 | 3.0 | 50% | $0.0000 |
| spacing-effect | 4.8 | 3.6 | 3.0 | 0% | $0.0000 |
| water-boiling | 5.0 | 4.3 | 2.7 | 0% | $0.0000 |

- Judge means: faithfulness 4.9 · question quality 4.0 · distractors 3.0 · keep rate 54% · judge cost $0.0000

Judge would not keep:

- mitochondria: draft 2: Distractors are mostly implausible technical nonsense.
- mitochondria: draft 3: Distractors are fabricated and not plausible confusions.
- nato-alphabet: draft 3: Distractors rely on superficial variants and weak alternatives.
- sourdough: draft 4: The answer omits that the source also requires a feeding procedure, not just frequency.
- gdpr-basis: draft 1: Question is too compound for one card.
- gdpr-basis: draft 2: Question adds 'formal administrative action' wording not in source.
- hope-feathers: draft 1: The question is too trivial and tests only one adjacent line.
- hope-feathers: draft 2: The item is a compound recall prompt for two exact lines.
- hope-feathers: draft 3: The prompt is far too long and non-atomic for spaced repetition.
- hope-feathers: draft 4: The item is a compound two-line continuation prompt.
- hope-feathers: draft 5: The item asks for two exact lines at once rather than one atom.
- hope-feathers: draft 6: The prompt is overly long and tests a whole stanza verbatim.
- hope-feathers: draft 7: The prompt is extremely overbroad and unsuitable for one review card.
- pythagorean: draft 3: Question is compound because it asks for both definition and examples.
- curie: draft 3: Distractors include options not parallel to the asked discovery.
- curie: draft 4: Distractors are weak and partly nonparallel terms.
- git-branching: draft 1: The condition is phrased imprecisely and may invert target/current branch roles.
- git-branching: draft 3: It adds 'pushed' and extra explanation not stated in the source.
- spacing-effect: draft 1: Question is wordy and contrasts two ideas instead of testing one clean atom.
- spacing-effect: draft 2: Question is slightly redundant and the answer overstates memory 'strength'.
- spacing-effect: draft 3: Question is unnecessarily verbose for a simple point.
- spacing-effect: draft 4: Question is somewhat broad and jargon-heavy.
- spacing-effect: draft 5: Question is a bit long and asks for a causal chain rather than one atom.
- water-boiling: draft 1: Distractors are mostly generic temperature guesses.
- water-boiling: draft 2: Distractors are weak near-number guesses.
- water-boiling: draft 3: Distractors include unsupported causes from outside the source.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 96% · mean key-term coverage: 81% · count-in-range: 11/12
- Intent shape matches: 9/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.3471 · mean per source: $0.0289
- Latency p50: 2548ms · p95: 2869ms
