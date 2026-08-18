---
id: 120
status: proof
priority: p1
type: bug
---

# Shuffle multiple-choice order every presentation

## Outcome

Every time a multiple-choice card is presented, the choices appear in a
fresh order. Position is not a memory cue.

## Why now

NATO cards showed choices in stored order. `projected_choices` only rotated.

## Acceptance

- [x] Seeded Fisher-Yates on `(review_unit_id, display_attempts)`.
- [x] Graded recap uses `attempts - 1` so order matches the presentation.
- [x] `graded_mcq_recap_keeps_presentation_choice_order` covers
      `submit_answer`.
- [ ] One phone card on production changes order across reviews.

## Dependencies

None.

## Proof

Merged as `9c78d59` (#127). Deployed in host release `391fb55`.
Phone walk remains.

## Non-goals

No new prompt type. No client-only shuffle.
