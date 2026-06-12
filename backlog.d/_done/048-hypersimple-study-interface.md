# Make the study interface pristine and hyper-simple

Priority: P0 · Status: pending · Estimate: L

## Goal

The deployed study app embodies Scry's hypersimplicity doctrine: open it on
a phone, the most-needed review appears instantly, answer it, see the next —
zero chrome, zero machinery talk, pristine typography. First contact
(docs/dogfood/first-contact-2026-06-11.md) proved the current UI blocks
dogfooding outright; this ticket is the gate to daily use.

## Oracle

- [ ] Opening the app with due reviews shows exactly one prompt and an answer
      affordance — no source list, draft list, account panel, or pipeline
      state on the review screen.
- [ ] No demo data anywhere: the NATO prefill (`render_start_form` /
      `render_source_form`) is gone; forms open empty with a one-line
      placeholder hint, and a first-run empty state explains the loop in
      plain words ("Add something you want to learn").
- [ ] Post-login surface never shows the account ULID or an email re-entry
      form ("Save account email"); identity is at most a small signed-in-as
      line with sign-out.
- [ ] No internal vocabulary reaches the user: "drafts", "attempts",
      "validation reasons", raw `activity_stage` values, and `{:?}` verdict
      debug formatting are all replaced with human language.
- [ ] Sources can be deleted/archived from the management surface; the
      Antikythera QA seed in the production account is purged via that path,
      and QA never seeds real accounts again (throwaway QA account).
- [ ] Paste→generate→keep is one obvious flow with honest progress feedback;
      generation failure states are human sentences, not validation strings.
- [ ] Real counts, no comfort features: due count shown plainly (Scry
      doctrine: "if 300 are due, you see 300").
- [ ] Design receipts: before/after phone-viewport screenshots committed
      under docs/design/; a design critique pass (hierarchy, type, spacing,
      density) is run on the live deployed app, not local mocks.
- [ ] Lighthouse mobile performance + accessibility ≥ 90 on the deployed app.

## Notes

First-contact findings (2026-06-11) made the ticket-37 critique ("the UI
still explains the machinery") concrete: account ULID in a `<code>` block,
NATO structured-block prefill ("intimidating — makes me feel like I don't
know what I'm supposed to do"), QA seed data with no delete path, opaque
"1 sources, 3 drafts, 0 reviews, 0 attempts" metrics, review buried at the
bottom of one dense column. Scry (`../scry`, vision.md) is the canonical
aesthetic: quizzes over flashcards, no daily limits, brutal honesty. This
interface is a prototype destined to move into Scry (see 049) — keep it
server-rendered and dependency-light so the design transfers as patterns,
not as a framework.

## Children

1. Kill demo prefill + account-panel machinery; plain-language vocabulary
   pass over every rendered string.
2. IA split: review-first screen; sources/material behind one management
   surface with delete/archive; purge QA seed data.
3. Honest empty/failure/progress states everywhere (pairs with 043 child 2).
4. Design pass on type/spacing/hierarchy with phone screenshots as receipts.
5. Live design critique + Lighthouse receipts on the deployed app.
