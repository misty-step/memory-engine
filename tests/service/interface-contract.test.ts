import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { createMemoryService } from '../../service';
import type { MemoryServiceStore, ServiceAttemptRecord } from '../../service';
import { type QueueCandidate, Rating, type ReviewUnitId, type ScheduleState } from '../../src';

const now = Date.UTC(2026, 4, 14, 12, 0, 0);

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

function queueCandidate(
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

function createStore(initialCandidates: QueueCandidate[] = []) {
  const attempts: ServiceAttemptRecord[] = [];
  const schedules = new Map<ReviewUnitId, ScheduleState>();
  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      attempts.push(attempt);
    },
    async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
      return schedules.get(reviewUnitId) ?? null;
    },
    async applyReview(
      reviewUnitId: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      schedule: ScheduleState,
    ): Promise<void> {
      attempts.push(attempt);
      schedules.set(reviewUnitId, schedule);
    },
    async listQueueCandidates(): Promise<QueueCandidate[]> {
      return initialCandidates;
    },
  };

  return {
    attempts,
    schedules,
    store,
  };
}

describe('memory service command interface', () => {
  test('records an attempt without grading or scheduling it', async () => {
    const { attempts, store } = createStore();
    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
    });

    const result = await service.execute({
      kind: 'record-attempt',
      attempt: {
        reviewUnitId: reviewUnitId('latin-amo'),
        promptId: 'latin-present-active',
        submittedAnswer: 'amo',
        responseTimeMs: 2_400,
      },
    });

    expect(result).toEqual({
      kind: 'attempt-recorded',
      attempt: {
        reviewUnitId: reviewUnitId('latin-amo'),
        promptId: 'latin-present-active',
        submittedAnswer: 'amo',
        responseTimeMs: 2_400,
        occurredAt: now,
      },
    });
    expect(attempts).toEqual([result.attempt]);
  });

  test('grades an answer and applies the review to scheduler state', async () => {
    const { attempts, schedules, store } = createStore();
    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
    });

    const result = await service.execute({
      kind: 'grade/apply-review',
      prompt: {
        kind: 'shortAnswer',
        reviewUnitId: reviewUnitId('prayer-kyrie'),
        prompt: 'Kyrie eleison means what?',
        acceptedAnswers: ['Lord have mercy'],
        equivalenceGroups: [],
        ignoredTokens: [],
      },
      submittedAnswer: 'Lord have mercy',
      responseTimeMs: 3_200,
    });

    expect(result.kind).toBe('review-applied');
    expect(result.grade).toMatchObject({
      verdict: 'correct',
      rating: Rating.Good,
      isCorrect: true,
      submittedAnswer: 'Lord have mercy',
      expectedAnswer: 'Lord have mercy',
    });
    const persistedSchedule = schedules.get(reviewUnitId('prayer-kyrie'));

    if (persistedSchedule === undefined) {
      throw new Error('Expected service to persist the applied review schedule');
    }

    expect(result.scheduleState).toEqual(persistedSchedule);
    expect(result.scheduleState.reps).toBe(1);
    expect(attempts).toEqual([
      {
        reviewUnitId: reviewUnitId('prayer-kyrie'),
        promptId: null,
        submittedAnswer: 'Lord have mercy',
        responseTimeMs: 3_200,
        occurredAt: now,
        grade: result.grade,
      },
    ]);
  });

  test('selects the next queue candidate through the shared queue primitive', async () => {
    const dueReview = queueCandidate({
      reviewUnitId: reviewUnitId('review-due'),
      scheduleState: scheduleState({
        state: State.Review,
        reps: 4,
        scheduled_days: 5,
        due: now - 3_600_000,
      }),
      due: now - 3_600_000,
      sourceKey: 'mass-core',
    });
    const fresh = queueCandidate({
      reviewUnitId: reviewUnitId('fresh-new'),
      due: now - 60_000,
      sourceKey: 'mass-core',
    });
    const { store } = createStore([fresh, dueReview]);
    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
    });

    const result = await service.execute({
      kind: 'next-queue',
    });

    expect(result).toEqual({
      kind: 'queue-selected',
      candidate: dueReview,
    });
  });
});
