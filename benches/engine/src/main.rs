use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use serde::Serialize;
use xsync_bench::manifest::build_manifest;
use xsync_core::planner::{
    try_plan_spooled, DestinationIndex, EntryPlan, IndexConfig, Plan, PlanningSpool,
};
use xsync_core::scanner::scan_with_capacity;
use xsync_engine_bench::environment::environment;
use xsync_engine_bench::report::{EngineReport, EngineSample, MINIMUM_REPETITIONS};

const DEFAULT_MEMORY_BUDGET_MIB: u64 = 512;

#[derive(Debug, Parser)]
#[command(
    name = "xsync-engine-bench",
    about = "Isolated xsync scanner, planner, queue, and peak-RSS measurements"
)]
struct Cli {
    #[command(subcommand)]
    command: BenchCommand,
}

#[derive(Debug, Subcommand)]
enum BenchCommand {
    /// Run isolated repetitions and emit versioned JSON plus Markdown.
    Run {
        /// Content-pinned corpus root to scan as both source and destination.
        #[arg(long)]
        root: PathBuf,
        /// Corpus topology label recorded in the report.
        #[arg(long)]
        shape: String,
        /// Isolated process repetitions.
        #[arg(long, default_value_t = 5)]
        repetitions: u32,
        /// Scanner result-channel capacity.
        #[arg(long, default_value_t = 1_024)]
        channel_capacity: usize,
        /// Explicit peak-RSS budget in MiB.
        #[arg(long, default_value_t = DEFAULT_MEMORY_BUDGET_MIB)]
        memory_budget_mib: u64,
        /// Versioned JSON output.
        #[arg(long)]
        json: PathBuf,
        /// Human-readable Markdown output.
        #[arg(long)]
        markdown: PathBuf,
        /// Additional platform or methodology qualification; repeatable.
        #[arg(long)]
        note: Vec<String>,
    },
    /// Internal isolated worker.
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        channel_capacity: usize,
        #[arg(long, default_value_t = DEFAULT_MEMORY_BUDGET_MIB)]
        memory_budget_mib: u64,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xsync-engine-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), BenchError> {
    match cli.command {
        BenchCommand::Run {
            root,
            shape,
            repetitions,
            channel_capacity,
            memory_budget_mib,
            json,
            markdown,
            note,
        } => run_report(RunOptions {
            root,
            shape,
            repetitions,
            channel_capacity,
            memory_budget_mib,
            json,
            markdown,
            notes: note,
        }),
        BenchCommand::Worker {
            root,
            channel_capacity,
            memory_budget_mib,
        } => {
            let memory_budget_bytes = memory_budget_mib
                .checked_mul(1024 * 1024)
                .ok_or(BenchError::MemoryBudgetOverflow)?;
            let sample = worker_sample(&root, channel_capacity, memory_budget_bytes)?;
            serde_json::to_writer(std::io::stdout().lock(), &sample)?;
            Ok(())
        }
    }
}

struct RunOptions {
    root: PathBuf,
    shape: String,
    repetitions: u32,
    channel_capacity: usize,
    memory_budget_mib: u64,
    json: PathBuf,
    markdown: PathBuf,
    notes: Vec<String>,
}

fn run_report(options: RunOptions) -> Result<(), BenchError> {
    if usize::try_from(options.repetitions).unwrap_or(0) < MINIMUM_REPETITIONS {
        return Err(BenchError::TooFewRepetitions);
    }
    if options.channel_capacity == 0 {
        return Err(BenchError::ZeroCapacity);
    }
    let root = fs::canonicalize(&options.root).map_err(|source| BenchError::Io {
        operation: "canonicalize corpus",
        path: options.root.clone(),
        source,
    })?;
    let manifest = build_manifest(&root)?;
    let memory_budget_bytes = options
        .memory_budget_mib
        .checked_mul(1024 * 1024)
        .ok_or(BenchError::MemoryBudgetOverflow)?;
    let mut samples = Vec::with_capacity(options.repetitions as usize);
    for repetition in 0..options.repetitions {
        let mut sample =
            run_isolated_worker(&root, options.channel_capacity, options.memory_budget_mib)?;
        sample.repetition = repetition;
        samples.push(sample);
    }
    let mut notes = options.notes;
    notes.push(
        "Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l)."
            .to_owned(),
    );
    notes.push(
        "The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze."
            .to_owned(),
    );
    let report = EngineReport::from_samples(
        environment(&root, "xsync-engine-bench"),
        options.shape,
        manifest.manifest_digest,
        options.channel_capacity as u64,
        memory_budget_bytes,
        samples,
        notes,
    )?;
    write_json(&options.json, &report)?;
    atomic_write(&options.markdown, report.to_markdown().as_bytes())?;
    println!(
        "scan {:.0} entries/s, plan {:.6}s, peak RSS {} bytes, queue {}/{}; memory budget {}",
        report.summary.median_scan_entries_per_second,
        report.summary.median_planner_seconds,
        report.summary.peak_rss_bytes,
        report.summary.queue_high_water,
        report.channel_capacity,
        if report.memory_budget_passed {
            "passed"
        } else {
            "EXCEEDED"
        }
    );
    Ok(())
}

