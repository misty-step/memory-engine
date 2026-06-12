# Post-answer feedback QA

Refs-backlog: 053

## Scope

Verified post-answer feedback and concept health against the server-rendered
mobile app, plus focused Rust tests for study/API contracts and the browser
textarea CRLF generation path.

## Local App

```sh
env -u OPENROUTER_API_KEY -u MEMORY_ENGINE_GENERATION_MODEL \
PORT=18083 \
MEMORY_ENGINE_ENABLE_FILE_STORE=true \
MEMORY_ENGINE_API_STORE_DIR=/tmp/memory-engine-053-live-qa/store \
MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=learner@example.com \
MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=/tmp/memory-engine-053-live-qa/outbox.txt \
cargo run -p memory-engine-api
```

Browser flow used Chrome at a 390 x 844 viewport:

1. Request magic link for `learner@example.com`, then open the outbox link.
2. Save a two-block NATO capture through the rendered textarea form.
3. Generate review drafts from that browser-submitted source.
4. Keep the first draft, answer the NATO letter item incorrectly with `BRAVO`,
   and verify the result view.
5. Advance, keep the remaining CAT composition draft, answer it correctly, and
   verify the management concept list.

Artifacts:

- `/tmp/memory-engine-053-live-qa/post-answer-feedback.png`
- `/tmp/memory-engine-053-live-qa/concept-health-worst-first.png`
- `/tmp/memory-engine-053-live-qa/browser-observations.json`

Observed verdicts:

- Browser-submitted CRLF source generated two keepable drafts.
- The result view showed `Try again`, expected answer `ALFA`, item history
  `1 attempt, 0 of 1 correct (0.0%)`, and the concept line for `nato letter a`.
- The result view did not expose `reviewState` or `scheduleChange`.
- The final concept list placed `nato letter a` before
  `nato cat composition`, marked the weak concept as `struggling`, and used no
  chart, streak, or badge wording.
- Once both drafts were approved, the keep-flow no longer showed already
  approved drafts.

## Focused Commands

```sh
cargo test -p memory-engine-generation browser_form_line_endings_preserve_multiple_structured_blocks
cargo test -p memory-engine-study --test beta_study
cargo test -p memory-engine-api --lib
jq empty docs/api/openapi.v1.json
```

All focused commands passed locally on June 12, 2026.
