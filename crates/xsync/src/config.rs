//! Named jobs loaded from a TOML configuration file (Story V3.9).
//!
//! The problem this solves is that nobody retypes an exclude list and a
//! destination path daily; they write a shell alias, and from then on the
//! tool's own `--dry-run`, logging and error messages never see the real
//! configuration. A job moves that configuration inside xsync, where it can be
//! inspected, dry-run and reported on.
//!
//! Two rules shape everything here:
//!
//! - **A malformed config is fatal at startup, never partially applied.** The
//!   whole file is parsed and validated before a single value reaches the run.
//!   Unknown keys are errors rather than being ignored, because a silently
//!   ignored `excludes = [...]` (note the plural) is a backup that quietly
//!   copies files the user believed were excluded.
//! - **An explicit command-line flag always wins.** Precedence is
//!   flag > job > built-in default, decided by what the user actually typed
//!   rather than by whether a value happens to differ from its default.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The name of the config file inside its directory.
const FILE_NAME: &str = "config.toml";

/// Environment variable naming a config file outright.
const ENV_CONFIG: &str = "XSYNC_CONFIG";

/// A parsed configuration file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Named jobs, keyed by the name used on the command line.
    #[serde(default)]
    pub jobs: BTreeMap<String, Job>,
}

/// One saved source, destination and flag set.
///
/// Every field except `src` and `dest` is optional: an absent field means "no
/// opinion", which leaves the built-in default or the command line in charge.
/// Only the flags that make sense to save are representable — `--server`,
/// `--man` and `--completions` describe how the process is being driven, not
/// what transfer to perform.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// Source path. Either side may be `[user@]host:path`.
    pub src: String,
    /// Destination path. Either side may be `[user@]host:path`.
    pub dest: String,
    /// Human-readable note, shown by `--list-jobs`.
    pub description: Option<String>,
    /// Exclude globs, matched against the relative path.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Delete extraneous destination files after a successful transfer.
    pub delete: Option<bool>,
    /// Classify by content hash rather than size and mtime.
    pub checksum: Option<bool>,
    /// Re-read every written file and verify its hash.
    pub paranoid: Option<bool>,
    /// Disable zstd compression.
    pub no_compress: Option<bool>,
    /// zstd compression level.
    pub compress_level: Option<i32>,
    /// Parallel transport streams.
    pub streams: Option<u8>,
    /// Local file workers.
    pub local_workers: Option<u16>,
    /// Suppress non-error output.
    pub quiet: Option<bool>,
    /// Remote shell used to invoke the server.
    pub rsh: Option<String>,
    /// Append structured failure records to this file.
    pub log_json: Option<String>,
    /// Remote transport preference: `auto`, `xsync` or `rsync`.
    pub transport: Option<String>,
    /// Cloud placeholder policy: `download`, `skip` or `error`.
    pub cloud_files: Option<String>,
    /// Remote bootstrap policy: `off`, `once` or `persist`.
    pub bootstrap: Option<String>,
    /// Destination path collision policy: `fail` or `skip`.
    pub on_path_collision: Option<String>,
}

/// Why a configuration could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file named explicitly does not exist or cannot be read.
    #[error("cannot read config '{}': {source}", path.display())]
    Unreadable {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The file is not valid TOML, or contains keys this version does not know.
    #[error("invalid config '{}': {message}", path.display())]
    Invalid {
        /// The offending file.
        path: PathBuf,
        /// The parser's message, including a line and column where known.
        message: String,
    },
    /// A job's fields are individually well-formed but jointly unusable.
    #[error("invalid config '{}': job '{job}': {message}", path.display())]
    InvalidJob {
        /// The offending file.
        path: PathBuf,
        /// The job at fault.
        job: String,
        /// What is wrong with it.
        message: String,
    },
    /// The named job is not in the config.
    #[error("no job named '{job}'{}", suffix(known))]
    UnknownJob {
        /// The requested name.
        job: String,
        /// Names that do exist, for the suggestion.
        known: Vec<String>,
    },
    /// A job was requested but no config file was found.
    #[error("no config file found; looked at {}", searched.iter().map(|p| format!("'{}'", p.display())).collect::<Vec<_>>().join(", "))]
    NoConfig {
        /// Every location consulted, in order.
        searched: Vec<PathBuf>,
    },
    /// A bare argument names both a job and an existing path.
    ///
    /// Guessing here could copy the wrong tree, so it is refused instead.
    #[error(
        "'{name}' is both a saved job and an existing path; use '--job {name}' to run the job, \
         or './{name}' to name the path"
    )]
    AmbiguousJob {
        /// The ambiguous name.
        name: String,
    },
}

