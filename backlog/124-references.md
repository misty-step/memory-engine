---
id: 124
status: design
priority: p1
type: feature
---

# Punch out to durable concept references

## Outcome

Reference punches the learner out of the quiz into durable study material
for this concept, then back into review. The app has two primary objects:
quiz items and study references. A reference is generated once and attached
to the concept, not rebuilt on every tap.

## Why now

Overflow Reference on NATO letter N inserted a one-line "Reference / NATO
phonetic alphabet" under the answers. That is not a study surface.

## Open design

- Vocabulary lock: quiz item vs reference. One term per object.
- Where the reference lives: concept page, source page, or a new object.
- What punch-out means on a phone: full page, sheet, or browser.
- Who owns generation: first Reference tap, capture time, or keep time.
- How the note stays grounded in the source.

## Acceptance

- [ ] Domain vocabulary locked in `VISION.md` or a short design note and
      used in the PWA.
- [ ] A reference is a durable, concept-attached object. A second Reference
      tap does not spend another model call if the note exists.
- [ ] Reference leaves the answering chrome and shows the broader concept
      plus the atom.
- [ ] Back returns to the same quiz item, unanswered if it was unanswered.
- [ ] Implementation does not start until the vocabulary and punch-out form
      are locked.

## Dependencies

[118](118-gemini-37.md) if generation must run to seed a missing note.

## Proof

Phone: Reference on letter N opens the NATO note focused on N; back resumes
that card. A second tap is a read, not a generation.

## Non-goals

No course generator. No automatic remediation packs. No chat tutor.
