//! `calibrate` subcommand: validate the model judge against human keep labels.
//!
//! The rigor reference (harness `verification-system-first.md`, "Eval &
//! Benchmark Rigor") requires a model judge be calibrated against human labels —
//! target Cohen's kappa ~= 0.80 — before its keep rate is trusted. memory-engine's
//! judge has never been calibrated; this is the tool that does it.
//!
//! Workflow:
//! 1. run a judged generation eval and, for each draft, note the judge's keep
//!    decision (the receipt's reject-notes list the drops; the rest are keeps);
//! 2. a domain expert labels each draft keep/drop, producing a JSON array of
//!    `{judge_keep, human_keep}` (a `question` field is optional, for reference);
//! 3. `calibrate --labels <path>` reports kappa + TPR/TNR against the 0.80 bar.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::stats::{judge_agreement, JudgeAgreement, KeepConfusion};

/// Human-level inter-rater agreement; a judge below this is not yet trustworthy.
const KAPPA_BAR: f64 = 0.80;

#[derive(Debug, Deserialize)]
struct Label {
    #[serde(default)]
    #[allow(dead_code)]
    question: String,
    #[serde(alias = "judgeKeep")]
    judge_keep: bool,
    /// `null`/absent until a human has labeled this draft; such rows are skipped.
    #[serde(alias = "humanKeep")]
    human_keep: Option<bool>,
}

/// Run the `calibrate` subcommand.
///
/// # Errors
///
/// Returns a message on an unknown flag, a missing/unreadable/non-JSON labels
/// file, a file with no human-labeled rows, or an unwritable `--out` path.
pub fn run(arguments: &[String]) -> Result<(), String> {
    let mut labels_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut iterator = arguments.iter();
    while let Some(flag) = iterator.next() {
        match flag.as_str() {
            "--labels" => {
                labels_path = Some(PathBuf::from(
                    iterator.next().ok_or("--labels requires a file path")?,
                ));
            }
            "--out" => {
                out = Some(PathBuf::from(
                    iterator.next().ok_or("--out requires a file path")?,
                ));
            }
            other => {
                return Err(format!(
                    "unknown flag {other}; usage: calibrate --labels <path> [--out <path>]"
                ));
            }
        }
    }
    let path = labels_path.ok_or("calibrate requires --labels <path>")?;
    let contents = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let labels: Vec<Label> = serde_json::from_str(&contents)
        .map_err(|error| format!("labels file is not valid JSON: {error}"))?;

    let (confusion, labeled) = tally(&labels);
    let agreement =
        judge_agreement(&confusion).ok_or("no human-labeled rows (every human_keep is null)")?;

    let receipt = render(&agreement, labeled, labels.len());
    println!("{receipt}");
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &receipt).map_err(|error| error.to_string())?;
        println!("receipt written to {}", path.display());
    }
    Ok(())
}

/// Build the judge-vs-human confusion from labels, skipping unlabeled rows.
/// Returns the confusion and the count of human-labeled rows.
fn tally(labels: &[Label]) -> (KeepConfusion, usize) {
    let mut confusion = KeepConfusion::default();
    let mut labeled = 0;
    for label in labels {
        if let Some(human_keep) = label.human_keep {
            confusion.record(label.judge_keep, human_keep);
            labeled += 1;
        }
    }
    (confusion, labeled)
}

fn pct(value: f64) -> String {
    if value.is_nan() {
        "n/a".to_owned()
    } else {
        format!("{:.0}%", value * 100.0)
    }
}

