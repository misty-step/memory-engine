# Evals And Benchmarks

Refs-Powder: memory-engine-020
Refs-Powder: memory-engine-050
Refs-Powder: memory-engine-051
Refs-Powder: memory-engine-052

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
- FSRS synthetic histories for steady-success versus repaired-lapse behavior
- queue selection over 1,000 candidates
- queue interleaving anti-clumping over same-domain alternatives
- service command composition for grade/apply-review plus next-queue

The learning-science rationale for the FSRS and interleaving benchmark receipts
lives in `docs/science/README.md`. Keep that file synchronized when a benchmark
case exists primarily to protect a learning-science claim rather than a raw
runtime surface.

Use benchmark output to compare branches manually. If a future regression is
large and repeatable, shape a ticket with an explicit budget and enough history
to avoid brittle thresholds.

## Generation model evals

The prose→quiz generation pipeline is scored by deterministic judges (no model
in the judge loop) over a fixed corpus in
`crates/memory-engine-bench/corpus/generation/`. The bench runs the selected
provider through the same beta generation runner used at runtime, so receipts
score accepted drafts after the production trust gate, duplicate suppression,
and bounded repair pass rather than raw provider output.

Run against the deterministic fake provider (the CI-safe default — no network):

```sh
cargo run -p memory-engine-bench -- generation
```

Run a live model field comparison through Mint's credential-safe egress path:

```sh
# MINT_BASE_URL and the runtime credential alias are private environment inputs.
OPENROUTER_BASE_URL="${MINT_BASE_URL}/proxy/https/openrouter.ai/api/v1" \
OPENROUTER_PROXY_TOKEN="${OPENROUTER_MINT_ALIAS:?private runtime alias required}" \
cargo run -p memory-engine-bench -- generation --model google/gemini-3.5-flash \
  --prompt principled --out docs/evals/generation-<model>-<date>.md
```

Never commit the private base URL or runtime credential alias. The dated
Mint-routed field receipt is [`generation-061-live-mint-2026-07-21.md`](evals/generation-061-live-mint-2026-07-21.md).

`--max-drafts <n>` changes the model draft budget for field sweeps. Keep the
runtime prompt on `prompt-principled` unless a shaped ticket adds and proves a
new prompt variant with a judged receipt that clears its oracle.

Judges score runtime acceptance (accepted persisted drafts divided by persisted
drafts plus pre-persistence trust-gate failures), provenance (evidence quote
actually in source — the same predicate the production trust gate enforces),
answerability,
duplicate rate, count-in-range, key-term coverage, intent shape match, and
variant quality. Duplicate rate uses the same cheap concept + answer + question
surface similarity predicate as the production generation gate, so near-copy
questions that would be rejected at runtime are filtered before the receipt
judges accepted output. The production trust gate also rejects compound MCQs
that ask for multiple atoms and MCQ distractors that duplicate the correct
answer; rejected candidates are now eligible for the same bounded one-repair
pass even when the source already produced other accepted drafts. Variant
quality checks same-concept same-stage groups for meaningfully different
question surfaces and rejects questions that leak the answer text.
Intent shape match is the 051 capture-anything oracle: fixtures annotate
verbatim memorization, enumerable sets, concept understanding, fact recall, and
procedure/process sources, and the provider must emit different activity kinds,
stages, and distractor shapes rather than collapsing them into generic
recognition quizzes. Enumerable and sequential sources additionally pass
through deterministic source coverage policy so a model cannot omit a required
mapping or recitation unit; conceptual prose keeps the fewer-better path. The
receipt also runs the selected provider through a
bridge-material fixture that must use the recent failed attempt context,
produce lower-stage items, stay faithful to the parent concept, and avoid
duplicates against the parent item. The receipt also reports tokens, dollars,
and latency p50/p95.

CI never calls live models; field runs are explicit and local, and their dated
receipts live in `docs/evals/`. The 2026-06-11 field run that picked the
default model is `docs/evals/generation-field-2026-06-11.md`.
