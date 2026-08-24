#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

//! Multi-stream striping crossover (Story 4.3 gate, closed against the real
//! Story 4.2 implementation).
//!
//! Measures end-to-end push wall time for a workload at `--streams` 1 (the
//! single-session path) versus `--streams 4` (the multi-stream orchestrator),
//! sweeping large-file sizes plus a many-small-file corpus, over the same local
//! pipe children the rest of the suite uses. Real ssh adds a per-session RTT the
//! pipe path cannot reproduce, so this is the *optimistic* crossover: if multi
//! does not pay in-pipe, it will not pay over ssh.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    repetitions: usize,
    stream_counts: &'static [usize; 2],
    large_file_sizes_mib: &'static [u64; 3],
    many_small_ratio: f64,
    results: Vec<CellResult>,
}

#[derive(Debug, Serialize, Clone)]
struct CellResult {
    label: &'static str,
    logical_bytes: u64,
    streams: usize,
    median_ms: f64,
    mad_ms: f64,
}

#[derive(Debug, thiserror::Error)]
enum BenchError {
    #[error("repetitions must be at least one")]
    ZeroRepetitions,
    #[error("cannot create scratch '{}': {source}", path.display())]
    Scratch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("transfer failed: {0}")]
    Transfer(xsync_core::server::ServerError),
    #[error("cannot write report '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot serialize report: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Parser)]
#[command(
    name = "xsync-stripe-bench",
    about = "Measure where multi-stream striping pays off relative to single-stream"
)]
struct Cli {
    #[arg(long, default_value_t = 5)]
    repetitions: usize,
    #[arg(
        long,
        default_value = "benches/results/story-4.3/stripe-crossover.json"
    )]
    json: PathBuf,
    #[arg(long, default_value = "benches/results/story-4.3/stripe-crossover.md")]
    markdown: PathBuf,
    /// Hidden: run as an `xsync --server` child so the bench can host its own
    /// sessions without a dependency on an installed xsync.
    #[arg(long, hide = true, value_name = "ROOT")]
    server: Option<PathBuf>,
}

const STREAM_COUNTS: [usize; 2] = [1, 4];
const LARGE_SIZES_MIB: [u64; 3] = [4, 16, 64];

fn main() -> Result<(), BenchError> {
    let cli = Cli::parse();
    if let Some(root) = &cli.server {
        xsync_core::server::run_server_stdio(root.clone()).map_err(BenchError::Transfer)?;
        return Ok(());
    }
    if cli.repetitions == 0 {
        return Err(BenchError::ZeroRepetitions);
    }

    let scratch = std::env::temp_dir().join(format!("xsync-stripe-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(|source| BenchError::Scratch {
        path: scratch.clone(),
        source,
    })?;

    let mut results = Vec::new();
    // Huge-file sweep at several sizes.
    for &mib in &LARGE_SIZES_MIB {
        let corpus = scratch.join(format!("large{mib}M"));
        fs::create_dir_all(&corpus).map_err(|source| BenchError::Scratch {
            path: corpus.clone(),
            source,
        })?;
        fs::write(
            corpus.join("big.bin"),
            vec![0x5A; (mib as usize) * 1024 * 1024],
        )
        .map_err(|source| BenchError::Scratch {
            path: corpus.clone(),
            source,
        })?;
        for &streams in &STREAM_COUNTS {
            results.push(measure(
                &scratch,
                format!("large-{mib}M").leak(),
                streams,
                &corpus,
                cli.repetitions,
            )?);
        }
    }
    // Many-small-files corpus.
    let many = scratch.join("many-small");
    fs::create_dir_all(&many).map_err(|source| BenchError::Scratch {
        path: many.clone(),
        source,
    })?;
    for i in 0u32..400 {
        fs::write(
            many.join(format!("f{i:03}.bin")),
            vec![0x33 + (i % 64) as u8; 4096],
        )
        .map_err(|source| BenchError::Scratch {
            path: many.clone(),
            source,
        })?;
    }
    for &streams in &STREAM_COUNTS {
        results.push(measure(
            &scratch,
            "many-small".to_owned().leak(),
            streams,
            &many,
            cli.repetitions,
        )?);
    }

    let report = Report {
        schema: "xsync.stripe-bench.v1",
        repetitions: cli.repetitions,
        stream_counts: &STREAM_COUNTS,
        large_file_sizes_mib: &LARGE_SIZES_MIB,
        many_small_ratio: ratio_of(&results, "many-small"),
        results,
    };
    write_report(&cli.json, &serde_json::to_vec_pretty(&report)?)?;
    write_report(&cli.markdown, report_markdown(&report).as_bytes())?;

    let _ = fs::remove_dir_all(&scratch);
    println!(
        "stripe crossover written to {} and {}",
        cli.json.display(),
        cli.markdown.display()
    );
    Ok(())
}

