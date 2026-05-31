import { mkdir, rename } from 'node:fs/promises';
import { dirname } from 'node:path';

import type { MemoryServiceStore, ServiceAttemptRecord } from '../../service';
import type { Prompt, QueueCandidate, ReviewUnitId, ScheduleState } from '../../src';

export type SourceDocumentKind = 'text' | 'link' | 'file' | 'image' | 'video-transcript';

export type SourcePermission = 'local-only' | 'model-eligible';

export type SourceDocument = {
  id: string;
  kind: SourceDocumentKind;
  title: string;
  body: string | null;
  uri: string | null;
  permission: SourcePermission;
  freshness: number | null;
  createdAt: number;
};

export type ReferenceSpan = {
  id: string;
  sourceDocumentId: string;
  label: string;
  text: string;
  locator: string;
  createdAt: number;
};

export type GeneratedPromptValidation = {
  status: 'pending' | 'accepted' | 'rejected';
  reasons: string[];
};

export type GeneratedPromptModel = {
  provider: string;
  name: string;
  version: string;
};

export type GeneratedLearningActivityKind = 'quiz' | 'exercise';

export type PersistedQueueCandidate = Omit<QueueCandidate, 'scheduleState'>;

export type GeneratedPromptDraft = {
  id: string;
  sourceDocumentIds: string[];
  referenceSpanIds: string[];
  generationRunId: string | null;
  reviewUnitId: ReviewUnitId;
  promptId: string;
  prompt: Prompt;
  queue: PersistedQueueCandidate;
  activityKind: GeneratedLearningActivityKind;
  activityStage: string;
  workedSolution: string | null;
  scoringRubric: string | null;
  model: GeneratedPromptModel;
  validation: GeneratedPromptValidation;
  critiqueNotes: string[];
  createdAt: number;
};

export type GenerationRun = {
  id: string;
  sourceDocumentIds: string[];
  draftIds: string[];
  provider: string;
  model: string;
  startedAt: number;
  completedAt: number | null;
  validationFailures: string[];
};

export type BetaReviewUnitRecord = {
  reviewUnitId: ReviewUnitId;
  promptId: string;
  prompt: Prompt;
  queue: PersistedQueueCandidate;
  referenceSpanIds: string[];
  generatedPromptDraftId: string | null;
  createdAt: number;
};

export type ScheduleRecord = {
  reviewUnitId: ReviewUnitId;
  state: ScheduleState;
};

type AppliedReviewReceipt = {
  key: string;
  attempt: ServiceAttemptRecord;
  expectedPriorScheduleState: ScheduleState | null;
  scheduleState: ScheduleState;
};

export type BetaStoreSnapshot = {
  version: 1;
  sourceDocuments: SourceDocument[];
  referenceSpans: ReferenceSpan[];
  generatedPromptDrafts: GeneratedPromptDraft[];
  reviewUnits: BetaReviewUnitRecord[];
  schedules: ScheduleRecord[];
  attempts: ServiceAttemptRecord[];
  generationRuns: GenerationRun[];
  appliedReviews: AppliedReviewReceipt[];
};

export type ApproveGeneratedPromptDraftOptions = {
  initialScheduleState?: ScheduleState | null;
};

export type ReviseGeneratedPromptDraftInput = {
  prompt: string;
  expectedAnswer: string;
};

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

export class BetaPersistenceStore implements MemoryServiceStore {
  readonly path: string;

  private data: BetaStoreSnapshot;
  private failNextCommit = false;

  private constructor(path: string, snapshot: BetaStoreSnapshot) {
    this.path = path;
    this.data = snapshot;
  }

  static async open(path: string): Promise<BetaPersistenceStore> {
    const snapshot = await loadSnapshot(path);
    return new BetaPersistenceStore(path, snapshot);
  }

  snapshot(): BetaStoreSnapshot {
    return cloneSnapshot(this.data);
  }

  failNextCommitForTest(): void {
    this.failNextCommit = true;
  }

