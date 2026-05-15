# Learning Science References

This note captures research inputs for `memory-engine` API and dogfood-client
design. It is a local reference pack, not a literature review claim of
exhaustiveness.

Refs-backlog: 20
Refs-backlog: 21
Refs-backlog: 22
Refs-backlog: 23

## Core Findings

### Retrieval Beats Passive Review

Practice testing and retrieval practice are among the strongest, most portable
learning techniques. Dunlosky et al. rate practice testing and distributed
practice as high-utility methods across learner ages, ability levels, and many
content domains. Roediger and Karpicke's test-enhanced-learning work shows that
retrieval practice improves delayed retention relative to repeated study.

Design consequence: `memory-engine` should treat active attempts as the primary
learning event. Passive reveal/re-read can exist, but it should not masquerade
as mastery.

### Spacing Is Robust, But Timing Is A Policy

Distributed practice reliably outperforms massed practice, but optimal gaps
depend on the final retention interval, item history, and workload. FSRS-style
scheduling improves on older ease-factor algorithms by tracking difficulty,
stability, and retrievability rather than only a coarse interval/ease state.

Design consequence: scheduling must stay policy-driven and replayable. The core
should preserve review events and schedule state cleanly enough for dry-runs,
future scheduler migration, and load planning.

### Interleaving Helps When Discrimination Matters

Interleaving is useful when learners must distinguish similar concepts,
strategies, or problem types. It is less useful, and can become noise, when
items are unrelated or when novices lack enough foundation to compare cases.

Design consequence: queue policy should use concept/source/domain metadata to
mix related items intentionally, not globally randomize everything.

### Feedback Needs Content And Timing Policy

Feedback is effective on average, but effects vary substantially. Specific,
task-focused feedback beats generic correctness. Immediate feedback is useful
for preventing error reinforcement, while delayed feedback can be useful when
the goal is transfer, reflection, or calibration.

Design consequence: feedback timing and content should be explicit client or
service policy. The kernel should expose enough grade detail, expected answer,
rubric results, and attempt metadata for clients to implement feedback tiers.

### Calibration Is A First-Class Learning Signal

Learners often misjudge what they know. Delayed judgments of learning and
confidence probes can improve monitoring because they often induce retrieval or
use retrieval as a diagnostic cue. Calibration should be measured as predicted
success versus actual performance, not just raw confidence.

Design consequence: attempts should be able to carry confidence/prediction
metadata in experiments. Evals should include calibration error and
overconfidence-after-reveal scenarios before any core field is promoted.

### Self-Explanation And Generation Are Selective Boosters

Self-explanation and generation have positive evidence, especially when prompts
are concrete, structured, and tied to mechanism or error correction. They are
not universal replacements for retrieval; they are scaffolds that should appear
when the task calls for explanation, transfer, or misconception repair.

Design consequence: clients should experiment with "why was this wrong?",
"which rule applies?", and "generate before reveal" modes. The API should not
hardcode one explanation workflow.

### Worked Examples Need Fading

Worked examples and cognitive-load research support high guidance for novices,
then progressive fading as expertise grows. The expertise-reversal effect warns
against keeping heavy guidance once learners can solve independently.

Design consequence: progression metadata can represent stages such as worked
example, cued attempt, cloze, free recall, and transfer prompt. The core should
own stage relationships and mastery gates, not product-specific lesson copy.

## API Implications

- Keep `AttemptEvent` or service attempt records central. Retrieval attempts,
  response time, grade, feedback mode, confidence, and reveal state are the
  behavioral evidence.
- Add eval scenarios before adding API fields. Candidate eval names:
  `retrieval-beats-reveal`, `failed-recall-needs-feedback`,
  `interleave-similar-not-random`, `worked-example-fades-to-recall`,
  `confidence-calibration-drift`, `misconception-repair-retains`.
- Keep scheduler strategy versioning on the roadmap. FSRS state leakage is
  acceptable today only because it is documented; future scheduler polymorphism
  needs explicit mode/version boundaries.
