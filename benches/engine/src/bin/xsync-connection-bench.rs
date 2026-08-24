#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

//! SSH connection-model benchmark (Story 4.3).
//!
//! Estimates the cost of establishing N parallel `xsync --server` sessions and
//! separates that *connection setup* phase from actual transfer. This is the
//! building-block measurement that gates Story 4.2: if each extra session costs
//! a full connection setup plus a destination scan, striping may not pay off
//! for small and medium jobs, and the crossover point should drive whether the
//! coordination complexity is worth enabling multi-stream at all.
//!
//! The benchmark spawns real `xsync --server` child processes (the same local
//! pipe path the fake-rsh tests use, and the same `{:b}` ssh line a production
//! host would use), hands them a v1 `Handshake` + `SessionConfig`, stops the clock
//! at the `SessionConfig` acknowledgement, and reaps the child. A reference
//! end-to-end transfer of a small corpus is measured separately so `transfer`
//! is reported independently of `setup`.

use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Instant;

use clap::Parser;
use serde::Serialize;
use xsync_core::protocol::{
    encode_frame, CompressionMode, FrameDecoder, Message, Role, DEFAULT_UNACKNOWLEDGED_WINDOW,
    MAX_COMPLETE_PAYLOAD, MAX_DATA_SEGMENT,
};
use xsync_core::server::remote_server_command;

/// Canvas for the benchmark. All reported timings are in milliseconds.
#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    repetitions: usize,
    stream_counts: [usize; 4],
    setup_kind: &'static str,
    per_session_setup_ms_1: f64,
    reference_transfer_ms: f64,
    reference_files: usize,
    reference_bytes: u64,
    results: Vec<CellResult>,
}

#[derive(Debug, Serialize)]
struct CellResult {
    streams: usize,
    setup_ms_median: f64,
    setup_ms_mad: f64,
    /// Wall-time increase of this stream count over the previous one, i.e. the
    /// marginal cost of adding one more parallel session.
    delta_setup_vs_previous_ms: f64,
    /// Reference transfer time over this stream count's setup time.
    transfer_over_setup_ratio: f64,
}

#[derive(Debug, thiserror::Error)]
enum BenchError {
    #[error("repetitions must be at least one")]
    ZeroRepetitions,
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
    #[error("cannot create scratch directory: {0}")]
    Scratch(std::io::Error),
    #[error("session handshake failed: {0}")]
    Handshake(std::io::Error),
    #[error("reference transfer failed: {0}")]
    Transfer(xsync_core::server::ServerError),
    #[error("server child failed: {0}")]
    Serve(xsync_core::server::ServerError),
}

#[derive(Debug, Parser)]
#[command(
    name = "xsync-connection-bench",
    about = "Measure per-session SSH/pipe connection setup separately from transfer"
)]
struct Cli {
    #[arg(long, default_value_t = 5)]
    repetitions: usize,
    #[arg(long, default_value = "benches/results/story-4.3/connection.json")]
    json: PathBuf,
    #[arg(long, default_value = "benches/results/story-4.3/connection.md")]
    markdown: PathBuf,
    /// Hidden: run as a `xsync --server` child so the benchmark can measure
    /// real child-process session setup without depending on an installed xsync.
    #[arg(long, hide = true, value_name = "ROOT")]
    server: Option<PathBuf>,
}

