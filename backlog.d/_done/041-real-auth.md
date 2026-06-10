# 041 - Real Auth

## Status

Ready

## PRD Summary

- User: the owner/learner using the deployed mobile study app from more than
  one browser or device.
- Problem: the shipped app has account-scoped persistence, but its credential is
  an opaque `session_token` carried in hidden form fields and `x-session-token`
  headers. That is an account/session prototype, not real authentication.
- Goal: replace hidden session tokens with owner-only passwordless email login
  and cookie-backed sessions that can safely resume study state on a new device.
- Why now: the production app is already live on Fly; leaving "save account
  email" as if it were login is misleading and unsafe for real use.
- UX enabled: a learner enters an allowed email, receives a sign-in link, opens
  the link on any device, and resumes their existing study set without seeing or
  carrying raw session credentials in forms.
- Deliverable type: working code plus deployed auth QA evidence.
- Success signal: production `/app/login` sends a real email login link for an
  allowed address, `/auth/verify` issues a secure HttpOnly session cookie, and
  the authenticated app renders existing sources/reviews without any
  `sessionToken` hidden inputs.

## Product Requirements

- P0: Passwordless email login proves control of an allowed email address before
  account state can be resumed on a new browser or device.
- P0: The first production policy is owner-only or allowlist-only auth via
  `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS`; no open public signup.
- P0: Authenticated browser state uses a server-side session id in a cookie with
  `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`, bounded `Max-Age`, and no
  `Domain`.
- P0: Server-rendered forms no longer include `accountId` or `sessionToken`
  hidden inputs. The server resolves the account from the session cookie.
- P0: Every mutating browser form validates CSRF proof that is not the auth
  credential.
- P0: Magic-link challenges are single-use, short-lived, stored hashed at rest,
  and consumed atomically.
- P0: Login request responses do not reveal whether an email is allowed,
  registered, or unknown.
- P0: Logout revokes the server-side session and clears the cookie.
- P0: Existing account-scoped study, generation, reveal, review, duplicate
  submit, and restart/resume behavior remain intact.
- P1: Preserve source-first study UX by allowing a short-lived guest session
  cookie and binding that guest account to a verified email after link
  verification.
- P1: Add a local/staging auth mailer stub that records the link in test output,
  but production must require a real mail provider.
- Non-goals: passwords, OAuth, passkeys/WebAuthn, MFA, billing, teams,
  organization accounts, public signup, native app auth, JWT/OAuth public API
  tokens, and provider-backed generation.

## Non-Goals

- Do not move auth, email, cookie, HTTP, SQL, randomness, clock, logging, or
  provider code into `crates/memory-engine-core`.
- Do not keep hidden `sessionToken` fields as the post-auth credential model.
- Do not implement password storage or reset flows in this slice.
- Do not implement Google/Apple/GitHub OAuth in this slice.
- Do not make the Fly app an open public registration surface.
- Do not claim production auth from local package tests alone.

## Constraints / Invariants

- `bun run ci` remains the canonical repo gate.
- `bun run qa` / `cargo run -p memory-engine-qa -- --full` remains the handoff
  QA sweep.
- Auth belongs in `crates/memory-engine-api`; credential/session persistence
  belongs in `crates/memory-engine-persistence-postgres`.
- `memory-engine-core` remains framework-free and persistence-free.
- Account/user identity scopes all mutable production state before invoking
  study/session/service operations.
- `ReviewUnitId` remains opaque; verdicts remain `correct`, `close`, `wrong`,
  and `revealed`.
- Reveal remains display-only and duplicate submit remains idempotent.
- Postgres migrations stay additive and idempotent.
- Production must not use the file store for account-backed state.

## Authority Order

tests > type system > code > docs > memory/lore

## Repo Anchors

- `crates/memory-engine-api/src/lib.rs` — current account/session routes,
  server-rendered forms, hidden credential plumbing, and API tests.
- `crates/memory-engine-api/src/main.rs` — production env selection and startup
  failure behavior.
- `crates/memory-engine-api/Cargo.toml` — HTTP/cookie/auth dependency surface.
- `crates/memory-engine-persistence-postgres/src/lib.rs` — account/session
  schema and Postgres account-scoped store.
- `crates/memory-engine-persistence-postgres/Cargo.toml` — Postgres adapter
  dependency surface.
