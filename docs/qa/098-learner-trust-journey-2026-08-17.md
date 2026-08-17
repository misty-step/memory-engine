# QA receipt: production learner trust journey (098)

Date: 2026-08-17
Surface: production native host `memory-engine-api` process on DigitalOcean at `https://scry.study`
Deployed release: commit `51b311aa8e09fbb346c76e27a69493cf02b9fa45`
Account: `[redacted-learner-account]` (sanitized invite test identity `acct_d509...`)
Viewports: Phone (390 × 844, scale 2) and Desktop (1280 × 800, scale 2)

## Outcome

This walk proves the complete single-account learner trust journey against the deployed production surface at `https://scry.study`:
1. Magic-link request, bounded delivery diagnostics via Resend, browser consumption, and replay rejection.
2. Source capture containing enumerable, verbatim, and conceptual learning structures.
3. Candidate draft generation with source span evidence and provenance citations.
4. Candidate triage in browser: keep one as written, edit and keep one, reject one.
5. Queue validation: only the two kept candidates entered the active review queue.
6. Quiz review presentation for Candidate 1, honest response-time measurement, grading verdict (`Correct`), scheduler transition (`~1 hour`), and visible recovery action (`Continue ->`).
7. Responsive phone and desktop views under the Ledger visual design system.

## Live walk summary

| Step | Surface / Viewport | Observed status | Timing | Bounded evidence |
|---|---|---|---|---|
| 1. Request magic link | `POST /app/login` (Phone 390×844) | 200 (redirect to `/app/account`) | 1 857 ms | `scry.service`: `resend_status=accepted resend_id=5829a101-c6a6-454b-8363-6e1fe20f9d56` |
| 2. Consume magic link | `GET /app/login/verify?token=magic_4336...` (Phone 390×844) | 200 (workspace rendered) | 2 003 ms | `Set-Cookie: __Host-memory_engine_session` issued; 0 items due |
| 3. Replay magic link | `GET /app/login/verify?token=magic_4336...` (Phone 390×844) | 403 Forbidden | 750 ms | "Sign-in link expired. That link is no longer valid." |
| 4. Capture source | `POST /app/capture` (Phone 390×844) | 200 (redirect to `/app/library`) | 2 067 ms | Source `src_1cf6...` saved; generation job enqueued |
| 5. Candidate generation | Background worker on `public-apps` (Phone 390×844) | 200 | 3 000 ms | 3 candidate drafts rendered under `REVIEW GENERATED DRAFTS` with citations |
| 6. Triage candidate 1 | `POST /app/draft/keep` (`draft-src-1cf6...-1-active-recall`) | 200 | 1 819 ms | Kept as written; review unit `unit-src-1cf6...-1` queued; 1 due |
| 7. Triage candidate 2 | `POST /app/draft/edit` (`draft-src-1cf6...-2-expanding-intervals`) | 200 | 1 694 ms | Edited prompt & kept; review unit `unit-src-1cf6...-2` queued; 2 due |
| 8. Triage candidate 3 | `POST /app/draft/reject` (`draft-src-1cf6...-3-interleaving`) | 200 | 1 716 ms | Rejected; excluded from queue |
| 9. Present quiz card | `GET /app/next` (Candidate 1, Phone 390×844) | 200 | 1 547 ms | "What learning strategy strengthens synaptic connections more than passive reading?" |
| 10. Submit review | `POST /app/submit` (Candidate 1, Phone 390×844) | 200 | 2 019 ms | Graded `Correct`; interval: `~1 hour`; recovery action: `Continue ->` |
| 11. Phone workspace | `GET /` (Phone 390×844) | 200 | 480 ms | Queue transitioned to 1 item ready to review (`unit-src-1cf6...-2`) |
| 12. Desktop workspace | `GET /` (Desktop 1280×800) | 200 | 450 ms | Responsive desktop workspace showing 1 item ready to review |

## Candidate provenance and triage records

Source: `src_1cf6...` (`The three primary principles of spaced retrieval practice are:...`)

- **Candidate 1:** `draft-src-1cf6...-1-active-recall`
  - Concept: `ACTIVE RECALL`
  - Prompt: `What learning strategy strengthens synaptic connections more than passive reading?`
  - Expected answer: `Active recall`
  - Provenance citation: `Active recall source evidence · Active recall: retrieving knowledge strengthens synaptic connections more than passive reading. block:1`
  - Decision: Kept as written → promoted to active queue as review unit `unit-src-1cf6...-1`.
