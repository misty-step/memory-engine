import { createMemoryService } from '../../service';
import {
  type ReviewStateProjection,
  type ReviewStateScheduleChange as ScheduleChange,
  projectReviewStateChange,
  projectReviewStateProjection,
} from '../../service/review-state-projection';
import type { GradeResult, Prompt, QueueCandidate, ReviewUnitId, ScheduleState } from '../../src';
import { runBetaGeneration } from '../beta-generation';
import {
  type BetaPersistenceStore,
  type GeneratedPromptDraft,
  type SourceDocument,
  createBetaPersistenceStore,
} from '../beta-store';

const defaultNow = Date.UTC(2026, 4, 22, 12, 0, 0);

export type BetaStudyOptions = {
  path: string;
  now?: () => number;
};

export type BetaStudyStatus = 'empty' | 'drafting' | 'answering' | 'revealed' | 'graded';

export type BetaStudyCurrent = {
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

export type BetaStudyDraftRow = {
  id: string;
  activityKind: GeneratedPromptDraft['activityKind'];
  activityStage: string;
  prompt: string;
  expectedAnswer: string;
  referenceText: string | null;
  validationStatus: GeneratedPromptDraft['validation']['status'];
  validationReasons: string[];
  workedSolution: string | null;
  approved: boolean;
};

export type BetaStudyQueueRow = {
  reviewUnitId: ReviewUnitId;
  due: number;
  reps: number;
  state: number | null;
  activityKind: GeneratedPromptDraft['activityKind'] | null;
  activityStage: string | null;
  conceptKey: string | null;
  progressionGroup: string | null;
};

export type BetaStudySummary = {
  sourceCount: number;
  acceptedDraftCount: number;
  approvedReviewUnitCount: number;
  generationFailureCount: number;
  attemptCount: number;
  lastOutcome: GradeResult['verdict'] | null;
  nextReviewUnitId: ReviewUnitId | null;
};

export type BetaStudyView = {
  status: BetaStudyStatus;
  sources: Pick<SourceDocument, 'id' | 'title' | 'kind' | 'createdAt'>[];
  drafts: BetaStudyDraftRow[];
  queue: BetaStudyQueueRow[];
  current: BetaStudyCurrent | null;
  generationFailures: string[];
  summary: BetaStudySummary;
  apiPressure: string[];
};

export type BetaStudySession = {
  start(): Promise<BetaStudyView>;
  addSource(input: BetaStudySourceInput): Promise<BetaStudyView>;
  generate(sourceDocumentIds?: string[]): Promise<BetaStudyView>;
  approveDraft(draftId: string): Promise<BetaStudyView>;
  rejectDraft(draftId: string, reason?: string): Promise<BetaStudyView>;
  reviseDraft(input: BetaStudyDraftRevision): Promise<BetaStudyView>;
  reveal(): Promise<BetaStudyView>;
  submitAnswer(answer: string, responseTimeMs: number): Promise<BetaStudyView>;
  next(): Promise<BetaStudyView>;
  view(): Promise<BetaStudyView>;
};

export type BetaStudyDraftRevision = {
  draftId: string;
  prompt: string;
  expectedAnswer: string;
};

export type BetaStudySourceInput = {
  id: string;
  title: string;
  body: string;
};

const apiPressure = [
  'Beta study still owns source creation, draft approval, reveal state, and mobile UI state.',
  'The service boundary is usable for queue selection and grade/apply-review without promoting persistence into src.',
  'Worked-solution display is activity metadata, not a kernel scheduling concern yet.',
];

export async function createBetaStudySession(options: BetaStudyOptions): Promise<BetaStudySession> {
  const now = options.now ?? (() => defaultNow);
  const store = await createBetaPersistenceStore(options.path);
  const service = createMemoryService({
    store,
    now,
    masteryPolicy: (schedule) => schedule.state === 2 && schedule.reps >= 3,
  });

  let current: GeneratedPromptDraft | null = null;
  let status: BetaStudyStatus =
    store.snapshot().sourceDocuments.length === 0 ? 'empty' : 'drafting';
  let expectedAnswer: string | null = null;
  let grade: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null = null;
  let scheduleChange: ScheduleChange | null = null;

  async function selectNext(): Promise<void> {
    const result = await service.execute({ kind: 'next-queue' });
    current = result.candidate === null ? null : findApprovedDraft(store, result.candidate);
    status = current === null ? 'drafting' : 'answering';
    expectedAnswer = null;
    grade = null;
    scheduleChange = null;
  }

  async function readView(): Promise<BetaStudyView> {
    const snapshot = store.snapshot();
    const queue = (await store.listQueueCandidates()).sort((left, right) => left.due - right.due);
    const nextReviewUnitId = queue[0]?.reviewUnitId ?? null;

    return {
      status,
      sources: snapshot.sourceDocuments.map((source) => ({
        id: source.id,
        title: source.title,
        kind: source.kind,
        createdAt: source.createdAt,
      })),
      drafts: snapshot.generatedPromptDrafts.map((draft) => draftRow(snapshot, draft)),
      queue: queue.map((candidate) => queueRow(snapshot.generatedPromptDrafts, candidate)),
      current:
        current === null
          ? null
          : currentView(
              current,
              await store.readScheduleState(current.reviewUnitId),
              expectedAnswer,
              grade,
              scheduleChange,
            ),
      generationFailures: latestGenerationFailures(snapshot),
      summary: {
        sourceCount: snapshot.sourceDocuments.length,
        acceptedDraftCount: snapshot.generatedPromptDrafts.filter(
          (draft) => draft.validation.status === 'accepted',
        ).length,
        approvedReviewUnitCount: snapshot.reviewUnits.length,
        generationFailureCount: latestGenerationFailures(snapshot).length,
        attemptCount: snapshot.attempts.length,
        lastOutcome: snapshot.attempts.at(-1)?.grade?.verdict ?? null,
        nextReviewUnitId,
      },
      apiPressure,
    };
  }

  return {
    async start(): Promise<BetaStudyView> {
      await selectNext();
      return readView();
    },
    async addSource(input: BetaStudySourceInput): Promise<BetaStudyView> {
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
    async generate(sourceDocumentIds?: string[]): Promise<BetaStudyView> {
      const snapshot = store.snapshot();
      const ids = sourceDocumentIds ?? snapshot.sourceDocuments.map((source) => source.id);
      await runBetaGeneration(store, {
        runId: `study-run-${snapshot.generationRuns.length + 1}`,
        sourceDocumentIds: ids,
        startedAt: now(),
        completedAt: now(),
        defaultDue: now() - 60_000,
      });
      status = 'drafting';
      return readView();
    },
    async approveDraft(draftId: string): Promise<BetaStudyView> {
      await store.approveGeneratedPromptDraft(draftId);
      if (hasApprovableDrafts(store)) {
        status = 'drafting';
        current = null;
        expectedAnswer = null;
        grade = null;
        scheduleChange = null;
      } else {
        await selectNext();
      }
      return readView();
    },
    async rejectDraft(draftId: string, reason = 'Skipped by learner'): Promise<BetaStudyView> {
      await store.rejectGeneratedPromptDraft(draftId, reason);
      status = 'drafting';
      current = null;
      expectedAnswer = null;
      grade = null;
      scheduleChange = null;
      return readView();
    },
    async reviseDraft(input: BetaStudyDraftRevision): Promise<BetaStudyView> {
      await store.reviseGeneratedPromptDraft(input.draftId, {
        prompt: input.prompt,
        expectedAnswer: input.expectedAnswer,
      });
      status = 'drafting';
      current = null;
      expectedAnswer = null;
      grade = null;
      scheduleChange = null;
      return readView();
    },
    async reveal(): Promise<BetaStudyView> {
      const active = requireCurrent(current);
      if (status === 'graded') {
        return readView();
      }

      expectedAnswer = promptExpectedAnswer(active.prompt);
      status = 'revealed';
      return readView();
    },
    async submitAnswer(answer: string, responseTimeMs: number): Promise<BetaStudyView> {
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
        idempotencyKey: `beta-study:${active.reviewUnitId}:${active.promptId}:${answer}`,
      });

      expectedAnswer = review.grade.expectedAnswer;
      grade = {
        verdict: review.grade.verdict,
        rating: review.grade.rating,
        isCorrect: review.grade.isCorrect,
      };
      scheduleChange = projectReviewStateChange(priorSchedule, review.scheduleState);
      status = 'graded';
      return readView();
    },
    async next(): Promise<BetaStudyView> {
      await selectNext();
      return readView();
    },
    async view(): Promise<BetaStudyView> {
      return readView();
    },
  };
}