- Queue explainability should start as debug/eval output: why was a candidate
  due, filtered, locked, buried, anti-clumped, or selected?
- Dogfood clients should measure whole review loops, not isolated helpers:
  attempt -> grade/feedback -> schedule update -> next queue -> reflection.

## Kernel Vs Client Boundary

Belongs in the kernel/API:

- canonical prompt, grade, schedule, progression, and queue contracts
- pure scheduling and queue selection primitives
- deterministic grading and rubric result normalization
- fixture/eval corpora that verify shared learning semantics
- adapter contracts for external graders

Belongs in clients/experiments:

- UI, copy, identity, auth, analytics, and streaks
- content authoring formats and parsers
- session choreography and learner motivation loops
- feedback presentation, hints, and reflection screens
- model provider calls and tutor prompts
- study-mode presets until repeated clients prove a stable abstraction

## Source Index

### Reviews And Meta-Analyses

- Dunlosky, Rawson, Marsh, Nathan, Willingham. "Improving Students' Learning
  With Effective Learning Techniques" (2013).
  https://www.psychologicalscience.org/publications/journals/pspi/learning-techniques.html
- Cepeda, Pashler, Vul, Wixted, Rohrer. "Distributed practice in verbal recall
  tasks" (2006). DOI: 10.1037/0033-2909.132.3.354.
  https://cir.nii.ac.jp/crid/1361418520534596992
- Cepeda et al. "Spacing Effects in Learning" (2008).
  https://journals.sagepub.com/doi/10.1111/j.1467-9280.2008.02209.x
- Rowland. "The Effect of Testing Versus Restudy on Retention" (2014).
  https://www.researchgate.net/publication/264988491_The_Effect_of_Testing_Versus_Restudy_on_Retention_A_Meta-Analytic_Review_of_the_Testing_Effect
- Agarwal, Nunes, Blunt. "Retrieval Practice Consistently Benefits Student
  Learning" (2021).
  https://link.springer.com/article/10.1007/s10648-021-09595-9
- Brunmair and Richter. Interleaving meta-analysis (2019).
  https://pubmed.ncbi.nlm.nih.gov/31556629/
- Wisniewski, Zierer, Hattie. "The Power of Feedback Revisited" (2019).
  https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2019.03087/full
- Hattie and Timperley. "The Power of Feedback" (2007).
  https://assess.ucr.edu/sites/default/files/2019-02/hattietimperley_2007.pdf
- MIT Teaching + Learning Lab. "Metacognition."
  https://tll.mit.edu/teaching-resources/how-people-learn/metacognition/
- Kalyuga. "Expertise Reversal Effect and Its Implications" (2007).
  https://link.springer.com/article/10.1007/s10648-007-9054-3

### Primary Studies And Classic Sources

- Roediger and Karpicke. "Test-enhanced learning" (2006).
  https://pubmed.ncbi.nlm.nih.gov/16507066/
- Karpicke and Roediger. "The Critical Importance of Retrieval for Learning"
  (2008).
  https://www.researchgate.net/publication/5574966_The_Critical_Importance_of_Retrieval_for_Learning
- Butler, Karpicke, Roediger. Feedback and confidence after testing (2008).
  https://pubmed.ncbi.nlm.nih.gov/18605878/
- Rohrer and Taylor. Interleaved mathematics practice.
  https://citeseerx.ist.psu.edu/document?doi=87e71836483e99f64e051650b1f749c2b9cc4bcd&repid=rep1&type=pdf

### Scheduling Systems And Docs

- Anki Manual: Background.
  https://docs.ankiweb.net/background.html
- Anki Manual: Deck Options / FSRS.
  https://docs.ankiweb.net/deck-options
- FSRS algorithm notes.
  https://github-wiki-see.page/m/shigeyukey/fsrs4anki/wiki/The-Algorithm
- SuperMemo SM-2.
  https://supermemo.guru/wiki/Algorithm_SM-2
