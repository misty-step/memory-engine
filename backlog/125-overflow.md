---
id: 125
status: design
priority: p1
type: feature
---

# Lock snooze-concept grain and Bridge quality

## Outcome

Overflow actions match learner intent. Snooze Concept has a defined grain.
Bridge produces a small set of easier quiz items with the same
draft-approval quality as capture.

## Why now

Skip and snooze were silent (confirmation lives on [119](119-instant-actions.md)).
Snooze Concept looked identical to Snooze on a single-letter card. Bridge
stayed enabled with a looping bar, then produced one pending draft.

## Open design

- Snooze Concept grain: atom (letter R), parent concept (NATO phonetic
  alphabet), or a picker.
- How many Bridge items, and how much easier than the parent.
- Whether Bridge stays in-session or punches out like
  [124](124-references.md).

## Acceptance

- [ ] Written lock for Snooze Concept grain.
- [ ] Written lock for Bridge cardinality and difficulty.
- [ ] After the lock: production Bridge yields more than one pending draft
      on a miss-prone card, with usable styling.
- [ ] [119](119-instant-actions.md) owns in-flight disable and confirmation.
      This item owns meaning.

## Dependencies

[119](119-instant-actions.md), [122](122-edit-distractors.md),
[118](118-gemini-37.md).

## Proof

Operator-approved phone: snooze one atom vs the parent set; Bridge on a
hard card yields multiple inspectable drafts.

## Non-goals

No automatic packs on submit. No knowledge graph.
