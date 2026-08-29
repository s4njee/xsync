//! The `xs` binary — CLI frontend over the `xsync-core` engine.
//!
//! Story 1.2: full clap argument surface with rsync-familiar wording.

use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{ArgMatches, CommandFactory as _, FromArgMatches as _, Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

mod config;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum TransportArg {
    #[default]
    Auto,
    Xsync,
    Rsync,
}

/// What to do when the remote has no xsync binary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum BootstrapArg {
    /// Never upload; require xsync to already be installed remotely.
    #[default]
    Off,
    /// Upload a verified binary for this run, then remove it.
    Once,
    /// Upload a verified binary and leave it in place for later runs.
    Persist,
}

impl From<BootstrapArg> for xsync_core::bootstrap::BootstrapPolicy {
    fn from(value: BootstrapArg) -> Self {
        match value {
            BootstrapArg::Off => Self::Disabled,
            BootstrapArg::Once => Self::Ephemeral,
            BootstrapArg::Persist => Self::Persist,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum PathCollisionArg {
    /// Refuse the transfer and name the colliding paths.
    #[default]
    Fail,
    /// Skip every colliding path, reporting each as a failure.
    Skip,
}

impl From<PathCollisionArg> for xsync_core::local::PathCollisionPolicy {
    fn from(value: PathCollisionArg) -> Self {
        match value {
            PathCollisionArg::Fail => Self::Fail,
            PathCollisionArg::Skip => Self::Skip,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum CloudFilesArg {
    #[default]
    Download,
    Skip,
    Error,
}

/// Provenance recorded at build time, so a bug report can name exactly what ran.
///
/// The semantic version is not stamped: it comes from `CARGO_PKG_VERSION`, read
/// by Cargo from the manifest, which is the single source of truth. A second
/// source could disagree with the tag the binary shipped under.
const BUILD_COMMIT: &str = env!("XSYNC_BUILD_COMMIT");
const BUILD_DATE: &str = env!("XSYNC_BUILD_DATE");
const BUILD_TARGET: &str = env!("XSYNC_BUILD_TARGET");

/// Short form, shown by `-V`.
static VERSION_SHORT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{} ({BUILD_COMMIT} {BUILD_DATE}) {BUILD_TARGET}",
        xsync_core::version()
    )
});

/// Long form, shown by `--version`.
///
/// Adds the capabilities this build actually has. zstd is unconditional since
/// story D0.2 removed the Windows gate, and saying so explicitly is the point:
/// the field report should state what the binary can do rather than leave it to
/// be inferred from the platform.
static VERSION_LONG: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let mut features = vec!["blake3", "zstd"];
    if cfg!(unix) {
        features.push("unix-permissions");
        features.push("non-utf8-paths");
    }
    if cfg!(target_os = "macos") {
        features.push("cloud-placeholder-detection");
    }
    format!(
        "{}\ncommit:    {BUILD_COMMIT}\nbuilt:     {BUILD_DATE}\ntarget:    {BUILD_TARGET}\nprotocol:  wire v{}, browse v2\nfeatures:  {}",
        *VERSION_SHORT,
        xsync_core::PROTOCOL_VERSION,
        features.join(", ")
    )
});

/// High-performance rsync replacement built on a parallel pipeline and BLAKE3.
#[derive(Debug, Parser)]
#[command(
    name = "xs",
    version = VERSION_SHORT.as_str(),
    long_version = VERSION_LONG.as_str(),
    about = "High-performance rsync replacement",
    long_about = "xsync is an rsync-compatible file synchronization tool with a parallel \
                  pipeline, BLAKE3 integrity, and workload-adaptive transfer strategies."
)]
#[allow(clippy::struct_excessive_bools)] // a CLI with many boolean flags is expected
struct Cli {
    /// Read named jobs from FILE instead of the default search path.
    ///
    /// A file named here must exist: falling back to the default when an
    /// explicitly named config is missing would run a different configuration
    /// than the one asked for.
    #[arg(long, value_name = "FILE")]
    config: Option<std::path::PathBuf>,

    /// Run the named job from the config file.
    ///
    /// Equivalent to passing the job's name as the only positional argument,
    /// but unambiguous when a directory of the same name also exists.
    #[arg(long, value_name = "NAME")]
    job: Option<String>,

    /// List the jobs defined in the config file and exit.
    #[arg(long)]
    list_jobs: bool,

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

    /// What to do when two source paths name one file on the destination.
    ///
    /// Case-insensitive and Unicode-normalization-insensitive destinations such
    /// as APFS and NTFS treat `Readme.md`/`readme.md`, and the NFC and NFD forms
    /// of one name, as the same file. Publishing both keeps only one.
    #[arg(long, value_enum, value_name = "POLICY", default_value_t = PathCollisionArg::Fail)]
    on_path_collision: PathCollisionArg,

    /// Number of local file workers, 1..=64 (default: one per logical core).
    ///
    /// Independent of `--streams`, which controls remote transport sessions.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=64))]
    local_workers: Option<u16>,

