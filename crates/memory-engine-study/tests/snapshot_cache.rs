use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use memory_engine_core::{QueueCandidate, ReviewUnitId, ReviewUnitLifecycle, ScheduleState};
use memory_engine_generation::BetaGenerationStore;
use memory_engine_persistence::{
    BetaPersistenceStore, BetaReviewUnitRecord, BetaStoreSnapshot, ConceptReferenceNote,
    GeneratedPromptDraft, GenerationRun, ReferenceSpan, SourceDocument, SourcePermission,
};
use memory_engine_service::{MemoryServiceStore, ServiceAttemptRecord};
use memory_engine_study::{BetaStudySession, BetaStudySourceInput};

const NOW: i64 = 1_779_984_000_000;

type StoreError = memory_engine_persistence::BetaStoreError;

struct SnapshotCountingStore {
    inner: BetaPersistenceStore,
    snapshots: Rc<Cell<usize>>,
}

impl SnapshotCountingStore {
    fn new(path: &Path, snapshots: Rc<Cell<usize>>) -> Self {
        Self {
            inner: BetaPersistenceStore::open(path).expect("store"),
            snapshots,
        }
    }
}

impl BetaGenerationStore for SnapshotCountingStore {
    type Error = StoreError;

    fn snapshot(&self) -> Result<BetaStoreSnapshot, Self::Error> {
        self.snapshots.set(self.snapshots.get() + 1);
        Ok(self.inner.snapshot())
    }

    fn save_generation_run(&mut self, run: GenerationRun) -> Result<GenerationRun, Self::Error> {
        self.inner.save_generation_run(run)
    }

    fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, Self::Error> {
        self.inner.save_reference_span(reference)
    }

    fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, Self::Error> {
        self.inner.save_concept_reference_note(note)
    }

    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error> {
        self.inner.save_generated_prompt_draft(draft)
    }
}

impl MemoryServiceStore for SnapshotCountingStore {
    type Error = StoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.inner.record_attempt(attempt)
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        self.inner.read_schedule_state(review_unit_id)
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        self.inner.apply_review(
            review_unit_id,
            attempt,
            schedule_state,
            expected_prior_schedule_state,
        )
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        self.inner.list_queue_candidates()
    }
}

impl memory_engine_study::BetaStudyStore for SnapshotCountingStore {
    fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, StoreError> {
        self.inner.save_source_document(document)
    }

    fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, StoreError> {
        self.inner
            .archive_source_document(source_document_id, archived_at)
    }

    fn update_source_document_permission(
        &mut self,
        source_document_id: &str,
        permission: memory_engine_study::SourcePermission,
    ) -> Result<SourceDocument, StoreError> {
        self.inner
            .update_source_document_permission(source_document_id, permission)
    }

    fn keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner.keep_generated_prompt_draft(draft_id, decided_at)
    }

    fn edit_and_keep_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        prompt_text: &str,
        expected_answer: &str,
        decided_at: i64,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner.edit_and_keep_generated_prompt_draft(
            draft_id,
            prompt_text,
            expected_answer,
            decided_at,
        )
    }

    fn reject_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        decided_at: i64,
    ) -> Result<GeneratedPromptDraft, StoreError> {
        self.inner
            .reject_generated_prompt_draft(draft_id, decided_at)
    }

    fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner
            .update_review_unit_prompt_text(review_unit_id, prompt_text, expected_answer)
    }

    fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner.archive_review_unit(review_unit_id, archived_at)
    }

    fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner
            .snooze_review_unit_until(review_unit_id, snoozed_until)
    }

    fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, StoreError> {
        self.inner
            .snooze_review_units_for_concept_until(concept_key, snoozed_until)
    }

    fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, StoreError> {
        self.inner
            .set_review_unit_lifecycle(review_unit_id, lifecycle)
    }
}

#[test]
fn review_actions_reuse_snapshot_but_refresh_after_grading() {
    let directory = tempfile_dir();
    let path = directory.join("study.json");
    seed_review(&path);

    let snapshots = Rc::new(Cell::new(0));
    let store = SnapshotCountingStore::new(&path, Rc::clone(&snapshots));
    let mut study = BetaStudySession::from_store(store, || NOW);

    let started = study.start().expect("start");
    assert_eq!(started.current.expect("current").prompt, "What is ALFA?");
    assert_eq!(
        snapshots.get(),
        1,
        "start should reuse the constructor snapshot"
    );

    let graded = study.submit_answer("ALFA", 1_800).expect("submit");
    assert_eq!(graded.summary.attempt_count, 1);
    assert_eq!(snapshots.get(), 2, "submit must refresh after its write");

    let duplicate = study
        .submit_answer("ALFA", 1_800)
        .expect("duplicate submit");
    assert_eq!(duplicate.summary.attempt_count, 1);
    assert_eq!(
        snapshots.get(),
        2,
        "idempotent duplicate should remain view-only"
    );
}

#[test]
fn source_write_refreshes_cached_view() {
    let directory = tempfile_dir();
    let path = directory.join("study.json");
    let snapshots = Rc::new(Cell::new(0));
    let store = SnapshotCountingStore::new(&path, Rc::clone(&snapshots));
    let mut study = BetaStudySession::from_store(store, || NOW);

    assert_eq!(study.view().expect("initial view").summary.source_count, 0);
    assert_eq!(snapshots.get(), 1);

    let view = study
        .add_source(BetaStudySourceInput::from_capture(
            "src-note",
            "Remember ALFA",
        ))
        .expect("source");
    assert_eq!(view.summary.source_count, 1);
    assert_eq!(
        snapshots.get(),
        2,
        "source write must invalidate the snapshot"
    );
}

fn seed_review(path: &Path) {
    let mut study =
        BetaStudySession::open(memory_engine_study::BetaStudyOptions::new(path).with_clock(|| NOW))
            .expect("open");
    study
        .add_source(BetaStudySourceInput {
            id: "src-alfa".to_owned(),
            title: "ALFA".to_owned(),
            body: [
                "Concept: NATO letter A",
                "Activity: quiz",
                "Stage: recognition",
                "Question: What is ALFA?",
                "Answer: ALFA",
                "Distractors: BRAVO, CHARLIE",
                "Reference: ALFA is the NATO word for A.",
            ]
            .join("\n"),
            project_key: None,
            ttl_expires_at: None,
            permission: SourcePermission::ModelEligible,
        })
        .expect("source");
    let generated = study.generate(None).expect("generate");
    study.keep_draft(&generated.drafts[0].id).expect("keep");
}

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
fn tempfile_dir() -> std::path::PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "memory-engine-snapshot-cache-{}-{serial}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp directory");
    path
}
