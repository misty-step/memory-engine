import { createMemoryService } from '../../service';
import type { GradeResult, Prompt, ReviewUnitId, ScheduleState } from '../../src';
import { runBetaGeneration } from '../beta-generation';
import {
  type GeneratedPromptDraft,
  type SourceDocument,
  createBetaPersistenceStore,
} from '../beta-store';

const defaultNow = Date.UTC(2026, 4, 24, 12, 0, 0);

export type BetaCoachOptions = {
  path: string;
  now?: () => number;
};

export type BetaCoachStatus = 'drafting' | 'answering' | 'revealed' | 'graded';

export type BetaCoachSourceInput = {
  id: string;
  title: string;
  body: string;
};

export type ReviewStateProjection = Pick<
  ScheduleState,
  'due' | 'reps' | 'lapses' | 'state' | 'last_review'
>;

export type ScheduleChange = {
  before: ReviewStateProjection | null;
  after: ReviewStateProjection;
};

export type BetaCoachCurrent = {
  reviewUnitId: ReviewUnitId;
  promptId: string;
  activityKind: GeneratedPromptDraft['activityKind'];
  activityStage: string;
  prompt: string;
  expectedAnswer: string | null;
  workedSolution: string | null;
  scoringRubric: string | null;
  grade: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null;
  reviewState: ReviewStateProjection | null;
  scheduleChange: ScheduleChange | null;
};

export type BetaCoachDraftRow = {
  id: string;
  activityKind: GeneratedPromptDraft['activityKind'];
  activityStage: string;
  prompt: string;
  validationStatus: GeneratedPromptDraft['validation']['status'];
};

export type BetaCoachQueueRow = {
  reviewUnitId: ReviewUnitId;
  due: number;
  reps: number;
  state: number | null;
  activityKind: GeneratedPromptDraft['activityKind'] | null;
  activityStage: string | null;
};

export type BetaCoachSummary = {
  sourceCount: number;
  acceptedDraftCount: number;
  approvedReviewUnitCount: number;
  attemptCount: number;
  lastOutcome: GradeResult['verdict'] | null;
  nextReviewUnitId: ReviewUnitId | null;
};

export type BetaCoachView = {
  status: BetaCoachStatus;
  commands: string[];
  sources: Pick<SourceDocument, 'id' | 'title' | 'kind' | 'createdAt'>[];
  drafts: BetaCoachDraftRow[];
  queue: BetaCoachQueueRow[];
  current: BetaCoachCurrent | null;
  summary: BetaCoachSummary;
  pressureSignals: string[];
};

export type BetaCoachSession = {
  start(): Promise<BetaCoachView>;
  ingestSource(input: BetaCoachSourceInput): Promise<BetaCoachView>;
  generate(sourceDocumentIds?: string[]): Promise<BetaCoachView>;
  approveAcceptedDrafts(): Promise<BetaCoachView>;
  reveal(): Promise<BetaCoachView>;
  submitAnswer(answer: string, responseTimeMs: number): Promise<BetaCoachView>;
  next(): Promise<BetaCoachView>;
  view(): Promise<BetaCoachView>;
};

const pressureSignals = [
  'Queue/review semantics stay in service commands while this client owns command choreography.',
  'Reveal remains display-only UI state and is not persisted as a service command.',
  'Review-state projection is still a compact learner-facing DTO assembled in the client.',
];

