use memory_engine_core::ReviewUnitId;
use memory_engine_service::{
    record_content_feedback, ContentFeedback, ContentFeedbackStore, ContentFeedbackVerdict,
    RecordContentFeedbackCommand,
};

#[derive(Default)]
struct Store {
    records: Vec<ContentFeedback>,
}

impl ContentFeedbackStore for Store {
    type Error = String;

    fn record_content_feedback(
        &mut self,
        feedback: ContentFeedback,
    ) -> Result<ContentFeedback, Self::Error> {
        if let Some(existing) = self
            .records
            .iter()
            .find(|existing| existing.id == feedback.id)
        {
            return Ok(existing.clone());
        }
        self.records.push(feedback.clone());
        Ok(feedback)
    }
}

#[test]
fn content_feedback_command_is_binary_json_safe_and_idempotent() {
    let mut store = Store::default();
    let command = RecordContentFeedbackCommand {
        feedback_id: "feedback-1".to_owned(),
        review_unit_id: ReviewUnitId::new("unit-1"),
        verdict: ContentFeedbackVerdict::Dropped,
        rationale: Some("The distractors give away the answer.".to_owned()),
        account_id: "account-1".to_owned(),
        occurred_at: 1_779_465_600_000,
        supersedes_id: None,
    };

    let first = record_content_feedback(&mut store, command.clone()).expect("record feedback");
    let mut retry = command;
    retry.occurred_at += 1;
    let second = record_content_feedback(&mut store, retry).expect("repeat feedback");

    assert_eq!(first, second);
    assert_eq!(store.records.len(), 1);
    assert_eq!(first.source.as_str(), "human");
    assert_eq!(first.verdict, ContentFeedbackVerdict::Dropped);
    let json = serde_json::to_value(&first).expect("feedback is JSON-safe");
    assert_eq!(json["verdict"], "dropped");
    assert_eq!(json["source"], "human");
    assert_eq!(json["accountId"], "account-1");
}
