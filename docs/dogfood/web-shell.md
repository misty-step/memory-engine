# Web Shell Dogfood

Refs-Powder: memory-engine-023

## Purpose

`crates/memory-engine-web-shell` is a local interface experiment for the
memory-engine API. It renders a study loop over the import-probe dogfood
fixture, then drives answer submission, reveal, review-state visibility, and
queue transitions through the Rust service boundary.

The former TypeScript `experiments/web-shell/` runtime oracle was deleted after
the Rust crate covered session, receipt, and HTTP route parity. The former
static HTML/JavaScript asset was deleted after the Rust host owned the
server-rendered form UI.

## Commands

```sh
cargo test -p memory-engine-web-shell
cargo run -p memory-engine-web-shell -- --receipt
cargo run -p memory-engine-web-shell
```

The Rust server listens on `http://127.0.0.1:4173` unless `HOST` or `PORT` is
set.
For phone testing over a trusted Tailscale tailnet, run:

```sh
HOST=0.0.0.0 cargo run -p memory-engine-web-shell
```

## Fixture

Fixture name: `latin-prayer-authored-v1`

The shell reuses the import-probe compiler output for two review units:

- `import-credo-in-unum-deum`
- `import-pater-noster`

The first review unit starts with a review schedule. A correct answer updates
that schedule and moves the queue to the second, unscheduled unit.

## Service Commands Exercised

- `next-queue`
- `grade/apply-review`

Reveal is intentionally UI-owned in this experiment. The service has no
first-class revealed-review command, and the ticket did not add one.

The Rust shell exposes this as `WebShellSession::advance()` at the library
boundary to avoid overloading Rust's standard `Iterator::next` convention, while
the HTTP route stays `/next` for TypeScript web-shell parity.

## Interface Pressure

- Review-state visibility needs a compact DTO. Raw `ScheduleState` is useful
  engine state but too engine-shaped for learner-facing copy.
- Reveal is a real interaction path, but today it is client-owned. Promoting it
  would require a shaped service command and a scheduler policy for revealed
  attempts.
- Prompt copy, confidence copy, answer draft state, and layout state stay
  outside the kernel.
- The web shell did not require a UI framework dependency or changes under
  `crates/memory-engine-core`.
- `memory-engine-service` exposes a read-only `store()` accessor so app shells
  can build compact view DTOs without moving or reassembling the service-owned
  workflow.

## Extraction Recommendation

Keep experimenting.

The CLI review, import probe, and web shell now show repeated pressure around
client-owned session choreography and review-state presentation. They do not yet
justify extracting a standalone application repository or promoting web-shell
helpers to `testkit`. A future extraction gate should compare whether a second
interactive client needs the same view DTO and reveal semantics.
