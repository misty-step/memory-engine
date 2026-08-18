# Scry Product Vision

Status: Canonical consumer-product vision. Scry is the product, and this repository is its Rust engine. Revise this file when that product premise, the shared capability boundary, or the proof bar materially changes.

## The Product

Scry helps a person learn and memorize anything they want to know. Its promise is **Remember everything.** The experience should feel effortless: bring a goal or source, answer a useful quiz, and let the system turn each attempt into the next best review. Quiz-driven memorization is the heart of Scry. Learning material exists to make the quiz loop better, especially when a learner misses or is close.

Scry is not a generic agent-memory store, a chat tutor, or a card database that makes the learner design scheduling policy by hand. The product earns trust by making good learning decisions visible: what to review, why an answer was graded, and what easier bridge quiz comes before a missed item returns.

## Product Loop

1. A learner brings a goal or source material, such as available study material or a short prompt.
2. Scry turns that input into atomic, reviewable quiz material and the context needed to answer it.
3. The learner starts a quiz quickly, answers, and receives clear graded feedback.
4. Scry schedules the next review from the result and adapts difficulty from the learner's evidence.
5. A wrong, close, or revealed attempt produces easier bridge quiz items for prerequisite or atomic concepts before the failed item returns.

The near-term bet is a strong quiz loop, not full course generation. More learning material, richer question types, AI-graded free response, and broader remediation can follow evidence from that loop. Anki import is a later roadmap item, not a near-term requirement or the differentiator.

## Five Faces, One Capability System

Scry's capability boundary serves five faces:

- **PWA** — the primary human surface, designed for a fast phone-first study flow.
- **CLI** — a direct operator and power-user interface.
- **Skill** — a product-facing agent workflow over the same capabilities.
- **MCP** — a typed tool surface for agents and applications.
- **API** — the service boundary for the PWA and other clients.

The faces must agree on the same identifiers, quiz semantics, grading verdicts, scheduling behavior, and lifecycle states. The PWA is primary; the other faces are not separate products with separate learning rules.

## Access and Business Model

Beta access is invite-gated. The entry path makes the waitlist visible instead of pretending that public access is open. Human sign-in uses magic links only; Scry does not add OAuth as a hidden second path. Machine faces use operator-gated service sessions instead of email.

Subscription is the intended business model. Public signup remains closed until generation costs are bounded and privacy, reliability, and Stripe billing behavior are proven. Beta access must not create commitments that the production cost and trust boundaries cannot support.

## Fast and Smooth Are Product Bars

Fast and smooth describes the user experience, not a benchmark vanity metric. These are explicit product acceptance bars:

- Every interaction is acknowledged within **p95 < 100 ms**.
- A quiz tap reaches graded-visible feedback within **p95 < 300 ms**.
- The first quiz becomes visible within **p95 < 20 s** from the learner's start action.

Optimistic UI may acknowledge input before slower work completes, but it must not hide a lost answer or misrepresent grading. Benchmark buckets and performance campaigns are evidence for these bars; they are not substitutes for the bars or for a usable study flow.

## Engine and Boundary Architecture

Scry's Rust engine keeps the pure kernel framework-free and persistence-free. `crates/memory-engine-core` owns deterministic learning behavior: scheduling, grading, queue selection, progression, difficulty, interleaving, opaque review-unit identity, and domain invariants. It does not know about the filesystem, network, auth, analytics, UI state, logging, model vendors, React, Hono, Node, or Bun.

Boundary crates own orchestration, persistence, source ingestion, generation providers, sessions, identity, API routes, rendering, clients, deployment, and QA. AI may generate, explain, adapt, or grade material through an explicit boundary; deterministic Rust owns policy and state transitions. Keep the boundary explicit instead of moving service complexity into the kernel.

## Current Production Surface

The current production proof surface is the native Rust `memory-engine-api` process on Misty Step's isolated DigitalOcean public application host, backed by Neon Postgres and served at `https://scry.study`. Its deployment, auth, storage, rollback, and smoke contract live in `docs/runbook.md`. This document describes the product; the runbook and executable QA documents provide operational proof.

## Proof Bar

The first real outcome gate is a 30-day retention proof, not a seeded fixture, a planning document, or a green aggregate check. It must use production evidence from the quiz-driven learning loop, and product claims stay bounded by what that evidence establishes. The 30-day protocol is a proof of a working learner workflow and retention signal, not a claim of market fit.

## What Scry Refuses

- Generic agent memory as the product category.
- A quiz loop that hides why an answer was graded or what to do after a miss.
- Prompt-only learning science where scheduling, grading, progression, and remediation have no explicit domain model or testable invariant.
- OAuth or public signup before the invite, cost, privacy, reliability, and billing gates are ready.
- Native-first expansion before the PWA and retention proof justify it.
- Runtime dependencies in the pure Rust kernel.

## Where the Depth Lives

- `AGENTS.md` is the repository operating contract and kernel boundary map.
- `README.md` explains Scry's product, Rust workspace, production surface, and current usage.
- `SPEC.md` and `docs/rust-migration.md` retain technical strategy and cutover context; this vision governs product positioning.
- `docs/runbook.md`, `docs/qa/system.md`, `docs/dogfood/`, and `docs/beta/` hold operational and executable evidence.
- `backlog/` is the shaped-work queue, assignment ledger, and closure history.
- `bun run ci` is the direct host Cargo fast gate; `bun run ci:full` is the Dagger-backed ship-parity gate. This docs correction does not replace Main-owned gates.
