//! The `xs` binary — CLI frontend over the `xsync-core` engine.
//!
//! Story 1.2: full clap argument surface with rsync-familiar wording.

use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum TransportArg {
    #[default]
    Auto,
    Xsync,
    Rsync,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum CloudFilesArg {
    #[default]
    Download,
    Skip,
    Error,
}

/// High-performance rsync replacement built on a parallel pipeline and BLAKE3.
#[derive(Debug, Parser)]
#[command(
    name = "xs",
    version = xsync_core::version(),
    about = "High-performance rsync replacement",
    long_about = "xsync is an rsync-compatible file synchronization tool with a parallel \
                  pipeline, BLAKE3 integrity, and workload-adaptive transfer strategies."
)]
#[allow(clippy::struct_excessive_bools)] // a CLI with many boolean flags is expected
struct Cli {
    /// Run as the remote xsync server, speaking the protocol over stdin/stdout.
    ///
    /// This is how `ssh host xsync --server` drives a remote sync; it is not
    /// intended for interactive use.
    #[arg(long, hide = true)]
    server: bool,

    /// Remote transport: auto prefers xsync and falls back only when unavailable.
    #[arg(long, value_enum, default_value_t = TransportArg::Auto)]
    transport: TransportArg,

    /// Number of parallel streams, 1..=16 (default: 1; explicit values are honored).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=16))]
    streams: Option<u8>,

    /// Disable the local directory-clone fast path (benchmarking only).
    #[arg(long, hide = true)]
    no_directory_clone: bool,

    /// Delete extraneous files from the destination after a successful transfer.
    #[arg(long)]
    delete: bool,

    /// Exclude files matching the GLOB pattern (repeatable; matched against the relative path).
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Dry run: show what would be done without writing anything.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Classify by content hash (BLAKE3) instead of size+mtime.
    #[arg(long)]
    checksum: bool,

    /// Cloud placeholder policy: download, skip, or error.
    #[arg(long, value_enum, default_value_t = CloudFilesArg::Download)]
    cloud_files: CloudFilesArg,

    /// Re-read every written file from disk and verify its BLAKE3 hash.
    #[arg(long)]
    paranoid: bool,

    /// Emit a machine-readable JSONL event stream instead of progress bars.
    #[arg(long)]
    progress_json: bool,

    /// Disable data compression (zstd).
    #[arg(long)]
    no_compress: bool,

    /// zstd compression level, 1..=22 (default: 3).
    #[arg(long, value_name = "L", value_parser = clap::value_parser!(i32).range(1..=22))]
    compress_level: Option<i32>,

    /// Quiet: suppress all non-error output.
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Remote shell used to invoke the server, default `ssh`.
    #[arg(short = 'e', long, value_name = "CMD")]
    rsh: Option<String>,

    /// Source path. Either side may be `[user@]host:path`.
    #[arg(value_name = "SRC", required_unless_present = "server")]
    src: Option<String>,

