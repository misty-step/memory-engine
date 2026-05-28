use std::cmp::Ordering;

use crate::{
    filter_eligible_candidates_with_fallback, ProgressionCandidate, ProgressionLike,
    ProgressionMetadata, QueueCandidate, QueueSelectionOptions, ScheduleState, ScheduleStatus,
};

const DAY_MS: f64 = 86_400_000.0;

impl ProgressionLike<ScheduleState> for QueueCandidate {
    fn review_unit_id(&self) -> &crate::ReviewUnitId {
        &self.review_unit_id
    }

    fn review(&self) -> Option<&ScheduleState> {
        self.schedule_state.as_ref()
    }

    fn progression(&self) -> Option<&ProgressionMetadata> {
        self.progression.as_ref()
    }
}

#[must_use]
pub fn reviewable_queue_candidates(
    candidates: &[QueueCandidate],
    mastery_policy: impl Fn(&ScheduleState) -> bool + Copy,
    options: &QueueSelectionOptions<'_>,
) -> Vec<QueueCandidate> {
    let due_candidates = candidates
        .iter()
        .filter(|candidate| candidate.due <= options.now)
        .cloned()
        .collect::<Vec<_>>();
    if due_candidates.is_empty() {
        return Vec::new();
    }

    let population = options.population.unwrap_or(candidates);
    let eligibility =
        filter_eligible_candidates_with_fallback(&due_candidates, mastery_policy, Some(population));
    let available_ids = eligibility
        .available
        .iter()
        .map(|candidate| candidate.review_unit_id.clone())
        .collect::<Vec<_>>();

    due_candidates
        .into_iter()
        .filter(|candidate| available_ids.contains(&candidate.review_unit_id))
        .collect()
}

#[must_use]
pub fn pick_next_queue_candidate(
    candidates: &[QueueCandidate],
    mastery_policy: impl Fn(&ScheduleState) -> bool + Copy,
    options: &QueueSelectionOptions<'_>,
) -> Option<QueueCandidate> {
    let reviewable = reviewable_queue_candidates(candidates, mastery_policy, options);
    if reviewable.is_empty() {
        return None;
    }

    let mut sorted = reviewable;
    sorted.sort_by(|left, right| compare_queue_priority(left, right, options.now));

    let top_priority = sorted.first().map_or(usize::MAX, |candidate| {
        state_priority(candidate.schedule_state.as_ref())
    });
    let window = sorted
        .iter()
        .filter(|candidate| state_priority(candidate.schedule_state.as_ref()) == top_priority)
        .take(options.candidate_window)
        .cloned()
        .collect::<Vec<_>>();

    for pass in options.separation_passes {
        if let Some(candidate) = window
            .iter()
            .find(|candidate| passes_separation(candidate, options, *pass))
        {
            return Some(candidate.clone());
        }
    }

    window.first().cloned().or_else(|| sorted.first().cloned())
}

#[must_use]
pub fn compare_queue_priority(left: &QueueCandidate, right: &QueueCandidate, now: i64) -> Ordering {
    let priority_delta = state_priority(left.schedule_state.as_ref())
        .cmp(&state_priority(right.schedule_state.as_ref()));
    if priority_delta != Ordering::Equal {
        return priority_delta;
    }

    if is_review_state(left.schedule_state.as_ref())
        && is_review_state(right.schedule_state.as_ref())
    {
        let urgency = review_urgency(right, now)
            .partial_cmp(&review_urgency(left, now))
            .unwrap_or(Ordering::Equal);
        if urgency != Ordering::Equal {
            return urgency;
        }
    }

    let left_progression = ProgressionMetadata::normalized(left.progression.as_ref());
    let right_progression = ProgressionMetadata::normalized(right.progression.as_ref());
    if left_progression.progression_group.is_some()
        && left_progression.progression_group == right_progression.progression_group
        && left_progression.stage_order != right_progression.stage_order
    {
        return right_progression
            .stage_order
            .cmp(&left_progression.stage_order);
    }

    compare_due_then_rep(left, right)
}

fn state_priority(state: Option<&ScheduleState>) -> usize {
    match state.map(|state| state.state) {
        Some(ScheduleStatus::Learning | ScheduleStatus::Relearning) => 0,
        Some(ScheduleStatus::Review) => 1,
        Some(ScheduleStatus::New) | None => 2,
    }
}

fn is_review_state(state: Option<&ScheduleState>) -> bool {
    matches!(state.map(|state| state.state), Some(ScheduleStatus::Review))
}

fn review_urgency(candidate: &QueueCandidate, now: i64) -> f64 {
    let Some(state) = candidate.schedule_state.as_ref() else {
        return 0.0;
    };
    if state.scheduled_days <= 0 {
        return 0.0;
    }

    urgency_ratio((now - candidate.due).max(0), state.scheduled_days)
}

#[allow(clippy::cast_precision_loss)]
fn urgency_ratio(overdue_ms: i64, scheduled_days: i64) -> f64 {
    overdue_ms as f64 / (scheduled_days as f64 * DAY_MS)
}

fn compare_due_then_rep(left: &QueueCandidate, right: &QueueCandidate) -> Ordering {
    left.due
        .cmp(&right.due)
        .then_with(|| {
            right
                .schedule_state
                .as_ref()
                .map_or(0, |state| state.reps)
                .cmp(&right.schedule_state.as_ref().map_or(0, |state| state.reps))
        })
        .then_with(|| left.review_unit_id.cmp(&right.review_unit_id))
}

