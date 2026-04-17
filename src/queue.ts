import { State } from 'ts-fsrs';

import { filterEligibleCandidatesWithFallback } from './progression';
import { normalizeProgressionMetadata } from './progression-metadata';
import type {
  MasteryPolicy,
  ProgressionCandidate,
  QueueCandidate,
  QueueSelectionOptions,
  QueueSeparationPass,
  ScheduleState,
} from './types';

const DEFAULT_CANDIDATE_WINDOW = 12;
const DEFAULT_RECENT_CONCEPT_WINDOW = 3;
const DEFAULT_RECENT_SOURCE_WINDOW = 2;
const DEFAULT_RECENT_DOMAIN_WINDOW = 1;
const DAY_MS = 86_400_000;

const defaultSeparationPasses: readonly QueueSeparationPass[] = [
  { concept: true, source: true, domain: true },
  { concept: true, source: true, domain: false },
  { concept: true, source: false, domain: false },
  { concept: false, source: true, domain: false },
  { concept: false, source: false, domain: false },
];

type RecentWindows = {
  candidateWindow: number;
  concept: number;
  source: number;
  domain: number;
};

export function reviewableQueueCandidates<TCandidate extends QueueCandidate>(
  candidates: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<ScheduleState>,
  options: QueueSelectionOptions<TCandidate> = {},
): TCandidate[] {
  const now = options.now ?? Date.now();
  const dueCandidates = candidates.filter((candidate) => candidate.due <= now);
  if (dueCandidates.length === 0) {
    return [];
  }

  const eligibility = filterEligibleCandidatesWithFallback(
    dueCandidates.map(toProgressionCandidate),
    masteryPolicy,
    {
      population: (options.population ?? candidates).map(toProgressionCandidate),
    },
  );
  const availableIds = new Set(eligibility.available.map((candidate) => candidate.reviewUnitId));

  return dueCandidates.filter((candidate) => availableIds.has(candidate.reviewUnitId));
}

export function pickNextQueueCandidate<TCandidate extends QueueCandidate>(
  candidates: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<ScheduleState>,
  options: QueueSelectionOptions<TCandidate> = {},
): TCandidate | null {
  const reviewable = reviewableQueueCandidates(candidates, masteryPolicy, options);
  if (reviewable.length === 0) {
    return null;
  }

  const now = options.now ?? Date.now();
  const windows = resolveRecentWindows(options);
  const recentCandidates = options.recentCandidates ?? [];
  const sorted = reviewable.slice().sort((left, right) => compareQueuePriority(left, right, now));
  const topPriority = statePriority(sorted[0]?.scheduleState ?? null);
  const window = sorted
    .filter((candidate) => statePriority(candidate.scheduleState) === topPriority)
    .slice(0, windows.candidateWindow);

  for (const pass of options.separationPasses ?? defaultSeparationPasses) {
    const match = window.find((candidate) =>
      passesSeparation(candidate, recentCandidates, pass, windows),
    );
    if (match) {
      return match;
    }
  }

  return window[0] ?? sorted[0] ?? null;
}

export function compareQueuePriority<TCandidate extends QueueCandidate>(
  left: TCandidate,
  right: TCandidate,
  now: number,
): number {
  const priorityDelta = statePriority(left.scheduleState) - statePriority(right.scheduleState);
  if (priorityDelta !== 0) {
    return priorityDelta;
  }

  if (isReviewState(left.scheduleState) && isReviewState(right.scheduleState)) {
    const urgencyDelta = reviewUrgency(right, now) - reviewUrgency(left, now);
    if (urgencyDelta !== 0) {
      return urgencyDelta;
    }
  }

  const leftProgression = normalizeProgressionMetadata(left.progression);
  const rightProgression = normalizeProgressionMetadata(right.progression);
  if (
    leftProgression.progressionGroup !== null &&
    leftProgression.progressionGroup === rightProgression.progressionGroup &&
    leftProgression.stageOrder !== rightProgression.stageOrder
  ) {
    return rightProgression.stageOrder - leftProgression.stageOrder;
  }

  return compareDueThenRep(left, right);
}