- `crates/memory-engine-qa/src/main.rs` — QA lane ownership and the place to
  add an auth smoke lane if the proof becomes repeatable.
- `docs/architecture/adr-001-production-shell-boundary.md` — accepted boundary:
  auth/account scoping in the API shell, not the kernel.
- `docs/qa/fly-staging.md` — deployed smoke record and current explicit auth
  gap.
- `backlog.d/_done/040-production-mobile-study-app-boundary.md` — prior slice
  acceptance oracle to preserve.

## Lead Repo Read

- Source files read directly:
  - `crates/memory-engine-api/src/lib.rs` for `create_account`,
    `save_account`, `read_session_token`, `hidden_account_inputs`, form routes,
    and account/session tests.
  - `crates/memory-engine-persistence-postgres/src/lib.rs` for
    `memory_engine_accounts`, `memory_engine_api_sessions`, and account-scoped
    study rows.
  - `crates/memory-engine-qa/src/main.rs` for current local/full QA lanes.
- ADRs / invariants read directly:
  - `docs/architecture/adr-001-production-shell-boundary.md`
  - `docs/architecture/production-mobile-study-app.md`
  - root `AGENTS.md` from the prompt.
- Commands or artifacts inspected:
  - `git status --short --branch --untracked-files=all`
  - `rg`/`nl` reads over the auth, Postgres, docs, and QA files above.
- Subagent summaries used only for:
  - repo coverage, security critique, and oracle critique.

## Prior Art

- `docs/architecture/adr-001-production-shell-boundary.md` — auth is an API
  shell concern and production storage is Postgres.
- `docs/qa/fly-staging.md` — current deployed proof and explicit hidden-token
  gap.
- `exemplars.md` — ignore app-specific session builders; do not contaminate the
  learning kernel with app choreography.
- OWASP Authentication Cheat Sheet:
  `https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html`
  — user identifiers should not be predictable, and session management must use
  difficult-to-predict identifiers.
- OWASP Session Management Cheat Sheet:
  `https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html`
  — auth tokens belong in secure session mechanisms, not JavaScript-readable
  storage.
- OWASP CSRF Prevention Cheat Sheet:
  `https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html`
  — SameSite helps but is not a full CSRF strategy for mutating forms.
- MDN Set-Cookie:
  `https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie`
  — `Set-Cookie` is the browser/server session transport surface.
- MDN Secure Cookie Configuration:
  `https://developer.mozilla.org/en-US/docs/Web/Security/Practical_implementation_guides/Cookies`
  — session cookies should use `Secure`, `HttpOnly`, and `SameSite=Strict` or
  `Lax`.

## Alternatives Considered

| Option | Shape | Strength | Failure Mode | Verdict |
|---|---|---|---|---|
| Owner-only magic link | Allowlisted emails, one-time email challenges, cookie sessions | Fastest real cross-device auth; no password store; fits no-JS mobile app | Email delivery outage can lock out owner; needs CSRF/rate limits | Choose |
| Open public magic link | Anyone can enter email and get account | Simple public UX | Creates spam/abuse and mail reputation risk before product proof | Reject now |
| Password auth | Email/password with Argon2 and reset flow | Familiar, no email-link dependency after signup | Larger attack surface: password policy, hashing, reset, breach posture | Reject |
| OAuth first | Google/GitHub/Apple login | Outsources credential ownership | Provider setup, redirect state, account linking, and mobile OAuth complexity | Defer |
| Passkeys/WebAuthn first | Device-bound public key auth | Strong phishing resistance | Poor bootstrap/recovery for new device without another identity channel | Defer |
| Keep hidden session token + magic link | Magic link returns same `accountId/sessionToken` hidden model | Smallest diff | Still not real auth; credential remains exposed in HTML forms | Reject |
| Local-only Tailscale auth | Keep app private behind Tailscale/Obscura | Good private dogfood | Does not solve browser/device login or production identity | Reject for this ask |

## Tradeoff Matrix

