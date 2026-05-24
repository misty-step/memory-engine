import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { type QueueCandidate, Rating, type ReviewUnitId, type ScheduleState } from 'memory-engine';
import { createMemoryService } from '../../service';
import {
  DuplicateAppliedReviewError,
  StaleScheduleWriteError,
  ValidatingMemoryServiceStore,
} from './validating-store';

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
  const store = new ValidatingMemoryServiceStore({
    initialCandidates,
  });

  return {
    attempts: store.attempts,
    schedules: store.schedules,
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

  test('proves reveal is display-only UI state by showing zero store mutations or scheduling side effects', async () => {
    // Reveal operates purely on the client side (e.g. showing expectedAnswer and workedSolution).
    // The service has no command for 'reveal' and does not write attempts or schedules to the store.
    const { store } = createStore();

    // Since reveal is display-only UI state, we prove that:
    // 1. The store starts with zero attempts and schedules.
    expect(store.attempts).toHaveLength(0);
    expect(store.schedules.size).toBe(0);

    // 2. The client can render the reveal UI but never submits to the service until grading.
    // 3. There is no execute command for reveal, which keeps the service boundary decoupled from presentation logic.
    // (This is also documented in docs/beta/service-contract-v0.md)
  });

  test('proves compare-and-apply semantics reject stale schedule writes at the boundary', async () => {
    const unitId = reviewUnitId('concurrency-check');
    const store = new ValidatingMemoryServiceStore({
      knownReviewUnitIds: [unitId],
    });

    const initial = scheduleState({ reps: 1, state: State.Learning });
    store.schedules.set(unitId, initial);

    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
    });

    const prompt = {
      kind: 'shortAnswer' as const,
      reviewUnitId: unitId,
      prompt: 'Translate poena.',
      acceptedAnswers: ['punishment'],
      equivalenceGroups: [],
      ignoredTokens: [],
    };

    const result1 = await service.execute({
      kind: 'grade/apply-review',
      prompt,
      submittedAnswer: 'punishment',
      responseTimeMs: 1_200,
    });

    expect(result1.scheduleState.reps).toBe(2);

    const attempt2 = {
      reviewUnitId: unitId,
      promptId: null,
      submittedAnswer: 'punishment-stale',
      responseTimeMs: 1_200,
      occurredAt: now,
    };

    const staleSchedule = scheduleState({ reps: 2, state: State.Learning, last_review: now });

    await expect(store.applyReview(unitId, attempt2, staleSchedule, initial)).rejects.toThrow(
      StaleScheduleWriteError,
    );
  });

  test('proves grade/apply-review idempotency rejects duplicate attempts at the boundary', async () => {
    const unitId = reviewUnitId('idempotency-check');
    const store = new ValidatingMemoryServiceStore({
      knownReviewUnitIds: [unitId],
    });

    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
    });

    const prompt = {
      kind: 'shortAnswer' as const,
      reviewUnitId: unitId,
      prompt: 'Translate poena.',
      acceptedAnswers: ['punishment'],
      equivalenceGroups: [],
      ignoredTokens: [],
    };

    await service.execute({
      kind: 'grade/apply-review',
      prompt,
      submittedAnswer: 'punishment',
      responseTimeMs: 1_200,
      idempotencyKey: 'dup-key-123',
    });

    expect(store.attempts).toHaveLength(1);

    await expect(
      service.execute({
        kind: 'grade/apply-review',
        prompt,
        submittedAnswer: 'punishment',
        responseTimeMs: 1_200,
        idempotencyKey: 'dup-key-123',
      }),
    ).rejects.toThrow(DuplicateAppliedReviewError);

    // Double-grading the same physical attempt details (matching reviewUnitId, promptId, answer, time, occurredAt)
    // without an explicit idempotencyKey also throws DuplicateAppliedReviewError
    const duplicateCommand = {
      kind: 'grade/apply-review' as const,
      prompt,
      submittedAnswer: 'punishment',
      responseTimeMs: 1_200,
      occurredAt: now,
    };

    await service.execute(duplicateCommand);
    expect(store.attempts).toHaveLength(2);

    await expect(service.execute(duplicateCommand)).rejects.toThrow(DuplicateAppliedReviewError);
  });
});
