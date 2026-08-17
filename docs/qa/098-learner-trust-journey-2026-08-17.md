# QA receipt: production learner trust journey (098)

Date: 2026-08-17
Surface: production native host `memory-engine-api` process on DigitalOcean at `https://scry.study`
Deployed release: commit `51b311aa8e09fbb346c76e27a69493cf02b9fa45`
Account: `trust-journey-20260817@mistystep.io`
Viewports: Phone (390 × 844, scale 2) and Desktop (1280 × 800, scale 2)

## Outcome

This walk proves the complete single-account learner trust journey against the deployed production surface at `https://scry.study`:
1. Magic-link request, bounded delivery diagnostics via Resend, browser consumption, and replay rejection.
2. Source capture containing enumerable, verbatim, and conceptual learning structures.
3. Candidate draft generation with source span evidence and provenance citations.
4. Candidate triage in browser: keep one as written, edit and keep one, reject one.
5. Quiz review presentation for the kept candidate, honest response-time measurement, grading verdict (`Correct`), scheduler transition (`~1 hour`), and content feedback.
6. Responsive phone and desktop views under the Ledger visual design system.

## Live walk summary

| Step | Surface / Action | Observed status | Timing | Bounded evidence |
|---|---|---|---|---|
| 1. Request magic link | `POST /app/login` (`trust-journey-20260817@mistystep.io`) | 200 (redirect to `/app/account`) | 1 857 ms | `scry.service`: `resend_status=accepted resend_id=5829a101-c6a6-454b-8363-6e1fe20f9d56` |
| 2. Consume magic link | `GET /app/login/verify?token=magic_4336...` | 200 (workspace rendered) | 2 003 ms | `Set-Cookie: __Host-memory_engine_session` issued; 0 items due |
| 3. Replay magic link | `GET /app/login/verify?token=magic_4336...` | 403 Forbidden | 750 ms | "Sign-in link expired. That link is no longer valid." |
| 4. Capture source | `POST /app/capture` (structured retrieval practice) | 200 (redirect to `/app/library`) | 2 067 ms | Background generation job enqueued |
| 5. Candidate generation | Background worker on `public-apps` | 200 | 3 000 ms | 3 candidate drafts rendered under `REVIEW GENERATED DRAFTS` with citations |
| 6. Triage candidate 1 | `POST /app/draft/keep` (`Active recall`) | 200 | 1 819 ms | Kept as written; queue updated to 1 due |
| 7. Triage candidate 2 | `POST /app/draft/edit` (`Expanding intervals`) | 200 | 1 694 ms | Edited prompt & kept; queue updated to 2 due |
| 8. Triage candidate 3 | `POST /app/draft/reject` (`Interleaving`) | 200 | 1 716 ms | Rejected; excluded from queue |
| 9. Present quiz card | Presented card for Candidate 1 | 200 | — | "What learning strategy strengthens synaptic connections more than passive reading?" |
| 10. Submit review | `POST /app/submit` (`Active recall`) | 200 | 2 019 ms | Graded `Correct`; scheduler: `~1 hour`; queue: 2 due → 1 due |
| 11. Desktop view | `GET /` (1280 × 800) | 200 | 450 ms | Responsive desktop workspace showing 1 item ready to review |

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
- Magic-link dispatch + provider acceptance: 1 857 ms
- Session verification + cookie establishment: 2 003 ms
- Replay rejection check: 750 ms
- Source ingest + worker dispatch: 2 067 ms
- Candidate draft generation: 3 000 ms
- Triage decisions (keep, edit, reject): 1 819 ms, 1 694 ms, 1 716 ms
- Review presentation + render: immediate
- Answer submission + grading + schedule commit: 2 019 ms
- Desktop workspace load: 450 ms

## Gates

- `bun run ci`: green (Rust formatting, workspace tests, Clippy `-D warnings`, rustdoc, action latency diff).
- `bun run ci:full`: green (Dagger containerized Postgres parity lane, Postgres action latency receipt, Gitleaks secrets audit).
- Production smoke (`./bin/smoke-production`): passed on `https://scry.study`.
