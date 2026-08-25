//! Synthetic calibration matrix for logical strategy thresholds.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use clap::Parser;
use serde::Serialize;
use xsync_core::scanner::{EntryKind, FileEntry, SourceFingerprint};
use xsync_core::strategy::{
    logical_queue_bound_bytes, shared_bounded_work_queues_with_config, DispatchError,
    DispatchStats, StrategyConfig,
};

const BATCH_TARGETS_MIB: [u64; 4] = [8, 16, 32, 64];
const CHUNK_SIZES_MIB: [u64; 3] = [4, 8, 16];
const WORKER_COUNTS: [usize; 5] = [1, 2, 4, 8, 16];
const SMALL_FILE_COUNT: usize = 50_000;
const SMALL_FILE_SIZE: u64 = 4 * 1024;
const LARGE_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "xsync-strategy-bench",
    about = "Sweep logical strategy thresholds and shared worker scheduling"
)]
struct Cli {
    /// Repetitions per matrix cell. The acceptance gate requires at least five.
    #[arg(long, default_value_t = 5)]
    repetitions: usize,
    /// JSON report destination.
    #[arg(
        long,
        default_value = "benches/results/story-2.3b/strategy-matrix.json"
    )]
    json: PathBuf,
    /// Markdown report destination.
    #[arg(long, default_value = "benches/results/story-2.3b/strategy-matrix.md")]
    markdown: PathBuf,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    repetitions: usize,
    batch_targets_mib: &'static [u64; 4],
    chunk_sizes_mib: &'static [u64; 3],
    worker_counts: &'static [usize; 5],
    queue_capacity: usize,
    logical_queue_bound_bytes_at_16_streams: u64,
    wire_frame_limit_is_not_a_strategy_input: bool,
    results: Vec<CellResult>,
}

#[derive(Debug, Serialize)]
struct CellResult {
    corpus: &'static str,
    batch_target_mib: u64,
    chunk_size_mib: u64,
    workers: usize,
    median_dispatch_ns: u128,
    mad_dispatch_ns: u128,
    batches: usize,
    batched_files: usize,
    whole_files: usize,
    chunks: usize,
}

