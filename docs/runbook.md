# Production runbook — memory-engine-api

Everything here is CLI-driven; no dashboard is required. Scry runs as the
native Rust `memory-engine-api` process on Misty Step's isolated DigitalOcean
public application host, backed by Postgres on that same host and served at
`https://scry.study`. DNS and TLS terminate at Caddy on the same host. Rollback
switches an immutable host release; it does not change the local store.

## Agent surface summary

- Service: `memory-engine-api`, systemd unit `scry.service`.
- Host: isolated public application Droplet, reached through its Tailscale SSH
  name; public TCP 22 is closed.
- Product URL and only production origin: `https://scry.study`.
- Runtime: native Rust binary from `crates/memory-engine-api`; immutable
  releases live under `/opt/public-apps/scry/releases/<git-commit>` and
  `/opt/public-apps/scry/current` selects the active release.
- Store contract: production must set `MEMORY_ENGINE_POSTGRES_URL`; file store
  requires `MEMORY_ENGINE_ENABLE_FILE_STORE=true` and is local/dev only.
- Human auth contract: invite allowlist plus magic links; production account creation and
  magic-link delivery require `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` plus either
  `MEMORY_ENGINE_AUTH_MAILER_COMMAND` or the temporary outbox path, and
  `MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET` for signed reminder links.
  Machine consumers use operator-gated service sessions
  (`MEMORY_ENGINE_ADMIN_TOKEN`, below) — never email.
- Return scheduler: the API process owns a bounded, Postgres-backed sweep;
  disable it with `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED=false`,
  and keep the operator-only `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN`
  out of source control.
- Smoke contract: every host release runs the service, health, readiness,
  home-page, and anonymous mutation boundary checks below before it is live.

## Deployed smoke

These commands exercise the sole production runtime. The retired provider
workflow must not be restored: it previously reactivated an obsolete runtime
after every green `master` push.

```sh
base="https://scry.study"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w "%{http_code}" "$base/healthz")
test "$status" = "200"
grep -q '"status":"ok"' /tmp/memory-engine-healthz

# Readiness is dependency-aware: it is only 200 when Postgres and the
# generation worker are available. `/healthz` above remains liveness-only.
status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-readyz -w "%{http_code}" "$base/readyz")
test "$status" = "200"
grep -q '"status":"ready"' /tmp/memory-engine-readyz

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-home -w "%{http_code}" "$base/")
test "$status" = "200"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w "%{http_code}" -X POST "$base/app/generate")
case "$status" in 4??) ;; *) echo "expected 4xx, got $status"; exit 1;; esac

curl -fsS --max-time 15 "$base/manifest.webmanifest" | jq -e \
  '.display == "standalone" and (.icons | length >= 2)' >/dev/null
curl -fsS --max-time 15 "$base/favicon.png" | file - | grep -q 'PNG image data, 192 x 192'
curl -fsS --max-time 15 "$base/apple-touch-icon.png" | file - | grep -q 'PNG image data, 180 x 180'
```

## Service sessions (machine consumers)

Agents, QA runs, and future service consumers authenticate without email.
`POST /v1/service-sessions` issues an independent, expiring account-scoped
session token for an allowlisted service account, gated by the operator admin token.
The surface is disabled entirely unless the runtime sets
`MEMORY_ENGINE_ADMIN_TOKEN`. Agents never read mail-provider archives; magic
links stay a human-only delivery channel.

Provisioning (operator, once):

1. Add the dedicated dogfood email to `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` in
   the root-owned `/etc/public-apps/scry.env` file. Use a dedicated address.
   Magic-link and service/browser logins create independent expiring sessions;
   logging out one browser profile does not revoke another, while logout-all
   is explicit.
2. Set `MEMORY_ENGINE_ADMIN_TOKEN` in the same mode-`0600` environment file.
   Never commit or print it.
3. Issue the credential and import it directly into the consuming client's
   mode-`0600` credential file:

```sh
base="https://scry.study"
curl -fsS --max-time 20 \
  -H 'content-type: application/json' \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -d '{"email":"<dogfood-email>"}' \
  "$base/v1/service-sessions"
# -> {"accountId":"acct_...","sessionToken":"sess_..."}
```

The response maps onto the receipt variables below:
`MEMORY_ENGINE_ACCOUNT_ID=accountId`,
`MEMORY_ENGINE_SESSION_TOKEN=sessionToken`.

Each issue mints a fresh independently auditable token; existing sessions stay
valid until expiry or explicit revocation. API clients can revoke one bearer
session or all bearer sessions through the account-scoped DELETE routes below;
browser logout has the same one/all semantics for cookie sessions. Issuance is
audited in the app log (`service session issued account=...`).

