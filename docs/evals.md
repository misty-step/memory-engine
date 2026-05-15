# Evals And Benchmarks

Refs-backlog: 20

`memory-engine` uses behavior tests as the first eval layer. The goal is to
catch learning-semantic drift before dogfood clients and experimental AI
features build on top of changed behavior.

## Regression Corpus

Run:

```sh
bun test tests/evals/regression-corpus.test.ts
```

The first corpus replays stable fixture data through live API surfaces:

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
