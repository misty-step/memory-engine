import type { MemoryServiceStore, ServiceAttemptRecord } from '../../service';
import type { QueueCandidate, ReviewUnitId, ScheduleState } from '../../src';

export class DuplicateAppliedReviewError extends Error {
  constructor(key: string) {
    super(`Duplicate applied review: ${key}`);
    this.name = 'DuplicateAppliedReviewError';
  }
}

export class StaleScheduleWriteError extends Error {
  constructor(reviewUnitId: ReviewUnitId) {
    super(`Stale schedule write for review unit: ${reviewUnitId}`);
    this.name = 'StaleScheduleWriteError';
  }
}

export type ValidatingStoreOptions = {
  knownReviewUnitIds?: ReviewUnitId[];
  initialCandidates?: QueueCandidate[];
  initialSchedules?: Map<ReviewUnitId, ScheduleState>;
  failMode?: 'record' | 'read' | 'apply' | null;
};

export class ValidatingMemoryServiceStore implements MemoryServiceStore {
  readonly attempts: ServiceAttemptRecord[] = [];
  readonly schedules = new Map<ReviewUnitId, ScheduleState>();
  readonly appliedReviews = new Set<string>();

  private knownReviewUnitIds: Set<ReviewUnitId> | null = null;
  private candidates: QueueCandidate[] = [];
  private failMode: 'record' | 'read' | 'apply' | null = null;

  constructor(options: ValidatingStoreOptions = {}) {
    if (options.knownReviewUnitIds) {
      this.knownReviewUnitIds = new Set(options.knownReviewUnitIds);
    }
    if (options.initialCandidates) {
      this.candidates = [...options.initialCandidates];
      for (const candidate of options.initialCandidates) {
        if (candidate.scheduleState) {
          this.schedules.set(candidate.reviewUnitId, candidate.scheduleState);
        }
      }
    }
    if (options.initialSchedules) {
      for (const [id, state] of options.initialSchedules.entries()) {
        this.schedules.set(id, state);
      }
    }
    this.failMode = options.failMode ?? null;
  }

  setFailMode(failMode: 'record' | 'read' | 'apply' | null): void {
    this.failMode = failMode;
  }

  setCandidates(candidates: QueueCandidate[]): void {
    this.candidates = [...candidates];
  }

  private assertKnownReviewUnit(reviewUnitId: ReviewUnitId): void {
    if (this.knownReviewUnitIds && !this.knownReviewUnitIds.has(reviewUnitId)) {
      throw new Error(`Unknown review unit: ${reviewUnitId}`);
    }
  }

  private assertAttemptContract(attempt: ServiceAttemptRecord): void {
    this.assertKnownReviewUnit(attempt.reviewUnitId);

    if (attempt.submittedAnswer.trim().length === 0) {
      throw new Error('Attempt answer must not be blank');
    }

    if (!Number.isInteger(attempt.responseTimeMs) || attempt.responseTimeMs <= 0) {
      throw new Error('Attempt response time must be a positive integer');
    }

    if (!Number.isInteger(attempt.occurredAt)) {
      throw new Error('Attempt timestamp must be an integer epoch millisecond value');
    }
  }

  private getAppliedReviewKey(attempt: ServiceAttemptRecord): string {
    if (attempt.idempotencyKey !== undefined) {
      return `idempotency:${attempt.idempotencyKey}`;
    }

    return [
      'attempt',
      attempt.reviewUnitId,
      attempt.promptId,
      attempt.submittedAnswer,
      attempt.responseTimeMs.toString(),
      attempt.occurredAt.toString(),
    ].join('\u0000');
  }

  async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
    if (this.failMode === 'record') {
      throw new Error('recordAttempt failed');
    }

    this.assertAttemptContract(attempt);
    this.attempts.push(attempt);
  }

  async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
    if (this.failMode === 'read') {
      throw new Error('readScheduleState failed');
    }

    this.assertKnownReviewUnit(reviewUnitId);
    return this.schedules.get(reviewUnitId) ?? null;
  }

  async applyReview(
    reviewUnitId: ReviewUnitId,
    attempt: ServiceAttemptRecord,
    scheduleState: ScheduleState,
    expectedPriorScheduleState: ScheduleState | null,
  ): Promise<void> {
    if (this.failMode === 'apply') {
      throw new Error('applyReview failed');
    }

    this.assertKnownReviewUnit(reviewUnitId);

    if (reviewUnitId !== attempt.reviewUnitId) {
      throw new Error('Applied review unit must match the attempt review unit');
    }

    if (scheduleState.last_review !== attempt.occurredAt) {
      throw new Error('Schedule last_review must match the attempt timestamp');
    }

    const key = this.getAppliedReviewKey(attempt);
    if (this.appliedReviews.has(key)) {
      throw new DuplicateAppliedReviewError(key);
    }

    const currentSchedule = this.schedules.get(reviewUnitId) ?? null;
    if (JSON.stringify(currentSchedule) !== JSON.stringify(expectedPriorScheduleState)) {
      throw new StaleScheduleWriteError(reviewUnitId);
    }

    this.assertAttemptContract(attempt);
    this.attempts.push(attempt);
    this.schedules.set(reviewUnitId, scheduleState);
    this.appliedReviews.add(key);
  }

  async listQueueCandidates(): Promise<QueueCandidate[]> {
    return this.candidates.map((candidate) => {
      const schedule = this.schedules.get(candidate.reviewUnitId) ?? null;
      return {
        ...candidate,
        scheduleState: schedule,
        due: schedule?.due ?? candidate.due,
      };
    });
  }
}
