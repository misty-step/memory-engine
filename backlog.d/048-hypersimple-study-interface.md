# Make the study interface pristine and hyper-simple

Priority: P1 · Status: pending · Estimate: M

## Goal

The deployed study app embodies Scry's hypersimplicity doctrine: open it on
a phone, the most-needed review appears instantly, answer it, see the next —
zero chrome, zero machinery talk, pristine typography.

## Oracle

- [ ] Opening the app with due reviews shows exactly one prompt and an answer
      affordance — no source list, draft list, or pipeline state on the
      review screen.
- [ ] Paste→generate→keep is one obvious flow with honest progress feedback;
      generation failure states are human sentences, not validation strings.
- [ ] Real counts, no comfort features: due count shown plainly (Scry
      doctrine: "if 300 are due, you see 300").
- [ ] Design receipts: before/after phone-viewport screenshots committed
      under docs/design/; a design critique pass (hierarchy, type, spacing,
      density) is run on the live deployed app, not local mocks.
- [ ] Lighthouse mobile performance + accessibility ≥ 90 on the deployed app.

## Notes

The current UI (`memory-engine-api` render fns) is a single dense page mixing
account/session mechanics, source CRUD, drafts, and review — the ticket-37
critique ("the UI still explains the machinery") still applies. Scry
(`../scry`, vision.md) is the canonical aesthetic: quizzes over flashcards,
no daily limits, brutal honesty. This interface is a prototype destined to
move into Scry (see 049) — keep it server-rendered and dependency-light so
the design transfers as patterns, not as a framework.

## Children

1. IA split: review-first screen; sources/drafts behind one management
   surface.
2. Design pass on type/spacing/hierarchy with phone screenshots as receipts.
3. Honest empty/failure/progress states everywhere (pairs with 043 child 2).
4. Live design critique + Lighthouse receipts on the deployed app.
