import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';

import { createBetaStudySession } from './index';

const now = Date.UTC(2026, 4, 22, 12, 0, 0);

async function withTempStudy<T>(run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), 'memory-engine-beta-study-'));
  try {
    return await run(join(directory, 'study.json'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

function sourceBody(): string {
  return [
    'Concept: NATO letter A',
    'Activity: quiz',
    'Stage: recognition-3',
    'Question: What is the NATO phonetic alphabet word for A?',
    'Answer: ALFA',
    'Distractors: BRAVO, CHARLIE',
    'Reference: The NATO phonetic alphabet word for A is ALFA.',
    '',
    'Concept: NATO CAT composition',
    'Activity: exercise',
    'Stage: composition',
    'Question: Spell CAT over the phone using the NATO phonetic alphabet.',
    'Answer: CHARLIE ALFA TANGO',
    'Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.',
    'Reference: C is CHARLIE. A is ALFA. T is TANGO.',
  ].join('\n');
}

describe('mobile beta study interface session', () => {
  test('creates source material, approves quiz and exercise drafts, reviews, reveals, and advances queue', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });

      expect(await study.start()).toMatchObject({
        status: 'drafting',
        summary: { sourceCount: 0, attemptCount: 0 },
      });

      const sourced = await study.addSource({
        id: 'src-nato',
        title: 'NATO practice notes',
        body: sourceBody(),
      });
      expect(sourced.summary.sourceCount).toBe(1);

      const generated = await study.generate();
      expect(generated.drafts).toMatchObject([
        {
          id: 'study-run-1-draft-src-nato-1-nato-letter-a',
          activityKind: 'quiz',
          validationStatus: 'accepted',
        },
        {
          id: 'study-run-1-draft-src-nato-2-nato-cat-composition',
          activityKind: 'exercise',
          validationStatus: 'accepted',
          workedSolution: 'C is CHARLIE, A is ALFA, and T is TANGO.',
        },
      ]);

      await study.approveDraft('study-run-1-draft-src-nato-2-nato-cat-composition');
      const approved = await study.approveDraft('study-run-1-draft-src-nato-1-nato-letter-a');
      expect(approved).toMatchObject({
        status: 'answering',
        current: {
          prompt: 'Spell CAT over the phone using the NATO phonetic alphabet.',
          activityKind: 'exercise',
          expectedAnswer: null,
          reviewState: null,
        },
        summary: { approvedReviewUnitCount: 2 },
      });

      const revealed = await study.reveal();
      expect(revealed.current).toMatchObject({
        expectedAnswer: 'CHARLIE ALFA TANGO',
        workedSolution: 'C is CHARLIE, A is ALFA, and T is TANGO.',
      });

      const reviewed = await study.submitAnswer('CHARLIE ALFA TANGO', 4_200);
      expect(reviewed).toMatchObject({
        status: 'graded',
        current: {
          grade: { verdict: 'correct', isCorrect: true },
          reviewState: { reps: 1, state: 1, last_review: now },
          scheduleChange: {
            before: null,
            after: { reps: 1, state: 1, last_review: now },
          },
        },
        summary: { attemptCount: 1, lastOutcome: 'correct' },
      });

      const next = await study.next();
      expect(next).toMatchObject({
        status: 'answering',
        current: {
          prompt: 'What is the NATO phonetic alphabet word for A?',
          activityKind: 'quiz',
          expectedAnswer: null,
        },
      });
      expect(next.queue.map((row) => row.activityKind)).toEqual(['quiz', 'exercise']);
    });
  });

  test('resumes from persisted state after a saved review without regenerating content', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });
      await study.addSource({ id: 'src-nato', title: 'NATO practice notes', body: sourceBody() });
      await study.generate();
      await study.approveDraft('study-run-1-draft-src-nato-2-nato-cat-composition');
      await study.approveDraft('study-run-1-draft-src-nato-1-nato-letter-a');
      await study.submitAnswer('CHARLIE ALFA TANGO', 4_200);

      const resumed = await createBetaStudySession({ path, now: () => now + 1_000 });
      const view = await resumed.start();

      expect(view.summary).toMatchObject({
        sourceCount: 1,
        acceptedDraftCount: 2,
        approvedReviewUnitCount: 2,
        attemptCount: 1,
        lastOutcome: 'correct',
      });
      expect(view.drafts).toHaveLength(2);
      expect(view.current).toMatchObject({
        prompt: 'What is the NATO phonetic alphabet word for A?',
        reviewState: null,
      });
      expect(view.queue.find((row) => row.activityKind === 'exercise')).toMatchObject({
        reps: 1,
        state: 1,
      });
    });
  });

  test('ignores a duplicate submit after grading without double-counting attempts or schedule history', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });
      await study.addSource({ id: 'src-nato', title: 'NATO practice notes', body: sourceBody() });
      await study.generate();
      await study.approveDraft('study-run-1-draft-src-nato-1-nato-letter-a');

      const first = await study.submitAnswer('ALFA', 1_800);
      const duplicate = await study.submitAnswer('ALFA', 1_800);

      expect(duplicate.summary.attemptCount).toBe(1);
      expect(duplicate.current?.reviewState).toEqual(first.current?.reviewState);
      expect(duplicate.current?.scheduleChange).toEqual(first.current?.scheduleChange);

      const revealedAfterGrade = await study.reveal();
      const afterRevealSubmit = await study.submitAnswer('ALFA', 1_800);

      expect(revealedAfterGrade.status).toBe('graded');
      expect(afterRevealSubmit.summary.attemptCount).toBe(1);
      expect(afterRevealSubmit.current?.reviewState).toEqual(first.current?.reviewState);
    });
  });
});