    /// Disable the local directory-clone fast path (benchmarking only).
    #[arg(long, hide = true)]
    no_directory_clone: bool,

    /// Delete extraneous files from the destination after a successful transfer.
    #[arg(long)]
    delete: bool,

    /// Exclude files matching the GLOB pattern (repeatable; matched against the relative path).
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Include files matching the GLOB pattern, overriding a later exclude.
    ///
    /// Rules are evaluated in the order written on the command line and the
    /// first that matches decides. Unlike rsync, there is no need to add
    /// `--include '*/'`: a directory is walked automatically whenever an
    /// include rule could match something inside it.
    #[arg(long, value_name = "GLOB")]
    include: Vec<String>,

    /// Read exclude patterns from FILE, one per line (repeatable).
    ///
    /// Blank lines and `#` comments are ignored. A line may start with `+ ` or
    /// `- ` to override the file's default action.
    #[arg(long, value_name = "FILE")]
    exclude_from: Vec<std::path::PathBuf>,

    /// Read include patterns from FILE, one per line (repeatable).
    #[arg(long, value_name = "FILE")]
    include_from: Vec<std::path::PathBuf>,

    /// Ignore per-directory `.xsyncignore` files.
    ///
    /// They are honoured by default. Their rules are always weaker than
    /// command-line rules, so a command line can override a tree's own opinion
    /// and never the other way round.
    #[arg(long)]
    no_ignore_file: bool,

    /// Print the rule that decided each excluded path.
    ///
    /// Most useful with `--dry-run`, where the whole plan can be inspected
    /// before anything is written.
    #[arg(long)]
    explain_filter: bool,

    /// Dry run: show what would be done without writing anything.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Classify by content hash (BLAKE3) instead of size+mtime.
    #[arg(long)]
    checksum: bool,

    /// Cloud placeholder policy: download, skip, or error.
    #[arg(long, value_enum, default_value_t = CloudFilesArg::Download)]
    cloud_files: CloudFilesArg,

    /// Upload a verified xsync binary when the remote has none.
    ///
    /// Off by default: this copies an executable to the remote and runs it, so
    /// it never happens unless asked for. The binary is checksummed on the
    /// remote before execution, is written only under the invoking user's home
    /// directory, and never requires root. `once` removes it afterwards;
    /// `persist` leaves it for later runs.
    #[arg(long, value_enum, default_value_t = BootstrapArg::Off)]
    bootstrap: BootstrapArg,

    /// Re-read every written file from disk and verify its BLAKE3 hash.
    #[arg(long)]
    paranoid: bool,

    /// Emit a machine-readable JSONL event stream instead of progress bars.
    #[arg(long)]
    progress_json: bool,

    /// Print the roff man page to stdout and exit.
    ///
    /// Generated from the parser, like the completions, so packaging cannot
    /// ship a man page describing flags that no longer exist.
    #[arg(long, hide = true)]
    man: bool,

    /// Print a shell completion script to stdout and exit.
    ///
    /// Generated from the parser rather than maintained by hand, so completions
    /// cannot drift from the flags they complete.
    #[arg(long, value_name = "SHELL", value_enum)]
    completions: Option<clap_complete::Shell>,

    /// Append structured failure records to FILE, or to stderr when FILE is `-`.
    ///
    /// Independent of `--progress-json`: failures are captured during ordinary
    /// human-readable runs too, and the file survives a terminal nobody was
    /// watching. Records append rather than truncate, so a post-mortem is not
    /// destroyed by the next run. When a remote is involved, the far end's
    /// records are relayed into the same log, tagged `"origin":"server"`.
    #[arg(long, value_name = "FILE")]
    log_json: Option<String>,

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

    /// Source path, or the name of a saved job. Either side may be `[user@]host:path`.
    #[arg(
        value_name = "SRC",
        required_unless_present_any = ["server", "completions", "man", "job", "list_jobs"]
    )]
    src: Option<String>,

    /// Destination path. Either side may be `[user@]host:path`.
    ///
    /// Not required when SRC names a saved job, which supplies both endpoints.
    #[arg(value_name = "DEST")]
    dest: Option<String>,
}

