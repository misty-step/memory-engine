//! Statistical honesty for the generation eval receipt.
//!
//! The judge produces per-source rates (keep rate is a binary keep/drop
//! decision per draft, aggregated per source). Reporting a bare mean invites
//! the classic error — "keep rate 70% -> 74%, improved" — when the change is
//! pure run-to-run noise at n = 12 sources. These helpers attach a confidence
//! interval to a source-clustered mean and turn a baseline comparison into a
//! paired verdict that says "within noise" when the interval includes zero.
//!
//! Doctrine: harness-kit `verification-system-first.md`, "Eval & Benchmark
//! Rigor". Small-n honesty matters here, so the interval is Student-t (df =
//! n-1), not the 1.96 large-sample shorthand: at n = 12 the t factor is 2.20.

/// A mean with the half-width of its two-sided 95% confidence interval, so the
/// interval is `mean ± half_width`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeanCi {
    pub mean: f64,
    pub half_width: f64,
}

/// Two-sided 95% Student-t critical value for `df` degrees of freedom.
fn t_critical_95(df: usize) -> f64 {
    // t_{0.975, df} for df = 1..=30; beyond that the normal approximation (1.96)
    // is within ~2% and good enough.
    const TABLE: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match df {
        0 => f64::INFINITY,
        d if d <= 30 => TABLE[d - 1],
        _ => 1.96,
    }
}

/// Mean of `values` with the half-width of its two-sided 95% CI, treating each
/// value as one cluster (one source). Returns `None` for fewer than two values
/// — spread cannot be estimated from a single cluster. Sources are weighted
/// equally regardless of draft count (the conservative cluster-robust default:
/// a source with fewer drafts is noisier, so equal weighting widens the CI).
#[must_use]
pub fn mean_ci_95(values: &[f64]) -> Option<MeanCi> {
    let n = values.len();
    if n < 2 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = n as f64;
    let mean = values.iter().sum::<f64>() / count;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (count - 1.0);
    let standard_error = (variance / count).sqrt();
    Some(MeanCi {
        mean,
        half_width: t_critical_95(n - 1) * standard_error,
    })
}

/// A paired comparison of one per-source rate against a baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct PairedVerdict {
    pub mean_delta: f64,
    pub half_width: f64,
    pub paired: usize,
    /// True when the 95% CI of the mean paired delta includes zero: the change
    /// is not resolved at this n (not detected, not proof of no effect — a true
    /// effect up to the interval bound could still hide).
    pub within_noise: bool,
}

/// Pair `current` and `baseline` per-source rates by source id and compute the
/// mean paired delta (`current - baseline`) with its 95% CI. Sources missing
/// from either side are dropped (pairing is the variance reduction; unpaired
/// items would only add noise). Returns `None` with fewer than two paired
/// sources.
#[must_use]
pub fn paired_verdict(
    current: &[(String, f64)],
    baseline: &[(String, f64)],
) -> Option<PairedVerdict> {
    let deltas: Vec<f64> = current
        .iter()
        .filter_map(|(id, value)| {
            baseline
                .iter()
                .find(|(baseline_id, _)| baseline_id == id)
                .map(|(_, baseline_value)| value - baseline_value)
        })
        .collect();
    let interval = mean_ci_95(&deltas)?;
    Some(PairedVerdict {
        mean_delta: interval.mean,
        half_width: interval.half_width,
        paired: deltas.len(),
        // CI includes 0. (Identical deltas give zero variance and a zero-width
        // CI, reading any nonzero shift as detectable; real per-source rates
        // always vary, so that degenerate case cannot fire here.)
        within_noise: interval.mean.abs() <= interval.half_width,
    })
}

/// Extract per-source keep rates (0.0–1.0) from a rendered judge receipt: the
/// `source` and `keep` columns of the `## Model judge` table. Rows where the
/// judge failed (cells are `—`) are skipped. Used to read a prior receipt as
/// the paired baseline.
#[must_use]
pub fn parse_keep_rates(receipt: &str) -> Vec<(String, f64)> {
    let mut lines = receipt
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("## Model judge"));
    if lines.next().is_none() {
        return Vec::new();
    }
    let Some(header) = lines
        .by_ref()
        .find(|line| line.trim_start().starts_with('|') && line.contains("source"))
    else {
        return Vec::new();
    };
    let columns: Vec<String> = header
        .split('|')
        .map(|cell| cell.trim().to_lowercase())
        .collect();
    let (Some(source_index), Some(keep_index)) = (
        columns.iter().position(|column| column == "source"),
        columns.iter().position(|column| column == "keep"),
    ) else {
        return Vec::new();
    };

    let mut rates = Vec::new();
    for line in lines {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('|') {
            break;
        }
        if trimmed.contains("---") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').collect();
        let (Some(source), Some(keep)) = (cells.get(source_index), cells.get(keep_index)) else {
            continue;
        };
        // Require the trailing '%' so a stray bare number cannot be silently
        // mis-scaled (e.g. "0.33" -> 0.0033).
        if let Some(percent) = keep
            .trim()
            .strip_suffix('%')
            .and_then(|n| n.parse::<f64>().ok())
        {
            rates.push((source.trim().to_owned(), percent / 100.0));
        }
    }
    rates
}

