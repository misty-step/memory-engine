import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import type { ScheduleState } from 'memory-engine/types';
import {
  parseReviewStateProjectionCandidate,
  projectReviewStateChange,
  projectReviewStateProjection,
} from '../../service/review-state-projection';

const now = Date.UTC(2026, 4, 24, 12, 0, 0);

function scheduleState(overrides: Partial<ScheduleState> = {}): ScheduleState {
  return {
    due: now - 60_000,
    stability: 1.4,
    difficulty: 4.8,
    elapsed_days: 1,
    scheduled_days: 2,
    reps: 3,
    lapses: 1,
    state: State.Review,
    last_review: now - 86_400_000,
    ...overrides,
  };
}

describe('service review-state projection contract candidate', () => {
  test('accepts scheduler state and projects a compact cross-client review DTO', () => {
    const projection = projectReviewStateProjection(
      scheduleState({
        due: now + 3_600_000,
        reps: 5,
        lapses: 2,
        state: State.Learning,
        last_review: now,
      }),
    );

    expect(projection).toEqual({
      due: now + 3_600_000,
      reps: 5,
      lapses: 2,
      state: State.Learning,
      last_review: now,
    });
  });

  test('accepts schedule changes and preserves before/after projection semantics', () => {
    const result = projectReviewStateChange(
      scheduleState({
        due: now - 60_000,
        reps: 1,
        lapses: 0,
        state: State.Learning,
        last_review: now - 60_000,
      }),
      scheduleState({
        due: now + 7_200_000,
        reps: 2,
        lapses: 0,
        state: State.Review,
        last_review: now,
      }),
    );

    expect(result).toEqual({
      before: {
        due: now - 60_000,
        reps: 1,
        lapses: 0,
        state: State.Learning,
        last_review: now - 60_000,
      },
      after: {
        due: now + 7_200_000,
        reps: 2,
        lapses: 0,
        state: State.Review,
        last_review: now,
      },
    });
  });

  test('rejects candidate payloads with client-specific fields', () => {
    expect(() =>
      parseReviewStateProjectionCandidate({
        due: now,
        reps: 1,
        lapses: 0,
        state: 1,
        last_review: now,
        activityKind: 'quiz',
      }),
    ).toThrow('unsupported field: activityKind');
  });

  test('rejects candidate payloads with invalid scheduler state values', () => {
    expect(() =>
      parseReviewStateProjectionCandidate({
        due: now,
        reps: 1,
        lapses: 0,
        state: 4,
        last_review: now,
      }),
    ).toThrow('state must be one of 0, 1, 2, or 3');
  });
});