fn main() -> std::process::ExitCode {
    // Parse through `ArgMatches` rather than `Cli::parse()` so the merge below
    // can tell a flag the user typed from one that merely holds its default.
    // Without that distinction "CLI overrides config" is undecidable for every
    // flag that has a default, which is most of them.
    let matches = match Cli::command().try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let _ = error.print();
            return if error.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let mut cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(2);
        }
    };

    if cli.man {
        let command = <Cli as clap::CommandFactory>::command();
        if clap_mangen::Man::new(command)
            .render(&mut std::io::stdout())
            .is_err()
        {
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    if let Some(shell) = cli.completions {
        let mut command = <Cli as clap::CommandFactory>::command();
        clap_complete::generate(shell, &mut command, "xs", &mut std::io::stdout());
        return ExitCode::SUCCESS;
    }

    match resolve_job(&mut cli, &matches) {
        Ok(JobOutcome::Proceed) => {}
        Ok(JobOutcome::Listed) => return ExitCode::SUCCESS,
        Err(error) => {
            report_fatal(&cli, &error);
            return ExitCode::FAILURE;
        }
    }

    // Configure logging after job resolution so job-level log_json settings
    // are active before any transfer setup can fail.
    if let Some(spec) = cli.log_json.as_deref() {
        if let Err(error) = xsync_core::faillog::enable(spec) {
            eprintln!("xs: {error}");
            return ExitCode::FAILURE;
        }
    }
    xsync_core::server::configure_remote_logging(
        cli.log_json.is_some() || cli.progress_json,
        cli.progress_json,
    );

    match run(&cli, &matches) {
        Ok(RunOutcome::Complete) => ExitCode::SUCCESS,
        Ok(RunOutcome::Partial) => ExitCode::from(xsync_core::local::PARTIAL_FAILURE_EXIT_CODE),
        Err(error) => {
            report_fatal(&cli, &error);
            ExitCode::FAILURE
        }
    }
}

/// Report the error that ended the run.
///
/// Previously this was only ever `eprintln!("xs: {e}")`, so the single most
/// important event -- the thing that killed the run -- was the one event absent
/// from the `--progress-json` stream. It now reaches every configured sink.
fn report_fatal(cli: &Cli, error: &CliError) {
    let message = error.to_string();
    let authority = remote_authority(cli);
    // `xs --server` runs through this same main, so the origin has to come from
    // the role this process is playing. Hard-coding Client made the remote's own
    // fatal arrive at the client's log labelled as client-origin, which is the
    // one thing the field exists to distinguish.
    let origin = if cli.server {
        xsync_core::faillog::Origin::Server
    } else {
        xsync_core::faillog::Origin::Client
    };
    let record = xsync_core::faillog::Record {
        severity: xsync_core::faillog::Severity::Fatal,
        origin,
        kind: error.kind(),
        path: None,
        host: authority.as_deref(),
        message: &message,
    };
    xsync_core::faillog::write(&record);
    if cli.progress_json {
        println!("{}", xsync_core::faillog::render(&record));
    }
    // Keep the human line: stderr is where an operator looks, and a structured
    // sink is an addition to that rather than a replacement. The exception is a
    // server already writing records to its stderr -- there the plain line is
    // relayed alongside the record that already carries it, so it is pure
    // duplication in the client's output.
    let structured_to_stderr = cli.server && cli.log_json.as_deref() == Some("-");
    if !structured_to_stderr {
        eprintln!("xs: {message}");
    }
}

/// The remote authority this run concerns, when either side is remote.
fn remote_authority(cli: &Cli) -> Option<String> {
    for spec in [cli.src.as_deref(), cli.dest.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Ok(parsed) = xsync_core::path::parse(spec) {
            if let Some(authority) = parsed.authority() {
                return Some(authority);
            }
        }
    }
    None
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
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Filter(#[from] xsync_core::filter::FilterError),
    #[error("{0}")]
    Usage(String),
}

impl CliError {
    /// Stable machine-readable family, for consumers routing on failures.
    ///
    /// Server errors delegate so the client and the remote report the same
    /// vocabulary for the same condition: a consumer should not have to learn
    /// two names for one transport failure depending on which end noticed it.
    fn kind(&self) -> &'static str {
        match self {
            Self::Path(_) => "path",
            Self::Local(_) => "local",
            Self::Server(error) => error.kind(),
            Self::Rsync(_) => "rsync",
            Self::Transport(_) => "transport",
            Self::Config(_) => "config",
            Self::Filter(_) => "filter",
            Self::Usage(_) => "usage",
        }
    }
}

/// What job resolution decided should happen next.
#[derive(Debug)]
enum JobOutcome {
    /// Carry on with the transfer described by the (possibly merged) `Cli`.
    Proceed,
    /// `--list-jobs` printed its listing; there is nothing left to do.
    Listed,
}

/// Resolve `--job`/`--list-jobs`/a bare job name and merge the job into `cli`.
///
/// Config is only read when it could matter. A plain `xs SRC DEST` never opens
/// a file, so a broken config cannot break a run that does not use it.
fn resolve_job(cli: &mut Cli, matches: &ArgMatches) -> Result<JobOutcome, CliError> {
    if cli.server {
        return Ok(JobOutcome::Proceed);
    }

    if cli.list_jobs {
        list_jobs(cli.config.as_deref())?;
        return Ok(JobOutcome::Listed);
    }

    if cli.job.is_some() && (cli.src.is_some() || cli.dest.is_some()) {
        return Err(CliError::Usage(
            "--job cannot be combined with explicit SRC or DEST positional arguments".to_owned(),
        ));
    }

    // Three ways to name a job, in decreasing explicitness.
    let name = if let Some(name) = cli.job.clone() {
        name
    } else if cli.dest.is_none() {
        // A single positional. It is a job name only if a config defines it,
        // and only if nothing on disk answers to the same name.
        let Some(candidate) = cli.src.clone() else {
            return Ok(JobOutcome::Proceed);
        };
        let Some(loaded) = config::load(cli.config.as_deref())? else {
            return Err(CliError::Usage(format!(
                "expected SRC and DEST, but only '{candidate}' was given, and there is no \
                 config file defining it as a job (looked at {})",
                config::search_path()
                    .iter()
                    .map(|path| format!("'{}'", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if !loaded.config.jobs.contains_key(&candidate) {
            return Err(CliError::Usage(format!(
                "expected SRC and DEST, but only '{candidate}' was given, and '{}' defines no \
                 job by that name{}",
                loaded.path.display(),
                if loaded.config.jobs.is_empty() {
                    String::new()
                } else {
                    format!(
                        " (known jobs: {})",
                        loaded
                            .config
                            .jobs
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            )));
        }
        if std::path::Path::new(&candidate).exists() {
            // Choosing for the user here could copy the wrong tree.
            return Err(config::ConfigError::AmbiguousJob { name: candidate }.into());
        }
        candidate
    } else {
        return Ok(JobOutcome::Proceed);
    };

    let Some(loaded) = config::load(cli.config.as_deref())? else {
        return Err(config::ConfigError::NoConfig {
            searched: config::search_path(),
        }
        .into());
    };
    let job = loaded.config.job(&name)?.clone();
    merge_job(cli, matches, &job);
    Ok(JobOutcome::Proceed)
}

/// Apply `job` to `cli`, leaving every value the user typed alone.
///
/// Precedence is flag > job > built-in default. Note the one asymmetry, which
/// is documented rather than hidden: a boolean the job turns on cannot be
/// turned off from the command line, because there is no `--no-delete`. A job
/// that sets `delete` is a job that always deletes.
fn merge_job(cli: &mut Cli, matches: &ArgMatches, job: &config::Job) {
    let typed = |id: &str| {
        matches!(
            matches.value_source(id),
            Some(clap::parser::ValueSource::CommandLine)
        )
    };

    cli.src = Some(config::expand_home(&job.src));
    cli.dest = Some(config::expand_home(&job.dest));

    if !typed("exclude") && !job.exclude.is_empty() {
        cli.exclude.clone_from(&job.exclude);
    }
    if !typed("delete") {
        cli.delete = job.delete.unwrap_or(cli.delete);
    }
    if !typed("checksum") {
        cli.checksum = job.checksum.unwrap_or(cli.checksum);
    }
    if !typed("paranoid") {
        cli.paranoid = job.paranoid.unwrap_or(cli.paranoid);
    }
    if !typed("no_compress") {
        cli.no_compress = job.no_compress.unwrap_or(cli.no_compress);
    }
    if !typed("quiet") {
        cli.quiet = job.quiet.unwrap_or(cli.quiet);
    }
    if !typed("compress_level") && job.compress_level.is_some() {
        cli.compress_level = job.compress_level;
    }
    if !typed("streams") && job.streams.is_some() {
        cli.streams = job.streams;
    }
    if !typed("local_workers") && job.local_workers.is_some() {
        cli.local_workers = job.local_workers;
    }
    if !typed("rsh") && job.rsh.is_some() {
        cli.rsh.clone_from(&job.rsh);
    }
    if !typed("log_json") && job.log_json.is_some() {
        cli.log_json.clone_from(&job.log_json);
    }
    // The enum fields are validated at load time, so an unparseable value here
    // is impossible rather than silently ignored.
    if !typed("transport") {
        if let Some(value) = job.transport.as_deref() {
            cli.transport = match value {
                "xsync" => TransportArg::Xsync,
                "rsync" => TransportArg::Rsync,
                _ => TransportArg::Auto,
            };
        }
    }
    if !typed("cloud_files") {
        if let Some(value) = job.cloud_files.as_deref() {
            cli.cloud_files = match value {
                "skip" => CloudFilesArg::Skip,
                "error" => CloudFilesArg::Error,
                _ => CloudFilesArg::Download,
            };
        }
    }
    if !typed("bootstrap") {
        if let Some(value) = job.bootstrap.as_deref() {
            cli.bootstrap = match value {
                "once" => BootstrapArg::Once,
                "persist" => BootstrapArg::Persist,
                _ => BootstrapArg::Off,
            };
        }
    }
    if !typed("on_path_collision") {
        if let Some(value) = job.on_path_collision.as_deref() {
            cli.on_path_collision = match value {
                "skip" => PathCollisionArg::Skip,
                _ => PathCollisionArg::Fail,
            };
        }
    }
}

/// Print every defined job with its endpoints.
fn list_jobs(explicit: Option<&std::path::Path>) -> Result<(), CliError> {
    let Some(loaded) = config::load(explicit)? else {
        return Err(config::ConfigError::NoConfig {
            searched: config::search_path(),
        }
        .into());
    };
    if loaded.config.jobs.is_empty() {
        println!("{}: no jobs defined", loaded.path.display());
        return Ok(());
    }
    println!("{}:", loaded.path.display());
    for (name, job) in &loaded.config.jobs {
        println!("  {name}");
        println!("    {} -> {}", job.src, job.dest);
        if let Some(description) = &job.description {
            println!("    {description}");
        }
        let mut flags = Vec::new();
        if job.delete == Some(true) {
            flags.push("delete".to_owned());
        }
        if job.checksum == Some(true) {
            flags.push("checksum".to_owned());
        }
        if job.paranoid == Some(true) {
            flags.push("paranoid".to_owned());
        }
        if !job.exclude.is_empty() {
            flags.push(format!("{} exclude pattern(s)", job.exclude.len()));
        }
        if !flags.is_empty() {
            println!("    {}", flags.join(", "));
        }
    }
    Ok(())
}

/// Refuse, or warn about, the parts of a filter a remote peer cannot honour.
///
/// The v1 wire carries a flat list of exclude patterns and nothing else. That
/// is enough for `--exclude` and `--exclude-from`, and not enough for include
/// rules, whose whole meaning is their position relative to the excludes.
/// Sending the excludes alone would transfer a *different, larger* set of files
/// than the user asked for, silently — so it is refused instead.
fn reconcile_remote_filter(
    filter: &xsync_core::filter::FilterSet,
    source_is_remote: bool,
    quiet: bool,
) -> Result<(), CliError> {
    if filter.has_includes() {
        return Err(CliError::Usage(
            "--include is not supported for remote transfers yet: the v1 wire carries only \
             exclude patterns, and sending those alone would transfer more than you asked \
             for. Use --exclude/--exclude-from, or run xsync on the remote host so both \
             ends of the filter are local."
                .to_owned(),
        ));
    }
    if source_is_remote && filter.honours_ignore_files() && !quiet {
        // Not fatal: unlike an include rule, an unseen ignore file cannot make
        // the transfer wider than the explicit rules already allow. But the
        // user asked for behaviour they are not getting, so say so.
        eprintln!(
            "xs: note: per-directory .xsyncignore files are not honoured when the source is \
             remote; only --exclude/--exclude-from rules apply"
        );
    }
    Ok(())
}

/// Assemble the filter in the order the rules were written.
///
/// clap groups repeated occurrences by flag, so `--include a --exclude b` and
/// `--exclude b --include a` arrive identically. First-match-wins is meaningless
/// under that, so the original order is recovered from `ArgMatches::indices_of`
/// and the four sources are merged back into one sequence.
///
/// `--include-from`/`--exclude-from` files expand in place, so a file's rules sit
/// exactly where the flag appeared relative to the others.
fn build_filter<'a>(
    cli: &'a Cli,
    matches: &ArgMatches,
) -> Result<xsync_core::filter::FilterSet, CliError> {
    use xsync_core::filter::{rules_from_file, Action, Origin, Rule};

    // (command-line index, how to turn it into rules)
    enum Source<'a> {
        Pattern(Action, &'a str),
        File(Action, &'a std::path::Path),
    }

    let mut sources: Vec<(usize, Source<'a>)> = Vec::new();
    for (id, action, values) in [
        ("include", Action::Include, &cli.include),
        ("exclude", Action::Exclude, &cli.exclude),
    ] {
        let typed_count = matches.indices_of(id).map_or(0, |indices| {
            for (index, value) in indices.zip(values) {
                sources.push((index, Source::Pattern(action, value.as_str())));
            }
            values.len()
        });
        // Config-derived job excludes are appended after clap parsing and have
        // no ArgMatches indices. Put them after command-line rules so explicit
        // CLI rules retain precedence.
        for (offset, value) in values.iter().enumerate().skip(typed_count) {
            let rank = values.len().saturating_sub(1).saturating_sub(offset);
            sources.push((usize::MAX - rank, Source::Pattern(action, value.as_str())));
        }
    }
    for (id, action, values) in [
        ("include_from", Action::Include, &cli.include_from),
        ("exclude_from", Action::Exclude, &cli.exclude_from),
    ] {
        if let Some(indices) = matches.indices_of(id) {
            for (index, value) in indices.zip(values) {
                sources.push((index, Source::File(action, value.as_path())));
            }
        }
    }

    sources.sort_by_key(|(index, _)| *index);

    let mut rules: Vec<Rule> = Vec::new();
    for (_, source) in sources {
        match source {
            Source::Pattern(action, pattern) => {
                rules.push(Rule::new(action, pattern, Origin::CommandLine)?);
            }
            Source::File(action, path) => rules.extend(rules_from_file(path, action)?),
        }
    }

    Ok(xsync_core::filter::FilterSet::from_rules(rules).with_ignore_files(!cli.no_ignore_file))
}

/// Run the CLI command.
#[allow(clippy::too_many_lines)]
fn run(cli: &Cli, matches: &ArgMatches) -> Result<RunOutcome, CliError> {
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

    // A filter must reach whichever end does the walking, or it silently means
    // something different than what was asked for.
    let mut filter = build_filter(cli, matches)?;
    let remote = src.is_remote() || dest.is_remote();
    if remote {
        reconcile_remote_filter(&filter, src.is_remote(), cli.quiet)?;
        if src.is_remote() {
            // The remote source is walked by the far side, which never sees the
            // tree's own ignore files.
            filter = filter.with_ignore_files(false);
        }
    }
    // Only the exclude patterns cross the v1 wire, so `--exclude-from` rules
    // have to be folded into that list; leaving it as the raw `--exclude`
    // values would drop a rules file on every remote transfer without a word.
    let wire_excludes: Vec<String> = filter
        .rules()
        .iter()
        .filter(|rule| rule.action == xsync_core::filter::Action::Exclude)
        .map(|rule| rule.pattern.clone())
        .collect();

    let mut options = xsync_core::local::LocalSyncOptions {
        streams: usize::from(cli.streams.unwrap_or(xsync_core::DEFAULT_REMOTE_STREAMS)),
        directory_clones: !cli.no_directory_clone,
        on_path_collision: cli.on_path_collision.into(),
        dry_run: cli.dry_run,
        delete: cli.delete,
        checksum: cli.checksum,
        bootstrap: cli.bootstrap.into(),
        cloud_files: match cli.cloud_files {
            CloudFilesArg::Download => xsync_core::local::CloudFilesPolicy::Download,
            CloudFilesArg::Skip => xsync_core::local::CloudFilesPolicy::Skip,
            CloudFilesArg::Error => xsync_core::local::CloudFilesPolicy::Error,
        },
        paranoid: cli.paranoid,
        exclude_patterns: wire_excludes,
        filter: Some(filter),
        explain_filter: cli.explain_filter,
        compress: !cli.no_compress,
        compress_level: cli.compress_level.unwrap_or(3),
        ..xsync_core::local::LocalSyncOptions::default()
    };
    if let Some(workers) = cli.local_workers {
        options.local_workers = usize::from(workers);
    }
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

    // An ephemeral bootstrap leaves nothing behind. This runs after the
    // transfer rather than on drop so a failure to clean up can be reported
    // without turning a completed transfer into a failed one.
    if cli.bootstrap == BootstrapArg::Once {
        if let Some(host) = src_authority.as_deref().or(dest_authority.as_deref()) {
            if let Some(remote_path) =
                xsync_core::server::bootstrapped_program(cli.rsh.as_deref(), host)
            {
                deprovision_remote(cli, host, &remote_path, quiet);
            }
        }
    }

    Ok(if report.partial_failure() {
        RunOutcome::Partial
    } else {
        RunOutcome::Complete
    })
}

/// Remove a binary uploaded for this run only.
fn deprovision_remote(cli: &Cli, host: &str, remote_path: &str, quiet: bool) {
    let rsh = cli.rsh.as_deref();
    let shell = xsync_core::server::learned_remote_shell(rsh, host);
    if let Err(error) = xsync_core::bootstrap::remove_remote(rsh, host, shell, remote_path) {
        // The transfer already succeeded; a leftover file is worth reporting
        // but is not a reason to fail the run.
        if !quiet {
            eprintln!(
                "warning: could not remove the bootstrapped binary at {remote_path}: {error}"
            );
        }
    } else if !quiet {
        eprintln!("bootstrap: removed {remote_path}");
    }
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
    // Per-entry failures reach the failure log regardless of the display mode,
    // so a plain human-readable run still leaves a machine-readable record of
    // what went wrong.
    if xsync_core::faillog::is_enabled() {
        match &event {
            xsync_core::local::LocalEvent::Failed { path, message } => {
                xsync_core::faillog::write(&xsync_core::faillog::Record {
                    severity: xsync_core::faillog::Severity::Failed,
                    origin: xsync_core::faillog::Origin::Client,
                    kind: "entry",
                    path: Some(path),
                    host: None,
                    message,
                });
            }
            xsync_core::local::LocalEvent::Warning { path, message } => {
                xsync_core::faillog::write(&xsync_core::faillog::Record {
                    severity: xsync_core::faillog::Severity::Info,
                    origin: xsync_core::faillog::Origin::Client,
                    kind: "warning",
                    path: Some(path),
                    host: None,
                    message,
                });
            }
            _ => {}
        }
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
            detection_performed,
        } => {
            if detection_performed {
                println!(
                    "cloud placeholders: {files} file(s), {bytes} bytes (detection available: {detection_available})"
                );
            } else {
                println!(
                    "cloud placeholders: not inspected (detection available: {detection_available})"
                );
            }
        }
        xsync_core::local::LocalEvent::Action { path, action } => {
            println!("{action} {path}");
        }
        xsync_core::local::LocalEvent::FilterDecision { path, reason, .. } => {
            println!("filtered {path}: {reason}");
        }
        xsync_core::local::LocalEvent::Warning { path, message } => {
            if path.is_empty() {
                println!("warning: {message}");
            } else {
                println!("warning: {path}: {message}");
            }
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
        LocalEvent::FilterDecision { .. } => "filter-decision",
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
        LocalEvent::FilterDecision {
            path,
            included,
            reason,
        } => serde_json::json!({
            "event": "filter-decision",
            "path": path,
            "included": included,
            "reason": reason,
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
            detection_performed,
        } => serde_json::json!({
            "event": "cloud_placeholders",
            "files": files,
            "bytes": bytes,
            "detection_available": detection_available,
            "detection_performed": detection_performed,
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
            dropped_metadata,
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
            "dropped_hardlinked_files": dropped_metadata.hardlinked,
            "dropped_hardlink_extra_bytes": dropped_metadata.hardlink_extra_bytes,
            "dropped_xattr_entries": dropped_metadata.with_xattrs,
            "foreign_owner_entries": dropped_metadata.foreign_owner,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

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

    /// Parse into both the matches and the struct, the way `main` does.
    fn parse_with_matches(args: &[&str]) -> (Cli, clap::ArgMatches) {
        let matches = Cli::command().try_get_matches_from(args).unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();
        (cli, matches)
    }

    fn job_fixture() -> config::Job {
        config::Job {
            src: "/from/".to_owned(),
            dest: "/to".to_owned(),
            exclude: vec!["*.tmp".to_owned()],
            delete: Some(true),
            checksum: Some(true),
            compress_level: Some(9),
            streams: Some(4),
            transport: Some("rsync".to_owned()),
            ..config::Job::default()
        }
    }

    #[test]
    fn a_job_supplies_values_the_command_line_left_alone() {
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j"]);
        merge_job(&mut cli, &matches, &job_fixture());

        assert_eq!(cli.src.as_deref(), Some("/from/"));
        assert_eq!(cli.dest.as_deref(), Some("/to"));
        assert_eq!(cli.exclude, vec!["*.tmp".to_owned()]);
        assert!(cli.delete);
        assert!(cli.checksum);
        assert_eq!(cli.compress_level, Some(9));
        assert_eq!(cli.streams, Some(4));
        assert_eq!(cli.transport, TransportArg::Rsync);
    }

    #[test]
    fn job_excludes_reach_the_filter() {
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j"]);
        merge_job(&mut cli, &matches, &job_fixture());
        let filter = build_filter(&cli, &matches).unwrap();
        assert!(!filter.decide("scratch.tmp").is_included());
        assert!(filter.decide("keep.txt").is_included());
    }

    #[test]
    fn job_rejects_explicit_positional_paths() {
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j", "src", "dest"]);
        assert!(resolve_job(&mut cli, &matches).is_err());
    }

    #[test]
    fn an_explicit_flag_beats_the_job() {
        let (mut cli, matches) = parse_with_matches(&[
            "xs",
            "--job",
            "j",
            "--exclude",
            "*.log",
            "--compress-level",
            "1",
            "--streams",
            "2",
            "--transport",
            "xsync",
        ]);
        merge_job(&mut cli, &matches, &job_fixture());

        assert_eq!(
            cli.exclude,
            vec!["*.log".to_owned()],
            "flag replaces, never merges"
        );
        assert_eq!(cli.compress_level, Some(1));
        assert_eq!(cli.streams, Some(2));
        assert_eq!(cli.transport, TransportArg::Xsync);
    }

    #[test]
    fn a_flag_typed_at_its_default_value_still_beats_the_job() {
        // The reason resolution goes through `ArgMatches`: `--transport auto`
        // is indistinguishable from an untouched default by value alone, and
        // treating it as absent would let the job silently win an argument the
        // user explicitly made.
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j", "--transport", "auto"]);
        merge_job(&mut cli, &matches, &job_fixture());
        assert_eq!(cli.transport, TransportArg::Auto);
    }

    #[test]
    fn a_job_that_says_nothing_leaves_the_defaults_alone() {
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j"]);
        let job = config::Job {
            src: "/from/".to_owned(),
            dest: "/to".to_owned(),
            ..config::Job::default()
        };
        merge_job(&mut cli, &matches, &job);

        assert!(!cli.delete);
        assert!(!cli.checksum);
        assert_eq!(cli.compress_level, None);
        assert_eq!(cli.streams, None);
        assert_eq!(cli.transport, TransportArg::Auto);
        assert!(cli.exclude.is_empty());
    }

    #[test]
    fn dry_run_survives_a_job() {
        // The AC's whole point: a saved job must be inspectable before it runs.
        let (mut cli, matches) = parse_with_matches(&["xs", "-n", "--job", "j"]);
        merge_job(&mut cli, &matches, &job_fixture());
        assert!(cli.dry_run);
        assert_eq!(cli.src.as_deref(), Some("/from/"));
    }

    #[test]
    fn a_lone_positional_that_is_not_a_job_names_the_jobs_that_are() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jobs.photos]\nsrc = \"/a/\"\ndest = \"/b\"\n").unwrap();

        let (mut cli, matches) = parse_with_matches(&["xs", "no-such-job"]);
        cli.config = Some(path);
        let error = resolve_job(&mut cli, &matches).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("no-such-job"), "{message}");
        assert!(
            message.contains("photos"),
            "names what does exist: {message}"
        );
        assert_eq!(error.kind(), "usage");
    }

    #[test]
    fn an_explicitly_named_config_that_is_missing_is_fatal() {
        // Falling back to the default search path here would run a different
        // configuration than the one the user asked for.
        let (mut cli, matches) = parse_with_matches(&["xs", "--job", "j"]);
        cli.config = Some(std::path::PathBuf::from(
            "/nonexistent/xsync-test-config.toml",
        ));
        let error = resolve_job(&mut cli, &matches).unwrap_err();
        assert_eq!(error.kind(), "config");
        assert!(
            error.to_string().contains("xsync-test-config.toml"),
            "{error}"
        );
    }

    #[test]
    fn filter_rules_keep_the_order_they_were_typed_in() {
        use xsync_core::filter::Action;

        // clap groups repeated occurrences by flag, so without index recovery
        // these two command lines would produce identical filters — and
        // first-match-wins would mean nothing.
        let (cli, matches) =
            parse_with_matches(&["xs", "--include", "a", "--exclude", "b", "s", "d"]);
        let rules = build_filter(&cli, &matches).unwrap();
        let order: Vec<_> = rules
            .rules()
            .iter()
            .map(|rule| (rule.action, rule.pattern.as_str()))
            .collect();
        assert_eq!(order, vec![(Action::Include, "a"), (Action::Exclude, "b")]);

        let (cli, matches) =
            parse_with_matches(&["xs", "--exclude", "b", "--include", "a", "s", "d"]);
        let rules = build_filter(&cli, &matches).unwrap();
        let order: Vec<_> = rules
            .rules()
            .iter()
            .map(|rule| (rule.action, rule.pattern.as_str()))
            .collect();
        assert_eq!(order, vec![(Action::Exclude, "b"), (Action::Include, "a")]);
    }

    #[test]
    fn a_rules_file_expands_where_its_flag_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let path = path.to_str().unwrap();

        let (cli, matches) = parse_with_matches(&[
            "xs",
            "--include",
            "first",
            "--exclude-from",
            path,
            "--include",
            "last",
            "s",
            "d",
        ]);
        let patterns: Vec<_> = build_filter(&cli, &matches)
            .unwrap()
            .rules()
            .iter()
            .map(|rule| rule.pattern.clone())
            .collect();
        assert_eq!(patterns, vec!["first", "one", "two", "last"]);
    }

    #[test]
    fn ignore_files_are_honoured_unless_turned_off() {
        let (cli, matches) = parse_with_matches(&["xs", "s", "d"]);
        assert!(build_filter(&cli, &matches).unwrap().honours_ignore_files());

        let (cli, matches) = parse_with_matches(&["xs", "--no-ignore-file", "s", "d"]);
        assert!(!build_filter(&cli, &matches).unwrap().honours_ignore_files());
    }

    #[test]
    fn a_remote_transfer_refuses_include_rules_rather_than_approximating_them() {
        // Sending the excludes alone would transfer a larger set than asked for,
        // silently. Failing is the only safe answer until the wire can carry the
        // whole ruleset.
        let filter =
            xsync_core::filter::FilterSet::from_rules(vec![xsync_core::filter::Rule::new(
                xsync_core::filter::Action::Include,
                "keep/**",
                xsync_core::filter::Origin::CommandLine,
            )
            .unwrap()]);
        let error = reconcile_remote_filter(&filter, false, true).unwrap_err();
        assert_eq!(error.kind(), "usage");
        assert!(error.to_string().contains("--include"), "{error}");
    }

    #[test]
    fn a_remote_transfer_accepts_an_exclude_only_filter() {
        let filter = xsync_core::filter::from_exclude_patterns(&["*.tmp".to_owned()]).unwrap();
        assert!(reconcile_remote_filter(&filter, true, true).is_ok());
    }

    #[test]
    fn an_invalid_pattern_fails_before_any_transfer() {
        let (cli, matches) = parse_with_matches(&["xs", "--exclude", "a[", "s", "d"]);
        let error = build_filter(&cli, &matches).unwrap_err();
        assert_eq!(error.kind(), "filter");
    }

    #[test]
    fn parse_failure_is_not_a_panic() {
        // Unknown flag.
        assert_eq!(
            parse(&["xs", "--bogus", "a", "b"]).unwrap_err().kind(),
            ErrorKind::UnknownArgument
        );
        // Missing SRC and DEST both.
        assert_eq!(
            parse(&["xs"]).unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
        // A single positional is no longer a parse error: it may name a saved
        // job. It is rejected later, by resolution, with a message that says so.
        assert!(parse(&["xs", "only-a"]).is_ok());
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
