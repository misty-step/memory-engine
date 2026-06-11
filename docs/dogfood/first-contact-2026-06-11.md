# First contact: operator's first real login — 2026-06-11

The operator logged into production (`memory-engine-api.fly.dev`) for the
first time as a genuine user. Verdict: **not dogfoodable yet.** This report
is the design oracle for the usability grooming wave (tickets 048, 051–056).
Quotes are lightly edited from the operator's voice notes.

## What happened, step by step

1. **The magic link landed in spam.** Login worked only after digging it out
   of Gmail's spam folder. (`onboarding@resend.dev` shared sender, no
   verified domain → ticket 056.)
2. **Logged-in page opens with machinery, not study.** "It's giving me some
   long string for the account. It says save account email for some reason."
   — the page shows the raw account ULID in a `<code>` block and an email
   re-entry form *after* the user just authenticated by email.
3. **Add-source form arrives pre-filled with demo content.** Title
   "NATO practice notes" and a textarea full of structured-block syntax
   (`Concept: … / Stage: … / Question: … / Distractor: …`). "That is
   intimidating and makes me feel like I don't know what I'm supposed to do
   here." The prefill is a developer convenience leaking to production.
4. **Mystery data.** A source about the Antikythera mechanism appeared that
   the user never created — it is QA seed data from the deployment receipt
   run (2026-06-11), written into the production account during smoke
   verification. There is no way to delete it from the UI.
5. **Opaque metrics.** "Progress: 1 sources, 3 drafts, 0 reviews, 0
   attempts. I don't know what any of this means." Internal pipeline
   vocabulary (drafts, attempts, validation reasons, activity stages like
   `recognition`) is rendered verbatim.
6. **Everything on one page.** Account form + add-source + source list +
   generated material + review item stacked in one column; the actual review
   is at the bottom.
7. **Aesthetics.** "Designed and styled aesthetically terribly." No visual
   hierarchy, default-form look.

## What the operator actually wants (product intent)

- **Open the app → immediately review.** The home surface is the next due
  quiz item, nothing else. (048)
- **Escape hatches while reviewing.** When an item is too hard or the
  underlying concept isn't understood: punch out to reference material
  (linked, or generated on demand); request *bridge material* — generated
  reference + easier quiz items that walk from the user's demonstrated
  ability up to the item they're failing, informed by what they're already
  acing; or snooze/skip and move on. (052)
- **Capture anything.** One add/plus affordance accepting arbitrary input —
  a word, a paragraph, a pasted article, an image — with the model inferring
  what the user wants to learn and the right learning modality. Memorizing a
  poem needs different items than understanding mitochondria. (051)
- **Feedback after answering.** Stats on this concept and this question:
  how have I done historically, what changed. (053)
- **Don't let me memorize the question.** Mitigate pattern-matching the
  prompt instead of knowing the concept — vary retrieval form across
  reviews. (054, grounded by 050's literature pass)
- **Design worth using daily.** Full aesthetic pass. (048)

## Reference

[Nutlope/pdf-to-interactive-lesson](https://github.com/Nutlope/pdf-to-interactive-lesson)
— operator-flagged as worth learning from. Notable patterns: parallel
generation stages to cut perceived latency; an explicit *repair* stage that
regenerates or hides weak/duplicate questions (Jaccard dedup, no extra model
calls); diverse item types (short answer, true/false, MCQ, process
ordering); source-anchored questions. Feeds tickets 051 and 055.

## Immediate ops follow-ups (not tickets)

- Purge the Antikythera QA seed source from the production account once a
  delete path exists (048 child); stop seeding QA data into real accounts —
  QA runs get their own throwaway account.
