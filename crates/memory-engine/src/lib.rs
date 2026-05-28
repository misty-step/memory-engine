//! Consumer-facing Rust facade for memory-engine.
//!
//! This crate is the Rust cutover target for package consumers. It does not
//! reimplement learning behavior; each module exposes the deeper crate that
//! owns that concern.

pub mod beta {
    //! Repo-local beta study workflow and durable beta state.

    pub mod generation {
        pub use memory_engine_generation::{
            run_beta_generation, BetaGenerationError, BetaGenerationRequest, BetaGenerationResult,
        };
    }

    pub mod persistence {
        pub use memory_engine_persistence::{
            AppliedReviewReceipt, ApproveGeneratedPromptDraftOptions, BetaPersistenceStore,
            BetaReviewUnitRecord, BetaStoreError, BetaStoreSnapshot, GeneratedLearningActivityKind,
            GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
            GeneratedPromptValidationStatus, GenerationRun, PersistedQueueCandidate, ReferenceSpan,
            ScheduleRecord, SourceDocument, SourceDocumentKind, SourcePermission,
        };
    }

    pub mod study {
        pub use memory_engine_study::{
            BetaStudyCurrent, BetaStudyDraftRow, BetaStudyError, BetaStudyGrade, BetaStudyOptions,
            BetaStudyQueueRow, BetaStudySession, BetaStudySourceInput, BetaStudySourceRow,
            BetaStudyStatus, BetaStudySummary, BetaStudyView, ReviewStateProjection,
            ScheduleChange, DEFAULT_BETA_STUDY_NOW,
        };
    }
}

pub mod dogfood {
    //! Repo-local dogfood clients and receipts.

    pub mod cli_review {
        pub use memory_engine_cli::{run_cli_review, CliReviewError, CliReviewReceipt};
    }

    pub mod import_probe {
        pub use memory_engine_import::{
            compile_latin_prayer_fixture, run_import_probe, CompiledImportProbe, CompiledSchedule,
            ImportProbeError, ImportProbeReceipt, ImportStoreError,
        };
    }

    pub mod web_shell {
        pub use memory_engine_web_shell::{
            route, run_web_shell_flow, serve, HttpRequest, HttpResponse, WebShellConfig,
            WebShellCurrent, WebShellError, WebShellGrade, WebShellQueueRow, WebShellReceipt,
            WebShellReviewState, WebShellSession, WebShellStatus, WebShellStoreError, WebShellView,
        };
    }
}

pub mod grading {
    //! Deterministic grading and grade result types.

    pub use memory_engine_core::{
        default_rating_policy, ExactPrompt, ExactPromptKind, GradeContext, GradeResult, Grader,
        GraderKind, Rating, RubricCriterionResult, RubricCriterionVerdict, Verdict,
    };
}

pub mod progression {
    //! Progression eligibility and mastery policy helpers.

    pub use memory_engine_core::{
        filter_eligible_candidates, filter_eligible_candidates_with_fallback, is_mastered,
        ProgressionCandidate, ProgressionFilterResult, ProgressionLike, ProgressionMetadata,
    };
}

pub mod queue {
    //! Queue candidate filtering and priority selection.

    pub use memory_engine_core::{
        compare_queue_priority, pick_next_queue_candidate, reviewable_queue_candidates,
        QueueCandidate, QueueSelectionOptions, QueueSeparationPass,
    };
}

pub mod scheduling {
    //! Schedule advancement through the Rust scheduler boundary.

    pub use memory_engine_core::{next, FsrsScheduler, Scheduler, SchedulerError};
}

pub mod service {
    //! Typed command boundary that composes grading, scheduling, and queueing.

    pub use memory_engine_service::{
        AttemptRecordedResult, GradeApplyReviewCommand, MemoryService, MemoryServiceCommand,
        MemoryServiceResult, MemoryServiceStore, NextQueueCommand, NextQueueOptions,
        QueueSelectedResult, RecordAttemptCommand, RecordAttemptInput, ReviewAppliedResult,
        ServiceAttemptRecord, ServiceError,
    };
}

pub mod types {
    //! Canonical learning-domain data types.

    pub use memory_engine_core::{
        ExactPrompt, ExactPromptKind, GradeContext, GradeResult, GraderKind, ProgressionMetadata,
        Prompt, QueueCandidate, QueueSelectionOptions, QueueSeparationPass, Rating, ReviewUnitId,
        RubricCriterionResult, RubricCriterionVerdict, ScheduleState, ScheduleStatus, Verdict,
    };
}

pub use grading::{default_rating_policy, ExactPrompt, ExactPromptKind, Grader, Rating, Verdict};
pub use progression::{
    filter_eligible_candidates, filter_eligible_candidates_with_fallback, is_mastered,
    ProgressionCandidate, ProgressionFilterResult, ProgressionMetadata,
};
pub use queue::{compare_queue_priority, pick_next_queue_candidate, reviewable_queue_candidates};
pub use scheduling::next;
pub use types::{
    GradeContext, Prompt, QueueCandidate, ReviewUnitId, ScheduleState, ScheduleStatus,
};

#[cfg(test)]
mod tests {
    use memory_engine_core::{ProgressionMetadata, QueueCandidate, ScheduleState, ScheduleStatus};

