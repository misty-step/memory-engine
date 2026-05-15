import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../service';
import { Grader } from '../src/grader';
import { pickNextQueueCandidate } from '../src/queue';
import { next } from '../src/scheduler';
import { type QueueCandidate, Rating, type ReviewUnitId, type ScheduleState } from '../src/types';

const now = Date.UTC(2026, 4, 15, 12, 0, 0);

type BenchCase = {
  name: string;
  operations: number;
  run: () => void | Promise<void>;
};

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function scheduleState(overrides: Partial<ScheduleState> = {}): ScheduleState {
  return {
    due: now - 60_000,
    stability: 2.3065,
    difficulty: 2.11810397,
    elapsed_days: 0,
    scheduled_days: 1,
    reps: 1,
    lapses: 0,
    state: 2,
    last_review: now - 86_400_000,
    ...overrides,
  };
}

function queueCandidate(index: number): QueueCandidate {
  const dueOffset = index % 10 === 0 ? 3_600_000 : 60_000 + index;

  return {
    reviewUnitId: reviewUnitId(`bench-${index.toString().padStart(4, '0')}`),
    scheduleState:
      index % 3 === 0
        ? scheduleState({
            reps: 3 + (index % 7),
            scheduled_days: 2 + (index % 9),
            due: now - dueOffset,
          })
        : null,
    due: now - dueOffset,
    progression: null,
    conceptKey: `concept-${index % 17}`,
    sourceKey: `source-${index % 11}`,
    domainKey: `domain-${index % 5}`,
  };
}

function createStore(candidates: QueueCandidate[]): {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const attempts: ServiceAttemptRecord[] = [];
  const schedules = new Map<ReviewUnitId, ScheduleState>();

  return {
    attempts,
    schedules,
    store: {
      async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
        attempts.push(attempt);
      },
      async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
        return schedules.get(reviewUnitId) ?? null;
      },
      async applyReview(
        reviewUnitId: ReviewUnitId,
        attempt: ServiceAttemptRecord,
        nextScheduleState: ScheduleState,
      ): Promise<void> {
        attempts.push(attempt);
        schedules.set(reviewUnitId, nextScheduleState);
      },
      async listQueueCandidates(): Promise<QueueCandidate[]> {
        return candidates;
      },
    },
  };
}

async function main(): Promise<void> {
  const grader = new Grader();
  const prompt = {
    kind: 'shortAnswer' as const,
    reviewUnitId: reviewUnitId('bench-prompt'),
    prompt: 'Translate poena.',
    acceptedAnswers: ['punishment'],
    equivalenceGroups: [],
    ignoredTokens: [],
  };
  const candidates = Array.from({ length: 1_000 }, (_value, index) => queueCandidate(index));
  const masteryPolicy = (schedule: ScheduleState) => schedule.state === 2 && schedule.reps >= 3;
  const service = createMemoryService({
    store: createStore(candidates).store,
    now: () => now,
    masteryPolicy,
  });

  const cases: BenchCase[] = [
    {
      name: 'grading.shortAnswer',
      operations: 10_000,
      run: () => {
        for (let index = 0; index < 10_000; index += 1) {
          grader.grade(prompt, 'Punishment', {
            responseTimeMs: 2_000,
            priorReps: index % 5,
          });
        }
      },
    },
    {
      name: 'scheduling.next',
      operations: 10_000,
      run: () => {
        let state: ScheduleState | null = null;

        for (let index = 0; index < 10_000; index += 1) {
          state = next(state, Rating.Good, now + index * 60_000);
        }
      },
    },
    {
      name: 'queue.pickNext.1000',
      operations: 1_000,
      run: () => {
        for (let index = 0; index < 1_000; index += 1) {
          pickNextQueueCandidate(candidates, masteryPolicy, {
            now,
            recentCandidates: candidates.slice(index % 25, (index % 25) + 3),
          });
        }
      },
    },
    {
      name: 'service.gradeApplyReview.nextQueue',
      operations: 500,
      run: async () => {
        for (let index = 0; index < 500; index += 1) {
          await service.execute({
            kind: 'grade/apply-review',
            prompt: {
              ...prompt,
              reviewUnitId: reviewUnitId(`bench-service-${index}`),
            },
            submittedAnswer: 'Punishment',
            responseTimeMs: 2_000,
          });
          await service.execute({ kind: 'next-queue' });
        }
      },
    },
  ];

  console.log('memory-engine benchmark receipts');
  console.log('name,operations,elapsed_ms,ops_per_ms');

  for (const benchCase of cases) {
    const start = performance.now();
    await benchCase.run();
    const elapsed = performance.now() - start;
    const opsPerMs = benchCase.operations / elapsed;

    console.log(
      `${benchCase.name},${benchCase.operations},${elapsed.toFixed(3)},${opsPerMs.toFixed(3)}`,
    );
  }
}

await main();
