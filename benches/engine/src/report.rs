//! Versioned scanner/planner benchmark evidence.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Scanner/planner evidence schema.
pub const ENGINE_REPORT_SCHEMA: &str = "xsync.engine-bench.report.v1";
/// Minimum repetitions in a report used as benchmark evidence.
pub const MINIMUM_REPETITIONS: usize = 5;

/// Machine and build identity for one benchmark run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Source revision or explicit `unknown`.
    pub source_revision: String,
    /// Benchmark binary build identifier.
    pub build_id: String,
    /// Host name.
    pub host: String,
    /// Hardware description.
    pub hardware: String,
    /// Operating system.
    pub os: String,
    /// Kernel version.
    pub kernel: String,
    /// Filesystem containing the corpus.
    pub filesystem: String,
}

/// One isolated worker observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSample {
    /// Zero-based repetition identifier.
    pub repetition: u32,
    /// Entries found in each scan.
    pub item_count: u64,
    /// Destination scan time.
    pub destination_scan_seconds: f64,
    /// Source scan time.
    pub source_scan_seconds: f64,
    /// Sum of both syscall-sensitive scan phases.
    pub syscall_phase_seconds: f64,
    /// Entries per second across both scans.
    pub scan_entries_per_second: f64,
    /// Time to build the destination `HashMap`.
    pub destination_index_seconds: f64,
    /// Metadata classification time after the destination index exists.
    pub planner_seconds: f64,
    /// Maximum bounded-channel occupancy observed by producers.
    pub queue_high_water: u64,
    /// Process high-water resident set size.
    pub peak_rss_bytes: u64,
    /// Number of source items represented by the resulting plan.
    pub planned_items: u64,
}

/// Derived medians and dispersion while retaining every raw sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSummary {
    /// Median scan rate.
    pub median_scan_entries_per_second: f64,
    /// MAD of scan rate.
    pub mad_scan_entries_per_second: f64,
    /// Median combined syscall-sensitive scan time.
    pub median_syscall_phase_seconds: f64,
    /// MAD of combined syscall-sensitive scan time.
    pub mad_syscall_phase_seconds: f64,
    /// Median destination-index build time.
    pub median_destination_index_seconds: f64,
    /// Median classification time.
    pub median_planner_seconds: f64,
    /// Highest process peak RSS across repetitions.
    pub peak_rss_bytes: u64,
    /// Highest queue high-water mark across repetitions.
    pub queue_high_water: u64,
}

/// Complete scanner/planner report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineReport {
    /// Report schema.
    pub schema: String,
    /// Build and machine identity.
    pub environment: Environment,
    /// Human-readable corpus topology label.
    pub shape: String,
    /// Independent content/metadata manifest digest.
    pub corpus_manifest_digest: String,
    /// Configured bounded-channel capacity.
    pub channel_capacity: u64,
    /// Explicit peak-RSS budget.
    pub memory_budget_bytes: u64,
    /// True when every isolated repetition stayed within the memory budget.
    pub memory_budget_passed: bool,
    /// Raw isolated repetitions.
    pub samples: Vec<EngineSample>,
    /// Derived summary.
    pub summary: EngineSummary,
    /// Platform/correctness qualification notes.
    pub notes: Vec<String>,
}

