# QA 074 — Honest browser response time before grading (2026-07-09)

Legacy work item: `memory-engine-074`. The rendered review form used to post a
hard-coded `responseTimeMs=1800` for every answer, so every mature correct
answer looked like fast recall and rated `Easy` (the rating policy treats a
correct answer with `prior_reps >= 3` at or under 4 000 ms as `Easy`). This
walk verifies the fix end to end against a live local server.

## What changed

- `crates/memory-engine-api-render/src/render.rs` renders the review form
  with a **blank** `responseTimeMs`.
- `crates/memory-engine-api/assets/app.js` fills the field with the real
  presentation-to-submit elapsed milliseconds at the moment of submission
  (progressive enhancement — JavaScript off submits the blank).
- `crates/memory-engine-api/src/routes.rs` accepts the field as raw text and
  sanitizes it: valid positive values pass through clamped to
  `MAX_PLAUSIBLE_RESPONSE_TIME_MS` (600 000 ms); missing, blank, malformed,
  negative, zero, or overflowing values map to that same conservative
  ceiling, which can never rate `Easy`. The typed `SubmitReviewRequest`
  boundary and the JSON v1 API are unchanged.

## Live walk (phone-sized)

Setup: `memory-engine-api` debug build on `127.0.0.1:8474`, file store,
allowlisted `qa074@example.com`, debug sign-in links; agent-browser with
iPhone 12 emulation (390 × 844), HAR capture on.

Flow: magic-link sign-in → captured the two-concept NATO structured source →
background generation scheduled both cards → reviewed both.

### Slow correct attempt (free response)

Presented the CAT composition card, deliberately idled, then typed
`CHARLIE ALFA TANGO` and submitted.

- Browser posted `responseTimeMs=13631` — the real elapsed time, not 1 800.
- Graded `Correct`; scheduler feedback "you'll see this again in ~1 hour"
  (fresh card, learning step).
- Screenshots: `074-slow-card-presented-phone.png`,
  `074-slow-correct-graded-phone.png`.

### Fast correct attempt (multiple choice)

Advanced to the NATO-letter-A card and clicked `ALFA` immediately.

- Browser posted `responseTimeMs=287`.
- Graded `Correct`; same learning-step horizon (Easy-vs-Good divergence only
  exists for mature cards — proven at the route boundary below).
- Screenshot: `074-fast-correct-graded-phone.png`.

### Captured `/app/submit` payloads (HAR, csrf omitted)

```
POST /app/submit -> 200
  reviewUnitId=generated-exercise-src-…-nato-cat-composition
  answer=CHARLIE ALFA TANGO   responseTimeMs=13631
POST /app/submit -> 200
  reviewUnitId=generated-quiz-src-…-nato-letter-a
  answer=ALFA                 responseTimeMs=287
```

### Persisted attempt timing (file store readback)

`store/acct_…/study.json` after the walk:

```
/attempts[0] responseTimeMs: 13631  (nato-cat-composition)
/attempts[1] responseTimeMs: 287    (nato-letter-a)
```

The scheduler now receives the learner's actual effort, and stored attempts
are usable for later response-time analysis.

## Mature-card rating proof (route boundary, no internal mocks)

`crates/memory-engine-api/src/tests/mod.rs`:

- `review_form_leaves_response_time_blank_for_honest_measurement` — the
  rendered form carries `name="responseTimeMs" value=""` and no constant.
- `mature_correct_answers_rate_easy_only_when_genuinely_fast` — drives the
  real router under an advancing registry clock: matures a card through
  three correct reviews (magic-link re-sign-in between cycles, since browser
  sessions are a fixed 14 days), then submits a fourth correct answer. Slow
  (6 500 ms) rates `Good`; fast (900 ms) rates `Easy` with a strictly longer
  next-review interval; six dishonest shapes (missing field, blank,
  `not-a-number`, `-250`, `0`, 20-digit overflow) all grade successfully and
  land on exactly the slow control's interval — never `Easy`.
- `sanitize_response_time_maps_dishonest_shapes_to_the_conservative_ceiling`
  — unit coverage of the sanitizer policy.

## Gates

- `bun run ci` (fmt, workspace tests, clippy `-D warnings`, rustdoc): green.
- `bun run ci:full`: recorded in the delivery evidence.

## Residual

- The local dogfood hosts (`memory-engine-web-shell`, `memory-engine-beta-app`)
  still render their own hard-coded `responseTimeMs=2400`; out of scope here
  (production browser loop only), flagged for backlog.
- Inherent limit, unchanged by this fix: the browser owns the timer, so a
  hostile client can still post a small-but-valid value (e.g. `1`) and rate
  `Easy` on a mature card. The server has no presented-at anchor to check
  against; this fix closes the dishonest-shape surface (missing, malformed,
  negative, huge), not the low-value surface, which is exactly as forgeable
  as before.
- The timing script in `app.js` has no automated test (the repo has no JS
  execution harness). The route-boundary tests post literal values instead.
  Reviewed by hand: the app is full-page-reload SSR, so `shownAt` resets on
  every navigation, and any bfcache staleness inflates elapsed time — the
  safe direction (toward `Good`, never toward `Easy`). The live walk above
  is the executable evidence for the script.
