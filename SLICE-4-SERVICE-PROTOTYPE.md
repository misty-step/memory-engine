# Slice 4: Service Interface Prototype

> Historical note (2026-06-11): This slice packet is archival boundary evidence
> from the service-prototype shaping phase. It is not active ground truth, and
> references to root `src/` predate the Rust workspace cutover. Current strategy
> lives in `SPEC.md`; current gates and deployed surface live in `AGENTS.md`,
> `README.md`, `docs/qa/system.md`, and `docs/runbook.md`.

## Context

Slices 1 through 3 proved the learning kernel: canonical types, FSRS scheduling,
deterministic grading, progression/queue primitives, recitation, rubric
contracts, and adapter boundaries.

The strategy has changed. Scry and the Vault FSRS app are decommission targets,
so their canary branches are historical boundary evidence, not consumer branches
to merge. The next product direction is a focused dedicated microservice.

## Goal

Prototype the service/interface form factor in this repo before extracting the
chosen application into its own repository.

The prototype should answer:

- what command/API envelope the service exposes
- where persistence begins and the pure kernel ends
- how authored learning material enters the system
- how review sessions consume queue, grading, and scheduling behavior
- which code should remain in `memory-engine` after extraction

## Boundary

Keep `src/` pure. It remains framework-free domain code.

Service experiments may live beside the kernel, but they must not force runtime
dependencies, storage clients, logging, auth, or deployment concerns into
`src/`.

## Acceptance

- service contract tests pin the first command surface
- persistence-boundary tests prove storage is outside the kernel
- README and SPEC describe the new direction
- backlog ticket 15 names extraction criteria and non-goals
- `bun run ci` remains green
