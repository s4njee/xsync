//! Versioned paired clone/copy benchmark report.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::clone_spike::CloneDisposition;
use crate::report::Environment;

/// Clone spike evidence schema.
pub const CLONE_REPORT_SCHEMA: &str = "xsync.clone-bench.report.v1";
const MINIMUM_REPETITIONS: usize = 5;

/// Copy method in one paired observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneMethod {
    /// Ordinary staged copy with full verification.
    BufferedVerified,
    /// Platform clone/reflink attempt with transparent staged fallback.
    CloneOrFallback,
}

/// Truthful cache label for the observation sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    /// First observation without an eviction claim.
    FirstPass,
    /// Later observation over a previously accessed corpus.
    Warm,
}

/// One paired clone/copy observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneSample {
    /// Pair identifier.
    pub repetition: u32,
    /// Within-pair execution order.
    pub method_order: u32,
    /// Executed method.
    pub method: CloneMethod,
    /// Staged operation wall time; independent verification follows outside the timer.
    pub wall_seconds: f64,
    /// Whether the platform clone succeeded or buffered fallback was used.
    pub disposition: CloneDisposition,
    /// True only after the final output passed verification.
    pub verification_passed: bool,
    /// First-pass or warm-cache label.
    pub cache_state: CacheState,
}

/// Per-method derived evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneMethodSummary {
    /// Method.
    pub method: CloneMethod,
    /// Median verified wall time.
    pub median_wall_seconds: f64,
    /// Median absolute deviation.
    pub mad_wall_seconds: f64,
}

/// Complete paired clone/copy report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneReport {
    /// Report schema.
    pub schema: String,
    /// Machine/build identity.
    pub environment: Environment,
    /// `file` or `directory`.
    pub object_kind: String,
    /// Independent source manifest digest.
    pub source_manifest_digest: String,
    /// Source logical bytes.
    pub logical_bytes: u64,
    /// Whether final-name paranoid readback was enabled.
    pub paranoid: bool,
    /// Every paired observation.
    pub samples: Vec<CloneSample>,
    /// Per-method medians and MAD.
    pub summaries: Vec<CloneMethodSummary>,
    /// Median same-repetition buffered/clone speedup.
    pub paired_clone_speedup: f64,
    /// MAD of same-repetition speedup.
    pub paired_clone_speedup_mad: f64,
    /// Whether every clone attempt used the platform capability.
    pub clone_capability_available: bool,
}

impl CloneReport {
    /// Validate paired observations and derive statistics.
    ///
    /// # Errors
    ///
    /// Returns an error for missing/failed samples, fixed method order, or
    /// invalid timing.
    pub fn from_samples(
        environment: Environment,
        object_kind: String,
        source_manifest_digest: String,
        logical_bytes: u64,
        paranoid: bool,
        samples: Vec<CloneSample>,
    ) -> Result<Self, CloneReportError> {
        let mut by_method = BTreeMap::<CloneMethod, Vec<&CloneSample>>::new();
        let mut by_repetition = BTreeMap::<u32, Vec<&CloneSample>>::new();
        for sample in &samples {
            if !sample.verification_passed
                || !sample.wall_seconds.is_finite()
                || sample.wall_seconds <= 0.0
            {
                return Err(CloneReportError::InvalidSample {
                    repetition: sample.repetition,
                });
            }
            by_method.entry(sample.method).or_default().push(sample);
            by_repetition
                .entry(sample.repetition)
                .or_default()
                .push(sample);
        }
        for method in [CloneMethod::BufferedVerified, CloneMethod::CloneOrFallback] {
            if by_method.get(&method).map_or(0, Vec::len) < MINIMUM_REPETITIONS {
                return Err(CloneReportError::TooFewRepetitions { method });
            }
        }
        let mut clone_before = false;
        let mut clone_after = false;
        let mut ratios = Vec::new();
        for (repetition, pair) in by_repetition {
            if pair.len() != 2 {
                return Err(CloneReportError::InvalidPair { repetition });
            }
            let buffered = pair
                .iter()
                .find(|sample| sample.method == CloneMethod::BufferedVerified)
                .ok_or(CloneReportError::InvalidPair { repetition })?;
            let clone = pair
                .iter()
                .find(|sample| sample.method == CloneMethod::CloneOrFallback)
                .ok_or(CloneReportError::InvalidPair { repetition })?;
            clone_before |= clone.method_order < buffered.method_order;
            clone_after |= clone.method_order > buffered.method_order;
            ratios.push(buffered.wall_seconds / clone.wall_seconds);
        }
        if !clone_before || !clone_after {
            return Err(CloneReportError::FixedOrder);
        }
        let summaries = [CloneMethod::BufferedVerified, CloneMethod::CloneOrFallback]
            .into_iter()
            .map(|method| {
                let values = by_method[&method]
                    .iter()
                    .map(|sample| sample.wall_seconds)
                    .collect::<Vec<_>>();
                CloneMethodSummary {
                    method,
                    median_wall_seconds: median(&values),
                    mad_wall_seconds: mad(&values),
                }
            })
            .collect();
        let clone_capability_available = by_method[&CloneMethod::CloneOrFallback]
            .iter()
            .all(|sample| sample.disposition == CloneDisposition::Cloned);
        Ok(Self {
            schema: CLONE_REPORT_SCHEMA.to_owned(),
            environment,
            object_kind,
            source_manifest_digest,
            logical_bytes,
            paranoid,
            samples,
            summaries,
            paired_clone_speedup: median(&ratios),
            paired_clone_speedup_mad: mad(&ratios),
            clone_capability_available,
        })
    }

