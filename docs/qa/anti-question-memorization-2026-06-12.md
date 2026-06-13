# Anti-question Memorization QA

Refs-backlog: 054

## Scope

Verified the beta study boundary for retrieval variability:

- same-concept same-stage variants persist as separate review units;
- multiple-choice choices render in a changing deterministic order;
- post-answer feedback exposes success and response-time trend hooks;
- the live mobile-width beta app shows choices, feedback, and the next variant.

## Automated Evidence

```sh
cargo test -p memory-engine-study --test beta_study
cargo test -p memory-engine-api v1_json_api_returns_post_answer_feedback_and_concept_progress
cargo test -p memory-engine-api v1_openapi_artifact_matches_registered_routes
cargo test -p memory-engine-bench generation::tests::variant_quality_requires_distinct_same_concept_stage_phrasings_without_answer_leakage -- --exact
cargo test -p memory-engine-generation --test beta_generation structured_generation_preserves_same_stage_variants_for_one_concept -- --exact
jq empty docs/api/openapi.v1.json
cargo run -p memory-engine-bench -- generation
cargo test -p memory-engine-beta-app
```

Observed results:

- study session: 20 passed;
- API feedback contract: 1 passed;
- API OpenAPI route and required-field contract: 1 passed;
- bench variant-quality judge: 1 passed;
- generation variants: 1 passed;
- OpenAPI JSON parsed cleanly;
- deterministic generation receipt printed `variants` at 100% for
  `pythagorean` and the variant-gated `water-boiling` source, 0% for rows with
  no same-concept same-stage variant group, with 0 provider failures and 12/12
  intent shape matches;
- beta app: 10 passed.

The generation receipt still shows the existing non-gating count-in-range misses
for `http-caching` and `spacing-effect`, and the expected key-term coverage
misses on fixture-generated rows.

## Live Browser QA

Command:

```sh
BETA_STUDY_STORE=/tmp/memory-engine-054-live-qa/store.json cargo run -p memory-engine-beta-app
```

Browser setup:

- in-app browser;
- viewport override: 390 x 844;
- target: `http://127.0.0.1:4174`;
- clean store: `/tmp/memory-engine-054-live-qa/store.json`.

Flow:

1. Added a structured source with three `NATO letter A` `recognition-3`
   variants.
2. Generated three accepted drafts and approved all three.
3. Verified first review displayed three choices in projected order:
   `CHARLIE`, `ALFA`, `BRAVO`.
4. Submitted wrong answer `BRAVO`.
5. Verified feedback showed `0 of 1 correct (0.0%)`, `last seen just now`, and
   response-time trend text.
6. Clicked `Next`.
7. Verified the next review changed to the sibling prompt `Choose the code word
   used for the letter A.` with choices `ALFA`, `BRAVO`, `CHARLIE`.

Artifacts:

- `/tmp/memory-engine-054-live-qa/first-review.png`
- `/tmp/memory-engine-054-live-qa/answer-feedback.png`
- `/tmp/memory-engine-054-live-qa/next-variant.png`
- `/tmp/memory-engine-054-live-qa/browser-observations.json`

## Residual Risk

The response-time trend is a queryable beta-study projection over existing
attempt records, not a production heuristic. Future memorization detection
still needs a shaped threshold and product decision before it changes queue or
schedule policy.