| Option | Fit | Size | Privacy | Agent-manageable | Reversible | Testable | Operating Burden |
|---|---:|---:|---:|---:|---:|---:|---:|
| Owner-only magic link | 5 | 4 | 4 | 4 | 4 | 5 | 3 |
| Open public magic link | 3 | 4 | 3 | 3 | 3 | 4 | 2 |
| Password auth | 3 | 2 | 3 | 3 | 3 | 4 | 2 |
| OAuth first | 4 | 2 | 4 | 2 | 3 | 3 | 3 |
| Passkeys/WebAuthn first | 3 | 1 | 5 | 1 | 2 | 2 | 3 |
| Hidden-token plus link | 1 | 5 | 1 | 5 | 4 | 4 | 4 |
| Local-only network auth | 2 | 4 | 4 | 4 | 5 | 3 | 4 |

Owner-only magic link wins because it solves the real user problem
cross-device resume while keeping signup abuse contained. Hidden-token plus
link is intentionally rejected even though it is smaller, because the user
complaint is specifically that the current session model is not real auth.
OAuth and passkeys are better later once the app needs public users or stronger
anti-phishing, but they are larger than this first correction.

## Technical Design

- Chosen architecture: app-owned passwordless email authentication in
  `memory-engine-api`, backed by Postgres challenge/session tables, with
  server-rendered no-JS forms using cookie session state and CSRF proof.
- Files/systems touched:
  - `crates/memory-engine-api/src/lib.rs`
  - `crates/memory-engine-api/src/main.rs`
  - `crates/memory-engine-api/Cargo.toml`
  - `crates/memory-engine-persistence-postgres/src/lib.rs`
  - `crates/memory-engine-persistence-postgres/Cargo.toml`
  - `crates/memory-engine-qa/src/main.rs` if adding an auth-specific QA lane
  - `docs/architecture/adr-001-production-shell-boundary.md`
  - `docs/qa/fly-staging.md`
- Data/control flow:
  1. `GET /app/login` renders a single email form.
  2. `POST /app/login` normalizes the email, checks rate limits and allowlist
     policy, and always returns the same "check your email" response.
  3. If allowed, the server creates a random challenge token, stores only its
     hash with `email`, `account_id` or pending guest account id, `created_at_ms`,
     `expires_at_ms`, and `consumed_at_ms = NULL`, then dispatches a login link.
  4. `GET /auth/verify?token=...` hashes the token, atomically consumes an
     unexpired challenge, creates or resolves the account, rotates/creates a
     server-side session row, sets `__Host-memory_engine_session`, and redirects
     to `/`.
  5. Every authenticated `/app/*` request resolves account state from the
     session cookie. Mutating forms include a CSRF token that is validated
     against the server-side session.
  6. `POST /app/logout` revokes the server session and clears the cookie.
  7. JSON account routes either accept the same cookie-bound session or stay
     behind an explicit dev/test compatibility flag; production must not depend
     on `x-session-token`.
- New/changed Postgres shape:
  - `memory_engine_auth_identities(email_normalized TEXT UNIQUE, account_id TEXT
    REFERENCES memory_engine_accounts(account_id), created_at_ms BIGINT NOT NULL,
    verified_at_ms BIGINT NOT NULL)`
  - `memory_engine_auth_challenges(challenge_hash TEXT PRIMARY KEY,
    email_normalized TEXT NOT NULL, pending_account_id TEXT, created_at_ms
    BIGINT NOT NULL, expires_at_ms BIGINT NOT NULL, consumed_at_ms BIGINT)`
  - replace or extend `memory_engine_api_sessions` to support multiple
    sessions: `session_id_hash TEXT PRIMARY KEY`, `account_id TEXT NOT NULL`,
    `csrf_token_hash TEXT NOT NULL`, `created_at_ms`, `expires_at_ms`,
    `revoked_at_ms`, `last_seen_at_ms`.
  - Keep a compatibility resolver: if an email has no identity row, compute the
    legacy `account_id_for(email)` and map it to an identity row only if that
    account exists. New accounts should use random account ids.
- Build/check boundary:
  - Unit/route tests fail on missing cookie attributes, hidden auth inputs,
    challenge replay, expired challenge, revoked session, missing/invalid CSRF,
    unknown/known email response divergence, and logout reuse.
  - Live Postgres tests must be runnable with
    `MEMORY_ENGINE_POSTGRES_TEST_URL=...`; if Dagger does not provide Postgres,
    the implementation must add a local Postgres-backed auth lane or explicitly
    record the waiver.
  - `bun run ci`, `cargo run -p memory-engine-qa -- --full`, and deployed Fly
    smoke must pass before closeout.
