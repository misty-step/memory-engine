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
direction is a world-class modular API for building learning and memorization
applications, plus experimental clients that dogfood the API. Winning clients
can be extracted into their own repositories after executable evidence.

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

The Rust migration has cut over the main runtime:

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

Roadmap and shaping docs:

- [SPEC.md](./SPEC.md)
- [SLICE-1-KERNEL.md](./SLICE-1-KERNEL.md)
- [SLICE-2-PROGRESSION.md](./SLICE-2-PROGRESSION.md)
- [SLICE-3-RUBRIC.md](./SLICE-3-RUBRIC.md)
- [SLICE-4-SERVICE-PROTOTYPE.md](./SLICE-4-SERVICE-PROTOTYPE.md)

Active backlog now tracks beta usefulness, service hardening, graduated
activity support, and extraction decisions on top of the Rust stack.

The active Rust migration ledger is [docs/rust-migration.md](./docs/rust-migration.md).

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

## Development

```sh
git config core.hooksPath .githooks
bun run ci:local
bun run rust:beta-study
bun run rust:cli-review
bun run rust:import-probe
bun run rust:web-shell
bun run ci
dagger call check --source=.
```

## License

MIT
