import type {
  GradeResult,
  Prompt,
  QueueCandidate,
  ReviewUnitId,
  ScheduleState,
} from 'memory-engine';

import {
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';
import { compileAuthoredFixture, latinPrayerFixture } from '../import-probe';

const defaultNow = Date.UTC(2026, 4, 15, 12, 0, 0);

type WebShellSessionOptions = {
  now?: () => number;
};

type WebShellStatus = 'answering' | 'revealed' | 'graded';

export type WebShellCurrent = {
  reviewUnitId: ReviewUnitId;
  promptId: string | null;
  prompt: string;
  expectedAnswer: string | null;
  grade: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null;
  reviewState: Pick<ScheduleState, 'due' | 'reps' | 'state'> | null;
};

export type WebShellQueueRow = {
  reviewUnitId: ReviewUnitId;
  due: number;
  reps: number;
  state: number | null;
};

export type WebShellView = {
  fixture: string;
  status: WebShellStatus;
  current: WebShellCurrent | null;
  queue: WebShellQueueRow[];
  attempts: number;
  commands: string[];
  interfacePressure: string[];
};

export type WebShellReceipt = {
  fixture: string;
  commands: string[];
  submittedAnswer: string;
  gradedVerdict: GradeResult['verdict'];
  gradedRating: GradeResult['rating'];
  scheduledReps: number;
  nextReviewUnitId: ReviewUnitId | null;
  interfacePressure: string[];
  extractionRecommendation: 'keep experimenting';
};

type WebShellUnit = {
  prompt: Prompt;
  promptId: string | null;
  queue: QueueCandidate;
};

type WebShellStore = {
  attempts: ServiceAttemptRecord[];
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
};

export type WebShellSession = {
  start(): Promise<WebShellView>;
  reveal(): WebShellView;
  submitAnswer(answer: string, responseTimeMs: number): Promise<WebShellView>;
  next(): Promise<WebShellView>;
  view(): WebShellView;
};

const interfacePressure = [
  'Reveal is UI-owned because the service has no first-class revealed review command.',
  'Review-state visibility needs a compact DTO; raw ScheduleState is too engine-shaped for UI copy.',
  'Prompt copy, confidence copy, and answer draft state remain client-owned.',
];

export function createWebShellSession(options: WebShellSessionOptions = {}): WebShellSession {
  const now = options.now ?? (() => defaultNow);
  const compiled = compileAuthoredFixture(latinPrayerFixture, now());
  const units = compiled.prompts.map((prompt) => {
    const queue = compiled.queue.find(
      (candidate) => candidate.reviewUnitId === prompt.reviewUnitId,
    );

    if (queue === undefined) {
      throw new Error(`Missing queue candidate for ${prompt.reviewUnitId}`);
    }

    return {
      prompt,
      promptId: compiled.promptIds.get(prompt.reviewUnitId) ?? null,
      queue,
    };
  });
  const unitsById = new Map(units.map((unit) => [unit.prompt.reviewUnitId, unit]));
  const webStore = createWebShellStore(compiled, unitsById);
  const service = createMemoryService({
    store: webStore.store,
    now,
    masteryPolicy: (schedule) => schedule.state === 2 && schedule.reps >= 4,
  });
  const commands: string[] = [];
  let current: WebShellUnit | null = null;
  let status: WebShellStatus = 'answering';
  let grade: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null = null;
  let expectedAnswer: string | null = null;

  async function selectNext(): Promise<void> {
    commands.push('next-queue');
    const selected = await service.execute({ kind: 'next-queue' });
    current =
      selected.candidate === null ? null : (unitsById.get(selected.candidate.reviewUnitId) ?? null);
    status = 'answering';
    grade = null;
    expectedAnswer = null;
  }

  function readView(): WebShellView {
    return {
      fixture: compiled.fixture,
      status,
      current:
        current === null ? null : currentView(current, webStore.schedules, expectedAnswer, grade),
      queue: queueRows(units, webStore.schedules),
      attempts: webStore.attempts.length,
      commands: [...commands],
      interfacePressure,
    };
  }

  return {
    async start(): Promise<WebShellView> {
      await selectNext();
      return readView();
    },
    reveal(): WebShellView {
      const active = requireCurrent(current);
      commands.push('reveal');
      expectedAnswer = promptExpectedAnswer(active.prompt);
      status = 'revealed';
      return readView();
    },
    async submitAnswer(answer: string, responseTimeMs: number): Promise<WebShellView> {
      const active = requireCurrent(current);
      commands.push('grade/apply-review');
      const review = await service.execute({
        kind: 'grade/apply-review',
        prompt: active.prompt,
        promptId: active.promptId,
        submittedAnswer: answer,
        responseTimeMs,
      });
      grade = {
        verdict: review.grade.verdict,
        rating: review.grade.rating,
        isCorrect: review.grade.isCorrect,
      };
      expectedAnswer = review.grade.expectedAnswer;
      status = 'graded';
      return readView();
    },
    async next(): Promise<WebShellView> {
      await selectNext();
      return readView();
    },
    view(): WebShellView {
      return readView();
    },
  };
}

export async function runWebShellFlow(
  options: WebShellSessionOptions = {},
): Promise<WebShellReceipt> {
  const shell = createWebShellSession(options);
  await shell.start();
  shell.reveal();
  const reviewed = await shell.submitAnswer('I believe in one God', 2_400);
  const next = await shell.next();

  return {
    fixture: reviewed.fixture,
    commands: ['next-queue', 'reveal', 'grade/apply-review', 'next-queue'],
    submittedAnswer: 'I believe in one God',
    gradedVerdict: requireCurrentView(reviewed).grade?.verdict ?? 'wrong',
    gradedRating: requireCurrentView(reviewed).grade?.rating ?? 1,
    scheduledReps: requireCurrentView(reviewed).reviewState?.reps ?? 0,
    nextReviewUnitId: next.current?.reviewUnitId ?? null,
    interfacePressure,
    extractionRecommendation: 'keep experimenting',
  };
}

function createWebShellStore(
  compiled: ReturnType<typeof compileAuthoredFixture>,
  unitsById: Map<ReviewUnitId, WebShellUnit>,
): WebShellStore {
  const attempts: ServiceAttemptRecord[] = [];
  const schedules = new Map(
    compiled.schedules.map((schedule) => [schedule.reviewUnitId, schedule.state]),
  );

  function assertKnown(reviewUnitId: ReviewUnitId): void {
    if (!unitsById.has(reviewUnitId)) {
      throw new Error(`Unknown web shell review unit: ${reviewUnitId}`);
    }
  }

  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      assertKnown(attempt.reviewUnitId);
      attempts.push(attempt);
    },
    async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
      assertKnown(reviewUnitId);
      return schedules.get(reviewUnitId) ?? null;
    },
    async applyReview(
      reviewUnitId: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      scheduleState: ScheduleState,
    ): Promise<void> {
      assertKnown(reviewUnitId);
      attempts.push(attempt);
      schedules.set(reviewUnitId, scheduleState);
    },
    async listQueueCandidates(): Promise<QueueCandidate[]> {
      return compiled.queue.map((candidate) => {
        const scheduleState = schedules.get(candidate.reviewUnitId) ?? null;

        return {
          ...candidate,
          scheduleState,
          due: scheduleState?.due ?? candidate.due,
        };
      });
    },
  };

  return { attempts, schedules, store };
}

