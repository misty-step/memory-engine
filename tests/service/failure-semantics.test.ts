import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import type { ReviewUnitId, ScheduleState, ShortAnswerPrompt } from 'memory-engine/types';
import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';
import { ValidatingMemoryServiceStore } from './validating-store';

const now = Date.UTC(2026, 4, 15, 12, 0, 0);

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
    reps: 0,
    lapses: 0,
    state: State.New,
    last_review: null,
    ...overrides,
  };
}

function prompt(reviewUnitIdValue = reviewUnitId('known-unit')): ShortAnswerPrompt {
  return {
    kind: 'shortAnswer',
    reviewUnitId: reviewUnitIdValue,
    prompt: 'Kyrie eleison means what?',
    acceptedAnswers: ['Lord have mercy'],
    equivalenceGroups: [],
    ignoredTokens: [],
  };
}

type StoreFailureMode = 'record' | 'read' | 'apply';

function createValidatingStore(
  options: {
    knownReviewUnitIds?: ReviewUnitId[];
    fail?: StoreFailureMode;
  } = {},
): {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const store = new ValidatingMemoryServiceStore({
    knownReviewUnitIds: options.knownReviewUnitIds ?? [reviewUnitId('known-unit')],
    failMode: options.fail ?? null,
  });

  return {
    attempts: store.attempts,
    schedules: store.schedules,
    store,
  };
}

function createTestService(store: MemoryServiceStore) {
  return createMemoryService({
    store,
    now: () => now,
    masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
  });
}

describe('memory service failure semantics', () => {
  test('does not report attempt success when recordAttempt rejects', async () => {
    const { attempts, store } = createValidatingStore({ fail: 'record' });
    const service = createTestService(store);

    await expect(
      service.execute({
        kind: 'record-attempt',
        attempt: {
          reviewUnitId: reviewUnitId('known-unit'),
          submittedAnswer: 'Lord have mercy',
          responseTimeMs: 1_200,
        },
      }),
    ).rejects.toThrow('recordAttempt failed');
    expect(attempts).toEqual([]);
  });

  test('propagates schedule read failures before grading or applying a review', async () => {
    const { attempts, schedules, store } = createValidatingStore({ fail: 'read' });
    const service = createTestService(store);

    await expect(
      service.execute({
        kind: 'grade/apply-review',
        prompt: prompt(),
        submittedAnswer: 'Lord have mercy',
        responseTimeMs: 1_200,
      }),
    ).rejects.toThrow('readScheduleState failed');
    expect(attempts).toEqual([]);
    expect(schedules.size).toBe(0);
  });

  test('propagates applyReview failures without returning a grading verdict as success', async () => {
    const { attempts, schedules, store } = createValidatingStore({ fail: 'apply' });
    const service = createTestService(store);

    await expect(
      service.execute({
        kind: 'grade/apply-review',
        prompt: prompt(),
        submittedAnswer: 'Lord have mercy',
        responseTimeMs: 1_200,
      }),
    ).rejects.toThrow('applyReview failed');
    expect(attempts).toEqual([]);
    expect(schedules.size).toBe(0);
  });

  test('uses store validation for unknown review units and malformed attempts', async () => {
    const { attempts, store } = createValidatingStore();
    const service = createTestService(store);

    await expect(
      service.execute({
        kind: 'record-attempt',
        attempt: {
          reviewUnitId: reviewUnitId('unknown-unit'),
          submittedAnswer: 'Lord have mercy',
          responseTimeMs: 1_200,
        },
      }),
    ).rejects.toThrow('Unknown review unit: unknown-unit');

    await expect(
      service.execute({
        kind: 'record-attempt',
        attempt: {
          reviewUnitId: reviewUnitId('known-unit'),
          submittedAnswer: '   ',
          responseTimeMs: 1_200,
        },
      }),
    ).rejects.toThrow('Attempt answer must not be blank');

    await expect(
      service.execute({
        kind: 'record-attempt',
        attempt: {
          reviewUnitId: reviewUnitId('known-unit'),
          submittedAnswer: 'Lord have mercy',
          responseTimeMs: 0,
        },
      }),
    ).rejects.toThrow('Attempt response time must be a positive integer');

    expect(attempts).toEqual([]);
  });

  test('validating stores reject mismatched applied review units at the boundary', async () => {
    const { store } = createValidatingStore();
    const attempt: ServiceAttemptRecord = {
      reviewUnitId: reviewUnitId('known-unit'),
      promptId: null,
      submittedAnswer: 'Lord have mercy',
      responseTimeMs: 1_200,
      occurredAt: now,
    };

    await expect(
      store.applyReview(
        reviewUnitId('other-unit'),
        attempt,
        scheduleState({ last_review: now }),
        null,
      ),
    ).rejects.toThrow('Unknown review unit: other-unit');

    await expect(
      store.applyReview(
        reviewUnitId('known-unit'),
        {
          ...attempt,
          reviewUnitId: reviewUnitId('other-unit'),
        },
        scheduleState({ last_review: now }),
        null,
      ),
    ).rejects.toThrow('Applied review unit must match the attempt review unit');
  });
});