- ADR decision: no new ADR required if the work stays inside ADR-001's accepted
  API-shell auth boundary. Update ADR-001 consequences/verification with real
  auth proof. Escalate to ADR-002 if choosing OAuth/passkeys, public signup, a
  third-party identity service as source of truth, or moving auth state outside
  Postgres.
- ADR-style invariants:
  - Auth credentials are never rendered into HTML as `accountId` or
    `sessionToken`; violation reintroduces the shipped defect.
  - Email challenge tokens and session ids are stored hashed at rest; violation
    makes database read access equivalent to account takeover.
  - All browser mutating routes require CSRF proof; violation turns cookie auth
    into CSRF exposure.
  - Unknown, disallowed, and known emails return indistinguishable login-request
    responses; violation leaks account/allowlist state.
  - `memory-engine-core` remains free of auth dependencies; violation breaks
    the kernel boundary.
- Design X vs Y:
  - Choose cookie sessions over bearer tokens in hidden fields because browser
    auth credentials need secure transport attributes and server-side revocation.
  - Choose owner-only magic link over public magic link because the immediate
    product need is personal use, not open registration.
  - Choose app-owned Postgres auth tables over OAuth because this slice needs a
    small no-JS cross-device fix before broader identity provider policy.

## Alignment Questions

- Q1: Should this be owner-only first instead of open signup?
  Recommended answer: yes, use `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS`.
  Evidence: the app is deployed publicly at `memory-engine-api.fly.dev`, but
  the product need is "use it myself"; open signup adds mail abuse and account
  policy work.
  Risk if wrong: legitimate early users outside the allowlist cannot sign in
  until invited.
- Q2: Should magic links create accounts or only resume existing accounts?
  Recommended answer: create a verified account when the email owner consumes a
  valid challenge, and bind pending guest study state when present.
  Evidence: the source-first UX in backlog 040 defers signup until useful
  material exists.
  Risk if wrong: forcing login before source capture regresses the current
  mobile flow; forbidding first-login account creation complicates onboarding.
- Q3: Should JSON routes keep `x-session-token`?
  Recommended answer: production should move JSON routes to cookie session
  auth or gate header sessions behind a dev/test compatibility flag.
  Evidence: current `read_session_token` is the same bearer credential problem
  as hidden fields.
  Risk if wrong: scripts may need update; keeping headers in production leaves a
  second auth model to harden.
- Q4: Which mail provider?
  Recommended answer: define a narrow mailer trait and wire the first provider
  through env, with local tests using an in-memory recorder. Production must
  fail startup if auth is enabled without a non-console mailer.
  Evidence: repo keeps external providers outside the kernel and already uses
  env-based production startup checks.
  Risk if wrong: provider choice can block deploy; a too-generic mailer can
  over-abstract before one provider is proven.

## Agent Readiness

- Profile source: `.harness-kit/agent-readiness.yaml` missing; global roster at
  `/Users/phaedrus/.harness-kit/agents.yaml`.
- Stack feedback strength: strong Rust compiler/clippy/tests/rustdoc/Dagger;
  weaker around live Postgres if `MEMORY_ENGINE_POSTGRES_TEST_URL` is absent in
  CI.
- ADR decision: update existing ADR; new ADR only if the auth source of truth
  leaves the accepted API/Postgres boundary.
- Infrastructure path: CLI/API-managed Fly app, Postgres migration, mailer
  secret via Fly/GitHub secrets.
- Gate: `bun run ci` plus `cargo run -p memory-engine-qa -- --full`; deployed
  smoke on `https://memory-engine-api.fly.dev/`.
- Evidence storage: `docs/qa/fly-staging.md` for deploy receipt, and
  `.evidence/cx-real-auth/<date>/` if the delivery branch records browser/API
  smoke artifacts.
- Mock policy impact: preserved if only the external mail delivery boundary is
  mocked/recorded; auth, sessions, CSRF, Postgres, and study routes must be real
  integration tests.

## Delegation Evidence

- Roster providers used:
  - `claude` / Claude Opus 4.8 / repo investigator.
  - `pi` / Kimi K2.6 through Pi / architecture-security critic.
  - `agy` / Gemini 3.5 Flash / product and oracle reviewer.
