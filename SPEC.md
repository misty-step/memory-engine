# Memory Engine Spec

Status: shaping / pre-build  
Date: 2026-04-16

## Executive Summary

`Memory Engine` is the proposed shared learning core for four related applications:

- `../ruminatio`
- `../scry`
- `../caesar-in-a-year`
- `../../Documents/daybook/tools/vault-srs`

The recommendation from the code audit, architecture debate bench, and external research is:

1. Extract a **shared learning engine**.
2. Start it as a **library/package**, not a remote service.
3. Keep the shared core narrow and semantic:
   - canonical domain types
   - scheduling contracts and reference implementations
   - grading contracts
   - progression graph semantics
   - queue selection primitives
   - evaluation fixtures and contract tests
4. Keep product-specific pedagogy and UX outside the core:
   - content taxonomy
   - UI and copy
   - auth, billing, entitlements
   - session choreography
   - app-specific analytics and gamification
   - vendor-specific tutor prompts

This is worth doing because the current duplication is already real and expensive, but a networked microservice would lock unstable semantics too early and add a distributed-systems tax before the domain has stabilized.

## Problem Statement

We now have multiple learning products that rely on overlapping SRS and assessment capabilities:

- Ruminatio
- Vault SRS
- Scry
- Caesar in a Year

Each application currently carries its own version of:

- scheduler state and rescheduling logic
- attempt capture and review history
- grading logic
- queue selection
- progression and mastery logic
- study-session advancement

That duplication is already producing semantic drift. The same underlying ideas are being modeled differently, tested differently, and evolved separately.

The goal is not to centralize all learning-product logic. The goal is to extract the **stable substrate** so each product can build distinct pedagogy on top of one rigorous core.

## Why Now

Three things are simultaneously true:

1. The overlap is no longer hypothetical.
2. The differences between the products are now clear enough to draw a boundary.
3. A service-first move would be premature.

This is the right time to extract a package because the shared kernel is visible, but the deployment topology question is still downstream.

## Source Systems Audited

### Ruminatio

Relevant files:

- [`../ruminatio/convex/scheduler.ts`](../ruminatio/convex/scheduler.ts)
- [`../ruminatio/convex/lib/grading.ts`](../ruminatio/convex/lib/grading.ts)
- [`../ruminatio/convex/lib/progression.ts`](../ruminatio/convex/lib/progression.ts)
- [`../ruminatio/scripts/lib/content.mjs`](../ruminatio/scripts/lib/content.mjs)

Key traits:

- FSRS-backed review records
- stage-gated progression for memorization prompts
- deterministic grading with support for `mc`, `truefalse`, `cloze`, `shortanswer`, `recitation`
- authored content import path from markdown

### Vault SRS

Relevant files:

- [`../../Documents/daybook/tools/vault-srs/src/types.ts`](../../Documents/daybook/tools/vault-srs/src/types.ts)
- [`../../Documents/daybook/tools/vault-srs/src/queue.ts`](../../Documents/daybook/tools/vault-srs/src/queue.ts)
- [`../../Documents/daybook/tools/vault-srs/src/grading.ts`](../../Documents/daybook/tools/vault-srs/src/grading.ts)
- [`../../Documents/daybook/tools/vault-srs/src/parser.ts`](../../Documents/daybook/tools/vault-srs/src/parser.ts)

Key traits:

- richest engine-like semantics today
- explicit `progressionGroup`, `requires`, `supersedes`, `stageOrder`
- deterministic and rubric grading contract
- queue logic with anti-clumping and dependency gating
- content parser that normalizes authored card files into canonical quiz items

### Caesar in a Year

Relevant files:

- [`../caesar-in-a-year/lib/srs/fsrs.ts`](../caesar-in-a-year/lib/srs/fsrs.ts)
- [`../caesar-in-a-year/convex/reviews.ts`](../caesar-in-a-year/convex/reviews.ts)
- [`../caesar-in-a-year/lib/session/builder.ts`](../caesar-in-a-year/lib/session/builder.ts)

Key traits:

- focused FSRS wrapper
- review persistence and stats
- richer session composition around reading, vocab, phrase drills, and review
- AI-assisted grading in adjacent app code for translation/gist-style exercises

### Scry

Relevant files:

- [`../scry/convex/fsrs/engine.ts`](../scry/convex/fsrs/engine.ts)
- [`../scry/convex/phrasings.ts`](../scry/convex/phrasings.ts)

Key traits:

- review infrastructure embedded inside a broader product
- FSRS engine with concept-level state shape
- phrasing generation and review tied into higher-level app concerns

## Observed Overlap Across Apps

The common capabilities are strong enough to justify extraction.

