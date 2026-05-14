import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { createMemoryService } from '../../service';
import type { MemoryServiceStore, ServiceAttemptRecord } from '../../service';
import type { QueueCandidate, ReviewUnitId, ScheduleState } from '../../src';

const now = Date.UTC(2026, 4, 14, 12, 0, 0);

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

describe('memory service persistence boundary', () => {
  test('keeps storage behind an injected repository contract', async () => {
    const calls: string[] = [];
    const writtenSchedules = new Map<ReviewUnitId, ScheduleState>();
    const store: MemoryServiceStore = {
      async recordAttempt(): Promise<void> {
        calls.push('recordAttempt');
      },
      async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
        calls.push(`readScheduleState:${reviewUnitId}`);
        return null;
      },
      async applyReview(
        reviewUnitId: ReviewUnitId,
        _attempt: ServiceAttemptRecord,
        schedule: ScheduleState,
      ): Promise<void> {
        calls.push(`applyReview:${reviewUnitId}`);
        writtenSchedules.set(reviewUnitId, schedule);
      },
      async listQueueCandidates(): Promise<QueueCandidate[]> {
        calls.push('listQueueCandidates');
        return [];
      },
    };

    const service = createMemoryService({
      now: () => now,
      masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
      store,
    });

    await service.execute({
      kind: 'grade/apply-review',
      prompt: {
        kind: 'boolean',
        reviewUnitId: reviewUnitId('truth-claim'),
        prompt: 'Is the scheduler state stored outside the kernel?',
        correctAnswer: true,
      },
      submittedAnswer: 'true',
      responseTimeMs: 1_200,
    });

    const queueResult = await service.execute({ kind: 'next-queue' });

    expect(queueResult).toEqual({ kind: 'queue-selected', candidate: null });
    expect(writtenSchedules.has(reviewUnitId('truth-claim'))).toBe(true);
    expect(calls).toEqual([
      'readScheduleState:truth-claim',
      'applyReview:truth-claim',
      'listQueueCandidates',
    ]);
  });
});