function draftRow(
  snapshot: ReturnType<BetaPersistenceStore['snapshot']>,
  draft: GeneratedPromptDraft,
): BetaStudyDraftRow {
  const referenceSpanId = draft.referenceSpanIds[0] ?? null;
  const referenceSpan =
    referenceSpanId === null
      ? null
      : snapshot.referenceSpans.find((span) => span.id === referenceSpanId);
  return {
    id: draft.id,
    activityKind: draft.activityKind,
    activityStage: draft.activityStage,
    prompt: draft.prompt.prompt,
    expectedAnswer: promptExpectedAnswer(draft.prompt),
    referenceText: referenceSpan?.text ?? null,
    validationStatus: draft.validation.status,
    validationReasons: [...draft.validation.reasons],
    workedSolution: draft.workedSolution,
    approved: snapshot.reviewUnits.some((unit) => unit.generatedPromptDraftId === draft.id),
  };
}

function latestGenerationFailures(
  snapshot: ReturnType<BetaPersistenceStore['snapshot']>,
): string[] {
  const latest = snapshot.generationRuns
    .slice()
    .sort((left, right) => right.startedAt - left.startedAt)[0];
  return latest?.validationFailures ?? [];
}

function queueRow(drafts: GeneratedPromptDraft[], candidate: QueueCandidate): BetaStudyQueueRow {
  const draft = drafts.find((item) => item.reviewUnitId === candidate.reviewUnitId) ?? null;

  return {
    reviewUnitId: candidate.reviewUnitId,
    due: candidate.due,
    reps: candidate.scheduleState?.reps ?? 0,
    state: candidate.scheduleState?.state ?? null,
    activityKind: draft?.activityKind ?? null,
    activityStage: draft?.activityStage ?? null,
    conceptKey: candidate.conceptKey,
    progressionGroup: candidate.progression?.progressionGroup ?? null,
  };
}

