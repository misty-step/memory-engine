import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';

import { createBetaStudySession } from './index';
import { createBetaCoachSession } from './multi-client';

const now = Date.UTC(2026, 4, 24, 12, 0, 0);

async function withTempStudy<T>(run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), 'memory-engine-beta-multi-client-'));
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

async function seedStudy(path: string): Promise<void> {
  const study = await createBetaStudySession({ path, now: () => now });
  await study.start();
  await study.addSource({ id: 'src-nato', title: 'NATO practice notes', body: sourceBody() });
  const generated = await study.generate();
  for (const draft of generated.drafts) {
    if (draft.validationStatus === 'accepted') {
      await study.approveDraft(draft.id);
    }
  }
}

describe('multi-client beta pressure', () => {
  test('runs a second independent beta workflow through ingest, approval, queue, reveal, submit, and restart/resume', async () => {
    await withTempStudy(async (path) => {
      const coach = await createBetaCoachSession({ path, now: () => now });

      expect(await coach.start()).toMatchObject({
        status: 'drafting',
        summary: { sourceCount: 0, attemptCount: 0 },
      });

      await coach.ingestSource({
        id: 'src-nato',
        title: 'NATO practice notes',
        body: sourceBody(),
      });
      const generated = await coach.generate();
      expect(generated.drafts).toHaveLength(2);

      const approved = await coach.approveAcceptedDrafts();
      expect(approved).toMatchObject({
        status: 'answering',
        summary: { approvedReviewUnitCount: 2 },
      });

      const revealed = await coach.reveal();
      expect(revealed).toMatchObject({
        status: 'revealed',
        summary: { attemptCount: 0 },
      });
      expect(revealed.current?.expectedAnswer).not.toBeNull();

      const reviewed = await coach.submitAnswer(revealed.current?.expectedAnswer ?? 'ALFA', 2_400);
      expect(reviewed).toMatchObject({
        status: 'graded',
        summary: { attemptCount: 1, lastOutcome: 'correct' },
        current: {
          grade: { verdict: 'correct', isCorrect: true },
          reviewState: { reps: 1, state: 1, last_review: now },
        },
      });

      const resumed = await createBetaCoachSession({ path, now: () => now + 1_000 });
      const resumedView = await resumed.start();
      expect(resumedView.summary).toMatchObject({
        sourceCount: 1,
        acceptedDraftCount: 2,
        approvedReviewUnitCount: 2,
        attemptCount: 1,
        lastOutcome: 'correct',
      });
      expect(resumedView.current).not.toBeNull();
    });
  });

  test('keeps reveal semantics, duplicate-submit handling, and review-state projection aligned across both clients', async () => {
    await withTempStudy(async (studyPath) => {
      await seedStudy(studyPath);
      const study = await createBetaStudySession({ path: studyPath, now: () => now });
      await study.start();
      const studyRevealed = await study.reveal();
      const studyExpected = studyRevealed.current?.expectedAnswer ?? 'ALFA';
      const studyReviewed = await study.submitAnswer(studyExpected, 2_100);
      const studyDuplicate = await study.submitAnswer(studyExpected, 2_100);
      const studyRevealAfterGrade = await study.reveal();

      await withTempStudy(async (coachPath) => {
        const coach = await createBetaCoachSession({ path: coachPath, now: () => now });
        await coach.start();
        await coach.ingestSource({
          id: 'src-nato',
          title: 'NATO practice notes',
          body: sourceBody(),
        });
        await coach.generate();
        await coach.approveAcceptedDrafts();
        const coachRevealed = await coach.reveal();
        const coachExpected = coachRevealed.current?.expectedAnswer ?? 'ALFA';
        const coachReviewed = await coach.submitAnswer(coachExpected, 2_100);
        const coachDuplicate = await coach.submitAnswer(coachExpected, 2_100);
        const coachRevealAfterGrade = await coach.reveal();

        expect(studyRevealed.summary.attemptCount).toBe(0);
        expect(coachRevealed.summary.attemptCount).toBe(0);
        expect(studyRevealed.current?.expectedAnswer).toBe(coachRevealed.current?.expectedAnswer);

        expect(studyDuplicate.summary.attemptCount).toBe(1);
        expect(coachDuplicate.summary.attemptCount).toBe(1);
        expect(coachDuplicate.current?.reviewState).toEqual(coachReviewed.current?.reviewState);
        expect(studyDuplicate.current?.reviewState).toEqual(studyReviewed.current?.reviewState);

        expect(studyReviewed.current?.reviewState).toEqual(coachReviewed.current?.reviewState);
        expect(studyReviewed.current?.scheduleChange?.after).toEqual(
          coachReviewed.current?.scheduleChange?.after,
        );

        expect(studyRevealAfterGrade.status).toBe('graded');
        expect(coachRevealAfterGrade.status).toBe('graded');
      });
    });
  });
});
