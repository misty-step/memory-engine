import type { Prompt, QueueCandidate, ReviewUnitId, ScheduleState } from 'memory-engine/types';
import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';

const now = Date.UTC(2026, 4, 15, 12, 0, 0);

export type CliReviewReceipt = {
  fixture: string;
  commands: string[];
  confidence: number;
  calibrationError: number;
  attemptCount: number;
  gradedVerdict: string;
  gradedRating: number;
  scheduledReps: number;
  nextReviewUnitId: string | null;
  stayedOutsideSrc: string[];
};

type CliReviewUnit = {
  reviewUnitId: ReviewUnitId;
  promptId: string;
  prompt: Prompt;
  submittedAnswer: string;
  confidence: number;
  responseTimeMs: number;
  queue: Omit<QueueCandidate, 'reviewUnitId' | 'scheduleState' | 'due'>;
};

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

const fixture: CliReviewUnit[] = [
  {
    reviewUnitId: reviewUnitId('cli-credo-opening'),
    promptId: 'cli-credo-opening-en',
    prompt: {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitId('cli-credo-opening'),
      prompt: 'What does Credo in unum Deum mean?',
      acceptedAnswers: ['I believe in one God'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submittedAnswer: 'I believe in one God',
    confidence: 0.72,
    responseTimeMs: 2_800,
    queue: {
      progression: null,
      conceptKey: 'creed-opening',
      sourceKey: 'mass-core',
      domainKey: 'latin',
    },
  },
  {
    reviewUnitId: reviewUnitId('cli-pater-opening'),
    promptId: 'cli-pater-opening-en',
    prompt: {
      kind: 'shortAnswer',
      reviewUnitId: reviewUnitId('cli-pater-opening'),
      prompt: 'What does Pater noster mean?',
      acceptedAnswers: ['Our Father'],
      equivalenceGroups: [],
      ignoredTokens: [],
    },
    submittedAnswer: 'Our Father',
    confidence: 0.61,
    responseTimeMs: 2_500,
    queue: {
      progression: null,
      conceptKey: 'lords-prayer-opening',
      sourceKey: 'mass-core',
      domainKey: 'latin',
    },
  },
];

function createCliStore(units: CliReviewUnit[]): {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const attempts: ServiceAttemptRecord[] = [];
  const schedules = new Map<ReviewUnitId, ScheduleState>();
  const unitsById = new Map(units.map((unit) => [unit.reviewUnitId, unit]));

  function assertKnownReviewUnit(reviewUnitIdValue: ReviewUnitId): void {
    if (!unitsById.has(reviewUnitIdValue)) {
      throw new Error(`Unknown review unit: ${reviewUnitIdValue}`);
    }
  }

  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      assertKnownReviewUnit(attempt.reviewUnitId);
      attempts.push(attempt);
    },
    async readScheduleState(reviewUnitIdValue: ReviewUnitId): Promise<ScheduleState | null> {
      assertKnownReviewUnit(reviewUnitIdValue);
      return schedules.get(reviewUnitIdValue) ?? null;
    },
    async applyReview(
      reviewUnitIdValue: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      nextScheduleState: ScheduleState,
    ): Promise<void> {
      assertKnownReviewUnit(reviewUnitIdValue);
      attempts.push(attempt);
      schedules.set(reviewUnitIdValue, nextScheduleState);
    },
    async listQueueCandidates(): Promise<QueueCandidate[]> {
      return units.map((unit) => {
        const currentScheduleState = schedules.get(unit.reviewUnitId) ?? null;

        return {
          reviewUnitId: unit.reviewUnitId,
          scheduleState: currentScheduleState,
          due: currentScheduleState?.due ?? now - 60_000,
          ...unit.queue,
        };
      });
    },
  };

  return { attempts, schedules, store };
}

export async function runCliReview(): Promise<CliReviewReceipt> {
  const [first] = fixture;

  if (first === undefined) {
    throw new Error('CLI review fixture must contain at least one review unit');
  }

  const { attempts, schedules, store } = createCliStore(fixture);
  const service = createMemoryService({
    store,
    now: () => now,
    masteryPolicy: (schedule) => schedule.state === 2 && schedule.reps >= 3,
  });
  const review = await service.execute({
    kind: 'grade/apply-review',
    prompt: first.prompt,
    promptId: first.promptId,
    submittedAnswer: first.submittedAnswer,
    responseTimeMs: first.responseTimeMs,
  });
  const next = await service.execute({ kind: 'next-queue' });
  const actual = review.grade.isCorrect ? 1 : 0;

  return {
    fixture: 'latin-prayer-opening',
    commands: ['grade/apply-review', 'next-queue'],
    confidence: first.confidence,
    calibrationError: Math.abs(first.confidence - actual),
    attemptCount: attempts.length,
    gradedVerdict: review.grade.verdict,
    gradedRating: review.grade.rating,
    scheduledReps: schedules.get(first.reviewUnitId)?.reps ?? 0,
    nextReviewUnitId: next.candidate?.reviewUnitId ?? null,
    stayedOutsideSrc: [
      'fixture content',
      'confidence capture',
      'calibration metric',
      'CLI receipt formatting',
      'in-memory dogfood store',
    ],
  };
}

if (import.meta.main) {
  const receipt = await runCliReview();
  console.log(JSON.stringify(receipt, null, 2));
}
