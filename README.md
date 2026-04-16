# Memory Engine

[![CI](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/misty-step/memory-engine/actions/workflows/ci.yml)

`memory-engine` is a framework-free TypeScript kernel for spaced repetition and answer grading.

Extracted from four learning apps:

- Ruminatio
- Scry
- Caesar in a Year
- Vault SRS

## What It Owns

- Canonical learning-domain types
- FSRS state transitions
- Deterministic grading
- Consumer fixture corpora for contract tests

It does not own UI, storage, auth, billing, content authoring, or app-specific session choreography.

## Status

Slice 1 is in place:

- canonical types
- FSRS scheduler wrapper
- deterministic grader
- exported test fixtures
- Vault SRS canary integration

Roadmap and shaping docs:

- [SPEC.md](./SPEC.md)
- [SLICE-1-KERNEL.md](./SLICE-1-KERNEL.md)
- [SLICE-2-PROGRESSION.md](./SLICE-2-PROGRESSION.md)
- [SLICE-3-RUBRIC.md](./SLICE-3-RUBRIC.md)

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

Test fixtures for consumer contract tests:

```ts
import { gradingFixtures, schedulerFixtures } from 'memory-engine/testkit';
```

## Development

```sh
bun install
bun run ci
dagger call ci --source=.
```

## License

MIT