  async saveSourceDocument(document: SourceDocument): Promise<SourceDocument> {
    assertNonBlank(document.id, 'Source document id');
    assertNonBlank(document.title, 'Source document title');
    const next = cloneSnapshot(this.data);
    upsertById(next.sourceDocuments, document);
    await this.commit(next);
    return document;
  }

  async saveReferenceSpan(reference: ReferenceSpan): Promise<ReferenceSpan> {
    assertNonBlank(reference.id, 'Reference span id');
    assertKnownSource(this.data, reference.sourceDocumentId);
    assertNonBlank(reference.text, 'Reference span text');
    const next = cloneSnapshot(this.data);
    upsertById(next.referenceSpans, reference);
    await this.commit(next);
    return reference;
  }

  async saveGenerationRun(run: GenerationRun): Promise<GenerationRun> {
    assertNonBlank(run.id, 'Generation run id');
    for (const sourceDocumentId of run.sourceDocumentIds) {
      assertKnownSource(this.data, sourceDocumentId);
    }
    const next = cloneSnapshot(this.data);
    upsertById(next.generationRuns, run);
    await this.commit(next);
    return run;
  }

  async saveGeneratedPromptDraft(draft: GeneratedPromptDraft): Promise<GeneratedPromptDraft> {
    assertDraftContract(this.data, draft);
    const next = cloneSnapshot(this.data);
    upsertById(next.generatedPromptDrafts, draft);
    await this.commit(next);
    return draft;
  }

  async approveGeneratedPromptDraft(
    draftId: string,
    options: ApproveGeneratedPromptDraftOptions = {},
  ): Promise<BetaReviewUnitRecord> {
    const draft = findById(this.data.generatedPromptDrafts, draftId);
    if (draft === undefined) {
      throw new Error(`Unknown generated prompt draft: ${draftId}`);
    }
    if (draft.validation.status !== 'accepted') {
      throw new Error('Only accepted generated prompt drafts can be approved');
    }
    if (
      draft.generationRunId === null ||
      findById(this.data.generationRuns, draft.generationRunId) === undefined
    ) {
      throw new Error('Accepted generated prompt drafts require a generation run before approval');
    }

    const reviewUnit: BetaReviewUnitRecord = {
      reviewUnitId: draft.reviewUnitId,
      promptId: draft.promptId,
      prompt: draft.prompt,
      queue: draft.queue,
      referenceSpanIds: [...draft.referenceSpanIds],
      generatedPromptDraftId: draft.id,
      createdAt: draft.createdAt,
    };
    const next = cloneSnapshot(this.data);
    upsertReviewUnit(next.reviewUnits, reviewUnit);
    applyScheduleRecord(next, draft.reviewUnitId, options.initialScheduleState ?? null);
    await this.commit(next);
    return reviewUnit;
  }

  async rejectGeneratedPromptDraft(draftId: string, reason: string): Promise<GeneratedPromptDraft> {
    assertNonBlank(reason, 'Generated prompt rejection reason');
    const draft = findById(this.data.generatedPromptDrafts, draftId);
    if (draft === undefined) {
      throw new Error(`Unknown generated prompt draft: ${draftId}`);
    }
    if (this.data.reviewUnits.some((unit) => unit.generatedPromptDraftId === draft.id)) {
      throw new Error('Approved generated prompt drafts cannot be rejected');
    }
    if (draft.validation.status === 'rejected') {
      return draft;
    }

    const rejected: GeneratedPromptDraft = {
      ...draft,
      validation: {
        status: 'rejected',
        reasons: [reason],
      },
      critiqueNotes: [`Rejected: ${reason}`],
    };
    const next = cloneSnapshot(this.data);
    upsertById(next.generatedPromptDrafts, rejected);
    await this.commit(next);
    return rejected;
  }