export async function createBetaCoachSession(options: BetaCoachOptions): Promise<BetaCoachSession> {
  const now = options.now ?? (() => defaultNow);
  const store = await createBetaPersistenceStore(options.path);
  const service = createMemoryService({
    store,
    now,
    masteryPolicy: (schedule) => schedule.state === 2 && schedule.reps >= 3,
  });

  let current: GeneratedPromptDraft | null = null;
  let status: BetaCoachStatus = 'drafting';
  let expectedAnswer: string | null = null;
  let grade: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null = null;
  let scheduleChange: ScheduleChange | null = null;
  const commands: string[] = [];

  async function selectNext(): Promise<void> {
    commands.push('next-queue');
    const result = await service.execute({ kind: 'next-queue' });
    current =
      result.candidate === null
        ? null
        : findApprovedDraft(store.snapshot().generatedPromptDrafts, result.candidate.reviewUnitId);
    status = current === null ? 'drafting' : 'answering';
    expectedAnswer = null;
    grade = null;
    scheduleChange = null;
  }

  async function readView(): Promise<BetaCoachView> {
    const snapshot = store.snapshot();
    const queue = (await store.listQueueCandidates()).sort((left, right) => left.due - right.due);
    const nextReviewUnitId = queue[0]?.reviewUnitId ?? null;

    return {
      status,
      commands: [...commands],
      sources: snapshot.sourceDocuments.map((source) => ({
        id: source.id,
        title: source.title,
        kind: source.kind,
        createdAt: source.createdAt,
      })),
      drafts: snapshot.generatedPromptDrafts.map((draft) => ({
        id: draft.id,
        activityKind: draft.activityKind,
        activityStage: draft.activityStage,
        prompt: draft.prompt.prompt,
        validationStatus: draft.validation.status,
      })),
      queue: queue.map((candidate) => {
        const draft = findApprovedDraft(snapshot.generatedPromptDrafts, candidate.reviewUnitId);
        return {
          reviewUnitId: candidate.reviewUnitId,
          due: candidate.due,
          reps: candidate.scheduleState?.reps ?? 0,
          state: candidate.scheduleState?.state ?? null,
          activityKind: draft?.activityKind ?? null,
          activityStage: draft?.activityStage ?? null,
        };
      }),
      current:
        current === null
          ? null
          : {
              reviewUnitId: current.reviewUnitId,
              promptId: current.promptId,
              activityKind: current.activityKind,
              activityStage: current.activityStage,
              prompt: current.prompt.prompt,
              expectedAnswer,
              workedSolution: expectedAnswer === null ? null : current.workedSolution,
              scoringRubric: expectedAnswer === null ? null : current.scoringRubric,
              grade,
              reviewState: projectSchedule(await store.readScheduleState(current.reviewUnitId)),
              scheduleChange,
            },
      summary: {
        sourceCount: snapshot.sourceDocuments.length,
        acceptedDraftCount: snapshot.generatedPromptDrafts.filter(
          (draft) => draft.validation.status === 'accepted',
        ).length,
        approvedReviewUnitCount: snapshot.reviewUnits.length,
        attemptCount: snapshot.attempts.length,
        lastOutcome: snapshot.attempts.at(-1)?.grade?.verdict ?? null,
        nextReviewUnitId,
      },
      pressureSignals,
    };
  }

  return {
    async start(): Promise<BetaCoachView> {
      await selectNext();
      return readView();
    },
    async ingestSource(input: BetaCoachSourceInput): Promise<BetaCoachView> {
      commands.push('save-source');
      await store.saveSourceDocument({
        id: input.id,
        kind: 'text',
        title: input.title,
        body: input.body,
        uri: null,
        permission: 'model-eligible',
        freshness: now(),
        createdAt: now(),
      });
      status = 'drafting';
      return readView();
    },
    async generate(sourceDocumentIds?: string[]): Promise<BetaCoachView> {
      commands.push('generate');
      const snapshot = store.snapshot();
      const ids = sourceDocumentIds ?? snapshot.sourceDocuments.map((source) => source.id);
      await runBetaGeneration(store, {
        runId: `coach-run-${snapshot.generationRuns.length + 1}`,
        sourceDocumentIds: ids,
        startedAt: now(),
        completedAt: now(),
        defaultDue: now() - 60_000,
      });
      status = 'drafting';
      return readView();
    },
    async approveAcceptedDrafts(): Promise<BetaCoachView> {
      commands.push('approve-accepted-drafts');
      const acceptedDrafts = store
        .snapshot()
        .generatedPromptDrafts.filter((draft) => draft.validation.status === 'accepted');
      for (const draft of acceptedDrafts) {
        await store.approveGeneratedPromptDraft(draft.id);
      }
      await selectNext();
      return readView();
    },
    async reveal(): Promise<BetaCoachView> {
      commands.push('reveal');
      const active = requireCurrent(current);
      if (status === 'graded') {
        return readView();
      }

      expectedAnswer = promptExpectedAnswer(active.prompt);
      status = 'revealed';
      return readView();
    },
    async submitAnswer(answer: string, responseTimeMs: number): Promise<BetaCoachView> {
      commands.push('grade/apply-review');
      const active = requireCurrent(current);
      if (status === 'graded') {
        return readView();
      }

      const priorSchedule = await store.readScheduleState(active.reviewUnitId);
      const review = await service.execute({
        kind: 'grade/apply-review',
        prompt: active.prompt,
        promptId: active.promptId,
        submittedAnswer: answer,
        responseTimeMs,
        idempotencyKey: `beta-coach:${active.reviewUnitId}:${active.promptId}:${answer}`,
      });

      expectedAnswer = review.grade.expectedAnswer;
      grade = {
        verdict: review.grade.verdict,
        rating: review.grade.rating,
        isCorrect: review.grade.isCorrect,
      };
      scheduleChange = {
        before: projectSchedule(priorSchedule),
        after: projectRequiredSchedule(review.scheduleState),
      };
      status = 'graded';
      return readView();
    },
    async next(): Promise<BetaCoachView> {
      await selectNext();
      return readView();
    },
    async view(): Promise<BetaCoachView> {
      return readView();
    },
  };
}

function findApprovedDraft(
  drafts: GeneratedPromptDraft[],
  reviewUnitId: ReviewUnitId,
): GeneratedPromptDraft | null {
  return drafts.find((draft) => draft.reviewUnitId === reviewUnitId) ?? null;
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

function projectSchedule(schedule: ScheduleState | null): ReviewStateProjection | null {
  if (schedule === null) {
    return null;
  }

  return projectRequiredSchedule(schedule);
}

function projectRequiredSchedule(schedule: ScheduleState): ReviewStateProjection {
  return {
    due: schedule.due,
    reps: schedule.reps,
    lapses: schedule.lapses,
    state: schedule.state,
    last_review: schedule.last_review,
  };
}

function requireCurrent(current: GeneratedPromptDraft | null): GeneratedPromptDraft {
  if (current === null) {
    throw new Error('Beta coach session has no active review unit');
  }

  return current;
}
