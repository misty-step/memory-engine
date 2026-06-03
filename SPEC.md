# Memory Engine Spec

Status: modular API and dogfood shaping
Date: 2026-05-14

## Executive Summary

`Memory Engine` is the modular learning API and dogfood workspace for building
learning and memorization applications.

The original extraction was grounded in four related applications:

- `../ruminatio`
- `../scry`
- `../caesar-in-a-year`
- `../../Documents/daybook/tools/vault-srs`

The strategic direction changed on 2026-05-14:

1. Keep the package/API as the primary product surface.
2. Make the API modular enough for multiple learning and memorization apps to
   compose scheduling, grading, progression, queue planning, fixtures, evals,
   and adapters without reaching into internals.
3. Build a stable of experimental interfaces and clients beside the API to
   dogfood it.
4. Use dogfood receipts, evals, and benchmarks to identify winners and extract
   them into their own application repositories.

The current recommendation is:

1. Keep the Rust core crate as the semantic core.
2. Use the Rust facade crate for stable API surfaces.
3. Add experiments beside, not inside, the pure kernel.
4. Keep the shared core narrow and semantic:
   - canonical domain types
   - scheduling contracts and reference implementations
   - grading contracts
   - progression graph semantics
   - queue selection primitives
   - evaluation fixtures and contract tests
5. Keep interface and product-specific choices outside the core until selected:
   - content taxonomy
   - UI and copy
   - auth, billing, entitlements
   - session choreography
   - app-specific analytics and gamification
   - vendor-specific tutor prompts

This is worth doing because the kernel semantics are now concrete enough to
support real client experiments, while the winning product/interface shape is
still unresolved. The repo should be the proving ground, not the permanent home
for every client.

## Problem Statement

We had multiple learning products that relied on overlapping SRS and assessment
capabilities:

- Ruminatio
- Vault SRS
- Scry
- Caesar in a Year

Those systems demonstrated recurring needs:

- scheduler state and rescheduling logic
- attempt capture and review history
- grading logic
- queue selection
- progression and mastery logic
- study-session advancement

The immediate problem is no longer only multi-consumer package adoption. Scry
and Vault FSRS are decommission targets, so their canaries are evidence for the
kernel boundary rather than branches to merge.

The goal is to turn the stable substrate into a world-class modular API:
canonical learning semantics first, executable evals and benchmarks second,
dogfood interfaces third, separate application repositories after evidence.

## Why Now

Four things are simultaneously true:

1. The kernel is no longer hypothetical; slices 1 through 3 exist.
2. The consumer-canary branches proved useful boundaries but are not the future
   adoption path.
3. The next uncertainty is API ergonomics and client form factor, not whether
   the kernel primitives are useful.
4. Keeping experiments in this repo temporarily lowers coordination cost while
   preserving the option to extract winning clients later.

This is the right time to modularize the API and dogfood it because the
semantic core is visible, but the final application boundary is still
downstream.

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
- Dogfood the API through experimental clients before extracting winners.
- Add evals and benchmarks so API changes are judged by behavior and speed.

## Non-Goals

- Building a public SaaS learning API
- Replacing app UIs or content systems
- Owning authentication, billing, or entitlements
- Standardizing all pedagogy into one queue policy
- Forcing every app onto identical session flow
- Solving content authoring and canonical corpora inside this project
- Extracting an app before at least two dogfood experiments expose repeated
  boundary pressure

## Core Recommendation

Build `Memory Engine` as a **Rust-first modular API** with stable logical
surfaces:

1. `memory_engine::types`
2. `memory_engine::scheduling`
3. `memory_engine::grading`
4. `memory_engine::progression`
5. `memory_engine::queue`
6. `memory_engine::adapters`
7. `memory_engine::testkit`

Keep the root `memory_engine` facade as the ergonomic consumer surface. Defer
any extracted app form until experiments prove that the boundary is real.

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

### Phase 2: Historical consumer canaries

Completed canaries:

- Scry `memory-engine-canary`
- Vault `memory-engine-rubric-canary`

These are retained as boundary evidence. They are not the next adoption path
because Scry and the Vault FSRS app are being decommissioned.

### Phase 3: Service/interface prototype

Prototype the dedicated service shape inside this repo:

- HTTP or RPC command contract
- durable state boundary
- import/export story for authored learning material
- review-session interaction model
- evaluation fixtures for interface-level behavior

Keep experiments outside `crates/memory-engine-core` unless they are pure
reusable kernel code.

### Phase 4: Extract selected service/app

Once the service contract and interface form factor are selected, extract the
application/service into its own repository. Leave this repo as the kernel,
testkit, and possibly protocol package if that split remains useful.

## Extraction Criteria For The Dedicated Service

Extract the service/application only after these are true:

- the command/API surface has survived at least one interface prototype
- the persistence boundary is explicit
- review-session UX constraints are known
- import/export fixtures exist for representative authored material
- the kernel/service split does not require consumer-specific flags
- the extraction plan names which code stays here and which code moves

Do not extract a separate repo while the form factor is still churning.

## Risks, Failure Modes, And Kill Criteria

### Risks

- the boundary is drawn too wide and absorbs app-specific policy
- the boundary is drawn too narrow and fails to reduce duplication
- concept-level and phrasing-level systems normalize poorly
- service prototypes leak framework/runtime coupling into
  `crates/memory-engine-core`
- AI-assisted grading pushes unstable semantics into core

### Failure modes

- “one true queue” API that cannot express the chosen product interaction
- adapter sprawl caused by weak service boundaries
- prototype code masquerading as a stable package API
- regression in learner outcomes despite cleaner architecture

### Kill criteria

Pause or cut back extraction if:

- the service prototype cannot define one narrow user or workflow
- the API needs frequent product-specific flags
- interface experiments keep changing kernel contracts instead of using them
- more than 20% of core API surface is effectively single-app
- learning behavior degrades without compensating product gains

## Open Questions

- What is the exact normalization boundary for `ReviewUnit`?
- How much of queue policy should live in core versus consumer policy modules?
- What is the smallest stable contract for AI-assisted grading?
- Should progression be represented as direct edges, typed relations, or both?
- What migration order minimizes schedule-state risk?
- What learner-outcome metrics will prove the extraction was worth doing?
- What is the smallest dedicated service surface worth extracting?
- Which interface should be proven first: CLI, HTTP API, local web app, or
  hosted service shell?

## Immediate Next Work

1. Shape and implement the first service/interface prototype in this repo.
2. Keep prototype code outside the pure `crates/memory-engine-core` kernel
   unless it is reusable domain logic.
3. Use fixtures and executable interface tests to decide whether the form
   factor is worth extracting into a separate service/application repository.

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
- focused dedicated microservice: yes, but prototype here first
- strongest current form: kernel plus explicit service/interface experiment

Main reason:

The canaries proved enough semantic stability to move from package adoption to
service form-factor discovery.
