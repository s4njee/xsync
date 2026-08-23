//! In-process local-to-local synchronization.
//!
//! Local transfers deliberately do not use protocol messages. Discovery and
//! planning remain metadata-only; worker threads read source bytes only after
//! a file has been classified for transfer.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::clone::{self, CloneKind};
use crate::planner::{try_plan, DestinationIndex, EntryPlan, IndexConfig, Plan, PlannerError};
use crate::scanner::{
    fingerprint_from_metadata, permission_mode, scan, EntryKind, FileEntry, ScanError,
};
use crate::sink::{Sink, SinkError, SymlinkTargetKind};
use crate::source::{SourceReadError, SourceReader};

/// Exit status used when a local job completed with per-entry failures.
pub const PARTIAL_FAILURE_EXIT_CODE: u8 = 23;

/// Default number of pending local file tasks per worker.
pub const DEFAULT_LOCAL_QUEUE_CAPACITY: usize = 2;

/// Data path used for one successful transfer event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMethod {
    /// A complete source directory was cloned/reflinked.
    DirectoryClone,
    /// One regular file was cloned/reflinked.
    FileClone,
    /// Bytes were read and written through the verified streaming path.
    ByteCopy,
}

/// Events emitted by a local transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEvent {
    /// The local pipeline has started. `streams` is reported for observability
    /// but does not configure local worker scheduling.
    Started {
        /// Number of local I/O workers.
        local_workers: usize,
        /// Requested remote stream count, ignored for this local route.
        streams: usize,
    },
    /// Metadata planning has completed.
    Planned {
        /// Number of regular files requiring transfer.
        files: usize,
        /// Logical bytes in those files at discovery time.
        bytes: u64,
    },
    /// A regular file was published successfully.
    Transferred {
        /// Destination-relative path.
        path: String,
        /// Logical bytes published.
        bytes: u64,
        /// Bytes physically moved through the streaming path. Clone paths
        /// report zero because they do not use byte streaming.
        physical_bytes: u64,
        /// Selected local data path.
        method: TransferMethod,
    },
    /// An unchanged regular file was not transferred.
    Skipped {
        /// Destination-relative path.
        path: String,
    },
    /// A non-fatal source or destination warning was recorded.
    Warning {
        /// Destination-relative path when available.
        path: String,
        /// Human-readable warning.
        message: String,
    },
    /// An entry failed while unrelated work continued.
    Failed {
        /// Destination-relative path when available.
        path: String,
        /// Human-readable failure.
        message: String,
    },
    /// An extraneous destination entry was removed after successful transfer.
    Deleted {
        /// Destination-relative path.
        path: String,
    },
    /// The local pipeline has finished.
    Finished {
        /// Number of files published.
        transferred_files: usize,
        /// Logical bytes published.
        transferred_bytes: u64,
        /// Number of unchanged files skipped.
        skipped_files: usize,
        /// Number of entries that failed.
        failed_entries: usize,
        /// Number of warnings emitted.
        warnings: usize,
        /// Bytes physically moved through the streaming path.
        physical_bytes: u64,
        /// Number of directory clone operations.
        directory_clones: usize,
        /// Number of file clone operations.
        file_clones: usize,
        /// Number of streaming byte-copy operations.
        byte_copies: usize,
        /// Number of local I/O workers.
        local_workers: usize,
        /// Requested remote stream count, ignored on this route.
        streams: usize,
        /// Whether the result maps to [`PARTIAL_FAILURE_EXIT_CODE`].
        partial_failure: bool,
    },
}

/// Options for an in-process local transfer.
#[derive(Debug, Clone)]
pub struct LocalSyncOptions {
    /// Number of local I/O workers. This is independent of `streams`.
    pub local_workers: usize,
    /// Requested remote stream count, retained only for event reporting.
    pub streams: usize,
    /// Capacity of the shared local file queue.
    pub queue_capacity: usize,
    /// Plan without mutating the destination.
    pub dry_run: bool,
    /// Remove destination-only entries after all transfers succeed.
    pub delete: bool,
    /// Re-read clone output and verify content hashes.
    pub paranoid: bool,
    /// Relative-path glob patterns that disable directory cloning and exclude
    /// matching source/destination entries.
    pub exclude_patterns: Vec<String>,
}