### 1. Study-object modeling

Every app has some version of:

- a study unit
- grouping above that unit
- learner review state
- attempts or submissions
- correctness/outcome

The names differ, but the underlying semantics are the same.

### 2. Scheduling

Multiple apps wrap `ts-fsrs` or equivalent review-state transitions.

This includes:

- mapping domain outcomes to FSRS ratings
- creating empty cards for first review
- serializing/deserializing card state
- calculating next due timestamps

### 3. Grading

All four apps need some form of answer evaluation:

- exact / canonical answer grading
- accepted variant support
- near-miss handling
- reveal semantics
- richer rubric or AI-assisted grading in at least part of the portfolio

### 4. Queue planning

Every app decides what to show next based on some combination of:

- due items
- new items
- progression gating
- anti-clumping / spacing within a session
- app-specific pacing

### 5. Progression

The current portfolio already demonstrates that flat SRS is not enough.

We need support for:

- stage ladders
- prerequisites
- supersession
- concept families
- different prompt forms for the same underlying knowledge

### 6. Content normalization

Even though content authoring formats differ, multiple apps already need a normalization boundary between authored material and runtime study objects.

### 7. Evaluation

None of these systems should evolve blindly.

A shared engine needs:

- golden fixtures
- contract tests
- simulation harnesses
- regression corpora for grading and scheduling behavior

## Where The Apps Legitimately Diverge

The shared engine should not erase these differences.

### Unit of study

- Scry leans toward concept-level review state
- Ruminatio leans toward phrasing-level review state
- Caesar mixes reading/session units with review units
- Vault models richly authored quiz items

This implies that the engine needs a normalized abstraction such as `ReviewUnit`, not a hardcoded `"card"` concept tied to one product.

### Pedagogy

The products are not identical:

- Ruminatio needs staged memorization and recitation
- Caesar needs translation and reading flows
- Vault needs rubric-aware knowledge work
- Scry has its own phrasing generation and broader product context

The engine should own the **primitive semantics** needed by these pedagogies, not the pedagogy itself.

### Session choreography

Session building is clearly app-specific. Caesar’s session builder is not the same problem as Ruminatio’s focused recall loop.

### Content taxonomy

Decks, concepts, passages, prayers, mass responses, language drills, and knowledge prompts belong to the apps or shared content systems, not to the learning engine core.

## Goals

- Eliminate duplicated low-level learning logic across the portfolio.
- Define one canonical domain model for review semantics.
- Support deterministic, rubric, and AI-assisted grading behind one contract.
- Support both flat SRS and progression-graph learning.
- Make learning behavior easier to test, simulate, and compare.
- Create a stable integration point for multiple apps without forcing a service too early.

## Non-Goals

- Building a public SaaS learning API
- Replacing app UIs or content systems
- Owning authentication, billing, or entitlements
- Standardizing all pedagogy into one queue policy
- Forcing every app onto identical session flow
- Solving content authoring and canonical corpora inside this project

## Core Recommendation

Build `Memory Engine` as a **TypeScript-first shared package** with four planned surfaces:

1. `packages/contracts`
2. `packages/core`
3. `packages/adapters`
4. `packages/testkit`

Defer any HTTP or daemon form until the package proves that the boundary is real.

## Architecture Options Considered

### Option 1: Shared library/package

Pros:

- lowest operational cost
- preserves in-process performance
- easiest to adopt incrementally
- reversible if the boundary turns out wrong
- solves the actual current pain: semantic drift

Cons:

- requires discipline around versioning and contracts
- does not automatically solve cross-language reuse
- still allows apps to bypass the core if governance is weak

Verdict: **best option now**

### Option 2: Local sidecar / daemon

Pros:

- isolates runtime from app process
- can support desktop/offline-heavy systems later

Cons:

- adds process lifecycle and IPC complexity
- not obviously better than a package for the current portfolio
- introduces deployment/runtime indirection before needed

Verdict: not justified now

### Option 3: Internal HTTP service

Pros:

- centralizes policy and audit if the domain stabilizes
- enables polyglot clients later

Cons:

- adds auth, latency, availability, tracing, rollout, and compatibility burden
- risks freezing the wrong abstractions too early

Verdict: possible later, not now

### Option 4: Remote multi-tenant microservice

Pros:

- externalizable platform surface if this becomes a product in its own right

Cons:

- highest complexity by far
- worst blast radius
- tenant isolation, quota, SLA, and support burden appear immediately

Verdict: explicit non-goal for now

## Recommended Boundary

The strongest boundary is:

- **inside core**
  - canonical types
  - event schemas
  - scheduling interfaces and reference implementation
  - grading interfaces
  - progression graph primitives
  - queue selection primitives
  - evaluation/testkit

