import { describe, expect, test } from 'bun:test';

import { Grader } from 'memory-engine/grading';
import {
  filterEligibleCandidates,
  filterEligibleCandidatesWithFallback,
} from 'memory-engine/progression';
import { pickNextQueueCandidate } from 'memory-engine/queue';
import { next } from 'memory-engine/scheduling';
import {
  type ProgressionFixture,
  type QueueFixture,
  gradingFixtures,
  progressionFixtures,
  queueFixtures,
  schedulerFixtures,
} from 'memory-engine/testkit';
import type {
  MasteryPolicy,
  ProgressionCandidate,
  QueueCandidate,
  QueueSelectionOptions,
  ScheduleState,
} from 'memory-engine/types';

type FixtureReview = {
  reps: number;
  state: number;
};

const masteryPolicies: Record<string, MasteryPolicy<FixtureReview>> = {
  ruminatio: (review) => review.state === 2 && review.reps >= 2,
  vault: (review) => review.state === 2 && review.reps >= 3,
};

const vaultScheduleMastery: MasteryPolicy<ScheduleState> = (schedule) =>
  schedule.state === 2 && schedule.reps >= 3;

function progressionOptions(fixture: ProgressionFixture): {
  population?: readonly ProgressionCandidate<FixtureReview>[];
} {
  return fixture.population === undefined ? {} : { population: fixture.population };
}

function queueOptions(fixture: QueueFixture): QueueSelectionOptions<QueueCandidate> {
  return {
    now: fixture.now,
    ...(fixture.recentCandidates === undefined
      ? {}
      : {
          recentCandidates: fixture.recentCandidates,
        }),
    ...(fixture.population === undefined
      ? {}
      : {
          population: fixture.population,
        }),
    ...(fixture.recentSourceWindow === undefined
      ? {}
      : {
          recentSourceWindow: fixture.recentSourceWindow,
        }),
  };
}

describe('learning behavior regression corpus', () => {
  test.each(gradingFixtures)('grading: $name', (fixture) => {
    expect(new Grader().grade(fixture.prompt, fixture.submitted, fixture.ctx)).toEqual(
      fixture.expected,
    );
  });

  test.each(schedulerFixtures)('scheduling: $name', (fixture) => {
    expect(next(fixture.initialState, fixture.rating, fixture.now)).toEqual(fixture.expected);
  });

  test.each(progressionFixtures)('progression: $name', (fixture) => {
    const masteryPolicy = masteryPolicies[fixture.masteryPolicy];

    if (masteryPolicy === undefined) {
      throw new Error(`Missing mastery policy for ${fixture.masteryPolicy}`);
    }

    const result =
      fixture.mode === 'fallback'
        ? filterEligibleCandidatesWithFallback(
            fixture.candidates,
            masteryPolicy,
            progressionOptions(fixture),
          )
        : filterEligibleCandidates(fixture.candidates, masteryPolicy, progressionOptions(fixture));

    expect(result.available.map((candidate) => candidate.reviewUnitId)).toEqual(
      fixture.expectedAvailableReviewUnitIds,
    );
    expect(result.lockedFreshCount).toBe(fixture.expectedLockedFreshCount);
  });

  test.each(queueFixtures)('queue: $name', (fixture) => {
    const result = pickNextQueueCandidate(
      fixture.candidates,
      vaultScheduleMastery,
      queueOptions(fixture),
    );

    expect(result?.reviewUnitId ?? null).toBe(fixture.expectedNextReviewUnitId);
  });
});
