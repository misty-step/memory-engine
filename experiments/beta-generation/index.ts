import type { Prompt, ReviewUnitId } from '../../src';
import type {
  BetaPersistenceStore,
  GeneratedLearningActivityKind,
  GeneratedPromptDraft,
  GeneratedPromptModel,
  SourceDocument,
} from '../beta-store';

export type BetaGenerationRequest = {
  runId: string;
  sourceDocumentIds: string[];
  startedAt: number;
  completedAt?: number;
  defaultDue: number;
  model?: GeneratedPromptModel;
};

export type BetaGenerationResult = {
  runId: string;
  draftIds: string[];
  acceptedDraftIds: string[];
  rejectedDraftIds: string[];
  validationFailures: string[];
};

type ParsedCandidate = {
  source: SourceDocument;
  blockIndex: number;
  activityKind: GeneratedLearningActivityKind;
  activityStage: string;
  concept: string;
  question: string;
  answer: string;
  reference: string | null;
  distractors: string[];
  workedSolution: string | null;
  scoringRubric: string | null;
  unsupported: boolean;
};

const defaultModel: GeneratedPromptModel = {
  provider: 'fixture',
  name: 'deterministic-beta-generator',
  version: 'v1',
};

export async function runBetaGeneration(
  store: BetaPersistenceStore,
  request: BetaGenerationRequest,
): Promise<BetaGenerationResult> {
  const model = request.model ?? defaultModel;
  const snapshot = store.snapshot();
  const sources = request.sourceDocumentIds.map((sourceDocumentId) =>
    requireSource(snapshot.sourceDocuments, sourceDocumentId),
  );
  const validationFailures: string[] = [];
  const draftIds: string[] = [];
  const acceptedDraftIds: string[] = [];
  const rejectedDraftIds: string[] = [];

  await store.saveGenerationRun({
    id: request.runId,
    sourceDocumentIds: request.sourceDocumentIds,
    draftIds: [],
    provider: model.provider,
    model: model.name,
    startedAt: request.startedAt,
    completedAt: null,
    validationFailures: [],
  });

  const seenSignatures = new Set(existingDraftSignatures(snapshot.generatedPromptDrafts));
  const previousUnitByGroup = new Map<string, ReviewUnitId>();
  for (const source of sources) {
    const candidates = parseSourceDocument(source, validationFailures);
    for (const candidate of candidates) {
      if (candidate.reference === null) {
        validationFailures.push(
          `${source.id} block ${candidate.blockIndex}: generated drafts require source provenance`,
        );
        continue;
      }

      const signature = candidateSignature(candidate);
      const duplicate = seenSignatures.has(signature);
      seenSignatures.add(signature);

      const referenceSpan = await store.saveReferenceSpan({
        id: generatedId(request.runId, 'ref', candidate),
        sourceDocumentId: source.id,
        label: `${candidate.concept} source evidence`,
        text: candidate.reference,
        locator: `block:${candidate.blockIndex}`,
        createdAt: request.startedAt,
      });
      const progressionGroup = slug(candidate.concept);
      const requires = previousUnitByGroup.get(progressionGroup);
      const draft = await store.saveGeneratedPromptDraft(
        buildDraft(candidate, {
          runId: request.runId,
          referenceSpanId: referenceSpan.id,
          model,
          due: request.defaultDue,
          createdAt: request.startedAt,
          duplicate,
          requires: requires === undefined ? [] : [requires],
        }),
      );
      if (draft.validation.status === 'accepted') {
        previousUnitByGroup.set(progressionGroup, draft.reviewUnitId);
      }

      draftIds.push(draft.id);
      if (draft.validation.status === 'accepted') {
        acceptedDraftIds.push(draft.id);
      } else {
        rejectedDraftIds.push(draft.id);
      }
    }
  }

  await store.saveGenerationRun({
    id: request.runId,
    sourceDocumentIds: request.sourceDocumentIds,
    draftIds,
    provider: model.provider,
    model: model.name,
    startedAt: request.startedAt,
    completedAt: request.completedAt ?? request.startedAt,
    validationFailures,
  });

  return {
    runId: request.runId,
    draftIds,
    acceptedDraftIds,
    rejectedDraftIds,
    validationFailures,
  };
}

