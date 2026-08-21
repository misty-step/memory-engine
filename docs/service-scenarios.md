# Service Scenario Fixtures


## Purpose

The service scenario fixtures prove that the Slice 4 prototype can compose the
first command envelope into a narrow review loop without moving app-owned
behavior into the pure kernel.

They are contract probes, not product workflows. The scenarios translate small
authored fixtures into existing `Prompt`, `QueueCandidate`, and `ScheduleState`
data inside tests instead of adding a parser, session builder, or published API.

## Covered Scenarios

### Deterministic Prompt Loop

`crates/memory-engine-service/tests/command_contract.rs` records an ungraded
note attempt, grades a short-answer prompt, applies the resulting FSRS schedule
state through `MemoryServiceStore::apply_review`, and then asks `next-queue` for
the following candidate.

This proves:

- `record-attempt` can capture learner interaction without grading or
  scheduling it.
- `grade/apply-review` can persist a graded attempt and schedule transition at
  the storage boundary.
- `next-queue` reads consumer-owned candidate data after schedule updates.
- The service can operate on authored prompt data after a consumer has
  normalized it into kernel types.

### Progression-Aware Queue Loop

The progression scenario starts with a locked recitation stage whose prerequisite
memorization stage is not yet mastered. The queue selects the prerequisite first.
After the fake store is seeded with a mastered, not-due prerequisite schedule, `next-queue`
selects the unlocked recitation stage.

This proves:

- Progression unlocks can be driven by persisted `ScheduleState`.
- The service does not need to inspect product-specific concept identity to
  unlock the next stage.
- Queue selection composes with the injected mastery policy and the existing
  progression primitive.

## App-Owned Boundaries

These scenarios deliberately leave the following outside `memory-engine`:

- durable storage implementations
- learner identity and authorization
- authored content parsing
- session choreography
- import/export pipelines
- tutor prompts and model clients
- product analytics, streaks, XP, and UI state

## Remaining Extraction Questions

- Which interface form should be proven first: CLI, HTTP API, local web shell,
  or another command transport?
- Does the selected app need a separate `gradeAttempt` command, or is the
  current `grade/apply-review` transaction boundary the right default?
- What import/export fixture is representative enough to test authored material
  without baking one product taxonomy into the kernel?
- Should service scenario fixtures stay repo-local, or should stable portions
  eventually become testkit fixtures for an extracted service repository?