  async reviseGeneratedPromptDraft(
    draftId: string,
    revision: ReviseGeneratedPromptDraftInput,
  ): Promise<GeneratedPromptDraft> {
    assertNonBlank(revision.prompt, 'Generated prompt revision prompt');
    assertNonBlank(revision.expectedAnswer, 'Generated prompt revision answer');
    const draft = findById(this.data.generatedPromptDrafts, draftId);
    if (draft === undefined) {
      throw new Error(`Unknown generated prompt draft: ${draftId}`);
    }
    if (draft.validation.status !== 'accepted') {
      throw new Error('Only accepted generated prompt drafts can be revised');
    }
    if (this.data.reviewUnits.some((unit) => unit.generatedPromptDraftId === draft.id)) {
      throw new Error('Approved generated prompt drafts cannot be revised');
    }

    const revised = reviseDraftPrompt(draft, revision);
    const next = cloneSnapshot(this.data);
    upsertById(next.generatedPromptDrafts, revised);
    await this.commit(next);
    return revised;
  }

  async saveReviewUnit(reviewUnit: BetaReviewUnitRecord): Promise<BetaReviewUnitRecord> {
    assertReviewUnitContract(this.data, reviewUnit);
    const next = cloneSnapshot(this.data);
    upsertReviewUnit(next.reviewUnits, reviewUnit);
    await this.commit(next);
    return reviewUnit;
  }

  async setScheduleState(
    reviewUnitId: ReviewUnitId,
    scheduleState: ScheduleState | null,
  ): Promise<void> {
    assertKnownReviewUnit(this.data, reviewUnitId);
    const next = cloneSnapshot(this.data);
    applyScheduleRecord(next, reviewUnitId, scheduleState);
    await this.commit(next);
  }

  async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
    assertAttemptContract(this.data, attempt);
    const next = cloneSnapshot(this.data);
    next.attempts.push(attempt);
    await this.commit(next);
  }

  async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
    assertKnownReviewUnit(this.data, reviewUnitId);
    return findSchedule(this.data, reviewUnitId)?.state ?? null;
  }

  async applyReview(
    reviewUnitId: ReviewUnitId,
    attempt: ServiceAttemptRecord,
    scheduleState: ScheduleState,
    expectedPriorScheduleState: ScheduleState | null,
  ): Promise<void> {
    assertKnownReviewUnit(this.data, reviewUnitId);
    assertAttemptContract(this.data, attempt);
    if (reviewUnitId !== attempt.reviewUnitId) {
      throw new Error('Applied review unit must match the attempt review unit');
    }
    if (scheduleState.last_review !== attempt.occurredAt) {
      throw new Error('Schedule last_review must match the attempt timestamp');
    }

    const key = appliedReviewKey(attempt);
    if (this.data.appliedReviews.some((receipt) => receipt.key === key)) {
      throw new DuplicateAppliedReviewError(key);
    }

    const currentSchedule = findSchedule(this.data, reviewUnitId)?.state ?? null;
    if (!scheduleStatesEqual(currentSchedule, expectedPriorScheduleState)) {
      throw new StaleScheduleWriteError(reviewUnitId);
    }

    const next = cloneSnapshot(this.data);
    next.attempts.push(attempt);
    applyScheduleRecord(next, reviewUnitId, scheduleState);
    next.appliedReviews.push({
      key,
      attempt,
      expectedPriorScheduleState,
      scheduleState,
    });
    await this.commit(next);
  }

  async listQueueCandidates(): Promise<QueueCandidate[]> {
    return this.data.reviewUnits.map((reviewUnit) => {
      const schedule = findSchedule(this.data, reviewUnit.reviewUnitId)?.state ?? null;
      return {
        ...reviewUnit.queue,
        scheduleState: schedule,
        due: schedule?.due ?? reviewUnit.queue.due,
      };
    });
  }

  private async commit(next: BetaStoreSnapshot): Promise<void> {
    if (this.failNextCommit) {
      this.failNextCommit = false;
      throw new Error('Injected beta store commit failure');
    }

    await mkdir(dirname(this.path), { recursive: true });
    const temporaryPath = `${this.path}.${Date.now()}.${Math.random().toString(36).slice(2)}.tmp`;
    await Bun.write(temporaryPath, `${JSON.stringify(next, null, 2)}\n`);
    await rename(temporaryPath, this.path);
    this.data = next;
  }
}