fn passes_separation(
    candidate: &QueueCandidate,
    options: &QueueSelectionOptions<'_>,
    pass: crate::QueueSeparationPass,
) -> bool {
    if pass.concept
        && matches_recent_key(
            candidate.concept_key.as_deref(),
            options.recent_candidates,
            options.recent_concept_window,
            |recent| recent.concept_key.as_deref(),
        )
    {
        return false;
    }

    if pass.source
        && matches_recent_key(
            candidate.source_key.as_deref(),
            options.recent_candidates,
            options.recent_source_window,
            |recent| recent.source_key.as_deref(),
        )
    {
        return false;
    }

    if pass.domain
        && matches_recent_key(
            candidate.domain_key.as_deref(),
            options.recent_candidates,
            options.recent_domain_window,
            |recent| recent.domain_key.as_deref(),
        )
    {
        return false;
    }

    true
}

fn matches_recent_key(
    key: Option<&str>,
    recent_candidates: &[QueueCandidate],
    window: usize,
    select: impl Fn(&QueueCandidate) -> Option<&str>,
) -> bool {
    let Some(key) = normalize_key(key) else {
        return false;
    };
    if window == 0 {
        return false;
    }

    recent_candidates
        .iter()
        .take(window)
        .any(|recent| normalize_key(select(recent)).as_deref() == Some(key.as_str()))
}

fn normalize_key(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[allow(dead_code)]
fn to_progression_candidate(candidate: &QueueCandidate) -> ProgressionCandidate<ScheduleState> {
    ProgressionCandidate {
        review_unit_id: candidate.review_unit_id.clone(),
        review: candidate.schedule_state.clone(),
        progression: candidate.progression.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{ProgressionMetadata, QueueSeparationPass, ReviewUnitId};

    use super::*;

    const NOW: i64 = 1_775_650_400_000;

    fn state(status: ScheduleStatus, reps: u32, scheduled_days: i64, due: i64) -> ScheduleState {
        ScheduleState {
            due,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days,
            reps,
            lapses: 0,
            state: status,
            last_review: None,
        }
    }

    fn candidate(
        id: &str,
        schedule_state: Option<ScheduleState>,
        due: i64,
        source_key: Option<&str>,
    ) -> QueueCandidate {
        QueueCandidate {
            review_unit_id: ReviewUnitId::new(id),
            schedule_state,
            due,
            progression: None,
            concept_key: None,
            source_key: source_key.map(str::to_owned),
            domain_key: None,
        }
    }

    fn mastered(state: &ScheduleState) -> bool {
        state.state == ScheduleStatus::Review && state.reps >= 3
    }

    fn options<'a>(
        recent_candidates: &'a [QueueCandidate],
        population: Option<&'a [QueueCandidate]>,
    ) -> QueueSelectionOptions<'a> {
        QueueSelectionOptions {
            now: NOW,
            recent_candidates,
            population,
            ..QueueSelectionOptions::default()
        }
    }

    #[test]
    fn prefers_review_candidates_over_fresh_due_candidates() {
        let review = candidate(
            "review",
            Some(state(ScheduleStatus::Review, 4, 5, NOW - 3_600_000)),
            NOW - 3_600_000,
            Some("core"),
        );
        let fresh = candidate("fresh", None, NOW - 60_000, Some("core"));

        let next = pick_next_queue_candidate(&[fresh, review], mastered, &options(&[], None));

        assert_eq!(
            next.map(|entry| entry.review_unit_id),
            Some(ReviewUnitId::new("review"))
        );
    }

    #[test]
    fn avoids_same_source_clumps_when_equally_urgent_alternative_exists() {
        let recent = [candidate(
            "recent",
            Some(state(ScheduleStatus::Review, 3, 8, NOW - 60_000)),
            NOW - 60_000,
            Some("abolition"),
        )];
        let same_source = candidate(
            "same",
            Some(state(ScheduleStatus::Review, 3, 8, NOW - 120_000)),
            NOW - 120_000,
            Some("abolition"),
        );
        let alternative = candidate(
            "alternative",
            Some(state(ScheduleStatus::Review, 3, 8, NOW - 100_000)),
            NOW - 100_000,
            Some("nato"),
        );

        let next = pick_next_queue_candidate(
            &[same_source, alternative],
            mastered,
            &options(&recent, None),
        );

        assert_eq!(
            next.map(|entry| entry.review_unit_id),
            Some(ReviewUnitId::new("alternative"))
        );
    }

    #[test]
    fn falls_back_to_locked_candidate_when_progression_would_hide_every_due_item() {
        let mut locked = candidate("stage-2", None, NOW - 60_000, None);
        locked.progression = Some(ProgressionMetadata {
            progression_group: Some("ladder".to_owned()),
            stage_order: 2,
            requires: Vec::new(),
            supersedes: Vec::new(),
        });

        let next = pick_next_queue_candidate(
            std::slice::from_ref(&locked),
            mastered,
            &QueueSelectionOptions {
                now: NOW,
                separation_passes: &[QueueSeparationPass {
                    concept: false,
                    source: false,
                    domain: false,
                }],
                ..QueueSelectionOptions::default()
            },
        );

        assert_eq!(next, Some(locked));
    }
}
