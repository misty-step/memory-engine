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
