use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use serde::Serialize;
use xsync_bench::manifest::{build_manifest, verify_manifest};
use xsync_engine_bench::clone_report::{CacheState, CloneMethod, CloneReport, CloneSample};
use xsync_engine_bench::clone_spike::{
    clone_directory_or_fallback, clone_file_or_fallback, copy_directory_baseline,
    copy_file_baseline, CloneOutcome, DirectoryClonePolicy,
};
use xsync_engine_bench::environment::environment;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ObjectKind {
    File,
    Directory,
}

impl ObjectKind {
    const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "xsync-clone-bench",
    about = "Paired verified ordinary-copy and clone/reflink spike"
)]
struct Cli {
    /// Content-pinned source file or complete directory tree.
    #[arg(long)]
    source: PathBuf,
    /// Exact temporary destination path, which must not exist.
    #[arg(long)]
    destination: PathBuf,
    /// Source object kind.
    #[arg(long, value_enum)]
    kind: ObjectKind,
    /// Paired repetitions; method order rotates every repetition.
    #[arg(long, default_value_t = 5)]
    repetitions: u32,
    /// Verify the published final name in addition to the staging result.
    #[arg(long)]
    paranoid: bool,
    /// Versioned JSON output.
    #[arg(long)]
    json: PathBuf,
    /// Human-readable Markdown output.
    #[arg(long)]
    markdown: PathBuf,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xsync-clone-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Cli) -> Result<(), BenchError> {
    if options.repetitions < 5 {
        return Err(BenchError::TooFewRepetitions);
    }
    let source = fs::canonicalize(&options.source).map_err(|source_error| BenchError::Io {
        operation: "canonicalize source",
        path: options.source,
        source: source_error,
    })?;
    validate_kind(&source, options.kind)?;
    if fs::symlink_metadata(&options.destination).is_ok() {
        return Err(BenchError::DestinationExists(options.destination));
    }
    let expected = build_manifest(&source)?;
    let mut samples = Vec::with_capacity(options.repetitions as usize * 2);
    let mut observation = 0_u32;
    for repetition in 0..options.repetitions {
        let order = if repetition.is_multiple_of(2) {
            [CloneMethod::CloneOrFallback, CloneMethod::BufferedVerified]
        } else {
            [CloneMethod::BufferedVerified, CloneMethod::CloneOrFallback]
        };
        for (method_order, method) in order.into_iter().enumerate() {
            let start = Instant::now();
            let outcome = execute(
                options.kind,
                method,
                &source,
                &options.destination,
                options.paranoid,
            )?;
            let elapsed = start.elapsed().as_secs_f64();
            let verification = verify_manifest(&options.destination, &expected)?;
            if !verification.passed {
                return Err(BenchError::Verification {
                    path: options.destination,
                    mismatches: verification.mismatch_count,
                });
            }
            samples.push(CloneSample {
                repetition,
                method_order: u32::try_from(method_order).expect("two methods fit in u32"),
                method,
                wall_seconds: elapsed,
                disposition: outcome.disposition,
                verification_passed: true,
                cache_state: if observation == 0 {
                    CacheState::FirstPass
                } else {
                    CacheState::Warm
                },
            });
            observation += 1;
            remove_exact(&options.destination)?;
        }
    }
    let report = CloneReport::from_samples(
        environment(&source, "xsync-clone-bench"),
        options.kind.label().to_owned(),
        expected.manifest_digest,
        expected.logical_bytes,
        options.paranoid,
        samples,
    )?;
    write_json(&options.json, &report)?;
    atomic_write(&options.markdown, report.to_markdown().as_bytes())?;
    println!(
        "paired verified clone speedup {:.3}x (MAD {:.3}x); capability {}",
        report.paired_clone_speedup,
        report.paired_clone_speedup_mad,
        if report.clone_capability_available {
            "available"
        } else {
            "fell back"
        }
    );
    Ok(())
}

fn validate_kind(source: &Path, kind: ObjectKind) -> Result<(), BenchError> {
    let metadata = fs::symlink_metadata(source).map_err(|source_error| BenchError::Io {
        operation: "inspect source",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let valid = match kind {
        ObjectKind::File => metadata.is_file(),
        ObjectKind::Directory => metadata.is_dir() && !metadata.file_type().is_symlink(),
    };
    if valid {
        Ok(())
    } else {
        Err(BenchError::WrongKind {
            path: source.to_path_buf(),
            expected: kind.label(),
        })
    }
}

fn execute(
    kind: ObjectKind,
    method: CloneMethod,
    source: &Path,
    destination: &Path,
    paranoid: bool,
) -> Result<CloneOutcome, BenchError> {
    let policy = DirectoryClonePolicy::default();
    match (kind, method) {
        (ObjectKind::File, CloneMethod::BufferedVerified) => {
            copy_file_baseline(source, destination, paranoid)
        }
        (ObjectKind::File, CloneMethod::CloneOrFallback) => {
            clone_file_or_fallback(source, destination, paranoid)
        }
        (ObjectKind::Directory, CloneMethod::BufferedVerified) => {
            copy_directory_baseline(source, destination, policy, paranoid)
        }
        (ObjectKind::Directory, CloneMethod::CloneOrFallback) => {
            clone_directory_or_fallback(source, destination, policy, paranoid)
        }
    }
    .map_err(BenchError::Clone)
}

fn remove_exact(path: &Path) -> Result<(), BenchError> {
    let metadata = fs::symlink_metadata(path).map_err(|source_error| BenchError::Io {
        operation: "inspect completed benchmark target",
        path: path.to_path_buf(),
        source: source_error,
    })?;
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|source_error| BenchError::Io {
        operation: "remove completed benchmark target",
        path: path.to_path_buf(),
        source: source_error,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), BenchError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), BenchError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source_error| BenchError::Io {
        operation: "create report directory",
        path: parent.to_path_buf(),
        source: source_error,
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BenchError::Clock)?
        .as_nanos();
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("xsync-clone-bench"),
        std::process::id(),
        nonce
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source_error| BenchError::Io {
            operation: "create temporary report",
            path: temporary.clone(),
            source: source_error,
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source_error| BenchError::Io {
            operation: "write and sync temporary report",
            path: temporary.clone(),
            source: source_error,
        })?;
    fs::rename(&temporary, path).map_err(|source_error| BenchError::Io {
        operation: "publish report",
        path: path.to_path_buf(),
        source: source_error,
    })
}

#[derive(Debug, thiserror::Error)]
enum BenchError {
    #[error("clone evidence requires at least five paired repetitions")]
    TooFewRepetitions,
    #[error("benchmark destination already exists: '{}'", .0.display())]
    DestinationExists(PathBuf),
    #[error("source '{}' is not a {expected}", path.display())]
    WrongKind {
        path: PathBuf,
        expected: &'static str,
    },
    #[error("independent verification of '{}' found {mismatches} mismatch(es)", path.display())]
    Verification { path: PathBuf, mismatches: u64 },
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
    Clone(#[from] xsync_engine_bench::clone_spike::CloneError),
    #[error(transparent)]
    Manifest(#[from] xsync_bench::manifest::ManifestError),
    #[error(transparent)]
    Report(#[from] xsync_engine_bench::clone_report::CloneReportError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