function requireSource(sources: SourceDocument[], sourceDocumentId: string): SourceDocument {
  const source = sources.find((candidate) => candidate.id === sourceDocumentId);
  if (source === undefined) {
    throw new Error(`Unknown source document: ${sourceDocumentId}`);
  }
  if (source.body === null || source.body.trim().length === 0) {
    throw new Error(`Source document has no text body: ${sourceDocumentId}`);
  }
  return source;
}

function parseSourceDocument(
  source: SourceDocument,
  validationFailures: string[],
): ParsedCandidate[] {
  const body = source.body ?? '';
  return body
    .split(/\n\s*\n/)
    .map((block, index) => parseCandidateBlock(source, block, index + 1, validationFailures))
    .filter((candidate): candidate is ParsedCandidate => candidate !== null);
}

function parseCandidateBlock(
  source: SourceDocument,
  block: string,
  blockIndex: number,
  validationFailures: string[],
): ParsedCandidate | null {
  const fields = parseFields(block);
  const concept = fields.get('concept');
  const question = fields.get('question');
  const answer = fields.get('answer');

  if (concept === undefined || question === undefined || answer === undefined) {
    validationFailures.push(
      `${source.id} block ${blockIndex}: concept, question, and answer are required`,
    );
    return null;
  }

  return {
    source,
    blockIndex,
    activityKind: parseActivityKind(fields.get('activity')),
    activityStage: fields.get('stage') ?? 'recognition',
    concept,
    question,
    answer,
    reference: fields.get('reference') ?? null,
    distractors: splitList(fields.get('distractors')),
    workedSolution: fields.get('worked solution') ?? fields.get('worked') ?? null,
    scoringRubric: fields.get('scoring rubric') ?? fields.get('rubric') ?? null,
    unsupported:
      parseBoolean(fields.get('unsupported')) || parseBoolean(fields.get('supported')) === false,
  };
}

function parseFields(block: string): Map<string, string> {
  const fields = new Map<string, string>();
  for (const rawLine of block.split('\n')) {
    const line = rawLine.trim();
    if (line.length === 0) {
      continue;
    }
    const separator = line.indexOf(':');
    if (separator === -1) {
      continue;
    }
    const key = line.slice(0, separator).trim().toLowerCase();
    const value = line.slice(separator + 1).trim();
    if (key.length > 0 && value.length > 0) {
      fields.set(key, value);
    }
  }
  return fields;
}

function parseActivityKind(value: string | undefined): GeneratedLearningActivityKind {
  return value?.toLowerCase() === 'exercise' ? 'exercise' : 'quiz';
}

function parseBoolean(value: string | undefined): boolean | null {
  if (value === undefined) {
    return null;
  }
  const normalized = value.toLowerCase();
  if (normalized === 'true' || normalized === 'yes') {
    return true;
  }
  if (normalized === 'false' || normalized === 'no') {
    return false;
  }
  return null;
}

