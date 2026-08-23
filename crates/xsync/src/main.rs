//! The `xsync` binary — CLI frontend over the `xsync-core` engine.
//!
//! Story 1.2: full clap argument surface with rsync-familiar wording.

use std::process::ExitCode;

use clap::Parser;

/// High-performance rsync replacement built on a parallel pipeline and BLAKE3.
#[derive(Debug, Parser)]
#[command(
    name = "xsync",
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

    /// Number of parallel streams, 1..=16 (default: 1; explicit values are honored).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=16))]
    streams: Option<u8>,

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
    #[arg(short = 'e', value_name = "CMD")]
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
            eprintln!("xsync: {e}");
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
}

/// Run the CLI command.
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
        dry_run: cli.dry_run,
        delete: cli.delete,
        paranoid: cli.paranoid,
        exclude_patterns: cli.exclude.clone(),
        ..xsync_core::local::LocalSyncOptions::default()
    };
    let progress_json = cli.progress_json;
    let quiet = cli.quiet;

    let report = if src.is_remote() {
        xsync_core::server::sync_pull_server(
            &src.path,
            src.trailing_slash,
            std::path::Path::new(&dest.path),
            dest.trailing_slash,
            &options,
            cli.rsh.as_deref(),
            src.host(),
            |event| render_event(event, progress_json, quiet),
        )?
    } else if dest.is_remote() {
        xsync_core::server::sync_push_server(
            std::path::Path::new(&src.path),
            src.trailing_slash,
            &dest.path,
            dest.trailing_slash,
            &options,
            cli.rsh.as_deref(),
            dest.host(),
            |event| render_event(event, progress_json, quiet),
        )?
    } else {
        xsync_core::local::sync(
            &src.path,
            src.trailing_slash,
            &dest.path,
            dest.trailing_slash,
            &options,
            |event| render_event(event, progress_json, quiet),
        )?
    };

    Ok(if report.partial_failure() {
        RunOutcome::Partial
    } else {
        RunOutcome::Complete
    })
}

fn render_event(event: xsync_core::local::LocalEvent, progress_json: bool, quiet: bool) {
    let is_error = matches!(
        &event,
        xsync_core::local::LocalEvent::Warning { .. }
            | xsync_core::local::LocalEvent::Failed { .. }
    );
    if quiet && !is_error {
        return;
    }
    if progress_json {
        println!("{}", json_event(&event));
        return;
    }
    match event {
        xsync_core::local::LocalEvent::Started {
            local_workers,
            streams,
        } => println!("local workers: {local_workers} (streams: {streams})"),
        xsync_core::local::LocalEvent::Planned { files, bytes } => {
            println!("planned {files} file(s), {bytes} bytes");
        }
        xsync_core::local::LocalEvent::Transferred {
            path,
            bytes,
            method,
            ..
        } => {
            println!("transferred {path} ({bytes} bytes, {})", method_name(method));
        }
        xsync_core::local::LocalEvent::Skipped { path } => println!("skipped {path}"),
        xsync_core::local::LocalEvent::Warning { path, message } => {
            println!("warning: {path}: {message}");
        }
        xsync_core::local::LocalEvent::Failed { path, message } => {
            println!("failed: {path}: {message}");
        }
        xsync_core::local::LocalEvent::Deleted { path } => println!("deleted {path}"),
        xsync_core::local::LocalEvent::Finished {
            transferred_files,
            transferred_bytes,
            physical_bytes,
            skipped_files,
            failed_entries,
            local_workers,
            partial_failure,
            ..
        } => println!(
            "finished: {transferred_files} transferred ({transferred_bytes} logical, {physical_bytes} physical bytes), {skipped_files} skipped, {failed_entries} failed, workers {local_workers}{}",
            if partial_failure {
                ", partial failure"
            } else {
                ""
            }
        ),
    }
}

fn method_name(method: xsync_core::local::TransferMethod) -> &'static str {
    match method {
        xsync_core::local::TransferMethod::DirectoryClone => "directory-clone",
        xsync_core::local::TransferMethod::FileClone => "file-clone",
        xsync_core::local::TransferMethod::ByteCopy => "byte-copy",
    }
}

fn json_event(event: &xsync_core::local::LocalEvent) -> serde_json::Value {
    use xsync_core::local::LocalEvent;

    match event {
        LocalEvent::Started {
            local_workers,
            streams,
        } => serde_json::json!({
            "event": "started",
            "local_workers": local_workers,
            "streams": streams,
        }),
        LocalEvent::Planned { files, bytes } => serde_json::json!({
            "event": "planned",
            "files": files,
            "bytes": bytes,
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
        LocalEvent::Skipped { path } => serde_json::json!({
            "event": "skipped",
            "path": path,
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
            transferred_files,
            transferred_bytes,
            physical_bytes,
            skipped_files,
            failed_entries,
            warnings,
            local_workers,
            streams,
            partial_failure,
            directory_clones,
            file_clones,
            byte_copies,
        } => serde_json::json!({
            "event": "finished",
            "transferred_files": transferred_files,
            "transferred_bytes": transferred_bytes,
            "physical_bytes": physical_bytes,
            "skipped_files": skipped_files,
            "failed_entries": failed_entries,
            "warnings": warnings,
            "local_workers": local_workers,
            "streams": streams,
            "partial_failure": partial_failure,
            "directory_clones": directory_clones,
            "file_clones": file_clones,
            "byte_copies": byte_copies,
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
            parse(&["xsync", "--bogus", "a", "b"]).unwrap_err().kind(),
            ErrorKind::UnknownArgument
        );
        // Missing SRC/DEST.
        assert_eq!(
            parse(&["xsync", "only-a"]).unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
        // --streams out of range.
        assert_eq!(
            parse(&["xsync", "--streams", "99", "a", "b"])
                .unwrap_err()
                .kind(),
            ErrorKind::ValueValidation
        );
        // --compress-level out of range.
        assert_eq!(
            parse(&["xsync", "--compress-level", "0", "a", "b"])
                .unwrap_err()
                .kind(),
            ErrorKind::ValueValidation
        );
    }

    #[test]
    fn server_mode_does_not_require_paths() {
        let cli = parse(&["xsync", "--server"]).unwrap();
        assert!(cli.server);
        assert!(cli.src.is_none());
        assert!(cli.dest.is_none());
    }

    #[test]
    fn streams_is_optional_and_defaults_to_none() {
        let cli = parse(&["xsync", "a", "b"]).unwrap();
        assert_eq!(cli.streams, None);
    }

    #[test]
    fn help_uses_the_story_0_5_stream_default() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("default: 1"));
        assert!(!help.contains("min(cpus, 8)"));
    }
}
