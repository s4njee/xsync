//! Correctness-first benchmark regression policy.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::report::{BenchResult, Report, REPORT_SCHEMA};

/// Minimum samples for a gated result.
pub const MINIMUM_REPETITIONS: usize = 5;
/// Maximum MAD/median before a result is too noisy to compare.
pub const MAXIMUM_DISPERSION: f64 = 0.15;
/// Allowed paired-speedup degradation.
pub const REGRESSION_TOLERANCE: f64 = 0.15;

/// Machine-readable gate decision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOutcome {
    /// Correctness or performance failures.
    pub failures: Vec<String>,
    /// Conditions which prevented comparison or weaken evidence.
    pub warnings: Vec<String>,
    /// Informational comparison details.
    pub notes: Vec<String>,
    /// Historical paired comparisons actually performed.
    pub comparisons_performed: u64,
    /// Candidate results skipped during historical comparison.
    pub comparisons_skipped: u64,
    /// Whether reports passed comparability checks.
    pub reports_comparable: bool,
}

impl GateOutcome {
    /// Whether the gate passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// Render a concise human-readable decision.
    #[must_use]
    pub fn render(&self) -> String {
        let status = if self.passed() { "passed" } else { "FAILED" };
        let mut output = format!(
            "gate {status}: {} comparison(s) performed, {} skipped",
            self.comparisons_performed, self.comparisons_skipped
        );
        for failure in &self.failures {
            write!(&mut output, "\nFAIL: {failure}").expect("writing to String cannot fail");
        }
        for warning in &self.warnings {
            write!(&mut output, "\nWARN: {warning}").expect("writing to String cannot fail");
        }
        for note in &self.notes {
            write!(&mut output, "\nNOTE: {note}").expect("writing to String cannot fail");
        }
        output
    }
}

/// Evaluate correctness and optional historical performance regression.
///
/// Correctness is always checked on `current`. Performance comparisons are
/// performed only when both reports have identical environment, session, and
/// content-pinned corpus identity. Strict mode fails when no historical paired
/// comparison was possible.
#[must_use]
pub fn evaluate_gate(current: &Report, historical: Option<&Report>, strict: bool) -> GateOutcome {
    let mut outcome = GateOutcome::default();
    check_current(current, &mut outcome);

    let Some(historical) = historical else {
        outcome
            .warnings
            .push("no historical baseline supplied; performance is advisory".to_owned());
        if strict {
            outcome
                .failures
                .push("strict: no historical paired comparison was performed".to_owned());
        }
        return outcome;
    };

    compare_reports(current, historical, strict, &mut outcome);
    outcome
}

fn compare_reports(current: &Report, historical: &Report, strict: bool, outcome: &mut GateOutcome) {
    if !historical_is_valid(historical, strict, outcome) {
        return;
    }

    let mut mismatch = Vec::new();
    if current.schema != REPORT_SCHEMA || historical.schema != REPORT_SCHEMA {
        mismatch.push("report schema");
    }
    if current.environment != historical.environment {
        mismatch.push("environment");
    }
    if current.session != historical.session {
        mismatch.push("session configuration");
    }
    if current.corpus.schema != historical.corpus.schema
        || current.corpus.manifest_digest != historical.corpus.manifest_digest
    {
        mismatch.push("content-pinned corpus");
    }
    if !mismatch.is_empty() {
        outcome.warnings.push(format!(
            "reports differ in {}; no performance comparison performed",
            mismatch.join(", ")
        ));
        if strict {
            outcome
                .failures
                .push("strict: historical report is not comparable".to_owned());
        }
        return;
    }
    outcome.reports_comparable = true;

    let historical_by_name: BTreeMap<_, _> = historical
        .results
        .iter()
        .map(|result| (result.name.as_str(), result))
        .collect();
    for result in &current.results {
        let Some(previous) = historical_by_name.get(result.name.as_str()) else {
            outcome.comparisons_skipped += 1;
            outcome
                .notes
                .push(format!("{}: no historical counterpart", result.name));
            continue;
        };
        let (Some(current_ratio), Some(previous_ratio)) =
            (result.paired_ratio_median, previous.paired_ratio_median)
        else {
            outcome.comparisons_skipped += 1;
            outcome.notes.push(format!(
                "{}: no paired ratio; absolute time is not gated",
                result.name
            ));
            continue;
        };
        if result.baseline != previous.baseline {
            outcome.comparisons_skipped += 1;
            outcome.notes.push(format!(
                "{}: paired baseline differs ({:?} vs {:?})",
                result.name, result.baseline, previous.baseline
            ));
            continue;
        }
        if !stable(result) || !stable(previous) {
            outcome.comparisons_skipped += 1;
            outcome.notes.push(format!(
                "{}: wall or paired-ratio dispersion exceeds 15%",
                result.name
            ));
            continue;
        }
        outcome.comparisons_performed += 1;
        if current_ratio < previous_ratio * (1.0 - REGRESSION_TOLERANCE) {
            outcome.failures.push(format!(
                "{}: paired speedup fell from {:.3}x to {:.3}x (>15% worse)",
                result.name, previous_ratio, current_ratio
            ));
        } else {
            outcome.notes.push(format!(
                "{}: paired speedup {:.3}x (historical {:.3}x)",
                result.name, current_ratio, previous_ratio
            ));
        }
    }

    if outcome.comparisons_performed == 0 {
        outcome
            .warnings
            .push("no paired performance comparisons were performed".to_owned());
        if strict {
            outcome
                .failures
                .push("strict: zero paired comparisons".to_owned());
        }
    }
}