fn main() -> Result<(), BenchError> {
    let cli = Cli::parse();

    // If launched as a server child, serve over stdio and exit.
    if let Some(root) = &cli.server {
        xsync_core::server::run_server_stdio(root.clone())
            .map_err(BenchError::Serve)?;
        return Ok(());
    }
    if cli.repetitions == 0 {
        return Err(BenchError::ZeroRepetitions);
    }

    // A root the servers never need to materialize (handshake stops before the
    // destination scan); it is also the dest for the reference transfer.
    let scratch = std::env::temp_dir().join(format!(
        "xsync-conn-bench-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(BenchError::Scratch)?;

    let stream_counts: [usize; 4] = [1, 2, 4, 8];

    // Reference end-to-end single-stream transfer cost.
    let (ref_files, ref_bytes, ref_transfer_ms) =
        measure_reference_transfer(&scratch, cli.repetitions)?;

    let mut results: Vec<CellResult> = Vec::new();
    for &streams in &stream_counts {
        let setups = (0..cli.repetitions)
            .map(|_| measure_setup(&scratch, streams))
            .collect::<Result<Vec<f64>, _>>()?;
        let mut sorted = setups.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let mad = median_of_abs_deviation(&setups, median);
        let previous = results.last().map_or(0.0, |cell| cell.setup_ms_median);
        results.push(CellResult {
            streams,
            setup_ms_median: median,
            setup_ms_mad: mad,
            delta_setup_vs_previous_ms: median - previous,
            transfer_over_setup_ratio: if median > 0.0 {
                ref_transfer_ms / median
            } else {
                0.0
            },
        });
    }

    // median per-session (streams=1) for the headline.
    let per_session = results.first().map_or(0.0, |cell| cell.setup_ms_median);

    let report = Report {
        schema: "xsync.connection-bench.v1",
        repetitions: cli.repetitions,
        stream_counts,
        setup_kind: "pipe-child (same transport line as production ssh)",
        per_session_setup_ms_1: per_session,
        reference_transfer_ms: ref_transfer_ms,
        reference_files: ref_files,
        reference_bytes: ref_bytes,
        results,
    };

    write_report(&cli.json, &serde_json::to_vec_pretty(&report)?)?;
    write_report(&cli.markdown, report_markdown(&report).as_bytes())?;

    let _ = fs::remove_dir_all(&scratch);
    println!(
        "connection setup for {}/{} written to {} and {} (per-session setup {:.2} ms)",
        report.results.len(),
        cli.repetitions,
        cli.json.display(),
        cli.markdown.display(),
        report.per_session_setup_ms_1
    );
    Ok(())
}

/// Measure the setup cost for `streams` parallel `xsync --server` sessions,
/// stopping the clock at the `SessionConfig` acknowledgement.
fn measure_setup(root: &Path, streams: usize) -> Result<f64, BenchError> {
    let wall = Instant::now();
    let mut children: Vec<Child> = Vec::new();
    for _ in 0..streams {
        let (program, args) = remote_server_command(&root.to_string_lossy(), None, None);
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BenchError::Handshake)?;
        children.push(child);
    }

    let handles: Vec<_> = children
        .into_iter()
        .map(|mut child| {
            thread::spawn(move || -> Result<(), BenchError> {
                let stdin = child.stdin.take().ok_or_else(|| {
                    BenchError::Handshake(std::io::Error::other("no stdin"))
                })?;
                let stdout = child.stdout.take().ok_or_else(|| {
                    BenchError::Handshake(std::io::Error::other("no stdout"))
                })?;
                let mut writer = BufWriter::new(stdin);
                let mut reader = BufReader::new(stdout);
                let mut decoder = FrameDecoder::new();
                handshake_session(&mut reader, &mut writer, &mut decoder)?;
                Ok(())
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("handshake thread panicked")?;
    }
    Ok(wall.elapsed().as_secs_f64() * 1000.0)
}

fn handshake_session<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    decoder: &mut FrameDecoder,
) -> Result<(), BenchError> {
    // Client is Source; measure only up to the SessionConfig ack.
    writer
        .write_all(&encode_frame(
            1,
            &Message::Handshake {
                role: Role::Source,
                capabilities: 0,
                max_payload: MAX_COMPLETE_PAYLOAD as u32,
                max_segment: MAX_DATA_SEGMENT as u32,
                window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                job_id: [0u8; 16],
                compression: CompressionMode::None,
            },
        )
        .map_err(|e| BenchError::Handshake(std::io::Error::other(e)))?)
        .map_err(BenchError::Handshake)?;
    writer.flush().map_err(BenchError::Handshake)?;

    let server_hs = decoder
        .read(reader)
        .map_err(|e| BenchError::Handshake(std::io::Error::other(e)))?;
    if !matches!(server_hs.message, Message::Handshake { .. }) {
        return Err(BenchError::Handshake(std::io::Error::other(
            "no server handshake",
        )));
    }
    let _server_ack = decoder
        .read(reader)
        .map_err(|e| BenchError::Handshake(std::io::Error::other(e)))?;

    writer
        .write_all(&encode_frame(
            2,
            &Message::SessionConfig {
                streams: 1,
                batch_bytes: 32 * 1024 * 1024,
                chunk_bytes: 16 * 1024 * 1024,
                window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                delete: false,
                checksum: false,
                paranoid: false,
            },
        )
        .map_err(|e| BenchError::Handshake(std::io::Error::other(e)))?)
        .map_err(BenchError::Handshake)?;
    writer.flush().map_err(BenchError::Handshake)?;
    let _config_ack = decoder
        .read(reader)
        .map_err(|e| BenchError::Handshake(std::io::Error::other(e)))?;
    Ok(())
}

/// Reference end-to-end transfer of a fixed small corpus over the pipe
/// transport, so transfer cost is reported separately from connection setup.
#[allow(clippy::cast_precision_loss)]
fn measure_reference_transfer(
    root: &Path,
    repetitions: usize,
) -> Result<(usize, u64, f64), BenchError> {
    // Prepare a source tree under the scratch root.
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(BenchError::Scratch)?;
    for i in 0..200 {
        fs::write(src.join(format!("f{i:04}.bin")), vec![0xAB; 4096])
            .map_err(BenchError::Scratch)?;
    }
    fs::write(src.join("large.bin"), vec![0x5A; 1024 * 1024])
        .map_err(BenchError::Scratch)?;
    let files = 201u64;
    let bytes = 200 * 4096 + 1024 * 1024;

    let mut totals = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let dest = root.join("dst");
        let options = xsync_core::local::LocalSyncOptions {
            local_workers: 1,
            ..Default::default()
        };
        let start = Instant::now();
        // host=None -> in-process/local child server (no ssh).
        xsync_core::server::sync_push_server(&src, true, &dest.to_string_lossy(), true, &options, None, None, |_| {})
            .map_err(BenchError::Transfer)?;
        totals.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = totals.clone();
    sorted.sort_by(f64::total_cmp);
    Ok((files as usize, bytes, sorted[sorted.len() / 2]))
}

fn median_of_abs_deviation(values: &[f64], median: f64) -> f64 {
    let mut devs: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
    devs.sort_by(f64::total_cmp);
    devs[devs.len() / 2]
}

fn report_markdown(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# Story 4.3 — SSH connection-model benchmark\n");
    let _ = writeln!(out, "- schema: `{}`", report.schema);
    let _ = writeln!(out, "- repetitions: {}", report.repetitions);
    let _ = writeln!(
        out,
        "- setup kind: {}",
        report.setup_kind
    );
    let _ = writeln!(
        out,
        "- reference transfer: {} files, {} bytes, {:.2} ms",
        report.reference_files, report.reference_bytes, report.reference_transfer_ms
    );
    let _ = writeln!(
        out,
        "- per-session setup (streams=1): {:.2} ms",
        report.per_session_setup_ms_1
    );
    let _ = writeln!(out, "\n| streams | setup median (ms) | MAD (ms) | delta vs prev (ms) | transfer/setup |");
    let _ = writeln!(out, "|---:|---:|---:|---:|---:|");
    for r in &report.results {
        let _ = writeln!(
            out,
            "| {} | {:.2} | {:.2} | {:.2} | {:.2} |",
            r.streams,
            r.setup_ms_median,
            r.setup_ms_mad,
            r.delta_setup_vs_previous_ms,
            r.transfer_over_setup_ratio
        );
    }
    out
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