---
id: 117
status: proof
priority: p0
type: bug
---

# Stay signed in on the phone

## Outcome

Opening the installed or mobile-Safari PWA resumes the existing study
session. A magic link is for a new device or an explicit sign-out, not a
daily ritual.

## Why now

Sessions already last 90 days in code. The phone still asked for a magic
link. Named cause: iOS standalone PWA is a cross-site context, so
`SameSite=Lax` dropped the `__Host-` cookie.

## Acceptance

- [x] Host cookie is `SameSite=None; Secure`. Local HTTP stays Lax.
- [x] CSRF still blocks forged POSTs. Logout clears the host cookie.
- [ ] After one successful magic-link consume, returning the next day and a
      week later opens Home/review without a new link.

## Dependencies

None.

## Proof

Merged as `4c944be` (#126). Deployed in host release `391fb55`.
Phone two-day walk remains.

## Non-goals

No passwords. No public signup.