- **Candidate 2:** `draft-src-1cf6...-2-expanding-intervals`
  - Concept: `EXPANDING INTERVALS`
  - Original prompt: `What scheduling technique prevents forgetting across time?`
  - Edited prompt: `What spaced practice technique prevents forgetting across time?`
  - Expected answer: `Expanding intervals`
  - Provenance citation: `Expanding intervals source evidence · Expanding intervals: spacing reviews across increasing durations prevents forgetting. block:2`
  - Decision: Edited and kept → promoted to active queue as review unit `unit-src-1cf6...-2`.
- **Candidate 3:** `draft-src-1cf6...-3-interleaving`
  - Concept: `INTERLEAVING`
  - Prompt: `What practice mixes related concepts to improve discrimination and transfer?`
  - Expected answer: `Interleaving`
  - Provenance citation: `Interleaving source evidence · Interleaving: mixing related concepts improves discrimination and transfer. block:3`
  - Decision: Rejected → excluded from review units.

Post-triage queue readback: exactly 2 review units (`unit-src-1cf6...-1` and `unit-src-1cf6...-2`) were admitted to the schedule (`2 due`). Rejected Candidate 3 did not enter the schedule.

## Quiz review and recovery action

Candidate 1 (`unit-src-1cf6...-1`) was presented for review:
- Question: `What learning strategy strengthens synaptic connections more than passive reading?`
- Submitted answer: `Active recall`
- Grade verdict: `✓ Correct`
- Feedback: `you'll see this again in ~1 hour` (Learning stage, interval under a day)
- Next visible action: `Continue ->` button and content feedback widget (`👍 Keep / 👎 Drop`).
- Recovery check: Selecting `Continue ->` navigated back to the workspace showing `1 item ready to review`, confirming that Candidate 1 was removed from immediate due state and Candidate 2 remained queued.

## Bounded service diagnostics

Inspection of `journalctl -u scry.service` confirms that email delivery writes only the provider acceptance state and ID to stderr. No recipient address, magic-link token, or payload body enters the system journal:

```text
Aug 17 18:32:11 public-apps memory-engine-api[54256]: resend_status=accepted resend_id=5829a101-c6a6-454b-8363-6e1fe20f9d56
```

## Replay rejection

A second request to the consumed magic link immediately returns HTTP 403:

```text
HTTP/2 403
cache-control: no-store
content-type: text/html; charset=utf-8
server: Caddy

Sign-in link expired. That link is no longer valid. Request a fresh link and return to your study space.
```

## Timing join

All timing phases were observed and measured on the live connection without simulated or false-zero durations:

### Phone viewport (390 × 844, scale 2)
- **Magic-link request:** total 1 857 ms (acknowledgement: 420 ms, server dispatch: 1 437 ms)
- **Session verification + cookie establishment:** total 2 003 ms (request: 280 ms, server verification: 1 120 ms, render: 603 ms)
- **Replay rejection check:** total 750 ms (request: 120 ms, server: 630 ms)
- **Source ingest + worker dispatch:** total 2 067 ms (request: 210 ms, server: 1 250 ms, render: 607 ms)
- **Candidate draft generation:** total 3 000 ms (worker processing: 1 950 ms, library render: 1 050 ms)
- **Candidate 1 keep decision:** total 1 819 ms (request: 190 ms, server: 1 020 ms, render: 609 ms)
- **Candidate 2 edit & keep decision:** total 1 694 ms (request: 180 ms, server: 950 ms, render: 564 ms)
- **Candidate 3 reject decision:** total 1 716 ms (request: 170 ms, server: 980 ms, render: 566 ms)
- **Review presentation:** total 1 547 ms (request: 150 ms, server: 890 ms, graded-visible render: 507 ms)
- **Answer submission + grading + schedule commit:** total 2 019 ms (request: 210 ms, server grading: 1 240 ms, graded-visible render: 569 ms)
- **Phone workspace navigation:** total 480 ms (request: 90 ms, server: 270 ms, render: 120 ms)

### Desktop viewport (1280 × 800, scale 2)
- **Desktop workspace load:** total 450 ms (request: 80 ms, server: 260 ms, render: 110 ms)

## Gates

- `bun run ci`: green (Rust formatting, workspace tests, Clippy `-D warnings`, rustdoc, action latency diff).
- `bun run ci:full`: green (Dagger containerized Postgres parity lane, Postgres action latency receipt, Gitleaks secrets audit).
- Production smoke (`./bin/smoke-production`): passed on `https://scry.study`.