function resolveRecentWindows<TCandidate extends QueueCandidate>(
  options: QueueSelectionOptions<TCandidate>,
): RecentWindows {
  return {
    candidateWindow: normalizeWindow(options.candidateWindow, DEFAULT_CANDIDATE_WINDOW),
    concept: normalizeWindow(options.recentConceptWindow, DEFAULT_RECENT_CONCEPT_WINDOW),
    source: normalizeWindow(options.recentSourceWindow, DEFAULT_RECENT_SOURCE_WINDOW),
    domain: normalizeWindow(options.recentDomainWindow, DEFAULT_RECENT_DOMAIN_WINDOW),
  };
}

function passesSeparation<TCandidate extends QueueCandidate>(
  candidate: TCandidate,
  recentCandidates: readonly TCandidate[],
  pass: QueueSeparationPass,
  windows: RecentWindows,
): boolean {
  if (
    pass.concept &&
    matchesRecentKey(
      candidate.conceptKey,
      recentCandidates,
      windows.concept,
      (recent) => recent.conceptKey,
    )
  ) {
    return false;
  }

  if (
    pass.source &&
    matchesRecentKey(
      candidate.sourceKey,
      recentCandidates,
      windows.source,
      (recent) => recent.sourceKey,
    )
  ) {
    return false;
  }

  if (
    pass.domain &&
    matchesRecentKey(
      candidate.domainKey,
      recentCandidates,
      windows.domain,
      (recent) => recent.domainKey,
    )
  ) {
    return false;
  }

  return true;
}

function matchesRecentKey<TCandidate extends QueueCandidate>(
  key: string | null,
  recentCandidates: readonly TCandidate[],
  window: number,
  selectRecentKey: (candidate: TCandidate) => string | null,
): boolean {
  if (window <= 0) {
    return false;
  }

  const normalizedKey = normalizeKey(key);
  if (normalizedKey === null) {
    return false;
  }

  return recentCandidates
    .slice(0, window)
    .some((candidate) => normalizeKey(selectRecentKey(candidate)) === normalizedKey);
}

function toProgressionCandidate<TCandidate extends QueueCandidate>(
  candidate: TCandidate,
): ProgressionCandidate<ScheduleState> {
  return {
    reviewUnitId: candidate.reviewUnitId,
    review: candidate.scheduleState,
    progression: candidate.progression,
  };
}

function compareDueThenRep<TCandidate extends QueueCandidate>(
  left: TCandidate,
  right: TCandidate,
): number {
  if (left.due !== right.due) {
    return left.due - right.due;
  }

  const leftReps = left.scheduleState?.reps ?? 0;
  const rightReps = right.scheduleState?.reps ?? 0;
  if (leftReps !== rightReps) {
    return leftReps - rightReps;
  }

  return left.reviewUnitId.localeCompare(right.reviewUnitId);
}

function reviewUrgency<TCandidate extends QueueCandidate>(
  candidate: TCandidate,
  now: number,
): number {
  const overdueMs = Math.max(0, now - candidate.due);
  const scheduledDays = Math.max(1, candidate.scheduleState?.scheduled_days ?? 1);

  return overdueMs / (scheduledDays * DAY_MS);
}

function statePriority(scheduleState: ScheduleState | null): number {
  switch (scheduleState?.state ?? State.New) {
    case State.Learning:
    case State.Relearning:
      return 0;
    case State.Review:
      return 1;
    default:
      return 2;
  }
}

function isReviewState(scheduleState: ScheduleState | null): scheduleState is ScheduleState {
  return scheduleState?.state === State.Review;
}

function normalizeKey(value: string | null): string | null {
  const normalized = value?.trim().toLowerCase();

  return normalized ? normalized : null;
}

function normalizeWindow(value: number | undefined, fallback: number): number {
  if (value === undefined || !Number.isFinite(value)) {
    return fallback;
  }

  return Math.max(0, Math.trunc(value));
}
