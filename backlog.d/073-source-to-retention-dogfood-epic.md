# Prove the daily source-to-retention loop over 30 days

Priority: P0 · Status: pending · Estimate: XL

## Goal

Prove that one learner can use production daily to turn three materially
different kinds of source into trustworthy study material, recover from misses,
and retain it later without hand-designing an Anki deck.

## Oracle

- [ ] Browser answers carry honest response time and scheduler ratings; 074 is
      shipped before longitudinal evidence starts.
- [ ] Production login reaches the inbox reliably; 056 is completed or the run
      records an explicit solo-dogfood waiver with the residual onboarding risk.
- [ ] Three real learner-owned goals enter the loop: an enumerable set, a
      verbatim sequence, and conceptual material. 061's green content-fit
      receipt is the generation prerequisite.
- [ ] The learner can inspect, edit, reject, or explain rejection of generated
      material before bad cards silently shape the schedule; every verdict
      resolves to its generation run/config and exports to the eval corpus.
- [ ] At least one real miss exercises reference or bridge remediation and is
      later re-tested as cold recall; the evidence distinguishes first exposure
      from retrieval after spacing.
- [ ] A predeclared 30-day protocol runs on the production DigitalOcean app and
      publishes attempt-level evidence: completed days, median session time,
      workload, accepted/rejected material, cold attempts, recall rate with an
      interval, and qualitative failure log. The report says “underpowered”
      rather than inventing a learning-effect claim the sample cannot support.
- [ ] Every observed content or interaction failure becomes a deterministic
      fixture, route test, or explicitly-shaped follow-up before the epic closes.

## Verification System

- Claim: the product creates a sustainable, trustworthy learning habit rather
  than merely completing a card-management workflow.
- Falsifier: fabricated effort data, unsuitable auto-approved cards, an
  abandoned daily loop, no cold attempts, or a miss that cannot produce useful
  remediation.
- Driver: one operator-run production session per day for 30 days across the
  three goal types, with the phone-sized web loop as primary and the shipped
  CLI as a receipt cross-check.
- Grader: predeclared protocol plus operator material-quality verdicts and
  attempt/schedule state; no model grades its own product outcome.
- Evidence packet: `docs/dogfood/073-source-to-retention-<start>-<end>.md`,
  exported NDJSON/JSON summaries with secrets removed, screenshots of the
  critical loop, and linked regression receipts.
- Cadence: daily capture; weekly failure review; final paired narrative and
  quantitative read after day 30.

## Non-Goals

- Public launch, billing, broad multi-user onboarding, or a generic tutor chat.
- Replatforming production as part of the learning experiment.
- A cosmetic redesign that does not change or prove the learner outcome.

## Children

1. 074: honest browser response timing and scheduler inputs.
2. 056: inbox-safe production login or explicit solo-dogfood waiver.
3. 061: green enumerable/verbatim/conceptual generation.
4. Learner material-quality review + provenance/export, absorbing the
   learner-facing outcome of 062 rather than duplicating its storage spine.
5. Phone-sized authenticated capture → inspect → review → miss → remediate QA.
6. Predeclare and execute the 30-day protocol; feed every failure back into
   deterministic proof.

## Notes

This is the proposed successor to stale UX-audit epic 066. It also proposes
consolidating the user-facing outcome of 056 and 062, but those tickets remain
open until the operator ratifies consolidation; this groom does not silently
delete them.
