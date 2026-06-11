# Evals And Benchmarks

Refs-backlog: 20

`memory-engine` uses behavior tests as the first eval layer. The goal is to
catch learning-semantic drift before dogfood clients and experimental AI
features build on top of changed behavior.

## Regression Corpus

Run:

```sh
cargo test -p memory-engine
cargo test -p memory-engine-core
```

The Rust facade and core tests replay stable fixture data through live API
surfaces:

- grading fixtures through `Grader`
- scheduler fixtures through `next`
- progression fixtures through eligibility filters
- queue fixtures through `pickNextQueueCandidate`

Add cases when a behavior matters across clients, not for one-off implementation
details. Good eval names should describe the learning behavior, such as
`near-miss-is-close`, `prerequisite-unlocks-next-stage`, or
`anti-clump-yields-to-urgent-review`.

## Benchmarks

Run:

```sh
bun run bench
```

The benchmark script prints receipts with case name, operation count, elapsed
milliseconds, and operations per millisecond. These receipts are intentionally
non-gating: local machines and CI containers vary too much for stable thresholds
until there is more history.

Current benchmark cases cover:

- deterministic short-answer grading
- FSRS schedule transitions
- queue selection over 1,000 candidates
- service command composition for grade/apply-review plus next-queue

Use benchmark output to compare branches manually. If a future regression is
large and repeatable, shape a ticket with an explicit budget and enough history
to avoid brittle thresholds.

## Generation model evals

The prose→quiz generation pipeline is scored by deterministic judges (no model
in the judge loop) over a fixed corpus in
`crates/memory-engine-bench/corpus/generation/`.

Run against the deterministic fake provider (the CI-safe default — no network):

```sh
cargo run -p memory-engine-bench -- generation
```

Run a live model field comparison (requires `OPENROUTER_API_KEY`):

```sh
cargo run -p memory-engine-bench -- generation --model google/gemini-3.5-flash \
  --prompt principled --out docs/evals/generation-<model>-<date>.md
```

Judges score schema validity, provenance (evidence quote actually in source —
the same predicate the production trust gate enforces), answerability,
duplicate rate, count-in-range, and key-term coverage, alongside tokens,
dollars, and latency p50/p95. CI never calls live models; field runs are
explicit and local, and their dated receipts live in `docs/evals/`. The
2026-06-11 field run that picked the default model is
`docs/evals/generation-field-2026-06-11.md`.