impl Default for LocalSyncOptions {
    fn default() -> Self {
        Self {
            local_workers: default_local_workers(),
            streams: 1,
            queue_capacity: DEFAULT_LOCAL_QUEUE_CAPACITY,
            dry_run: false,
            delete: false,
            paranoid: false,
            exclude_patterns: Vec::new(),
        }
    }
}

/// Summary of one local transfer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalSyncReport {
    /// Number of files published.
    pub transferred_files: usize,
    /// Logical bytes published.
    pub transferred_bytes: u64,
    /// Bytes physically moved through the streaming path.
    pub physical_bytes: u64,
    /// Number of unchanged files skipped.
    pub skipped_files: usize,
    /// Number of entries that failed.
    pub failed_entries: usize,
    /// Number of warnings emitted.
    pub warnings: usize,
    /// Number of deleted destination entries.
    pub deleted_entries: usize,
    /// Number of complete directory clone operations.
    pub directory_clones: usize,
    /// Number of per-file clone operations.
    pub file_clones: usize,
    /// Number of verified streaming file copies.
    pub byte_copies: usize,
    /// Number of local I/O workers used.
    pub local_workers: usize,
    /// Requested remote stream count, ignored on this route.
    pub streams: usize,
}

impl LocalSyncReport {
    /// Whether the job completed with any per-entry failure.
    #[must_use]
    pub fn partial_failure(&self) -> bool {
        self.failed_entries != 0
    }
}