`DELETE /v1/accounts/{account_id}/service-sessions/current` revokes only the
presented bearer token. `DELETE /v1/accounts/{account_id}/service-sessions/all`
revokes every bearer token for that account. Both routes require the raw bearer
credential, never a persisted SHA-256 digest.

## Waitlist (invite-beta first-run)

The signed-out landing page offers two actions: sign in (allowlisted emails,
existing flow) and join the waitlist (anyone, no account). `POST
/app/waitlist` records only a normalized email, a created/updated timestamp
pair, and a source tag (`"first-run"`) — no account, session, or generation
job is ever created. Joining is idempotent on normalized email and returns
the identical response whether the address is brand new, already on the
list, or already allowlisted, so the response can never be used to probe
registration or allowlist state. Rate limiting runs per normalized email and
per trusted edge-overwritten client identity. Caddy overwrites
`do-connecting-ip` with `{http.request.remote.host}` before proxying to the
loopback service. The API uses that value. Generic caller-controlled
`x-real-ip` and `x-forwarded-for` headers never influence a quota. If the edge
identity is missing, requests use the deterministic `unknown` bucket. The
active edge contract is `/etc/caddy/Caddyfile`.

Storage is dual-backend, dispatched the same way as every other
`memory-engine-api` store: `MEMORY_ENGINE_POSTGRES_URL` set → Postgres
(`memory_engine_waitlist_entries` plus an append-only
`memory_engine_waitlist_audit_log` recording every join/invite/delete
transition); unset → the local file store only
(`crates/memory-engine-api-state/src/waitlist.rs`, `_waitlist.json` beside
the other store-root sidecars under `MEMORY_ENGINE_API_STORE_DIR`, with its
own `_waitlist_audit.jsonl` mirroring the same audit contract for local
dev/tests without a database). Production always runs Postgres-backed; the
join and admin routes below no longer return `503` there.

Operator surface, gated by `MEMORY_ENGINE_ADMIN_TOKEN` (the same admin token
used by service sessions) — list, export, mark invited, and delete, with no
direct SQL required:

```sh
base="https://scry.study"

# List every entry as JSON.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  "$base/internal/waitlist"
# -> [{"email":"...","createdAtMs":...,"updatedAtMs":...,"source":"first-run","invitedAtMs":null}, ...]

# Export the same rows as CSV (email,createdAtMs,updatedAtMs,source,invitedAtMs).
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  "$base/internal/waitlist/export"

# Mark one address invited. Idempotent: inviting an already-invited address
# again returns its existing invitedAtMs unchanged. 404 if the address never
# joined.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"<address>"}' \
  "$base/internal/waitlist/invite"

# Delete one address. Removes only the operational row; the audit log keeps
# a permanent record that the address joined (and was invited, if it was).
# 404 if the address is not present.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"<address>"}' \
  "$base/internal/waitlist/delete"
```

All four routes sit beside `/internal/scheduler/return-notifications`,
outside the versioned `/v1/*` contract — operator tooling, not a public API
surface. Marking invited is durable admission state: every magic-link request
consults the persisted waitlist row, so a restart or another replica sees the
same decision. Deleting the operational row revokes admission and outstanding
unconsumed links; the append-only audit row remains for recovery evidence.
The configured `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` entries remain an explicit
operator allowlist, while invite-beta admission is persisted in the waitlist
adapter rather than copied into process memory.

## Queued generation (machine consumers)

Bearer-authenticated clients enqueue the same bounded, durable generation job
used by the browser app, then poll that account-scoped job until it reaches
`succeeded` or `failed`. No browser cookie, CSRF token, or synchronous model
request is involved. A duplicate POST while the same source is queued or
running returns `200` with the existing job id and `"coalesced": true`; a new
job returns `202`.
Admission-control rejections return `409`; a transient queue-store failure
returns `503` so machine clients can retry it.

```sh
set -euo pipefail

base="https://scry.study"
account_id="${MEMORY_ENGINE_ACCOUNT_ID:?set MEMORY_ENGINE_ACCOUNT_ID}"
session_token="${MEMORY_ENGINE_SESSION_TOKEN:?set MEMORY_ENGINE_SESSION_TOKEN}"
source_id="${MEMORY_ENGINE_SOURCE_ID:?set MEMORY_ENGINE_SOURCE_ID}"

job_response="$(curl -fsS --max-time 20 \
  -H "authorization: Bearer $session_token" \
  -X POST \
  "$base/v1/accounts/$account_id/sources/$source_id/generation-jobs")"
job_id="$(printf '%s' "$job_response" | jq -er '.id')"

while :; do
  job_response="$(curl -fsS --max-time 20 \
    -H "authorization: Bearer $session_token" \
    "$base/v1/accounts/$account_id/generation-jobs/$job_id")"
  case "$(printf '%s' "$job_response" | jq -r '.status')" in
    succeeded) break ;;
    failed) printf '%s\n' "$job_response" >&2; exit 1 ;;
    queued|running|retry) sleep 1 ;;
    *) printf 'unexpected job response: %s\n' "$job_response" >&2; exit 1 ;;
  esac
done
printf 'scheduled cards created by generation: %s\n' "$(printf '%s' "$job_response" | jq -r '.cardCount')"
```

