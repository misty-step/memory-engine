# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled) · judge: anthropic/claude-sonnet-4.6
- Corpus: 12 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1471/1419 | $0.0150 | 1736ms |
| nato-alphabet | enumerable-list | 9 | 1 | 0 | 90% | 100% | 100% | 0% | NO | 100% | yes | 0% | 100% | 100% | 3023/4804 | $0.0478 | 3125ms |
| http-caching | technical-doc | 7 | 3 | 0 | 70% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3189/4383 | $0.0442 | 3023ms |
| rubicon | narrative-history | 5 | 3 | 0 | 62% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3183/4446 | $0.0448 | 3280ms |
| sourdough | how-to | 8 | 2 | 0 | 80% | 100% | 100% | 0% | yes | 67% | NO | 0% | 100% | 100% | 3184/4761 | $0.0476 | 3004ms |
| gdpr-basis | regulatory | 5 | 1 | 0 | 83% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3114/5887 | $0.0577 | 2898ms |
| hope-feathers | verbatim-verse | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1432/3492 | $0.0336 | 1373ms |
| pythagorean | math-concept | 4 | 2 | 0 | 67% | 100% | 100% | 0% | yes | 67% | yes | 0% | 100% | 100% | 3188/4305 | $0.0435 | 2929ms |
| curie | biography | 7 | 1 | 0 | 88% | 100% | 100% | 0% | yes | 100% | yes | 0% | 100% | 100% | 3118/5773 | $0.0566 | 2671ms |
| git-branching | product-doc | 7 | 0 | 0 | 100% | 100% | 86% | 0% | yes | 100% | yes | 0% | 100% | 100% | 1479/2894 | $0.0283 | 1572ms |
| spacing-effect | long-science-prose | 5 | 1 | 0 | 83% | 100% | 100% | 0% | yes | 75% | yes | 100% | 100% | 100% | 3500/4305 | $0.0440 | 2807ms |
| water-boiling | tiny-fact | 3 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | 0% | 100% | 100% | 1405/1631 | $0.0168 | 1383ms |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| mitochondria | 4.8 | 4.8 | 4.0 | 75% | $0.0069 |
| nato-alphabet | 5.0 | 4.9 | 3.3 | 100% | $0.0115 |
| http-caching | 5.0 | 5.0 | 4.0 | 100% | $0.0090 |
| rubicon | 4.8 | 4.8 | 3.8 | 60% | $0.0084 |
| sourdough | 5.0 | 4.6 | 4.2 | 88% | $0.0101 |
| gdpr-basis | 5.0 | 4.8 | 4.0 | 80% | $0.0075 |
| hope-feathers | 4.8 | 4.2 | 3.5 | 50% | $0.0068 |
| pythagorean | 5.0 | 5.0 | 3.8 | 100% | $0.0063 |
| curie | 4.9 | 4.6 | 3.7 | 57% | $0.0103 |
| git-branching | 5.0 | 4.9 | 3.9 | 57% | $0.0090 |
| spacing-effect | 5.0 | 4.6 | 3.6 | 60% | $0.0086 |
| water-boiling | 5.0 | 5.0 | 4.3 | 67% | $0.0053 |

- Judge means: faithfulness 4.9 · question quality 4.8 · distractors 3.8 · keep rate 74% · judge cost $0.0996

Judge would not keep:

- mitochondria: draft 3: Platelets and T cells are not strictly 'blood cells' in the same category sense, and the source only specifies red blood cells, making distractors feel imprecise.
- rubicon: draft 2: Two distractors ('et tu Brute', 'sic semper tyrannis') are not Latin phrases Caesar would have said at the Rubicon and feel cheap; only 'veni vidi vici' is a genuine Caesar confusion.
- rubicon: draft 5: The source says both Pompey and 'much of the Senate' retreated, so singling out Pompey is fair, but Crassus was already dead by 49 BC making him a poor distractor.
- sourdough: draft 8: This is a near-duplicate of draft 2 and adds no new learning value.
- gdpr-basis: draft 3: 'Formal privacy impact assessment' is a real GDPR concept and may mislead learners into thinking it is correct.
- hope-feathers: draft 2: Distractors are weak—'without a sound' and 'without a rhythm' are not confusions a real learner would make given the poem's phrasing.
- hope-feathers: draft 4: The answer is imprecise—the poem says the storm must be 'sore,' not that the storm abashes the bird; the question slightly misrepresents the conditional logic.
- curie: draft 2: Faithfulness is slightly imprecise: Linus Pauling and Frederick Sanger also won two Nobels, making the answer contestable without the 'two different sciences' qualifier in the answer.
- curie: draft 4: Distractor 'radium and uranium' reuses the correct answer element 'radium', which is a giveaway by elimination.
- curie: draft 7: Short-answer format is acceptable, but the absence of distractors makes this a weaker multiple-choice candidate given the very specific French phrase being tested.
- git-branching: draft 2: Distractors are all arbitrary byte counts with no pedagogical grounding.
- git-branching: draft 3: ORIGIN and MASTER are not pointers tracking current branch, making distractors weak confusions.
- git-branching: draft 6: Distractors are implausible actions that rebase obviously does not do, reducing learning value.
- spacing-effect: draft 2: 'The generation effect' is a real phenomenon that could confuse, but 'primacy effect' is unrelated enough to be a weak distractor.
- spacing-effect: draft 5: Near-duplicate of draft 3 with weaker distractors; 'productive struggle' and 'optimal friction' are less confusing than draft 3's options.
- water-boiling: draft 3: Only two distractors are provided and 'remains exactly the same' is weak; a third plausible distractor is missing.

## Totals

- Provider failures: 0/12 sources
- Mean provenance: 100% · mean answerability: 99% · mean key-term coverage: 92% · count-in-range: 11/12
- Intent shape matches: 8/12 sources
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.4798 · mean per source: $0.0400
- Latency p50: 2807ms · p95: 3280ms

## Rigor read — vs 2026-06-11 baseline (paired, source-clustered)

Read per `harness-kit/harnesses/shared/references/verification-system-first.md`
("Eval & Benchmark Rigor"). Baseline:
`generation-gemini-3.5-flash-judged-2026-06-11.md` (same generator, prompt,
judge, and 12 sources). Deltas are paired per source (after − before), n = 12
source-level observations.

| metric | baseline | post-055 | paired Δ | 95% CI | verdict |
| --- | --- | --- | --- | --- | --- |
| distractors (1–5) | 3.8 | 3.8 | +0.02 | [−0.18, +0.22] | within noise |
| keep rate | 70% | 74% | +4.0pp | [−11.6, +19.6] | within noise |

Both CIs include 0, so neither change is distinguishable from run-to-run noise.
Per-source keep swings −33pp…+50pp (generation is non-deterministic, ~2–4 drafts
per source), which is where the wide interval comes from.

**Power.** Detecting a ~3pp keep change at 80% power / 5% needs ~1000 drafts;
this suite has ~12 sources (~30–40 drafts) and resolves only large regressions.

**Verdict for 055 oracle 1.** The judged field run does **not** show a
statistically detectable distractor-quality or keep-rate improvement over the
baseline — but it confirms **no large regression** (faithfulness 4.9, provenance
100%, 0 provider failures). The oracle as written ("improved over baseline")
cannot be met by a 12-source Likert suite: reframe it to "no large regression",
or expand the corpus and switch to binary judge criteria (ticket 058) before
claiming a small improvement. The distractor 1–5 Likert is itself non-actionable
per the rigor reference.