- Native subagents used: none; lead performed direct repo read.
- Accepted evidence:
  - Claude identified deterministic `account_id_for(email)`, duplicate-email
    enumeration, plaintext session storage, hidden-form transport, Postgres
    session table limits, and the need to preserve cross-machine durable
    validation.
  - Pi identified that cookie auth must add CSRF, challenge replay protection,
    session expiry/revocation, rate limiting, and a hidden-field removal oracle.
  - Agy confirmed magic link as the smallest cross-device product slice and
    proposed concrete route/database/browser oracles.
- Rejected evidence:
  - Agy's suggestion to verify a magic link and then return the same
    `accountId/sessionToken` hidden-input model is rejected because it preserves
    the defect.
  - Pi's suggested IP/device-fingerprint binding is deferred; it can create
    false lockouts on mobile networks and is not needed for the first owner-only
    slice.
- Waivers:
  - The repo has no provider receipt script; provider evidence is summarized
    here.
  - Pi returned useful output but exited nonzero from a stale extension-context
    watchdog after printing its critique.

## Premise Source

Premise Source Waiver: the raw premise is the current Codex chat request,
`/shape real auth`, following the user's complaint that the shipped app is not a
real auth system. That chat is not a repo file and should not be copied into the
repo.

Residual risk: future implementers cannot verify the exact tone/context of the
operator complaint from this repo alone; they can verify the technical gap from
`docs/qa/fly-staging.md` and the current `memory-engine-api` code.

## Exemplar Techniques

- `exemplars.md` says to ignore app-specific session builders; this auth work
  must remain API-shell choreography and must not shape kernel concepts.
- `exemplars.md` reinforces that app content/session flows are outside
  `memory-engine-core`; follow that boundary when adding cookies, challenges,
  and mail delivery.

## Oracle (Definition of Done)

- [ ] `cargo test -p memory-engine-api auth_magic_link_cross_device_resume -- --nocapture`
  proves Device A creates/saves study state, Device B requests a link, consumes
  it, receives a cookie, and renders Device A state.
- [ ] `cargo test -p memory-engine-api auth_rejects_replay_expiry_logout_and_missing_csrf -- --nocapture`
  proves challenge one-time use, expiry, logout revocation, session expiry, and
  CSRF rejection.
- [ ] `cargo test -p memory-engine-api auth_login_request_does_not_enumerate_emails -- --nocapture`
  proves known, unknown, and disallowed emails receive indistinguishable login
  request responses.
- [ ] `cargo test -p memory-engine-api auth_rendered_forms_do_not_expose_session_credentials -- --nocapture`
  proves rendered HTML contains no hidden `sessionToken` or hidden `accountId`.
- [ ] `MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test cargo test -p memory-engine-persistence-postgres auth_challenges_and_sessions_are_hashed_expiring_and_revocable -- --nocapture`
  proves Postgres auth tables, hashed challenge/session ids, expiry, one-time
  consume, and revocation.
- [ ] `cargo run -p memory-engine-qa -- --full` passes, or the branch adds an
  auth lane and reports its exact status.
- [ ] `bun run ci` passes.
- [ ] Deployed smoke proves `/healthz`, `/app/login`, `POST /app/login`,
  `/auth/verify`, authenticated source-to-review resume, logout, and post-logout
  rejection on `https://memory-engine-api.fly.dev/`.
- [ ] Mobile smoke at `390 x 844` verifies login, sent, verify success/failure,
  authenticated study, and logout pages have no horizontal overflow.

## Formal Spec

- Formal Spec Required: yes. Triggers: auth/security behavior changes,
  permissions/session contract changes, user-facing route examples, expensive
  post-merge regressions, and multi-agent implementation risk.
- Informal spec: possession of an allowed email inbox is the proof of identity;
  a short-lived one-time challenge upgrades a browser to a server-side session;
  the session cookie, not hidden form fields, authenticates subsequent browser
  traffic.
- Formal examples:
  - API route tests named in the Oracle section.
  - Postgres contract test named in the Oracle section.
  - Deployed smoke transcript appended to `docs/qa/fly-staging.md`.
- Acceptance oracle: the commands in the Oracle section must fail before
  implementation and pass after.