/// Errors that prevent a local job from safely reaching per-entry transfer.
#[derive(Debug, thiserror::Error)]
pub enum LocalSyncError {
    /// The source root could not be inspected.
    #[error("cannot inspect source root '{}': {source}", path.display())]
    SourceRoot {
        /// Source root path.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// The source root is not a supported filesystem object.
    #[error("unsupported source root '{path}' ({kind:?})")]
    UnsupportedSource {
        /// Source root path.
        path: PathBuf,
        /// Discovered object kind.
        kind: EntryKind,
    },
    /// A source or destination scan failed before transfer.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// Metadata planning failed before transfer.
    #[error(transparent)]
    Planning(#[from] PlannerError),
    /// Creating the destination sink failed before transfer.
    #[error(transparent)]
    Sink(#[from] SinkError),
    /// A directory fast-path clone could not be staged or verified.
    #[error(transparent)]
    Clone(#[from] clone::CloneError),
    /// A worker queue could not be drained safely.
    #[error("local worker queue disconnected after dispatching {dispatched} file(s)")]
    WorkerDisconnected {
        /// Number of tasks sent before the disconnect.
        dispatched: usize,
    },
    /// A local worker thread panicked.
    #[error("local transfer worker panicked")]
    WorkerPanicked,
    /// A scanned source path could not be mapped to the destination layout.
    #[error("source path '{path}' could not be mapped to the destination")]
    PathMapping {
        /// Source-relative path without a destination mapping.
        path: String,
    },
    /// A local option is invalid.
    #[error("{field} must be at least 1")]
    InvalidOption {
        /// Invalid option name.
        field: &'static str,
    },
    /// An exclude glob could not be compiled.
    #[error("invalid exclude pattern '{pattern}': {message}")]
    InvalidExclude {
        /// User-supplied pattern.
        pattern: String,
        /// Glob compiler error.
        message: String,
    },
}

/// Synchronize a local source to a local destination and emit events.
///
/// `source_trailing_slash` preserves the parsed rsync convention: directory
/// contents are copied into `destination` when true, while the source
/// directory itself is represented by `destination/<basename>` when false.
/// `destination_trailing_slash` makes a file source a child of a destination
/// directory even when that directory does not exist yet.
///
/// # Errors
/// Returns an error when discovery, planning, sink setup, or worker lifecycle
/// fails. Per-entry source races and write failures are reported as events and
/// leave the returned report in partial-failure state.
pub fn sync<F>(
    source: impl AsRef<Path>,
    source_trailing_slash: bool,
    destination: impl AsRef<Path>,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    mut emit: F,
) -> Result<LocalSyncReport, LocalSyncError>
where
    F: FnMut(LocalEvent),
{
    validate_options(options)?;
    if !options.dry_run {
        if let Some(report) = try_directory_fast_path(
            source.as_ref(),
            source_trailing_slash,
            destination.as_ref(),
            destination_trailing_slash,
            options,
            &mut emit,
        )? {
            return Ok(report);
        }
    }
    let prepared = prepare_transfer(
        source.as_ref(),
        source_trailing_slash,
        destination.as_ref(),
        destination_trailing_slash,
        options,
        &mut emit,
    )?;
    let PreparedTransfer {
        destination_sink,
        source_reader_root,
        source_root_entry,
        source_by_destination,
        plan,
    } = prepared;

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams: options.streams,
        ..LocalSyncReport::default()
    };
    report.skipped_files = plan.files.unchanged.len();
    for entry in &plan.files.unchanged {
        emit(LocalEvent::Skipped {
            path: entry.path.clone(),
        });
    }
    for entry in plan.other.new.iter().chain(&plan.other.changed) {
        record_failure(
            &mut report,
            &mut emit,
            entry.path.clone(),
            "unsupported filesystem object".to_owned(),
        );
    }

    if !options.dry_run {
        transfer_directories(&destination_sink, &plan.directories, &mut report, &mut emit);
        transfer_symlinks(
            &destination_sink,
            &source_reader_root,
            &plan.symlinks,
            &source_by_destination,
            &mut report,
            &mut emit,
        );
        transfer_files(
            &destination_sink,
            &source_reader_root,
            &plan.files,
            &source_by_destination,
            options,
            &mut report,
            &mut emit,
        )?;

        if let Some(root_entry) = source_root_entry {
            if let Err(error) = destination_sink.finish_root_directory(&root_entry) {
                record_failure(&mut report, &mut emit, String::from("."), error.to_string());
            }
        }
        finish_directories(&destination_sink, &plan.directories, &mut report, &mut emit);

        if options.delete && !report.partial_failure() {
            delete_extraneous(&destination_sink, &plan, &mut report, &mut emit);
        }
    }

    emit(LocalEvent::Finished {
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        directory_clones: report.directory_clones,
        file_clones: report.file_clones,
        byte_copies: report.byte_copies,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
    });
    Ok(report)
}

fn try_directory_fast_path(
    source: &Path,
    source_trailing_slash: bool,
    destination: &Path,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<Option<LocalSyncReport>, LocalSyncError> {
    if options.delete || !options.exclude_patterns.is_empty() {
        return Ok(None);
    }
    let source_metadata =
        fs::symlink_metadata(source).map_err(|source_error| LocalSyncError::SourceRoot {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let layout = Layout::new(
        source,
        EntryKind::Directory,
        source_trailing_slash,
        destination_trailing_slash,
        destination,
    );
    if layout.destination_root.exists() {
        return Ok(None);
    }
    let entries = collect_scan(source)?;
    let root = root_entry(source, EntryKind::Directory)?;
    let file_count = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .count();
    let logical_bytes = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| entry.size)
        .sum();
    let Some(outcome) = clone::try_clone_directory(
        source,
        &layout.destination_root,
        &root,
        &entries,
        options.paranoid,
    )?
    else {
        return Ok(None);
    };

    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams: options.streams,
    });
    emit(LocalEvent::Planned {
        files: file_count,
        bytes: logical_bytes,
    });
    let report = LocalSyncReport {
        transferred_files: file_count,
        transferred_bytes: logical_bytes,
        local_workers: options.local_workers,
        streams: options.streams,
        directory_clones: usize::from(outcome.kind == CloneKind::Directory),
        ..LocalSyncReport::default()
    };
    emit(LocalEvent::Transferred {
        path: ".".to_owned(),
        bytes: logical_bytes,
        physical_bytes: 0,
        method: TransferMethod::DirectoryClone,
    });
    emit(LocalEvent::Finished {
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        directory_clones: report.directory_clones,
        file_clones: report.file_clones,
        byte_copies: report.byte_copies,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
    });
    Ok(Some(report))
}

struct PreparedTransfer {
    destination_sink: Sink,
    source_reader_root: PathBuf,
    source_root_entry: Option<FileEntry>,
    source_by_destination: SourcePathMap,
    plan: Plan,
}

struct ExcludeMatcher {
    set: GlobSet,
}

impl ExcludeMatcher {
    fn new(patterns: &[String]) -> Result<Self, LocalSyncError> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = Glob::new(pattern).map_err(|error| LocalSyncError::InvalidExclude {
                pattern: pattern.clone(),
                message: error.to_string(),
            })?;
            builder.add(glob);
        }
        builder
            .build()
            .map(|set| Self { set })
            .map_err(|error| LocalSyncError::InvalidExclude {
                pattern: "<combined>".to_owned(),
                message: error.to_string(),
            })
    }

    fn matches(&self, path: &str) -> bool {
        if self.set.is_match(path) {
            return true;
        }
        let mut prefix = path;
        while let Some((ancestor, _)) = prefix.rsplit_once('/') {
            if self.set.is_match(ancestor) {
                return true;
            }
            prefix = ancestor;
        }
        false
    }
}

fn prepare_transfer(
    source: &Path,
    source_trailing_slash: bool,
    destination: &Path,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<PreparedTransfer, LocalSyncError> {
    let excludes = ExcludeMatcher::new(&options.exclude_patterns)?;
    let source_metadata =
        fs::symlink_metadata(source).map_err(|source_error| LocalSyncError::SourceRoot {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let source_kind = entry_kind(&source_metadata);
    if !matches!(
        source_kind,
        EntryKind::File | EntryKind::Directory | EntryKind::Symlink
    ) {
        return Err(LocalSyncError::UnsupportedSource {
            path: source.to_path_buf(),
            kind: source_kind,
        });
    }
    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams: options.streams,
    });

    let source_entries: Vec<_> = collect_scan(source)?
        .into_iter()
        .filter(|entry| !excludes.matches(&entry.path))
        .collect();
    let source_reader_root = source_reader_root(source, source_kind);
    let source_root_entry = (source_kind == EntryKind::Directory)
        .then(|| root_entry(source, source_kind))
        .transpose()?;
    let layout = Layout::new(
        source,
        source_kind,
        source_trailing_slash,
        destination_trailing_slash,
        destination,
    );
    let source_by_destination = layout.map_source_entries(&source_entries);
    let destination_sink = Sink::new(&layout.destination_root)?;
    let destination_entries = collect_scan(destination_sink.root())?
        .into_iter()
        .filter(|entry| {
            layout
                .direct_destination_name
                .as_ref()
                .is_none_or(|name| entry.path == *name)
                && !excludes.matches(&entry.path)
        });
    let mut destination_index = DestinationIndex::with_config(IndexConfig::default())?;
    for entry in destination_entries {
        destination_index.insert(entry)?;
    }
    let planned_source: Result<Vec<_>, _> = source_entries
        .iter()
        .map(|entry| {
            let mut planned = entry.clone();
            planned.path = source_by_destination
                .destination_for_source
                .get(&entry.path)
                .cloned()
                .ok_or_else(|| LocalSyncError::PathMapping {
                    path: entry.path.clone(),
                })?;
            Ok::<FileEntry, LocalSyncError>(planned)
        })
        .collect();
    let plan = try_plan(planned_source?, destination_index)?;
    let (planned_files, planned_bytes) = transfer_totals(&plan.files);
    emit(LocalEvent::Planned {
        files: planned_files,
        bytes: planned_bytes,
    });
    Ok(PreparedTransfer {
        destination_sink,
        source_reader_root,
        source_root_entry,
        source_by_destination,
        plan,
    })
}

fn validate_options(options: &LocalSyncOptions) -> Result<(), LocalSyncError> {
    if options.local_workers == 0 {
        return Err(LocalSyncError::InvalidOption {
            field: "local_workers",
        });
    }
    if options.streams == 0 {
        return Err(LocalSyncError::InvalidOption { field: "streams" });
    }
    if options.queue_capacity == 0 {
        return Err(LocalSyncError::InvalidOption {
            field: "queue_capacity",
        });
    }
    Ok(())
}

fn default_local_workers() -> usize {
    thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

struct Layout {
    destination_root: PathBuf,
    direct_destination_name: Option<String>,
    direct_source_name: Option<String>,
}

impl Layout {
    fn new(
        source: &Path,
        source_kind: EntryKind,
        source_trailing_slash: bool,
        destination_trailing_slash: bool,
        destination: &Path,
    ) -> Self {
        if source_kind == EntryKind::Directory {
            let destination_root = if source_trailing_slash {
                destination.to_path_buf()
            } else {
                destination.join(path_basename(source))
            };
            Self {
                destination_root,
                direct_destination_name: None,
                direct_source_name: None,
            }
        } else {
            let source_name = path_basename(source);
            let destination_is_directory = fs::symlink_metadata(destination)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                || source_trailing_slash
                || destination_trailing_slash;
            if destination_is_directory {
                Self {
                    destination_root: destination.to_path_buf(),
                    direct_destination_name: None,
                    direct_source_name: None,
                }
            } else {
                let destination_name = path_basename(destination);
                Self {
                    destination_root: destination
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf(),
                    direct_destination_name: Some(destination_name),
                    direct_source_name: Some(source_name),
                }
            }
        }
    }

    fn map_source_entries(&self, entries: &[FileEntry]) -> SourcePathMap {
        let mut destination_for_source = BTreeMap::new();
        let mut source_for_destination = BTreeMap::new();
        for entry in entries {
            let destination_path = if let Some(destination_name) = &self.direct_destination_name {
                let source_name = self
                    .direct_source_name
                    .as_deref()
                    .expect("direct destinations have a source name");
                if entry.path == source_name {
                    destination_name.clone()
                } else {
                    entry.path.clone()
                }
            } else {
                entry.path.clone()
            };
            destination_for_source.insert(entry.path.clone(), destination_path.clone());
            source_for_destination.insert(destination_path, entry.path.clone());
        }
        SourcePathMap {
            destination_for_source,
            source_for_destination,
        }
    }
}

struct SourcePathMap {
    destination_for_source: BTreeMap<String, String>,
    source_for_destination: BTreeMap<String, String>,
}

fn collect_scan(root: &Path) -> Result<Vec<FileEntry>, LocalSyncError> {
    let scan = scan(root)?;
    let mut entries = Vec::new();
    let mut first_error = None;
    for result in scan.entries() {
        match result {
            Ok(entry) => entries.push(entry),
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    scan.finish()?;
    if let Some(error) = first_error {
        return Err(error.into());
    }
    Ok(entries)
}

fn root_entry(path: &Path, kind: EntryKind) -> Result<FileEntry, LocalSyncError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| LocalSyncError::SourceRoot {
        path: path.to_path_buf(),
        source,
    })?;
    let mtime = metadata
        .modified()
        .map_err(|source| LocalSyncError::SourceRoot {
            path: path.to_path_buf(),
            source,
        })?;
    let fingerprint = fingerprint_from_metadata(&metadata, kind, mtime).map_err(|source| {
        LocalSyncError::SourceRoot {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(FileEntry {
        path: String::new(),
        kind,
        size: metadata.len(),
        mtime,
        mode: permission_mode(&metadata),
        fingerprint,
    })
}

fn source_reader_root(source: &Path, kind: EntryKind) -> PathBuf {
    if kind == EntryKind::Directory {
        source.to_path_buf()
    } else {
        source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("source")
        .to_owned()
}

fn entry_kind(metadata: &fs::Metadata) -> EntryKind {
    if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    }
}

fn transfer_totals(entries: &EntryPlan) -> (usize, u64) {
    let bytes = entries
        .new
        .iter()
        .chain(&entries.changed)
        .map(|entry| entry.size)
        .sum();
    (entries.new.len() + entries.changed.len(), bytes)
}

fn transfer_directories(
    sink: &Sink,
    directories: &EntryPlan,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    for entry in directories.new.iter().chain(&directories.changed) {
        if let Err(error) = sink.create_directories(std::slice::from_ref(entry)) {
            record_failure(report, emit, entry.path.clone(), error.to_string());
        }
    }
}

fn transfer_symlinks(
    sink: &Sink,
    source_reader_root: &Path,
    symlinks: &EntryPlan,
    paths: &SourcePathMap,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    for entry in symlinks.new.iter().chain(&symlinks.changed) {
        let Some(source_relative) = paths.source_for_destination.get(&entry.path) else {
            record_failure(
                report,
                emit,
                entry.path.clone(),
                "source path mapping is missing".to_owned(),
            );
            continue;
        };
        let source_path = source_reader_root.join(Path::new(source_relative));
        let target = match fs::read_link(&source_path) {
            Ok(target) => target,
            Err(error) => {
                record_failure(report, emit, entry.path.clone(), error.to_string());
                continue;
            }
        };
        let target_kind = fs::metadata(&source_path).map_or(SymlinkTargetKind::File, |metadata| {
            if metadata.is_dir() {
                SymlinkTargetKind::Directory
            } else {
                SymlinkTargetKind::File
            }
        });
        if let Err(error) = sink.create_symlink(entry, &target, target_kind) {
            record_failure(report, emit, entry.path.clone(), error.to_string());
        }
    }
}

struct FileTask {
    source: FileEntry,
    destination: FileEntry,
    source_path: PathBuf,
}

struct FileTransfer {
    bytes: u64,
    physical_bytes: u64,
    method: TransferMethod,
}

struct FileOutcome {
    path: String,
    result: Result<FileTransfer, String>,
}

fn transfer_files(
    sink: &Sink,
    source_reader_root: &Path,
    files: &EntryPlan,
    paths: &SourcePathMap,
    options: &LocalSyncOptions,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<(), LocalSyncError> {
    let tasks: Vec<_> = files
        .new
        .iter()
        .chain(&files.changed)
        .filter_map(|destination| {
            let source_relative = paths.source_for_destination.get(&destination.path)?;
            let mut source = destination.clone();
            source.path.clone_from(source_relative);
            Some(FileTask {
                source,
                destination: destination.clone(),
                source_path: source_reader_root.join(Path::new(source_relative)),
            })
        })
        .collect();
    let (task_sender, task_receiver) = bounded(options.queue_capacity);
    let (outcome_sender, outcome_receiver) = unbounded();
    let mut workers = Vec::with_capacity(options.local_workers);
    for _ in 0..options.local_workers {
        workers.push(spawn_file_worker(
            task_receiver.clone(),
            outcome_sender.clone(),
            sink.clone(),
            SourceReader::new(source_reader_root),
            options.paranoid,
        ));
    }
    drop(outcome_sender);

    let mut dispatched = 0;
    for task in tasks {
        if task_sender.send(task).is_err() {
            drop(task_sender);
            join_workers(workers)?;
            return Err(LocalSyncError::WorkerDisconnected { dispatched });
        }
        dispatched += 1;
    }
    drop(task_sender);

    for _ in 0..dispatched {
        let Ok(outcome) = outcome_receiver.recv() else {
            join_workers(workers)?;
            return Err(LocalSyncError::WorkerDisconnected { dispatched });
        };
        match outcome.result {
            Ok(transfer) => {
                report.transferred_files += 1;
                report.transferred_bytes = report.transferred_bytes.saturating_add(transfer.bytes);
                report.physical_bytes = report
                    .physical_bytes
                    .saturating_add(transfer.physical_bytes);
                match transfer.method {
                    TransferMethod::FileClone => report.file_clones += 1,
                    TransferMethod::ByteCopy => report.byte_copies += 1,
                    TransferMethod::DirectoryClone => report.directory_clones += 1,
                }
                emit(LocalEvent::Transferred {
                    path: outcome.path,
                    bytes: transfer.bytes,
                    physical_bytes: transfer.physical_bytes,
                    method: transfer.method,
                });
            }
            Err(message) => record_failure(report, emit, outcome.path, message),
        }
    }
    join_workers(workers)
}

fn spawn_file_worker(
    tasks: Receiver<FileTask>,
    outcomes: Sender<FileOutcome>,
    sink: Sink,
    source_reader: SourceReader,
    paranoid: bool,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("xsync-local-worker".to_owned())
        .spawn(move || {
            for task in tasks {
                let path = task.destination.path.clone();
                let result = transfer_one_file(&sink, &source_reader, task, paranoid);
                if outcomes.send(FileOutcome { path, result }).is_err() {
                    break;
                }
            }
        })
        .expect("local transfer worker thread should start")
}

fn transfer_one_file(
    sink: &Sink,
    source_reader: &SourceReader,
    task: FileTask,
    paranoid: bool,
) -> Result<FileTransfer, String> {
    let destination_path = sink
        .path_for(&task.destination.path)
        .map_err(|error| error.to_string())?;
    match clone::try_clone_file(&task.source_path, &destination_path, &task.source, paranoid) {
        Ok(Some(outcome)) => {
            if outcome.kind == CloneKind::File {
                return Ok(FileTransfer {
                    bytes: task.source.size,
                    physical_bytes: 0,
                    method: TransferMethod::FileClone,
                });
            }
        }
        Ok(None) | Err(clone::CloneError::WrongKind { .. }) => {}
        Err(error) => return Err(error.to_string()),
    }
    let stable = source_reader
        .read(&task.source)
        .map_err(|error: SourceReadError| error.to_string())?;
    let mut destination = stable.entry.clone();
    destination.path = task.destination.path;
    let bytes = stable.bytes;
    let length = u64::try_from(bytes.len()).map_err(|_| "file is too large".to_owned())?;
    sink.write_file_with_retry(&destination, &stable.blake3, |_| Ok(bytes.clone()))
        .map_err(|error| error.to_string())?;
    Ok(FileTransfer {
        bytes: length,
        physical_bytes: length,
        method: TransferMethod::ByteCopy,
    })
}

fn join_workers(workers: Vec<JoinHandle<()>>) -> Result<(), LocalSyncError> {
    for worker in workers {
        worker.join().map_err(|_| LocalSyncError::WorkerPanicked)?;
    }
    Ok(())
}

fn finish_directories(
    sink: &Sink,
    directories: &EntryPlan,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    let mut entries: Vec<_> = directories.new.iter().chain(&directories.changed).collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(path_depth(&entry.path)));
    for entry in entries {
        if let Err(error) = sink.finish_directories(std::slice::from_ref(entry)) {
            record_failure(report, emit, entry.path.clone(), error.to_string());
        }
    }
}

fn delete_extraneous(
    sink: &Sink,
    plan: &Plan,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    let mut entries: Vec<&FileEntry> = plan
        .files
        .extraneous
        .iter()
        .chain(&plan.directories.extraneous)
        .chain(&plan.symlinks.extraneous)
        .chain(&plan.other.extraneous)
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(path_depth(&entry.path)));
    for entry in entries {
        match sink.delete_entry(entry) {
            Ok(()) => {
                report.deleted_entries += 1;
                emit(LocalEvent::Deleted {
                    path: entry.path.clone(),
                });
            }
            Err(error) => record_failure(report, emit, entry.path.clone(), error.to_string()),
        }
    }
}

fn path_depth(path: &str) -> usize {
    path.bytes().filter(|&byte| byte == b'/').count()
}

fn record_failure(
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
    path: String,
    message: String,
) {
    report.failed_entries += 1;
    report.warnings += 1;
    emit(LocalEvent::Warning {
        path: path.clone(),
        message: message.clone(),
    });
    emit(LocalEvent::Failed { path, message });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    use super::*;

    fn options() -> LocalSyncOptions {
        LocalSyncOptions {
            local_workers: 2,
            streams: 7,
            queue_capacity: 1,
            dry_run: false,
            delete: false,
            paranoid: false,
            exclude_patterns: Vec::new(),
        }
    }

    #[test]
    fn directory_layout_and_second_run_are_observable() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("nested/file"), b"payload").unwrap();
        fs::File::create(source.join("large"))
            .unwrap()
            .set_len(100 * 1024 * 1024)
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("nested/file", source.join("link")).unwrap();

        let mut first_events = Vec::new();
        let first = sync(&source, false, &destination, false, &options(), |event| {
            first_events.push(event);
        })
        .unwrap();
        assert_eq!(first.transferred_files, 2);
        assert_eq!(first.transferred_bytes, 100 * 1024 * 1024 + 7);
        assert_eq!(first.skipped_files, 0);
        assert_eq!(first.local_workers, 2);
        assert_eq!(first.streams, 7);
        assert!(destination.join("source/nested/file").is_file());
        assert_eq!(
            fs::metadata(destination.join("source/large"))
                .unwrap()
                .len(),
            100 * 1024 * 1024
        );
        assert!(destination.join("source/nested/empty").is_dir());
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(destination.join("source/link")).unwrap(),
            Path::new("nested/file")
        );

        let mut second_events = Vec::new();
        let second = sync(&source, false, &destination, false, &options(), |event| {
            second_events.push(event);
        })
        .unwrap();
        assert_eq!(second.transferred_files, 0);
        assert_eq!(second.transferred_bytes, 0);
        assert_eq!(second.skipped_files, 2);
        assert!(second_events.iter().any(|event| matches!(
            event,
            LocalEvent::Skipped { path } if path == "nested/file"
        )));
        assert!(first_events.iter().any(|event| matches!(
            event,
            LocalEvent::Started {
                local_workers: 2,
                streams: 7
            }
        )));
    }

    #[test]
    fn trailing_slash_puts_directory_contents_at_destination() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("file"), b"data").unwrap();

        sync(&source, true, &destination, false, &options(), |_| {}).unwrap();
        assert!(destination.join("file").is_file());
        assert!(!destination.join("source/file").exists());
    }

