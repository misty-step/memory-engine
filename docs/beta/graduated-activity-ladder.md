# Graduated Activity Ladder

Refs-Powder: memory-engine-032

## Model

The beta study ladder treats activity variants as separate review units that
share one concept key and one progression group. A concept can move from
recognition into less cued recall and finally into composition or practice
exercise work without changing the underlying concept identity.

The first deterministic fixture is the NATO phonetic alphabet:

1. `recognition-3`: choose `ALFA` from three shuffled choices.
2. `recognition-5`: choose `ALFA` from five shuffled choices.
3. `typed-recall`: type the NATO word for `A`.
4. `composition`: spell `CAT` as `CHARLIE ALFA TANGO`.

Each accepted draft gets its own `reviewUnitId`, prompt id, attempt trail, and
schedule state. Later variants require mastery of the prior accepted variant in
the same progression group. Unlocking a harder variant does not copy the easier
variant's schedule state; the harder unit starts with its own fresh schedule
history and preserves its own attempt provenance.

Multiple-choice variants can change answer-choice count while keeping the same
concept key. The beta study projection rotates choices deterministically from
the review-unit id and attempt count so the correct answer is not positionally
fixed across reviews, while still preserving the same `correctChoice` and
concept identity.

Exercise fixtures can carry a worked solution and a scoring rubric. The NATO
composition fixture records both so QA can distinguish a harder application
task from merely a different wording of the same recall prompt.

## Boundary

The kernel remains pure and unchanged. It owns prompt grading, FSRS schedule
transitions, progression metadata semantics, queue eligibility, and queue
selection.

The service boundary remains the application-facing command surface. Typed
recall and composition exercise submissions both go through `grade/apply-review`;
`next-queue` handles progression unlocks from persisted schedule state. Reveal
is still display-only UI state and has no service command.

The beta app owns ladder authoring and interpretation: deterministic source
blocks, activity stage names, distractors, choice-count pressure, worked
solutions, scoring rubrics, draft approval, mobile session projection, and
dogfood copy. None of this is a public package export in this ticket.

## Dogfood Friction

- Stage names are still convention-based strings. That is acceptable for a
  deterministic beta fixture, but broader authoring would need tighter
  validation before promotion.
- Progression currently follows accepted draft order within a concept group.
  That makes the fixture legible, but a future authoring surface will need a
  way to inspect or edit ordering before approval.
- Composition exercises still use deterministic exact/recitation grading. The
  rubric is persisted for QA and display, but rubric-aware scoring remains a
  separate adapter concern.
- The beta session can prove unlocked typed recall and exercise review through
  the service, but it still lacks a richer learner-facing explanation of why a
  stage unlocked.

## Promotion Criteria

Promote ladder concepts only after repeated beta receipts show that more than
one source domain needs the same provider-neutral fields. The likely promotion
threshold is evidence that concept identity, variant ordering, and exercise
rubric metadata remain stable across deterministic fixtures and provider-backed
drafts.

Do not promote distractor wording, mobile display state, source parsing,
generated scenario templates, or activity-stage copy until separate consumers
need the same semantics.
