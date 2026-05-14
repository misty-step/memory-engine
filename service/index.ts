import {
  type GradeResult,
  Grader,
  type MasteryPolicy,
  type Prompt,
  type QueueCandidate,
  type QueueSelectionOptions,
  type ReviewUnitId,
  type ScheduleState,
  next,
  pickNextQueueCandidate,
} from '../src';

export type ServiceAttemptRecord = {
  reviewUnitId: ReviewUnitId;
  promptId: string | null;
  submittedAnswer: string;
  responseTimeMs: number;
  occurredAt: number;
  grade?: GradeResult;
};

export type MemoryServiceStore = {
  recordAttempt(attempt: ServiceAttemptRecord): Promise<void>;
  readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null>;
  applyReview(
    reviewUnitId: ReviewUnitId,
    attempt: ServiceAttemptRecord,
    scheduleState: ScheduleState,
  ): Promise<void>;
  listQueueCandidates(): Promise<QueueCandidate[]>;
};

export type RecordAttemptCommand = {
  kind: 'record-attempt';
  attempt: {
    reviewUnitId: ReviewUnitId;
    promptId?: string | null;
    submittedAnswer: string;
    responseTimeMs: number;
    occurredAt?: number;
  };
};

export type GradeApplyReviewCommand = {
  kind: 'grade/apply-review';
  prompt: Prompt;
  submittedAnswer: string;
  responseTimeMs: number;
  promptId?: string | null;
  occurredAt?: number;
};

export type NextQueueCommand = {
  kind: 'next-queue';
  options?: Omit<QueueSelectionOptions<QueueCandidate>, 'now' | 'population'>;
};

export type MemoryServiceCommand =
  | RecordAttemptCommand
  | GradeApplyReviewCommand
  | NextQueueCommand;

export type AttemptRecordedResult = {
  kind: 'attempt-recorded';
  attempt: ServiceAttemptRecord;
};

export type ReviewAppliedResult = {
  kind: 'review-applied';
  attempt: ServiceAttemptRecord;
  grade: GradeResult;
  scheduleState: ScheduleState;
};

export type QueueSelectedResult = {
  kind: 'queue-selected';
  candidate: QueueCandidate | null;
};

export type MemoryServiceResult = AttemptRecordedResult | ReviewAppliedResult | QueueSelectedResult;

export type MemoryServiceOptions = {
  store: MemoryServiceStore;
  masteryPolicy: MasteryPolicy<ScheduleState>;
  now?: () => number;
  grader?: Grader;
};

export type MemoryService = {
  execute(command: RecordAttemptCommand): Promise<AttemptRecordedResult>;
  execute(command: GradeApplyReviewCommand): Promise<ReviewAppliedResult>;
  execute(command: NextQueueCommand): Promise<QueueSelectedResult>;
  execute(command: MemoryServiceCommand): Promise<MemoryServiceResult>;
};

export function createMemoryService(options: MemoryServiceOptions): MemoryService {
  const now = options.now ?? Date.now;
  const grader = options.grader ?? new Grader();

  function execute(command: RecordAttemptCommand): Promise<AttemptRecordedResult>;
  function execute(command: GradeApplyReviewCommand): Promise<ReviewAppliedResult>;
  function execute(command: NextQueueCommand): Promise<QueueSelectedResult>;
  async function execute(command: MemoryServiceCommand): Promise<MemoryServiceResult> {
    switch (command.kind) {
      case 'record-attempt':
        return recordAttempt(options.store, command, now());
      case 'grade/apply-review':
        return gradeAndApplyReview(options.store, grader, command, now());
      case 'next-queue':
        return selectNextQueue(options.store, options.masteryPolicy, command, now());
    }
  }

  return { execute };
}

async function recordAttempt(
  store: MemoryServiceStore,
  command: RecordAttemptCommand,
  defaultOccurredAt: number,
): Promise<AttemptRecordedResult> {
  const attempt: ServiceAttemptRecord = {
    reviewUnitId: command.attempt.reviewUnitId,
    promptId: command.attempt.promptId ?? null,
    submittedAnswer: command.attempt.submittedAnswer,
    responseTimeMs: command.attempt.responseTimeMs,
    occurredAt: command.attempt.occurredAt ?? defaultOccurredAt,
  };

  await store.recordAttempt(attempt);

  return {
    kind: 'attempt-recorded',
    attempt,
  };
}

async function gradeAndApplyReview(
  store: MemoryServiceStore,
  grader: Grader,
  command: GradeApplyReviewCommand,
  defaultOccurredAt: number,
): Promise<ReviewAppliedResult> {
  const priorSchedule = await store.readScheduleState(command.prompt.reviewUnitId);
  const grade = grader.grade(command.prompt, command.submittedAnswer, {
    responseTimeMs: command.responseTimeMs,
    priorReps: priorSchedule?.reps ?? 0,
  });
  const attempt: ServiceAttemptRecord = {
    reviewUnitId: command.prompt.reviewUnitId,
    promptId: command.promptId ?? null,
    submittedAnswer: command.submittedAnswer,
    responseTimeMs: command.responseTimeMs,
    occurredAt: command.occurredAt ?? defaultOccurredAt,
    grade,
  };
  const scheduleState = next(priorSchedule, grade.rating, attempt.occurredAt);

  await store.applyReview(command.prompt.reviewUnitId, attempt, scheduleState);

  return {
    kind: 'review-applied',
    attempt,
    grade,
    scheduleState,
  };
}

async function selectNextQueue(
  store: MemoryServiceStore,
  masteryPolicy: MasteryPolicy<ScheduleState>,
  command: NextQueueCommand,
  currentTime: number,
): Promise<QueueSelectedResult> {
  const candidates = await store.listQueueCandidates();
  const candidate = pickNextQueueCandidate(candidates, masteryPolicy, {
    ...command.options,
    now: currentTime,
    population: candidates,
  });

  return {
    kind: 'queue-selected',
    candidate,
  };
}
