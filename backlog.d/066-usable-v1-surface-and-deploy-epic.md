# Usable v1 — thin surface audit + deploy target clarification — epic

Priority: P1 · Status: pending · Estimate: M

## Goal

Close the gap between "the engine works" and "a person would want to use
this daily" on the surface that already exists, and resolve a real
contradiction in the dispatch brief about where this should be deployed
before any infra work starts.

## Oracle

- [ ] **Deploy-target contradiction resolved with the operator before any
      migration work starts** (see Notes — this blocks the rest of the
      epic's deploy-shaped children, not the UX-audit children).
- [ ] A cold-start usability pass on the existing server-rendered study UI
      (`crates/memory-engine-beta-app` / `-web-shell`, served from
      `memory-engine-api`): capture, generate, review, miss-and-remediate,
      logout/login-again, on a phone-sized viewport. Friction points are
      logged as follow-on tickets, not fixed inline in this epic.
- [ ] Magic-link deliverability (056, already shaped, "in progress" per its
      own status line) is confirmed landed or explicitly re-scoped here if
      still blocking real usage — an unusable login path makes every other
      UX improvement moot.
- [ ] `docs/runbook.md`'s deployed-smoke contract still passes after any
      change in this epic.

## Notes

Operator's words, relayed via groom dispatch 2026-07-02: "optimizing the user
experience," with a stated deploy target of "Sanctum (the self-host box)"
and instruction to write "the Sanctum deploy plan per bastion's app pattern."

**Contradiction found, not resolved — needs the operator, not an assumption:**

1. No file under `~/Development/*` or the daybook vault names or describes a
   host, box, service, or plan called "Sanctum," as of this investigation
   (2026-07-02). It may be real and simply undocumented, or a
   misremembering of another name — I can't tell from here, so I didn't
   invent a plan for a target I can't verify exists.
2. Memory Engine is **already in production** on Fly (`memory-engine-api`,
   org `misty-step`, region `ord`), with Postgres/Neon, allowlist + magic-link
   auth, Canary error reporting, and a CI-gated deploy pipeline — all
   documented and live per `docs/runbook.md`. This is not a "make it
   deployable" task; it is a "should we move a working production service"
   task, which is a materially bigger and more reversible-cost decision.
3. The bastion pattern referenced (`bastion/docs/self-hosted-tailnet-app.md`)
   is itself a **Fly deployment pattern** (one Fly app, one Machine, one
   SQLite volume, Litestream replication, Tailscale routing via Bastion) —
   not a physical self-host box, and explicitly single-writer SQLite-shaped.
   Memory Engine's production store is Postgres/Neon for a multi-account
   service; forcing it into the SQLite+Litestream shape would be a
   downgrade, not a fit, unless the actual intent is something else
   entirely (e.g., moving off Fly's cost surface to owned hardware, in which
   case the bastion doc is the wrong reference pattern to cite).

Given (1)-(3), this epic's deploy-shaped children are blocked on a direct
answer from the operator: what is Sanctum, and is the ask "move a working
prod service off Fly" or something narrower (e.g., a dev/staging mirror, or
routing Fly's existing app through Bastion's tailnet rather than relocating
the data)? Proceeding on a guess here risks migrating a live, dogfooded,
paying-no-money-but-working service for no clear reason.

The UX-audit children are **not** blocked and should proceed regardless of
the deploy question — "optimizing the user experience" and "where it's
hosted" are separable, and the existing thin surface (048 hypersimple study
interface, 052 review escape hatches, 053 post-answer feedback) is real
work to audit now.

## Children

1. Get the deploy-target question answered by the operator; do not guess.
2. Phone-viewport cold-start usability pass on the existing study UI; log
   friction as new tickets.
3. Confirm 056 (magic-link deliverability) status and re-scope here if it's
   still blocking real (non-allowlist-owner) usage.
4. Only after (1): either a Bastion-tailnet-routing plan for the existing
   Fly app (narrow, additive), or a full redeploy plan to whatever "Sanctum"
   actually is (large, needs its own ticket) — do not conflate the two.
