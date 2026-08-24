use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;
use serde::Serialize;
use xsync_bench::corpus::{create_corpus, CorpusClass, CorpusRequest, Tier, Workload};
use xsync_bench::gate::evaluate_gate;
use xsync_bench::manifest::{build_manifest, verify_manifest, verify_manifest_sampled, Manifest};
use xsync_bench::report::{rotated_schedule, Report, ReportInput};
use xsync_bench::scratch::{clean_owned, OwnedScratch};

#[derive(Debug, Parser)]
#[command(
    name = "xsync-bench",
    about = "Reproducible xsync benchmark reports, correctness oracle, and gates"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a deterministic, content-pinned corpus and workload state.
    Corpus {
        /// Parent for a new marker-owned run directory.
        #[arg(long)]
        base: PathBuf,
        /// Corpus shape or content class.
        #[arg(long, value_enum)]
        class: CorpusClass,
        /// Size tier; smoke is suitable for ordinary CI.
        #[arg(long, value_enum, default_value_t = Tier::Smoke)]
        tier: Tier,
        /// Initial source/destination state.
        #[arg(long, value_enum, default_value_t = Workload::InitialCopy)]
        workload: Workload,
        /// Deterministic content and metadata seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Override generated entries below the source root.
        #[arg(long)]
        entry_count: Option<u64>,
        /// Override bytes for the one-large-file corpus.
        #[arg(long)]
        large_file_bytes: Option<u64>,
    },
    /// Create an independent content/metadata manifest for a filesystem tree.
    Manifest {
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify a filesystem tree against an expected manifest.
    Verify {
        root: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        json: Option<PathBuf>,
        /// Fraction of regular-file contents to hash; metadata is always checked.
        #[arg(long)]
        sample: Option<f64>,
        /// Seed for deterministic sampled content selection.
        #[arg(long, default_value_t = 0)]
        sample_seed: u64,
    },
    /// Validate raw samples and emit versioned JSON plus Markdown.
    Report {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        json: PathBuf,
        #[arg(long)]
        markdown: PathBuf,
    },
    /// Compare a report to a comparable historical report.
    Gate {
        #[arg(long)]
        current: PathBuf,
        #[arg(long)]
        baseline: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Emit deterministic rotated measurement order as JSON.
    Schedule {
        #[arg(long, value_delimiter = ',', num_args = 2..)]
        methods: Vec<String>,
        #[arg(long, default_value_t = 5)]
        repetitions: u32,
        #[arg(long)]
        out: PathBuf,
    },
    /// Allocate a marker-owned benchmark scratch run.
    ScratchCreate {
        #[arg(long)]
        base: PathBuf,
    },
    /// Remove exactly one marker-owned benchmark scratch run.
    ScratchClean {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        base: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xsync-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<bool, CliError> {
    match cli.command {
        Command::Corpus {
            base,
            class,
            tier,
            workload,
            seed,
            entry_count,
            large_file_bytes,
        } => {
            let request = CorpusRequest {
                class,
                tier,
                workload,
                seed,
                entry_count,
                large_file_bytes,
            };
            create_and_print_corpus(&base, &request)
        }
        Command::Manifest { root, out } => {
            let manifest = build_manifest(root)?;
            write_json(&out, &manifest)?;
            println!("{}", manifest.manifest_digest);
            Ok(true)
        }
        Command::Verify {
            root,
            manifest,
            json,
            sample,
            sample_seed,
        } => verify_tree(&root, &manifest, json.as_deref(), sample, sample_seed),
        Command::Report {
            input,
            json,
            markdown,
        } => {
            let input: ReportInput = read_json(&input)?;
            let report = Report::from_input(input)?;
            write_json(&json, &report)?;
            atomic_write(&markdown, report.to_markdown().as_bytes())?;
            println!(
                "wrote {} result(s) to '{}' and '{}'",
                report.results.len(),
                json.display(),
                markdown.display()
            );
            Ok(true)
        }
        Command::Gate {
            current,
            baseline,
            strict,
            json,
        } => {
            let current: Report = read_json(&current)?;
            let baseline = baseline.as_ref().map(|path| read_json(path)).transpose()?;
            let outcome = evaluate_gate(&current, baseline.as_ref(), strict);
            if let Some(path) = json {
                write_json(&path, &outcome)?;
            }
            println!("{}", outcome.render());
            Ok(outcome.passed())
        }
        Command::Schedule {
            methods,
            repetitions,
            out,
        } => {
            let schedule = rotated_schedule(&methods, repetitions)?;
            write_json(&out, &schedule)?;
            Ok(true)
        }
        Command::ScratchCreate { base } => {
            let scratch = OwnedScratch::create(base)?;
            println!("{}", scratch.path().display());
            Ok(true)
        }
        Command::ScratchClean { root, base } => {
            clean_owned(root, base)?;
            Ok(true)
        }
    }
}

fn create_and_print_corpus(base: &Path, request: &CorpusRequest) -> Result<bool, CliError> {
    let generated = create_corpus(base, request)?;
    println!("{}", generated.root().display());
    Ok(true)
}

fn verify_tree(
    root: &Path,
    manifest: &Path,
    json: Option<&Path>,
    sample: Option<f64>,
    sample_seed: u64,
) -> Result<bool, CliError> {
    let expected: Manifest = read_json(manifest)?;
    let verification = match sample {
        Some(fraction) => verify_manifest_sampled(root, &expected, fraction, sample_seed)?,
        None => verify_manifest(root, &expected)?,
    };
    if let Some(path) = json {
        write_json(path, &verification)?;
    }
    if verification.passed {
        println!(
            "manifest verified: {} items, {} logical bytes",
            verification.item_count, verification.logical_bytes
        );
    } else {
        eprintln!(
            "manifest mismatch: {} difference(s); expected {}, actual {}",
            verification.mismatch_count,
            verification.expected_manifest_digest,
            verification.actual_manifest_digest
        );
        for mismatch in &verification.mismatches {
            eprintln!("{}: {}", mismatch.path, mismatch.reason);
        }
    }
    Ok(verification.passed)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, CliError> {
    let bytes = fs::read(path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| CliError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), CliError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| CliError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::Clock)?
        .as_nanos();
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("xsync-bench"),
        std::process::id(),
        nonce
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| CliError::Io {
            path: temporary.clone(),
            source,
        })?;
    file.write_all(bytes).map_err(|source| CliError::Io {
        path: temporary.clone(),
        source,
    })?;
    file.sync_all().map_err(|source| CliError::Io {
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("I/O failed at '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON at '{}': {source}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    JsonEncode(#[from] serde_json::Error),
    #[error(transparent)]
    Corpus(#[from] xsync_bench::corpus::CorpusError),
    #[error(transparent)]
    Manifest(#[from] xsync_bench::manifest::ManifestError),
    #[error(transparent)]
    Report(#[from] xsync_bench::report::ReportError),
    #[error(transparent)]
    Scratch(#[from] xsync_bench::scratch::ScratchError),
    #[error("system clock is before the Unix epoch")]
    Clock,
}
