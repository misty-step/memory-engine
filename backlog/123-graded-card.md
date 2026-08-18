---
id: 123
status: design
priority: p1
type: feature
---

# Tighten the post-grade card

## Outcome

After an answer, the phone shows a tight result: verdict, when the card
returns, and Continue. Secondary facts do not compete with that.

## Why now

Answering is fine. The post-grade stack is not: Correct, next-interval,
Stage, Last seen, Success, Concept, Keep/Drop, Why, Continue. That chrome
lives in `render_graded_review`.

Do not implement a new layout until the hierarchy is locked.

## Open design

- What must remain on the card vs move to Home / concept?
- Keep/Drop card quality: on-card, overflow, or later?
- How small can the result be and still teach the schedule?
- Visual hierarchy on a 390-wide phone.

## Acceptance

- [ ] Written hierarchy lock (what stays, what leaves, what becomes overflow).
- [ ] Phone mock or production preview of one correct MCQ result.
- [ ] Continue remains explicit. No auto-advance.
- [ ] Implementation follows the lock.

## Dependencies

None. Pair with [119](119-instant-actions.md) for speed; this item owns
hierarchy, not fetch.

## Proof

Operator-approved phone result for one NATO-style MCQ.

## Non-goals

No analytics redesign. No new grade verdicts. No kernel schedule change.
