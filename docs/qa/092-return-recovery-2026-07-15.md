# QA receipt: learner return and recovery (092)

Date: 2026-07-15
Surface: local `memory-engine-api` file-store host at `http://127.0.0.1:18092`
Viewport: Chrome headless, 390 × 844, light mode

## Live checks

- `GET /healthz` returned `{"status":"ok","service":"memory-engine-api"}`.
- `GET /manifest.webmanifest` returned `200 application/manifest+json`; `jq`
  verified `display=standalone`, `start_url=/`, and two declared icon sizes.
- `GET /favicon.png` and `GET /apple-touch-icon.png` returned `200 image/png`
  with truthful 192×192 and 180×180 PNG dimensions.
- `GET /` contained manifest, favicon, apple-touch-icon, theme-color, and the
  opt-in “Enable due-count reminders” action.
- A real local login request wrote an auth outbox record; following its link
  rendered the signed-in workspace without exposing session credentials.
- Posting the opt-in form wrote a `due-count` outbox receipt. The signed email
  GET rendered a confirmation without changing persisted state; its token POST
  disabled the channel and rendered “Reminders are off”; no second mail record
  was written.

## Retained phone proof

[390px first-contact screenshot](092-phone-first-contact-2026-07-15.png)

The screenshot covers the auth-first entry point, responsive Ledger shell,
email action, and no-JavaScript-compatible form path. The existing route tests
cover expired magic-link and expired browser-session recovery with their
original 403/401 status and `text/html` content type.

## Not covered here

This is a local receipt, not a production deployment. Production DNS/SPF/DKIM/
DMARC verification and first-contact inbox placement remain the explicit human
gate owned by card `memory-engine-056`; no claim is made for those criteria.

A read-only probe of the current production deployment confirmed `/healthz` and
`/` at 200, while the new manifest and icon paths are still 404. That is
expected before this branch is merged and deployed; it is recorded so the
production phone receipt is not misrepresented as complete.
