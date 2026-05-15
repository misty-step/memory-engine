# Service Prototype Notes

Refs-backlog: 15

## Current Shape

The service prototype lives in `service/`, outside the published pure kernel in
`src/`. It is a contract probe for a future dedicated memory service, not a new
runtime surface for the `memory-engine` package.

The first command envelope has three commands:

- `record-attempt` records a learner submission without grading or scheduling.
- `grade/apply-review` grades a prompt, records the attempt, and applies the
  resulting rating to scheduler state.
- `next-queue` asks the shared queue primitive to choose the next review
  candidate from consumer-owned candidate data.

## Stays In This Repo

- Canonical prompt, grade, queue, progression, and schedule types.
- Deterministic grading and rating-policy behavior.
- FSRS-backed schedule transitions through `next`.
- Queue eligibility and selection primitives.
- Contract tests that pin the command envelope while the service shape is still
  being discovered.

## Moves On Extraction

- Durable storage implementations for attempts, schedules, content, sessions,
  and learner identity.
- HTTP, RPC, CLI, worker, or daemon adapters.
- Auth, billing, deployment, logging, telemetry, retries, and rate limits.
- Product-specific session choreography and content authoring/import flows.
- Vendor-specific tutor prompts or model clients.

## Boundary Decision

Storage stays behind `MemoryServiceStore`. The prototype may orchestrate kernel
calls, but it does not make `src/` aware of persistence, framework lifecycles,
network clients, or product workflow policy.

`grade/apply-review` hands the graded attempt and the new schedule state to the
store through one `applyReview` boundary method so a future repository can make
that write transactional.

## Failure Semantics

Service commands report success only after the injected store operation
resolves. Store rejections propagate to the caller; the prototype does not
swallow persistence failures, remap them to grading verdicts, retry them, or
pretend a partial write succeeded.

Validation also belongs at the store boundary. A production store or realistic
fake should reject unknown review units, blank submitted answers, invalid
response times, mismatched applied review units, and schedule writes whose
`last_review` does not match the persisted attempt timestamp.

`applyReview` is the transaction seam. Consumers that need durable persistence
must commit the graded attempt and next `ScheduleState` together, or reject the
command so clients can treat the review as unapplied.
