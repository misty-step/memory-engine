import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type QueueCandidate,
  type ReviewUnitId,
  type ScheduleState,
  compareQueuePriority,
} from '../../src';

const now = Date.UTC(2026, 3, 8, 16, 0, 0);

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function scheduleState(overrides: Partial<ScheduleState> = {}): ScheduleState {
  return {
    due: now - 60_000,
    stability: 0,
    difficulty: 0,
    elapsed_days: 0,
    scheduled_days: 0,
    learning_steps: 0,
    reps: 0,
    lapses: 0,
    state: State.New,
    last_review: null,
    ...overrides,
  };
}

function candidate(
  overrides: Partial<QueueCandidate> & Pick<QueueCandidate, 'reviewUnitId'>,
): QueueCandidate {
  const currentScheduleState = overrides.scheduleState ?? null;

  return {
    reviewUnitId: overrides.reviewUnitId,
    scheduleState: currentScheduleState,
    due: overrides.due ?? currentScheduleState?.due ?? now - 60_000,
    progression: overrides.progression ?? null,
    conceptKey: overrides.conceptKey ?? null,
    sourceKey: overrides.sourceKey ?? null,
    domainKey: overrides.domainKey ?? null,
  };
}

describe('compareQueuePriority', () => {
  test('orders learning before review before fresh candidates', () => {
    const learningCandidate = candidate({
      reviewUnitId: reviewUnitId('learning-01'),
      scheduleState: scheduleState({ state: State.Learning }),
    });
    const reviewCandidate = candidate({
      reviewUnitId: reviewUnitId('review-01'),
      scheduleState: scheduleState({ state: State.Review, reps: 4 }),
    });
    const freshCandidate = candidate({
      reviewUnitId: reviewUnitId('fresh-01'),
      due: now - 60_000,
    });

    const sorted = [freshCandidate, reviewCandidate, learningCandidate].sort((left, right) => {
      return compareQueuePriority(left, right, now);
    });

    expect(sorted.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('learning-01'),
      reviewUnitId('review-01'),
      reviewUnitId('fresh-01'),
    ]);
  });

  test('prefers the more urgent review candidate by overdue ratio', () => {
    const moreUrgent = candidate({
      reviewUnitId: reviewUnitId('review-urgent'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 2,
        due: now - 172_800_000,
      }),
      due: now - 172_800_000,
    });
    const lessUrgent = candidate({
      reviewUnitId: reviewUnitId('review-calm'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 10,
        due: now - 86_400_000,
      }),
      due: now - 86_400_000,
    });

    const sorted = [lessUrgent, moreUrgent].sort((left, right) => {
      return compareQueuePriority(left, right, now);
    });

    expect(sorted.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('review-urgent'),
      reviewUnitId('review-calm'),
    ]);
  });

  test('prefers the later stage within the same progression family', () => {
    const easierStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-01'),
      due: now - 60_000,
      progression: {
        progressionGroup: 'st-michael-prayer',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const harderStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-03'),
      due: now - 60_000,
      progression: {
        progressionGroup: 'st-michael-prayer',
        stageOrder: 3,
        requires: [],
        supersedes: [],
      },
    });

    const sorted = [easierStage, harderStage].sort((left, right) => {
      return compareQueuePriority(left, right, now);
    });

    expect(sorted.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('st-michael-03'),
      reviewUnitId('st-michael-01'),
    ]);
  });

  test('normalizes progression-group keys before comparing stage order', () => {
    const easierStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-01'),
      due: now - 60_000,
      progression: {
        progressionGroup: 'St-Michael-Prayer',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const harderStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-03'),
      due: now - 60_000,
      progression: {
        progressionGroup: ' st-michael-prayer ',
        stageOrder: 3,
        requires: [],
        supersedes: [],
      },
    });

    const sorted = [easierStage, harderStage].sort((left, right) => {
      return compareQueuePriority(left, right, now);
    });

    expect(sorted.map((entry) => entry.reviewUnitId)).toEqual([
      reviewUnitId('st-michael-03'),
      reviewUnitId('st-michael-01'),
    ]);
  });

  test('falls back to due time, then reps, then review unit id', () => {
    const laterDue = candidate({
      reviewUnitId: reviewUnitId('review-b'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        due: now - 60_000,
      }),
      due: now - 60_000,
    });
    const earlierDue = candidate({
      reviewUnitId: reviewUnitId('review-a'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        due: now - 120_000,
      }),
      due: now - 120_000,
    });
    const lowerReps = candidate({
      reviewUnitId: reviewUnitId('review-c'),
      scheduleState: scheduleState({
        state: State.New,
        reps: 1,
        due: now - 30_000,
      }),
      due: now - 30_000,
    });
    const higherReps = candidate({
      reviewUnitId: reviewUnitId('review-d'),
      scheduleState: scheduleState({
        state: State.New,
        reps: 2,
        due: now - 30_000,
      }),
      due: now - 30_000,
    });
    const lexicalA = candidate({
      reviewUnitId: reviewUnitId('review-e'),
      scheduleState: scheduleState({
        state: State.New,
        reps: 2,
        due: now - 30_000,
      }),
      due: now - 30_000,
    });
    const lexicalB = candidate({
      reviewUnitId: reviewUnitId('review-f'),
      scheduleState: scheduleState({
        state: State.New,
        reps: 2,
        due: now - 30_000,
      }),
      due: now - 30_000,
    });

    expect(compareQueuePriority(earlierDue, laterDue, now)).toBeLessThan(0);
    expect(compareQueuePriority(lowerReps, higherReps, now)).toBeLessThan(0);
    expect(compareQueuePriority(lexicalA, lexicalB, now)).toBeLessThan(0);
  });
});
