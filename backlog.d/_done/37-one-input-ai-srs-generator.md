ticket: 37
status: delivered
shaping: false

# One-input AI SRS generator

## Goal

Make the beta study path start from one arbitrary text input, generate
source-grounded learning artifacts with an LLM, and move the learner into a
minimal retrieval-practice session without exposing pipeline mechanics.

## Problem Frame

The current beta app proves the storage, generation, approval, review, reveal,
resume, and duplicate-submit spine, but the UI still explains the machinery.
The learner's job should be dumb simple: paste text, wait for the system to
turn it into cited practice, keep or skip the proposed items, then answer from
memory.

This is not yet a general AI tutor, upload hub, dashboard, or chat app. The
next proof is whether arbitrary pasted prose can become a small, trustworthy
set of reference-linked quiz and exercise drafts that are worth reviewing on a
phone.

## Non-Goals

- No provider SDK, prompt template, network call, database dependency, UI state,
  auth, analytics, or file ingestion under `src/`.
- No chat-first tutor surface.
- No multi-input setup wizard, stepper, persistent queue sidebar, or visible
  draft table on the first screen.
- No automatic promotion of generated content into review without provenance
  and an explicit keep/skip decision.
- No uploads, OCR, PDFs, URLs, images, video transcripts, embeddings, or vector
  indexes in this slice.
- No production hosting work in this ticket.

## Constraints / Invariants

- Runtime `src/` remains framework-free, persistence-free, vendor-free, and
  model-free.
- Generation, prompt templates, provider adapters, source parsing, citations,
  approval UX, reveal state, and session choreography stay in `experiments/`.
- Every accepted generated draft must reference stored source evidence.
- Reveal remains display-only; scheduling changes only after a submitted
  answer.
- Tests must not depend on live model calls. Real providers require deterministic
  fixtures or recorded model-test doubles.
- `ReviewUnitId` remains opaque; concept/phrasing meaning stays outside the
  kernel.

## Authority Order

tests > type system > code > docs > external research > memory

## Repo Anchors

- `experiments/beta-generation/index.ts` - current deterministic source to
  generated draft contract.
- `experiments/beta-generation/beta-generation.test.ts` - provenance,
  validation, duplicate, ladder, and rejection oracles.
- `experiments/beta-store/index.ts` - persistence spine for sources, reference
  spans, generation runs, drafts, review units, attempts, and applied reviews.
- `experiments/beta-study/index.ts` - current session state and review loop over
  generated drafts.
- `experiments/beta-study/index.html` - current UI to replace with the one-input
  phone shell.
- `experiments/beta-study/server.ts` - local HTTP surface for phone testing.
- `docs/beta/content-generation.md` - existing generation contract and eval gaps.
- `docs/beta/mobile-study.md` - current phone proof and friction.
- `docs/research/beta-interface-scope.md` - beta boundary and usable-beta
  success criteria.
- `docs/research/ai-learning-design-brainstorm.md` - learning compiler,
  provenance, and tutor-trace ideas.

## External Research

### Exa

Exa surfaced existing products that converge on the same pattern:
AI-generated notes/quizzes from source material, citations or source links,
editable generated questions, and SRS scheduling. Recall's quiz docs emphasize
generated questions tied to saved cards, optional scheduling, multiple question
types, hints, source review, editing, and pruning. Memo, Tali, Learnedly, and
Quizzer similarly point toward source-grounded generation plus active recall,
not an exposed internal queue.

### xAI / Grok

Grok's web run emphasized three risks: generated cards without provenance,
UI complexity that turns SRS into a dashboard, and hallucinated or low-quality
questions. Its recommendation was a hybrid workflow: AI drafts, user verifies
against sources, then SRS schedules only accepted items. It also called out
schema-enforced structured outputs over regex/JSON parsing.

### Thinktank

Thinktank output directory: `/tmp/memory-engine-one-input-thinktank`.

Status: complete. The verification lane validated the repo boundary and
existing oracles. The systems lane sharpened the main gap: the current
generator is a deterministic key-value parser, not arbitrary-prose AI
generation. It also flagged that browser submissions currently use a hardcoded
`responseTimeMs: 2400`, which would corrupt real scheduling evidence if left in
the phone beta.

### Codebase

The repo already has the right spine for this shape: deterministic generation
creates reference-backed drafts, accepted drafts become review units, and the
service boundary handles `next-queue` plus `grade/apply-review`. The missing
piece is a model-backed arbitrary prose compiler and a UI that hides the
pipeline until user action is required.

## Model Research

The generation task needs:

- long context for arbitrary pasted notes;
- strict structured outputs or tool/function calling;
- low enough latency for a phone beta;
- enough reasoning to extract concepts, evidence spans, distractors, exercise
  rubrics, and stage order without inventing unsupported claims;
- deterministic test doubles so CI never needs a live model.

Live findings on 2026-05-27:

- OpenAI GPT-5.5 is OpenAI's current frontier API model, supports structured
  outputs and function calling, has a 1,050,000 token context window, and is
  priced at $5/M input and $30/M output. Snapshot: `gpt-5.5-2026-04-23`.
