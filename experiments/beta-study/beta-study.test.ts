import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';

import type { ReviewUnitId, ScheduleState } from '../../src';
import { runBetaGeneration } from '../beta-generation';
import { createBetaPersistenceStore } from '../beta-store';
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

function quizOnlySourceBody(): string {
  return [
    'Concept: NATO letter A',
    'Activity: quiz',
    'Stage: recognition-3',
    'Question: What is the NATO phonetic alphabet word for A?',
    'Answer: ALFA',
    'Distractors: BRAVO, CHARLIE',
    'Reference: The NATO phonetic alphabet word for A is ALFA.',
  ].join('\n');
}

function ladderSourceBody(): string {
  return [
    'Concept: NATO phonetic alphabet',
    'Activity: quiz',
    'Stage: recognition-3',
    'Question: A is which NATO phonetic alphabet word?',
    'Answer: ALFA',
    'Distractors: BRAVO, CHARLIE',
    'Reference: A is ALFA in the NATO phonetic alphabet.',
    '',
    'Concept: NATO phonetic alphabet',
    'Activity: quiz',
    'Stage: recognition-5',
    'Question: A is which NATO phonetic alphabet word among these five choices?',
    'Answer: ALFA',
    'Distractors: BRAVO, CHARLIE, DELTA, ECHO',
    'Reference: A is ALFA in the NATO phonetic alphabet.',
    '',
    'Concept: NATO phonetic alphabet',
    'Activity: quiz',
    'Stage: typed-recall',
    'Question: Type the NATO phonetic alphabet word for A.',
    'Answer: ALFA',
    'Reference: A is ALFA in the NATO phonetic alphabet.',
    '',
    'Concept: NATO phonetic alphabet',
    'Activity: exercise',
    'Stage: composition',
    'Question: Spell CAT over the phone using the NATO phonetic alphabet.',
    'Answer: CHARLIE ALFA TANGO',
    'Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.',
    'Scoring Rubric: Pass only when each letter is converted to the matching NATO word in order.',
    'Reference: C is CHARLIE. A is ALFA. T is TANGO.',
  ].join('\n');
}

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function masteredSchedule(overrides: Partial<ScheduleState> = {}): ScheduleState {
  return {
    due: now + 86_400_000,
    stability: 4.2,
    difficulty: 3.1,
    elapsed_days: 1,
    scheduled_days: 1,
    reps: 3,
    lapses: 0,
    state: 2,
    last_review: now - 86_400_000,
    ...overrides,
  };
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

      const firstApproval = await study.approveDraft(
        'study-run-1-draft-src-nato-2-nato-cat-composition',
      );
      expect(firstApproval).toMatchObject({
        status: 'drafting',
        current: null,
        drafts: [
          { id: 'study-run-1-draft-src-nato-1-nato-letter-a', approved: false },
          { id: 'study-run-1-draft-src-nato-2-nato-cat-composition', approved: true },
        ],
      });

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
      await study.addSource({
        id: 'src-nato',
        title: 'NATO practice notes',
        body: quizOnlySourceBody(),
      });
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

  test('surfaces generation failures for prose that cannot produce source-backed drafts', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });
      await study.addSource({
        id: 'src-vague',
        title: 'Vague notes',
        body: 'Remember this later maybe somehow.',
      });

      const generated = await study.generate();

      expect(generated).toMatchObject({
        status: 'drafting',
        drafts: [],
        generationFailures: [
          'src-vague: arbitrary text did not contain enough source-backed facts to generate practice',
        ],
        summary: {
          generationFailureCount: 1,
          approvedReviewUnitCount: 0,
        },
      });
    });
  });

  test('supports one-at-a-time keep, skip, and edit decisions before study starts', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });
      await study.addSource({
        id: 'src-greek',
        title: 'Greek notes',
        body: [
          'Alpha is the first Greek letter.',
          'Beta is the second Greek letter.',
          'Gamma measures the rate of change of Delta.',
        ].join(' '),
      });

      const generated = await study.generate();
      expect(generated.drafts[0]).toMatchObject({
        id: 'study-run-1-draft-src-greek-1-alpha',
        prompt: 'According to Greek notes, what is Alpha?',
        expectedAnswer: 'first Greek letter',
        referenceText: 'Alpha is the first Greek letter.',
        approved: false,
      });

      const revised = await study.reviseDraft({
        draftId: 'study-run-1-draft-src-greek-1-alpha',
        prompt: 'Which Greek letter starts the sequence?',
        expectedAnswer: 'Alpha',
      });
      expect(revised.drafts[0]).toMatchObject({
        prompt: 'Which Greek letter starts the sequence?',
        expectedAnswer: 'Alpha',
        approved: false,
      });

      const kept = await study.approveDraft('study-run-1-draft-src-greek-1-alpha');
      expect(kept).toMatchObject({
        status: 'drafting',
        summary: { approvedReviewUnitCount: 1 },
      });

      const skipped = await study.rejectDraft('study-run-1-draft-src-greek-2-beta');
      expect(skipped.drafts).toContainEqual(
        expect.objectContaining({
          id: 'study-run-1-draft-src-greek-2-beta',
          validationStatus: 'rejected',
          validationReasons: ['Skipped by learner'],
        }),
      );
      expect(skipped.queue.map((row) => row.reviewUnitId)).toEqual([
        reviewUnitId('generated-quiz-src-greek-1-alpha'),
      ]);

      const started = await study.next();
      expect(started).toMatchObject({
        status: 'answering',
        current: {
          prompt: 'Which Greek letter starts the sequence?',
          expectedAnswer: null,
        },
      });
    });
  });

  test('keeps an entire accepted NATO alphabet draft set in one action', async () => {
    await withTempStudy(async (path) => {
      const study = await createBetaStudySession({ path, now: () => now });
      await study.addSource({
        id: 'src-nato-alphabet',
        title: 'NATO phonetic alphabet',
        body: [
          'I want to learn the NATO phonetic alphabet.',
          'A Alfa, B Bravo, C Charlie, D Delta, E Echo, F Foxtrot, G Golf, H Hotel.',
          'I India, J Juliett, K Kilo, L Lima, M Mike, N November, O Oscar.',
          'P Papa, Q Quebec, R Romeo, S Sierra, T Tango, U Uniform.',
          'V Victor, W Whiskey, X X-ray, Y Yankee, Z Zulu.',
        ].join(' '),
      });

      const generated = await study.generate();
      expect(generated.summary.acceptedDraftCount).toBe(27);
      expect(generated.summary.approvedReviewUnitCount).toBe(0);
      expect(generated.drafts.filter((draft) => draft.activityKind === 'quiz')).toHaveLength(26);

      const kept = await study.approveAllDrafts();

      expect(kept.summary.approvedReviewUnitCount).toBe(27);
      expect(kept.status).toBe('answering');
      expect(kept.current).toMatchObject({
        prompt: 'According to NATO phonetic alphabet, what is A?',
        expectedAnswer: null,
      });
      expect(kept.queue).toHaveLength(27);
    });
  });

  test('unlocks a shared-concept ladder without copying schedule history and records typed recall plus exercise attempts through the service', async () => {
    await withTempStudy(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-nato-ladder',
        kind: 'text',
        title: 'NATO ladder notes',
        body: ladderSourceBody(),
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });
      await runBetaGeneration(store, {
        runId: 'run-nato-ladder',
        sourceDocumentIds: ['src-nato-ladder'],
        startedAt: now,
        completedAt: now,
        defaultDue: now - 60_000,
      });

      const recognition3 = reviewUnitId('generated-quiz-src-nato-ladder-1-nato-phonetic-alphabet');
      const recognition5 = reviewUnitId('generated-quiz-src-nato-ladder-2-nato-phonetic-alphabet');
      const typedRecall = reviewUnitId('generated-quiz-src-nato-ladder-3-nato-phonetic-alphabet');
      const composition = reviewUnitId(
        'generated-exercise-src-nato-ladder-4-nato-phonetic-alphabet',
      );

      await store.approveGeneratedPromptDraft(
        'run-nato-ladder-draft-src-nato-ladder-1-nato-phonetic-alphabet',
        { initialScheduleState: masteredSchedule() },
      );
      await store.approveGeneratedPromptDraft(
        'run-nato-ladder-draft-src-nato-ladder-2-nato-phonetic-alphabet',
        { initialScheduleState: masteredSchedule({ reps: 4 }) },
      );
      await store.approveGeneratedPromptDraft(
        'run-nato-ladder-draft-src-nato-ladder-3-nato-phonetic-alphabet',
      );
      await store.approveGeneratedPromptDraft(
        'run-nato-ladder-draft-src-nato-ladder-4-nato-phonetic-alphabet',
      );

      const study = await createBetaStudySession({ path, now: () => now });
      const typed = await study.start();
      expect(typed).toMatchObject({
        status: 'answering',
        current: {
          reviewUnitId: typedRecall,
          activityKind: 'quiz',
          activityStage: 'typed-recall',
          prompt: 'Type the NATO phonetic alphabet word for A.',
          reviewState: null,
        },
      });
      expect(typed.queue.map((row) => [row.reviewUnitId, row.conceptKey])).toEqual([
        [typedRecall, 'nato-phonetic-alphabet'],
        [composition, 'nato-phonetic-alphabet'],
        [recognition3, 'nato-phonetic-alphabet'],
        [recognition5, 'nato-phonetic-alphabet'],
      ]);

      const typedReview = await study.submitAnswer('ALFA', 1_900);
      expect(typedReview).toMatchObject({
        status: 'graded',
        current: {
          grade: { verdict: 'correct', isCorrect: true },
          scheduleChange: {
            before: null,
            after: { reps: 1, state: 1, last_review: now },
          },
        },
      });

      const progressionStore = await createBetaPersistenceStore(path);
      await progressionStore.setScheduleState(typedRecall, masteredSchedule({ reps: 5 }));
      const resumed = await createBetaStudySession({ path, now: () => now });
      const exercise = await resumed.start();
      expect(exercise).toMatchObject({
        status: 'answering',
        current: {
          reviewUnitId: composition,
          activityKind: 'exercise',
          activityStage: 'composition',
          workedSolution: null,
          reviewState: null,
        },
      });

      const revealed = await resumed.reveal();
      expect(revealed.current).toMatchObject({
        expectedAnswer: 'CHARLIE ALFA TANGO',
        workedSolution: 'C is CHARLIE, A is ALFA, and T is TANGO.',
        scoringRubric:
          'Pass only when each letter is converted to the matching NATO word in order.',
      });

      const exerciseReview = await resumed.submitAnswer('CHARLIE ALFA TANGO', 5_100);
      expect(exerciseReview).toMatchObject({
        status: 'graded',
        current: {
          grade: { verdict: 'correct', isCorrect: true },
          scheduleChange: {
            before: null,
            after: { reps: 1, state: 1, last_review: now },
          },
        },
        summary: { attemptCount: 2 },
      });

      const snapshot = (await createBetaPersistenceStore(path)).snapshot();
      expect(snapshot.schedules.map((record) => [record.reviewUnitId, record.state.reps])).toEqual([
        [recognition3, 3],
        [recognition5, 4],
        [typedRecall, 5],
        [composition, 1],
      ]);
      expect(snapshot.attempts.map((attempt) => [attempt.reviewUnitId, attempt.promptId])).toEqual([
        [typedRecall, `${typedRecall}-prompt`],
        [composition, `${composition}-prompt`],
      ]);
      expect(snapshot.appliedReviews.map((receipt) => receipt.expectedPriorScheduleState)).toEqual([
        null,
        null,
      ]);
    });
  });
});
