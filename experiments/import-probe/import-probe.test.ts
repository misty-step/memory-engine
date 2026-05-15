import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';
import type { ReviewUnitId, ScheduleState } from '../../src';
import { compileAuthoredFixture, latinPrayerFixture } from './index';

const now = Date.UTC(2026, 4, 15, 12, 0, 0);

function createFixtureStore(compiled: ReturnType<typeof compileAuthoredFixture>): {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const attempts: ServiceAttemptRecord[] = [];
  const prompts = new Map(compiled.prompts.map((prompt) => [prompt.reviewUnitId, prompt]));
  const queueById = new Map(compiled.queue.map((candidate) => [candidate.reviewUnitId, candidate]));
  const schedules = new Map(
    compiled.schedules.map((schedule) => [schedule.reviewUnitId, schedule.state]),
  );

  function assertKnown(reviewUnitId: ReviewUnitId): void {
    if (!prompts.has(reviewUnitId) || !queueById.has(reviewUnitId)) {
      throw new Error(`Unknown review unit: ${reviewUnitId}`);
    }
  }

  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      assertKnown(attempt.reviewUnitId);
      attempts.push(attempt);
    },
    async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
      assertKnown(reviewUnitId);
      return schedules.get(reviewUnitId) ?? null;
    },
    async applyReview(
      reviewUnitId: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      nextScheduleState: ScheduleState,
    ): Promise<void> {
      assertKnown(reviewUnitId);
      attempts.push(attempt);
      schedules.set(reviewUnitId, nextScheduleState);
    },
    async listQueueCandidates() {
      return compiled.queue.map((candidate) => {
        const scheduleState = schedules.get(candidate.reviewUnitId) ?? null;

        return {
          ...candidate,
          scheduleState,
          due: scheduleState?.due ?? candidate.due,
        };
      });
    },
  };

  return { attempts, schedules, store };
}

describe('import probe', () => {
  test('compiles authored material into canonical API inputs for the service loop', async () => {
    const compiled = compileAuthoredFixture(latinPrayerFixture, now);

    expect(compiled).toMatchObject({
      fixture: 'latin-prayer-authored-v1',
      productOwnedFields: ['sourceText', 'translation', 'confidencePrompt', 'notes'],
      apiGap: null,
    });
    expect(compiled.prompts).toHaveLength(2);
    expect(compiled.queue).toHaveLength(2);
    expect(compiled.schedules).toHaveLength(1);
    expect(compiled.prompts[0]).toMatchObject({
      kind: 'shortAnswer',
      prompt: 'Translate: Credo in unum Deum',
      acceptedAnswers: ['I believe in one God'],
      equivalenceGroups: [['God', 'god']],
      ignoredTokens: ['.', ',', ';', ':'],
    });
    expect(compiled.queue[0]).toMatchObject({
      conceptKey: 'creed-opening',
      sourceKey: 'mass-ordinary',
      domainKey: 'latin',
      due: now - 60_000,
    });
    expect(compiled.schedules[0]?.state).toMatchObject({
      state: State.Review,
      reps: 3,
      last_review: now - 86_400_000,
    });

    const firstPrompt = compiled.prompts[0];
    if (firstPrompt === undefined) {
      throw new Error('fixture must compile at least one prompt');
    }

    const { attempts, schedules, store } = createFixtureStore(compiled);
    const service = createMemoryService({
      store,
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 4,
    });

    const review = await service.execute({
      kind: 'grade/apply-review',
      prompt: firstPrompt,
      promptId: compiled.promptIds.get(firstPrompt.reviewUnitId) ?? null,
      submittedAnswer: 'I believe in one God',
      responseTimeMs: 2_400,
    });
    const next = await service.execute({ kind: 'next-queue' });

    expect(review.grade.verdict).toBe('correct');
    expect(attempts).toHaveLength(1);
    expect(schedules.get(firstPrompt.reviewUnitId)).toEqual(review.scheduleState);
    expect(next.candidate?.reviewUnitId).toBe(compiled.prompts[1]?.reviewUnitId);
  });
});
