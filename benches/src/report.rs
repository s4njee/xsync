//! Versioned benchmark report schema, validation, summaries, and Markdown.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manifest::Verification;

/// Raw input schema accepted by the report builder.
pub const REPORT_INPUT_SCHEMA: &str = "xsync.bench.input.v1";
/// Stable report schema emitted by the harness.
pub const REPORT_SCHEMA: &str = "xsync.bench.report.v1";

/// Source/build identity for a benchmark run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub source_revision: String,
    pub build_id: String,
    pub profile: String,
}

/// Machine and route identity relevant to comparability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub hardware: String,
    pub os: String,
    pub kernel: String,
    pub filesystem: String,
    pub transport: String,
    pub route: String,
    #[serde(default)]
    pub shaping: String,
}

/// Transfer/session choices which affect results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub streams: u16,
    pub compression: String,
}

/// Content-pinned corpus identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusIdentity {
    pub schema: String,
    pub manifest_digest: String,
    pub description: String,
}

/// Tool identity recorded alongside the benchmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    pub name: String,
    pub version: String,
    pub command: String,
}

/// Truthful page-cache state label for a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    FirstPass,
    Warm,
    ColdEvicted,
}

/// One raw benchmark repetition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub repetition: u32,
    pub method_order: u32,
    pub wall_seconds: f64,
    pub cpu_seconds: f64,
    pub peak_rss_bytes: u64,
    pub item_count: u64,
    pub logical_bytes: u64,
    /// Logical source allocation basis for this sample.
    #[serde(default)]
    pub source_allocated_bytes: u64,
    /// Physical destination allocation measured by the independent oracle.
    #[serde(default)]
    pub destination_allocated_bytes: u64,
    pub wire_bytes: u64,
    pub phases_seconds: BTreeMap<String, f64>,
    /// Time spent preparing the destination, outside the transfer phase budget.
    #[serde(default)]
    pub seed_destination_seconds: f64,
    /// Time spent in the independent destination oracle, outside transfer timing.
    #[serde(default)]
    pub verify_oracle_seconds: f64,
    pub cache_state: CacheState,
    pub cache_eviction_method: Option<String>,
    pub oracle: Verification,
    /// Source manifest digest observed for this repetition, when using a real corpus.
    #[serde(default)]
    pub source_manifest_digest: Option<String>,
    /// Deterministic destination entries changed for a real-corpus mutation.
    #[serde(default)]
    pub mutation_selection: Vec<String>,
}

/// Raw measurements for one method/result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultInput {
    pub name: String,
    pub baseline: Option<String>,
    pub samples: Vec<Sample>,
}

/// Complete raw input consumed by the report builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportInput {
    pub schema: String,
    pub build: BuildIdentity,
    pub environment: Environment,
    pub session: SessionConfig,
    pub corpus: CorpusIdentity,
    pub tools: Vec<ToolIdentity>,
    pub results: Vec<ResultInput>,
}

/// One same-repetition paired speedup observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedObservation {
    pub repetition: u32,
    pub baseline_seconds: f64,
    pub candidate_seconds: f64,
    pub speedup: f64,
}

/// Derived summary plus retained raw samples for one result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub baseline: Option<String>,
    pub samples: Vec<Sample>,
    pub median_wall_seconds: f64,
    pub mad_wall_seconds: f64,
    pub median_cpu_seconds: f64,
    pub peak_rss_bytes: u64,
    pub item_count: u64,
    pub logical_bytes: u64,
    pub median_source_allocated_bytes: u64,
    pub median_destination_allocated_bytes: u64,
    pub allocated_throughput_bytes_per_second: f64,
    pub median_wire_bytes: u64,
    pub phase_medians_seconds: BTreeMap<String, f64>,
    pub paired_observations: Vec<PairedObservation>,
    pub paired_ratio_median: Option<f64>,
    pub paired_ratio_mad: Option<f64>,
}