A succeeded job reports the scheduled cards created by generation in
`cardCount`; this remains 0 while generated drafts await learner decisions.
Generation itself creates no review units or due schedules; a learner keep or
edit decision is the explicit scheduling gate. Inspect the study view for pending
drafts.

## Legacy v1 compatibility latency

Use this only when a ticket explicitly needs the synchronous compatibility
path. `/v1/accounts/{account_id}/sources/{source_id}/generate` remains a
legacy direct API surface and returns `409` on production Postgres hosts;
production consumers use the queued generation-job workflow above.
The command below uses an existing allowlisted v1 account/session, saves one article-sized source,
times the end-to-end direct request, records the response, and archives the
source. Production account creation is allowlist-protected, so do not use a
throwaway email here; export a pre-provisioned account id and session token
first.

```sh
set -euo pipefail

base="https://scry.study"
receipt_dir="docs/qa"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
account_id="${MEMORY_ENGINE_ACCOUNT_ID:?set MEMORY_ENGINE_ACCOUNT_ID}"
session_token="${MEMORY_ENGINE_SESSION_TOKEN:?set MEMORY_ENGINE_SESSION_TOKEN}"
source_id=""
cleanup_source() {
  if [ -n "$source_id" ]; then
    curl -fsS --max-time 20 \
      -H "authorization: Bearer $session_token" \
      -X DELETE \
      "$base/v1/accounts/$account_id/sources/$source_id" \
      >/dev/null || true
  fi
}
trap cleanup_source EXIT
body='Spaced practice improves long-term retention because each retrieval attempt forces the learner to reconstruct the memory rather than reread it passively. Feedback closes the loop by showing whether the reconstruction was accurate. When practice is delayed, the extra effort strengthens later access, but if the delay is too long the learner may fail without a useful cue. A good study system therefore mixes short recognition checks, cued recall, free recall, and applied composition so the learner moves from identifying an answer toward using the idea in context.'

source_json="$(jq -n --arg title "Latency receipt $stamp" --arg body "$body" \
  '{title:$title, body:$body}')"
source_response="$(curl -fsS --max-time 20 \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $session_token" \
  -d "$source_json" \
  "$base/v1/accounts/$account_id/sources")"
source_id="$(printf '%s' "$source_response" | jq -r '.sourceId')"

generate_status="$(curl -fsS --max-time 150 \
  -H "authorization: Bearer $session_token" \
  -o "$receipt_dir/production-generation-$stamp.json" \
  -w "status=%{http_code} time_total=%{time_total}\n" \
  -X POST \
  "$base/v1/accounts/$account_id/sources/$source_id/generate")"
case "$generate_status" in
  status=2??\ *) ;;
  *) echo "generation failed: $generate_status"; exit 1 ;;
esac
printf '%s\n' "$generate_status" \
  | tee "$receipt_dir/production-generation-$stamp.latency.txt"
```

## Auth/session migration and rollback

Postgres migration version 6 replaces the legacy account-scoped raw session
columns with per-session rows keyed by session_token_hash/session_id_hash,
adds expiry and revocation timestamps, and hashes legacy rows in one transaction.
The file adapter performs the equivalent one-time migration for legacy
session.token and browser-session rows on first read; raw values are not
retained after the rewrite.

Before applying migration 6, stop writes and capture a provider snapshot,
Neon branch, or `pg_dump` archive of the pre-migration database, and retain
the migration receipt:

```sh
export DATABASE_URL='postgres://...'
export PRE_V6_SNAPSHOT_FILE="/secure/backup/memory-engine-pre-auth-v6-$(date -u +%Y%m%dT%H%M%SZ).dump"
pg_dump --format=custom --no-owner --file="$PRE_V6_SNAPSHOT_FILE" "$DATABASE_URL"
```

Roll forward by running the API with the versioned Postgres migrator, then
verify row counts, non-empty 64-character hashes, expiry timestamps, and
absence of the legacy columns/tables. Run the migrator twice: the second run
must be a no-op and must leave the same row counts. Do not roll back only
the binary after migration 6: the hashed-session schema is not compatible
with a pre-migration binary.

