# QA receipt — persistent dogfood deployment

Date: 2026-06-11. Legacy work item: `memory-engine-044` (partial; see card for
remaining scope). Production: `https://memory-engine-api.fly.dev`.

## Pipeline (first real runs of the gated deploy)

- master push `edf6106` → ci success → gated deploy fired via `workflow_run`,
  deployed that SHA, post-deploy smoke passed.
- master push `b7dde29` (TLS fix) → same pipeline, green end to end.

## Live verification

1. **Allowlist live**: `POST /accounts` with a stranger email → 403
   `"This email is not allowed to register."`; owner email → 201 with
   account `acct_48e443e2719d6f90`.
2. **Neon over TLS (pooled endpoint)**: account creation, source creation,
   and generation all 2xx against `memory-engine-prod`
   (`twilight-brook-49749008`). Row counts after the walk: 1 account,
   1 source, 3 drafts, 1 generation run with usage/cost recorded.
3. **Model generation in production**: pasted Antikythera prose →
   `POST .../generate` → 200 with 3 drafts ("What was the primary function
   of the ancient Greek Antikythera mechanism?", …), zero notices.
4. **Canary closed the loop on a real failure**: the initial Neon cutover
   failed in production with `Postgres error: error performing TLS
   handshake`; that 500 was captured by Canary (`ApiFailure::internal`,
   severity error) before any human saw it. The TLS fix (rustls + webpki
   roots, channel binding disabled) was verified against a disposable Neon
   branch before deploying. No new Canary errors after the fix.

## Notes

- The JSON API authenticates with the `x-session-token` header (not
  `Authorization: Bearer`).
- The test fixture's pooled-endpoint failure (`options=search_path` startup
  parameter) is a pgbouncer limitation of the test harness only; the app
  sends no `options` and works over the pooled URL, as verified above.
- The previous Fly Managed Postgres cluster `memory-engine-api-pg` still
  exists and is unused; operator decision pending on decommission.
