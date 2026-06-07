# Evidence Index: Production Mobile Study App

Branch: `cx/production-mobile-study-app`
Backlog: `040-production-mobile-study-app-boundary`
Date: 2026-06-06

## Primary Proof

- `demo.md` records the end-to-end behavior proof for the deployed mobile study
  app boundary.
- `receipt.json` is the machine-readable delivery receipt for this packet.
- `delivery-brief.md` is the human merge-ready brief.
- `learning-packet.md` records the bounded reflection and follow-up candidates.

## Acceptance Sources

- `backlog.d/040-production-mobile-study-app-boundary.md`
  - sha256: `bd60ab2d623c56fcb9122dab27ad590ee5bb243aa629f22999b4c9be8b1fa92d`
- `docs/architecture/production-mobile-study-app.md`
  - sha256: `c691896c9319ecaca8e6aba840c119501d78bb8d050481224c7cc816ada70909`
- `docs/architecture/adr-001-production-shell-boundary.md`
  - sha256: `8439b0c8c51af7e4b049f264b074addd1c2cdd12bf1cd79822eefa49c07da807`
- `docs/qa/fly-staging.md`
  - sha256: `5b74e2270fe463a0dd280290d7af8828d850be38b50a6d0f4fd70f481105ca89`

## Runtime Evidence

- Fly URL: `https://memory-engine-api.fly.dev/`
- Fly app: `memory-engine-api`
- Fly image: `memory-engine-api:deployment-01KTF8VG6WT38CZDSKE9QKJN95`
- Fly Managed Postgres cluster: `memory-engine-api-pg` (`nlkxjo56lnlry93v`)
- Machines observed after deploy:
  - `080395df316758` in `ord`
  - `84e474a4266518` in `ord`

## Verification Commands

- `cargo fmt --all --check`
- `MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test cargo test -p memory-engine-api postgres_backend_routes_drive_source_to_review -- --nocapture`
- `cargo test -p memory-engine-persistence-postgres migration_uses_account_scoped_primary_keys_and_durable_receipts -- --nocapture`
- `cargo test -p memory-engine-api -p memory-engine-persistence-postgres`
- `cargo clippy -p memory-engine-api -p memory-engine-persistence-postgres --all-targets -- -D warnings`
- `cargo run -p memory-engine-qa -- --local`
- `flyctl deploy -a memory-engine-api --remote-only`
- `curl -fsS https://memory-engine-api.fly.dev/healthz`
- `flyctl machine restart 84e474a4266518 -a memory-engine-api`
- `flyctl machine restart 080395df316758 -a memory-engine-api`
- deployed JSON route smoke with duplicate submit after API state recreation
  using the original session token
- `bun run ci`

## Secret Hygiene

Gitleaks passed inside `bun run ci`.

The additional repository scan:

```sh
rg -n "postgresql://|postgres://.*@|MEMORY_ENGINE_POSTGRES_URL=.*://|Ww7TOCfv|pgbouncer" -S . --glob '!target/**' --glob '!Cargo.lock'
```

returned only local test fixture URLs and a redacted placeholder command in
`docs/qa/fly-staging.md`; no deployed database credential is recorded in this
packet.
