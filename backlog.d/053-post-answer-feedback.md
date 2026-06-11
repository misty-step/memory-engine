# Post-answer feedback and concept analytics

Priority: P2 · Status: pending · Estimate: M

## Goal

Answering a quiz item gives the user real feedback: how they did on this
question and this concept over time, presented in human language on the
result screen — enough signal to feel progress (or honestly see the lack of
it) without dashboard sprawl.

## Oracle

- [ ] After submitting an answer, the result view shows the verdict in human
      words (not `{:?}` debug), the expected answer, and a compact history
      for this item: attempts, success rate, last-seen, current
      stage/interval in plain terms ("you'll see this again in ~4 days").
- [ ] Concept-level rollup: items sharing a concept aggregate (attempts,
      success rate, trend) and the result view names how the concept as a
      whole is going.
- [ ] The data comes from the existing attempt log through the study
      boundary — no new analytics store; queries are tested at the
      service/study layer.
- [ ] Honest by doctrine: no streaks, no badges, no comfort rounding. If
      the user is failing a concept, the screen says so.
- [ ] A management-surface view lists concepts ranked by health (worst
      first) so the user can see where they're weak — one page, no charts
      required for v1.

## Notes

User intent from first contact: "after answering a question… they will want
to see some stats, some analytics, about how they've done on this concept in
general, how they've done on this question in particular." Attempt history
already persists (idempotency-keyed attempts); this is surfacing, not new
collection. Concept identity exists on generated items (concept field) —
the rollup keys on it. Keep the kernel pure: aggregation lives in the
service/study layer, not the scheduler. Depends on 048 (result screen to
build on). Trend data also feeds 052's bridge-material prompt context —
build the query once, share it.

## Children

1. Item-history query + human-language result view.
2. Concept rollup query + result-view line + worst-first concept list.
3. Doctrine pass: wording honest, no comfort features.