For a deterministic Postgres rollback, stop API writes and restore the
snapshot captured *before* migration 6 — never a dump taken during rollback,
which would only capture the already-migrated state and restore a no-op:

```sh
export DATABASE_URL='postgres://...'
# The exact file captured in the pre-migration step above, not a fresh dump.
test -r "$PRE_V6_SNAPSHOT_FILE"
# Roll back only while writes are stopped.
pg_restore --clean --if-exists --no-owner --dbname="$DATABASE_URL" "$PRE_V6_SNAPSHOT_FILE"
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c   "SELECT version FROM memory_engine_schema_migrations ORDER BY version DESC LIMIT 1;"
```

Deploy the matching pre-v6 commit only after the restore reports the pre-v6
schema. For file stores, stop all API processes, archive the complete store
root before migration, and restore it atomically on rollback:

```sh
export STORE_ROOT='/var/lib/memory-engine/store'
export STORE_BACKUP="/secure/backup/memory-engine-store-pre-auth-v6-$(date -u +%Y%m%dT%H%M%SZ).tar"
tar --create --file="$STORE_BACKUP" --directory="$STORE_ROOT" .
# Roll back with writes stopped; restore into a sibling and swap atomically.
rm -rf "${STORE_ROOT}.restore"
mkdir "${STORE_ROOT}.restore"
tar --extract --file="$STORE_BACKUP" --directory="${STORE_ROOT}.restore"
mv "$STORE_ROOT" "${STORE_ROOT}.failed"
mv "${STORE_ROOT}.restore" "$STORE_ROOT"
```

A failed migration transaction leaves the legacy tables intact; retry is safe.
A file migration cleanup failure revokes the replacement hash and locks the
account until the operator restores the pre-migration store snapshot.

## Deploy and rollback

Production deployment is an explicit host release from a protected `master`
commit. A merge does not mutate production. Branch protection (`ci`, including
administrators) proves the source gate; the release receipt must name the
deployed commit and pass the real public smoke below.

Build and install a release from the Scry repository:

```sh
set -eu
test "$(git branch --show-current)" = master
test -z "$(git status --porcelain)"
git fetch --quiet origin master
release=$(git rev-parse HEAD)
test "$release" = "$(git rev-parse refs/remotes/origin/master)"
SCRY_SSH_HOST="${SCRY_SSH_HOST:-root@public-apps.tail5f5eb4.ts.net}"

RUST_MIN_STACK=33554432 CARGO_BUILD_JOBS=8 \
  cargo build --release --locked \
    -p memory-engine-api \
    -p memory-engine-canary --bin memory-engine-canary-receipt

stage=$(mktemp -d)
archive="/tmp/scry-${release}.tar.gz"
trap 'rm -rf "$stage" "$archive"' EXIT
install -D -m 0755 target/release/memory-engine-api \
  "$stage/memory-engine-api"
install -D -m 0755 target/release/memory-engine-canary-receipt \
  "$stage/memory-engine-canary-receipt"
install -D -m 0755 bin/send-magic-link "$stage/bin/send-magic-link"
tar --sort=name --mtime=@0 --owner=0 --group=0 -C "$stage" -czf "$archive" .
digest=$(sha256sum "$archive" | cut -d' ' -f1)

scp "$archive" "$SCRY_SSH_HOST:$archive"
ssh "$SCRY_SSH_HOST" sh -s -- "$release" "$digest" <<'REMOTE'
set -eu
release=$1
digest=$2
root=/opt/public-apps/scry
archive="/tmp/scry-${release}.tar.gz"
test -f /etc/public-apps/scry.env
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
install -d -o root -g root -m 0755 "$root/releases/$release"
tar -xzf "$archive" -C "$root/releases/$release"
chown -R root:root "$root/releases/$release"
ln -sfn "releases/$release" "$root/current.next"
mv -Tf "$root/current.next" "$root/current"
ln -sfn "$root/current/bin/send-magic-link" /usr/local/bin/send-magic-link
systemctl enable --now scry.service
systemctl restart scry.service
curl -fsS --max-time 20 http://127.0.0.1:3005/readyz
rm -f "$archive"
REMOTE
./bin/smoke-production
```

`/etc/systemd/system/scry.service` is the runtime contract:
`User=scry`, `EnvironmentFile=/etc/public-apps/scry.env`,
`ExecStart=/opt/public-apps/scry/current/memory-engine-api`, loopback
`HOST=127.0.0.1`, and `PORT=3005`. Caddy is the only public ingress and proxies
`scry.study` and `www.scry.study` to that port while overwriting
`do-connecting-ip`.

