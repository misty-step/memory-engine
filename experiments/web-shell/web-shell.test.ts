import { describe, expect, test } from 'bun:test';

import type { ReviewUnitId } from 'memory-engine/types';

import { createWebShellSession, runWebShellFlow } from './index';

const now = Date.UTC(2026, 4, 15, 12, 0, 0);

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

describe('web shell dogfood client', () => {
  test('drives answer, reveal, scheduling, and queue transitions through the service boundary', async () => {
    const shell = createWebShellSession({ now: () => now });

    const initial = await shell.start();
    expect(initial).toMatchObject({
      fixture: 'latin-prayer-authored-v1',
      status: 'answering',
      current: {
        reviewUnitId: 'import-credo-in-unum-deum',
        prompt: 'Translate: Credo in unum Deum',
        expectedAnswer: null,
        grade: null,
        reviewState: { reps: 3, state: 2 },
      },
    });
    expect(initial.queue.map((candidate) => candidate.reviewUnitId)).toEqual([
      reviewUnitId('import-credo-in-unum-deum'),
      reviewUnitId('import-pater-noster'),
    ]);

    const revealed = shell.reveal();
    expect(revealed).toMatchObject({
      status: 'revealed',
      current: {
        expectedAnswer: 'I believe in one God',
        grade: null,
      },
    });
    expect(revealed.interfacePressure).toContain(
      'Reveal is UI-owned because the service has no first-class revealed review command.',
    );

    const reviewed = await shell.submitAnswer('I believe in one God', 2_400);
    expect(reviewed).toMatchObject({
      status: 'graded',
      attempts: 1,
      current: {
        grade: { verdict: 'correct', rating: 4 },
        reviewState: { reps: 4, state: 2 },
      },
    });
    expect(reviewed.commands).toEqual(['next-queue', 'reveal', 'grade/apply-review']);

    const next = await shell.next();
    expect(next).toMatchObject({
      status: 'answering',
      current: {
        reviewUnitId: 'import-pater-noster',
        prompt: 'Translate: Pater noster',
        expectedAnswer: null,
        grade: null,
        reviewState: null,
      },
    });
    expect(next.queue.map((candidate) => candidate.reviewUnitId)).toEqual([
      reviewUnitId('import-pater-noster'),
      reviewUnitId('import-credo-in-unum-deum'),
    ]);
  });

  test('emits a dogfood receipt for documentation and extraction review', async () => {
    const receipt = await runWebShellFlow({ now: () => now });

    expect(receipt).toMatchObject({
      fixture: 'latin-prayer-authored-v1',
      commands: ['next-queue', 'reveal', 'grade/apply-review', 'next-queue'],
      submittedAnswer: 'I believe in one God',
      gradedVerdict: 'correct',
      scheduledReps: 4,
      nextReviewUnitId: 'import-pater-noster',
      extractionRecommendation: 'keep experimenting',
    });
    expect(receipt.interfacePressure).toContain(
      'Review-state visibility needs a compact DTO; raw ScheduleState is too engine-shaped for UI copy.',
    );
  });
});