#[derive(Debug, thiserror::Error)]
enum BenchError {
    #[error("repetitions must be at least one")]
    ZeroRepetitions,
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error("cannot create report parent '{}': {source}", path.display())]
    CreateParent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write report '{}': {source}", path.display())]
    WriteReport {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot serialize report: {0}")]
    Serialize(#[from] serde_json::Error),
}

fn main() -> Result<(), BenchError> {
    let cli = Cli::parse();
    if cli.repetitions == 0 {
        return Err(BenchError::ZeroRepetitions);
    }

    let mut results = Vec::new();
    for (corpus_name, files) in [
        ("flat-small", small_corpus("flat")),
        ("deep-small", small_corpus("deep")),
        ("large-file", vec![file("large/file", LARGE_FILE_SIZE)]),
    ] {
        for &batch_target_mib in &BATCH_TARGETS_MIB {
            for &chunk_size_mib in &CHUNK_SIZES_MIB {
                for &workers in &WORKER_COUNTS {
                    results.push(benchmark_cell(
                        corpus_name,
                        &files,
                        batch_target_mib,
                        chunk_size_mib,
                        workers,
                        cli.repetitions,
                    )?);
                }
            }
        }
    }

    let report = Report {
        schema: "xsync.strategy-bench.v1",
        repetitions: cli.repetitions,
        batch_targets_mib: &BATCH_TARGETS_MIB,
        chunk_sizes_mib: &CHUNK_SIZES_MIB,
        worker_counts: &WORKER_COUNTS,
        queue_capacity: 2,
        logical_queue_bound_bytes_at_16_streams: logical_queue_bound_bytes(
            2,
            16,
            StrategyConfig::default(),
        )?,
        wire_frame_limit_is_not_a_strategy_input: true,
        results,
    };
    write_report(&cli.json, &serde_json::to_vec_pretty(&report)?)?;
    write_report(&cli.markdown, report_markdown(&report).as_bytes())?;
    println!(
        "{} cells, {} repetitions; reports written to {} and {}",
        report.results.len(),
        report.repetitions,
        cli.json.display(),
        cli.markdown.display()
    );
    Ok(())
}

fn benchmark_cell(
    corpus: &'static str,
    files: &[FileEntry],
    batch_target_mib: u64,
    chunk_size_mib: u64,
    workers: usize,
    repetitions: usize,
) -> Result<CellResult, BenchError> {
    let config = StrategyConfig {
        batch_target_size: batch_target_mib * 1024 * 1024,
        chunk_size: chunk_size_mib * 1024 * 1024,
        ..StrategyConfig::default()
    };
    let mut durations = Vec::with_capacity(repetitions);
    let mut stats = DispatchStats::default();
    for _ in 0..repetitions {
        let (dispatcher, queues) =
            shared_bounded_work_queues_with_config(workers, 2, workers, config)?;
        let duration_and_stats = std::thread::scope(|scope| {
            let local_handles: Vec<_> = queues
                .local
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().count()))
                .collect();
            let stream_handles: Vec<_> = queues
                .streams
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().count()))
                .collect();
            let start = Instant::now();
            let stats = dispatcher.dispatch(files.iter().cloned())?;
            let elapsed = start.elapsed();
            let local_items = local_handles
                .into_iter()
                .map(|handle| handle.join().expect("local benchmark worker panicked"))
                .sum::<usize>();
            let stream_items = stream_handles
                .into_iter()
                .map(|handle| handle.join().expect("stream benchmark worker panicked"))
                .sum::<usize>();
            assert_eq!(
                local_items + stream_items,
                stats.batches + stats.whole_files + stats.chunks
            );
            Ok::<_, DispatchError>((elapsed, stats))
        })?;
        durations.push(duration_and_stats.0);
        stats = duration_and_stats.1;
    }
    durations.sort_unstable();
    let median = durations[durations.len() / 2];
    let mut deviations: Vec<_> = durations
        .iter()
        .map(|duration| duration.abs_diff(median))
        .collect();
    deviations.sort_unstable();
    Ok(CellResult {
        corpus,
        batch_target_mib,
        chunk_size_mib,
        workers,
        median_dispatch_ns: median.as_nanos(),
        mad_dispatch_ns: deviations[deviations.len() / 2].as_nanos(),
        batches: stats.batches,
        batched_files: stats.batched_files,
        whole_files: stats.whole_files,
        chunks: stats.chunks,
    })
}

fn small_corpus(shape: &str) -> Vec<FileEntry> {
    (0..SMALL_FILE_COUNT)
        .map(|index| {
            let path = if shape == "flat" {
                format!("file-{index:05}")
            } else {
                format!("level/{:04}/branch/{:04}/file", index % 1_000, index)
            };
            file(path, SMALL_FILE_SIZE)
        })
        .collect()
}

fn file(path: impl Into<String>, size: u64) -> FileEntry {
    FileEntry {
        path: xsync_core::path::WirePath::from(path.into().as_str()),
        kind: EntryKind::File,
        size,
        mtime: UNIX_EPOCH,
        mode: 0o644,
        fingerprint: SourceFingerprint::synthetic(EntryKind::File, size, UNIX_EPOCH),
    }
}

fn write_report(path: &Path, bytes: &[u8]) -> Result<(), BenchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BenchError::CreateParent {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| BenchError::WriteReport {
        path: path.to_path_buf(),
        source,
    })
}

fn report_markdown(report: &Report) -> String {
    let mut markdown = "# Story 2.3b strategy calibration\n\n"
        .to_owned()
        + "Schema: `xsync.strategy-bench.v1`\n\n"
        + "This matrix measures metadata dispatch only. Logical batches and chunks are independent of wire-frame limits.\n\n"
        + "| Corpus | Batch MiB | Chunk MiB | Workers | Median ns | MAD ns | Batches | Whole | Chunks |\n"
        + "|---|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for result in &report.results {
        writeln!(
            markdown,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            result.corpus,
            result.batch_target_mib,
            result.chunk_size_mib,
            result.workers,
            result.median_dispatch_ns,
            result.mad_dispatch_ns,
            result.batches,
            result.whole_files,
            result.chunks,
        )
        .expect("writing Markdown report to a String cannot fail");
    }
    markdown
}
