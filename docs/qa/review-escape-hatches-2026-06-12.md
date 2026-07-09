# Review escape hatches QA receipt

Refs-Powder: memory-engine-052

Date: 2026-06-12

## Local surface

Started a throwaway file-store API instance with model generation forced to the
deterministic fake provider:

```sh
env -u OPENROUTER_API_KEY -u MEMORY_ENGINE_GENERATION_MODEL \
PORT=18080 \
MEMORY_ENGINE_ENABLE_FILE_STORE=true \
MEMORY_ENGINE_API_STORE_DIR=/tmp/memory-engine-052-live-qa/store \
MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=learner@example.com \
MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=/tmp/memory-engine-052-live-qa/outbox.txt \
cargo run -p memory-engine-api
```

Browser flow used the server-rendered app at `http://127.0.0.1:18080/`:
request magic link, open the local outbox link, save capture, create review,
keep the exercise draft, then press `Reference`, `Bridge`, `Skip`, and
`Snooze`.

Browser evidence was saved locally for this run:

- `/tmp/memory-engine-052-live-qa/browser-observations.json`
- `/tmp/memory-engine-052-live-qa/review-escape-hatches-final.png`

## Browser observations

- Signed in through the local magic-link outbox.
- Saved source text covering NATO CAT composition and generated one accepted
  exercise draft through the rendered app.
- Kept the composition draft through the rendered UI. The review screen kept
  the one-prompt rule and rendered secondary `Reference`, `Skip`, `Snooze`,
  and `Bridge` actions alongside the answer/reveal controls.
- `Reference` kept the same current item and rendered the source-backed
  passage: `C is CHARLIE. A is ALFA. T is TANGO.`
- `Bridge` moved from the parent item to the first scaffolded bridge question,
  `Which smaller cue helps with "Spell CAT over the phone using the NATO
  phonetic alphabet."?`, and increased the due count from `1 due` to `2 due`.
- `Skip` moved from the recognition bridge item to the cued-recall bridge
  exercise, `Use the cue "CHARLIE ALFA TANGO" to answer the original item in
  one step.`, and decreased the due count from `2 due` to `1 due`.
- The bridge-only browser flow had no normal sibling left after snoozing the
  second bridge item, so the same live server/account was extended with two
  sibling review units through the public V1 API. Snoozing active `BEE`
  surfaced `FOX` as the next due item with `attemptCount: 0`.

## Store verification

After the browser flow plus the sibling V1 snooze check:

```sh
jq '. as $root | {attempts: (.attempts | length), generationRuns: [.generationRuns[] | {id, status, parentReviewUnitId}], conceptReferenceNotes: (.conceptReferenceNotes | length), queue: ([.reviewUnits[] | . as $unit | {reviewUnitId: .reviewUnitId, due: .queue.due, activityStage: ($root.generatedPromptDrafts[]? | select(.id == $unit.generatedPromptDraftId) | .activityStage), snoozedUntil: .snoozedUntil}] | sort_by(.due))}' \
  /tmp/memory-engine-052-live-qa/store/acct_fc9e1ff15d47bd67/study.json
```

Verdict:

- `attempts` was `0`; neither skip nor snooze recorded a failed review.
- The bridge run persisted
  `parentReviewUnitId: generated-exercise-src-cfbeccacf75c13d8-1-nato-cat-composition`.
- One concept reference note was cached for the bridge material.
- Bridge rows used distinct scaffold stages: `recognition-bridge` before
  `cued-recall-bridge`, both due ahead of the deferred parent.
- Sibling snooze moved from `BEE` to due `FOX`, leaving the snoozed row with a
  future `snoozedUntil`.
