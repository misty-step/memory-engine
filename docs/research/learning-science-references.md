# Learning Science References

This note captures research inputs for `memory-engine` API and dogfood-client
design. It is a local reference pack, not a literature review claim of
exhaustiveness.

Refs-backlog: 20
Refs-backlog: 21
Refs-backlog: 22
Refs-backlog: 23
Refs-backlog: 26
Refs-backlog: 27
Refs-backlog: 28
Refs-backlog: 29
Refs-backlog: 31

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

## Evidence Matrix

This matrix is the durable design context for backlog shaping. It does not make
every finding a kernel feature. It records what the product should test before
promoting new API surface.

| Evidence | What it supports | Product/API consequence |
| --- | --- | --- |
| Dunlosky et al. (2013) rate practice testing and distributed practice as high utility across many learners and domains. | Active recall and spacing should be first-order product loops. | The beta interface should optimize for fast retrieval attempts, not passive reading or card management. |
| Roediger and Karpicke (2006) show test-enhanced learning improves delayed retention relative to repeated study. | Quiz/retrieval loops are a better default than review-only flashcards. | `memory-engine` attempts are the canonical learning event; reveal without attempt should be tracked differently from graded retrieval. |
| Karpicke and Roediger (2008) argue retrieval itself is critical, not merely an assessment proxy. | Correctness is not the only signal; the act of recall matters. | Store attempts, latency, confidence, reveal state, and retry/repair metadata so future evals can reason about learning behavior. |
| Cepeda et al. (2006, 2008) support distributed practice and show gap timing depends on retention interval. | Scheduling policy must be explicit and replayable. | Keep persisted schedule state JSON-safe and version future scheduler policies rather than hiding schedule mutations in UI code. |
| Rowland (2014) meta-analysis supports testing over restudy for retention. | Beta success should include retention-oriented behavior, not just completion. | Add evals or receipts for repeated retrieval over time once a persisted beta exists. |
| Agarwal, Nunes, and Blunt (2021) reinforce retrieval practice benefits in educational contexts. | Product should make retrieval low-friction and frequent. | Mobile-first interface should reduce time-to-first-attempt and keep answer entry ergonomic. |
| Brunmair and Richter (2019) show interleaving helps, especially where discrimination between similar categories matters. | Queue policy should mix intentionally, not randomly. | Use concept/source/domain metadata for confusable contrast and anti-clumping; add queue explanation before adding opaque AI queueing. |
| Hattie and Timperley (2007), Wisniewski et al. (2019) show feedback effects depend on content, level, and timing. | Feedback is a policy and pedagogy layer, not a binary correct/incorrect decoration. | Keep grade details rich enough for clients to implement immediate feedback, delayed feedback, hints, and repair loops. |
| Butler, Karpicke, and Roediger (2008) connect feedback and confidence after testing. | Calibration can be a learning signal. | Experiments should capture predicted confidence versus actual outcome before promoting confidence fields to core. |
| MIT Teaching + Learning Lab metacognition guidance emphasizes monitoring and regulation. | Learners need visibility into what they think they know versus what they can retrieve. | Beta should include lightweight calibration and review-state explanations, not only due counts. |
| Kalyuga (2007) expertise reversal effect warns that high guidance can become counterproductive. | Worked examples and hints should fade as mastery grows. | Progression metadata can represent worked example -> cued attempt -> cloze -> free recall -> transfer, while copy and lesson flow stay client-owned. |

## Beta Interface Research Requirements

The next usable interface needs persistence, but persistence should live outside
`src/` until repeated experiments prove a stable package contract. For beta
work, the application layer should own:

- local database tables or files for learner-owned content, generated prompts,
  attempts, schedules, sources, references, and generation provenance;
- import/generation workflows for typed text, pasted documents, uploaded files,
  images, links, and eventually video transcripts;
- reference links and source passages attached to generated prompts;
- review-session state, UI copy, hints, feedback timing, confidence prompts,
  and repair flows;
- privacy policy for what source material may be sent to model providers.

The shared kernel should remain responsible for:

- canonical prompt, grade, schedule, progression, and queue types;
- pure grading, scheduling, progression, and queue primitives;
- eval fixtures that replay learning semantics;
- adapter contracts only after at least two clients show repeated pressure.

## Ticket Mapping

- `26-beta-persistence-spine`: durable attempts and schedule state make
  retrieval practice and spacing auditable across sessions.
- `27-ai-content-generation-probe`: generated quizzes must be grounded in
  sources and approved before they become retrieval prompts.
- `28-mobile-beta-study-interface`: phone-first answer entry and review flow
  test whether retrieval practice is actually low friction.
- `29-service-contract-v0-hardening`: reveal, feedback, retry, and typed
  failure semantics decide whether review events are trustworthy learning
  evidence.
- `31-beta-extraction-decision`: extraction waits until beta evidence shows
  which learning workflow contracts are stable.

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