- **outside core**
  - UI
  - content authoring format
  - app-level session choreography
  - auth and billing
  - app-specific analytics
  - product copy
  - prompt-engineering details for product-specific tutors

## Canonical Domain Model

These names are working proposals, not final API.

### `ReviewUnit`

The normalized thing that can be scheduled and assessed.

Must support:

- concept-backed units
- phrasing-backed units
- passage-backed units
- generated or authored prompt forms

### `Prompt`

The presented form of a `ReviewUnit` for a specific exercise.

Examples:

- cloze
- short answer
- multiple choice
- true/false
- recitation
- rubric-scored response

### `AttemptEvent`

Immutable record of a learner interaction.

Includes:

- learner id
- review unit id
- prompt id or prompt fingerprint
- submitted answer
- response time
- grading mode
- timestamp

### `GradingResult`

Canonical assessment envelope.

Includes:

- verdict
- correctness
- rating for scheduler
- expected answer or rubric guide
- comparison or rationale
- grader kind
- confidence when applicable

### `ScheduleState`

Scheduler-facing state for one review unit.

Includes:

- due
- stability
- difficulty
- elapsed/scheduled days
- reps
- lapses
- scheduler state
- last review

### `ProgressionEdge`

Represents dependencies and replacement relationships.

Examples:

- `requires`
- `supersedes`
- stage order
- progression family membership

### `SessionPlan`

Optional structured output from queue-planning primitives, not a mandatory whole-app session abstraction.

This should stay shallow enough that apps can compose their own flows.

## Engine Responsibilities

### Scheduling

- scheduler interface
- FSRS reference implementation
- mapping from learning verdicts to scheduler ratings
- serialization-safe schedule state

### Grading

- deterministic grading contract
- rubric grading contract
- optional AI-assisted grading adapter interface
- reveal semantics

### Progression

- prerequisites
- supersession
- stage ordering
- concept-family semantics
- unlock/mastery checks

### Queue planning

- due-first primitives
- fresh-item selection primitives
- anti-clumping hooks
- progression-aware eligibility

### Contracts

- versioned schemas
- capability flags
- event envelopes
- compatibility rules

### Evals

- golden fixtures
- deterministic regression tests
- cross-app contract tests
- simulation harness for schedule changes

## Explicit Non-Responsibilities

- deck naming and taxonomy
- UI states, animations, and page structure
- auth or user identity infrastructure
- billing and subscriptions
- content import pipelines as a required core concern
- app-specific streak systems and XP systems
- general tutoring chat UX
- narrative lesson orchestration

## Package Topology

These are **logical surfaces first, physical packages second**. The current
repo can stay a single package with subpath exports as long as the boundaries
remain clean. Do not split the filesystem just to satisfy the diagram; promote a
surface into its own package only when adapter/runtime/versioning pressure is
real.

### `packages/contracts`

Owns:

- TypeScript types
- JSON schemas if needed
- versioned event envelopes
- capability flags

### `packages/core`

Owns:

- pure domain logic
- scheduling reference implementation
- queue and progression primitives
- grading interfaces and deterministic implementations

Constraints:

- no framework imports
- no Convex/Bun/React/Hono coupling
- no global singletons for time, user, or storage

### `packages/adapters`

Owns:

- storage adapters
- runtime-specific bridges
- Convex integration
- SQLite/Bun or Node adapters

### `packages/testkit`

Owns:

- golden fixtures
- grading regression corpus
- scheduling simulations
- compatibility harness for consumer apps

## API And Event Contracts

The initial surface should stay small.

Candidate commands:

- `recordAttempt`
- `gradeAttempt`
- `applyReview`
- `nextEligible`
- `planQueue`
- `snapshotState`
- `simulatePolicy`

Each should operate on canonical domain data and return canonical envelopes.

Avoid building a giant service-shaped RPC surface before the domain is stable.

## Scheduling, Grading, And Progression Model

### Scheduling

Use FSRS as the reference scheduler because it is already present in multiple portfolio apps and has strong adoption in modern SRS systems.

References:

- FSRS project: https://github.com/open-spaced-repetition/free-spaced-repetition-scheduler
- Anki deck options / FSRS docs: https://docs.ankiweb.net/deck-options

### Grading

The engine must support three classes of grading:

1. deterministic grading
2. rubric grading
3. AI-assisted grading behind a bounded contract

The engine should not hardcode one vendor’s prompting layer.

### Progression

Flat repetition is not enough for the product family we are building.

The progression model must support:

- stage ladders
- DAG-style prerequisites
- supersession
- multiple prompt forms per concept

