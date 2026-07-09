# Memory Engine

[![CI](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml)

`memory-engine` is a learning engine workspace for spaced repetition, answer
grading, modular API design, and dogfood client experiments. It is now a Rust
library/application stack; the former TypeScript runtime and Bun oracle tests
were removed after Rust parity gates covered the package, service, persistence,
generation, study, dogfood, QA, and benchmark surfaces.

It started as a framework-free kernel extracted from four learning apps:

- Ruminatio
- Scry
- Caesar in a Year
- Vault SRS

Scry and the Vault FSRS app are now decommission targets. The current product
direction is a human-facing system for learning and memorizing anything without
hand-designing cards or scheduling policy. The modular API, Rust kernel, and
experimental clients exist to prove and support that product; they are not the
category or the end state. Winning client contracts may be extracted only after
repeated learner evidence.

## What It Owns

- Canonical learning-domain types
- FSRS state transitions
- Deterministic grading
- Progression and queue primitives
- Recitation grading
- Async rubric grading contracts
- Vendor-neutral rubric adapter interfaces
- Fixture corpora for contract and interface tests
- Evals and benchmarks for learning-behavior regressions
- Experimental clients that consume the API outside the reusable kernel

The core runtime in `crates/memory-engine-core` stays framework-free: no
filesystem, network, UI, logging, model clients, or persistence. Service,
storage, UI, auth, content parsing, and deployment experiments live in dedicated
boundary crates until dogfood evidence proves a stable reusable contract.

## Status

The Rust migration is complete for the main runtime:

- canonical types
- FSRS scheduler wrapper
- deterministic grader
- progression metadata and eligibility helpers
- queue candidate filtering and selection
- deterministic recitation grading
- async rubric grading surface
- facade adapter/testkit modules
- service, persistence, generation, study, and local HTTP app hosts
- Rust QA and benchmark receipt runners
- historical Scry and Vault SRS canary branches

The current production dogfood surface is `memory-engine-api`, a Rust binary on
DigitalOcean App Platform backed by Neon Postgres. The former Fly deployment in
`ord` is a temporary standby pending explicit decommission. Agent-facing
deployment, environment, auth, storage, and smoke-test details live in
[docs/runbook.md](./docs/runbook.md).

Current strategy and verification docs:

- [SPEC.md](./SPEC.md)
- [docs/qa/system.md](./docs/qa/system.md)
- [docs/runbook.md](./docs/runbook.md)
- [docs/rust-migration.md](./docs/rust-migration.md)

Historical extraction packets, retained as boundary evidence rather than
active delivery oracles:

- [SLICE-1-KERNEL.md](./SLICE-1-KERNEL.md)
- [SLICE-2-PROGRESSION.md](./SLICE-2-PROGRESSION.md)
- [SLICE-3-RUBRIC.md](./SLICE-3-RUBRIC.md)
- [SLICE-4-SERVICE-PROTOTYPE.md](./SLICE-4-SERVICE-PROTOTYPE.md)
- [exemplars.md](./exemplars.md)

Active backlog now tracks production dogfood usefulness, service hardening,
learning-science quality, input capture, and extraction decisions on top of the
Rust stack.

## Usage

Rust consumers should use the facade crate:

```rust
use memory_engine::{next, ExactPrompt, ExactPromptKind, GradeContext, Grader, Prompt, ReviewUnitId};

let prompt = Prompt::Exact(ExactPrompt {
    kind: ExactPromptKind::ShortAnswer,
    review_unit_id: ReviewUnitId::new("latin-1"),
    prompt: "Translate poena".to_owned(),
    accepted_answers: vec!["punishment".to_owned()],
    equivalence_groups: Vec::new(),
    ignored_tokens: Vec::new(),
});

let grade = Grader::new().grade(
    &prompt,
    "Punishment",
    GradeContext {
        response_time_ms: 3_200,
        prior_reps: 3,
    },
);

let next_state = next(None, grade.rating, 1_779_465_600_000).expect("schedule");
```

Rubric grading stays adapter-backed; the Rust core owns normalization and
dispatch, while callers own any model client:

```rust
use memory_engine::{
    AsyncGrader, GradeContext, GradeablePrompt, RubricAssessment, RubricCriterion,
    RubricCriterionResult, RubricCriterionVerdict, RubricDefinition, RubricPrompt,
    ReviewUnitId, StaticRubricGrader,
};

let prompt = RubricPrompt {
    review_unit_id: ReviewUnitId::new("rubric-1"),
    prompt: "Continue the prayer.".to_owned(),
    rubric: RubricDefinition {
        answer_guide: vec!["Continue with the next line.".to_owned()],
        passing_score: 1,
        criteria: vec![RubricCriterion {
            name: "continuation".to_owned(),
            description: "Gives the next line.".to_owned(),
            required: true,
        }],
    },
};
let grader = AsyncGrader::with_rubric_grader(StaticRubricGrader::new(RubricAssessment {
    model: Some("fixture".to_owned()),
    confidence: 1.0,
    feedback: "Strong answer.".to_owned(),
    criterion_results: vec![RubricCriterionResult {
        name: "continuation".to_owned(),
        verdict: RubricCriterionVerdict::Pass,
        evidence: "Supplied the continuation.".to_owned(),
    }],
}));
let grade = grader.grade_prompt(
    GradeablePrompt::Rubric(&prompt),
    "Strong answer.",
    GradeContext {
        response_time_ms: 6_000,
        prior_reps: 0,
    },
).expect("rubric grade");
```

Test fixtures for contract and interface tests:

```rust
use memory_engine::testkit::{grading_fixtures, scheduler_fixtures};
```

## Quickstart

Prerequisites: the pinned Rust toolchain from `rust-toolchain.toml` and Bun for
the fast gate. Dagger is required for the full/ship parity handoff; it uses the
same Rust 1.94 line through the Dagger image.

Set the repository hook and run the local Rust verification loop:

```sh
git config core.hooksPath .githooks
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
bun run ci
bun run ci:local # compatibility alias for bun run ci
```

Run the Dagger-backed full gate before handoff:

```sh
bun run ci:full
```

Run the production-shaped API locally with a file store:

```sh
MEMORY_ENGINE_ENABLE_FILE_STORE=true MEMORY_ENGINE_API_STORE_DIR=.tmp/api-dev MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=owner@example.com MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=.tmp/api-dev/outbox.tsv HOST=127.0.0.1 PORT=18080 cargo run -p memory-engine-api
```

From another shell, verify the local health route:

```sh
curl -fsS http://127.0.0.1:18080/healthz
```

## License

MIT