/// Versioned benchmark report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub generated_unix_nanos: u128,
    pub build: BuildIdentity,
    pub environment: Environment,
    pub session: SessionConfig,
    pub corpus: CorpusIdentity,
    pub tools: Vec<ToolIdentity>,
    pub results: Vec<BenchResult>,
}

/// Report validation or construction failures.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ReportError {
    #[error("unsupported report input schema '{0}'")]
    UnsupportedSchema(String),
    #[error("required report field is empty: {0}")]
    EmptyField(&'static str),
    #[error("report requires at least one {0}")]
    EmptyCollection(&'static str),
    #[error("duplicate result name '{0}'")]
    DuplicateResult(String),
    #[error("{result} repetition {repetition}: {reason}")]
    InvalidSample {
        result: String,
        repetition: u32,
        reason: String,
    },
    #[error("result '{result}' references missing baseline '{baseline}'")]
    MissingBaseline { result: String, baseline: String },
    #[error("result '{result}' and baseline '{baseline}' have different repetition sets")]
    UnpairedRepetitions { result: String, baseline: String },
    #[error("result '{result}' was always measured on the same side of baseline '{baseline}'")]
    UnbalancedOrder { result: String, baseline: String },
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
    #[error("corpus manifest digest must be 64 lowercase hexadecimal characters")]
    InvalidManifestDigest,
    #[error("schedule requires at least two unique methods and one repetition")]
    InvalidSchedule,
}

/// Produce a deterministic rotated method order for every repetition.
///
/// # Errors
///
/// Returns an error for fewer than two unique methods or zero repetitions.
pub fn rotated_schedule(
    methods: &[String],
    repetitions: u32,
) -> Result<Vec<Vec<String>>, ReportError> {
    let unique = methods.iter().collect::<BTreeSet<_>>();
    if methods.len() < 2 || unique.len() != methods.len() || repetitions == 0 {
        return Err(ReportError::InvalidSchedule);
    }
    Ok((0..repetitions)
        .map(|repetition| {
            let offset = usize::try_from(repetition).unwrap_or(0) % methods.len();
            methods[offset..]
                .iter()
                .chain(methods[..offset].iter())
                .cloned()
                .collect()
        })
        .collect())
}

impl Report {
    /// Validate raw input and compute derived result fields.
    ///
    /// # Errors
    ///
    /// Returns an error for incomplete identity, malformed samples, missing
    /// baselines, unpaired repetitions, or fixed method ordering.
    pub fn from_input(input: ReportInput) -> Result<Self, ReportError> {
        validate_input(&input)?;
        let inputs: BTreeMap<_, _> = input
            .results
            .iter()
            .map(|result| (result.name.as_str(), result))
            .collect();
        let mut results = Vec::with_capacity(input.results.len());
        for result in &input.results {
            let samples = result
                .samples
                .iter()
                .map(sample_with_unaccounted_phase)
                .collect::<Vec<_>>();
            let paired_observations = result
                .baseline
                .as_ref()
                .map_or_else(Vec::new, |name| pair(result, inputs[name.as_str()]));
            let ratios = paired_observations
                .iter()
                .map(|observation| observation.speedup)
                .collect::<Vec<_>>();
            let walls = result
                .samples
                .iter()
                .map(|sample| sample.wall_seconds)
                .collect::<Vec<_>>();
            let cpu = result
                .samples
                .iter()
                .map(|sample| sample.cpu_seconds)
                .collect::<Vec<_>>();
            let wire = result
                .samples
                .iter()
                .map(|sample| sample.wire_bytes)
                .collect::<Vec<_>>();
            let source_allocated = result
                .samples
                .iter()
                .map(|sample| sample.source_allocated_bytes)
                .collect::<Vec<_>>();
            let destination_allocated = result
                .samples
                .iter()
                .map(|sample| sample.destination_allocated_bytes)
                .collect::<Vec<_>>();
            let wall_median = median_f64(&walls);
            let ratio_median = (!ratios.is_empty()).then(|| median_f64(&ratios));
            let phase_medians_seconds = phase_medians(&samples);
            results.push(BenchResult {
                name: result.name.clone(),
                baseline: result.baseline.clone(),
                samples,
                median_wall_seconds: wall_median,
                mad_wall_seconds: mad_f64(&walls, wall_median),
                median_cpu_seconds: median_f64(&cpu),
                peak_rss_bytes: result
                    .samples
                    .iter()
                    .map(|sample| sample.peak_rss_bytes)
                    .max()
                    .unwrap_or(0),
                item_count: result.samples[0].item_count,
                logical_bytes: result.samples[0].logical_bytes,
                median_source_allocated_bytes: median_u64(&source_allocated),
                median_destination_allocated_bytes: median_u64(&destination_allocated),
                allocated_throughput_bytes_per_second: allocated_throughput(
                    median_u64(&source_allocated),
                    wall_median,
                ),
                median_wire_bytes: median_u64(&wire),
                phase_medians_seconds,
                paired_ratio_median: ratio_median,
                paired_ratio_mad: ratio_median.map(|median| mad_f64(&ratios, median)),
                paired_observations,
            });
        }
        Ok(Self {
            schema: REPORT_SCHEMA.to_owned(),
            generated_unix_nanos: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ReportError::InvalidClock)?
                .as_nanos(),
            build: input.build,
            environment: input.environment,
            session: input.session,
            corpus: input.corpus,
            tools: input.tools,
            results,
        })
    }

    /// Render a compact but auditable Markdown report.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# xsync Benchmark Report\n\n");
        write!(
            &mut out,
            "- Schema: `{}`\n- Generated (Unix ns): `{}`\n- Source revision: `{}`\n- Build: `{}` (`{}`)\n\n",
            md(&self.schema),
            self.generated_unix_nanos,
            md(&self.build.source_revision),
            md(&self.build.build_id),
            md(&self.build.profile)
        )
        .expect("writing to String cannot fail");
        out.push_str("## Environment\n\n");
        write!(
            &mut out,
            "- Hardware: `{}`\n- OS: `{}`\n- Kernel: `{}`\n- Filesystem: `{}`\n- Transport: `{}`\n- Route: `{}`\n- Shaping: `{}`\n- Streams: `{}`\n- Compression: `{}`\n\n",
            md(&self.environment.hardware),
            md(&self.environment.os),
            md(&self.environment.kernel),
            md(&self.environment.filesystem),
            md(&self.environment.transport),
            md(&self.environment.route),
            md(&self.environment.shaping),
            self.session.streams,
            md(&self.session.compression)
        )
        .expect("writing to String cannot fail");
        out.push_str("## Corpus\n\n");
        write!(
            &mut out,
            "- Schema: `{}`\n- Manifest: `{}`\n- Description: {}\n\n",
            md(&self.corpus.schema),
            md(&self.corpus.manifest_digest),
            md(&self.corpus.description)
        )
        .expect("writing to String cannot fail");
        out.push_str("## Tools\n\n");
        for tool in &self.tools {
            writeln!(
                &mut out,
                "- **{}** `{}`: `{}`",
                md(&tool.name),
                md(&tool.version),
                md(&tool.command)
            )
            .expect("writing to String cannot fail");
        }
        out.push_str("\n## Results\n\n");
        out.push_str("| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |\n");
        out.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|\n");
        for result in &self.results {
            let paired = result
                .paired_ratio_median
                .map_or_else(|| "-".to_owned(), |value| format!("{value:.3}x"));
            let oracle = if result.samples.iter().all(|sample| sample.oracle.passed) {
                if result
                    .samples
                    .iter()
                    .any(|sample| sample.oracle.mode == "sampled")
                {
                    "pass (includes sampled)"
                } else {
                    "pass"
                }
            } else {
                "FAIL"
            };
            writeln!(
                &mut out,
                "| {} | {:.6}s | {:.6}s | {:.6}s | {} | {} | {} | {:.3} B/s | {} | {} | {} | {} |",
                md(&result.name),
                result.median_wall_seconds,
                result.mad_wall_seconds,
                result.median_cpu_seconds,
                result.peak_rss_bytes,
                result.item_count,
                result.logical_bytes,
                result.allocated_throughput_bytes_per_second,
                result.median_wire_bytes,
                paired,
                result.samples.len(),
                oracle
            )
            .expect("writing to String cannot fail");
        }
        out.push_str("\n## Repetitions\n\n");
        out.push_str("| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |\n");
        out.push_str("|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|\n");
        for result in &self.results {
            for sample in &result.samples {
                writeln!(
                    &mut out,
                    "| {} | {} | {} | {} | {:.6}s | {:.6}s | {} | {} | {} | {} | {} | {} | {} |",
                    md(&result.name),
                    sample.repetition,
                    sample.method_order,
                    cache_label(sample),
                    sample.wall_seconds,
                    sample.cpu_seconds,
                    sample.peak_rss_bytes,
                    sample.item_count,
                    sample.logical_bytes,
                    sample.source_allocated_bytes,
                    sample.destination_allocated_bytes,
                    sample.wire_bytes,
                    if sample.oracle.passed {
                        if sample.oracle.mode == "sampled" {
                            "pass (sampled)"
                        } else {
                            "pass"
                        }
                    } else {
                        "FAIL"
                    }
                )
                .expect("writing to String cannot fail");
            }
        }
        out
    }
}

