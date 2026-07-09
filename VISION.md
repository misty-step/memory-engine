# Memory Engine Vision

Status: Canonical root vision for Memory Engine. Revise when the human learning
product premise, core engine boundary, or application/service boundary
materially changes.

## What Memory Engine Is

Memory Engine is a human learning and memorization system: an Anki killer, not
an agent memory store. Its job is to help a person learn anything they care
about by turning messy input into high-quality learning material, quiz material,
review sessions, feedback, and adaptive next steps.

The durable product premise is simple: a user should be able to say what they
want to learn or memorize, provide whatever input they have, and get a learning
loop that applies modern learning-science practice without the user having to
design that system themselves.

The repo may ultimately be a full-stack product, a microservice powering several
interfaces, or both. That boundary is still open. What is not open is the core
business logic: spaced repetition, graduated difficulty, desirable difficulty,
atomic concepts, interleaving, prerequisite links, bridge content, grading,
personalization, and evidence-backed content generation belong in Memory Engine
as clear, tested, performant Rust behavior.

## The Learning Model

Memory Engine distinguishes two top-level kinds of material:

- **Learning material**: reference, explanation, study, examples, context, and
  remediation material used when the learner does not yet understand something.
- **Quiz material**: prompts that test recall, recognition, application, or
  understanding. Quiz material is what enters review sessions and spaced
  repetition.

Those materials are linked. If a learner misses a quiz, they should be able to
punch out to the relevant learning material, generate bridge material, or get a
simpler prerequisite path. If they master a topic quickly, the system should
generate harder, broader, or more interesting material rather than grinding the
same cards.

The first quiz formats can be multiple choice and true/false. The product should
progress toward cloze deletion, fill-in-the-blank, translation, recitation,
worked examples, free response, and AI-graded open-ended answers. The review
loop stays the heart of the product.

## North Star

A learner can bring words, sentence fragments, notes, documents, images, video,
or a broad goal like "learn basic biology," "memorize the NATO phonetic
alphabet," "recite this Shakespeare poem," or "understand advanced physics."
Memory Engine helps clarify the goal, generates appropriate learning and quiz
material, schedules review, grades attempts, explains misses, adapts difficulty,
and keeps improving the learner's path from performance evidence.

The experience should feel simple: learn and memorize anything. The underlying
engine can be sophisticated; the user should not have to manage card design,
spacing policy, concept graphs, prerequisite ladders, or content personalization
by hand.

## What Must Stay True

- Humans are the user. Agent workflows, benchmarks, and dogfood loops serve the
  human learning product; they are not the product category.
- `crates/memory-engine-core` stays framework-free and persistence-free: no
  Convex, React, Hono, Node/Bun APIs, filesystem, network, logging, auth,
  analytics, UI state, or vendor SDKs.
- Rust owns durable learning business logic: scheduling, grading, queue
  selection, concept relationships, difficulty progression, interleaving,
  personalization envelopes, and content/linkage semantics.
- AI generates, transforms, explains, adapts, and grades material, but
  deterministic code owns policy, state transitions, scheduler math, persistence
  boundaries, and testable invariants.
- The engine must scale to thousands and eventually millions of concepts without
  turning review selection, concept lookup, or personalization into a slow
  bespoke script.
- Learning quality is measurable. Content fit, quiz validity, grading quality,
  retention, difficulty calibration, latency, and coverage are product
  concerns, not optional benchmark decorations.
- Historical Scry, Vault, Ruminatio, and Caesar material is boundary evidence,
  not the current product direction.

## What Memory Engine Refuses

- Becoming a generic agent memory system.
- Treating Anki-style card review as enough. The system must own learning
  material, quiz material, links between them, personalization, and remediation.
- Prompt-only learning science. Core concepts like spacing, interleaving,
  prerequisite structure, graduated difficulty, and desirable difficulty need
  explicit domain models and tests.
- Unstable library extraction before dogfood evidence.
- Runtime dependencies in the pure kernel.
- Green aggregate tests without ticket-specific proof, live QA, bench evidence,
  or production smoke where the change calls for it.
- Prompt or grader enum drift without exhaustive Rust match coverage.

## Current Bets

1. Build the best review loop first: quiz generation, scheduling, grading,
   misses, remediation links, and post-answer feedback.
2. Model learning material and quiz material as distinct but connected objects.
3. Use AI to turn arbitrary learner input into atomic concepts, explanations,
   quizzes, and adaptive follow-up material.
4. Keep the current DigitalOcean-hosted `memory-engine-api` as the primary
   living proof surface while the full-stack-vs-service boundary remains open;
   the Fly deployment is a temporary standby, not a second product target.
5. Push performance and data-shape decisions early enough that large concept
   graphs remain plausible.
6. Keep `bun run ci` as the canonical gate and add ticket-specific QA, evals, or
   benchmarks when the aggregate gate cannot prove learning behavior.

## What Excellent Looks Like

**Near term.** A user can create a small learning goal, generate linked learning
and quiz material, review due quizzes, get clear feedback on misses, and see the
system choose sensible next reviews.

**Medium term.** Memory Engine handles broad source input, asks useful
clarifying questions, generates multiple quiz types, uses AI grading for free
response, adapts difficulty from learner performance, and remains fast over a
large personal concept graph.

**6–12 month proof.** One learner uses the production system for at least 30
days across enumerable facts, verbatim sequences, and conceptual material. The
attempt history shows whether the daily loop became a habit, how much work it
cost, which generated material was rejected, and whether later cold recall held
up. Product claims are made from that receipt, not from seeded fixtures or a
green aggregate gate.

**Ideal.** Memory Engine is the default tool for learning and memorizing
anything: simpler than Anki at the surface, deeper than Anki in the engine, and
powered by explicit learning-science logic plus AI-assisted material generation
and personalization.

## Where The Depth Lives

- `AGENTS.md` is the repo operating contract and kernel boundary map.
- `SPEC.md` is the older strategy document; when product positioning conflicts,
  this vision governs.
- `README.md` explains the Rust workspace, status, usage, and current docs.
- `docs/runbook.md` is the production API/deployment runbook and smoke contract.
- `docs/qa/system.md`, `docs/qa/quality-register.md`, `docs/dogfood/`, and
  `docs/beta/` hold executable QA and dogfood evidence.
- `backlog.d/` is the active shaped-work queue; `backlog.d/_done/` is closed
  history.
- `bun run ci` is the direct host Cargo fast gate; `bun run ci:full` is the
  Dagger-backed ship-parity gate.