/// Measure push wall time for `streams` over the real single/multi push paths.
fn measure(
    scratch: &Path,
    label: &'static str,
    streams: usize,
    source: &Path,
    repetitions: usize,
) -> Result<CellResult, BenchError> {
    let mut timings = Vec::with_capacity(repetitions);
    for rep in 0..repetitions {
        let dest = scratch.join(format!("dest-{}-{streams}-{rep}", label.trim()));
        let options = xsync_core::local::LocalSyncOptions {
            streams,
            local_workers: 1,
            ..Default::default()
        };
        let start = Instant::now();
        // host=None -> in-process/local child servers; sync_push_server routes
        // to the single-session path at streams 1 and the orchestrator above it.
        xsync_core::server::sync_push_server(
            source,
            true,
            &dest.to_string_lossy(),
            true,
            &options,
            None,
            None,
            |_| {},
        )
        .map_err(BenchError::Transfer)?;
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = timings.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let devs: Vec<f64> = timings.iter().map(|t| (t - median).abs()).collect();
    let mut ds = devs.clone();
    ds.sort_by(f64::total_cmp);
    let logical_bytes = total_dir_bytes(source).unwrap_or(0);
    Ok(CellResult {
        label,
        logical_bytes,
        streams,
        median_ms: median,
        mad_ms: ds[ds.len() / 2],
    })
}

fn ratio_of(results: &[CellResult], label: &str) -> f64 {
    let single = results
        .iter()
        .find(|c| c.label == label && c.streams == 1)
        .map_or(1.0, |c| c.median_ms);
    let multi = results
        .iter()
        .find(|c| c.label == label && c.streams == 4)
        .map_or(1.0, |c| c.median_ms);
    if multi == 0.0 {
        0.0
    } else {
        single / multi
    }
}

fn total_dir_bytes(dir: &Path) -> Option<u64> {
    let mut total = 0u64;
    for e in fs::read_dir(dir).ok()?.flatten() {
        total = total.saturating_add(e.metadata().ok()?.len());
    }
    Some(total)
}

fn report_markdown(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# Story 4.3 — multi-stream striping crossover\n");
    let _ = writeln!(out, "- schema: `{}`", report.schema);
    let _ = writeln!(out, "- repetitions: {}", report.repetitions);
    let _ = writeln!(
        out,
        "- many-small speedup at 4 streams: {:.2}x",
        report.many_small_ratio
    );
    let _ = writeln!(
        out,
        "- pipe-child setup (optimistic lower bound; real ssh adds per-session RTT)\n"
    );
    let _ = writeln!(
        out,
        "| corpus | logical bytes | streams | median (ms) | MAD (ms) | 4x/1x speedup |"
    );
    let _ = writeln!(out, "|---:|---:|---:|---:|---:|---:|");
    let mut by_label: Vec<(&str, Vec<&CellResult>)> = Vec::new();
    for cell in &report.results {
        if let Some(entry) = by_label.iter_mut().find(|(l, _)| *l == cell.label) {
            entry.1.push(cell);
        } else {
            by_label.push((cell.label, vec![cell]));
        }
    }
    for (label, cells) in &by_label {
        let single = cells
            .iter()
            .find(|c| c.streams == 1)
            .map_or(0.0, |c| c.median_ms);
        let multi = cells
            .iter()
            .find(|c| c.streams == 4)
            .map_or(0.0, |c| c.median_ms);
        let speedup = if multi == 0.0 { 0.0 } else { single / multi };
        let bytes = cells.first().map_or(0, |c| c.logical_bytes);
        for cell in cells {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {:.2} | {:.2} | {:.2}x |",
                label,
                bytes,
                cell.streams,
                cell.median_ms,
                cell.mad_ms,
                if cell.streams == 4 { speedup } else { 1.0 }
            );
        }
    }
    out
}

fn write_report(path: &Path, bytes: &[u8]) -> Result<(), BenchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BenchError::Scratch {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| BenchError::Write {
        path: path.to_path_buf(),
        source,
    })
}