fn validate_input(input: &ReportInput) -> Result<(), ReportError> {
    if input.schema != REPORT_INPUT_SCHEMA {
        return Err(ReportError::UnsupportedSchema(input.schema.clone()));
    }
    for (name, value) in [
        (
            "build.source_revision",
            input.build.source_revision.as_str(),
        ),
        ("build.build_id", input.build.build_id.as_str()),
        ("build.profile", input.build.profile.as_str()),
        ("environment.hardware", input.environment.hardware.as_str()),
        ("environment.os", input.environment.os.as_str()),
        ("environment.kernel", input.environment.kernel.as_str()),
        (
            "environment.filesystem",
            input.environment.filesystem.as_str(),
        ),
        (
            "environment.transport",
            input.environment.transport.as_str(),
        ),
        ("environment.route", input.environment.route.as_str()),
        ("environment.shaping", input.environment.shaping.as_str()),
        ("session.compression", input.session.compression.as_str()),
        ("corpus.schema", input.corpus.schema.as_str()),
        (
            "corpus.manifest_digest",
            input.corpus.manifest_digest.as_str(),
        ),
        ("corpus.description", input.corpus.description.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ReportError::EmptyField(name));
        }
    }
    if input.session.streams == 0 {
        return Err(ReportError::EmptyField("session.streams"));
    }
    if input.corpus.manifest_digest.len() != 64
        || !input
            .corpus
            .manifest_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ReportError::InvalidManifestDigest);
    }
    if input.tools.is_empty() {
        return Err(ReportError::EmptyCollection("tool"));
    }
    if input.results.is_empty() {
        return Err(ReportError::EmptyCollection("result"));
    }
    for tool in &input.tools {
        if tool.name.trim().is_empty()
            || tool.version.trim().is_empty()
            || tool.command.trim().is_empty()
        {
            return Err(ReportError::EmptyField("tool identity"));
        }
    }
    let mut names = BTreeSet::new();
    for result in &input.results {
        if result.name.trim().is_empty() {
            return Err(ReportError::EmptyField("result.name"));
        }
        if !names.insert(result.name.as_str()) {
            return Err(ReportError::DuplicateResult(result.name.clone()));
        }
        if result.samples.is_empty() {
            return Err(ReportError::EmptyCollection("sample"));
        }
        validate_samples(result)?;
    }
    let by_name: BTreeMap<_, _> = input
        .results
        .iter()
        .map(|result| (result.name.as_str(), result))
        .collect();
    for result in &input.results {
        if let Some(name) = &result.baseline {
            let baseline =
                by_name
                    .get(name.as_str())
                    .ok_or_else(|| ReportError::MissingBaseline {
                        result: result.name.clone(),
                        baseline: name.clone(),
                    })?;
            validate_pairing(result, baseline)?;
        }
    }
    Ok(())
}

