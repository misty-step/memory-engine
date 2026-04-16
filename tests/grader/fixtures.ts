import { type GradeCtx, type GradeResult, type Prompt, Rating, type ReviewUnitId } from '../../src';

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
