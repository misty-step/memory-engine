# Rust Migration

Refs-backlog: 38

## Target Shape

The migration target is a Rust library/application stack with the same learning
semantics as the current TypeScript package and beta-study experiments. The
TypeScript code remains the executable oracle until Rust has parity coverage and
the app surfaces have moved.

## Design Rules

- Keep the Rust core pure: no filesystem, network, logging, auth, UI, model
  clients, or service framework code.
- Hide concepts behind deep modules. Callers should ask for grading,
  progression filtering, queue selection, scheduling, service commands, or
  persistence commits; they should not assemble those algorithms from shallow
  helper wrappers.
- Keep `ReviewUnitId` opaque. Concept, phrasing, and activity identity are
  caller-owned mappings.
- Keep mastery policy injected because the source apps intentionally disagree.
- Keep service and storage as later crates with typed command/result enums.

## Current Rust Slice

`crates/memory-engine-core` ports the first pure-kernel surface:

- domain types for prompts, grades, schedule state, progression metadata, and
  queue candidates;
- deterministic grading for MCQ, boolean, cloze, short answer, and recitation;
- progression eligibility with strict and fallback modes;
- queue due filtering, priority ordering, anti-clumping, and progression
  fallback;
- scheduler advancement through a Rust `Scheduler` trait and default
  `FsrsScheduler`, pinned to the current TypeScript FSRS-6 fixture outputs.

The service boundary, persistence store, beta generation, beta-study server, and
web UI are still TypeScript-owned.

## Parity Strategy

The Rust tests intentionally mirror current Bun behavior first. Broader parity
requires:

- shared JSON fixtures for grading, progression, queue, and scheduler cases;
- deeper JSON fixture coverage beyond the first scheduler new/learning/review
  and relearning transitions;
- service scenario fixtures that execute both TypeScript and Rust command
  envelopes until cutover;
- beta-study smoke tests against the Rust server before TypeScript deletion.

## Cutover Matrix

| Surface | Current owner | Rust status | Cutover evidence |
| --- | --- | --- | --- |
| Domain types | TypeScript `src/types.ts` | First core port | JSON fixture parity |
| Deterministic grading | TypeScript `src/grader.ts` | First core port | Fixture parity and property tests |
| Progression | TypeScript `src/progression.ts` | First core port | Vault/Ruminatio-style fixtures |
| Queue | TypeScript `src/queue.ts` | First core port | Priority and anti-clump fixtures |
| Scheduling | TypeScript `src/scheduler.ts` | Rust core port | Shared JSON fixture parity |
| Service | TypeScript `service/` | Not migrated | Command scenario parity |
| Persistence | TypeScript `experiments/beta-store/` | Not migrated | Store commit/restart tests |
| Beta study app | TypeScript `experiments/beta-study/` | Not migrated | Phone/browser smoke on Rust host |
