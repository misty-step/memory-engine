import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';
import {
  type MasteryPolicy,
  type Prompt,
  type QueueCandidate,
  Rating,
  type ReviewUnitId,
  type ScheduleState,
} from '../../src';

const now = Date.UTC(2026, 4, 14, 12, 0, 0);

const masteryPolicy: MasteryPolicy<ScheduleState> = (scheduleState) => {
  return scheduleState.state === State.Review && scheduleState.reps >= 3;
};

type AuthoredReviewUnitFixture = {
  reviewUnitId: ReviewUnitId;
  promptId: string;
  prompt: Prompt;
  queue: Omit<QueueCandidate, 'reviewUnitId' | 'scheduleState' | 'due'>;
  scheduleState?: ScheduleState;
};

type ScenarioFixture = {
  units: AuthoredReviewUnitFixture[];
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
    reps: 0,
    lapses: 0,
    state: State.New,
    last_review: null,
    ...overrides,
  };
}

function createValidatingScenarioStore(fixture: ScenarioFixture): {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const attempts: ServiceAttemptRecord[] = [];
  const units = new Map(fixture.units.map((unit) => [unit.reviewUnitId, unit]));
  const schedules = new Map<ReviewUnitId, ScheduleState>(
    fixture.units.flatMap((unit) =>
      unit.scheduleState === undefined ? [] : [[unit.reviewUnitId, unit.scheduleState]],
    ),
  );

  function assertKnownReviewUnit(reviewUnitId: ReviewUnitId): void {
    if (!units.has(reviewUnitId)) {
      throw new Error(`Unknown review unit: ${reviewUnitId}`);
    }
  }

  function assertAttemptContract(attempt: ServiceAttemptRecord): void {
    assertKnownReviewUnit(attempt.reviewUnitId);
    if (attempt.submittedAnswer.trim() === '') {
      throw new Error('Attempt answer must not be blank');
    }
    if (!Number.isInteger(attempt.responseTimeMs) || attempt.responseTimeMs <= 0) {
      throw new Error('Attempt response time must be a positive integer');
    }
    if (!Number.isInteger(attempt.occurredAt)) {
      throw new Error('Attempt timestamp must be an integer epoch millisecond value');
    }
  }

  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      assertAttemptContract(attempt);
      attempts.push(attempt);
    },
    async readScheduleState(unitId: ReviewUnitId): Promise<ScheduleState | null> {
      assertKnownReviewUnit(unitId);
      return schedules.get(unitId) ?? null;
    },
    async applyReview(
      unitId: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      nextScheduleState: ScheduleState,
    ): Promise<void> {
      assertKnownReviewUnit(unitId);
      if (unitId !== attempt.reviewUnitId) {
        throw new Error('Applied review unit must match the attempt review unit');
      }
      if (nextScheduleState.last_review !== attempt.occurredAt) {
        throw new Error('Schedule last_review must match the attempt timestamp');
      }

      assertAttemptContract(attempt);
      attempts.push(attempt);
      schedules.set(unitId, nextScheduleState);
    },
    async listQueueCandidates(): Promise<QueueCandidate[]> {
      return fixture.units.map((unit) => {
        const currentScheduleState = schedules.get(unit.reviewUnitId) ?? null;

        return {
          reviewUnitId: unit.reviewUnitId,
          scheduleState: currentScheduleState,
          due: currentScheduleState?.due ?? now - 60_000,
          ...unit.queue,
        };
      });
    },
  };

  return { attempts, schedules, store };
}