impl EngineReport {
    /// Validate raw observations and derive summary statistics.
    ///
    /// # Errors
    ///
    /// Returns an error for fewer than five observations, count mismatches,
    /// invalid timings/rates, or queue capacity violations.
    pub fn from_samples(
        environment: Environment,
        shape: String,
        corpus_manifest_digest: String,
        channel_capacity: u64,
        memory_budget_bytes: u64,
        samples: Vec<EngineSample>,
        notes: Vec<String>,
    ) -> Result<Self, ReportError> {
        validate_samples(&samples, channel_capacity)?;
        let scan_rates = samples
            .iter()
            .map(|sample| sample.scan_entries_per_second)
            .collect::<Vec<_>>();
        let syscall_times = samples
            .iter()
            .map(|sample| sample.syscall_phase_seconds)
            .collect::<Vec<_>>();
        let summary = EngineSummary {
            median_scan_entries_per_second: median(&scan_rates),
            mad_scan_entries_per_second: mad(&scan_rates),
            median_syscall_phase_seconds: median(&syscall_times),
            mad_syscall_phase_seconds: mad(&syscall_times),
            median_destination_index_seconds: median(
                &samples
                    .iter()
                    .map(|sample| sample.destination_index_seconds)
                    .collect::<Vec<_>>(),
            ),
            median_planner_seconds: median(
                &samples
                    .iter()
                    .map(|sample| sample.planner_seconds)
                    .collect::<Vec<_>>(),
            ),
            peak_rss_bytes: samples
                .iter()
                .map(|sample| sample.peak_rss_bytes)
                .max()
                .unwrap_or(0),
            queue_high_water: samples
                .iter()
                .map(|sample| sample.queue_high_water)
                .max()
                .unwrap_or(0),
        };
        let memory_budget_passed = summary.peak_rss_bytes <= memory_budget_bytes;
        Ok(Self {
            schema: ENGINE_REPORT_SCHEMA.to_owned(),
            environment,
            shape,
            corpus_manifest_digest,
            channel_capacity,
            memory_budget_bytes,
            memory_budget_passed,
            samples,
            summary,
            notes,
        })
    }

    /// Render a reviewable Markdown artifact.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut output = String::from("# xsync scanner/planner evidence\n\n");
        writeln!(
            &mut output,
            "- Schema: `{}`\n- Shape: `{}`\n- Corpus manifest: `{}`",
            self.schema, self.shape, self.corpus_manifest_digest
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "- Host/OS: `{}` / `{}` / `{}`\n- Hardware: `{}`\n- Filesystem: `{}`",
            self.environment.host,
            self.environment.os,
            self.environment.kernel,
            self.environment.hardware,
            self.environment.filesystem
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "- Memory budget: {} bytes — **{}**\n",
            self.memory_budget_bytes,
            if self.memory_budget_passed {
                "within budget"
            } else {
                "OVER BUDGET"
            }
        )
        .expect("writing to String cannot fail");
        output.push_str("## Summary\n\n");
        output.push_str("| Metric | Value |\n|---|---:|\n");
        writeln!(
            &mut output,
            "| Scan rate median | {:.0} entries/s |",
            self.summary.median_scan_entries_per_second
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "| Scan rate MAD | {:.0} entries/s |",
            self.summary.mad_scan_entries_per_second
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "| Syscall-sensitive scan median | {:.6} s |",
            self.summary.median_syscall_phase_seconds
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "| Destination index median | {:.6} s |",
            self.summary.median_destination_index_seconds
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "| Planner median | {:.6} s |",
            self.summary.median_planner_seconds
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut output,
            "| Peak RSS | {} bytes |\n| Queue high-water | {} / {} |\n",
            self.summary.peak_rss_bytes, self.summary.queue_high_water, self.channel_capacity
        )
        .expect("writing to String cannot fail");

        output.push_str("## Repetitions\n\n");
        output.push_str("| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |\n|---:|---:|---:|---:|---:|---:|---:|---:|\n");
        for sample in &self.samples {
            writeln!(
                &mut output,
                "| {} | {} | {:.0} | {:.6} | {:.6} | {:.6} | {} | {} |",
                sample.repetition,
                sample.item_count,
                sample.scan_entries_per_second,
                sample.syscall_phase_seconds,
                sample.destination_index_seconds,
                sample.planner_seconds,
                sample.peak_rss_bytes,
                sample.queue_high_water
            )
            .expect("writing to String cannot fail");
        }
        if !self.notes.is_empty() {
            output.push_str("\n## Qualifications\n\n");
            for note in &self.notes {
                writeln!(&mut output, "- {note}").expect("writing to String cannot fail");
            }
        }
        output
    }
}