/// A 2×2 confusion of the model judge's keep decision against a human's, for
/// calibrating the judge before its scores are trusted (the rigor reference's
/// "validate the judge against human labels" step).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeepConfusion {
    pub both_keep: u64,
    /// Judge keep, human drop (the judge is too lenient on these).
    pub judge_only_keep: u64,
    /// Judge drop, human keep (the judge is too harsh on these).
    pub human_only_keep: u64,
    pub both_drop: u64,
}

impl KeepConfusion {
    pub fn record(&mut self, judge_keep: bool, human_keep: bool) {
        match (judge_keep, human_keep) {
            (true, true) => self.both_keep += 1,
            (true, false) => self.judge_only_keep += 1,
            (false, true) => self.human_only_keep += 1,
            (false, false) => self.both_drop += 1,
        }
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.both_keep + self.judge_only_keep + self.human_only_keep + self.both_drop
    }
}

/// Judge-vs-human agreement on the keep decision. `kappa` is Cohen's κ
/// (chance-corrected); the rigor bar is human-level κ ≈ 0.80. TPR/TNR are
/// reported instead of raw agreement because keep rates are usually imbalanced,
/// where a uniformly lenient or harsh judge can post high raw agreement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgeAgreement {
    pub kappa: f64,
    pub accuracy: f64,
    pub tpr: f64,
    pub tnr: f64,
    pub n: u64,
}

