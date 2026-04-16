import {
  type GradeCtx,
  type GradeResult,
  type Prompt,
  Rating,
  type ReviewUnitId,
  type ScheduleState,
} from '../src';
import type { Rating as ScheduleRating } from '../src/types';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function deterministicGrade(
  verdict: GradeResult['verdict'],
  rating: GradeResult['rating'],
  submittedAnswer: string,
  expectedAnswer: string,
  isCorrect: boolean,
): GradeResult {
  return {
    verdict,
    rating,
    isCorrect,
    submittedAnswer,
    expectedAnswer,
    graderKind: 'deterministic',
    graderModel: null,
    graderConfidence: null,
    feedback: '',
  };
}

export type GradingFixture = {
  name: string;
  prompt: Prompt;
  submitted: string;
  ctx: GradeCtx;
  expected: GradeResult;
};

export type SchedulerFixture = {
  name: string;
  initialState: ScheduleState | null;
  rating: ScheduleRating;
  now: number;
  expected: ScheduleState;
};

export const gradingFixtures: GradingFixture[] = [
  {
    name: 'mcq trims the submission and grades the correct choice',
    prompt: {
      kind: 'mcq',
      reviewUnitId: reviewUnitId('mcq-correct'),
      prompt: 'Pick one',
      choices: ['Alpha', 'Beta', 'Gamma'],
      correctChoice: 'Alpha',
    },
    submitted: ' Alpha ',
    ctx: { responseTimeMs: 5_100, priorReps: 0 },
    expected: deterministicGrade('correct', Rating.Good, 'Alpha', 'Alpha', true),
  },
  {
    name: 'mcq grades an incorrect choice as wrong',
    prompt: {
      kind: 'mcq',
      reviewUnitId: reviewUnitId('mcq-wrong'),
      prompt: 'Pick one',
      choices: ['Alpha', 'Beta', 'Gamma'],
      correctChoice: 'Alpha',
    },
    submitted: 'Beta',
    ctx: { responseTimeMs: 5_100, priorReps: 0 },
    expected: deterministicGrade('wrong', Rating.Again, 'Beta', 'Alpha', false),
  },
  {
    name: 'boolean uses normalized exact matching',
    prompt: {
      kind: 'boolean',
      reviewUnitId: reviewUnitId('boolean-correct'),
      prompt: 'Is this true?',
      correctAnswer: true,
    },
    submitted: ' true ',
    ctx: { responseTimeMs: 3_000, priorReps: 3 },
    expected: deterministicGrade('correct', Rating.Easy, 'true', 'True', true),
  },
  {
    name: 'short answer marks exact answers as good',
    prompt: {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitId('short-answer-good'),
      prompt: 'Answer?',
      acceptedAnswers: ['punishment'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submitted: 'Punishment',
    ctx: { responseTimeMs: 5_100, priorReps: 0 },
    expected: deterministicGrade('correct', Rating.Good, 'Punishment', 'punishment', true),
  },
  {
    name: 'short answer marks near misses as hard',
    prompt: {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitId('short-answer-close'),
      prompt: 'Answer?',
      acceptedAnswers: ['punishment'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submitted: 'punishmant',
    ctx: { responseTimeMs: 5_100, priorReps: 0 },
    expected: deterministicGrade('close', Rating.Hard, 'punishmant', 'punishment', false),
  },
  {
    name: 'cloze honors token equivalence groups',
    prompt: {
      kind: 'cloze',
      reviewUnitId: reviewUnitId('cloze-equivalence'),
      prompt: 'Respond.',
      acceptedAnswers: ['Glory to you, O Lord'],
      equivalenceGroups: [['o', 'oh']],
      ignoredTokens: [],
    },
    submitted: 'Glory to you oh lord',
    ctx: { responseTimeMs: 3_500, priorReps: 0 },
    expected: deterministicGrade(
      'correct',
      Rating.Good,
      'Glory to you oh lord',
      'Glory to you, O Lord',
      true,
    ),
  },
  {
    name: 'short answer evaluates near misses against each accepted answer independently',
    prompt: {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitId('short-answer-accepted-near-miss'),
      prompt: 'Answer?',
      acceptedAnswers: ['Q', 'Quebec'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submitted: 'Quebecc',
    ctx: { responseTimeMs: 5_100, priorReps: 0 },
    expected: deterministicGrade('close', Rating.Hard, 'Quebecc', 'Q / Quebec', false),
  },
];

export const schedulerFixtures: SchedulerFixture[] = [
  {
    name: 'new',
    initialState: {
      due: 0,
      stability: 0,
      difficulty: 0,
      elapsed_days: 0,
      scheduled_days: 0,
      reps: 0,
      lapses: 0,
      learning_steps: 0,
      state: 0,
      last_review: null,
    },
    rating: Rating.Good,
    now: 0,
    expected: {
      due: 600_000,
      stability: 2.3065,
      difficulty: 2.11810397,
      elapsed_days: 0,
      scheduled_days: 0,
      reps: 1,
      lapses: 0,
      learning_steps: 1,
      state: 1,
      last_review: 0,
    },
  },
  {
    name: 'learning',
    initialState: {
      due: 600_000,
      stability: 2.3065,
      difficulty: 2.11810397,
      elapsed_days: 0,
      scheduled_days: 0,
      reps: 1,
      lapses: 0,
      learning_steps: 1,
      state: 1,
      last_review: 0,
    },
    rating: Rating.Good,
    now: 600_000,
    expected: {
      due: 173_400_000,
      stability: 2.3065,
      difficulty: 2.11121424,
      elapsed_days: 0,
      scheduled_days: 2,
      learning_steps: 0,
      reps: 2,
      lapses: 0,
      state: 2,
      last_review: 600_000,
    },
  },
  {
    name: 'review',
    initialState: {
      due: 173_400_000,
      stability: 2.3065,
      difficulty: 2.11121424,
      elapsed_days: 0,
      scheduled_days: 2,
      reps: 2,
      lapses: 0,
      learning_steps: 0,
      state: 2,
      last_review: 600_000,
    },
    rating: Rating.Again,
    now: 173_400_000,
    expected: {
      due: 174_000_000,
      stability: 0.60770166,
      difficulty: 7.39223814,
      elapsed_days: 2,
      scheduled_days: 0,
      learning_steps: 0,
      reps: 3,
      lapses: 1,
      state: 3,
      last_review: 173_400_000,
    },
  },
  {
    name: 'relearning',
    initialState: {
      due: 174_000_000,
      stability: 0.60770166,
      difficulty: 7.39223814,
      elapsed_days: 2,
      scheduled_days: 0,
      learning_steps: 0,
      reps: 3,
      lapses: 1,
      state: 3,
      last_review: 173_400_000,
    },
    rating: Rating.Good,
    now: 174_000_000,
    expected: {
      due: 260_400_000,
      stability: 0.65979762,
      difficulty: 7.38007427,
      elapsed_days: 0,
      scheduled_days: 1,
      learning_steps: 0,
      reps: 4,
      lapses: 1,
      state: 2,
      last_review: 174_000_000,
    },
  },
];
