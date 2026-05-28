# Memory Engine

[![CI](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml)

`memory-engine` is a learning engine workspace for spaced repetition, answer
grading, modular API design, and dogfood client experiments. It is currently
migrating from a TypeScript package to a Rust library/application stack while
keeping the TypeScript runtime as the executable parity oracle until cutover.

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
- Experimental clients that consume the API from outside `src/`

The core runtime under `src/` stays framework-free. Service, storage, UI, auth,
content parsing, and deployment experiments must live outside that pure kernel
until dogfood evidence proves a stable boundary.

The Rust core crate follows the same boundary: no filesystem, network, UI,
logging, model clients, or persistence in the reusable kernel.

## Status

Slices 1 through 3 are in place:

- canonical types
- FSRS scheduler wrapper
- deterministic grader
- progression metadata and eligibility helpers
- queue candidate filtering and selection
- deterministic recitation grading
- async rubric grading surface
- dedicated `memory-engine/adapters` rubric adapter subpath
- exported test fixtures
- historical Scry and Vault SRS canary branches

Roadmap and shaping docs:

- [SPEC.md](./SPEC.md)
- [SLICE-1-KERNEL.md](./SLICE-1-KERNEL.md)
- [SLICE-2-PROGRESSION.md](./SLICE-2-PROGRESSION.md)
- [SLICE-3-RUBRIC.md](./SLICE-3-RUBRIC.md)
- [SLICE-4-SERVICE-PROTOTYPE.md](./SLICE-4-SERVICE-PROTOTYPE.md)

Active backlog now tracks Slice 5: modular API entrypoints, service-boundary
failure semantics, evals/benchmarks, CLI dogfood, import probes, web-shell
dogfood, and extraction gates.

The active Rust migration ledger is [docs/rust-migration.md](./docs/rust-migration.md).

## Install

Local path dependency:

```json
{
  "dependencies": {
    "memory-engine": "file:../memory-engine"
  }
}
```

Workspace-style usage also works as long as the package is linked into the consuming repo.

## Usage

Rust consumers should use the facade crate during the migration:

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

The TypeScript package remains available as the executable parity oracle until
cutover:

```ts
import { Grader } from 'memory-engine/grading';
import { next } from 'memory-engine/scheduling';
import type { ReviewUnitId } from 'memory-engine/types';

const grader = new Grader();
const reviewUnitId = 'latin-1' as ReviewUnitId;

const grade = grader.grade(
  {
    kind: 'shortAnswer',
    reviewUnitId,
    prompt: 'Translate poena',
    acceptedAnswers: ['punishment'],
    equivalenceGroups: [],
    ignoredTokens: [],
  },
  'Punishment',
  { responseTimeMs: 3200, priorReps: 3 },
);

const nextState = next(null, grade.rating, Date.now());
```

Test fixtures for contract and interface tests:

```rust
use memory_engine::testkit::{grading_fixtures, scheduler_fixtures};
```

```ts
import {
  gradingFixtures,
  progressionFixtures,
  queueFixtures,
  recitationFixtures,
  schedulerFixtures,
} from 'memory-engine/testkit';
```

Rubric adapters live on a separate subpath:

```ts
import { StaticRubricGrader } from 'memory-engine/adapters';
```

## Development

```sh
bun install
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
