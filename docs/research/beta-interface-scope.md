# Beta Interface Scope

Refs-backlog: 24
Refs-backlog: 26
Refs-backlog: 27
Refs-backlog: 28
Refs-backlog: 29
Refs-backlog: 30
Refs-backlog: 31

## Purpose

This note synthesizes the learning-science and AI-system references into the
next product direction: a repo-local beta interface that can be used for real
study sessions while preserving the pure `memory-engine` kernel boundary.

## Strategic Decision

Build the beta interface in this repository as an experiment, with its own
local persistence and product workflow outside `crates/memory-engine-core`.

The interface must save content, generated quiz/exercise drafts, approved
review units, attempts, schedule updates, reference material, generation
provenance, and session results. That database is necessary for dogfood
usefulness. It should not make the published kernel database-owned.

## Research Commitments

Learning science points to these product commitments:

- Retrieval practice is the central learning event. The beta should optimize
  for fast attempts, not passive browsing.
- Spacing matters, but timing remains policy. Persist schedule state and review
  events so scheduler behavior is replayable.
- Feedback must be specific and policy-driven. Store enough grade and attempt
  detail for feedback, reveal, repair, and calibration experiments.
- Interleaving should be intentional. Preserve source, domain, concept, and
  progression metadata for queue decisions.
- Guidance should fade. Represent progression relationships without hardcoding
  lesson copy in the kernel.
- Activity variation should prevent card memorization. Represent concept-level
  ladders that can progress from recognition to recall to composition or
  problem-solving exercises.

AI-system research points to these product commitments:

- Embeddings help with similarity, clustering, and retrieval; they do not prove
  correctness.
- RAG needs separate retrieval and generation receipts.
- Generated content needs provenance, validation, and approval before review.
- Tutoring should be structured as narrow pedagogical actions, not generic
  chat.
- Model graders and judges need evals, disagreement handling, and regression
  tracking.
- Source permissions and model-send policy are product data, not hidden
  implementation details.

## Boundary

Belongs in the beta app/service shell:

- local database or file-backed store;
- source ingestion and metadata for typed text, pasted documents, uploads,
  photos, links, and later video transcripts;
- generated prompt drafts, critiques, approval state, and provenance;
- generated exercise drafts, worked solutions, validation state, and
  provenance;
- reference material and cited spans connected to review units;
- review history, attempt timeline, feedback/reveal state, and session summary;
- mobile UI, copy, state presentation, hints, repair flows, and privacy policy;
- provider adapters, model prompts, retrieval indexes, and agent run receipts.

Belongs in the shared kernel/API:

- canonical prompt, grade, schedule, progression, queue, and review-unit types;
- pure grading, scheduling, progression, and queue primitives;
- provider-neutral adapter contracts only after repeated beta/client pressure;
- fixtures and evals that prove stable learning semantics.

## Ticket Mapping

| Ticket | Scope | Research basis |
| --- | --- | --- |
| `24-extraction-decision-gate` | Decide not to extract yet; use evidence to shape beta. | Current dogfood has one interactive client and no persistent beta. |
| `26-beta-persistence-spine` | Store sources, generated drafts, review units, attempts, schedules, references, and run receipts outside the pure kernel. | Retrieval/spaced-practice evidence requires durable attempts and schedule history. |
| `27-ai-content-generation-probe` | Generate source-grounded quiz/exercise drafts with provenance and validation. | AI output must be cited, evaluated, and approved before entering review or practice. |
| `28-mobile-beta-study-interface` | Mobile-first local study shell for real dogfood use. | Low-friction retrieval attempts and simple exercises are the product proof. |
| `29-service-contract-v0-hardening` | Decide DTOs, reveal semantics, activity-kind metadata, typed failures, and shared store harness after beta pressure. | Stable contracts should follow repeated interface evidence. |
| `30-backlog-hygiene-and-qa-receipts` | Keep tracker and QA evidence trustworthy. | Long-running product learning needs reliable evidence trails. |
| `31-beta-extraction-decision` | Decide whether to extract/promote/keep experimenting after beta evidence. | Extraction requires repeated pressure across clients or workflows. |
| `32-graduated-activity-ladder` | Add concept-level variant ladders and deterministic exercise progression after the basic beta loop exists. | Transfer requires varied practice and guidance fading, but the kernel should not absorb pedagogy prematurely. |

## Graduated Activity Ladder

Use a ladder rather than one static quiz item per concept. The first beta path
should support explicit, deterministic variants:

1. Recognition: multiple choice with shuffled distractors and adjustable choice
   count.
2. Cued recall: boolean, cloze, or typed answer with hints or partial context.
3. Free recall: typed answer without choices.
4. Composition: combine mastered atomic concepts into a real-world task.
5. Generative exercise: produce a fresh scenario or practice problem with a
   worked solution and validation evidence.

For NATO phonetic alphabet learning, the ladder can move from "A -> ALFA" as
multiple choice, to typed recall, to spelling "CAT" over the phone. For options
learning, the ladder can move from defining Gamma, to interpreting Gamma in a
position, to scenario exercises about hedging under price/volatility changes.

Boundary rule: the beta app owns activity authoring, variant selection,
distractor construction, worked solution copy, and generated scenarios. The
kernel may keep stable metadata only after beta evidence proves a repeatable
contract: concept keys, progression groups, stage order, prerequisites,
supersession, queue metadata, and grade/schedule results.

## Success Criteria For "Usable Beta"

The beta is useful when a user can:

1. add source material;
2. generate and approve quiz drafts grounded in that source;
3. generate or approve exercise drafts when the domain calls for practice
   problems;
4. review approved items and solve simple exercises on a phone-sized
   interface;
5. persist attempts, outcomes, schedule changes, and references;
6. inspect why an item was shown, what happened, and what to study next;
7. quit, restart the process, and continue from saved state;
8. retry safely after a duplicate submit or failed write without corrupting
   schedule history;
9. review from a realistic uneven pile, not only a two-item fixture;
10. produce at least one repeated-use dogfood receipt from actual study.

Anything less is still an API experiment, not a beta learning interface.