fn worker_sample(
    root: &Path,
    channel_capacity: usize,
    memory_budget_bytes: u64,
) -> Result<EngineSample, BenchError> {
    let memory_budget_bytes =
        usize::try_from(memory_budget_bytes).map_err(|_| BenchError::MemoryBudgetOverflow)?;
    let config = IndexConfig::with_budget(memory_budget_bytes);
    let mut destination_index = DestinationIndex::with_config(config.clone())?;
    let destination_start = Instant::now();
    let (destination_queue_high_water, destination_index_seconds) =
        scan_into_index(root, channel_capacity, &mut destination_index)?;
    let destination_scan_seconds = destination_start.elapsed().as_secs_f64();
    let item_count = destination_index.len();

    let mut source_spool = PlanningSpool::with_config(config)?;
    let source_start = Instant::now();
    let source_queue_high_water = scan_into_spool(root, channel_capacity, &mut source_spool)?;
    let source_scan_seconds = source_start.elapsed().as_secs_f64();

    let planner_start = Instant::now();
    let plan = try_plan_spooled(source_spool, destination_index)?;
    let planner_seconds = planner_start.elapsed().as_secs_f64();
    let planned_items = plan_item_count(&plan);
    let syscall_phase_seconds = destination_scan_seconds + source_scan_seconds;
    let numeric_item_count =
        f64::from(u32::try_from(item_count).map_err(|_| BenchError::TooManyEntries)?);
    let scan_entries_per_second = (numeric_item_count * 2.0) / syscall_phase_seconds;
    Ok(EngineSample {
        repetition: 0,
        item_count,
        destination_scan_seconds,
        source_scan_seconds,
        syscall_phase_seconds,
        scan_entries_per_second,
        destination_index_seconds,
        planner_seconds,
        queue_high_water: destination_queue_high_water.max(source_queue_high_water) as u64,
        peak_rss_bytes: process_peak_rss_bytes(),
        planned_items,
    })
}

fn scan_into_index(
    root: &Path,
    channel_capacity: usize,
    destination: &mut DestinationIndex,
) -> Result<(usize, f64), BenchError> {
    let scan = scan_with_capacity(root, channel_capacity)?;
    let index_start = Instant::now();
    let consume = scan
        .entries()
        .iter()
        .try_for_each(|result| destination.insert(result?).map_err(BenchError::from));
    let queue_high_water = scan.queue_high_water_mark();
    let finish = scan.finish();
    consume?;
    finish?;
    Ok((queue_high_water, index_start.elapsed().as_secs_f64()))
}

fn scan_into_spool(
    root: &Path,
    channel_capacity: usize,
    source: &mut PlanningSpool,
) -> Result<usize, BenchError> {
    let scan = scan_with_capacity(root, channel_capacity)?;
    let consume = scan
        .entries()
        .iter()
        .try_for_each(|result| source.push(result?).map_err(BenchError::from));
    let queue_high_water = scan.queue_high_water_mark();
    let finish = scan.finish();
    consume?;
    finish?;
    Ok(queue_high_water)
}

fn plan_item_count(plan: &Plan) -> u64 {
    [&plan.files, &plan.directories, &plan.symlinks, &plan.other]
        .into_iter()
        .map(entry_plan_count)
        .sum()
}

fn entry_plan_count(plan: &EntryPlan) -> u64 {
    [
        plan.new.len(),
        plan.changed.len(),
        plan.unchanged.len(),
        plan.extraneous.len(),
    ]
    .into_iter()
    .map(|count| count as u64)
    .sum()
}