/// Report validation error.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// Too few repetitions.
    #[error("scanner/planner evidence requires at least {MINIMUM_REPETITIONS} repetitions")]
    TooFewRepetitions,
    /// Sample counts disagree.
    #[error("repetition {repetition} has inconsistent scan/plan item counts")]
    CountMismatch {
        /// Invalid repetition.
        repetition: u32,
    },
    /// Timing or rate is non-finite or non-positive.
    #[error("repetition {repetition} has invalid timing or scan rate")]
    InvalidTiming {
        /// Invalid repetition.
        repetition: u32,
    },
    /// Observed queue occupancy exceeded its configured bound.
    #[error("repetition {repetition} queue high-water {observed} exceeds capacity {capacity}")]
    QueueOverflow {
        /// Invalid repetition.
        repetition: u32,
        /// Observed occupancy.
        observed: u64,
        /// Configured capacity.
        capacity: u64,
    },
}

fn validate_samples(samples: &[EngineSample], capacity: u64) -> Result<(), ReportError> {
    if samples.len() < MINIMUM_REPETITIONS {
        return Err(ReportError::TooFewRepetitions);
    }
    for sample in samples {
        if sample.item_count != sample.planned_items {
            return Err(ReportError::CountMismatch {
                repetition: sample.repetition,
            });
        }
        let positive = [
            sample.destination_scan_seconds,
            sample.source_scan_seconds,
            sample.syscall_phase_seconds,
            sample.scan_entries_per_second,
            sample.destination_index_seconds,
            sample.planner_seconds,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value > 0.0);
        if !positive {
            return Err(ReportError::InvalidTiming {
                repetition: sample.repetition,
            });
        }
        if sample.queue_high_water > capacity {
            return Err(ReportError::QueueOverflow {
                repetition: sample.repetition,
                observed: sample.queue_high_water,
                capacity,
            });
        }
    }
    Ok(())
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

    fn sample(repetition: u32, rate: f64) -> EngineSample {
        EngineSample {
            repetition,
            item_count: 100,
            destination_scan_seconds: 1.0,
            source_scan_seconds: 1.0,
            syscall_phase_seconds: 2.0,
            scan_entries_per_second: rate,
            destination_index_seconds: 0.2,
            planner_seconds: 0.1,
            queue_high_water: 8,
            peak_rss_bytes: 1000 + u64::from(repetition),
            planned_items: 100,
        }
    }

    fn environment() -> Environment {
        Environment {
            source_revision: "revision".to_owned(),
            build_id: "release".to_owned(),
            host: "host".to_owned(),
            hardware: "hardware".to_owned(),
            os: "os".to_owned(),
            kernel: "kernel".to_owned(),
            filesystem: "filesystem".to_owned(),
        }
    }

    #[test]
    fn derives_summary_and_retains_samples() {
        let report = EngineReport::from_samples(
            environment(),
            "flat-small".to_owned(),
            "a".repeat(64),
            16,
            2_000,
            (0..5)
                .map(|repetition| sample(repetition, 98.0 + f64::from(repetition)))
                .collect(),
            vec!["qualification".to_owned()],
        )
        .unwrap();
        assert!((report.summary.median_scan_entries_per_second - 100.0).abs() < f64::EPSILON);
        assert!((report.summary.mad_scan_entries_per_second - 1.0).abs() < f64::EPSILON);
        assert_eq!(report.summary.peak_rss_bytes, 1004);
        assert!(report.memory_budget_passed);
        assert!(report.to_markdown().contains("## Repetitions"));
    }

    #[test]
    fn rejects_too_few_mismatched_and_overflowing_samples() {
        assert!(matches!(
            EngineReport::from_samples(
                environment(),
                "flat".to_owned(),
                "a".repeat(64),
                16,
                2_000,
                vec![sample(0, 100.0)],
                Vec::new(),
            ),
            Err(ReportError::TooFewRepetitions)
        ));
        let mut samples = (0..5)
            .map(|repetition| sample(repetition, 100.0))
            .collect::<Vec<_>>();
        samples[0].queue_high_water = 17;
        assert!(matches!(
            EngineReport::from_samples(
                environment(),
                "flat".to_owned(),
                "a".repeat(64),
                16,
                2_000,
                samples,
                Vec::new(),
            ),
            Err(ReportError::QueueOverflow { .. })
        ));
    }
}
