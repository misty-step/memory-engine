import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type MasteryPolicy,
  type ProgressionCandidate,
  type ReviewUnitId,
  filterEligibleCandidates,
  filterEligibleCandidatesWithFallback,
} from '../../src';

type Review = {
  reps: number;
  state: number;
};

type Candidate = ProgressionCandidate<Review>;

const ruminatioMastery: MasteryPolicy<Review> = (review) => {
  return review.state >= State.Review || review.reps >= 2;
};

const vaultMastery: MasteryPolicy<Review> = (review) => {
  return review.state === State.Review && review.reps >= 3;
};

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function candidate(overrides: Partial<Candidate> & Pick<Candidate, 'reviewUnitId'>): Candidate {
  return {
    reviewUnitId: overrides.reviewUnitId,
    review: overrides.review ?? null,
    progression: overrides.progression ?? {
      progressionGroup: null,
      stageOrder: 1,
      requires: [],
      supersedes: [],
    },
  };
}

describe('filterEligibleCandidates', () => {
  test('keeps later fresh stages locked until earlier stages are stabilized', () => {
    const result = filterEligibleCandidates(
      [
        candidate({
          reviewUnitId: reviewUnitId('a-stage-1'),
          progression: {
            progressionGroup: 'concept-a',
            stageOrder: 1,
            requires: [],
            supersedes: [],
          },
        }),
        candidate({
          reviewUnitId: reviewUnitId('a-stage-2'),
          progression: {
            progressionGroup: 'concept-a',
            stageOrder: 2,
            requires: [],
            supersedes: [],
          },
        }),
      ],
      ruminatioMastery,
    );

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('a-stage-1'),
    ]);
    expect(result.lockedFreshCount).toBe(1);
  });

  test('unlocks the next fresh stage after the prior stage is stable', () => {
    const result = filterEligibleCandidates(
      [
        candidate({
          reviewUnitId: reviewUnitId('a-stage-1'),
          review: { state: State.Review, reps: 2 },
          progression: {
            progressionGroup: 'concept-a',
            stageOrder: 1,
            requires: [],
            supersedes: [],
          },
        }),
        candidate({
          reviewUnitId: reviewUnitId('a-stage-2'),
          progression: {
            progressionGroup: 'concept-a',
            stageOrder: 2,
            requires: [],
            supersedes: [],
          },
        }),
      ],
      ruminatioMastery,
    );

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('a-stage-1'),
      reviewUnitId('a-stage-2'),
    ]);
    expect(result.lockedFreshCount).toBe(0);
  });

  test('uses the wider population to unlock later stages', () => {
    const stageOne = candidate({
      reviewUnitId: reviewUnitId('a-stage-1'),
      review: { state: State.Review, reps: 2 },
      progression: {
        progressionGroup: 'concept-a',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const stageTwo = candidate({
      reviewUnitId: reviewUnitId('a-stage-2'),
      progression: {
        progressionGroup: 'concept-a',
        stageOrder: 2,
        requires: [],
        supersedes: [],
      },
    });

    const result = filterEligibleCandidates([stageTwo], ruminatioMastery, {
      population: [stageOne, stageTwo],
    });

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('a-stage-2'),
    ]);
    expect(result.lockedFreshCount).toBe(0);
  });

  test('treats progression-group keys case-insensitively across population checks', () => {
    const stageOne = candidate({
      reviewUnitId: reviewUnitId('a-stage-1'),
      review: { state: State.Review, reps: 2 },
      progression: {
        progressionGroup: 'Concept-A',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const stageTwo = candidate({
      reviewUnitId: reviewUnitId('a-stage-2'),
      progression: {
        progressionGroup: ' concept-a ',
        stageOrder: 2,
        requires: [],
        supersedes: [],
      },
    });

    const result = filterEligibleCandidates([stageTwo], ruminatioMastery, {
      population: [stageOne, stageTwo],
    });

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('a-stage-2'),
    ]);
  });

  test('suppresses superseded units once a harder stage is mastered', () => {
    const easierStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-01'),
      progression: {
        progressionGroup: 'st-michael-prayer',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const masteredHarderStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-03'),
      review: { state: State.Review, reps: 4 },
      progression: {
        progressionGroup: 'st-michael-prayer',
        stageOrder: 3,
        requires: [],
        supersedes: [reviewUnitId('st-michael-01')],
      },
    });

    const result = filterEligibleCandidates([easierStage], vaultMastery, {
      population: [easierStage, masteredHarderStage],
    });

    expect(result.available).toEqual([]);
    expect(result.lockedFreshCount).toBe(1);
  });

  test('skips locked higher stages while prerequisites are unmet', () => {
    const easierStage = candidate({
      reviewUnitId: reviewUnitId('creed-01'),
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const lockedHarderStage = candidate({
      reviewUnitId: reviewUnitId('creed-02'),
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 2,
        requires: [reviewUnitId('creed-01')],
        supersedes: [],
      },
    });

    const result = filterEligibleCandidates([easierStage, lockedHarderStage], vaultMastery);

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([reviewUnitId('creed-01')]);
  });
});

describe('filterEligibleCandidatesWithFallback', () => {
  test('falls back to an unsatisfied stage instead of hiding it forever', () => {
    const lockedOnlyStage = candidate({
      reviewUnitId: reviewUnitId('creed-02'),
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 2,
        requires: [reviewUnitId('missing-prereq')],
        supersedes: [],
      },
    });

    const result = filterEligibleCandidatesWithFallback([lockedOnlyStage], vaultMastery);

    expect(result.available.map((entry) => entry.reviewUnitId)).toEqual([reviewUnitId('creed-02')]);
    expect(result.lockedFreshCount).toBe(1);
  });
});