This is one of the strongest reasons to use Vault SRS as a design precedent rather than treating simple FSRS wrappers as the whole problem.

## Storage And Adapter Model

The engine should be storage-agnostic.

Two plausible models:

1. mutable snapshot model
2. event log plus derived state

Near-term recommendation:

- support a pragmatic hybrid
- canonical immutable events
- adapter-owned persisted snapshots for speed

Do not force full event sourcing unless it proves necessary.

## Testing And Evaluation Strategy

This project should be unusually strict about behavior.

### Required

- golden grading fixtures
- scheduling regression fixtures
- migration tests for state serialization
- contract tests run against each consumer app
- simulation harness for policy changes

### Strongly recommended

- benchmark checks for queue planning on realistic piles
- error-case fixtures for malformed or partial content
- rubric-grading evaluation corpus

### Acceptance philosophy

If the shared engine cannot prove behavior equivalence or justified improvement, it should not replace app-local logic.

## Migration Plan

### Phase 0: Spec and boundary lock

- define contracts
- define canonical types
- define eval corpus

### Phase 1: Kernel extraction

Start with the most reusable substrate:

- types
- scheduling interfaces
- deterministic grading contracts
- progression graph primitives

Likely strongest precedents:

- Vault SRS for progression and queue semantics
- Ruminatio/Caesar/Scry for integration realities

### Phase 2: First consumer canary

Preferred first candidate: Vault SRS or another least-coupled consumer of the core semantics.

Reason:

- it already looks closest to an engine
- it has rich progression semantics
- lower risk of UI/product breakage than Ruminatio

### Phase 3: Broaden consumers

Probable order:

1. Vault SRS
2. Scry
3. Caesar in a Year
4. Ruminatio

Ruminatio likely comes last because its staged memorization flow and content model are the most visibly product-shaped right now.

### Phase 4: Evaluate need for service wrapper

Only after the package has stable contracts and real multi-consumer usage.

## Promotion Criteria For A Future Service

Consider an internal HTTP service only if at least two or three of these become true:

- non-TypeScript clients need the engine
- independent scaling is required
- release cadence conflicts make package adoption painful
- centralized audit/policy becomes materially valuable
- version skew repeatedly harms behavior

Do not promote to service form just because “shared code” exists.

## Risks, Failure Modes, And Kill Criteria

### Risks

- the boundary is drawn too wide and absorbs app-specific policy
- the boundary is drawn too narrow and fails to reduce duplication
- concept-level and phrasing-level systems normalize poorly
- migration corrupts schedule state
- AI-assisted grading pushes unstable semantics into core

### Failure modes

- “one true queue” API that cannot express app differences
- adapter sprawl caused by weak core design
- version churn that slows every consumer
- regression in learner outcomes despite cleaner architecture

### Kill criteria

Pause or cut back extraction if:

- a proposed shared module has only one real consumer after a quarter
- the shared API needs frequent product-specific flags
- cross-app velocity drops for two consecutive sprints
- more than 20% of core API surface is effectively single-app
- learning behavior degrades without compensating product gains

## Open Questions

- What is the exact normalization boundary for `ReviewUnit`?
- How much of queue policy should live in core versus consumer policy modules?
- What is the smallest stable contract for AI-assisted grading?
- Should progression be represented as direct edges, typed relations, or both?
- What migration order minimizes schedule-state risk?
- What learner-outcome metrics will prove the extraction was worth doing?
- Is TypeScript-only the right medium-term constraint, or do we expect polyglot consumers soon?

## Immediate Next Work

1. Finish slice 2 in the current package:
   - progression metadata and injected-policy eligibility helpers
   - queue primitives
   - deterministic recitation
   - exported slice-2 fixture corpus
   - Scry canary
2. Finish slice 3 with additive async rubric support:
   - shared rubric contract
   - vendor-neutral adapter surface
   - Vault rubric canary
3. Re-evaluate a physical package split only after slice 3 proves that
   `adapters` is a durable surface rather than a speculative one.

## Appendix: Research Notes And References

### Internal research inputs

- direct code audit across the four consumer apps
- subagent architecture debate
- local prior-art review of progression and grading models

### External references

- FSRS official project: https://github.com/open-spaced-repetition/free-spaced-repetition-scheduler
- Anki documentation: https://docs.ankiweb.net/deck-options
- Martin Fowler on microservice prerequisites: https://martinfowler.com/bliki/MicroservicePrerequisites.html
- Retrieval practice / spacing review context: https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2025.1632206/full

### Summary of the architecture debate

Consensus:

- shared kernel: yes
- remote microservice now: no
- strongest current form: versioned package with adapters

Main reason:

The current portfolio’s biggest problem is duplicated learning semantics, not deployment topology.