export async function createBetaPersistenceStore(path: string): Promise<BetaPersistenceStore> {
  return BetaPersistenceStore.open(path);
}

function emptySnapshot(): BetaStoreSnapshot {
  return {
    version: 1,
    sourceDocuments: [],
    referenceSpans: [],
    generatedPromptDrafts: [],
    reviewUnits: [],
    schedules: [],
    attempts: [],
    generationRuns: [],
    appliedReviews: [],
  };
}

async function loadSnapshot(path: string): Promise<BetaStoreSnapshot> {
  const file = Bun.file(path);
  if (!(await file.exists())) {
    return emptySnapshot();
  }

  const parsed = JSON.parse(await file.text()) as BetaStoreSnapshot;
  if (parsed.version !== 1) {
    throw new Error(`Unsupported beta store snapshot version: ${String(parsed.version)}`);
  }
  return parsed;
}

function cloneSnapshot(snapshot: BetaStoreSnapshot): BetaStoreSnapshot {
  return structuredClone(snapshot);
}

function assertNonBlank(value: string, label: string): void {
  if (value.trim().length === 0) {
    throw new Error(`${label} must not be blank`);
  }
}

function assertKnownSource(snapshot: BetaStoreSnapshot, sourceDocumentId: string): void {
  if (findById(snapshot.sourceDocuments, sourceDocumentId) === undefined) {
    throw new Error(`Unknown source document: ${sourceDocumentId}`);
  }
}

function assertKnownReference(snapshot: BetaStoreSnapshot, referenceSpanId: string): void {
  if (findById(snapshot.referenceSpans, referenceSpanId) === undefined) {
    throw new Error(`Unknown reference span: ${referenceSpanId}`);
  }
}

function assertKnownReviewUnit(snapshot: BetaStoreSnapshot, reviewUnitId: ReviewUnitId): void {
  if (snapshot.reviewUnits.every((unit) => unit.reviewUnitId !== reviewUnitId)) {
    throw new Error(`Unknown review unit: ${reviewUnitId}`);
  }
}

