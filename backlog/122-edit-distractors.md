---
id: 122
status: ready
priority: p1
type: bug
---

# Edit drafts including distractors and stay in review

## Outcome

Approving a generated multiple-choice draft lets the learner see and edit
the distractors. Cancel and save both return to the review session that was
in progress, not a bare Home.

## Why now

A Bridge draft showed prompt + expected answer only. Edit has no distractor
fields (`render_pending_drafts`). Cancel left the review session.

## Acceptance

- [ ] Pending MCQ drafts show every choice, including distractors, before
      Keep.
- [ ] Edit can change prompt, correct answer, and each distractor. The saved
      prompt stays a coherent MCQ (correct value present in choices).
- [ ] Cancel / back from edit resumes the in-progress review, not `/`.
- [ ] Keep as written still works in one tap.

## Dependencies

None.

## Proof

Phone: generate or bridge an MCQ, inspect distractors, edit one distractor,
land back in review.

## Non-goals

No rich item editor. No new draft types.
