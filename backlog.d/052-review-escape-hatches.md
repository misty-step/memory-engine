# Review escape hatches: skip, reference, bridge material

Priority: P1 · Status: pending · Estimate: L

## Goal

A stuck reviewer is never trapped. From any quiz item the user can: skip or
snooze it and move to the next; punch out to reference material that
explains the underlying concept (linked if it exists, generated on demand if
not); or request *bridge material* — generated reference plus easier quiz
items that walk from what the user demonstrably knows up to the item they
are failing.

## Oracle

- [ ] Skip/snooze: one tap defers the current item and surfaces the next due
      item; the deferral has defined scheduler semantics (documented in the
      kernel: what happens to `ScheduleState`, and that skipping is not a
      failed review) with kernel tests.
- [ ] Reference punch-out: every review item can reach reference material
      for its concept. Source-backed items link their evidence/source
      passage; when none exists, "explain this" generates a short reference
      note via the provider boundary and persists it on the concept (cached,
      not regenerated per view).
- [ ] Bridge material: from a struggling item, one affordance generates a
      small set of easier items (lower progression stage / more scaffolded)
      plus a reference note, using the failing item, its concept, and the
      user's recent performance as prompt context; bridge items enter the
      queue ahead of the parent item, and the flow has an end-to-end test
      through the study boundary.
- [ ] All three affordances live on the review screen without cluttering it
      (048's one-prompt rule still holds — escape hatches are secondary
      affordances, not chrome).
- [ ] An eval/bench scenario judges bridge-material quality: easier than the
      parent item, faithful to the concept, no duplicate of existing items.

## Notes

Direct user intent from first contact: "insofar as any quiz item is too
hard, or they don't actually understand the underlying material… punch out
to reference material… generate bridge material… a combination of reference
material and quiz items that are easier… take them from where we think they
are to the knowledge and capabilities necessary to ace the quiz question
they are struggling with. And then they should also be able to snooze or
skip." The progression-stage ladder (ticket 32) already models difficulty
tiers — bridge generation targets lower rungs of the existing ladder rather
than inventing a new difficulty notion. Scheduler semantics for snooze must
respect kernel purity (invariant 2: scheduler takes state in, returns state
out). Depends on 048 (review-first screen to hang these on); pairs with 050
(desirable-difficulty literature should inform what "easier" means).

## Children

1. Kernel snooze/skip semantics + tests; study-boundary route + UI.
2. Reference punch-out: evidence-linked first, generated-and-cached
   fallback.
3. Bridge generation: prompt with performance context, queue insertion
   ahead of parent, end-to-end test.
4. Bridge-quality eval scenario in memory-engine-bench.