- OpenAI GPT-5.2 remains available and is priced at $1.75/M input and $14/M
  output, but GPT-5.5 is the fresher OpenAI family head.
- Anthropic Claude Opus 4.7 is available in the Claude API and major clouds at
  $5/M input and $25/M output.
- Google Gemini 3.5 Flash was announced as available through Gemini API /
  Google AI Studio and positioned for fast, large-scale agentic work.
- OpenRouter's live catalog reports structured-output/tool support and pricing
  for multiple viable candidates, including `google/gemini-3.5-flash`,
  `openai/gpt-5.5`, `anthropic/claude-opus-4.7`, `qwen/qwen3.7-max`, and
  `deepseek/deepseek-v4-flash`.

Recommended model plan for implementation:

1. Start with a `LearningContentGenerator` interface in
   `experiments/beta-generation`; keep the deterministic generator as the
   always-on CI implementation.
2. Add one live provider adapter behind an environment flag. First candidate:
   Gemini 3.5 Flash or GPT-5.5, chosen by a small schema-adherence and
   citation-quality eval before wiring the UI.
3. Keep GPT-5.5 or Claude Opus 4.7 as higher-cost fallback candidates for
   repair/regeneration if the cheaper primary misses schema or evidence quality.
4. Do not commit to a provider until the implementation branch records live
   latency, cost, schema-validity, and unsupported-claim receipts over at least
   20 arbitrary text fixtures.

## Design Options

### 1. Omnibar

One field always visible. Long or multiline input becomes source text; short
input answers the current prompt.

Tradeoff: cleanest possible chrome, but intent inference is fragile. A one-line
fact and a paragraph answer are ambiguous.

### 2. Morphing Field

The first screen is one large text area. After generation, the same surface
morphs into the answer box for the current card.

Tradeoff: preserves the one-control promise and supports study rhythm, but
needs a small "add more notes" escape after a session exists.

### 3. Question-first Recall

After generation, the prompt takes the whole screen. The input appears only
when the learner is ready to answer.

Tradeoff: best for retrieval focus, but adds one tap and is less convenient for
rapid answer entry.

### 4. Lazy Approval Sheet

Generated cards do not appear as a list. A slim banner says "N ready"; tapping
opens one card at a time with Keep / Skip / Edit.

Tradeoff: avoids dashboard sprawl, but approval is less discoverable than a
visible list.

### 5. Auto-start With Undo

The model generates and starts the first card immediately; the user can reject
or edit after seeing the first question.

Tradeoff: fastest path into recall, but too much trust in unproven generation
quality for this repo's current state.

### 6. Reference-first Reading Card

The app first shows a compact AI-written reference note, then one generated
question linked to the cited text.

Tradeoff: improves trust and context for unfamiliar material, but risks
turning the beta into passive reading instead of retrieval practice.

### 7. Chat Tutor

The user pastes text into a chat and the assistant asks questions, explains,
and schedules reviews conversationally.

Tradeoff: familiar, but it hides state, encourages explanation over retrieval,
and makes scheduling/provenance harder to verify.

### 8. Batch Review Console

After paste, show all generated references, quizzes, exercises, queue state,
and validation results.

Tradeoff: excellent for debugging, bad for a phone beta. This is an operator
view, not the learner's primary interface.

## Recommended Synthesis

Use **Morphing Field + Lazy Approval Sheet + Question-first Recall**.

Flow:

1. First screen shows only the brand line, one large text area, and one primary
   button: "Make practice".
2. Submitting saves the source and starts generation. While running, show only
   a compact progress phrase such as "Finding concepts..." and "Checking
   sources...".
3. Generation writes reference spans, draft activities, validation decisions,
   and a generation receipt. It does not create review units automatically.
4. When drafts exist, show one approval sheet card at a time: source excerpt,
   proposed prompt, expected answer, and Keep / Skip / Edit.
5. Keeping a draft creates a review unit. If at least one kept item exists, the
   UI immediately offers "Start practice".
6. Study mode is one prompt, one answer box, optional "Show answer", and
   result feedback. No sidebars, stepper, queue list, raw stage chips, reps, or
   review-state panel on the main phone screen.
7. Reference material is available only as an expandable "Source" affordance on
   the approval card and after feedback.

Why this is best:

- It honors the user's actual complaint: the beta should begin from one input,
  not a workflow diagram.
- It still respects the repo's safety posture: generated content remains
  reviewable and source-grounded before entering SRS.
- It preserves retrieval practice as the product center.
- It keeps the kernel untouched and lets the beta app absorb the product risk.
- It creates a narrow enough implementation slice to test with deterministic
  fixtures and one live model adapter later.

## Implementation Sequence

1. Add a model-generator interface around `runBetaGeneration` while preserving
   the deterministic generator as the default and CI path.
2. Add arbitrary-prose fixture compilation: source text to reference spans,
   concept candidates, draft quizzes, and draft exercises.
