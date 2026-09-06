---
name: powder
description: >
  Use Powder for Scry work selection, private claims, operator questions, and
  completion evidence. Keep drafts SPEC-LESS and labels non-authoritative.
---

# Powder

Powder is the exclusive-work ledger. Its CLI talks to one configured HTTP
origin: `powder use <url>` writes `~/.config/powder/config`, or set
`POWDER_URL`; there is no default or local-ledger fallback. Remote origins use
HTTPS; HTTP is loopback-only. `--agent` wins over `POWDER_AGENT` and config
metadata. `POWDER_API_KEY` authenticates transport; the managed label
`forest-misty-step/powder` is metadata, not authority.

A successful `take` returns a flat Job and a per-job random `claim_token`; the
server stores only its SHA-256 hash in `jobs.lease_token_hash`. The CLI stores
claims privately by validated origin and job id, resumes by job id, and never
prints claims. `held` means the presented claim does not match or is absent;
an audit label never grants resume. `release`, `renew`, `ask`, `done`, live-job
edits, and live `abandon` require the matching claim (`claim_required`,
`invalid_claim`). `note` is report-authorized and claim-independent. Claims are
deleted after release, ask, done, or abandon. `doctor` reports origin, key
presence, and health without exposing key material or claims.

## Subject flow

1. Read `forest.yaml`; a present `scope.subjects` list is the complete
   allowlist. A managed poll without `POWDER_AGENT` does not query Powder's
   `--mine` view; use an explicitly configured GitHub Subject instead.
2. With an audit label, run `powder list --mine <agent> --repo <repo>`, then
   `powder list --takeable --repo <repo>`. A takeable job is non-terminal,
   non-waiting, has a non-empty spec, no live lease, and only terminal direct
   blockers.
3. `powder show <id>` supplies the spec; `powder take <id>` claims it before a
   branch is created. Resume a live job only with its saved claim. Distinct
   jobs may be held under one label.
4. Publish `forest.review-request.v2` with `tracker: "powder"` for Powder or
   `tracker: "github"` for GitHub. Builders do not call `powder done`; after an
   approved Forest Gate, Scry's Verifier completes a Powder Subject with
   `powder done <id> --proof <revision>`. Use `powder ask` to park a claimed
   job for an operator answer.

Useful commands:

```sh
powder doctor
powder list --takeable --repo REPO
powder list --mine AGENT --repo REPO
powder show ID
powder take ID
powder release ID
powder ask ID --question '...'
powder done ID --proof PROOF
```

JSON is the default output; `list --plain` and `show --plain` print text.
Errors are JSON on stderr with a `code`. Repository-scoped API capabilities
control mutation; report/promote authority and claim authority remain separate.