    use super::{
        default_rating_policy, dogfood, filter_eligible_candidates, grading, next,
        pick_next_queue_candidate, queue, scheduling, types, ExactPrompt, ExactPromptKind, Grader,
        ProgressionCandidate, Prompt, Rating, ReviewUnitId,
    };

    const NOW: i64 = 1_779_465_600_000;

    #[test]
    fn root_facade_matches_readme_grade_and_schedule_usage() {
        let review_unit_id = ReviewUnitId::new("latin-1");
        let prompt = Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::ShortAnswer,
            review_unit_id,
            prompt: "Translate poena".to_owned(),
            accepted_answers: vec!["punishment".to_owned()],
            equivalence_groups: Vec::new(),
            ignored_tokens: Vec::new(),
        });

        let grade = Grader::new().grade(
            &prompt,
            "Punishment",
            types::GradeContext {
                response_time_ms: 3_200,
                prior_reps: 3,
            },
        );
        let schedule = next(None, grade.rating, NOW).expect("schedule");

        assert_eq!(grade.verdict, types::Verdict::Correct);
        assert_eq!(grade.rating, Rating::Easy);
        assert_eq!(schedule.reps, 1);
        assert_eq!(schedule.last_review, Some(NOW));
    }

    #[test]
    fn modular_facade_paths_compose_like_package_subpaths() {
        let (advanced, candidates, progression_candidates) = progression_fixture();
        let mastery_policy = |schedule: &ScheduleState| {
            schedule.state == ScheduleStatus::Review && schedule.reps >= 3
        };

        assert_eq!(
            default_rating_policy(
                types::Verdict::Correct,
                types::GradeContext {
                    response_time_ms: 3_200,
                    prior_reps: 3,
                },
            ),
            Rating::Easy,
        );
        assert_eq!(
            grading::default_rating_policy(
                types::Verdict::Correct,
                types::GradeContext {
                    response_time_ms: 10_000,
                    prior_reps: 0,
                },
            ),
            Rating::Good
        );
        assert_eq!(
            scheduling::next(None, Rating::Good, NOW)
                .expect("schedule")
                .state,
            ScheduleStatus::Learning
        );
        assert_eq!(
            filter_eligible_candidates(
                &progression_candidates,
                mastery_policy,
                Some(&progression_candidates),
            )
            .available
            .len(),
            2
        );
        assert_eq!(
            pick_next_queue_candidate(
                &candidates,
                mastery_policy,
                &queue::QueueSelectionOptions {
                    now: NOW,
                    ..queue::QueueSelectionOptions::default()
                },
            )
            .map(|candidate| candidate.review_unit_id),
            Some(advanced)
        );
    }

    #[test]
    fn facade_exposes_dogfood_receipts_without_promoting_them_to_core() {
        let cli = dogfood::cli_review::run_cli_review().expect("cli receipt");
        let import = dogfood::import_probe::run_import_probe().expect("import receipt");
        let web = dogfood::web_shell::run_web_shell_flow().expect("web receipt");

        assert_eq!(cli.fixture, "latin-prayer-opening");
        assert_eq!(import.fixture, "latin-prayer-authored-v1");
        assert_eq!(web.fixture, "latin-prayer-authored-v1");
    }

    fn progression_fixture() -> (
        ReviewUnitId,
        Vec<QueueCandidate>,
        Vec<ProgressionCandidate<ScheduleState>>,
    ) {
        let mastered = ScheduleState {
            due: NOW + 86_400_000,
            stability: 5.0,
            difficulty: 2.0,
            elapsed_days: 2,
            scheduled_days: 2,
            reps: 3,
            lapses: 0,
            state: ScheduleStatus::Review,
            last_review: Some(NOW - 86_400_000),
        };
        let prerequisite = ReviewUnitId::new("api-prerequisite");
        let advanced = ReviewUnitId::new("api-advanced");
        let candidates = vec![
            QueueCandidate {
                review_unit_id: prerequisite.clone(),
                schedule_state: Some(mastered.clone()),
                due: mastered.due,
                progression: Some(ProgressionMetadata {
                    progression_group: Some("api".to_owned()),
                    stage_order: 1,
                    requires: Vec::new(),
                    supersedes: Vec::new(),
                }),
                concept_key: Some("api".to_owned()),
                source_key: Some("source".to_owned()),
                domain_key: Some("domain".to_owned()),
            },
            QueueCandidate {
                review_unit_id: advanced.clone(),
                schedule_state: None,
                due: NOW - 60_000,
                progression: Some(ProgressionMetadata {
                    progression_group: Some("api".to_owned()),
                    stage_order: 2,
                    requires: vec![prerequisite],
                    supersedes: Vec::new(),
                }),
                concept_key: Some("api".to_owned()),
                source_key: Some("source".to_owned()),
                domain_key: Some("domain".to_owned()),
            },
        ];
        let progression_candidates = candidates
            .iter()
            .map(|candidate| ProgressionCandidate {
                review_unit_id: candidate.review_unit_id.clone(),
                review: candidate.schedule_state.clone(),
                progression: candidate.progression.clone(),
            })
            .collect::<Vec<_>>();

        (advanced, candidates, progression_candidates)
    }
}
