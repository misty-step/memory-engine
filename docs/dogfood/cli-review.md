# CLI Review Dogfood

Refs-backlog: 21

## Purpose

`crates/memory-engine-cli` is the first dogfood client. It exercises the Rust
facade and service crate from outside the reusable kernel with a narrow,
non-interactive review loop.

The experiment is calibration-aware: the fixture includes learner confidence,
then the receipt reports predicted-versus-actual calibration error after the
graded attempt.

## Commands

```sh
bun run experiments:cli-review
bun run rust:cli-review
cargo test -p memory-engine-cli
```

`bun run experiments:cli-review` resolves to the Rust dogfood path. The former
TypeScript oracle was deleted after Rust receipt parity landed.

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
- explicit list of behavior that stayed outside the pure kernel

## Stayed Outside The Pure Kernel

- fixture content
- confidence capture
- calibration metric
- CLI receipt formatting
- in-memory dogfood store

## Boundary Notes

The experiment uses the Rust facade and service crates without exporting a
product service, adding a parser, introducing persistence, or moving confidence
policy into the kernel.
