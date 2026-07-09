# ADR-002: Volatile Knowledge Two-Clocks Model

Status: proposed
Date: 2026-07-04
Refs-Powder: memory-engine-069

## Context

Memory Engine's durable review loop is built around forgetting. FSRS answers:
"when should this learner see the card again so recall stays strong?" That is
the right clock for stable knowledge such as vocabulary, formulae, procedures,
and conceptual bridges.

Some useful study material is not stable. Project-specific decision knowledge
can become wrong because the project changed: an architecture split landed, a
provider was replaced, an incident was resolved, or a local implementation
detail stopped being true. Showing that card again because FSRS thinks the user
is about to forget it is actively harmful. The learner is not forgetting; the
knowledge expired.

Volatile-knowledge mode gives that material a second clock: obsolescence.

## Decision

Memory Engine will model two independent scheduling clocks:

- **Forgetting clock:** FSRS schedule state models human memory over time.
- **Obsolescence clock:** volatile lifecycle metadata models whether the card's
  claim is still valid for the project scope that produced it.

The kernel owns only pure policy for lifecycle eligibility. Boundary crates own
deck creation, event ingestion, webhook/API triggers, persistence, and the
meaning of a project event.

## Accepted Architecture

### TTL Cards

Volatile cards may carry an absolute `ttl_expires_at` millisecond timestamp.
The timestamp is evaluated by pure functions against a caller-supplied `now`.
If `now >= ttl_expires_at`, the card is retired from queue scheduling even if
its FSRS `due` timestamp says it should be reviewed.

TTL is for claims expected to age out without a discrete event: "this audit is
fresh for 14 days", "this deploy runbook reflects the current staging shape",
or "this provider shortlist is valid through the current bakeoff window".

### Project-Scoped Decks

Volatile cards belong to project-scoped decks. A project deck is a boundary
object: account, project key, deck/source id, source material, optional TTL,
and operator-facing labels live outside `memory-engine-core`.

At the kernel boundary, the deck scope is represented only by queue metadata
such as `source_key` and lifecycle policy. The kernel never parses project
names, repository paths, GitHub issue numbers, webhook payloads, or architecture
events.

### Event-Based Invalidation

A boundary event can retire a project deck before TTL. Examples:

- A webhook reports an architecture-changing PR merge.
- An operator calls an API after accepting a new decision record.
- A deploy/runbook change makes an old troubleshooting card unsafe.

The API/storage layer translates that event into persisted lifecycle state or
archives the deck's source/review units. The kernel sees only
`invalidated_at` and decides that invalidated cards are unschedulable for a
passed-in `now`.

### Transferable-Principle Promotion

Volatile project cards are not always disposable. If a project-specific card
keeps proving useful after several events, the operator or future product flow
can promote the transferable principle into a durable deck.

Promotion creates new stable learning material with no volatile lifecycle, for
example:

- Project fact: "memory-engine-api-state owns account/session storage after
  the #27 split."
- Transferable principle: "keep HTTP route registration separate from account
  state and persistence when decomposing a Rust API crate."

Promotion is a boundary workflow. The kernel does not infer, generate, or
approve principles; it only schedules the resulting stable cards once created.

## Kernel Contract

`crates/memory-engine-core` may own:

- JSON-safe lifecycle types for queue candidates.
- Pure eligibility decisions for active, TTL-expired, and invalidated cards.
- Reference queue behavior that excludes unschedulable candidates when `now` is
  supplied by the caller.
- Tests proving that FSRS due cards stop scheduling when obsolescence retires
  them.

`crates/memory-engine-core` must not own:

- Clock reads, timers, cron jobs, or wall-clock construction.
- Webhook/API routes, HTTP status behavior, auth, account, or repository state.
- Persistence, filesystem, database, migrations, or event logs.
- Project naming, deck ownership, source ingestion, or operator approval.
- Model calls, semantic event classification, or promotion decisions.

The invariant is the same one used by FSRS scheduling: the caller supplies
`now`; the kernel applies deterministic policy over data it is given.

## Public Boundary Shape

The API can expose project deck operations as a thin v1 surface:

- Create a project deck by saving source material and optional TTL metadata.
- Generate/approve cards through the existing source-to-review path.
- Retire the deck by event, which archives or invalidates all generated review
  units linked to the project deck.
- Verify that `review/next` no longer returns retired cards.

HTTP routes belong in `crates/memory-engine-api`. State, storage, and
translation from API requests to study-session operations belong in
`crates/memory-engine-api-state`. Persistence details remain in persistence
crates. The study/session layer may compose those boundaries, but volatile
policy remains data-in/data-out.

## Rejected Alternatives

- **FSRS-only volatile cards:** rejected because a correct memory model can
  schedule an obsolete card. Obsolescence is a different failure mode.
- **Put deck events in the kernel:** rejected because project events are
  framework, repository, HTTP, and persistence concerns.
- **Archive sources only:** useful as boundary behavior, but insufficient as the
  kernel contract because it does not make TTL and invalidation policy explicit
  or reusable across stores.
- **Heuristic promotion in the scheduler:** rejected because promotion is a
  semantic/product decision that needs operator validation or a model boundary,
  not hidden queue logic.

## Consequences

- Persisted queue candidates gain backward-compatible lifecycle metadata.
- Queue selection must ignore expired or invalidated candidates before applying
  FSRS due priority, progression gates, or separation windows.
- API callers get a concrete way to retire project knowledge when an event
  proves it stale.
- Future promotion flows can reuse the same deck metadata without weakening the
  pure-kernel invariant.

## Verification

- Core unit tests: active candidates schedule normally; TTL-expired candidates
  do not schedule; invalidated candidates do not schedule; not-yet-expired
  candidates remain eligible.
- API route test: create a project deck, generate/approve cards, observe a due
  review, invalidate the deck by event, and observe that `review/next` no
  longer returns the retired card.
- Local API transcript: repeat the worked example with the running service and
  record request/response evidence in the lane receipt.
- Fast and full gates: `bun run ci` and `bun run ci:full`.
