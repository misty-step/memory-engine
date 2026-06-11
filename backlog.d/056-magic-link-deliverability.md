# Magic links land in the inbox, not spam

Priority: P2 · Status: pending · Estimate: S

## Goal

Sign-in emails arrive in the inbox. The first real login (2026-06-11) found
the magic link in Gmail's spam folder — a hard onboarding wall for anyone
who doesn't think to check.

## Oracle

- [ ] A domain the operator controls is verified in Resend (CLI/API-driven
      per the agent-friendly doctrine; DNS records documented in the
      runbook), and `bin/send-magic-link` sends from that domain instead of
      `onboarding@resend.dev`.
- [ ] SPF, DKIM, and DMARC records validate (checked with a real tool, e.g.
      `dig` + a deliverability checker; results recorded in the runbook or
      a docs/qa receipt).
- [ ] A fresh production magic link to the allowlist address lands in the
      Gmail inbox (not spam) — operator-confirmed, recorded in the receipt.
- [ ] Email content reviewed against spam heuristics: real from-name,
      plain-text body kept, no link-shortener, reasonable subject. An HTML
      part is optional; add only if it doesn't hurt deliverability.
- [ ] Resend webhook or send-log check documented in the runbook so future
      "didn't get the email" reports have a first diagnostic step.

## Notes

`onboarding@resend.dev` is a shared sender with no domain reputation —
spam-foldering is expected, not a bug in our script. Verifying a domain also
lifts the only-deliver-to-account-owner restriction, which matters the
moment a second allowlist address exists. Resend domain verification is
fully API-driven (`POST /domains`, returns DNS records; verify endpoint),
so this stays inside the no-dashboards doctrine — DNS record creation
depends on where the chosen domain is hosted (document the registrar step
if manual). Update the runbook's Login section and the `from` line in
`bin/send-magic-link` together (the script comment already anticipates
this).

## Children

1. Choose + verify domain in Resend via API; DNS records placed and
   validated.
2. Switch `from`, review content, redeploy, inbox-placement receipt.
3. Runbook: deliverability diagnostics section.