fn render(agreement: &JudgeAgreement, labeled: usize, total: usize) -> String {
    // A single-class label set (all keep or all drop) leaves a class unscored
    // (NaN) and κ undefined (the guard returns 1.0). That is no evidence of
    // calibration, so it must not read as "calibrated".
    let verdict = if agreement.tpr.is_nan() || agreement.tnr.is_nan() {
        "**insufficient labels** — the set has only one class; label both keep and drop drafts before trusting κ."
    } else if agreement.kappa >= KAPPA_BAR {
        "**calibrated** — kappa at/above the 0.80 human bar; the judge's keep rate is trustworthy."
    } else {
        "**not yet calibrated** — kappa below 0.80; do not trust the judge's keep rate. Revise the judge prompt/rubric and re-label."
    };
    format!(
        "# Judge calibration receipt\n\n\
         - Labeled drafts: {labeled} of {total}\n\
         - Cohen's kappa: {:.2} (human-level bar {KAPPA_BAR:.2})\n\
         - Accuracy: {} · TPR (recall on human-keeps): {} · TNR (on human-drops): {}\n\
         - Verdict: {verdict}\n",
        agreement.kappa,
        pct(agreement.accuracy),
        pct(agreement.tpr),
        pct(agreement.tnr),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(judge_keep: bool, human_keep: Option<bool>) -> Label {
        Label {
            question: String::new(),
            judge_keep,
            human_keep,
        }
    }

    #[test]
    fn tally_skips_unlabeled_rows() {
        let labels = vec![
            label(true, Some(true)),
            label(true, Some(false)),
            label(false, None), // not yet labeled — skipped
            label(false, Some(false)),
        ];
        let (confusion, labeled) = tally(&labels);
        assert_eq!(labeled, 3);
        assert_eq!(confusion.total(), 3);
        assert_eq!(confusion.both_keep, 1);
        assert_eq!(confusion.judge_only_keep, 1);
        assert_eq!(confusion.both_drop, 1);
    }

    #[test]
    fn render_flags_a_judge_below_the_bar() {
        let agreement = JudgeAgreement {
            kappa: 0.55,
            accuracy: 0.80,
            tpr: 0.9,
            tnr: 0.6,
            n: 40,
        };
        let receipt = render(&agreement, 40, 50);
        assert!(receipt.contains("not yet calibrated"), "{receipt}");
        assert!(receipt.contains("0.55"), "{receipt}");
    }

    #[test]
    fn render_passes_a_calibrated_judge() {
        let agreement = JudgeAgreement {
            kappa: 0.82,
            accuracy: 0.92,
            tpr: 0.95,
            tnr: 0.88,
            n: 40,
        };
        let receipt = render(&agreement, 40, 40);
        assert!(receipt.contains("calibrated"), "{receipt}");
        assert!(!receipt.contains("not yet"), "{receipt}");
    }

    #[test]
    fn render_flags_a_one_class_set_as_insufficient() {
        // No human-drops to score against → TNR is NaN. A skewed-but-real κ of
        // 1.0 here must NOT read as calibrated.
        let agreement = JudgeAgreement {
            kappa: 1.0,
            accuracy: 1.0,
            tpr: 0.9,
            tnr: f64::NAN,
            n: 10,
        };
        let receipt = render(&agreement, 10, 10);
        assert!(receipt.contains("insufficient labels"), "{receipt}");
        assert!(!receipt.contains("trustworthy"), "{receipt}");
        assert!(
            receipt.contains("n/a"),
            "a NaN rate must render as n/a: {receipt}"
        );
    }

    #[test]
    fn run_requires_a_labels_flag() {
        assert!(run(&[]).is_err());
    }

    #[test]
    fn run_errors_when_no_rows_are_labeled() {
        let path = std::env::temp_dir().join("me-calib-unlabeled.json");
        std::fs::write(&path, r#"[{"judge_keep":true,"human_keep":null}]"#).expect("write temp");
        let result = run(&["--labels".to_owned(), path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "all-null labels must error");
    }

    #[test]
    fn run_errors_on_invalid_json() {
        let path = std::env::temp_dir().join("me-calib-badjson.json");
        std::fs::write(&path, "not json").expect("write temp");
        let result = run(&["--labels".to_owned(), path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "invalid JSON must error");
    }

    #[test]
    fn run_accepts_the_content_feedback_export_shape() {
        let path = std::env::temp_dir().join("me-content-feedback-export.json");
        std::fs::write(
            &path,
            r#"[
                {"feedback_id":"f-keep","review_unit_id":"unit-keep","question":"Keep?","judge_keep":true,"human_keep":true,"gen_ai.prompt.version":"v1"},
                {"feedback_id":"f-drop","review_unit_id":"unit-drop","question":"Drop?","judge_keep":true,"human_keep":false,"gen_ai.evaluation.explanation":"Too vague"}
            ]"#,
        )
        .expect("write feedback export");
        let result = run(&["--labels".to_owned(), path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_ok(),
            "calibrate must consume feedback export: {result:?}"
        );
    }

    #[test]
    fn run_accepts_bytes_serialized_from_content_feedback_export() {
        let export: memory_engine_persistence::ContentFeedbackExport =
            serde_json::from_value(serde_json::json!({
                "feedbackId": "f-keep",
                "reviewUnitId": "unit-keep",
                "judgeKeep": true,
                "humanKeep": true,
                "question": "Keep?",
                "rationale": null,
                "gen_ai.system": "test",
                "gen_ai.request.model": "test-model",
                "gen_ai.prompt.version": "v1",
                "gen_ai.evaluation.score.value": 1.0,
                "gen_ai.evaluation.explanation": null
            }))
            .expect("construct actual export shape");
        let path = std::env::temp_dir().join(format!(
            "me-content-feedback-real-export-{}.json",
            std::process::id()
        ));
        let bytes = serde_json::to_vec(&vec![export]).expect("serialize actual export");
        let serialized = String::from_utf8(bytes.clone()).expect("export is UTF-8 JSON");
        assert!(serialized.contains("\"feedback_id\""), "{serialized}");
        assert!(serialized.contains("\"judge_keep\""), "{serialized}");
        std::fs::write(&path, bytes).expect("write actual export");
        let result = run(&["--labels".to_owned(), path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_ok(),
            "calibrate must consume bytes from ContentFeedbackExport: {result:?}"
        );
    }

    #[test]
    fn run_accepts_legacy_camel_case_feedback_labels() {
        let path = std::env::temp_dir().join(format!(
            "me-content-feedback-legacy-export-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"[{"judgeKeep":true,"humanKeep":true}]"#)
            .expect("write legacy export");
        let result = run(&["--labels".to_owned(), path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_ok(),
            "legacy export must remain readable: {result:?}"
        );
    }
}