Rollback changes only the active symlink, then reruns the same smoke. It does
not modify the environment file or the local Postgres data directory:

```sh
known_good="${KNOWN_GOOD_COMMIT:?set an installed release commit}"
SCRY_SSH_HOST="${SCRY_SSH_HOST:-root@public-apps.tail5f5eb4.ts.net}"
ssh "$SCRY_SSH_HOST" sh -s -- "$known_good" <<'REMOTE'
set -eu
release=$1
root=/opt/public-apps/scry
test -x "$root/releases/$release/memory-engine-api"
ln -sfn "releases/$release" "$root/current.next"
mv -Tf "$root/current.next" "$root/current"
systemctl restart scry.service
curl -fsS --max-time 20 http://127.0.0.1:3005/readyz
REMOTE
./bin/smoke-production
```

After a rollback, revert or repair `master`, run the deterministic gate, and
deploy that reviewed commit normally. Never revive the deleted App Platform
application or any retired Fly path.

## Health

```sh
curl -fsS https://scry.study/healthz
curl -fsS https://scry.study/readyz
ssh "${SCRY_SSH_HOST:-root@public-apps.tail5f5eb4.ts.net}" \
  'systemctl is-active scry.service && readlink -f /opt/public-apps/scry/current'
```

Errors, health check-ins, and closed performance aggregates land in Canary
through one bounded worker queue. Every API 500 (`ApiFailure::internal`) is
reported as service `memory-engine-api`, environment `production`.
Performance observations are merged in process and export at most one batch
per minute for each of the trusted server, untrusted browser, and job
namespaces. Queue saturation, Canary failure, and shutdown deadlines remain
fail-open for request handling; delivery/drop/invalid counts ride in the
bounded aggregate schema. The same JSON is printed as
`authority=non_authoritative_debug` for immediate log inspection.

`POST /app/submit` is the only HTTP route with per-request performance
headers. Every response carries `X-Request-ID: req_<32 lowercase hex>` and a
content-free `Server-Timing` value with `request`, `total`, and `render`.
Browser-enhanced submits also carry the opaque `handoff` token. Postgres-backed
submits add `pgconnect`, `pgop`, and the cumulative `pgstmt` call count; those
metrics are omitted rather than reported as zero when the phase did not run.
The response is always `Cache-Control: no-store`. Health/readiness, static
assets, generation SSE, and the telemetry endpoint are intentionally outside
this instrumentation.

After a graded page is visible for two animation frames, the browser consumes
its short-lived same-tab handoff and posts one strict, content-free receipt to
`POST /app/performance/submit`. Canary receives the trusted server route total
separately from the untrusted browser tap-to-ack and graded-visible durations;
the browser series carries only a coarse `mobile`, `tablet`, or `desktop`
viewport. Missing APIs, stale or mismatched handoffs, BFCache restores, and
malformed timings fail closed without delaying or changing review submission.

The host release includes a bounded live receipt. Run it through the private
Tailscale SSH path without printing any secret value:

```sh
ssh "${SCRY_SSH_HOST:-root@public-apps.tail5f5eb4.ts.net}" \
  '/opt/public-apps/scry/current/memory-engine-canary-receipt emit-openapi'
```

The receipt times a real loopback `GET /v1/openapi.json`, queues one closed
server observation, drains the worker, and exits nonzero on HTTP or delivery
failure. Readback uses a distinct service-bound read credential; the
production app keeps ingest-only authority:

```sh
CANARY_READ_ENDPOINT=https://canary.mistystep.io \
CANARY_READ_API_KEY=... \
CANARY_READ_SERVICE=memory-engine-api \
cargo run -p memory-engine-canary --bin memory-engine-canary-receipt -- readback
```

Never reuse or promote the app's ingest key for readback. The deterministic
admission-overhead receipt is
`cargo run -p memory-engine-canary --bin memory-engine-canary-receipt -- overhead`;
it fails above a 5 ms p95.

## Runtime environment and secrets

The root-owned mode-`0600` `/etc/public-apps/scry.env` file owns the live
runtime configuration below. Edit it only through the private host, never print
secret values, then restart `scry.service` and rerun the deployed smoke.