fn validate_samples(result: &ResultInput) -> Result<(), ReportError> {
    let mut repetitions = BTreeSet::new();
    for sample in &result.samples {
        let invalid = |reason: &str| ReportError::InvalidSample {
            result: result.name.clone(),
            repetition: sample.repetition,
            reason: reason.to_owned(),
        };
        if !repetitions.insert(sample.repetition) {
            return Err(invalid("duplicate repetition"));
        }
        if !sample.wall_seconds.is_finite() || sample.wall_seconds <= 0.0 {
            return Err(invalid("wall_seconds must be finite and positive"));
        }
        if !sample.cpu_seconds.is_finite() || sample.cpu_seconds < 0.0 {
            return Err(invalid("cpu_seconds must be finite and non-negative"));
        }
        if sample.phases_seconds.is_empty()
            || sample
                .phases_seconds
                .iter()
                .any(|(name, value)| name.trim().is_empty() || !value.is_finite() || *value < 0.0)
        {
            return Err(invalid(
                "phase timings must be named, finite, and non-negative",
            ));
        }
        match sample.cache_state {
            CacheState::ColdEvicted
                if sample
                    .cache_eviction_method
                    .as_deref()
                    .is_none_or(|method| method.trim().is_empty()) =>
            {
                return Err(invalid("cold_evicted requires a cache_eviction_method"));
            }
            CacheState::FirstPass | CacheState::Warm if sample.cache_eviction_method.is_some() => {
                return Err(invalid(
                    "cache_eviction_method is only valid for cold_evicted",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_pairing(result: &ResultInput, baseline: &ResultInput) -> Result<(), ReportError> {
    let repetitions = |value: &ResultInput| {
        value
            .samples
            .iter()
            .map(|sample| sample.repetition)
            .collect::<BTreeSet<_>>()
    };
    if repetitions(result) != repetitions(baseline) {
        return Err(ReportError::UnpairedRepetitions {
            result: result.name.clone(),
            baseline: baseline.name.clone(),
        });
    }
    if result.samples.len() > 1 {
        let orders = baseline
            .samples
            .iter()
            .map(|sample| (sample.repetition, sample.method_order))
            .collect::<BTreeMap<_, _>>();
        let before = result
            .samples
            .iter()
            .any(|sample| sample.method_order < orders[&sample.repetition]);
        let after = result
            .samples
            .iter()
            .any(|sample| sample.method_order > orders[&sample.repetition]);
        if !before || !after {
            return Err(ReportError::UnbalancedOrder {
                result: result.name.clone(),
                baseline: baseline.name.clone(),
            });
        }
    }
    Ok(())
}

fn pair(candidate: &ResultInput, baseline: &ResultInput) -> Vec<PairedObservation> {
    let baseline_times = baseline
        .samples
        .iter()
        .map(|sample| (sample.repetition, sample.wall_seconds))
        .collect::<BTreeMap<_, _>>();
    let mut observations = candidate
        .samples
        .iter()
        .map(|sample| PairedObservation {
            repetition: sample.repetition,
            baseline_seconds: baseline_times[&sample.repetition],
            candidate_seconds: sample.wall_seconds,
            speedup: baseline_times[&sample.repetition] / sample.wall_seconds,
        })
        .collect::<Vec<_>>();
    observations.sort_by_key(|observation| observation.repetition);
    observations
}

fn phase_medians(samples: &[Sample]) -> BTreeMap<String, f64> {
    let mut phases = BTreeMap::<String, Vec<f64>>::new();
    for sample in samples {
        for (name, seconds) in &sample.phases_seconds {
            phases.entry(name.clone()).or_default().push(*seconds);
        }
    }
    phases
        .into_iter()
        .map(|(name, values)| (name, median_f64(&values)))
        .collect()
}

fn sample_with_unaccounted_phase(sample: &Sample) -> Sample {
    let mut sample = sample.clone();
    let phase_sum: f64 = sample.phases_seconds.values().sum();
    let discrepancy = sample.wall_seconds - phase_sum;
    if discrepancy.abs() > sample.wall_seconds * 0.05 {
        sample
            .phases_seconds
            .insert("unaccounted".to_owned(), discrepancy.max(0.0));
    }
    sample
}

fn median_f64(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        sorted[middle - 1].midpoint(sorted[middle])
    } else {
        sorted[middle]
    }
}

fn median_u64(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

#[allow(clippy::cast_precision_loss)]
fn allocated_throughput(bytes: u64, seconds: f64) -> f64 {
    bytes as f64 / seconds
}

fn mad_f64(values: &[f64], median: f64) -> f64 {
    median_f64(
        &values
            .iter()
            .map(|value| (value - median).abs())
            .collect::<Vec<_>>(),
    )
}

fn cache_label(sample: &Sample) -> &'static str {
    match sample.cache_state {
        CacheState::FirstPass => "first_pass",
        CacheState::Warm => "warm",
        CacheState::ColdEvicted => "cold_evicted",
    }
}

fn md(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('`', "\\`")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{report_input, verification};

    #[test]
    fn computes_medians_mad_and_same_repetition_ratios() {
        let report = Report::from_input(report_input(1.0)).unwrap();
        let candidate = &report.results[1];
        assert!((candidate.median_wall_seconds - 1.0).abs() < f64::EPSILON);
        assert!(candidate.mad_wall_seconds.abs() < f64::EPSILON);
        assert!((candidate.paired_ratio_median.unwrap() - 2.0).abs() < f64::EPSILON);
        assert_eq!(candidate.paired_observations.len(), 5);
        assert!((candidate.phase_medians_seconds["transfer"] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_false_cold_cache_claim_and_fixed_order() {
        let mut bad_cache = report_input(1.0);
        bad_cache.results[0].samples[0].cache_state = CacheState::ColdEvicted;
        assert!(matches!(
            Report::from_input(bad_cache),
            Err(ReportError::InvalidSample { .. })
        ));

        let mut fixed_order = report_input(1.0);
        for sample in &mut fixed_order.results[0].samples {
            sample.method_order = 0;
        }
        for sample in &mut fixed_order.results[1].samples {
            sample.method_order = 1;
        }
        assert!(matches!(
            Report::from_input(fixed_order),
            Err(ReportError::UnbalancedOrder { .. })
        ));
    }

    #[test]
    fn records_large_phase_timing_gap_as_unaccounted() {
        let mut input = report_input(1.0);
        input.results[0].samples[0]
            .phases_seconds
            .insert("transfer".to_owned(), 0.5);
        let report = Report::from_input(input).unwrap();
        assert!(
            (report.results[0].samples[0].phases_seconds["unaccounted"] - 1.5).abs() < f64::EPSILON
        );
    }

    #[test]
    fn markdown_retains_environment_results_samples_and_oracle() {
        let mut input = report_input(1.0);
        input.results[1].samples[2].oracle = verification(false);
        let markdown = Report::from_input(input).unwrap().to_markdown();
        assert!(markdown.contains("## Environment"));
        assert!(markdown.contains("candidate"));
        assert!(markdown.contains("2.000x"));
        assert!(markdown.contains("FAIL"));
        assert!(markdown.contains("first_pass"));
    }

    #[test]
    fn schedule_rotates_method_order() {
        let methods = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let schedule = rotated_schedule(&methods, 4).unwrap();
        assert_eq!(schedule[0], ["a", "b", "c"]);
        assert_eq!(schedule[1], ["b", "c", "a"]);
        assert_eq!(schedule[2], ["c", "a", "b"]);
        assert_eq!(schedule[3], ["a", "b", "c"]);
    }
}
