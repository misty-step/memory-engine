//! Consumer-facing Rust facade for memory-engine.
//!
//! This crate is the Rust cutover target for package consumers. It does not
//! reimplement learning behavior; each module exposes the deeper crate that
//! owns that concern.

pub mod beta {
    //! Repo-local beta study workflow and durable beta state.

    pub mod generation {
        pub use memory_engine_generation::{
            run_beta_generation, run_bridge_generation_with_provider, BetaGenerationError,
            BetaGenerationRequest, BetaGenerationResult, BridgeGenerationRequest,
            BridgeGenerationResult, BridgeMaterial, BridgeMaterialProvider, BridgeMaterialRequest,
            ReferenceNoteDraft, ReferenceNoteProvider, ReferenceNoteRequest,
        };
    }

    pub mod persistence {
        pub use memory_engine_persistence::{
            AppliedReviewReceipt, BetaPersistenceStore,
            BetaReviewUnitRecord, BetaStoreError, BetaStoreSnapshot, ConceptReferenceNote,
            GeneratedLearningActivityKind, GeneratedPromptDraft, GeneratedPromptModel,
            GeneratedPromptValidation, GeneratedPromptValidationStatus, GenerationRun,
            PersistedQueueCandidate, ReferenceSpan, ScheduleRecord, SourceDocument,
            SourceDocumentKind, SourcePermission,
        };
    }