| Secret | Purpose |
| --- | --- |
| `MEMORY_ENGINE_POSTGRES_URL` | Local Unix-socket URL `postgresql:///scry?host=/var/run/postgresql&sslmode=disable`. Peer-auth as systemd user `scry`. |
| `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` | Comma-separated allowlist; account creation and magic links refuse other emails. |
| `MEMORY_ENGINE_AUTH_MAILER_COMMAND` | Path to the installed production command `/usr/local/bin/send-magic-link`; do not replace it with the local outbox. |
| `RESEND_API_KEY` | Encrypted Resend credential consumed only by the bundled mailer; never print or place in a command line. |
| `MEMORY_ENGINE_MAIL_FROM` | Replyable production sender on the verified `mistystep.io` domain; keep the operator-approved sender and do not switch domains without a new ruling. |
| `MEMORY_ENGINE_PUBLIC_BASE_URL` | Public link base used to construct the sign-in URL; keep aligned with the deployed Scry host. |
| `MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET` | Stable secret for HMAC-signed, seven-day, account/email-scoped reminder unsubscribe links. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED` | Optional kill switch; set `false` to disable scheduled reminder sweeps. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN` | Operator-only token for bounded manual scheduler runs; keep it only in the mode-`0600` environment file. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE` | Optional bounded sweep size, default 100 and capped at 1000. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS` | Optional sweep interval, default 900 seconds and capped at 86400. |
| `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` | Local/dev-only magic-link outbox fallback; never use it as production delivery proof. |
| `OPENROUTER_API_KEY` | Enables model-backed generation for pasted prose; absent → structured-block parsing only. |
| `MEMORY_ENGINE_GENERATION_MODEL` | Optional model override (default `google/gemini-3.7-flash`; see docs/evals/). |
| `CANARY_ENDPOINT` / `CANARY_API_KEY` | Ingest-only Canary error, check-in, and bounded performance export; absent → reporting is a no-op. |
| `MEMORY_ENGINE_ENVIRONMENT` | Environment label on Canary error events. |
## Store backend selection

`main.rs` picks the backend at boot:

1. `MEMORY_ENGINE_POSTGRES_URL` set → Postgres (production).
2. Else `MEMORY_ENGINE_ENABLE_FILE_STORE=true` + `MEMORY_ENGINE_API_STORE_DIR`
   → file store (local/dev only; it is not durable production storage).
3. Else: refuses to boot.

Migrations run once per process at first database use, not per request.

## Database (local Postgres)

Postgres 16 runs on `public-apps`. The `scry` OS user owns database `scry`
via peer auth on `/var/run/postgresql`. `scry.service` waits for
`postgresql.service`.

Nightly custom-format dumps land in `/var/backups/scry` via
`/usr/local/sbin/scry-pg-dump` (`/etc/cron.d/scry-pg-dump`), pruned after
seven days. Restore drill: `pg_restore` into a side database owned by
`scry`, then drop it.

### Off-host backup

`scry-backup-offhost` (03:45 UTC timer) encrypts the newest dump with GPG
AES256 and uploads it to the Scry Spaces bucket (`scry/` prefix). Source
of truth lives in this repository: `bin/scry-backup-offhost`,
`etc/systemd/scry-backup-*`, and the idempotent
`bin/install-scry-backup.sh`. Never edit `/usr/local` copies directly;
reinstall from the repo.

**Provisioning status:** the bucket does not exist yet. The 30-day
age-based lifecycle and versioning described in Estate ADR 0011 are
mandatory properties of that provisioning step — they are NOT active
today. Because the uploader has no DELETE path, objects would accumulate
without bound if credentials were added before lifecycle/versioning
exist; provisioning order is therefore bucket+lifecycle first, credentials
second. Until then the timer runs and records
`skipped target-not-provisioned`.

The pipeline stays inert until `/etc/public-apps/scry-backup.env`
(mode 0600) defines `SCRY_BACKUP_SPACES_BUCKET`, `SCRY_BACKUP_SPACES_KEY`,
`SCRY_BACKUP_SPACES_SECRET`, `SCRY_BACKUP_PASSPHRASE`, and optional
`SCRY_BACKUP_REGION` (default `nyc3`). The passphrase must also live in
an operator-managed off-host credential store; without that copy the
ciphertext is unrestorable after droplet loss. Outcomes record to
`/var/lib/scry-backup/last-run`; failures fail the unit and trigger
`scry-backup-alert.service` (`FAILURE` flag + `daemon.alert` journal).
Design and custody: Estate ADR 0011.

The retired Neon project `memory-engine-prod` (`twilight-brook-49749008`) is
kept until a restoreable Neon dump exists. Do not delete it. Do not point
production back at Neon.

## Human login (magic links over email)

