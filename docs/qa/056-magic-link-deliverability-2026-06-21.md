# QA: magic-link deliverability (ticket 056)

> Historical receipt from 2026-06-21. Provider-specific configuration and the
> open remainder below were superseded by the DigitalOcean cutover; use
> `docs/runbook.md` for current secret and deployment operations.

## What shipped (agent-doable increment)

- `bin/send-magic-link` reads `MEMORY_ENGINE_MAIL_FROM` (default
  `Memory Engine <onboarding@resend.dev>`), so switching to a verified-domain
  sender is a fly-secret change, not a code edit or redeploy of the script.
- `docs/runbook.md` "Deliverability" section: the full API-driven Resend
  domain-verification procedure, the `dig` SPF/DKIM/DMARC checks, the
  sender-switch command, and the "didn't get the email" diagnostic.
- Email content reviewed against spam heuristics — already compliant (real
  from-name, plain-text body, full sign-in URL with no shortener, plain
  subject); left unchanged.

## QA — live script against a mock Resend (no real email)

Ran `bin/send-magic-link` with `RESEND_API_URL` pointed at a local mock that
captures the request body (2026-06-21):

| run | `MEMORY_ENGINE_MAIL_FROM` | captured `from` | result |
| --- | --- | --- | --- |
| default | (unset) | `Memory Engine <onboarding@resend.dev>` | backward-compatible ✓ |
| override | `Memory Engine <noreply@mail.example.com>` | `Memory Engine <noreply@mail.example.com>` | config flip works ✓ |

Both: `to` = the allowlist address, subject = "Your Memory Engine sign-in link",
the full `https://…/app/login/verify?token=…` URL present in the plain-text body.
The script exits 0 on a 200 from Resend.

## Historical operator-gated remainder (superseded)

These need decisions/access only the operator has; exact commands are in the
runbook's Deliverability section:

1. **Choose a domain** to send from (no custom domain is configured on the Fly
   app today) and register it in Resend (`POST /domains`).
2. **Add the returned DNS records** (SPF/DKIM/DMARC) at the domain's host.
3. **Switch the sender**: `flyctl secrets set … MEMORY_ENGINE_MAIL_FROM=…`.
4. **Confirm inbox placement**: a fresh production magic link lands in the Gmail
   inbox, not spam.

Verifying a sending domain + writing DNS is persistent external config on the
operator's domain reputation, so it is confirm-first rather than autonomous —
and the Gmail inbox check needs the operator's eyes.