fn historical_is_valid(historical: &Report, strict: bool, outcome: &mut GateOutcome) -> bool {
    let mut historical_check = GateOutcome::default();
    check_current(historical, &mut historical_check);
    if historical_check.failures.is_empty() {
        return true;
    }

    outcome.warnings.push(format!(
        "historical report failed {} correctness check(s); no performance comparison performed",
        historical_check.failures.len()
    ));
    if strict {
        outcome
            .failures
            .push("strict: historical report failed correctness checks".to_owned());
    }
    false
}

fn check_current(report: &Report, outcome: &mut GateOutcome) {
    if report.schema != REPORT_SCHEMA {
        outcome.failures.push(format!(
            "unsupported current report schema '{}'",
            report.schema
        ));
    }
    if report.build.profile != "release" {
        outcome
            .failures
            .push("gated report was not produced by a release build".to_owned());
    }
    for result in &report.results {
        if result.samples.len() < MINIMUM_REPETITIONS {
            outcome.failures.push(format!(
                "{}: fewer than {MINIMUM_REPETITIONS} repetitions ({})",
                result.name,
                result.samples.len()
            ));
        }
        if result.median_wall_seconds > 0.0
            && result.mad_wall_seconds / result.median_wall_seconds > MAXIMUM_DISPERSION
        {
            outcome.warnings.push(format!(
                "{}: wall dispersion exceeds 15%; performance is unverified",
                result.name
            ));
        }
        let item_counts = result
            .samples
            .iter()
            .map(|sample| sample.item_count)
            .collect::<std::collections::BTreeSet<_>>();
        let logical_bytes = result
            .samples
            .iter()
            .map(|sample| sample.logical_bytes)
            .collect::<std::collections::BTreeSet<_>>();
        if item_counts.len() != 1 || logical_bytes.len() != 1 {
            outcome.failures.push(format!(
                "{}: item or logical-byte counts differ across repetitions",
                result.name
            ));
        }
        for sample in &result.samples {
            if !sample.oracle.passed
                || sample.oracle.mismatch_count != 0
                || sample.oracle.expected_manifest_digest != report.corpus.manifest_digest
                || sample.oracle.actual_manifest_digest != report.corpus.manifest_digest
            {
                outcome.failures.push(format!(
                    "{} repetition {}: destination manifest oracle failed",
                    result.name, sample.repetition
                ));
            }
            if sample.item_count != sample.oracle.item_count
                || sample.logical_bytes != sample.oracle.logical_bytes
            {
                outcome.failures.push(format!(
                    "{} repetition {}: reported counts differ from oracle",
                    result.name, sample.repetition
                ));
            }
        }
    }
}

fn stable(result: &BenchResult) -> bool {
    let wall_stable = result.median_wall_seconds > 0.0
        && result.mad_wall_seconds / result.median_wall_seconds <= MAXIMUM_DISPERSION;
    let ratio_stable = match (result.paired_ratio_median, result.paired_ratio_mad) {
        (Some(median), Some(mad)) => median > 0.0 && mad / median <= MAXIMUM_DISPERSION,
        _ => false,
    };
    result.samples.len() >= MINIMUM_REPETITIONS && wall_stable && ratio_stable
}

#[cfg(test)]
mod tests {
    use crate::test_support::{report, verification};

    use super::*;

    #[test]
    fn comparable_paired_reports_pass_and_regression_fails() {
        let historical = report(1.0);
        let current = report(1.0);
        let passed = evaluate_gate(&current, Some(&historical), true);
        assert!(passed.passed(), "{}", passed.render());
        assert_eq!(passed.comparisons_performed, 1);

        let regressed = report(1.5);
        let failed = evaluate_gate(&regressed, Some(&historical), true);
        assert!(!failed.passed());
        assert!(failed
            .failures
            .iter()
            .any(|failure| failure.contains("paired speedup fell")));
    }

    #[test]
    fn correctness_always_fails_even_without_baseline() {
        let mut current = report(1.0);
        current.results[1].samples[2].oracle = verification(false);
        let outcome = evaluate_gate(&current, None, false);
        assert!(!outcome.passed());
        assert!(outcome
            .failures
            .iter()
            .any(|failure| failure.contains("oracle failed")));
    }

    #[test]
    fn unlike_environment_skips_and_strict_fails() {
        let historical = report(1.0);
        let mut current = report(1.0);
        current.environment.filesystem = "ext4".to_owned();

        let advisory = evaluate_gate(&current, Some(&historical), false);
        assert!(advisory.passed());
        assert_eq!(advisory.comparisons_performed, 0);
        assert!(!advisory.reports_comparable);

        let strict = evaluate_gate(&current, Some(&historical), true);
        assert!(!strict.passed());
    }

    #[test]
    fn strict_mode_rejects_a_gate_that_compared_nothing() {
        let current = report(1.0);
        let outcome = evaluate_gate(&current, None, true);
        assert!(!outcome.passed());
        assert_eq!(outcome.comparisons_performed, 0);
    }

    #[test]
    fn invalid_historical_correctness_is_never_used() {
        let current = report(1.0);
        let mut historical = report(1.0);
        historical.results[1].samples[0].oracle = verification(false);

        let outcome = evaluate_gate(&current, Some(&historical), true);
        assert!(!outcome.passed());
        assert_eq!(outcome.comparisons_performed, 0);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("historical report failed")));
    }
}