Human auth is invite allowlist + magic link, delivered by
`bin/send-magic-link`. Each release carries that script, and deployment keeps
`/usr/local/bin/send-magic-link` pointed at the active immutable release.
Production sends through Resend using `RESEND_API_KEY` from the root-owned
environment file and a replyable `MEMORY_ENGINE_MAIL_FROM` address on the
verified `mistystep.io` domain. The operator ruling is to keep that verified
domain and sender, without upgrading billing, adding `scry.study`, or deleting
`mistystep.io`. Keep
`MEMORY_ENGINE_AUTH_MAILER_COMMAND=/usr/local/bin/send-magic-link` in
`/etc/public-apps/scry.env`; never put a secret value in a shell command or
checked-in file.

The bundled sender has two environment contracts. Magic-link mode uses
`MEMORY_ENGINE_AUTH_EMAIL` and `MEMORY_ENGINE_AUTH_LINK` and does not require
an idempotency key. Due-count reminder mode fails closed unless
`MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY` is present; it sends that
same value as Resend's `Idempotency-Key` HTTP header. Retries of one durable
reminder claim must reuse both the key and the original payload so Resend can
deduplicate the `POST /emails` request. The key is not shared with magic-link
mail.

While `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` is set instead, links land in an
instance-local outbox. That path is a temporary solo-dogfood fallback, not a
durable delivery channel.

A failed send surfaces as a 500 and therefore lands in Canary. The production
sender is the encrypted `MEMORY_ENGINE_MAIL_FROM` value on verified
`mistystep.io`; the script fallback `onboarding@resend.dev` is local-only and must
not be used as production proof. Switching the sender is a secret change, not a
code edit — see Deliverability below.

### Due-count return channel

The signed-in workspace offers one explicit, optional return channel: “Enable
due-count reminders.” It stores the normalized, allowlisted reminder address in
the account store and sends one confirmation through the same
`MEMORY_ENGINE_AUTH_MAILER_COMMAND` / `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH`
boundary. The scheduled boundary enumerates enabled preferences without a
browser request, computes each account's live due count, and sends only when
reviews are due and the persisted last-send time is at least 24 hours old. The
policy is deterministic, has no streaks or promotional content, and a learner
can disable it from the workspace or the signed unsubscribe link in the
plain-text message. The email GET only renders a confirmation; its POST carries
the scoped token and performs the mutation without requiring a browser session.
Disable is persisted and never sends mail. Home/render GETs are read-only and
never invoke this boundary.

### Scheduled execution and operations

Every API instance starts the scheduler unless
`MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED=false`. Each sweep is
bounded by `MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE` (default 100,
maximum 1000), runs at
`MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS` (default 900),
and uses the durable per-account claim as the multi-instance lease/fence. The
current implementation deliberately uses one synchronous worker per instance;
Postgres/file claims provide the cross-instance concurrency bound and provider
idempotency prevents duplicate logical sends.

The file adapter's per-account notification lock is a persistent path with an
OS descriptor lock. Every writer (save/disable preference, claim, complete,
release) acquires the lock with a blocking `flock`, so a contending writer
waits for the lock instead of skipping the account outright; after acquiring
it, the writer re-reads the account's current on-disk state and re-checks
eligibility (enabled, claim ownership, retry timing) before mutating, so a
recheck always sees the latest committed state rather than a stale in-memory
view. The lock path is never deleted as part of ownership release, so stale
paths are harmless, and a process crash releases the descriptor for the next
waiter. The same libc-backed helper protects the repository-owned file
outbox: it scans durable delivery keys while holding the descriptor lock and
does not append a duplicate after a lease-expiry reclaim.

The scheduler returns an owned lifecycle handle. The production binary joins
that handle during graceful shutdown, including any in-flight blocking
provider call, before exiting. Manual and interactive reminder routes run
storage and provider work on blocking workers; health requests remain
responsive while a provider is slow. `lastRunAtMs` records every sweep, while
`lastSuccessAtMs` advances only for a sweep with zero failed accounts.

The liveness counters are included in `/healthz` under
`returnNotificationScheduler`. A bounded manual/backfill run is available only
with the operator token:

```sh
curl -fsS -X POST \
  -H "x-scheduler-token: ${MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN:?set token}" \
  "$base/internal/scheduler/return-notifications"
```

Production proof is limited to scheduler health and authorization-failure
probes. It does not establish provider acceptance or inbox placement for
due-count reminders. Treat a manual production run as delivery evidence only
when it uses one allowlisted account with a genuinely due card and retains a
privacy-safe provider receipt or file-outbox `due-count` entry. Local runs,
page renders, and `/healthz` alone are not delivery proof.

A failed provider send releases the claim but preserves the complete delivery
envelope and applies bounded exponential retry backoff (one minute, doubling
to six hours). A crash after provider acceptance retries only after the lease
expires with the same idempotency key and payload; stale finalize is fenced by
claim id. To disable or roll back the trigger, set the scheduler flag false or
revert the application commit and use the native-host rollback procedure
below. Inspect `/healthz`, provider send logs, and Canary failures together
during an incident.