fn run_isolated_worker(
    root: &Path,
    channel_capacity: usize,
    memory_budget_mib: u64,
) -> Result<EngineSample, BenchError> {
    let executable = std::env::current_exe().map_err(|source| BenchError::Io {
        operation: "locate benchmark executable",
        path: PathBuf::from("<current executable>"),
        source,
    })?;
    let output = timed_worker_output(&executable, root, channel_capacity, memory_budget_mib)?;
    if !output.status.success() {
        return Err(BenchError::WorkerFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let sample: EngineSample = serde_json::from_slice(&output.stdout)?;
    #[cfg(target_os = "macos")]
    let sample = EngineSample {
        peak_rss_bytes: parse_macos_peak_rss(&output.stderr)?,
        ..sample
    };
    if sample.peak_rss_bytes == 0 {
        return Err(BenchError::MissingPeakRss);
    }
    Ok(sample)
}

#[cfg(target_os = "macos")]
fn timed_worker_output(
    executable: &Path,
    root: &Path,
    channel_capacity: usize,
    memory_budget_mib: u64,
) -> Result<Output, BenchError> {
    Command::new("/usr/bin/time")
        .arg("-l")
        .arg(executable)
        .arg("worker")
        .arg("--root")
        .arg(root)
        .arg("--channel-capacity")
        .arg(channel_capacity.to_string())
        .arg("--memory-budget-mib")
        .arg(memory_budget_mib.to_string())
        .output()
        .map_err(|source| BenchError::Io {
            operation: "run isolated timed worker",
            path: executable.to_path_buf(),
            source,
        })
}

#[cfg(not(target_os = "macos"))]
fn timed_worker_output(
    executable: &Path,
    root: &Path,
    channel_capacity: usize,
    memory_budget_mib: u64,
) -> Result<Output, BenchError> {
    Command::new(executable)
        .arg("worker")
        .arg("--root")
        .arg(root)
        .arg("--channel-capacity")
        .arg(channel_capacity.to_string())
        .arg("--memory-budget-mib")
        .arg(memory_budget_mib.to_string())
        .output()
        .map_err(|source| BenchError::Io {
            operation: "run isolated worker",
            path: executable.to_path_buf(),
            source,
        })
}

#[cfg(target_os = "macos")]
fn parse_macos_peak_rss(stderr: &[u8]) -> Result<u64, BenchError> {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| line.trim_end().ends_with("maximum resident set size"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .ok_or(BenchError::MissingPeakRss)
}

#[cfg(target_os = "linux")]
fn process_peak_rss_bytes() -> u64 {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|kib| kib.checked_mul(1024))
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
fn process_peak_rss_bytes() -> u64 {
    0
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), BenchError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), BenchError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| BenchError::Io {
        operation: "create report directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BenchError::Clock)?
        .as_nanos();
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("xsync-engine-bench"),
        std::process::id(),
        nonce
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| BenchError::Io {
            operation: "create temporary report",
            path: temporary.clone(),
            source,
        })?;
    file.write_all(bytes).map_err(|source| BenchError::Io {
        operation: "write temporary report",
        path: temporary.clone(),
        source,
    })?;
    file.sync_all().map_err(|source| BenchError::Io {
        operation: "sync temporary report",
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| BenchError::Io {
        operation: "publish report",
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, thiserror::Error)]
enum BenchError {
    #[error("scanner/planner evidence requires at least {MINIMUM_REPETITIONS} repetitions")]
    TooFewRepetitions,
    #[error("channel capacity must be at least one")]
    ZeroCapacity,
    #[error("memory budget is too large")]
    MemoryBudgetOverflow,
    #[error("scanner returned more entries than the benchmark schema supports")]
    TooManyEntries,
    #[error("isolated worker did not report peak RSS")]
    MissingPeakRss,
    #[error("isolated worker failed: {0}")]
    WorkerFailed(String),
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("system clock is before Unix epoch")]
    Clock,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Manifest(#[from] xsync_bench::manifest::ManifestError),
    #[error(transparent)]
    Scan(#[from] xsync_core::scanner::ScanError),
    #[error(transparent)]
    Planner(#[from] xsync_core::planner::PlannerError),
    #[error(transparent)]
    Report(#[from] xsync_engine_bench::report::ReportError),
}