3. Add validation for unsupported claims, missing evidence spans, duplicate-ish
   drafts, answer leakage, and missing worked solutions.
4. Surface generation failures in `BetaStudyView` and the browser; raw prose
   that yields no valid drafts must explain why instead of silently showing an
   empty approval state.
5. Replace the beta-study first screen with the one-input shell.
6. Replace the visible draft list with the lazy approval sheet.
7. Replace the study screen with question-first recall and hide all diagnostic
   state behind Details.
8. Measure actual response time in the browser instead of submitting the
   current hardcoded `2400` ms value.
9. Add one live provider adapter behind env configuration and record model,
   latency, token/cost, schema-validity, and rejection receipts.
10. Run phone browser proof over the full one-input path.

## Oracle

- [ ] `bun test experiments/beta-generation/ experiments/beta-study/`
- [ ] `bun run check`
- [ ] `bun run ci`
- [ ] A deterministic arbitrary-prose fixture produces reference spans,
  accepted quiz drafts, accepted exercise drafts, rejected unsupported drafts,
  and validation failures for missing source evidence.
- [ ] Raw prose that cannot produce valid drafts returns visible generation
  failure copy in the phone UI.
- [ ] The first mobile viewport at 390 x 844 contains exactly one primary input
  surface before generation starts.
- [ ] The generated approval flow shows one proposed item at a time with source
  evidence and Keep / Skip / Edit.
- [ ] Kept items enter the review queue; skipped/rejected items do not.
- [ ] Study mode has one prompt, one answer input, Show answer, Submit answer,
  and Next card; no persistent queue/sidebar/stepper/reps panel is visible on
  the main phone screen.
- [ ] Reveal does not increment attempts, reps, or schedule state.
- [ ] Browser submissions use measured elapsed response time, not a hardcoded
  constant.
- [ ] Duplicate submit after grading does not double-count attempts or schedule
  history.
- [ ] Browser smoke at 390 x 844 proves no horizontal overflow through paste,
  generate, keep, answer, reveal, submit, next, and resume.
- [ ] Live-provider proof, when enabled, records provider/model/version,
  latency, token/cost estimate, schema validity, accepted/rejected counts, and
  source-citation coverage; CI remains live-provider-free.

## Risk + Rollout

- Hallucinated or unsupported generated content: mitigate with mandatory
  reference spans, explicit keep/skip, rejection receipts, and live-provider
  evals before dogfood trust.
- Bad learning objects at scale: start with 3-7 drafts, not unlimited deck
  generation.
- UI ambiguity from one field: avoid pure intent inference; after the first
  paste, mode is driven by persisted app state, not guessed input length.
- Latency: generate asynchronously in the beta app; show progress copy and keep
  deterministic fixture tests fast.
- Cost drift: record model and token/cost receipts for every live generation
  run.
- Scope creep: defer uploads, embeddings, chat, hosting, auth, and production
  persistence until the one-input text path earns repeated dogfood receipts.

## What Was Built

- Added a `LearningContentGenerator` seam around beta generation while keeping
  the deterministic fixture generator as the CI path.
- Added arbitrary-prose compilation into source-backed quiz and exercise drafts
  with validation failures when prose has no citeable facts.
- Reworked the beta-study shell into a phone-first one-input flow: paste source,
  generate drafts, keep/skip/edit one draft at a time, then study from a single
  answer box.
- Replaced the hardcoded browser answer latency with measured elapsed response
  time.
- Preserved reveal as display-only UI state and duplicate-submit protection at
  the beta/session boundary.

## Verification Receipt

- `bun test experiments/beta-generation/ experiments/beta-study/`
- `bun run check`
- `bun run ci:local`
- `bun run ci`
- Browser smoke at `390 x 844` against `http://127.0.0.1:4176/` covered paste,
  generate, keep, reveal, submit, duplicate submit, next card, and no horizontal
  overflow. Screenshot: `.tmp/beta-study-037/mobile-smoke.png`.

## Provider Lanes

- `codex`: product critic; accepted the framing "paste text -> small cited
  draft set -> approve -> retrieval".
- `claude`: technical architect; accepted the existing beta pipeline and
  recommended a generator-pluggable extension under `experiments/`.
- `pi`: learning-science critic; accepted source-grounded graduated ladders,
  reveal-is-not-review, and rich attempt metadata.
- `agy`: model strategy; accepted strict schemas, long context, background
  generation, and live schema/cost/latency evaluation.
- `cursor-agent`: mobile UX; accepted morphing field plus lazy approval sheet
  and rejected stepper/sidebar/dashboard UI.
- `grok-build`: contrarian critic; accepted the need for a smallest disproof
  experiment over arbitrary prose before trusting LLM generation.
- `opencode`: QA/oracle designer; accepted focused beta tests, deterministic
  model doubles, mobile browser proof, and boundary verification.

## Delegation Receipts

Receipts are in `.tmp/shape-ai-input-lanes/delegations.jsonl` for this shaping
run. Transcript evidence is under `.tmp/shape-ai-input-lanes/transcripts/`.