/// Cohen's κ + TPR/TNR for a judge-vs-human keep confusion. `None` for an empty
/// set. TPR/TNR are `NaN` when a human class is unobserved (no positives /
/// negatives to score against).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn judge_agreement(confusion: &KeepConfusion) -> Option<JudgeAgreement> {
    let n = confusion.total();
    if n == 0 {
        return None;
    }
    let total = n as f64;
    let keep_keep = confusion.both_keep as f64;
    let keep_drop = confusion.judge_only_keep as f64;
    let drop_keep = confusion.human_only_keep as f64;
    let drop_drop = confusion.both_drop as f64;
    let observed = (keep_keep + drop_drop) / total;
    // Chance agreement: P(both call keep) + P(both call drop).
    let expected = ((keep_keep + keep_drop) / total) * ((keep_keep + drop_keep) / total)
        + ((drop_keep + drop_drop) / total) * ((keep_drop + drop_drop) / total);
    let kappa = if (1.0 - expected).abs() < f64::EPSILON {
        1.0 // perfect-by-construction agreement (one class only)
    } else {
        (observed - expected) / (1.0 - expected)
    };
    let human_keeps = keep_keep + drop_keep;
    let human_drops = keep_drop + drop_drop;
    Some(JudgeAgreement {
        kappa,
        accuracy: observed,
        tpr: if human_keeps == 0.0 {
            f64::NAN
        } else {
            keep_keep / human_keeps
        },
        tnr: if human_drops == 0.0 {
            f64::NAN
        } else {
            drop_drop / human_drops
        },
        n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-3
    }

    #[test]
    fn mean_ci_uses_student_t_for_small_n() {
        // values 2,4,6: mean 4, sample var 4, SE = sqrt(4/3) = 1.1547,
        // t_{0.975,2} = 4.303 -> half-width 4.969.
        let ci = mean_ci_95(&[2.0, 4.0, 6.0]).expect("three values");
        assert!(approx(ci.mean, 4.0), "mean {}", ci.mean);
        assert!(approx(ci.half_width, 4.969), "half-width {}", ci.half_width);
    }

    #[test]
    fn mean_ci_needs_at_least_two_clusters() {
        assert_eq!(mean_ci_95(&[]), None);
        assert_eq!(mean_ci_95(&[0.7]), None);
    }

    #[test]
    fn a_small_paired_delta_is_within_noise() {
        // The falsifier the whole ticket exists for: a few points of keep-rate
        // movement with real per-source spread must read as noise, not a win.
        let current = vec![
            ("a".to_owned(), 0.75),
            ("b".to_owned(), 0.50),
            ("c".to_owned(), 0.90),
        ];
        let baseline = vec![
            ("a".to_owned(), 0.80),
            ("b".to_owned(), 0.50),
            ("c".to_owned(), 0.71),
        ];
        let verdict = paired_verdict(&current, &baseline).expect("three pairs");
        assert_eq!(verdict.paired, 3);
        assert!(
            verdict.within_noise,
            "delta {} hw {}",
            verdict.mean_delta, verdict.half_width
        );
    }

    #[test]
    fn a_large_consistent_paired_delta_is_detectable() {
        let current = vec![
            ("a".to_owned(), 0.90),
            ("b".to_owned(), 0.92),
            ("c".to_owned(), 0.88),
        ];
        let baseline = vec![
            ("a".to_owned(), 0.40),
            ("b".to_owned(), 0.42),
            ("c".to_owned(), 0.38),
        ];
        let verdict = paired_verdict(&current, &baseline).expect("three pairs");
        assert!(
            !verdict.within_noise,
            "delta {} hw {}",
            verdict.mean_delta, verdict.half_width
        );
        assert!(
            approx(verdict.mean_delta, 0.50),
            "delta {}",
            verdict.mean_delta
        );
    }

    #[test]
    fn paired_verdict_matches_only_shared_sources() {
        let current = vec![
            ("a".to_owned(), 0.8),
            ("b".to_owned(), 0.6),
            ("c".to_owned(), 0.9),
        ];
        let baseline = vec![
            ("a".to_owned(), 0.7),
            ("b".to_owned(), 0.5),
            ("d".to_owned(), 0.4),
        ];
        let verdict = paired_verdict(&current, &baseline).expect("two shared pairs");
        assert_eq!(verdict.paired, 2);
    }

    #[test]
    fn parse_keep_rates_reads_the_judge_table_only() {
        let receipt = "\
# Generation eval receipt

| source | category | drafts |
| --- | --- | --- |
| nato | list | 3 |

## Model judge (rubric 1-5)

| source | faithfulness | question quality | distractors | keep | judge cost |
| --- | --- | --- | --- | --- | --- |
| nato | 5.0 | 4.3 | 2.7 | 33% | $0.0054 |
| curie | 4.8 | 4.7 | 3.5 | 50% | $0.0090 |
| broke | — | — | — | — | JUDGE FAILED: boom |

- Judge means: keep rate 42%
";
        let rates = parse_keep_rates(receipt);
        assert_eq!(
            rates,
            vec![("nato".to_owned(), 0.33), ("curie".to_owned(), 0.50)],
            "must read the judge table, skip the deterministic table and the failed row",
        );
    }

    #[test]
    fn judge_agreement_computes_cohens_kappa_and_rates() {
        // 40 both-keep, 10 judge-only-keep, 5 human-only-keep, 45 both-drop:
        // observed 0.85, chance 0.50 -> kappa 0.70.
        let mut confusion = KeepConfusion::default();
        for _ in 0..40 {
            confusion.record(true, true);
        }
        for _ in 0..10 {
            confusion.record(true, false);
        }
        for _ in 0..5 {
            confusion.record(false, true);
        }
        for _ in 0..45 {
            confusion.record(false, false);
        }
        let agreement = judge_agreement(&confusion).expect("nonempty");
        assert_eq!(agreement.n, 100);
        assert!(approx(agreement.kappa, 0.70), "kappa {}", agreement.kappa);
        assert!(
            approx(agreement.accuracy, 0.85),
            "accuracy {}",
            agreement.accuracy
        );
        assert!(approx(agreement.tpr, 40.0 / 45.0), "tpr {}", agreement.tpr);
        assert!(approx(agreement.tnr, 45.0 / 55.0), "tnr {}", agreement.tnr);
    }

    #[test]
    fn judge_agreement_perfect_is_kappa_one() {
        let mut confusion = KeepConfusion::default();
        for _ in 0..7 {
            confusion.record(true, true);
        }
        for _ in 0..3 {
            confusion.record(false, false);
        }
        let agreement = judge_agreement(&confusion).expect("nonempty");
        assert!(approx(agreement.kappa, 1.0), "kappa {}", agreement.kappa);
    }

    #[test]
    fn judge_agreement_empty_is_none() {
        assert_eq!(judge_agreement(&KeepConfusion::default()), None);
    }
}