describe('memory service scenario fixtures', () => {
  test('runs a deterministic prompt loop through attempt recording, grading, and queueing', async () => {
    const credo = reviewUnitId('credo-opening');
    const pater = reviewUnitId('pater-noster-opening');
    const credoUnit: AuthoredReviewUnitFixture = {
      reviewUnitId: credo,
      promptId: 'credo-opening-en',
      prompt: {
        kind: 'shortAnswer',
        reviewUnitId: credo,
        prompt: 'What does Credo in unum Deum mean?',
        acceptedAnswers: ['I believe in one God'],
        equivalenceGroups: [],
        ignoredTokens: [],
      },
      queue: {
        progression: null,
        conceptKey: 'creed-opening',
        sourceKey: 'mass-core',
        domainKey: 'latin',
      },
    };
    const paterUnit: AuthoredReviewUnitFixture = {
      reviewUnitId: pater,
      promptId: 'pater-opening-en',
      prompt: {
        kind: 'shortAnswer',
        reviewUnitId: pater,
        prompt: 'What does Pater noster mean?',
        acceptedAnswers: ['Our Father'],
        equivalenceGroups: [],
        ignoredTokens: [],
      },
      queue: {
        progression: null,
        conceptKey: 'lords-prayer-opening',
        sourceKey: 'mass-core',
        domainKey: 'latin',
      },
    };
    const fixture: ScenarioFixture = {
      units: [credoUnit, paterUnit],
    };
    const { attempts, schedules, store } = createValidatingScenarioStore(fixture);
    const service = createMemoryService({ store, now: () => now, masteryPolicy });

    const note = await service.execute({
      kind: 'record-attempt',
      attempt: {
        reviewUnitId: credo,
        promptId: 'credo-opening-note',
        submittedAnswer: 'I hesitated on unum',
        responseTimeMs: 4_200,
      },
    });
    const review = await service.execute({
      kind: 'grade/apply-review',
      prompt: credoUnit.prompt,
      promptId: credoUnit.promptId,
      submittedAnswer: 'I believe in one God',
      responseTimeMs: 2_600,
    });
    const next = await service.execute({ kind: 'next-queue' });

    expect(note.attempt).toEqual({
      reviewUnitId: credo,
      promptId: 'credo-opening-note',
      submittedAnswer: 'I hesitated on unum',
      responseTimeMs: 4_200,
      occurredAt: now,
    });
    expect(review.grade).toMatchObject({
      verdict: 'correct',
      rating: Rating.Good,
      isCorrect: true,
      expectedAnswer: 'I believe in one God',
    });
    expect(review.scheduleState).toMatchObject({
      reps: 1,
      state: State.Learning,
      last_review: now,
    });
    expect(schedules.get(credo)).toEqual(review.scheduleState);
    expect(attempts).toEqual([note.attempt, review.attempt]);
    expect(next).toEqual({
      kind: 'queue-selected',
      candidate: {
        reviewUnitId: pater,
        scheduleState: null,
        due: now - 60_000,
        progression: null,
        conceptKey: 'lords-prayer-opening',
        sourceKey: 'mass-core',
        domainKey: 'latin',
      },
    });
  });

  test('uses persisted mastery to unlock the next progression stage in the queue', async () => {
    const memorization = reviewUnitId('st-michael-opening-memorization');
    const recitation = reviewUnitId('st-michael-opening-recitation');
    const memorizationUnit: AuthoredReviewUnitFixture = {
      reviewUnitId: memorization,
      promptId: 'st-michael-opening-mcq',
      prompt: {
        kind: 'mcq',
        reviewUnitId: memorization,
        prompt: 'Complete the opening: Saint Michael the Archangel, defend us...',
        choices: ['in battle', 'at dawn', 'with silence'],
        correctChoice: 'in battle',
      },
      queue: {
        progression: {
          progressionGroup: 'st-michael-prayer',
          stageOrder: 1,
          requires: [],
          supersedes: [],
        },
        conceptKey: 'st-michael-opening',
        sourceKey: 'prayer-core',
        domainKey: 'recitation',
      },
    };
    const recitationUnit: AuthoredReviewUnitFixture = {
      reviewUnitId: recitation,
      promptId: 'st-michael-opening-recitation',
      prompt: {
        kind: 'recitation',
        reviewUnitId: recitation,
        prompt: 'Recite the opening of the Saint Michael prayer.',
        acceptedAnswers: ['Saint Michael the Archangel, defend us in battle'],
        equivalenceGroups: [],
        ignoredTokens: [],
      },
      queue: {
        progression: {
          progressionGroup: 'st-michael-prayer',
          stageOrder: 2,
          requires: [memorization],
          supersedes: [],
        },
        conceptKey: 'st-michael-opening',
        sourceKey: 'prayer-core',
        domainKey: 'recitation',
      },
    };
    const fixture: ScenarioFixture = {
      units: [memorizationUnit, recitationUnit],
    };
    const { attempts, schedules, store } = createValidatingScenarioStore(fixture);
    const service = createMemoryService({ store, now: () => now, masteryPolicy });

    const locked = await service.execute({ kind: 'next-queue' });
    schedules.set(
      memorization,
      scheduleState({
        due: now + 86_400_000,
        reps: 3,
        scheduled_days: 2,
        state: State.Review,
        last_review: now - 86_400_000,
      }),
    );
    const unlocked = await service.execute({ kind: 'next-queue' });
    const review = await service.execute({
      kind: 'grade/apply-review',
      prompt: recitationUnit.prompt,
      promptId: recitationUnit.promptId,
      submittedAnswer: 'Saint Michael the Archangel, defend us in battle',
      responseTimeMs: 7_500,
    });

    expect(locked.candidate?.reviewUnitId).toBe(memorization);
    expect(unlocked.candidate?.reviewUnitId).toBe(recitation);
    expect(review.grade).toMatchObject({
      verdict: 'correct',
      rating: Rating.Good,
      isCorrect: true,
    });
    expect(review.scheduleState).toMatchObject({
      reps: 1,
      state: State.Learning,
      last_review: now,
    });
    expect(schedules.get(recitation)).toEqual(review.scheduleState);
    expect(attempts).toEqual([review.attempt]);
  });
});
