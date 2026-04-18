import {
  type GradeCtx,
  type GradeResult,
  type ProgressionCandidate,
  type Prompt,
  type QueueCandidate,
  Rating,
  type RecitationPrompt,
  type ReviewUnitId,
  type ScheduleState,
} from '../src';
import type { Rating as ScheduleRating } from '../src/types';

type ProgressionFixtureReview = {
  reps: number;
  state: number;
};

export type Slice2MasteryPolicy = 'ruminatio' | 'vault';

export type GradingFixture = {
  name: string;
  prompt: Prompt;
  submitted: string;
  ctx: GradeCtx;
  expected: GradeResult;
};

export type RecitationFixture = GradingFixture & {
  prompt: RecitationPrompt;
};

export type SchedulerFixture = {
  name: string;
  initialState: ScheduleState | null;
  rating: ScheduleRating;
  now: number;
  expected: ScheduleState;
};

export type ProgressionFixture = {
  name: string;
  mode: 'strict' | 'fallback';
  masteryPolicy: Slice2MasteryPolicy;
  candidates: ProgressionCandidate<ProgressionFixtureReview>[];
  population?: ProgressionCandidate<ProgressionFixtureReview>[];
  expectedAvailableReviewUnitIds: ReviewUnitId[];
  expectedLockedFreshCount: number;
};

export type QueueFixture = {
  name: string;
  masteryPolicy: Extract<Slice2MasteryPolicy, 'vault'>;
  candidates: QueueCandidate[];
  now: number;
  recentCandidates?: QueueCandidate[];
  population?: QueueCandidate[];
  recentSourceWindow?: number;
  expectedNextReviewUnitId: ReviewUnitId | null;
};

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
    criterionResults: [],
  };
}

function scheduleState(overrides: Partial<ScheduleState> = {}): ScheduleState {
  return {
    due: 0,
    stability: 0,
    difficulty: 0,
    elapsed_days: 0,
    scheduled_days: 0,
    reps: 0,
    lapses: 0,
    state: 0,
    last_review: null,
    ...overrides,
  };
}

function progressionCandidate(
  overrides: Partial<ProgressionCandidate<ProgressionFixtureReview>> &
    Pick<ProgressionCandidate<ProgressionFixtureReview>, 'reviewUnitId'>,
): ProgressionCandidate<ProgressionFixtureReview> {
  return {
    reviewUnitId: overrides.reviewUnitId,
    review: overrides.review ?? null,
    progression: overrides.progression ?? {
      progressionGroup: null,
      stageOrder: 1,
      requires: [],
      supersedes: [],
    },
  };
}

function queueCandidate(
  overrides: Partial<QueueCandidate> & Pick<QueueCandidate, 'reviewUnitId'>,
): QueueCandidate {
  const currentScheduleState = overrides.scheduleState ?? null;

  return {
    reviewUnitId: overrides.reviewUnitId,
    scheduleState: currentScheduleState,
    due: overrides.due ?? currentScheduleState?.due ?? 0,
    progression: overrides.progression ?? null,
    conceptKey: overrides.conceptKey ?? null,
    sourceKey: overrides.sourceKey ?? null,
    domainKey: overrides.domainKey ?? null,
  };
}

const baseGradingFixtures: GradingFixture[] = [
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

export const recitationFixtures: RecitationFixture[] = [
  {
    name: 'recitation grades long-form deterministic recall from the Ruminatio study oracle',
    prompt: {
      kind: 'recitation',
      reviewUnitId: reviewUnitId('recitation-glory-be'),
      prompt: 'Recite the prayer.',
      acceptedAnswers: ['Glory be to the Father, and to the Son, and to the Holy Spirit.'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submitted: 'Glory be to the Father and to the Son and to the Holy Spirit',
    ctx: { responseTimeMs: 3_200, priorReps: 1 },
    expected: deterministicGrade(
      'correct',
      Rating.Good,
      'Glory be to the Father and to the Son and to the Holy Spirit',
      'Glory be to the Father, and to the Son, and to the Holy Spirit.',
      true,
    ),
  },
];

export const gradingFixtures: GradingFixture[] = [...baseGradingFixtures, ...recitationFixtures];

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
      reps: 4,
      lapses: 1,
      state: 2,
      last_review: 174_000_000,
    },
  },
];