function splitList(value: string | undefined): string[] {
  if (value === undefined) {
    return [];
  }
  return value
    .split(',')
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

function buildDraft(
  candidate: ParsedCandidate,
  context: {
    runId: string;
    referenceSpanId: string;
    model: GeneratedPromptModel;
    due: number;
    createdAt: number;
    duplicate: boolean;
    requires: ReviewUnitId[];
  },
): GeneratedPromptDraft {
  const unitId = reviewUnitId(generatedId('generated', candidate.activityKind, candidate));
  const reasons = validationReasons(candidate, context.duplicate);
  const status = reasons.length === 0 ? 'accepted' : 'rejected';

  return {
    id: generatedId(context.runId, 'draft', candidate),
    sourceDocumentIds: [candidate.source.id],
    referenceSpanIds: [context.referenceSpanId],
    generationRunId: context.runId,
    reviewUnitId: unitId,
    promptId: `${unitId}-prompt`,
    prompt: buildPrompt(candidate, unitId),
    queue: {
      reviewUnitId: unitId,
      due: context.due,
      progression: {
        progressionGroup: slug(candidate.concept),
        stageOrder: stageOrder(candidate.activityStage, candidate.activityKind),
        requires: context.requires,
        supersedes: [],
      },
      conceptKey: slug(candidate.concept),
      sourceKey: candidate.source.id,
      domainKey: candidate.source.kind,
    },
    activityKind: candidate.activityKind,
    activityStage: candidate.activityStage,
    workedSolution: candidate.workedSolution,
    scoringRubric: candidate.scoringRubric,
    model: context.model,
    validation: {
      status,
      reasons,
    },
    critiqueNotes:
      status === 'accepted'
        ? [`Grounded in ${context.referenceSpanId}.`]
        : reasons.map((reason) => `Rejected: ${reason}`),
    createdAt: context.createdAt,
  };
}

function buildPrompt(candidate: ParsedCandidate, reviewUnitId: ReviewUnitId): Prompt {
  if (candidate.activityKind === 'quiz' && candidate.distractors.length >= 2) {
    return {
      kind: 'mcq',
      reviewUnitId,
      prompt: candidate.question,
      choices: shuffleChoices(
        unique([candidate.answer, ...candidate.distractors]),
        candidate.answer,
      ),
      correctChoice: candidate.answer,
    };
  }

  return {
    kind: candidate.activityKind === 'exercise' ? 'recitation' : 'shortAnswer',
    reviewUnitId,
    prompt: candidate.question,
    acceptedAnswers: [candidate.answer],
    equivalenceGroups: [],
    ignoredTokens: ['.', ',', ';', ':'],
  };
}

function validationReasons(candidate: ParsedCandidate, duplicate: boolean): string[] {
  const reasons: string[] = [];
  if (candidate.unsupported) {
    reasons.push('Unsupported by cited source material');
  }
  if (duplicate) {
    reasons.push('Duplicate-ish generated draft');
  }
  if (
    candidate.activityKind === 'exercise' &&
    candidate.workedSolution === null &&
    candidate.scoringRubric === null
  ) {
    reasons.push('Exercises require a worked solution or scoring rubric');
  }
  return reasons;
}

function existingDraftSignatures(drafts: GeneratedPromptDraft[]): string[] {
  return drafts.map((draft) =>
    [
      draft.queue.conceptKey ?? '',
      draft.prompt.prompt.toLowerCase(),
      expectedAnswer(draft.prompt).toLowerCase(),
    ].join('\u0000'),
  );
}

function candidateSignature(candidate: ParsedCandidate): string {
  return [
    slug(candidate.concept),
    candidate.question.toLowerCase(),
    candidate.answer.toLowerCase(),
  ].join('\u0000');
}

function expectedAnswer(prompt: Prompt): string {
  switch (prompt.kind) {
    case 'mcq':
      return prompt.correctChoice;
    case 'boolean':
      return String(prompt.correctAnswer);
    case 'cloze':
    case 'shortAnswer':
    case 'recitation':
      return prompt.acceptedAnswers[0] ?? '';
  }
}

function stageOrder(stage: string, activityKind: GeneratedLearningActivityKind): number {
  const normalized = stage.toLowerCase();
  if (normalized.includes('recognition')) {
    const choiceCount = normalized.match(/(\d+)/)?.[1];
    if (choiceCount !== undefined && Number.parseInt(choiceCount, 10) >= 5) {
      return 2;
    }
    return 1;
  }
  if (normalized.includes('typed') || normalized.includes('cued') || normalized.includes('free')) {
    return 3;
  }
  if (normalized.includes('composition')) {
    return 4;
  }
  return activityKind === 'exercise' ? 5 : 1;
}

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function generatedId(prefix: string, kind: string, candidate: ParsedCandidate): string {
  return [
    prefix,
    kind,
    slug(candidate.source.id),
    candidate.blockIndex.toString(),
    slug(candidate.concept),
  ].join('-');
}

function slug(value: string): string {
  const slugged = value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slugged.length === 0 ? 'generated' : slugged;
}

function unique(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const key = value.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      result.push(value);
    }
  }
  return result;
}

function shuffleChoices(values: string[], correctChoice: string): string[] {
  const result = values.slice().reverse();
  const correctIndex = result.findIndex(
    (value) => value.toLowerCase() === correctChoice.toLowerCase(),
  );
  if (correctIndex > 0) {
    const previous = result[correctIndex - 1];
    const current = result[correctIndex];
    if (previous !== undefined && current !== undefined) {
      result[correctIndex - 1] = current;
      result[correctIndex] = previous;
    }
  }
  return result;
}
