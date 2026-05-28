# CLI Review Dogfood

Refs-backlog: 21

## Purpose

`experiments/cli-review/` is the first dogfood client. It exercises the modular
API and repo-local service prototype from outside `src/` with a narrow,
non-interactive review loop.

The experiment is calibration-aware: the fixture includes learner confidence,
then the receipt reports predicted-versus-actual calibration error after the
graded attempt.

## Commands

```sh
bun test experiments/cli-review/cli-review.test.ts
bun run rust:cli-review
```

`bun run rust:cli-review` is the active dogfood path. The TypeScript experiment
remains as a parity oracle during the Rust migration.

## Fixture

Fixture name: `latin-prayer-opening`

The fixture contains two normalized review units:

- `cli-credo-opening`
- `cli-pater-opening`

The first unit is answered and graded. The service applies the schedule update,
then `next-queue` selects the second unit.

## Service Commands Exercised

- `grade/apply-review`
- `next-queue`

## Receipt Fields

- fixture name
- service commands exercised
- confidence
- calibration error
- attempt count
- grade verdict and rating
- scheduled repetition count
- next selected review unit
- explicit list of behavior that stayed outside `src/`

## Stayed Outside `src/`

- fixture content
- confidence capture
- calibration metric
- CLI receipt formatting
- in-memory dogfood store

## Boundary Notes

The experiment imports public package subpaths for learning-domain types and
uses the repo-local `service/` prototype only as an experiment boundary. It does
not export the service, add a parser, introduce persistence, or move confidence
policy into the kernel.
