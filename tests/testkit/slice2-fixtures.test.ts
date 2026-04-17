import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type Slice2MasteryPolicy,
  progressionFixtures,
  queueFixtures,
  recitationFixtures,
} from 'memory-engine/testkit';
import {
  Grader,
  type MasteryPolicy,
  type ScheduleState,
  filterEligibleCandidates,
  filterEligibleCandidatesWithFallback,
  pickNextQueueCandidate,
} from '../../src';

type ProgressionReview = {
  reps: number;
  state: number;
};

const progressionMasteryPolicies: Record<Slice2MasteryPolicy, MasteryPolicy<ProgressionReview>> = {
  ruminatio: (review) => review.state >= State.Review || review.reps >= 2,
  vault: (review) => review.state === State.Review && review.reps >= 3,
};

const queueMasteryPolicies: Record<'vault', MasteryPolicy<ScheduleState>> = {
  vault: (review) => review.state === State.Review && review.reps >= 3,
};

describe('slice 2 testkit fixtures', () => {
  test('exports non-empty progression fixtures', () => {
    expect(progressionFixtures.length).toBeGreaterThan(0);
  });

  test('exports non-empty queue fixtures', () => {
    expect(queueFixtures.length).toBeGreaterThan(0);
  });

  test('exports non-empty recitation fixtures', () => {
    expect(recitationFixtures.length).toBeGreaterThan(0);
  });

  test('progression fixtures stay in sync with the live progression helpers', () => {
    for (const fixture of progressionFixtures) {
      const masteryPolicy = progressionMasteryPolicies[fixture.masteryPolicy];
      const options = fixture.population ? { population: fixture.population } : {};
      const result =
        fixture.mode === 'strict'
          ? filterEligibleCandidates(fixture.candidates, masteryPolicy, options)
          : filterEligibleCandidatesWithFallback(fixture.candidates, masteryPolicy, options);

      expect(result.available.map((candidate) => candidate.reviewUnitId)).toEqual(
        fixture.expectedAvailableReviewUnitIds,
      );
      expect(result.lockedFreshCount).toBe(fixture.expectedLockedFreshCount);
    }
  });

  test('queue fixtures stay in sync with the live queue selector', () => {
    for (const fixture of queueFixtures) {
      const options = {
        now: fixture.now,
        ...(fixture.recentCandidates ? { recentCandidates: fixture.recentCandidates } : {}),
        ...(fixture.population ? { population: fixture.population } : {}),
        ...(fixture.recentSourceWindow !== undefined
          ? { recentSourceWindow: fixture.recentSourceWindow }
          : {}),
      };
      const next = pickNextQueueCandidate(
        fixture.candidates,
        queueMasteryPolicies[fixture.masteryPolicy],
        options,
      );

      expect(next?.reviewUnitId ?? null).toBe(fixture.expectedNextReviewUnitId);
    }
  });

  test('recitation fixtures stay in sync with the live grader', () => {
    const grader = new Grader();

    for (const fixture of recitationFixtures) {
      expect(grader.grade(fixture.prompt, fixture.submitted, fixture.ctx)).toEqual(
        fixture.expected,
      );
    }
  });
});
