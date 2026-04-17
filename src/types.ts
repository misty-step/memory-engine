import { type Card, Rating as FsrsRating } from 'ts-fsrs';

declare const reviewUnitIdBrand: unique symbol;

export type ReviewUnitId = string & { readonly [reviewUnitIdBrand]: true };

type PromptBase = {
  reviewUnitId: ReviewUnitId;
  prompt: string;
};

type ExactPromptBase = PromptBase & {
  acceptedAnswers: string[];
  equivalenceGroups: string[][];
  ignoredTokens: string[];
};

export type McqPrompt = PromptBase & {
  kind: 'mcq';
  choices: string[];
  correctChoice: string;
};

export type BooleanPrompt = PromptBase & {
  kind: 'boolean';
  correctAnswer: boolean;
};

export type ClozePrompt = ExactPromptBase & {
  kind: 'cloze';
};

export type ShortAnswerPrompt = ExactPromptBase & {
  kind: 'shortAnswer';
};

export type Prompt = McqPrompt | BooleanPrompt | ClozePrompt | ShortAnswerPrompt;

export type Verdict = 'correct' | 'close' | 'wrong' | 'revealed';

export const Rating = {
  Again: FsrsRating.Again,
  Hard: FsrsRating.Hard,
  Good: FsrsRating.Good,
  Easy: FsrsRating.Easy,
} as const;

export type Rating = (typeof Rating)[keyof typeof Rating];

export type GradeCtx = {
  responseTimeMs: number;
  priorReps: number;
};

export type RatingPolicy = (verdict: Verdict, ctx: GradeCtx) => Rating;

export type ScheduleState = Omit<Card, 'due' | 'last_review'> & {
  due: number;
  last_review: number | null;
};

export type ProgressionMetadata = {
  progressionGroup: string | null;
  stageOrder: number;
  requires: ReviewUnitId[];
  supersedes: ReviewUnitId[];
};

export type ProgressionCandidate<TReview> = {
  reviewUnitId: ReviewUnitId;
  review: TReview | null;
  progression: ProgressionMetadata | null;
};

export type MasteryPolicy<TReview> = (review: TReview) => boolean;

export type ProgressionFilterResult<TCandidate> = {
  available: TCandidate[];
  lockedFreshCount: number;
};

export type QueueCandidate = {
  reviewUnitId: ReviewUnitId;
  scheduleState: ScheduleState | null;
  // Canonical queue timestamp used for due filtering and ordering.
  // Consumers may intentionally diverge from scheduleState.due when they apply
  // app-level deferrals or session windows outside the core scheduler.
  due: number;
  progression: ProgressionMetadata | null;
  conceptKey: string | null;
  sourceKey: string | null;
  domainKey: string | null;
};

export type QueueSeparationPass = {
  concept: boolean;
  source: boolean;
  domain: boolean;
};

export type QueueSelectionOptions<TCandidate extends QueueCandidate> = {
  now?: number;
  // Most-recent-first review history used by the anti-clumping passes.
  recentCandidates?: readonly TCandidate[];
  // Full candidate population used for progression unlocks and supersession.
  // Defaults to the full queue input, not just the due subset.
  population?: readonly TCandidate[];
  // Number of top-priority candidates to scan before falling back.
  candidateWindow?: number;
  recentConceptWindow?: number;
  recentSourceWindow?: number;
  recentDomainWindow?: number;
  separationPasses?: readonly QueueSeparationPass[];
};

export type GraderKind = 'deterministic' | 'rubric-llm';

export type GradeResult = {
  verdict: Verdict;
  rating: Rating;
  isCorrect: boolean;
  submittedAnswer: string;
  expectedAnswer: string;
  graderKind: GraderKind;
  graderModel: string | null;
  graderConfidence: number | null;
  feedback: string;
};