    /// Human-readable report retaining every observation.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut output = String::from("# xsync local clone/reflink spike\n\n");
        writeln!(
            &mut output,
            "- Schema: `{}`\n- Host/filesystem: `{}` / `{}`\n- Object: `{}` ({} logical bytes)\n- Source manifest: `{}`\n- Paranoid readback: `{}`\n- Platform clone available: `{}`\n",
            self.schema,
            self.environment.host,
            self.environment.filesystem,
            self.object_kind,
            self.logical_bytes,
            self.source_manifest_digest,
            self.paranoid,
            self.clone_capability_available
        )
        .expect("writing to String cannot fail");
        output.push_str("## Summary\n\n| Method | Median | MAD |\n|---|---:|---:|\n");
        for summary in &self.summaries {
            writeln!(
                &mut output,
                "| {:?} | {:.6} s | {:.6} s |",
                summary.method, summary.median_wall_seconds, summary.mad_wall_seconds
            )
            .expect("writing to String cannot fail");
        }
        writeln!(
            &mut output,
            "\nPaired verified clone speedup: **{:.3}x** (MAD {:.3}x).\n",
            self.paired_clone_speedup, self.paired_clone_speedup_mad
        )
        .expect("writing to String cannot fail");
        output.push_str(
            "Wall time covers the staged operation and publication. The independent oracle runs immediately afterward, outside the timer, and a sample is retained only when it passes.\n\n",
        );
        output.push_str("## Repetitions\n\n| Rep | Order | Method | Wall | Disposition | Cache | Verified |\n|---:|---:|---|---:|---|---|---|\n");
        for sample in &self.samples {
            writeln!(
                &mut output,
                "| {} | {} | {:?} | {:.6} s | {:?} | {:?} | {} |",
                sample.repetition,
                sample.method_order,
                sample.method,
                sample.wall_seconds,
                sample.disposition,
                sample.cache_state,
                sample.verification_passed
            )
            .expect("writing to String cannot fail");
        }
        output
    }
}

/// Clone report validation failure.
#[derive(Debug, thiserror::Error)]
pub enum CloneReportError {
    /// Too few method observations.
    #[error("method {method:?} requires at least {MINIMUM_REPETITIONS} repetitions")]
    TooFewRepetitions {
        /// Under-sampled method.
        method: CloneMethod,
    },
    /// Failed verification or timing.
    #[error("repetition {repetition} has an invalid or unverified observation")]
    InvalidSample {
        /// Invalid repetition.
        repetition: u32,
    },
    /// Pair does not contain exactly both methods.
    #[error("repetition {repetition} is not a complete method pair")]
    InvalidPair {
        /// Invalid repetition.
        repetition: u32,
    },
    /// Clone was always ordered on the same side of buffered copy.
    #[error("method order did not rotate across repetitions")]
    FixedOrder,
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        sorted[middle - 1].midpoint(sorted[middle])
    } else {
        sorted[middle]
    }
}

fn mad(values: &[f64]) -> f64 {
    let center = median(values);
    median(
        &values
            .iter()
            .map(|value| (value - center).abs())
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        Environment {
            source_revision: "revision".to_owned(),
            build_id: "build".to_owned(),
            host: "host".to_owned(),
            hardware: "hardware".to_owned(),
            os: "os".to_owned(),
            kernel: "kernel".to_owned(),
            filesystem: "fs".to_owned(),
        }
    }

    #[test]
    fn paired_report_rotates_and_derives_speedup() {
        let mut samples = Vec::new();
        for repetition in 0_u32..5 {
            let clone_first = repetition.is_multiple_of(2);
            samples.push(CloneSample {
                repetition,
                method_order: u32::from(clone_first),
                method: CloneMethod::BufferedVerified,
                wall_seconds: 2.0,
                disposition: CloneDisposition::BufferedFallback,
                verification_passed: true,
                cache_state: CacheState::Warm,
            });
            samples.push(CloneSample {
                repetition,
                method_order: u32::from(!clone_first),
                method: CloneMethod::CloneOrFallback,
                wall_seconds: 1.0,
                disposition: CloneDisposition::Cloned,
                verification_passed: true,
                cache_state: CacheState::Warm,
            });
        }
        let report = CloneReport::from_samples(
            environment(),
            "file".to_owned(),
            "a".repeat(64),
            100,
            true,
            samples,
        )
        .unwrap();
        assert!((report.paired_clone_speedup - 2.0).abs() < f64::EPSILON);
        assert!(report.clone_capability_available);
        assert!(report.to_markdown().contains("## Repetitions"));
    }
}
