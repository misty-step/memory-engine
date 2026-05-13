# Memory Engine

[![CI](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml)

`memory-engine` is a TypeScript learning engine workspace for spaced repetition, answer grading, and service-interface experiments.

It started as a framework-free kernel extracted from four learning apps:

- Ruminatio
- Scry
- Caesar in a Year
- Vault SRS

Scry and the Vault FSRS app are now decommission targets. The next product
direction is a focused dedicated microservice: prototype the service contracts
and interface form factors here, then extract the chosen service/application
into its own repository when the design is stable.

## What It Owns

- Canonical learning-domain types
- FSRS state transitions
- Deterministic grading
- Progression and queue primitives
- Recitation grading
- Async rubric grading contracts
- Vendor-neutral rubric adapter interfaces
- Fixture corpora for contract and interface tests

The core runtime under `src/` stays framework-free. Service, storage, UI, auth,
and deployment experiments must live outside that pure kernel until a focused
microservice shape is selected.

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

```ts
import { Grader, type ReviewUnitId, next } from 'memory-engine';

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
bun run ci
dagger call check --source=.
```

## License

MIT
