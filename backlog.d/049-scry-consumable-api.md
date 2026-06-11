# Shape memory-engine as the microservice Scry consumes

Priority: P2 · Status: pending · Estimate: L

## Goal

Memory-engine's HTTP surface becomes a versioned, documented, JSON API that
an external client (Scry) can build a fully opinionated interface on —
clean separation: engine owns memory science, client owns experience.

## Oracle

- [ ] A versioned JSON API (`/v1/...`) covers the full loop machine-to-
      machine: account/session, source, generate, drafts, queue/next, submit,
      reveal — no HTML coupling, no CSRF dance for token-authenticated
      clients.
- [ ] An OpenAPI (or equivalent) contract file is generated/checked in CI so
      drift between code and contract fails the gate.
- [ ] A contract test suite a consumer could run (fixtures in, responses
      asserted) passes against the deployed service.
- [ ] A demo external consumer (script or minimal client outside the api
      crate) completes the full loop against production using only the
      public contract.

## Notes

Strategy (2026-06-10 session): prototype the interface here, harden the API,
then Scry (`../scry` — Next.js/Convex, currently parked) is resurrected as
the opinionated client and the interface patterns migrate there. The current
JSON routes (`/accounts/...`) are a start but are entangled with the
browser-session/CSRF model. Blocked-by: 045 (decomposition gives the API a
seam to version) and 042 (real time). Don't start before the engine's own
loop is proven daily-usable.

## Children

1. Token-auth (API-key/bearer) lane alongside browser sessions.
2. /v1 JSON contract for the full loop + OpenAPI artifact in CI.
3. Consumer contract test suite + demo external client against production.
4. Migration notes for Scry consumption (what Scry owns vs what engine owns).