function currentView(
  unit: WebShellUnit,
  schedules: Map<ReviewUnitId, ScheduleState>,
  expectedAnswerValue: string | null,
  gradeValue: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null,
): WebShellCurrent {
  const schedule = schedules.get(unit.prompt.reviewUnitId) ?? null;

  return {
    reviewUnitId: unit.prompt.reviewUnitId,
    promptId: unit.promptId,
    prompt: unit.prompt.prompt,
    expectedAnswer: expectedAnswerValue,
    grade: gradeValue,
    reviewState:
      schedule === null
        ? null
        : {
            due: schedule.due,
            reps: schedule.reps,
            state: schedule.state,
          },
  };
}

function queueRows(
  units: WebShellUnit[],
  schedules: Map<ReviewUnitId, ScheduleState>,
): WebShellQueueRow[] {
  return units
    .map((unit) => {
      const schedule = schedules.get(unit.prompt.reviewUnitId) ?? null;

      return {
        reviewUnitId: unit.prompt.reviewUnitId,
        due: schedule?.due ?? unit.queue.due,
        reps: schedule?.reps ?? 0,
        state: schedule?.state ?? null,
      };
    })
    .sort((left, right) => left.due - right.due);
}

function promptExpectedAnswer(prompt: Prompt): string {
  switch (prompt.kind) {
    case 'mcq':
      return prompt.correctChoice;
    case 'boolean':
      return prompt.correctAnswer ? 'True' : 'False';
    case 'cloze':
    case 'shortAnswer':
    case 'recitation':
      return prompt.acceptedAnswers.join(' / ');
  }
}

function requireCurrent(current: WebShellUnit | null): WebShellUnit {
  if (current === null) {
    throw new Error('Web shell has no active review unit');
  }

  return current;
}

function requireCurrentView(view: WebShellView): WebShellCurrent {
  if (view.current === null) {
    throw new Error('Web shell view has no active review unit');
  }

  return view.current;
}

if (import.meta.main) {
  const receipt = await runWebShellFlow();
  console.log(JSON.stringify(receipt, null, 2));
}