function assertAttemptContract(snapshot: BetaStoreSnapshot, attempt: ServiceAttemptRecord): void {
  assertKnownReviewUnit(snapshot, attempt.reviewUnitId);
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

function assertDraftContract(snapshot: BetaStoreSnapshot, draft: GeneratedPromptDraft): void {
  assertNonBlank(draft.id, 'Generated prompt draft id');
  assertNonBlank(draft.promptId, 'Generated prompt draft prompt id');
  assertNonBlank(draft.activityStage, 'Generated prompt draft activity stage');
  assertNonBlank(draft.model.provider, 'Generated prompt draft provider');
  assertNonBlank(draft.model.name, 'Generated prompt draft model');
  assertNonBlank(draft.model.version, 'Generated prompt draft model version');
  if (draft.sourceDocumentIds.length === 0) {
    throw new Error('Generated prompt drafts require at least one source document');
  }
  if (draft.referenceSpanIds.length === 0) {
    throw new Error('Generated prompt drafts require at least one reference span');
  }
  if (
    draft.prompt.reviewUnitId !== draft.reviewUnitId ||
    draft.queue.reviewUnitId !== draft.reviewUnitId
  ) {
    throw new Error('Generated prompt draft review unit ids must match');
  }
  for (const sourceDocumentId of draft.sourceDocumentIds) {
    assertKnownSource(snapshot, sourceDocumentId);
  }
  for (const referenceSpanId of draft.referenceSpanIds) {
    assertKnownReference(snapshot, referenceSpanId);
  }
}

function assertReviewUnitContract(
  snapshot: BetaStoreSnapshot,
  reviewUnit: BetaReviewUnitRecord,
): void {
  if (
    reviewUnit.prompt.reviewUnitId !== reviewUnit.reviewUnitId ||
    reviewUnit.queue.reviewUnitId !== reviewUnit.reviewUnitId
  ) {
    throw new Error('Review unit ids must match prompt and queue ids');
  }
  for (const referenceSpanId of reviewUnit.referenceSpanIds) {
    assertKnownReference(snapshot, referenceSpanId);
  }
  if (
    reviewUnit.generatedPromptDraftId !== null &&
    findById(snapshot.generatedPromptDrafts, reviewUnit.generatedPromptDraftId) === undefined
  ) {
    throw new Error(`Unknown generated prompt draft: ${reviewUnit.generatedPromptDraftId}`);
  }
}

function upsertById<T extends { id: string }>(items: T[], item: T): void {
  const index = items.findIndex((candidate) => candidate.id === item.id);
  if (index === -1) {
    items.push(item);
    return;
  }
  items[index] = item;
}

function upsertReviewUnit(items: BetaReviewUnitRecord[], item: BetaReviewUnitRecord): void {
  const index = items.findIndex((candidate) => candidate.reviewUnitId === item.reviewUnitId);
  if (index === -1) {
    items.push(item);
    return;
  }
  items[index] = item;
}

function findById<T extends { id: string }>(items: readonly T[], id: string): T | undefined {
  return items.find((item) => item.id === id);
}

function findSchedule(
  snapshot: BetaStoreSnapshot,
  reviewUnitId: ReviewUnitId,
): ScheduleRecord | undefined {
  return snapshot.schedules.find((schedule) => schedule.reviewUnitId === reviewUnitId);
}

function applyScheduleRecord(
  snapshot: BetaStoreSnapshot,
  reviewUnitId: ReviewUnitId,
  scheduleState: ScheduleState | null,
): void {
  const existingIndex = snapshot.schedules.findIndex(
    (schedule) => schedule.reviewUnitId === reviewUnitId,
  );

  if (scheduleState === null) {
    if (existingIndex !== -1) {
      snapshot.schedules.splice(existingIndex, 1);
    }
    return;
  }

  const record = { reviewUnitId, state: scheduleState };
  if (existingIndex === -1) {
    snapshot.schedules.push(record);
    return;
  }
  snapshot.schedules[existingIndex] = record;
}

function reviseDraftPrompt(
  draft: GeneratedPromptDraft,
  revision: ReviseGeneratedPromptDraftInput,
): GeneratedPromptDraft {
  return {
    ...draft,
    prompt: revisePrompt(draft.prompt, revision),
    critiqueNotes: [...draft.critiqueNotes, 'Learner edited generated wording before approval.'],
  };
}

function revisePrompt(prompt: Prompt, revision: ReviseGeneratedPromptDraftInput): Prompt {
  switch (prompt.kind) {
    case 'mcq':
      return {
        ...prompt,
        prompt: revision.prompt,
        choices: uniqueChoices([revision.expectedAnswer, ...prompt.choices]),
        correctChoice: revision.expectedAnswer,
      };
    case 'boolean':
      return {
        ...prompt,
        prompt: revision.prompt,
        correctAnswer: revision.expectedAnswer.toLowerCase() === 'true',
      };
    case 'cloze':
    case 'shortAnswer':
    case 'recitation':
      return {
        ...prompt,
        prompt: revision.prompt,
        acceptedAnswers: [revision.expectedAnswer],
      };
  }
}

function uniqueChoices(values: string[]): string[] {
  const seen = new Set<string>();
  const uniqueValues: string[] = [];
  for (const value of values) {
    const key = value.trim().toLowerCase();
    if (key.length === 0 || seen.has(key)) {
      continue;
    }
    seen.add(key);
    uniqueValues.push(value);
  }
  return uniqueValues;
}

function appliedReviewKey(attempt: ServiceAttemptRecord): string {
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

function scheduleStatesEqual(left: ScheduleState | null, right: ScheduleState | null): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}