    /// Destination path. Either side may be `[user@]host:path`.
    #[arg(value_name = "DEST", required_unless_present = "server")]
    dest: Option<String>,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(RunOutcome::Complete) => ExitCode::SUCCESS,
        Ok(RunOutcome::Partial) => ExitCode::from(xsync_core::local::PARTIAL_FAILURE_EXIT_CODE),
        Err(e) => {
            eprintln!("xs: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    Complete,
    Partial,
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error(transparent)]
    Path(#[from] xsync_core::path::PathError),
    #[error(transparent)]
    Local(#[from] xsync_core::local::LocalSyncError),
    #[error(transparent)]
    Server(#[from] xsync_core::server::ServerError),
    #[error(transparent)]
    Rsync(#[from] xsync_core::rsync::RsyncError),
    #[error("{0}")]
    Transport(String),
}

/// Run the CLI command.
#[allow(clippy::too_many_lines)]
fn run(cli: &Cli) -> Result<RunOutcome, CliError> {
    if cli.server {
        let root = cli
            .src
            .as_deref()
            .or(cli.dest.as_deref())
            .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
        xsync_core::server::run_server_stdio(root)?;
        return Ok(RunOutcome::Complete);
    }

    let src = xsync_core::path::parse(cli.src.as_deref().expect("SRC is required"))?;
    let dest = xsync_core::path::parse(cli.dest.as_deref().expect("DEST is required"))?;
    xsync_core::path::validate_pair(&src, &dest)?;

    let options = xsync_core::local::LocalSyncOptions {
        streams: usize::from(cli.streams.unwrap_or(xsync_core::DEFAULT_REMOTE_STREAMS)),
        directory_clones: !cli.no_directory_clone,
        dry_run: cli.dry_run,
        delete: cli.delete,
        checksum: cli.checksum,
        cloud_files: match cli.cloud_files {
            CloudFilesArg::Download => xsync_core::local::CloudFilesPolicy::Download,
            CloudFilesArg::Skip => xsync_core::local::CloudFilesPolicy::Skip,
            CloudFilesArg::Error => xsync_core::local::CloudFilesPolicy::Error,
        },
        paranoid: cli.paranoid,
        exclude_patterns: cli.exclude.clone(),
        compress: !cli.no_compress,
        compress_level: cli.compress_level.unwrap_or(3),
        ..xsync_core::local::LocalSyncOptions::default()
    };
    let progress_json = cli.progress_json;
    let quiet = cli.quiet;
    let mut progress = ProgressRenderer::new();
    let src_authority = src.authority();
    let dest_authority = dest.authority();

    let report = if src.is_remote() {
        if cli.transport == TransportArg::Rsync {
            return Err(CliError::Transport(
                "rsync transport currently supports local-to-remote only; install xsync remotely for remote-to-local"
                    .to_owned(),
            ));
        }
        let mut selection = native_selection(
            if cli.transport == TransportArg::Xsync {
                "explicit --transport=xsync"
            } else {
                "remote-to-local requires the native xsync receiver"
            },
            !cli.no_compress,
        );
        render_selection(&selection, progress_json, quiet);
        xsync_core::server::sync_pull_server(
            &src.path,
            src.trailing_slash,
            std::path::Path::new(&dest.path),
            dest.trailing_slash,
            &options,
            cli.rsh.as_deref(),
            src_authority.as_deref(),
            |event| {
                render_event(
                    &mut progress,
                    event,
                    progress_json,
                    quiet,
                    Some(&mut selection),
                );
            },
        )?
    } else if dest.is_remote() {
        let host = dest_authority
            .as_deref()
            .expect("remote destination has authority");
        match cli.transport {
            TransportArg::Xsync => {
                let mut selection =
                    native_selection("explicit --transport=xsync", !cli.no_compress);
                render_selection(&selection, progress_json, quiet);
                xsync_core::server::sync_push_server(
                    std::path::Path::new(&src.path),
                    src.trailing_slash,
                    &dest.path,
                    dest.trailing_slash,
                    &options,
                    cli.rsh.as_deref(),
                    Some(host),
                    |event| {
                        render_event(
                            &mut progress,
                            event,
                            progress_json,
                            quiet,
                            Some(&mut selection),
                        );
                    },
                )?
            }
            TransportArg::Rsync => run_rsync_push(cli, &src, &dest, &options, host)?,
            TransportArg::Auto => {
                let mut selection = native_selection("native receiver available", !cli.no_compress);
                let mut selection_emitted = false;
                let native = xsync_core::server::sync_push_server(
                    std::path::Path::new(&src.path),
                    src.trailing_slash,
                    &dest.path,
                    dest.trailing_slash,
                    &options,
                    cli.rsh.as_deref(),
                    Some(host),
                    |event| {
                        if !selection_emitted {
                            render_selection(&selection, progress_json, quiet);
                            selection_emitted = true;
                        }
                        render_event(
                            &mut progress,
                            event,
                            progress_json,
                            quiet,
                            Some(&mut selection),
                        );
                    },
                );
                match native {
                    Ok(report) => report,
                    Err(xsync_core::server::ServerError::MissingRemoteXsync) => {
                        if !quiet {
                            eprintln!("warning: remote xsync unavailable; trying supported rsync fallback");
                        }
                        run_rsync_push(cli, &src, &dest, &options, host)?
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    } else {
        if cli.transport == TransportArg::Rsync {
            return Err(CliError::Transport(
                "--transport=rsync is inapplicable to local-to-local sync".to_owned(),
            ));
        }
        let mut selection = local_selection();
        render_selection(&selection, progress_json, quiet);
        xsync_core::local::sync(
            &src.path,
            src.trailing_slash,
            &dest.path,
            dest.trailing_slash,
            &options,
            |event| {
                render_event(
                    &mut progress,
                    event,
                    progress_json,
                    quiet,
                    Some(&mut selection),
                );
            },
        )?
    };

    Ok(if report.partial_failure() {
        RunOutcome::Partial
    } else {
        RunOutcome::Complete
    })
}

fn run_rsync_push(
    cli: &Cli,
    src: &xsync_core::path::PathSpec,
    dest: &xsync_core::path::PathSpec,
    options: &xsync_core::local::LocalSyncOptions,
    host: &str,
) -> Result<xsync_core::local::LocalSyncReport, CliError> {
    let mut progress = ProgressRenderer::new();
    if cli.checksum {
        return Err(CliError::Transport(
            "rsync transport does not support --checksum in v1 (rsync MD4 is not BLAKE3)"
                .to_owned(),
        ));
    }
    if cli.compress_level.is_some() {
        return Err(CliError::Transport(
            "rsync transport does not support compression in v1".to_owned(),
        ));
    }
    xsync_core::rsync::validate_options(options)?;
    let peer = xsync_core::rsync::probe_remote(cli.rsh.as_deref(), host)?;
    xsync_core::rsync::validate_peer(&peer)?;
    let reason = if cli.transport == TransportArg::Auto {
        "remote xsync executable unavailable"
    } else {
        "explicit --transport=rsync"
    };
    let mut selection = peer.selection(reason);
    render_selection(&selection, cli.progress_json, cli.quiet);
    xsync_core::rsync::sync_push(
        std::path::Path::new(&src.path),
        src.trailing_slash,
        &dest.path,
        dest.trailing_slash,
        options,
        cli.rsh.as_deref(),
        host,
        &peer,
        |event| {
            render_event(
                &mut progress,
                event,
                cli.progress_json,
                cli.quiet,
                Some(&mut selection),
            );
        },
    )
    .map_err(Into::into)
}

fn local_selection() -> xsync_core::transport::TransportSelection {
    xsync_core::transport::TransportSelection {
        transport: xsync_core::transport::TransportKind::Local,
        remote_implementation: "in-process".to_owned(),
        remote_version: None,
        wire_version: 0,
        capabilities: xsync_core::transport::TransportCapabilities {
            multi_stream: false,
            durable_resume: false,
            blake3_frames: false,
            paranoid_readback: true,
            whole_file: true,
        },
        mapped_options: vec!["whole-file", "paranoid-readback"],
        checksum_algorithm: Some("blake3"),
        compression_algorithm: None,
        unavailable_guarantees: Vec::new(),
        reason: "both paths are local".to_owned(),
    }
}

fn native_selection(
    reason: &str,
    compression_enabled: bool,
) -> xsync_core::transport::TransportSelection {
    xsync_core::transport::TransportSelection {
        transport: xsync_core::transport::TransportKind::Xsync,
        remote_implementation: "xsync".to_owned(),
        remote_version: None,
        wire_version: xsync_core::PROTOCOL_VERSION,
        capabilities: xsync_core::transport::TransportCapabilities {
            multi_stream: true,
            durable_resume: true,
            blake3_frames: true,
            paranoid_readback: true,
            whole_file: true,
        },
        mapped_options: vec![
            "multi-stream",
            "durable-resume",
            "blake3-frames",
            "paranoid-readback",
        ],
        checksum_algorithm: Some("blake3"),
        compression_algorithm: compression_enabled.then_some("zstd"),
        unavailable_guarantees: Vec::new(),
        reason: reason.to_owned(),
    }
}

fn render_selection(
    selection: &xsync_core::transport::TransportSelection,
    progress_json: bool,
    quiet: bool,
) {
    if quiet {
        return;
    }
    if progress_json {
        println!(
            "{}",
            serde_json::json!({
                "event": "transport-selected",
                "transport": selection.transport.as_str(),
                "remote_implementation": selection.remote_implementation,
                "remote_version": selection.remote_version,
                "wire_version": selection.wire_version,
                "whole_file": selection.capabilities.whole_file,
                "multi_stream": selection.capabilities.multi_stream,
                "durable_resume": selection.capabilities.durable_resume,
                "blake3_frames": selection.capabilities.blake3_frames,
                "paranoid_readback": selection.capabilities.paranoid_readback,
                "mapped_options": selection.mapped_options,
                "checksum_algorithm": selection.checksum_algorithm,
                "compression_algorithm": selection.compression_algorithm,
                "unavailable_guarantees": selection.unavailable_guarantees,
                "reason": selection.reason,
            })
        );
    } else {
        let version = selection
            .remote_version
            .as_deref()
            .map_or(String::new(), |version| format!(" {version}"));
        println!(
            "transport: {} ({}{}, wire {}; {})",
            selection.transport.as_str(),
            selection.remote_implementation,
            version,
            selection.wire_version,
            selection.reason
        );
        if !selection.unavailable_guarantees.is_empty() {
            println!(
                "unavailable guarantees: {}",
                selection.unavailable_guarantees.join(", ")
            );
        }
        println!(
            "mapped options: {}; checksum: {}; compression: {}",
            selection.mapped_options.join(", "),
            selection.checksum_algorithm.unwrap_or("none"),
            selection.compression_algorithm.unwrap_or("none")
        );
    }
}

struct ProgressRenderer {
    terminal: bool,
    multi: Option<MultiProgress>,
    spinner: Option<ProgressBar>,
    total: Option<ProgressBar>,
    children: HashMap<(usize, String), ProgressBar>,
    started: Instant,
    last_plain: Instant,
    transfer_started: Option<Instant>,
    total_files: Option<usize>,
    total_bytes: Option<u64>,
    files: usize,
    bytes: u64,
    deleted_entries: usize,
    rate_bytes: u64,
    progress_seen: HashSet<String>,
    progress_offsets: HashMap<String, u64>,
    progress_streams: HashMap<String, usize>,
}

fn format_rate(bytes_per_second: f64) -> String {
    const UNITS: [&str; 5] = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"];
    let mut value = if bytes_per_second.is_finite() {
        bytes_per_second.max(0.0)
    } else {
        0.0
    };
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl ProgressRenderer {
    fn new() -> Self {
        let terminal = io::stdout().is_terminal();
        let multi = terminal.then(MultiProgress::new);
        let spinner = multi.as_ref().map(|multi| {
            let bar = multi.add(ProgressBar::new_spinner());
            bar.set_style(
                ProgressStyle::with_template("{spinner} {msg}").expect("valid progress template"),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            bar
        });
        Self {
            terminal,
            multi,
            spinner,
            total: None,
            children: HashMap::new(),
            started: Instant::now(),
            last_plain: Instant::now(),
            transfer_started: None,
            total_files: None,
            total_bytes: None,
            files: 0,
            bytes: 0,
            deleted_entries: 0,
            rate_bytes: 0,
            progress_seen: HashSet::new(),
            progress_offsets: HashMap::new(),
            progress_streams: HashMap::new(),
        }
    }

    fn transfer_elapsed(&self) -> f64 {
        self.transfer_started
            .unwrap_or(self.started)
            .elapsed()
            .as_secs_f64()
            .max(0.001)
    }

    #[allow(clippy::cast_precision_loss)]
    fn draw(&mut self, scanning: bool) {
        if self.terminal {
            if scanning {
                if let Some(spinner) = &self.spinner {
                    spinner.set_message(format!(
                        "scanning... | {} files | {} bytes",
                        self.files, self.bytes
                    ));
                }
            } else if let Some(total) = &self.total {
                if let Some(spinner) = self.spinner.take() {
                    spinner.finish_and_clear();
                }
                let rate = format_rate(self.rate_bytes as f64 / self.transfer_elapsed());
                total.set_message(format!("{} files | {rate}", self.files));
                total.set_position(self.bytes);
            }
            return;
        }
        let elapsed = self.transfer_elapsed();
        let files = self
            .total_files
            .map_or_else(|| "?".to_owned(), |n| n.to_string());
        let bytes = self
            .total_bytes
            .map_or_else(|| "?".to_owned(), |n| n.to_string());
        let rate = format_rate(self.rate_bytes as f64 / elapsed);
        let line = if scanning {
            format!(
                "scanning… | {}/{} files | {}/{} bytes",
                self.files, files, self.bytes, bytes
            )
        } else {
            format!(
                "transfer | {}/{} files | {}/{} bytes | {rate}",
                self.files, files, self.bytes, bytes
            )
        };
        if self.terminal {
            print!("\r\x1b[2K{line}");
            let _ = io::stdout().flush();
        } else if self.last_plain.elapsed() >= Duration::from_millis(250) {
            println!("{line}");
            self.last_plain = Instant::now();
        }
    }

    fn finish(&self) {
        if self.terminal {
            if let Some(spinner) = &self.spinner {
                spinner.finish_and_clear();
            }
            if let Some(total) = &self.total {
                total.finish();
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::too_many_arguments)]
    fn summary(
        &self,
        transferred_files: usize,
        transferred_bytes: u64,
        wire_bytes: u64,
        skipped_files: usize,
        deleted_entries: usize,
        failed_entries: usize,
        partial_failure: bool,
    ) {
        if self.terminal {
            let elapsed = self.transfer_elapsed();
            let rate = transferred_bytes as f64 / elapsed / 1024.0 / 1024.0;
            println!(
                "summary: {transferred_files} transferred, {skipped_files} skipped, {deleted_entries} deleted, {failed_entries} failed | {transferred_bytes} logical, {wire_bytes} wire bytes | elapsed: {:.2}s | throughput: {:.2} MiB/s | verification: {}",
                elapsed,
                rate,
                if partial_failure {
                    "partial"
                } else {
                    "passed"
                }
            );
        }
    }

    fn plan(&mut self, files: usize, bytes: u64) {
        self.transfer_started = Some(Instant::now());
        self.total_files = Some(files);
        self.total_bytes = Some(bytes);
        if self.terminal {
            let total = self
                .multi
                .as_ref()
                .expect("terminal progress owns a multiprogress")
                .add(ProgressBar::new(bytes));
            total.set_style(
                ProgressStyle::with_template(
                    "{bar:32.cyan} {percent:>3}%  {bytes}/{total_bytes}  {msg}",
                )
                .expect("valid progress template")
                .progress_chars("━╸─"),
            );
            self.total = Some(total);
        }
        self.draw(false);
    }

    fn file_progress(&mut self, path: &str, stream: usize, completed: u64, total: u64) {
        let key = (stream, path.to_owned());
        self.progress_streams.insert(path.to_owned(), stream);
        let previous = self
            .progress_offsets
            .insert(path.to_owned(), completed)
            .unwrap_or_default();
        self.rate_bytes = self
            .rate_bytes
            .saturating_add(completed.saturating_sub(previous));
        self.progress_seen.insert(path.to_owned());
        self.draw(false);
        if !self.terminal || total < 1024 * 1024 {
            return;
        }
        let bar = self.children.entry(key.clone()).or_insert_with(|| {
            let bar = self
                .multi
                .as_ref()
                .expect("terminal progress owns a multiprogress")
                .add(ProgressBar::new(total));
            bar.set_style(
                ProgressStyle::with_template(
                    "  {bar:24.green} {percent:>3}%  {bytes}/{total_bytes}  {msg}",
                )
                .expect("valid progress template")
                .progress_chars("━╸─"),
            );
            bar.set_message(format!("stream {stream}"));
            bar
        });
        bar.set_position(completed.min(total));
        if completed >= total {
            bar.finish_and_clear();
            self.children.remove(&key);
        }
    }

    fn print_file_status(&self, path: &str, status: &str) {
        let line = self.progress_streams.get(path).map_or_else(
            || format!("{path} | {status}"),
            |stream| format!("stream {stream}: {path} | {status}"),
        );
        if self.terminal {
            if let Some(multi) = &self.multi {
                let _ = multi.println(&line);
            }
        } else {
            println!("{line}");
        }
    }

    fn skipped(&mut self, bytes: u64) {
        self.files = self.files.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        if let Some(total_bytes) = &mut self.total_bytes {
            *total_bytes = total_bytes.saturating_add(bytes);
        }
        if let Some(total) = &self.total {
            total.set_length(self.total_bytes.unwrap_or_default());
        }
        self.draw(false);
    }
}

#[allow(clippy::too_many_lines)]
fn render_event(
    progress: &mut ProgressRenderer,
    mut event: xsync_core::local::LocalEvent,
    progress_json: bool,
    quiet: bool,
    mut selection: Option<&mut xsync_core::transport::TransportSelection>,
) {
    if let xsync_core::local::LocalEvent::Negotiated {
        compression_algorithm,
        compression_reason,
    } = &event
    {
        if let Some(selection) = selection.as_deref_mut() {
            selection.compression_algorithm = (*compression_algorithm == "zstd").then_some("zstd");
            selection.reason = format!("{}; compression: {}", selection.reason, compression_reason);
        }
    }
    if let xsync_core::local::LocalEvent::ProtocolNegotiated {
        selected_version,
        browse_available,
        ..
    } = &event
    {
        if let Some(selection) = selection.as_deref_mut() {
            selection.wire_version = *selected_version;
            if !browse_available && !selection.unavailable_guarantees.contains(&"browse-v2") {
                selection.unavailable_guarantees.push("browse-v2");
            }
        }
    }
    if let xsync_core::local::LocalEvent::Finished { transport, .. } = &mut event {
        *transport = selection.cloned();
    }
    let is_error = matches!(
        &event,
        xsync_core::local::LocalEvent::Warning { .. }
            | xsync_core::local::LocalEvent::Failed { .. }
    );
    if quiet && !is_error {
        return;
    }
    if progress_json {
        let mut value = json_event(&event);
        if let Some(object) = value.as_object_mut() {
            object.insert("schema_version".to_owned(), serde_json::json!(1));
            object.insert("type".to_owned(), serde_json::json!(event_type(&event)));
            object.insert(
                "timestamp_unix_nanos".to_owned(),
                serde_json::json!(std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default()),
            );
        }
        println!("{value}");
        return;
    }
    match &event {
        xsync_core::local::LocalEvent::Started { .. } => progress.draw(true),
        xsync_core::local::LocalEvent::Planned { files, bytes } => {
            progress.plan(*files, *bytes);
        }
        xsync_core::local::LocalEvent::Transferred { path, bytes, .. } => {
            progress.files = progress.files.saturating_add(1);
            progress.bytes = progress.bytes.saturating_add(*bytes);
            if !progress.progress_seen.contains(path) {
                progress.rate_bytes = progress.rate_bytes.saturating_add(*bytes);
            }
            progress.draw(false);
            progress.print_file_status(path, "transferred");
        }
        xsync_core::local::LocalEvent::Progress {
            path,
            stream,
            completed,
            total,
        } => progress.file_progress(path, *stream, *completed, *total),
        xsync_core::local::LocalEvent::Skipped { path, bytes } => {
            progress.skipped(*bytes);
            progress.print_file_status(path, "already present");
        }
        xsync_core::local::LocalEvent::Finished {
            transferred_files,
            transferred_bytes,
            wire_bytes,
            skipped_files,
            failed_entries,
            partial_failure,
            ..
        } => {
            progress.finish();
            progress.summary(
                *transferred_files,
                *transferred_bytes,
                *wire_bytes,
                *skipped_files,
                progress.deleted_entries,
                *failed_entries,
                *partial_failure,
            );
        }
        xsync_core::local::LocalEvent::Deleted { .. } => {
            progress.deleted_entries = progress.deleted_entries.saturating_add(1);
        }
        _ => {}
    }
    match event {
        xsync_core::local::LocalEvent::Negotiated { .. }
        | xsync_core::local::LocalEvent::ProtocolNegotiated { .. }
        | xsync_core::local::LocalEvent::Metrics { .. }
        | xsync_core::local::LocalEvent::Phase { .. }
        | xsync_core::local::LocalEvent::Started { .. }
        | xsync_core::local::LocalEvent::Planned { .. }
        | xsync_core::local::LocalEvent::Transferred { .. }
        | xsync_core::local::LocalEvent::Progress { .. }
        | xsync_core::local::LocalEvent::Skipped { .. }
        | xsync_core::local::LocalEvent::Deleted { .. } => {}
        xsync_core::local::LocalEvent::CloudPlaceholders {
            files,
            bytes,
            detection_available,
        } => println!(
            "cloud placeholders: {files} file(s), {bytes} bytes (detection available: {detection_available})"
        ),
        xsync_core::local::LocalEvent::Action { path, action } => {
            println!("{action} {path}");
        }
        xsync_core::local::LocalEvent::Warning { path, message } => {
            println!("warning: {path}: {message}");
        }
        xsync_core::local::LocalEvent::Failed { path, message } => {
            println!("failed: {path}: {message}");
        }
        xsync_core::local::LocalEvent::Finished {
            transport: _,
            transferred_files,
            transferred_bytes,
            physical_bytes,
            wire_bytes,
            skipped_files,
            deleted_entries,
            failed_entries,
            local_workers,
            partial_failure,
            restarted_files,
            resumed_bytes,
            checkpoint_bytes,
            ..
        } => println!(
            "finished: {transferred_files} transferred ({transferred_bytes} logical, {physical_bytes} physical, {wire_bytes} wire bytes), {skipped_files} skipped, {deleted_entries} deleted, {failed_entries} failed, workers {local_workers}, resume: {restarted_files} restarted, {resumed_bytes} resumed, {checkpoint_bytes} checkpointed{}",
            if partial_failure {
                ", partial failure"
            } else {
                ""
            }
        ),
    }
}

fn event_type(event: &xsync_core::local::LocalEvent) -> &'static str {
    use xsync_core::local::LocalEvent;
    match event {
        LocalEvent::Phase { .. } => "phase",
        LocalEvent::Metrics { .. } => "metrics",
        LocalEvent::Started { .. } => "started",
        LocalEvent::Negotiated { .. } => "negotiated",
        LocalEvent::ProtocolNegotiated { .. } => "protocol-negotiated",
        LocalEvent::Planned { .. } => "planned",
        LocalEvent::CloudPlaceholders { .. } => "cloud_placeholders",
        LocalEvent::Transferred { .. } => "transferred",
        LocalEvent::Progress { .. } => "progress",
        LocalEvent::Skipped { .. } => "skipped",
        LocalEvent::Action { .. } => "action",
        LocalEvent::Warning { .. } => "warning",
        LocalEvent::Failed { .. } => "failed",
        LocalEvent::Deleted { .. } => "deleted",
        LocalEvent::Finished { .. } => "done",
    }
}

fn method_name(method: xsync_core::local::TransferMethod) -> &'static str {
    match method {
        xsync_core::local::TransferMethod::DirectoryClone => "directory-clone",
        xsync_core::local::TransferMethod::FileClone => "file-clone",
        xsync_core::local::TransferMethod::ByteCopy => "byte-copy",
    }
}

#[allow(clippy::too_many_lines)]
fn json_event(event: &xsync_core::local::LocalEvent) -> serde_json::Value {
    use xsync_core::local::LocalEvent;

    match event {
        LocalEvent::Phase { name, started } => serde_json::json!({
            "event": "phase",
            "name": name,
            "started": started,
        }),
        LocalEvent::Metrics {
            queue_high_water,
            compression_algorithm,
            compression_level,
        } => serde_json::json!({
            "event": "metrics",
            "queue_high_water": queue_high_water,
            "compression_algorithm": compression_algorithm,
            "compression_level": compression_level,
        }),
        LocalEvent::Started {
            local_workers,
            streams,
        } => serde_json::json!({
            "event": "started",
            "local_workers": local_workers,
            "streams": streams,
        }),
        LocalEvent::Negotiated {
            compression_algorithm,
            compression_reason,
        } => serde_json::json!({
            "event": "negotiated",
            "compression_algorithm": compression_algorithm,
            "compression_reason": compression_reason,
        }),
        LocalEvent::ProtocolNegotiated {
            selected_version,
            remote_capabilities,
            common_capabilities,
            browse_available,
        } => serde_json::json!({
            "event": "protocol-negotiated",
            "selected_version": selected_version,
            "remote_capabilities": remote_capabilities,
            "common_capabilities": common_capabilities,
            "browse_available": browse_available,
        }),
        LocalEvent::Planned { files, bytes } => serde_json::json!({
            "event": "planned",
            "files": files,
            "bytes": bytes,
        }),
        LocalEvent::CloudPlaceholders {
            files,
            bytes,
            detection_available,
        } => serde_json::json!({
            "event": "cloud_placeholders",
            "files": files,
            "bytes": bytes,
            "detection_available": detection_available,
        }),
        LocalEvent::Transferred {
            path,
            bytes,
            physical_bytes,
            method,
        } => serde_json::json!({
            "event": "transferred",
            "path": path,
            "bytes": bytes,
            "physical_bytes": physical_bytes,
            "method": method_name(*method),
        }),
        LocalEvent::Progress {
            path,
            stream,
            completed,
            total,
        } => serde_json::json!({
            "event": "progress",
            "path": path,
            "stream": stream,
            "completed": completed,
            "total": total,
        }),
        LocalEvent::Skipped { path, bytes } => serde_json::json!({
            "event": "skipped",
            "path": path,
            "bytes": bytes,
        }),
        LocalEvent::Action { path, action } => serde_json::json!({
            "event": "action",
            "schema_version": 1,
            "path": path,
            "action": action,
        }),
        LocalEvent::Warning { path, message } => serde_json::json!({
            "event": "warning",
            "path": path,
            "message": message,
        }),
        LocalEvent::Failed { path, message } => serde_json::json!({
            "event": "failed",
            "path": path,
            "message": message,
        }),
        LocalEvent::Deleted { path } => serde_json::json!({
            "event": "deleted",
            "path": path,
        }),
        LocalEvent::Finished {
            transport,
            transferred_files,
            transferred_bytes,
            physical_bytes,
            wire_bytes,
            skipped_files,
            failed_entries,
            deleted_entries,
            warnings,
            local_workers,
            streams,
            partial_failure,
            directory_clones,
            file_clones,
            byte_copies,
            restarted_files,
            resumed_bytes,
            retransmitted_bytes,
            checkpoint_bytes,
            checksum_cache_hits,
            checksum_cache_misses,
        } => serde_json::json!({
            "event": "finished",
            "transferred_files": transferred_files,
            "transferred_bytes": transferred_bytes,
            "physical_bytes": physical_bytes,
            "wire_bytes": wire_bytes,
            "skipped_files": skipped_files,
            "failed_entries": failed_entries,
            "deleted_entries": deleted_entries,
            "warnings": warnings,
            "local_workers": local_workers,
            "streams": streams,
            "partial_failure": partial_failure,
            "directory_clones": directory_clones,
            "file_clones": file_clones,
            "byte_copies": byte_copies,
            "restarted_files": restarted_files,
            "resumed_bytes": resumed_bytes,
            "retransmitted_bytes": retransmitted_bytes,
            "checkpoint_bytes": checkpoint_bytes,
            "checksum_cache_hits": checksum_cache_hits,
            "checksum_cache_misses": checksum_cache_misses,
            "transport": transport.as_ref().map(|selection| selection.transport.as_str()),
            "remote_implementation": transport.as_ref().map(|selection| selection.remote_implementation.as_str()),
            "remote_version": transport.as_ref().and_then(|selection| selection.remote_version.as_deref()),
            "wire_version": transport.as_ref().map(|selection| selection.wire_version),
            "mapped_options": transport.as_ref().map(|selection| selection.mapped_options.as_slice()),
            "unavailable_guarantees": transport.as_ref().map(|selection| selection.unavailable_guarantees.as_slice()),
            "checksum_algorithm": transport.as_ref().and_then(|selection| selection.checksum_algorithm),
            "compression_algorithm": transport.as_ref().and_then(|selection| selection.compression_algorithm),
            "selection_reason": transport.as_ref().map(|selection| selection.reason.as_str()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{error::ErrorKind, CommandFactory as _};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn parses_full_flag_surface() {
        let cli = parse(&[
            "xsync",
            "--streams",
            "4",
            "--delete",
            "--exclude",
            "*.log",
            "--exclude",
            "target",
            "-n",
            "--checksum",
            "--paranoid",
            "--progress-json",
            "--no-compress",
            "--compress-level",
            "9",
            "-q",
            "-e",
            "ssh",
            "host:src/",
            "dest",
        ])
        .unwrap();
        assert_eq!(cli.streams, Some(4));
        assert_eq!(cli.transport, TransportArg::Auto);
        assert!(cli.delete);
        assert_eq!(cli.exclude, ["*.log", "target"]);
        assert!(cli.dry_run);
        assert!(cli.checksum);
        assert!(cli.paranoid);
        assert!(cli.progress_json);
        assert!(cli.no_compress);
        assert_eq!(cli.compress_level, Some(9));
        assert!(cli.quiet);
        assert_eq!(cli.rsh.as_deref(), Some("ssh"));
        assert_eq!(cli.src.as_deref(), Some("host:src/"));
        assert_eq!(cli.dest.as_deref(), Some("dest"));
        assert!(!cli.server);
    }

    #[test]
    fn parse_failure_is_not_a_panic() {
        // Unknown flag.
        assert_eq!(
            parse(&["xs", "--bogus", "a", "b"]).unwrap_err().kind(),
            ErrorKind::UnknownArgument
        );
        // Missing SRC/DEST.
        assert_eq!(
            parse(&["xs", "only-a"]).unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
        // --streams out of range.
        assert_eq!(
            parse(&["xs", "--streams", "99", "a", "b"])
                .unwrap_err()
                .kind(),
            ErrorKind::ValueValidation
        );
        // --compress-level out of range.
        assert_eq!(
            parse(&["xs", "--compress-level", "0", "a", "b"])
                .unwrap_err()
                .kind(),
            ErrorKind::ValueValidation
        );
    }

    #[test]
    fn server_mode_does_not_require_paths() {
        let cli = parse(&["xs", "--server"]).unwrap();
        assert!(cli.server);
        assert!(cli.src.is_none());
        assert!(cli.dest.is_none());
    }

    #[test]
    fn streams_is_optional_and_defaults_to_none() {
        let cli = parse(&["xs", "a", "b"]).unwrap();
        assert_eq!(cli.streams, None);
        assert_eq!(cli.transport, TransportArg::Auto);
    }

    #[test]
    fn parses_transport_modes() {
        for (name, expected) in [
            ("auto", TransportArg::Auto),
            ("xsync", TransportArg::Xsync),
            ("rsync", TransportArg::Rsync),
        ] {
            let cli = parse(&["xs", "--transport", name, "a", "host:b"]).unwrap();
            assert_eq!(cli.transport, expected);
        }
    }

    #[test]
    fn help_uses_the_story_0_5_stream_default() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("default: 1"));
        assert!(!help.contains("min(cpus, 8)"));
    }

    #[test]
    fn progress_json_phase_event_has_stable_fields() {
        let value = json_event(&xsync_core::local::LocalEvent::Phase {
            name: "scan",
            started: true,
        });
        assert_eq!(value["event"], "phase");
        assert_eq!(value["name"], "scan");
        assert_eq!(value["started"], true);
    }

    #[test]
    fn progress_json_protocol_negotiation_is_observable() {
        let value = json_event(&xsync_core::local::LocalEvent::ProtocolNegotiated {
            selected_version: 1,
            remote_capabilities: 0,
            common_capabilities: 0,
            browse_available: false,
        });
        assert_eq!(value["event"], "protocol-negotiated");
        assert_eq!(value["selected_version"], 1);
        assert_eq!(value["browse_available"], false);
    }

    #[test]
    fn formats_transfer_rates_using_readable_units() {
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1024.0), "1.0 KiB/s");
        assert_eq!(format_rate(1024.0 * 1024.0), "1.0 MiB/s");
        assert_eq!(format_rate(1024.0 * 1024.0 * 1024.0), "1.0 GiB/s");
    }
}