- Hardening budget: at least one security-diff review, one replay/expiry
  negative-test pass, and one rendered-HTML credential leak grep. Mutation
  testing optional unless auth route tests are too broad to trust.
- Waiver path: only the operator may waive deployed mail-provider proof; if
  waived, record the exact missing provider secret and keep production startup
  from using a console mailer.

## Acceptance Evidence

- Acceptance source: executable auth route tests, Postgres auth contract test,
  full QA command, Dagger gate, deployed Fly smoke, and mobile viewport smoke.
- Evidence that proves it: passing command output, GitHub/Fly deploy receipt,
  and `docs/qa/fly-staging.md` auth receipt.
- Exact command/path/route exercised:
  - Commands listed in the Oracle section.
  - Routes: `GET /app/login`, `POST /app/login`, `GET /auth/verify`,
    `POST /app/logout`, authenticated `/app/source`, `/app/generate`,
    `/app/approve`, `/app/reveal`, `/app/submit`, and `/app/next`.
- Oracle / acceptance artifact hash: none yet; implementation must add hashes
  for any new fixture, transcript, screenshot, or golden artifact.
- Contract-change acknowledgment: this intentionally changes the browser auth
  contract from hidden bearer fields to cookie-backed server sessions.
- Residual risk: real mail deliverability and abuse controls remain weak until
  production mail credentials and rate-limit receipts are proven.

## Deliverable

- Output: working real-auth slice in `memory-engine-api` and Postgres adapter,
  updated docs/evidence, and deployed proof.
- Acceptance oracle: the exact commands/routes in the Oracle section.
- Evidence artifacts: `.evidence/cx-real-auth/<date>/` for local/deploy smoke
  receipts and `docs/qa/fly-staging.md` for durable operator-facing record.
- Residual risk: public signup, OAuth/passkeys/MFA, password recovery, account
  deletion/export, and production email reputation remain out of scope.

## Observability Plan

- Changed behavior to watch: login request rate, challenge creation/consume
  rate, challenge expiry/replay failures, session creation/revocation, CSRF
  failures, logout success, and auth mail delivery failures.
- Named signal or evidence surface: `docs/qa/fly-staging.md` smoke receipt now;
  future lightweight logs in `memory-engine-api` for auth decisions without
  logging tokens, email bodies, or raw links.
- Instrumentation debt if no signal exists: add structured auth event counters
  before removing owner-only allowlist or opening signup.

## Implementation Sequence

1. Add auth data model and tests first: identities, hashed challenges, hashed
   sessions, expiry/revocation, allowlist, and a fake mail recorder.
2. Add cookie helpers and CSRF helpers in `memory-engine-api`; assert exact
   `Set-Cookie` attributes.
3. Add `/app/login`, `POST /app/login`, `/auth/verify`, and logout routes.
4. Refactor app form handlers to resolve account from cookie-backed session and
   remove `account_id/session_token` form fields.
5. Preserve source-first UX with guest session binding after email verification.
6. Migrate JSON route auth to cookie session or explicitly gate legacy
   `x-session-token` behind a non-production compatibility flag.
7. Add route, Postgres, rendered-HTML, and mobile smoke tests.
8. Wire production mail env/secrets; fail production startup if configured to
   console-log login links.
9. Run `cargo run -p memory-engine-qa -- --full`, `bun run ci`, deploy from
   `master`, and append the Fly auth receipt.

## Risk + Rollout

- Main failure: email auth sends links but cookie/CSRF/session validation is
  incomplete, creating a false sense of security. Mitigation: hidden-field
  removal and CSRF/cookie-attribute tests are blocking oracles.
- Main product risk: owner gets locked out by mail delivery. Mitigation:
  verify production provider before removing any operator-only fallback; keep
  fallback disabled in public production.
- Main migration risk: existing deterministic email-derived account ids cannot
  be reverse-mapped to email. Mitigation: on first verified login, compute the
  legacy id for that email and attach it if present; new accounts use random
  ids.
- Rollout: ship behind owner allowlist, deploy to Fly, verify login on a clean
  browser/device, then remove hidden-token UI paths. Roll back by reverting the
  auth commit and restoring the previous deployed image; Postgres migrations
  are additive and should tolerate rollback.