    #[test]
    fn exclusions_disable_directory_clone_without_changing_output() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("keep"), b"keep").unwrap();
        fs::write(source.join("skip"), b"skip").unwrap();
        let mut options = options();
        options.exclude_patterns = vec!["skip".to_owned()];
        let mut events = Vec::new();

        let report = sync(&source, true, &destination, false, &options, |event| {
            events.push(event);
        })
        .unwrap();
        assert_eq!(report.transferred_files, 1);
        assert!(destination.join("keep").is_file());
        assert!(!destination.join("skip").exists());
        assert!(!events.iter().any(|event| matches!(
            event,
            LocalEvent::Transferred {
                method: TransferMethod::DirectoryClone,
                ..
            }
        )));
    }

    #[test]
    fn file_root_is_copied_into_an_existing_destination_directory() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination");
        fs::write(&source, b"file root").unwrap();
        fs::create_dir(&destination).unwrap();

        sync(&source, false, &destination, false, &options(), |_| {}).unwrap();
        assert_eq!(
            fs::read(destination.join("source.txt")).unwrap(),
            b"file root"
        );
    }

    #[test]
    fn delete_waits_for_success_and_removes_extraneous_entries() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("keep"), b"keep").unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("old"), b"old").unwrap();

        let mut options = options();
        options.delete = true;
        let report = sync(&source, true, &destination, false, &options, |_| {}).unwrap();
        assert_eq!(report.failed_entries, 0);
        assert_eq!(report.deleted_entries, 1);
        assert!(!destination.join("old").exists());
        assert_eq!(fs::read(destination.join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn reports_unsupported_source_entries_without_stopping_file_work() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"data").unwrap();

        let report = sync(&source, true, &destination, false, &options(), |_| {}).unwrap();
        assert_eq!(report.failed_entries, 0);
        assert_eq!(report.transferred_files, 1);
    }

    #[allow(dead_code)]
    fn _assert_event_paths_are_unique(events: &[LocalEvent]) {
        let mut paths = BTreeMap::new();
        for event in events {
            let path = match event {
                LocalEvent::Transferred { path, .. }
                | LocalEvent::Skipped { path }
                | LocalEvent::Warning { path, .. }
                | LocalEvent::Failed { path, .. }
                | LocalEvent::Deleted { path } => Some(path),
                _ => None,
            };
            if let Some(path) = path {
                *paths.entry(path).or_insert(0usize) += 1;
            }
        }
    }

    #[test]
    fn source_mtime_is_preserved_by_root_metadata_pass() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        let mtime = filetime::FileTime::from_unix_time(1_600_000_000, 123);
        filetime::set_file_mtime(&source, mtime).unwrap();
        sync(&source, true, &destination, false, &options(), |_| {}).unwrap();
        let actual =
            filetime::FileTime::from_last_modification_time(&fs::metadata(&destination).unwrap());
        assert_eq!(actual, mtime);
    }
}
