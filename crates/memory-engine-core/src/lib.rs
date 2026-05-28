//! Pure learning-kernel semantics for memory-engine.
//!
//! This crate intentionally contains no persistence, networking, UI, logging,
//! model clients, or runtime framework hooks. Callers own those boundaries.

mod domain;
mod grading;
mod progression;
mod queue;

pub use domain::{
    ExactPrompt, ExactPromptKind, GradeContext, GradeResult, GraderKind, ProgressionMetadata,
    Prompt, QueueCandidate, QueueSelectionOptions, QueueSeparationPass, Rating, ReviewUnitId,
    RubricCriterionResult, RubricCriterionVerdict, ScheduleState, ScheduleStatus, Verdict,
};
pub use grading::{default_rating_policy, Grader};
pub use progression::{
    filter_eligible_candidates, filter_eligible_candidates_with_fallback, is_mastered,
    ProgressionCandidate, ProgressionFilterResult, ProgressionLike,
};
pub use queue::{compare_queue_priority, pick_next_queue_candidate, reviewable_queue_candidates};