    pub mod study {
        pub use memory_engine_study::{
            BetaStudyCurrent, BetaStudyDraftRow, BetaStudyError, BetaStudyGrade, BetaStudyOptions,
            BetaStudyQueueRow, BetaStudySession, BetaStudySourceInput, BetaStudySourceRow,
            BetaStudyStatus, BetaStudySummary, BetaStudyView, ReviewStateProjection,
            ScheduleChange, DEFAULT_BETA_STUDY_NOW, DEFAULT_BRIDGE_PARENT_DEFER_MS,
            DEFAULT_SKIP_DEFER_MS, DEFAULT_SNOOZE_DEFER_MS,
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

pub mod adapters {
    //! Adapter contracts for rubric-backed grading.

    pub use memory_engine_core::{RubricGraderAdapter, StaticRubricGrader};
}

pub mod grading {
    //! Deterministic grading and grade result types.

    pub use memory_engine_core::{
        default_rating_policy, resolve_rubric_grade, AsyncGrader, ExactPrompt, ExactPromptKind,
        GradeContext, GradeResult, GradeablePrompt, Grader, GraderKind, Rating, RatingPolicy,
        RubricAssessment, RubricCriterion, RubricCriterionResult, RubricCriterionVerdict,
        RubricDefinition, RubricGradeError, RubricGraderAdapter, RubricPrompt, StaticRubricGrader,
        Verdict, DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
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
        compare_queue_priority, defer_queue_availability, pick_next_queue_candidate,
        reviewable_queue_candidates, QueueCandidate, QueueSelectionOptions, QueueSeparationPass,
        ReviewUnitLifecycle, ReviewUnitRetirement, ReviewUnitRetirementReason,
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

pub mod testkit;

pub mod types {
    //! Canonical learning-domain data types.

    pub use memory_engine_core::{
        ExactPrompt, ExactPromptKind, GradeContext, GradeResult, GraderKind, ProgressionMetadata,
        Prompt, QueueCandidate, QueueSelectionOptions, QueueSeparationPass, Rating, ReviewUnitId,
        ReviewUnitLifecycle, ReviewUnitRetirement, ReviewUnitRetirementReason, RubricAssessment,
        RubricCriterion, RubricCriterionResult, RubricCriterionVerdict, RubricDefinition,
        RubricPrompt, ScheduleState, ScheduleStatus, Verdict,
    };
}

pub use adapters::{RubricGraderAdapter, StaticRubricGrader};
pub use grading::{
    default_rating_policy, resolve_rubric_grade, AsyncGrader, ExactPrompt, ExactPromptKind,
    GradeablePrompt, Grader, Rating, RatingPolicy, RubricAssessment, RubricCriterion,
    RubricCriterionResult, RubricCriterionVerdict, RubricDefinition, RubricGradeError,
    RubricPrompt, Verdict, DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
};
pub use progression::{
    filter_eligible_candidates, filter_eligible_candidates_with_fallback, is_mastered,
    ProgressionCandidate, ProgressionFilterResult, ProgressionMetadata,
};
pub use queue::{
    compare_queue_priority, defer_queue_availability, pick_next_queue_candidate,
    reviewable_queue_candidates, ReviewUnitLifecycle, ReviewUnitRetirement,
    ReviewUnitRetirementReason,
};
pub use scheduling::next;
pub use types::{
    GradeContext, Prompt, QueueCandidate, ReviewUnitId, ScheduleState, ScheduleStatus,
};

#[cfg(test)]
mod tests {
    use memory_engine_core::{ProgressionMetadata, QueueCandidate, ScheduleState, ScheduleStatus};

    use super::{
        adapters, default_rating_policy, dogfood, filter_eligible_candidates,
        filter_eligible_candidates_with_fallback, grading, next, pick_next_queue_candidate, queue,
        scheduling, testkit, types, ExactPrompt, ExactPromptKind, Grader, ProgressionCandidate,
        Prompt, Rating, ReviewUnitId, ReviewUnitLifecycle, RubricAssessment, RubricCriterion,
        RubricCriterionResult, RubricCriterionVerdict, RubricDefinition, RubricPrompt,
        StaticRubricGrader,
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

    #[test]
    fn facade_exposes_rubric_grading_and_adapter_subpaths() {
        let prompt = RubricPrompt {
            review_unit_id: ReviewUnitId::new("api-rubric"),
            prompt: "Continue the prayer.".to_owned(),
            rubric: RubricDefinition {
                answer_guide: vec!["Continue with the next line.".to_owned()],
                passing_score: 1,
                criteria: vec![RubricCriterion {
                    name: "continuation".to_owned(),
                    description: "Gives the next line.".to_owned(),
                    required: true,
                }],
            },
        };
        let adapter = adapters::StaticRubricGrader::new(RubricAssessment {
            model: Some("gpt-5.4-mini".to_owned()),
            confidence: 0.91,
            feedback: "Strong answer.".to_owned(),
            criterion_results: vec![RubricCriterionResult {
                name: "continuation".to_owned(),
                verdict: RubricCriterionVerdict::Pass,
                evidence: "Continued with the correct line.".to_owned(),
            }],
        });

        let root_grader = grading::AsyncGrader::with_rubric_grader(adapter);
        let result = root_grader
            .grade_prompt(
                grading::GradeablePrompt::Rubric(&prompt),
                "Strong answer.",
                types::GradeContext {
                    response_time_ms: 6_000,
                    prior_reps: 0,
                },
            )
            .expect("rubric grade");

        assert_eq!(result.verdict, types::Verdict::Correct);
        assert_eq!(result.grader_kind, types::GraderKind::RubricLlm);
        assert!((grading::DEFAULT_RUBRIC_CONFIDENCE_FLOOR - 0.85).abs() < f64::EPSILON);

        let static_grader = StaticRubricGrader::new(RubricAssessment {
            model: None,
            confidence: 1.0,
            feedback: "Still available from the root facade.".to_owned(),
            criterion_results: vec![RubricCriterionResult {
                name: "continuation".to_owned(),
                verdict: RubricCriterionVerdict::Pass,
                evidence: "Root re-export works.".to_owned(),
            }],
        });
        let root_result = grading::AsyncGrader::with_rubric_grader(static_grader)
            .grade_prompt(
                grading::GradeablePrompt::Rubric(&prompt),
                "Strong answer.",
                types::GradeContext {
                    response_time_ms: 6_000,
                    prior_reps: 0,
                },
            )
            .expect("rubric grade");

        assert_eq!(root_result.verdict, types::Verdict::Correct);
    }

    #[test]
    fn testkit_fixtures_stay_in_sync_with_rust_public_surfaces() {
        let grader = Grader::new();

        for fixture in testkit::grading_fixtures() {
            assert_eq!(
                grader.grade(&fixture.prompt, &fixture.submitted, fixture.context),
                fixture.expected,
                "{}",
                fixture.name
            );
        }

        for fixture in testkit::scheduler_fixtures() {
            assert_eq!(
                next(fixture.initial_state.as_ref(), fixture.rating, fixture.now)
                    .expect("schedule"),
                fixture.expected,
                "{}",
                fixture.name
            );
        }

        for fixture in testkit::progression_fixtures() {
            let mastery_policy = progression_mastery_policy(fixture.mastery_policy);
            let population = fixture.population.as_deref();
            let result = match fixture.mode {
                testkit::ProgressionFixtureMode::Strict => {
                    filter_eligible_candidates(&fixture.candidates, mastery_policy, population)
                }
                testkit::ProgressionFixtureMode::Fallback => {
                    filter_eligible_candidates_with_fallback(
                        &fixture.candidates,
                        mastery_policy,
                        population,
                    )
                }
            };

            assert_eq!(
                result
                    .available
                    .iter()
                    .map(|candidate| candidate.review_unit_id.clone())
                    .collect::<Vec<_>>(),
                fixture.expected_available_review_unit_ids,
                "{}",
                fixture.name
            );
            assert_eq!(
                result.locked_fresh_count, fixture.expected_locked_fresh_count,
                "{}",
                fixture.name
            );
        }

        for fixture in testkit::queue_fixtures() {
            let mastery_policy = queue_mastery_policy(fixture.mastery_policy);
            let recent_source_window = fixture
                .recent_source_window
                .unwrap_or(queue::QueueSelectionOptions::default().recent_source_window);
            let candidate = pick_next_queue_candidate(
                &fixture.candidates,
                mastery_policy,
                &queue::QueueSelectionOptions {
                    now: fixture.now,
                    recent_candidates: &fixture.recent_candidates,
                    population: fixture.population.as_deref(),
                    recent_source_window,
                    ..queue::QueueSelectionOptions::default()
                },
            );

            assert_eq!(
                candidate.map(|candidate| candidate.review_unit_id),
                fixture.expected_next_review_unit_id,
                "{}",
                fixture.name
            );
        }

        assert!(!testkit::recitation_fixtures().is_empty());
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
                lifecycle: ReviewUnitLifecycle::active(),
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
                lifecycle: ReviewUnitLifecycle::active(),
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

    fn progression_mastery_policy(
        policy: testkit::Slice2MasteryPolicy,
    ) -> impl Fn(&testkit::ProgressionFixtureReview) -> bool + Copy {
        move |review| match policy {
            testkit::Slice2MasteryPolicy::Ruminatio => {
                review.state == ScheduleStatus::Review || review.reps >= 2
            }
            testkit::Slice2MasteryPolicy::Vault => {
                review.state == ScheduleStatus::Review && review.reps >= 3
            }
        }
    }

    fn queue_mastery_policy(
        policy: testkit::Slice2MasteryPolicy,
    ) -> impl Fn(&ScheduleState) -> bool + Copy {
        move |review| match policy {
            testkit::Slice2MasteryPolicy::Ruminatio => {
                review.state == ScheduleStatus::Review || review.reps >= 2
            }
            testkit::Slice2MasteryPolicy::Vault => {
                review.state == ScheduleStatus::Review && review.reps >= 3
            }
        }
    }
}