fn suffix(known: &[String]) -> String {
    if known.is_empty() {
        " (the config defines no jobs)".to_owned()
    } else {
        format!(" (known jobs: {})", known.join(", "))
    }
}

/// Where a config was loaded from, and what it contained.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The file the configuration came from.
    pub path: PathBuf,
    /// Its parsed contents.
    pub config: Config,
}

/// The locations consulted for a config file, most specific first.
///
/// `XSYNC_CONFIG` comes first so a test or a one-off run can redirect the
/// search without touching the user's real configuration.
#[must_use]
pub fn search_path() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(explicit) = std::env::var_os(ENV_CONFIG) {
        paths.push(PathBuf::from(explicit));
    }
    if let Some(dir) = config_directory() {
        paths.push(dir.join("xsync").join(FILE_NAME));
    }
    paths
}

#[cfg(windows)]
fn config_directory() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(not(windows))]
fn config_directory() -> Option<PathBuf> {
    // XDG on every Unix, macOS included. `~/Library/Application Support` is the
    // platform-native answer on macOS, but a config file is something people
    // edit by hand, and a path that is the same on the laptop and the server is
    // worth more here than platform purity.
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Some(xdg);
        }
    }
    home_directory().map(|home| home.join(".config"))
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| !home.as_os_str().is_empty())
}

