# 048 Hypersimple Study Interface Design Receipt

Local receipt captured on a 390 x 844 phone viewport before merge. The before
app ran from `master` at `http://127.0.0.1:18081`; the after app ran from
`cx/048-hypersimple-study-interface` at `http://127.0.0.1:18082`. Both review
screens used the real browser magic-link flow after seeding an allowlisted
account through the JSON API.

## Before

![Before home](048-before-home-phone.png)

![Before review entry](048-before-review-phone.png)

Observed blockers:

- First contact opened on a NATO-filled source form instead of an empty learning capture.
- The review entry viewport was consumed by account machinery, account ULID, email save form, and raw source text before the learner could review.
- Raw structured-source vocabulary such as stages, activities, distractors, and references was visible in the learning surface.

## After

![After home](048-after-home-phone.png)

![After review](048-after-review-phone.png)

Acceptance notes:

- First contact opens to an empty capture form with one plain placeholder hint.
- Signed-in review opens to one due count, one prompt, one answer input, and reveal/answer controls.
- The review viewport omits source lists, generated-card lists, account ids, email re-entry, pipeline counts, validation details, and raw activity stages.
- Source archive remains available from the management surface, outside the review viewport.

## Live deployed receipt

Production deploy `27388646397` shipped master commit `7f25b19` to
`https://memory-engine-api.fly.dev` on 2026-06-12. The deployed health check
returned `{"status":"ok","service":"memory-engine-api"}`.

Live phone critique:

- Hierarchy: the first viewport opens on the empty capture loop, with the
  capture form before sign-in and no account/source/review pipeline chrome.
- Type: the mobile-emulated Lighthouse screenshot wraps the headline and keeps
  form labels, placeholders, and button text readable at 390 x 844.
- Spacing and density: the form is sparse enough for first contact, and the
  sign-in surface is visually secondary below the capture action.
- Vocabulary: deployed home HTML contains "Add something you want to learn",
  "Paste anything worth remembering.", and "Create review"; it does not contain
  NATO demo text, account ids, draft/attempt/validation language, or raw
  activity-stage values.

Lighthouse mobile run:

```sh
npx -y lighthouse@latest https://memory-engine-api.fly.dev/ \
  --quiet \
  --chrome-flags="--headless=new --no-sandbox" \
  --form-factor=mobile \
  --screenEmulation.mobile=true \
  --screenEmulation.width=390 \
  --screenEmulation.height=844 \
  --screenEmulation.deviceScaleFactor=2 \
  --only-categories=performance,accessibility \
  --output=json \
  --output-path=.tmp/048-live-proof/lighthouse-home.json
```

Result: performance 100, accessibility 100, first contentful paint 0.8 s,
largest contentful paint 0.8 s, total blocking time 0 ms, cumulative layout
shift 0.

Production QA seed purge:

- Identified the old deployment seed source in production account
  `acct_48e443e2719d6f90`: `src_5fc8aff3662135d7`, title
  "Antikythera mechanism".
- Called the deployed source archive API:
  `DELETE /accounts/acct_48e443e2719d6f90/sources/src_5fc8aff3662135d7`;
  response `204`.
- Verified the deployed source list reports `sourceCount: 0` and
  `hasAntikythera: false`.
- Verified Neon state has `archivedSourceCount: 1`, `relatedDrafts: 3`,
  `relatedReviewUnitRows: 0`, and `activeRelatedReviewUnitRows: 0`.
