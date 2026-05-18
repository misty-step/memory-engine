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
local persistence and product workflow outside `src/`.

The interface must save content, generated quiz drafts, approved review units,
attempts, schedule updates, reference material, generation provenance, and
session results. That database is necessary for dogfood usefulness. It should
not make the published kernel database-owned.

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
| `26-beta-persistence-spine` | Store sources, generated drafts, review units, attempts, schedules, references, and run receipts outside `src`. | Retrieval/spaced-practice evidence requires durable attempts and schedule history. |
| `27-ai-content-generation-probe` | Generate source-grounded quiz drafts with provenance and validation. | AI output must be cited, evaluated, and approved before entering review. |
| `28-mobile-beta-study-interface` | Mobile-first local study shell for real dogfood use. | Low-friction retrieval attempts are the product proof. |
| `29-service-contract-v0-hardening` | Decide DTOs, reveal semantics, typed failures, and shared store harness after beta pressure. | Stable contracts should follow repeated interface evidence. |
| `30-backlog-hygiene-and-qa-receipts` | Keep tracker and QA evidence trustworthy. | Long-running product learning needs reliable evidence trails. |
| `31-beta-extraction-decision` | Decide whether to extract/promote/keep experimenting after beta evidence. | Extraction requires repeated pressure across clients or workflows. |

## Success Criteria For "Usable Beta"

The beta is useful when a user can:

1. add source material;
2. generate and approve quiz drafts grounded in that source;
3. review approved items on a phone-sized interface;
4. persist attempts, outcomes, schedule changes, and references;
5. inspect why an item was shown, what happened, and what to study next;
6. quit, restart the process, and continue from saved state;
7. retry safely after a duplicate submit or failed write without corrupting
   schedule history;
8. review from a realistic uneven pile, not only a two-item fixture;
9. produce at least one repeated-use dogfood receipt from actual study.

Anything less is still an API experiment, not a beta learning interface.
