import type { ScheduleState } from '../src';

const REVIEW_STATE_FIELDS = ['due', 'reps', 'lapses', 'state', 'last_review'] as const;
const REVIEW_STATE_ENUM_VALUES = new Set([0, 1, 2, 3]);

type ReviewStateField = (typeof REVIEW_STATE_FIELDS)[number];

export type ReviewStateProjection = Pick<
  ScheduleState,
  'due' | 'reps' | 'lapses' | 'state' | 'last_review'
>;

export type ReviewStateScheduleChange = {
  before: ReviewStateProjection | null;
  after: ReviewStateProjection;
};

export function projectReviewStateProjection(
  schedule: ScheduleState | null,
): ReviewStateProjection | null {
  if (schedule === null) {
    return null;
  }

  return projectRequiredReviewState(schedule);
}

export function projectReviewStateChange(
  before: ScheduleState | null,
  after: ScheduleState,
): ReviewStateScheduleChange {
  return {
    before: projectReviewStateProjection(before),
    after: projectRequiredReviewState(after),
  };
}

export function parseReviewStateProjectionCandidate(candidate: unknown): ReviewStateProjection {
  const objectValue = requireObject(candidate);
  const due = requireIntegerField(objectValue, 'due');
  const reps = requireIntegerField(objectValue, 'reps');
  const lapses = requireIntegerField(objectValue, 'lapses');
  const state = requireState(objectValue);
  const lastReview = requireLastReview(objectValue);

  return {
    due,
    reps,
    lapses,
    state,
    last_review: lastReview,
  };
}

function projectRequiredReviewState(schedule: ScheduleState): ReviewStateProjection {
  return {
    due: schedule.due,
    reps: schedule.reps,
    lapses: schedule.lapses,
    state: schedule.state,
    last_review: schedule.last_review,
  };
}

function requireObject(candidate: unknown): Record<ReviewStateField, unknown> {
  if (typeof candidate !== 'object' || candidate === null || Array.isArray(candidate)) {
    throw new Error('review-state projection must be an object');
  }

  const objectValue = candidate as Record<string, unknown>;

  for (const key of Object.keys(objectValue)) {
    if (!isReviewStateField(key)) {
      throw new Error(`unsupported field: ${key}`);
    }
  }

  for (const field of REVIEW_STATE_FIELDS) {
    if (!(field in objectValue)) {
      throw new Error(`missing field: ${field}`);
    }
  }

  return objectValue as Record<ReviewStateField, unknown>;
}

function isReviewStateField(value: string): value is ReviewStateField {
  return REVIEW_STATE_FIELDS.some((field) => field === value);
}

function requireIntegerField(
  objectValue: Record<ReviewStateField, unknown>,
  field: Exclude<ReviewStateField, 'last_review' | 'state'>,
): number {
  const value = objectValue[field];
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    throw new Error(`${field} must be an integer`);
  }
  return value;
}

function requireState(objectValue: Record<ReviewStateField, unknown>): ScheduleState['state'] {
  const value = objectValue.state;
  if (
    typeof value !== 'number' ||
    !Number.isInteger(value) ||
    !REVIEW_STATE_ENUM_VALUES.has(value)
  ) {
    throw new Error('state must be one of 0, 1, 2, or 3');
  }
  return value as ScheduleState['state'];
}

function requireLastReview(
  objectValue: Record<ReviewStateField, unknown>,
): ScheduleState['last_review'] {
  const value = objectValue.last_review;
  if (value === null) {
    return null;
  }
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    throw new Error('last_review must be an integer or null');
  }
  return value;
}
