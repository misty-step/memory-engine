import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type MasteryPolicy,
  type QueueCandidate,
  type ReviewUnitId,
  type ScheduleState,
  pickNextQueueCandidate,
} from '../../src';

const now = Date.UTC(2026, 3, 8, 16, 0, 0);

const vaultMastery: MasteryPolicy<ScheduleState> = (scheduleState) => {
  return scheduleState.state === State.Review && scheduleState.reps >= 3;
};

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

describe('pickNextQueueCandidate', () => {
  test('prefers review candidates over fresh ones when both are due', () => {
    const reviewCandidate = candidate({
      reviewUnitId: reviewUnitId('mass-01'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        scheduled_days: 5,
        due: now - 3_600_000,
      }),
      due: now - 3_600_000,
      sourceKey: 'mass-core',
    });
    const freshCandidate = candidate({
      reviewUnitId: reviewUnitId('latin-01'),
      due: now - 60_000,
      sourceKey: 'mass-core',
    });

    const next = pickNextQueueCandidate([freshCandidate, reviewCandidate], vaultMastery, { now });

    expect(next?.reviewUnitId).toBe(reviewUnitId('mass-01'));
  });

  test('avoids immediate same-source clumps when an equally urgent alternative exists', () => {
    const recentCandidates = [
      candidate({
        reviewUnitId: reviewUnitId('abolition-10'),
        scheduleState: scheduleState({
          state: State.Review,
          reps: 3,
          scheduled_days: 8,
        }),
        sourceKey: 'abolition-of-man',
      }),
    ];
    const sameSource = candidate({
      reviewUnitId: reviewUnitId('abolition-11'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 8,
        due: now - 120_000,
      }),
      due: now - 120_000,
      sourceKey: 'abolition-of-man',
    });
    const alternative = candidate({
      reviewUnitId: reviewUnitId('nato-01'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 8,
        due: now - 100_000,
      }),
      due: now - 100_000,
      sourceKey: 'nato-phonetic',
    });

    const next = pickNextQueueCandidate([sameSource, alternative], vaultMastery, {
      now,
      recentCandidates,
    });

    expect(next?.reviewUnitId).toBe(reviewUnitId('nato-01'));
  });

  test('falls back to the most urgent candidate when no non-clumped alternative exists', () => {
    const recentCandidates = [
      candidate({
        reviewUnitId: reviewUnitId('mass-01'),
        scheduleState: scheduleState({
          state: State.Review,
        }),
        sourceKey: 'mass-core',
      }),
    ];
    const onlyChoice = candidate({
      reviewUnitId: reviewUnitId('mass-02'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 6,
        scheduled_days: 12,
        due: now - 3_600_000,
      }),
      due: now - 3_600_000,
      sourceKey: 'mass-core',
    });

    const next = pickNextQueueCandidate([onlyChoice], vaultMastery, {
      now,
      recentCandidates,
    });

    expect(next?.reviewUnitId).toBe(reviewUnitId('mass-02'));
  });

  test('treats the top-level due timestamp as the queue ordering contract', () => {
    const deferredReview = candidate({
      reviewUnitId: reviewUnitId('mass-01'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        scheduled_days: 5,
        due: now - 60_000,
      }),
      due: now - 3_600_000,
    });
    const staleSnapshot = candidate({
      reviewUnitId: reviewUnitId('mass-02'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        scheduled_days: 5,
        due: now - 3_600_000,
      }),
      due: now - 60_000,
    });

    const next = pickNextQueueCandidate([staleSnapshot, deferredReview], vaultMastery, { now });

    expect(next?.reviewUnitId).toBe(reviewUnitId('mass-01'));
  });

  test('allows callers to disable source clumping with a zero source window', () => {
    const recentCandidates = [
      candidate({
        reviewUnitId: reviewUnitId('abolition-10'),
        scheduleState: scheduleState({
          state: State.Review,
          reps: 3,
          scheduled_days: 8,
        }),
        sourceKey: 'abolition-of-man',
      }),
    ];
    const sameSource = candidate({
      reviewUnitId: reviewUnitId('abolition-11'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 8,
        due: now - 120_000,
      }),
      due: now - 120_000,
      sourceKey: 'abolition-of-man',
    });
    const alternative = candidate({
      reviewUnitId: reviewUnitId('nato-01'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 3,
        scheduled_days: 8,
        due: now - 100_000,
      }),
      due: now - 100_000,
      sourceKey: 'nato-phonetic',
    });

    const next = pickNextQueueCandidate([sameSource, alternative], vaultMastery, {
      now,
      recentCandidates,
      recentSourceWindow: 0,
    });

    expect(next?.reviewUnitId).toBe(reviewUnitId('abolition-11'));
  });

  test('suppresses easier stages once a harder stage is mastered', () => {
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
    const masteredHarderStage = candidate({
      reviewUnitId: reviewUnitId('st-michael-03'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        due: now + 86_400_000,
      }),
      due: now + 86_400_000,
      progression: {
        progressionGroup: 'st-michael-prayer',
        stageOrder: 3,
        requires: [],
        supersedes: [reviewUnitId('st-michael-01')],
      },
    });

    const next = pickNextQueueCandidate([easierStage, masteredHarderStage], vaultMastery, { now });

    expect(next).toBeNull();
  });

  test('skips locked higher stages while prerequisites are unmet', () => {
    const easierStage = candidate({
      reviewUnitId: reviewUnitId('creed-01'),
      due: now - 120_000,
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 1,
        requires: [],
        supersedes: [],
      },
    });
    const lockedHarderStage = candidate({
      reviewUnitId: reviewUnitId('creed-02'),
      due: now - 60_000,
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 2,
        requires: [reviewUnitId('creed-01')],
        supersedes: [],
      },
    });

    const next = pickNextQueueCandidate([easierStage, lockedHarderStage], vaultMastery, { now });

    expect(next?.reviewUnitId).toBe(reviewUnitId('creed-01'));
  });

  test('falls back to an unsatisfied stage instead of hiding it forever', () => {
    const lockedOnlyStage = candidate({
      reviewUnitId: reviewUnitId('creed-02'),
      due: now - 60_000,
      progression: {
        progressionGroup: 'nicene-creed',
        stageOrder: 2,
        requires: [reviewUnitId('missing-prereq')],
        supersedes: [],
      },
    });

    const next = pickNextQueueCandidate([lockedOnlyStage], vaultMastery, { now });

    expect(next?.reviewUnitId).toBe(reviewUnitId('creed-02'));
  });
});