export const progressionFixtures: ProgressionFixture[] = [
  {
    name: 'progression unlocks a later stage using the wider population',
    mode: 'strict',
    masteryPolicy: 'ruminatio',
    candidates: [
      progressionCandidate({
        reviewUnitId: reviewUnitId('a-stage-2'),
        progression: {
          progressionGroup: ' concept-a ',
          stageOrder: 2,
          requires: [],
          supersedes: [],
        },
      }),
    ],
    population: [
      progressionCandidate({
        reviewUnitId: reviewUnitId('a-stage-1'),
        review: { state: 2, reps: 2 },
        progression: {
          progressionGroup: 'Concept-A',
          stageOrder: 1,
          requires: [],
          supersedes: [],
        },
      }),
      progressionCandidate({
        reviewUnitId: reviewUnitId('a-stage-2'),
        progression: {
          progressionGroup: ' concept-a ',
          stageOrder: 2,
          requires: [],
          supersedes: [],
        },
      }),
    ],
    expectedAvailableReviewUnitIds: [reviewUnitId('a-stage-2')],
    expectedLockedFreshCount: 0,
  },
  {
    name: 'progression suppresses superseded units once a harder stage is mastered',
    mode: 'strict',
    masteryPolicy: 'vault',
    candidates: [
      progressionCandidate({
        reviewUnitId: reviewUnitId('st-michael-01'),
        progression: {
          progressionGroup: 'st-michael-prayer',
          stageOrder: 1,
          requires: [],
          supersedes: [],
        },
      }),
    ],
    population: [
      progressionCandidate({
        reviewUnitId: reviewUnitId('st-michael-01'),
        progression: {
          progressionGroup: 'st-michael-prayer',
          stageOrder: 1,
          requires: [],
          supersedes: [],
        },
      }),
      progressionCandidate({
        reviewUnitId: reviewUnitId('st-michael-03'),
        review: { state: 2, reps: 4 },
        progression: {
          progressionGroup: 'st-michael-prayer',
          stageOrder: 3,
          requires: [],
          supersedes: [reviewUnitId('st-michael-01')],
        },
      }),
    ],
    expectedAvailableReviewUnitIds: [],
    expectedLockedFreshCount: 1,
  },
  {
    name: 'progression fallback returns the locked stage when nothing else is available',
    mode: 'fallback',
    masteryPolicy: 'vault',
    candidates: [
      progressionCandidate({
        reviewUnitId: reviewUnitId('creed-02'),
        progression: {
          progressionGroup: 'nicene-creed',
          stageOrder: 2,
          requires: [reviewUnitId('missing-prereq')],
          supersedes: [],
        },
      }),
    ],
    expectedAvailableReviewUnitIds: [reviewUnitId('creed-02')],
    expectedLockedFreshCount: 1,
  },
];

const queueNow = Date.UTC(2026, 3, 8, 16, 0, 0);

export const queueFixtures: QueueFixture[] = [
  {
    name: 'queue prefers review candidates over fresh ones when both are due',
    masteryPolicy: 'vault',
    now: queueNow,
    candidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('latin-01'),
        due: queueNow - 60_000,
        sourceKey: 'mass-core',
      }),
      queueCandidate({
        reviewUnitId: reviewUnitId('mass-01'),
        scheduleState: scheduleState({
          state: 2,
          reps: 4,
          scheduled_days: 5,
          due: queueNow - 3_600_000,
        }),
        due: queueNow - 3_600_000,
        sourceKey: 'mass-core',
      }),
    ],
    expectedNextReviewUnitId: reviewUnitId('mass-01'),
  },
  {
    name: 'queue avoids immediate same-source clumps when an alternative exists',
    masteryPolicy: 'vault',
    now: queueNow,
    candidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('abolition-11'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
          due: queueNow - 120_000,
        }),
        due: queueNow - 120_000,
        sourceKey: 'abolition-of-man',
      }),
      queueCandidate({
        reviewUnitId: reviewUnitId('nato-01'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
          due: queueNow - 100_000,
        }),
        due: queueNow - 100_000,
        sourceKey: 'nato-phonetic',
      }),
    ],
    recentCandidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('abolition-10'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
        }),
        sourceKey: 'abolition-of-man',
      }),
    ],
    expectedNextReviewUnitId: reviewUnitId('nato-01'),
  },
  {
    name: 'queue allows callers to disable source clumping with a zero source window',
    masteryPolicy: 'vault',
    now: queueNow,
    candidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('abolition-11'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
          due: queueNow - 120_000,
        }),
        due: queueNow - 120_000,
        sourceKey: 'abolition-of-man',
      }),
      queueCandidate({
        reviewUnitId: reviewUnitId('nato-01'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
          due: queueNow - 100_000,
        }),
        due: queueNow - 100_000,
        sourceKey: 'nato-phonetic',
      }),
    ],
    recentCandidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('abolition-10'),
        scheduleState: scheduleState({
          state: 2,
          reps: 3,
          scheduled_days: 8,
        }),
        sourceKey: 'abolition-of-man',
      }),
    ],
    recentSourceWindow: 0,
    expectedNextReviewUnitId: reviewUnitId('abolition-11'),
  },
  {
    name: 'queue falls back to an unsatisfied stage instead of hiding it forever',
    masteryPolicy: 'vault',
    now: queueNow,
    candidates: [
      queueCandidate({
        reviewUnitId: reviewUnitId('creed-02'),
        due: queueNow - 60_000,
        progression: {
          progressionGroup: 'nicene-creed',
          stageOrder: 2,
          requires: [reviewUnitId('missing-prereq')],
          supersedes: [],
        },
      }),
    ],
    expectedNextReviewUnitId: reviewUnitId('creed-02'),
  },
];