Each unsubscribe link is a seven-day HMAC token bound to the account, normalized
email, and a persisted unsubscribe nonce. The GET remains read-only; the POST
atomically compares the current nonce and rotates it while disabling the
preference, so a replayed or concurrent stale bearer cannot win against an
authenticated re-enable. The nonce column is an additive migration with an
empty default for existing Postgres rows, and legacy file JSON defaults the
same way. Legacy v1 links are intentionally rejected because they cannot carry
the nonce; the next authenticated enable or reminder delivery backfills the
nonce and issues only v2 links.

The command boundary receives these variables for a due-count message:
`MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL`,
`MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT`, and
`MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE`; it also receives
`MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY` so a retry can reuse the
same durable delivery identity. The bundled sender requires that key in
reminder mode and forwards it using Resend's supported `Idempotency-Key`
contract; a missing key is an error before any provider request. The bundled
sender supports both the magic-link and due-count envelopes. A file outbox line
beginning with `due-count` is a local proof receipt; production proof still
requires checking the provider send log and inbox placement.

### Deliverability (verified mistystep.io sender)

The operator elected to keep the existing verified `mistystep.io` Resend domain
and sender. Do not upgrade Resend billing, add `scry.study`, or delete
`mistystep.io` as part of this card. The public product/link host remains
`scry.study`; the resulting sender/link-domain mismatch is an explicit,
operator-approved residual risk, not a reason to silently change the sender.

Verify the active provider and DNS state without exposing credentials or message
content:

```sh
# Resend: use the deployed scoped Mint alias secret://memory-engine/resend-domain
# or an authenticated Resend operator surface. Never print the credential.
# The route allows only GET /domains and GET /domains/<id>; expected: mistystep.io
# remains status=verified.
dig +short TXT send.mistystep.io
dig +short TXT resend._domainkey.mistystep.io
dig +short TXT _dmarc.mistystep.io
```

The bundled mailer emits only one bounded diagnostic line to stderr/application
logs after a successful provider call:
`resend_status=accepted resend_id=<provider-id>`. Transport failures emit
`resend_status=transport_error`; non-2xx provider responses emit only
`resend_status=failed http_status=<status>`. No recipient, token, full link,
request body, or provider response body is logged. Control characters in recipient,
sender, subject, and reminder idempotency inputs fail closed before any provider
request; message paragraph breaks are encoded once as JSON newlines. Keep provider
IDs as bounded operator evidence and redact them in shared proof when not needed
for lookup.

For a delivery investigation, use the provider operator surface with the
redacted provider ID and classify the send as `accepted`, `delivered`,
`bounced`, `complained`, `delayed`, or `unknown`. Do not copy message
content or recipient addresses into logs. A provider-accepted event alone does
not prove Inbox placement.

### Production magic-link proof and rollback

1. Trigger one fresh sign-in request through the normal production Scry UI for
the already-approved invited Gmail account. Use a unique nonce in the test
request metadata only; never paste the nonce, recipient, token, or full link into
proof.
2. In Gmail, record only the classification (Inbox or Spam) and privacy-safe
source-header results for SPF, DKIM, and DMARC. Do not screenshot message body,
recipient, token, or the full link.
3. Open the link once in the production Scry UI and record successful sign-in.
Attempt the same link again and record rejection. Attempt the link from a second
account/session and record rejection; do not record either account identity.
4. If placement or authentication fails, restore the previous known-good
`MEMORY_ENGINE_MAIL_FROM` value on the verified `mistystep.io` domain,
deploy through DigitalOcean, and rerun the deployed smoke. Do not delete the
verified domain. The temporary outbox remains a local-only fallback and must not
be enabled as a production delivery claim.

Residual risk: Gmail placement can vary because the approved sender domain and
public link host differ. Re-run the bounded Inbox/Spam and source-header proof
after any sender, DNS, or provider reputation change.

`POST /app/account` is abuse-limited in the API boundary before any magic link
is sent. The fixed window is 5 attempts per 15 minutes per normalized email and
per trusted edge-overwritten `do-connecting-ip`; a missing edge identity is
grouped as `unknown`. Forwarding headers are ignored. Rejected requests return `429` with the generic
message "Too many sign-in attempts. Try again later."; they do not write an
outbox row or reveal whether an email is allowlisted.

The browser session is server-side. `POST /app/logout` requires the same CSRF
token as other app mutations, revokes the stored browser session, and clears
the `__Host-memory_engine_session` cookie with `Max-Age=0`. Reusing the old
cookie after logout must return `401`, including after a process restart.