function currentView(
  draft: GeneratedPromptDraft,
  schedule: ScheduleState | null,
  expectedAnswerValue: string | null,
  gradeValue: Pick<GradeResult, 'verdict' | 'rating' | 'isCorrect'> | null,
  scheduleChangeValue: ScheduleChange | null,
): BetaStudyCurrent {
  return {
    reviewUnitId: draft.reviewUnitId,
    promptId: draft.promptId,
    activityKind: draft.activityKind,
    activityStage: draft.activityStage,
    prompt: draft.prompt.prompt,
    expectedAnswer: expectedAnswerValue,
    workedSolution: expectedAnswerValue === null ? null : draft.workedSolution,
    scoringRubric: expectedAnswerValue === null ? null : draft.scoringRubric,
    grade: gradeValue,
    reviewState: projectReviewStateProjection(schedule),
    scheduleChange: scheduleChangeValue,
  };
}

function findApprovedDraft(
  store: BetaPersistenceStore,
  candidate: QueueCandidate,
): GeneratedPromptDraft | null {
  const snapshot = store.snapshot();
  const reviewUnit = snapshot.reviewUnits.find(
    (unit) => unit.reviewUnitId === candidate.reviewUnitId,
  );
  if (
    reviewUnit?.generatedPromptDraftId === undefined ||
    reviewUnit.generatedPromptDraftId === null
  ) {
    return null;
  }
  return (
    snapshot.generatedPromptDrafts.find(
      (draft) => draft.id === reviewUnit.generatedPromptDraftId,
    ) ?? null
  );
}

function hasApprovableDrafts(store: BetaPersistenceStore): boolean {
  const snapshot = store.snapshot();
  const approvedDraftIds = new Set(
    snapshot.reviewUnits
      .map((unit) => unit.generatedPromptDraftId)
      .filter((id): id is string => id !== null && id !== undefined),
  );
  return snapshot.generatedPromptDrafts.some(
    (draft) => draft.validation.status === 'accepted' && !approvedDraftIds.has(draft.id),
  );
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

function requireCurrent(current: GeneratedPromptDraft | null): GeneratedPromptDraft {
  if (current === null) {
    throw new Error('Beta study session has no active review unit');
  }

  return current;
}