/// Load the configuration, either from `explicit` or from the search path.
///
/// An explicit path that does not exist is an error; a searched path that does
/// not exist simply moves on to the next candidate. Returns `Ok(None)` when no
/// file was found at all, which is the ordinary case for a user who has never
/// written one.
///
/// # Errors
///
/// Returns [`ConfigError::Unreadable`] for an explicit file that cannot be
/// opened, and [`ConfigError::Invalid`] or [`ConfigError::InvalidJob`] for a
/// file that parses to something unusable.
pub fn load(explicit: Option<&Path>) -> Result<Option<Loaded>, ConfigError> {
    if let Some(path) = explicit {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Unreadable {
            path: path.to_path_buf(),
            source,
        })?;
        return Ok(Some(parse(path, &text)?));
    }
    for candidate in search_path() {
        match std::fs::read_to_string(&candidate) {
            Ok(text) => return Ok(Some(parse(&candidate, &text)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                // A file that exists but cannot be read is a real problem worth
                // reporting, not a reason to fall through to the next candidate
                // and run with a configuration the user did not intend.
                return Err(ConfigError::Unreadable {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Ok(None)
}

/// Parse and validate a config file's text.
///
/// # Errors
///
/// Returns [`ConfigError::Invalid`] when the text is not valid TOML or carries
/// unknown keys, and [`ConfigError::InvalidJob`] when a job is well-formed TOML
/// but cannot describe a transfer.
pub fn parse(path: &Path, text: &str) -> Result<Loaded, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|error| ConfigError::Invalid {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    // Validate every job before any of them is usable: a config where the
    // second job is broken must not run the first.
    for (name, job) in &config.jobs {
        validate_job(path, name, job)?;
    }
    Ok(Loaded {
        path: path.to_path_buf(),
        config,
    })
}

fn validate_job(path: &Path, name: &str, job: &Job) -> Result<(), ConfigError> {
    let invalid = |message: String| ConfigError::InvalidJob {
        path: path.to_path_buf(),
        job: name.to_owned(),
        message,
    };
    if name.is_empty() {
        return Err(invalid("a job name cannot be empty".to_owned()));
    }
    if job.src.trim().is_empty() {
        return Err(invalid("'src' cannot be empty".to_owned()));
    }
    if job.dest.trim().is_empty() {
        return Err(invalid("'dest' cannot be empty".to_owned()));
    }
    // Reject unusable endpoints here rather than at transfer time, so that
    // `--list-jobs` and a dry run both refuse a config that could never work.
    let src = xsync_core::path::parse(&expand_home(&job.src))
        .map_err(|error| invalid(format!("'src' is not a usable path: {error}")))?;
    let dest = xsync_core::path::parse(&expand_home(&job.dest))
        .map_err(|error| invalid(format!("'dest' is not a usable path: {error}")))?;
    xsync_core::path::validate_pair(&src, &dest)
        .map_err(|error| invalid(format!("'src' and 'dest' cannot be paired: {error}")))?;

    check_enum(
        &invalid,
        "transport",
        job.transport.as_deref(),
        &["auto", "xsync", "rsync"],
    )?;
    check_enum(
        &invalid,
        "cloud_files",
        job.cloud_files.as_deref(),
        &["download", "skip", "error"],
    )?;
    check_enum(
        &invalid,
        "bootstrap",
        job.bootstrap.as_deref(),
        &["off", "once", "persist"],
    )?;
    check_enum(
        &invalid,
        "on_path_collision",
        job.on_path_collision.as_deref(),
        &["fail", "skip"],
    )?;
    if let Some(streams) = job.streams {
        if !(1..=16).contains(&streams) {
            return Err(invalid(format!("'streams' must be 1..=16, got {streams}")));
        }
    }
    if let Some(workers) = job.local_workers {
        if !(1..=64).contains(&workers) {
            return Err(invalid(format!(
                "'local_workers' must be 1..=64, got {workers}"
            )));
        }
    }
    if let Some(level) = job.compress_level {
        if !(1..=22).contains(&level) {
            return Err(invalid(format!(
                "'compress_level' must be 1..=22, got {level}"
            )));
        }
    }
    Ok(())
}

fn check_enum(
    invalid: &impl Fn(String) -> ConfigError,
    field: &str,
    value: Option<&str>,
    allowed: &[&str],
) -> Result<(), ConfigError> {
    let Some(value) = value else {
        return Ok(());
    };
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(invalid(format!(
        "'{field}' must be one of {}, got '{value}'",
        allowed.join(", ")
    )))
}

/// Expand a leading `~/` against `$HOME`.
///
/// Only a leading `~/` is expanded, and only when it starts the whole string: a
/// remote spec is `host:~/path`, where the tilde belongs to the remote shell and
/// must survive untouched. Leaving it unexpanded locally would be worse than an
/// error, since it would silently create a directory literally named `~`.
#[must_use]
pub fn expand_home(spec: &str) -> String {
    let Some(rest) = spec.strip_prefix("~/") else {
        return spec.to_owned();
    };
    let Some(home) = home_directory() else {
        return spec.to_owned();
    };
    let mut expanded = home.to_string_lossy().into_owned();
    if !expanded.ends_with('/') {
        expanded.push('/');
    }
    expanded.push_str(rest);
    expanded
}

impl Config {
    /// Look a job up by name.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::UnknownJob`], listing the names that do exist.
    pub fn job(&self, name: &str) -> Result<&Job, ConfigError> {
        self.jobs.get(name).ok_or_else(|| ConfigError::UnknownJob {
            job: name.to_owned(),
            known: self.jobs.keys().cloned().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_text(text: &str) -> Result<Loaded, ConfigError> {
        parse(Path::new("/tmp/config.toml"), text)
    }

    #[test]
    fn a_minimal_job_parses() {
        let loaded = parse_text(
            r#"
            [jobs.backup]
            src = "/data/"
            dest = "/backup"
            "#,
        )
        .unwrap();
        let job = loaded.config.job("backup").unwrap();
        assert_eq!(job.src, "/data/");
        assert_eq!(job.dest, "/backup");
        assert!(job.exclude.is_empty());
        assert_eq!(job.delete, None, "an absent flag is an absent opinion");
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        // The plural is the realistic typo, and silently ignoring it would copy
        // files the user believed were excluded.
        let error = parse_text(
            r#"
            [jobs.backup]
            src = "/data/"
            dest = "/backup"
            excludes = ["*.tmp"]
            "#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("excludes"), "{message}");
    }

    #[test]
    fn an_unknown_top_level_table_is_refused() {
        let error = parse_text("[defaults]\ndelete = true\n").unwrap_err();
        assert!(error.to_string().contains("defaults"), "{error}");
    }

    #[test]
    fn a_broken_second_job_fails_the_whole_file() {
        // Never partial application: the good job must not become usable.
        let error = parse_text(
            r#"
            [jobs.good]
            src = "/data/"
            dest = "/backup"

            [jobs.bad]
            src = "/data/"
            dest = ""
            "#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("'bad'"), "{message}");
        assert!(message.contains("'dest' cannot be empty"), "{message}");
    }

    #[test]
    fn remote_to_remote_is_refused_at_load_time() {
        let error = parse_text(
            r#"
            [jobs.hop]
            src = "a.example:/data/"
            dest = "b.example:/backup"
            "#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot be paired"), "{error}");
    }

    #[test]
    fn out_of_range_numbers_are_refused_with_the_range() {
        let error = parse_text(
            r#"
            [jobs.j]
            src = "/a/"
            dest = "/b"
            streams = 99
            "#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("1..=16"), "{message}");
        assert!(message.contains("99"), "{message}");
    }

    #[test]
    fn a_misspelled_enum_value_names_the_alternatives() {
        let error = parse_text(
            r#"
            [jobs.j]
            src = "/a/"
            dest = "/b"
            transport = "rsyncd"
            "#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("auto, xsync, rsync"), "{message}");
    }

    #[test]
    fn an_unknown_job_lists_the_known_ones() {
        let loaded = parse_text(
            r#"
            [jobs.photos]
            src = "/a/"
            dest = "/b"

            [jobs.docs]
            src = "/c/"
            dest = "/d"
            "#,
        )
        .unwrap();
        let error = loaded.config.job("photo").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("docs"), "{message}");
        assert!(message.contains("photos"), "{message}");
    }

    #[test]
    fn an_empty_config_defines_no_jobs() {
        let loaded = parse_text("").unwrap();
        assert!(loaded.config.jobs.is_empty());
        let message = loaded.config.job("x").unwrap_err().to_string();
        assert!(message.contains("defines no jobs"), "{message}");
    }

    #[test]
    fn a_leading_tilde_expands_but_a_remote_tilde_does_not() {
        // SAFETY-adjacent: the remote form must survive untouched, because the
        // remote shell is what resolves it.
        let home = home_directory();
        let expanded = expand_home("~/Documents");
        if let Some(home) = home {
            // `$HOME` may itself be `/` -- it is under `cross`, whose test
            // container runs as root. `expand_home` collapses the separator;
            // the expectation has to do the same or it asserts `//Documents`.
            let base = home.to_string_lossy().into_owned();
            let separator = if base.ends_with('/') { "" } else { "/" };
            assert_eq!(expanded, format!("{base}{separator}Documents"));
        }
        assert_eq!(expand_home("mars:~/Documents"), "mars:~/Documents");
        assert_eq!(expand_home("/absolute/~/x"), "/absolute/~/x");
        assert_eq!(expand_home("~user/x"), "~user/x", "only '~/' is expanded");
    }
}
