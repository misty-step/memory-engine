# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-item-writer) · max_drafts: 5 · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | shape | variants | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1988/3174 | $0.0315 | 3542ms |
| nato-alphabet | enumerable-list | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1955/1728 | $0.0185 | 2988ms |
| http-caching | technical-doc | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 2016/2835 | $0.0285 | 2627ms |
| rubicon | narrative-history | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 1996/2567 | $0.0261 | 2736ms |
| sourdough | how-to | 3 | 100% | 100% | 100% | 0% | yes | 33% | yes | 0% | 2018/2185 | $0.0227 | 2840ms |
| gdpr-basis | regulatory | 2 | 100% | 100% | 100% | 0% | yes | 100% | yes | 0% | 2006/3064 | $0.0306 | 2518ms |
| hope-feathers | verbatim-verse | 3 | 100% | 100% | 100% | 0% | yes | 67% | yes | 0% | 1947/2159 | $0.0224 | 2904ms |
| pythagorean | math-concept | 3 | 100% | 100% | 100% | 0% | yes | 0% | yes | 0% | 2005/2116 | $0.0221 | 2899ms |
| curie | biography | 3 | 100% | 100% | 100% | 0% | yes | 50% | yes | 0% | 2006/2221 | $0.0230 | 3015ms |
| git-branching | product-doc | 3 | 100% | 100% | 67% | 0% | yes | 25% | yes | 0% | 1995/3746 | $0.0367 | 3149ms |
| spacing-effect | long-science-prose | 3 | 100% | 100% | 67% | 0% | yes | 50% | yes | 0% | 2190/2491 | $0.0257 | 3230ms |
| water-boiling | tiny-fact | 2 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 1922/1516 | $0.0165 | 3247ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 5.0 | 4.0 | 3.7 | 67% | $0.0055 |
| nato-alphabet | 5.0 | 5.0 | 4.0 | 67% | $0.0057 |
| http-caching | 5.0 | 4.3 | 3.3 | 67% | $0.0056 |
| rubicon | 5.0 | 4.3 | 3.7 | 33% | $0.0058 |
| sourdough | 5.0 | 5.0 | 4.0 | 67% | $0.0059 |
| gdpr-basis | 5.0 | 4.5 | 4.0 | 100% | $0.0049 |
| hope-feathers | 5.0 | 3.0 | 3.0 | 33% | $0.0056 |
| pythagorean | 4.7 | 4.0 | 3.0 | 33% | $0.0062 |
| curie | 5.0 | 4.7 | 3.3 | 33% | $0.0056 |
| git-branching | 5.0 | 4.7 | 3.3 | 67% | $0.0056 |
| spacing-effect | 5.0 | 5.0 | 3.7 | 67% | $0.0070 |
| water-boiling | 5.0 | 5.0 | 3.5 | 100% | $0.0042 |

- Judge means: faithfulness 5.0 · question quality 4.5 · distractors 3.5 · keep rate 61% · judge cost $0.0676

Judge would not keep:

- mitochondria: draft 3: The question is compound and essay-style, making it poorly suited for spaced-repetition flashcard format.
- nato-alphabet: draft 3: Distractors 'Fox' and 'Frank' feel too obviously wrong compared to real NATO alternatives like 'Freddie' or 'Firefly'.
- http-caching: draft 3: Distractors are implausible fabrications that no real learner would confuse with the correct answer.
- rubicon: draft 2: Distractors like 'usurpation' and 'sacrilege' are somewhat implausible as legal Roman charges a learner might genuinely confuse with treason.
- rubicon: draft 3: Question has a minor grammatical awkwardness ('boundary separation') that should be cleaned up before publishing.
- sourdough: draft 2: Third distractor ('exactly 24 hours without feeding') is partially plausible but oddly specific and lazy compared to the others.
- hope-feathers: draft 2: The question is somewhat low-value since 'soul' is a single common word and the fill-in tests trivial recall of an unremarkable line-ending.
- hope-feathers: draft 3: Asking the learner to 'recite' an entire line is a compound recall task that lacks a precise cue, making the question vague and hard to verify.
- pythagorean: draft 1: Distractors are too obviously wrong since 'flat planes' and 'Euclidean surfaces' are essentially the same wrong answer and no real learner would confuse them with curved surfaces.
- pythagorean: draft 2: The question asks to 'explain mechanism-wise' which is vague and compound, and the answer adds slight inferential language ('prevents') not directly stated in the source.
- curie: draft 1: First distractor invents a Nobel Peace Prize win that never happened, making it trivially dismissible.
- curie: draft 3: Distractors are implausible fabrications no real learner would confuse with the actual nickname.
- git-branching: draft 3: Short-answer format is acceptable here but the lack of distractors reduces its utility as a quiz item for self-testing.
- spacing-effect: draft 2: Short-answer format is acceptable here, but the lack of distractors weakens its value as a quiz item since the concept of recognition-vs-recall is nuanced enough to benefit from foils.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 94% · mean key-term coverage: 55% · count-in-range: 12/12
- Intent shape matches: 11/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.3043 · mean per source: $0.0254
- Latency p50: 2904ms · p95: 3542ms
