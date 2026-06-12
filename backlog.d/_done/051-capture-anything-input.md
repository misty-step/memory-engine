# Capture anything: one input, intent-aware generation

Priority: P1 · Status: pending · Estimate: L

## Goal

Adding study material is one obvious affordance (a plus/add button) that
accepts arbitrary input — a word, a phrase, a paragraph, a pasted article —
and the engine infers what the user wants to learn and the right learning
modality before generating. Memorizing a poem demands verbatim-recitation
items; understanding mitochondria demands concept/application items. The
user never fills in "title" and "source text" as separate framed fields and
never sees structured-block syntax.

## Oracle

- [ ] One capture affordance takes free-form text from one word to a full
      article; title is inferred (editable later), not demanded up front.
- [ ] An intent-classification step (model-backed, behind the existing
      `DraftProvider`-style boundary) labels the capture with a learning
      goal — at minimum: verbatim memorization (poem, quote, list),
      concept understanding, fact recall, procedure/process — and the
      generation prompt branches on it; eval fixtures in
      memory-engine-bench cover at least one source per intent and assert
      the produced item shapes differ accordingly.
- [ ] Verbatim-memorization captures produce recitation-ladder items
      (existing progression stages), not multiple-choice trivia about the
      text.
- [ ] Generation latency is honest in the UI: the user sees progress and
      can leave/return; a capture never silently produces nothing
      (zero-draft and provider-failure notices already exist — they render
      as human sentences here).
- [ ] Image capture is explicitly scoped in or out with a one-paragraph
      decision note (cost/latency/OCR quality); if out, the ticket says why
      and what would change the call.

## Notes

This is the user's stated capture story: "encountering something out in the
wild… click a plus button… punch in some kind of arbitrary input… have some
pretty intelligent AI use the context that it knows about the user to make
strong assumptions." Builds directly on 043's provider boundary and 047's
eval harness — intent classification is a new prompt + eval lane, not a new
architecture. pdf-to-interactive-lesson (see first-contact report) shows
parallel generation stages keeping perceived latency low; pair with 055.
Depends on 048 landing first so the capture affordance has a sane home.

## Children

1. Single free-form capture affordance + inferred titles (UI + route).
2. Intent classifier behind the provider boundary + branched generation
   prompts.
3. Bench fixtures per intent; assert item-shape divergence (recitation vs
   concept items).
4. Image-input decision note.
