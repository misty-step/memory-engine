import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { StaticRubricGrader } from 'memory-engine/adapters';
import { AsyncGrader, Grader } from 'memory-engine/grading';
import {
  filterEligibleCandidates,
  filterEligibleCandidatesWithFallback,
  isMastered,
} from 'memory-engine/progression';
import {
  compareQueuePriority,
  pickNextQueueCandidate,
  reviewableQueueCandidates,
} from 'memory-engine/queue';
import { next } from 'memory-engine/scheduling';
import { gradingFixtures } from 'memory-engine/testkit';
import {
  type QueueCandidate,
  Rating,
  type ReviewUnitId,
  type ScheduleState,
  type ShortAnswerPrompt,
} from 'memory-engine/types';

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

describe('modular package exports', () => {
  test('grading subpath exposes deterministic and async grading surfaces', async () => {
    const reviewUnitIdValue = reviewUnitId('api-grading');
    const prompt: ShortAnswerPrompt = {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitIdValue,
      prompt: 'Translate poena.',
      acceptedAnswers: ['punishment'],
      equivalenceGroups: [],
      ignoredTokens: [],
    };

    const grade = new Grader().grade(prompt, 'Punishment', {
      responseTimeMs: 1_200,
      priorReps: 0,
    });
    const asyncGrade = await new AsyncGrader().grade(prompt, 'Punishment', {
      responseTimeMs: 1_200,
      priorReps: 0,
    });

    expect(grade).toMatchObject({
      verdict: 'correct',
      rating: Rating.Good,
      isCorrect: true,
    });
    expect(asyncGrade).toEqual(grade);
  });

  test('scheduling subpath exposes the scheduler without the root barrel', () => {
    const nextState = next(null, Rating.Good, now);

    expect(nextState).toMatchObject({
      reps: 1,
      state: State.Learning,
      last_review: now,
    });
  });

  test('progression and queue subpaths compose with public types', () => {
    const mastered = scheduleState({
      state: State.Review,
      reps: 3,
      scheduled_days: 2,
      due: now + 86_400_000,
      last_review: now - 86_400_000,
    });
    const prerequisite = reviewUnitId('api-prerequisite');
    const advanced = reviewUnitId('api-advanced');
    const masteryPolicy = (schedule: ScheduleState) =>
      schedule.state === State.Review && schedule.reps >= 3;
    const candidates: QueueCandidate[] = [
      {
        reviewUnitId: prerequisite,
        scheduleState: mastered,
        due: mastered.due,
        progression: {
          progressionGroup: 'api',
          stageOrder: 1,
          requires: [],
          supersedes: [],
        },
        conceptKey: 'api',
        sourceKey: 'source',
        domainKey: 'domain',
      },
      {
        reviewUnitId: advanced,
        scheduleState: null,
        due: now - 60_000,
        progression: {
          progressionGroup: 'api',
          stageOrder: 2,
          requires: [prerequisite],
          supersedes: [],
        },
        conceptKey: 'api',
        sourceKey: 'source',
        domainKey: 'domain',
      },
    ];

    expect(isMastered(mastered, masteryPolicy)).toBe(true);
    expect(
      filterEligibleCandidates(
        candidates.map((candidate) => ({
          reviewUnitId: candidate.reviewUnitId,
          review: candidate.scheduleState,
          progression: candidate.progression,
        })),
        masteryPolicy,
      ).available.map((candidate) => candidate.reviewUnitId),
    ).toEqual([prerequisite, advanced]);
    expect(
      filterEligibleCandidatesWithFallback(
        candidates.map((candidate) => ({
          reviewUnitId: candidate.reviewUnitId,
          review: candidate.scheduleState,
          progression: candidate.progression,
        })),
        masteryPolicy,
      ).lockedFreshCount,
    ).toBe(0);
    const prerequisiteCandidate = candidates[0];
    const advancedCandidate = candidates[1];

    if (prerequisiteCandidate === undefined || advancedCandidate === undefined) {
      throw new Error('Expected test fixture to define prerequisite and advanced candidates');
    }

    expect(reviewableQueueCandidates(candidates, masteryPolicy, { now })).toEqual([
      advancedCandidate,
    ]);
    expect(pickNextQueueCandidate(candidates, masteryPolicy, { now })).toEqual(advancedCandidate);
    expect(compareQueuePriority(prerequisiteCandidate, advancedCandidate, now)).toBeLessThan(0);
  });

  test('adapters and testkit remain dedicated subpaths', async () => {
    const rubricGrader = new StaticRubricGrader({
      model: 'fixture',
      confidence: 1,
      feedback: 'Fixture assessment.',
      criterionResults: [],
    });

    await expect(
      rubricGrader.grade(
        {
          kind: 'rubric',
          reviewUnitId: reviewUnitId('api-rubric'),
          prompt: 'Explain it.',
          rubric: {
            answerGuide: ['A clear explanation.'],
            passingScore: 0,
            criteria: [],
          },
        },
        'Answer',
      ),
    ).resolves.toMatchObject({ model: 'fixture' });
    expect(gradingFixtures.length).toBeGreaterThan(0);
  });
});
