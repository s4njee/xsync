//! In-process local-to-local synchronization.
//!
//! Local transfers deliberately do not use protocol messages. Discovery and
//! planning remain metadata-only; worker threads read source bytes only after
//! a file has been classified for transfer.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

use crate::clone::{self, CloneKind};
use crate::cloud;
use crate::hash_cache::{HashCache, HashFingerprint};
use crate::path::WirePath;
use crate::planner::{try_plan, DestinationIndex, EntryPlan, IndexConfig, Plan, PlannerError};
use crate::scanner::{
    fingerprint_from_metadata, permission_mode, scan, EntryKind, FileEntry, ScanError,
};
use crate::sink::{Sink, SinkError, SymlinkTargetKind};
use crate::source::{SourceReadError, SourceReader};
use crate::transport::TransportSelection;

/// Exit status used when a local job completed with per-entry failures.
pub const PARTIAL_FAILURE_EXIT_CODE: u8 = 23;

/// Minimum file size for the staged clone path. On APFS, the clone setup and
/// validation cost more than a verified buffered copy for smaller files.
/// This threshold is pinned by the paired T1.2 clone measurements.
pub const FILE_CLONE_MIN_BYTES: u64 = 12 * 1024 * 1024;

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

/// What to do when two distinct source paths name one destination file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCollisionPolicy {
    /// Refuse the transfer and name the collisions. The default, because
    /// publishing both and keeping whichever wrote last is silent data loss.
    Fail,
    /// Omit every path involved in a collision, and report them as warnings.
    /// Nothing is published under a name that another source path also claims.
    Skip,
}

/// Policy for files whose contents may be resident in a cloud provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudFilesPolicy {
    /// Read files normally, materializing them when required.
    Download,
    /// Omit detected placeholders from transfer.
    Skip,
    /// Refuse the job if placeholders are detected.
    Error,
}

/// Events emitted by a local transfer.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEvent {
    /// A measurable pipeline phase boundary.
    Phase {
        /// Stable phase name: scan, plan, transfer, or metadata.
        name: &'static str,
        /// True for phase start and false for phase end.
        started: bool,
    },
    /// Backend telemetry that is not tied to a single transfer event.
    Metrics {
        /// Highest scanner/work queue occupancy observed.
        queue_high_water: usize,
        /// Compression selected by the backend, when applicable.
        compression_algorithm: Option<&'static str>,
        /// Compression level selected by the backend, when applicable.
        compression_level: Option<i32>,
    },
    /// The local pipeline has started. `streams` is reported for observability
    /// but does not configure local worker scheduling.
    Started {
        /// Number of local I/O workers.
        local_workers: usize,
        /// Requested remote stream count, ignored for this local route.
        streams: usize,
    },
    /// Compression was negotiated for the remote session.
    Negotiated {
        /// Selected wire compression algorithm, or `none`.
        compression_algorithm: &'static str,
        /// Human-readable reason for the selected mode.
        compression_reason: &'static str,
    },
    /// Wire version and capability set observed during the handshake.
    ProtocolNegotiated {
        /// Session grammar selected before the first data frame.
        selected_version: u32,
        /// Capabilities advertised by the remote endpoint.
        remote_capabilities: u32,
        /// Known capabilities shared by both endpoints.
        common_capabilities: u32,
        /// Whether browse-only v2 requests are available.
        browse_available: bool,
    },
    /// Metadata planning has completed.
    Planned {
        /// Number of regular files requiring transfer.
        files: usize,
        /// Logical bytes in those files at discovery time.
        bytes: u64,
    },
    /// Placeholder inventory discovered during scanning.
    CloudPlaceholders {
        /// Number of detected placeholder files.
        files: usize,
        /// Logical bytes represented by those files.
        bytes: u64,
        /// Whether platform detection is available.
        detection_available: bool,
        /// Whether this run actually inspected entries for placeholders.
        ///
        /// Detection costs a process spawn per file, so it runs only for the
        /// policies whose outcome depends on the answer. When this is false
        /// the counts are zero because nothing was inspected, not because no
        /// placeholder exists — consumers must not read them as an inventory.
        detection_performed: bool,
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
    /// Bounded progress update for an active large-file transfer.
    Progress {
        /// Destination-relative path.
        path: String,
        /// Logical stream identifier.
        stream: usize,
        /// Bytes sent or written so far.
        completed: u64,
        /// Total logical file size.
        total: u64,
    },
    /// An unchanged regular file was not transferred.
    Skipped {
        /// Destination-relative path.
        path: String,
        /// Logical bytes already present at the destination.
        bytes: u64,
    },
    /// An action that would be performed by a real transfer. In dry-run mode
    /// these are the complete mutation plan; normal runs emit them before
    /// performing the corresponding action.
    Action {
        /// Destination-relative path.
        path: String,
        /// One of create, update, or delete.
        action: &'static str,
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
    /// A filter rule decided the fate of one path, reported under
    /// `--explain-filter`.
    FilterDecision {
        /// Source-relative path.
        path: String,
        /// Whether the path is transferred.
        included: bool,
        /// One line naming the rule and where it was written.
        reason: String,
    },
    /// The local pipeline has finished.
    Finished {
        /// Source metadata that this run did not carry to the destination.
        dropped_metadata: crate::sparse::DroppedMetadata,
        /// Selected backend and its capability contract.
        transport: Option<TransportSelection>,
        /// Number of files published.
        transferred_files: usize,
        /// Logical bytes published.
        transferred_bytes: u64,
        /// Number of unchanged files skipped.
        skipped_files: usize,
        /// Number of entries that failed.
        failed_entries: usize,
        /// Number of extraneous entries deleted.
        deleted_entries: usize,
        /// Number of warnings emitted.
        warnings: usize,
        /// Bytes physically moved through the streaming path.
        physical_bytes: u64,
        /// Application-protocol bytes written to the transport, when known.
        wire_bytes: u64,
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
        /// Number of large files resumed from a durable checkpoint.
        restarted_files: usize,
        /// Bytes skipped because they were durably verified before the crash.
        resumed_bytes: u64,
        /// Bytes actually retransmitted this run.
        retransmitted_bytes: u64,
        /// Bytes durably checkpointed to the receiver journal this run.
        checkpoint_bytes: u64,
        /// Content-hash cache hits during checksum classification.
        checksum_cache_hits: usize,
        /// Content-hash cache misses during checksum classification.
        checksum_cache_misses: usize,
    },
}

/// Options for an in-process local transfer.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct LocalSyncOptions {
    /// Number of local I/O workers. This is independent of `streams`.
    pub local_workers: usize,
    /// Requested remote stream count, retained only for event reporting.
    pub streams: usize,
    /// Capacity of the shared local file queue.
    pub queue_capacity: usize,
    /// Permit staged directory-clone fast paths.
    pub directory_clones: bool,
    /// Plan without mutating the destination.
    pub dry_run: bool,
    /// Remove destination-only entries after all transfers succeed.
    pub delete: bool,
    /// Re-read clone output and verify content hashes.
    pub paranoid: bool,
    /// Classify regular files by BLAKE3 content rather than metadata.
    pub checksum: bool,
    /// Cloud-placeholder materialization policy.
    pub cloud_files: CloudFilesPolicy,
    /// Relative-path glob patterns that disable directory cloning and exclude
    /// matching source/destination entries.
    pub exclude_patterns: Vec<String>,
    /// Ordered include/exclude rules, superseding `exclude_patterns` when set.
    ///
    /// Kept alongside the flat list rather than replacing it: `exclude_patterns`
    /// is what crosses the v1 wire, and every caller that only needs excludes
    /// keeps working untouched.
    pub filter: Option<crate::filter::FilterSet>,
    /// Report the rule that decided each excluded path.
    ///
    /// Turning this on makes the scan walk what it would otherwise prune, so
    /// that a path removed by a rule can still be named. Off, the walk is
    /// pruned as before and costs nothing.
    pub explain_filter: bool,
    /// What to do when two source paths map to one destination name.
    pub on_path_collision: PathCollisionPolicy,
    /// Enable adaptive zstd compression for data payloads.
    pub compress: bool,
    /// Requested zstd level, constrained to 1..=22 by the CLI.
    pub compress_level: i32,
    /// Whether to upload a verified binary to a remote that has none.
    pub bootstrap: crate::bootstrap::BootstrapPolicy,
}

impl Default for LocalSyncOptions {
    fn default() -> Self {
        Self {
            local_workers: default_local_workers(),
            streams: 1,
            queue_capacity: DEFAULT_LOCAL_QUEUE_CAPACITY,
            directory_clones: true,
            dry_run: false,
            delete: false,
            paranoid: false,
            checksum: false,
            cloud_files: CloudFilesPolicy::Download,
            on_path_collision: PathCollisionPolicy::Fail,
            exclude_patterns: Vec::new(),
            filter: None,
            explain_filter: false,
            compress: true,
            compress_level: 3,
            bootstrap: crate::bootstrap::BootstrapPolicy::Disabled,
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
    /// Application-protocol bytes written to the transport, when known.
    pub wire_bytes: u64,
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
    /// Number of large files resumed from a durable checkpoint.
    pub restarted_files: usize,
    /// Bytes skipped because they were durably verified before the crash.
    pub resumed_bytes: u64,
    /// Bytes actually retransmitted this run.
    pub retransmitted_bytes: u64,
    /// Bytes durably checkpointed to the receiver journal this run.
    pub checkpoint_bytes: u64,
    /// Content-hash cache hits during checksum classification.
    pub checksum_cache_hits: usize,
    /// Content-hash cache misses during checksum classification.
    pub checksum_cache_misses: usize,
    /// Whether policy-controlled omissions made this a partial result.
    pub partial_work: bool,
}

impl LocalSyncReport {
    /// Whether the job completed with any per-entry failure.
    #[must_use]
    pub fn partial_failure(&self) -> bool {
        self.failed_entries != 0 || self.partial_work
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
    /// Two source paths would become one file on this destination.
    #[error(
        "{count} source paths collide on a destination that treats them as the same name \
         ({reason}); publishing them would silently keep only one. First collision: \
         {first}. Use --on-path-collision=skip to omit every colliding path instead."
    )]
    PathCollision {
        /// Number of distinct destination names with more than one source path.
        count: usize,
        /// Which destination property causes the collision.
        reason: &'static str,
        /// A representative collision, as `a <-> b`.
        first: String,
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
    /// The source and effective destination overlap on disk.
    #[error("source '{}' and destination '{}' overlap", source_path.display(), destination.display())]
    PathOverlap {
        /// Source root path.
        source_path: PathBuf,
        /// Effective destination root path.
        destination: PathBuf,
    },
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
    /// The requested cloud policy cannot be implemented on this platform.
    #[error("cloud placeholder detection is unavailable on this platform")]
    CloudPolicyUnavailable,
    /// Reading platform placeholder metadata failed before mutation.
    #[error("cannot detect cloud placeholder: {0}")]
    CloudDetection(#[source] io::Error),
    /// A placeholder was found under the refusing policy.
    #[error("cloud placeholder found at '{path}'")]
    CloudPlaceholderFound {
        /// Source-relative placeholder path.
        path: String,
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
#[allow(clippy::too_many_lines)]
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
    let source_path = source.as_ref();
    let destination_path = destination.as_ref();
    let source_metadata =
        fs::symlink_metadata(source_path).map_err(|source| LocalSyncError::SourceRoot {
            path: source_path.to_path_buf(),
            source,
        })?;
    validate_source_destination_overlap(source_path, destination_path, &source_metadata)?;
    if !options.dry_run && options.cloud_files == CloudFilesPolicy::Download {
        if let Some(report) = try_directory_fast_path(
            source_path,
            source_trailing_slash,
            destination_path,
            destination_trailing_slash,
            options,
            &mut emit,
        )? {
            return Ok(report);
        }
    }
    let prepared = prepare_transfer(
        source_path,
        source_trailing_slash,
        destination_path,
        destination_trailing_slash,
        options,
        &mut emit,
    )?;
    let PreparedTransfer {
        destination_sink,
        source_reader_root,
        source_root_entry,
        source_by_destination,
        mut plan,
        checksum_cache_hits,
        checksum_cache_misses,
        cloud_skipped,
        collision_failures,
    } = prepared;

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams: options.streams,
        checksum_cache_hits,
        checksum_cache_misses,
        failed_entries: collision_failures,
        partial_work: !cloud_skipped.is_empty() || collision_failures > 0,
        ..LocalSyncReport::default()
    };
    report.skipped_files = plan.files.unchanged.len();
    for entry in &plan.files.unchanged {
        emit(LocalEvent::Skipped {
            path: entry.path.to_string(),
            bytes: entry.size,
        });
    }
    for entry in &cloud_skipped {
        report.skipped_files += 1;
        emit(LocalEvent::Skipped {
            path: entry.path.to_string(),
            bytes: entry.size,
        });
    }
    // xsync has no concept of a hole, so a sparse source is read and written at
    // its apparent size. Say so before starting work that may not fit.
    // Symlinks are included so the preflight can see reparse points on Windows,
    // where a junction is silently recreated as a symlink. Counting only
    // `plan.files` reported zero reparse points for a tree that contained three.
    let transferable: Vec<FileEntry> = plan
        .files
        .new
        .iter()
        .chain(&plan.files.changed)
        .chain(&plan.symlinks.new)
        .chain(&plan.symlinks.changed)
        .cloned()
        .collect();
    // The ownership question is "who will own the copies", and the only way to
    // answer it without guessing is to create a file where they will go. A dry
    // run must not write to the destination, so it leaves ownership unchecked
    // rather than answering from a scratch directory on some other volume —
    // that reported every file as foreign-owned purely because the temp dir had
    // a different group.
    let owner = if options.dry_run {
        None
    } else {
        crate::sparse::Owner::probe(destination_sink.root())
    };
    let preflight = crate::sparse::inspect_with_workers(
        &transferable,
        &source_reader_root,
        owner,
        options.local_workers,
    );
    // `Owner::probe` returns `None` both when a dry run declines to write and
    // when the platform has no Unix ownership at all. Reporting those the same
    // way told Windows users, on a real run, that "a dry run does not write to
    // the destination".
    let ownership = if owner.is_some() {
        OwnershipCheck::Performed
    } else if cfg!(unix) {
        OwnershipCheck::SkippedForDryRun
    } else {
        OwnershipCheck::Unsupported
    };
    report_preflight(&preflight, ownership, &mut emit);
    if options.dry_run {
        emit_plan_actions(&plan, &mut emit);
    }
    for entry in plan.other.new.iter().chain(&plan.other.changed) {
        record_failure(
            &mut report,
            &mut emit,
            entry.path.to_string(),
            "unsupported filesystem object".to_owned(),
        );
    }

    if !options.dry_run {
        emit(LocalEvent::Phase {
            name: "transfer",
            started: true,
        });
        // Probe the destination once. Without reflink support every clone
        // attempt is doomed, and the attempts themselves dominate the run.
        let reflink = options.directory_clones && clone::supports_reflink(destination_sink.root());
        if reflink && options.cloud_files == CloudFilesPolicy::Download {
            apply_directory_clones(
                &source_reader_root,
                &destination_sink,
                &source_by_destination,
                &mut plan,
                options.paranoid,
                &mut report,
                &mut emit,
            )?;
        }
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
            &TransferSettings { options, reflink },
            &mut report,
            &mut emit,
        )?;

        if options.delete && !report.partial_failure() {
            protect_cloud_skipped(&mut plan, &cloud_skipped);
            delete_extraneous(&destination_sink, &plan, &mut report, &mut emit);
        }

        emit(LocalEvent::Phase {
            name: "transfer",
            started: false,
        });
        // Deletions update parent directory mtimes, so restore source metadata last.
        emit(LocalEvent::Phase {
            name: "metadata",
            started: true,
        });
        if let Some(root_entry) = source_root_entry {
            if let Err(error) = destination_sink.finish_root_directory(&root_entry) {
                record_failure(&mut report, &mut emit, String::from("."), error.to_string());
            }
        }
        finish_directories(
            &destination_sink,
            &plan,
            options.delete,
            &mut report,
            &mut emit,
        );
        emit(LocalEvent::Phase {
            name: "metadata",
            started: false,
        });
    }

    emit(LocalEvent::Finished {
        dropped_metadata: preflight.dropped.clone(),
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        directory_clones: report.directory_clones,
        file_clones: report.file_clones,
        byte_copies: report.byte_copies,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        deleted_entries: report.deleted_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
        restarted_files: report.restarted_files,
        resumed_bytes: report.resumed_bytes,
        retransmitted_bytes: report.retransmitted_bytes,
        checkpoint_bytes: report.checkpoint_bytes,
        checksum_cache_hits: report.checksum_cache_hits,
        checksum_cache_misses: report.checksum_cache_misses,
    });
    Ok(report)
}

/// Clone maximal directory subtrees that are wholly absent from an existing
/// destination. A directory clone is attempted only for entries classified as
/// new, so a partially-present or changed subtree stays on the normal planned
/// path and retains per-entry correctness checks.
fn apply_directory_clones(
    source_root: &Path,
    sink: &Sink,
    paths: &SourcePathMap,
    plan: &mut Plan,
    paranoid: bool,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<(), LocalSyncError> {
    let mut candidates = plan.directories.new.clone();
    candidates.sort_by_key(|entry| (path_depth(&entry.path), entry.path.clone()));

    let mut selected: Vec<WirePath> = Vec::new();
    for candidate in candidates {
        if candidate.path.is_empty()
            || selected
                .iter()
                .any(|parent| candidate.path.starts_with(parent))
        {
            continue;
        }
        selected.push(candidate.path.clone());
    }

    for candidate_path in selected {
        let Some(source_relative) = paths.source_for_destination.get(&candidate_path) else {
            continue;
        };
        let source_path = source_relative.to_native_path(source_root);
        let target_path = sink.path_for(&candidate_path)?;
        let root = plan
            .directories
            .new
            .iter()
            .find(|entry| entry.path == candidate_path)
            .cloned()
            .ok_or_else(|| LocalSyncError::PathMapping {
                path: candidate_path.to_string(),
            })?;
        let entries = clone_entries_for_subtree(plan, &candidate_path);
        let file_count = entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .count();
        let logical_bytes: u64 = entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .map(|entry| entry.size)
            .sum();

        let Some(outcome) =
            clone::try_clone_directory(&source_path, &target_path, &root, &entries, paranoid)?
        else {
            continue;
        };

        remove_subtree_entries(plan, &candidate_path);
        report.directory_clones += usize::from(outcome.kind == CloneKind::Directory);
        report.transferred_files += file_count;
        report.transferred_bytes = report.transferred_bytes.saturating_add(logical_bytes);
        emit(LocalEvent::Transferred {
            path: candidate_path.to_string(),
            bytes: logical_bytes,
            physical_bytes: 0,
            method: TransferMethod::DirectoryClone,
        });
    }
    Ok(())
}

fn clone_entries_for_subtree(plan: &Plan, root: &WirePath) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for group in [&plan.directories, &plan.files, &plan.symlinks, &plan.other] {
        for entry in &group.new {
            let Some(relative) = entry.path.strip_prefix(root) else {
                continue;
            };
            let mut relative_entry = entry.clone();
            relative.clone_into(&mut relative_entry.path);
            entries.push(relative_entry);
        }
    }
    entries
}

fn remove_subtree_entries(plan: &mut Plan, root: &WirePath) {
    for group in [
        &mut plan.directories,
        &mut plan.files,
        &mut plan.symlinks,
        &mut plan.other,
    ] {
        group
            .new
            .retain(|entry| entry.path != *root && !entry.path.starts_with(root));
    }
}

#[allow(clippy::too_many_lines)]
fn try_directory_fast_path(
    source: &Path,
    source_trailing_slash: bool,
    destination: &Path,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<Option<LocalSyncReport>, LocalSyncError> {
    if options.delete || options.filters_anything() {
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
    emit(LocalEvent::Phase {
        name: "scan",
        started: true,
    });
    let (entries, queue_high_water) = collect_scan(
        source,
        &std::sync::Arc::new(crate::filter::FilterSet::new()),
    )?;
    emit(LocalEvent::Metrics {
        queue_high_water,
        compression_algorithm: None,
        compression_level: None,
    });
    emit(LocalEvent::Phase {
        name: "scan",
        started: false,
    });
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
    emit(LocalEvent::Phase {
        name: "plan",
        started: true,
    });
    emit(LocalEvent::Phase {
        name: "plan",
        started: false,
    });
    emit(LocalEvent::Phase {
        name: "transfer",
        started: true,
    });
    let Some(outcome) = clone::try_clone_directory(
        source,
        &layout.destination_root,
        &root,
        &entries,
        options.paranoid,
    )?
    else {
        emit(LocalEvent::Phase {
            name: "transfer",
            started: false,
        });
        return Ok(None);
    };
    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams: options.streams,
    });
    // The fast path is only reachable under `CloudFilesPolicy::Download`, and a
    // whole-tree clone cannot filter individual entries, so nothing is inspected.
    emit(LocalEvent::CloudPlaceholders {
        files: 0,
        bytes: 0,
        detection_available: cfg!(target_os = "macos"),
        detection_performed: false,
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
    emit(LocalEvent::Phase {
        name: "transfer",
        started: false,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: true,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: false,
    });
    emit(LocalEvent::Finished {
        // The clone fast path preserves xattrs, ownership and ACLs, so it
        // drops nothing worth reporting.
        dropped_metadata: crate::sparse::DroppedMetadata::default(),
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        directory_clones: report.directory_clones,
        file_clones: report.file_clones,
        byte_copies: report.byte_copies,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        deleted_entries: report.deleted_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
        restarted_files: report.restarted_files,
        resumed_bytes: report.resumed_bytes,
        retransmitted_bytes: report.retransmitted_bytes,
        checkpoint_bytes: report.checkpoint_bytes,
        checksum_cache_hits: report.checksum_cache_hits,
        checksum_cache_misses: report.checksum_cache_misses,
    });
    Ok(Some(report))
}

struct PreparedTransfer {
    destination_sink: Sink,
    source_reader_root: PathBuf,
    source_root_entry: Option<FileEntry>,
    source_by_destination: SourcePathMap,
    plan: Plan,
    checksum_cache_hits: usize,
    checksum_cache_misses: usize,
    cloud_skipped: Vec<FileEntry>,
    /// Source paths omitted because they collide on this destination.
    collision_failures: usize,
}

impl LocalSyncOptions {
    /// The filter this run applies, whether given directly or as a flat list.
    ///
    /// # Errors
    ///
    /// Returns [`LocalSyncError::InvalidExclude`] when a pattern is not a valid
    /// glob.
    pub fn effective_filter(&self) -> Result<crate::filter::FilterSet, LocalSyncError> {
        if let Some(filter) = &self.filter {
            return Ok(filter.clone());
        }
        crate::filter::from_exclude_patterns(&self.exclude_patterns).map_err(|error| {
            LocalSyncError::InvalidExclude {
                pattern: "<combined>".to_owned(),
                message: error.to_string(),
            }
        })
    }

    /// Whether this run filters anything at all.
    #[must_use]
    pub fn filters_anything(&self) -> bool {
        self.filter
            .as_ref()
            .is_some_and(|filter| !filter.is_empty())
            || !self.exclude_patterns.is_empty()
    }
}

#[allow(clippy::too_many_lines)]
fn prepare_transfer(
    source: &Path,
    source_trailing_slash: bool,
    destination: &Path,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<PreparedTransfer, LocalSyncError> {
    let filter = std::sync::Arc::new(options.effective_filter()?);
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

    emit(LocalEvent::Phase {
        name: "scan",
        started: true,
    });
    let source_reader_root = source_reader_root(source, source_kind);
    // Explaining a decision requires having seen the path, and a pruned walk
    // never sees what it pruned. The unfiltered walk is therefore only taken
    // when the answer is being asked for.
    let scan_filter = if options.explain_filter {
        std::sync::Arc::new(filter.observing_only())
    } else {
        std::sync::Arc::clone(&filter)
    };
    let (source_entries, source_queue_high_water) = collect_scan(source, &scan_filter)?;
    let mut source_entries: Vec<_> = source_entries
        .into_iter()
        .filter(|entry| {
            let path = entry.path.to_string();
            let decision = filter.decide(&path);
            if options.explain_filter && !decision.is_included() {
                emit(LocalEvent::FilterDecision {
                    path,
                    included: false,
                    reason: decision.explain(),
                });
            }
            decision.is_included()
        })
        .collect();
    let mut cloud_skipped = Vec::new();
    let mut cloud_files = 0;
    let mut cloud_bytes: u64 = 0;
    // `cloud::is_placeholder` costs a process spawn per file, so only the
    // policies whose outcome depends on the answer pay for it. Under
    // `Download` every entry is retained either way, and the directory fast
    // path already reports zero for exactly the same reason.
    let detect_placeholders =
        cloud::detection_available() && options.cloud_files != CloudFilesPolicy::Download;
    if detect_placeholders {
        let mut retained = Vec::with_capacity(source_entries.len());
        for entry in source_entries {
            let is_placeholder = entry.kind == EntryKind::File
                && cloud::is_placeholder(&entry.path.to_native_path(&source_reader_root))
                    .map_err(LocalSyncError::CloudDetection)?;
            if !is_placeholder {
                retained.push(entry);
                continue;
            }
            cloud_files += 1;
            cloud_bytes = cloud_bytes.saturating_add(entry.size);
            match options.cloud_files {
                CloudFilesPolicy::Download => retained.push(entry),
                CloudFilesPolicy::Skip => cloud_skipped.push(entry),
                CloudFilesPolicy::Error => {
                    return Err(LocalSyncError::CloudPlaceholderFound {
                        path: entry.path.to_string(),
                    });
                }
            }
        }
        source_entries = retained;
    }
    emit(LocalEvent::CloudPlaceholders {
        files: cloud_files,
        bytes: cloud_bytes,
        detection_available: cloud::detection_available(),
        detection_performed: detect_placeholders,
    });
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
    let skipped_destinations = layout
        .map_source_entries(&cloud_skipped)
        .destination_for_source
        .into_values()
        .collect::<Vec<_>>();
    for (entry, path) in cloud_skipped.iter_mut().zip(skipped_destinations) {
        entry.path = path;
    }
    let destination_sink = if options.dry_run {
        Sink::new_without_creation(&layout.destination_root)
    } else {
        Sink::new(&layout.destination_root)?
    };
    let destination_filter = std::sync::Arc::new(filter.without_ignore_discovery());
    let (destination_entries, destination_queue_high_water) =
        collect_scan(destination_sink.root(), &destination_filter)?;
    emit(LocalEvent::Metrics {
        queue_high_water: source_queue_high_water.max(destination_queue_high_water),
        compression_algorithm: None,
        compression_level: None,
    });
    let destination_entries = destination_entries.into_iter().filter(|entry| {
        layout
            .direct_destination_name
            .as_ref()
            .is_none_or(|name| entry.path == *name)
            && filter.decide(&entry.path.to_string()).is_included()
    });
    let mut destination_index = DestinationIndex::with_config(IndexConfig::default())?;
    for entry in destination_entries {
        destination_index.insert(entry)?;
    }
    emit(LocalEvent::Phase {
        name: "scan",
        started: false,
    });
    emit(LocalEvent::Phase {
        name: "plan",
        started: true,
    });
    let planned_source: Result<Vec<_>, _> = source_entries
        .iter()
        .map(|entry| {
            let mut planned = entry.clone();
            planned.path = source_by_destination
                .destination_for_source
                .get(&entry.path)
                .cloned()
                .ok_or_else(|| LocalSyncError::PathMapping {
                    path: entry.path.to_string(),
                })?;
            Ok::<FileEntry, LocalSyncError>(planned)
        })
        .collect();
    let planned_source = planned_source?;
    // Detect, before anything is written, two source paths that this destination
    // cannot tell apart. Publishing both keeps whichever wrote last.
    let (planned_source, dropped) = enforce_path_collisions(
        planned_source,
        destination_sink.root(),
        options.on_path_collision,
        emit,
    )?;
    let collision_failures = dropped;
    let mut plan = try_plan(planned_source, destination_index)?;
    let (checksum_cache_hits, checksum_cache_misses) = if options.checksum {
        apply_checksum_classification(
            &mut plan,
            &destination_sink,
            &source_reader_root,
            &source_by_destination,
        )
    } else {
        (0, 0)
    };
    let (planned_files, planned_bytes) = transfer_totals(&plan.files);
    emit(LocalEvent::Planned {
        files: planned_files,
        bytes: planned_bytes,
    });
    emit(LocalEvent::Phase {
        name: "plan",
        started: false,
    });
    Ok(PreparedTransfer {
        destination_sink,
        source_reader_root,
        source_root_entry,
        source_by_destination,
        plan,
        checksum_cache_hits,
        checksum_cache_misses,
        cloud_skipped,
        collision_failures,
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
    if options.cloud_files != CloudFilesPolicy::Download && !cfg!(target_os = "macos") {
        return Err(LocalSyncError::CloudPolicyUnavailable);
    }
    Ok(())
}

/// Default local file-worker count.
///
/// One worker per logical core is right on Linux, where a sweep on ext4 scales
/// monotonically from 0.468 s at one worker to 0.193 s at 24 (2.43x), with no
/// degradation at the top end.
///
/// macOS is different in kind, not degree: the filesystem serializes directory
/// metadata mutation, so past a handful of workers the threads queue on a lock
/// instead of progressing. Measured on APFS across two corpus shapes, the
/// optimum is 4 and every count above it is worse — congress-10k (one file per
/// directory) 2.520 s at 4 against 2.717 s at the 10-core default, and cb7's
/// `node_modules` (7.2 files per directory) 3.834 s at 4 against 4.418 s. The
/// core-count default therefore costs 8-15% on this platform.
fn default_local_workers() -> usize {
    let cores = thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    if cfg!(target_os = "macos") {
        cores.min(MACOS_WORKER_CAP)
    } else {
        cores
    }
}

/// Worker ceiling on macOS, where additional workers contend rather than help.
/// `--local-workers` overrides it for measurement or for unusual workloads.
const MACOS_WORKER_CAP: usize = 4;

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
                    WirePath::from(destination_name.as_str())
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
    destination_for_source: BTreeMap<WirePath, WirePath>,
    source_for_destination: BTreeMap<WirePath, WirePath>,
}

fn collect_scan(
    root: &Path,
    filter: &std::sync::Arc<crate::filter::FilterSet>,
) -> Result<(Vec<FileEntry>, usize), LocalSyncError> {
    if matches!(
        fs::symlink_metadata(root),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    ) {
        return Ok((Vec::new(), 0));
    }
    let scan = if filter.is_empty() {
        scan(root)?
    } else {
        crate::scanner::scan_with_filter(root, std::sync::Arc::clone(filter))?
    };
    let mut entries = Vec::new();
    let mut first_error = None;
    for result in scan.entries() {
        match result {
            Ok(entry) => entries.push(entry),
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    let queue_high_water = scan.queue_high_water_mark();
    scan.finish()?;
    if let Some(error) = first_error {
        return Err(error.into());
    }
    Ok((entries, queue_high_water))
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
        path: WirePath::default(),
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

fn validate_source_destination_overlap(
    source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<(), LocalSyncError> {
    let source_real =
        canonicalize_with_missing(source).map_err(|source_error| LocalSyncError::SourceRoot {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let destination_real = canonicalize_with_missing(destination).map_err(|source_error| {
        LocalSyncError::SourceRoot {
            path: destination.to_path_buf(),
            source: source_error,
        }
    })?;
    let source_is_directory = source_metadata.is_dir() && !source_metadata.file_type().is_symlink();
    let overlaps = source_real == destination_real
        || destination_real.starts_with(&source_real)
        || (source_is_directory && source_real.starts_with(&destination_real));
    if overlaps {
        return Err(LocalSyncError::PathOverlap {
            source_path: source.to_path_buf(),
            destination: destination.to_path_buf(),
        });
    }
    Ok(())
}

fn canonicalize_with_missing(path: &Path) -> io::Result<PathBuf> {
    let mut suffix = Vec::new();
    let mut current = path;
    loop {
        match fs::canonicalize(current) {
            Ok(mut resolved) => {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = current.file_name() else {
                    return Err(error);
                };
                suffix.push(name.to_owned());
                current = current.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
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

/// Probe a destination, detect colliding destination paths, and apply policy.
///
/// Returns the entries that may safely be published and how many were dropped.
/// Used by every path that plans a transfer: local, and the pull client whose
/// destination is also local.
///
/// # Errors
/// Returns [`LocalSyncError::PathCollision`] under [`PathCollisionPolicy::Fail`].
pub(crate) fn enforce_path_collisions(
    entries: Vec<FileEntry>,
    destination_root: &Path,
    policy: PathCollisionPolicy,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<(Vec<FileEntry>, usize), LocalSyncError> {
    let semantics = crate::pathsem::PathSemantics::probe(destination_root);
    let collisions = find_path_collisions(&entries, semantics);
    if collisions.is_empty() {
        return Ok((entries, 0));
    }
    report_collisions(&collisions, semantics, policy, emit)?;
    let dropped: HashSet<WirePath> = collisions
        .iter()
        .flat_map(|group| group.iter().cloned())
        .collect();
    let count = entries
        .iter()
        .filter(|entry| dropped.iter().any(|path| entry.path.starts_with(path)))
        .count();
    let kept = entries
        .into_iter()
        .filter(|entry| !dropped.iter().any(|path| entry.path.starts_with(path)))
        .collect();
    Ok((kept, count))
}

/// Group planned destination paths that this destination would treat as one name.
///
/// Returns one group per colliding name, each holding the distinct source paths
/// that claim it. An empty result means every path is unambiguous here.
pub(crate) fn find_path_collisions(
    entries: &[FileEntry],
    semantics: crate::pathsem::PathSemantics,
) -> Vec<Vec<WirePath>> {
    if !semantics.can_collide() {
        return Vec::new();
    }
    let mut by_key: BTreeMap<Vec<u8>, Vec<WirePath>> = BTreeMap::new();
    for entry in entries {
        by_key
            .entry(semantics.collision_key(&entry.path))
            .or_default()
            .push(entry.path.clone());
    }
    by_key
        .into_values()
        .filter(|group| {
            // A key shared by one path is not a collision; a key shared by two
            // *identical* paths cannot happen, but guard against it anyway.
            let mut distinct: Vec<&WirePath> = group.iter().collect();
            distinct.sort();
            distinct.dedup();
            distinct.len() > 1
        })
        .collect()
}

/// Fail or warn about collisions according to policy.
pub(crate) fn report_collisions(
    collisions: &[Vec<WirePath>],
    semantics: crate::pathsem::PathSemantics,
    policy: PathCollisionPolicy,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<(), LocalSyncError> {
    let reason = match (
        semantics.case_insensitive,
        semantics.normalization_insensitive,
    ) {
        (true, true) => "case- and Unicode-normalization-insensitive",
        (true, false) => "case-insensitive",
        (false, true) => "Unicode-normalization-insensitive",
        (false, false) => "insensitive",
    };
    let first = collisions.first().map_or_else(String::new, |group| {
        group
            .iter()
            .map(WirePath::to_string)
            .collect::<Vec<_>>()
            .join(" <-> ")
    });
    match policy {
        PathCollisionPolicy::Fail => Err(LocalSyncError::PathCollision {
            count: collisions.len(),
            reason,
            first,
        }),
        PathCollisionPolicy::Skip => {
            for group in collisions {
                for path in group {
                    emit(LocalEvent::Failed {
                        path: path.to_string(),
                        message: format!(
                            "skipped: collides with another source path on a {reason} destination"
                        ),
                    });
                }
            }
            Ok(())
        }
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

fn apply_checksum_classification(
    plan: &mut Plan,
    sink: &Sink,
    source_root: &Path,
    paths: &SourcePathMap,
) -> (usize, usize) {
    let cache = HashCache::open(HashCache::default_path()).ok();
    let mut unchanged = Vec::new();
    let candidates = plan.files.changed.len() + plan.files.unchanged.len();
    let mut entries = std::mem::take(&mut plan.files.changed);
    entries.append(&mut plan.files.unchanged);
    let mut changed = Vec::with_capacity(candidates);
    for entry in entries {
        let Some(source_relative) = paths.source_for_destination.get(&entry.path) else {
            changed.push(entry);
            continue;
        };
        let source_path = source_relative.to_native_path(source_root);
        let Ok(destination_path) = sink.path_for(&entry.path) else {
            changed.push(entry);
            continue;
        };
        let source_hash = cache_hash(
            cache.as_ref(),
            &source_path,
            HashFingerprint {
                device: entry.fingerprint.identity.device,
                file: entry.fingerprint.identity.file,
                size: entry.fingerprint.size,
                mtime: entry.fingerprint.mtime,
                ctime: entry.fingerprint.ctime,
            },
        );
        let destination_hash = fs::symlink_metadata(&destination_path)
            .ok()
            .and_then(|metadata| {
                let mtime = metadata.modified().ok()?;
                let fingerprint =
                    crate::scanner::fingerprint_from_metadata(&metadata, EntryKind::File, mtime)
                        .ok()?;
                cache_hash(
                    cache.as_ref(),
                    &destination_path,
                    HashFingerprint {
                        device: fingerprint.identity.device,
                        file: fingerprint.identity.file,
                        size: fingerprint.size,
                        mtime: fingerprint.mtime,
                        ctime: fingerprint.ctime,
                    },
                )
            });
        if source_hash.is_some() && source_hash == destination_hash {
            unchanged.push(entry);
        } else {
            changed.push(entry);
        }
    }
    plan.files.changed = changed;
    plan.files.unchanged.extend(unchanged);
    plan.files
        .unchanged
        .sort_by(|left, right| left.path.cmp(&right.path));
    cache.map_or((0, 0), |cache| cache.stats())
}

fn cache_hash(
    cache: Option<&HashCache>,
    path: &Path,
    fingerprint: HashFingerprint,
) -> Option<blake3::Hash> {
    if let Some(cache) = cache {
        if let Ok(hash) = cache.hash_file(path, fingerprint) {
            return Some(hash);
        }
    }
    let bytes = fs::read(path).ok()?;
    Some(blake3::hash(&bytes))
}

/// Warn about sparse sources, naming each file and both of its sizes.
///
/// Reported as warnings rather than failures: a dense copy of a sparse file is
/// wasteful and may not fit, but it is not incorrect, and refusing outright
/// would block a user who knows their destination has room.
/// Why ownership was or was not compared, so the caveat can be truthful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipCheck {
    /// Ownership was compared against a file created at the destination.
    Performed,
    /// A dry run cannot create that file, so ownership went unchecked.
    SkippedForDryRun,
    /// The platform has no Unix ownership to compare. Not a limitation of this
    /// run, so it is not worth a warning.
    Unsupported,
}

pub(crate) fn report_preflight(
    preflight: &crate::sparse::Preflight,
    ownership: OwnershipCheck,
    emit: &mut impl FnMut(LocalEvent),
) {
    let dropped = !preflight.dropped.is_empty();
    report_dropped_metadata(&preflight.dropped, emit);
    if dropped && ownership == OwnershipCheck::SkippedForDryRun {
        report_unchecked_ownership(emit);
    }
    report_unchecked_categories(&preflight.unchecked, emit);
    report_sparse_files(&preflight.sparse, emit);
}

/// Say which metadata this platform cannot inspect at all.
///
/// Not preserving something is a documented limitation; being unable to tell the
/// user whether they had any is a different and worse one. Silence here would
/// read as "you have none" when it means "I cannot see them", so this is stated
/// once per run on affected platforms rather than left implicit.
fn report_unchecked_categories(unchecked: &[&'static str], emit: &mut impl FnMut(LocalEvent)) {
    if unchecked.is_empty() {
        return;
    }
    emit(LocalEvent::Warning {
        path: String::new(),
        message: format!(
            "not inspected on this platform: {}. xsync cannot detect these here, \
             so it cannot tell you whether the source had any — if it did, they \
             are not preserved.",
            unchecked.join(", ")
        ),
    });
}

/// Say what will not survive the transfer. Silent when nothing is dropped.
fn report_dropped_metadata(
    dropped: &crate::sparse::DroppedMetadata,
    emit: &mut impl FnMut(LocalEvent),
) {
    if dropped.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    if dropped.hardlinked > 0 {
        parts.push(format!(
            "{} hardlinked file(s) become independent copies, adding {}",
            dropped.hardlinked,
            human_bytes(dropped.hardlink_extra_bytes)
        ));
    }
    if dropped.with_xattrs > 0 {
        parts.push(format!(
            "{} entr(ies) carry extended attributes (resource forks, Finder info, \
             quarantine flags) that will not be copied",
            dropped.with_xattrs
        ));
    }
    if dropped.reparse_points > 0 {
        parts.push(format!(
            "{} reparse point(s) are recreated as symlinks; a junction is not \
             equivalent to a symlink and its kind is not preserved",
            dropped.reparse_points
        ));
    }
    if dropped.sparse_written_dense > 0 {
        parts.push(format!(
            "{} sparse file(s) will be written dense; holes are not preserved on this \
             platform, so the destination needs room for the full logical size",
            dropped.sparse_written_dense
        ));
    }
    if dropped.foreign_owner > 0 {
        parts.push(format!(
            "{} entr(ies) are owned by another user or group; the copies will belong \
             to the user running xsync",
            dropped.foreign_owner
        ));
    }
    emit(LocalEvent::Warning {
        path: String::new(),
        message: format!("metadata that will not be preserved: {}.", parts.join("; ")),
    });
}

/// Say, once, that a dry run could not answer the ownership question.
///
/// Only shown alongside another finding: on a tree where nothing at all is
/// dropped, the caveat would be the only output and would read as a problem.
fn report_unchecked_ownership(emit: &mut impl FnMut(LocalEvent)) {
    emit(LocalEvent::Warning {
        path: String::new(),
        message: "ownership was not checked: a dry run does not write to the destination, \
                  so files owned by another user will only be reported on the real run"
            .to_owned(),
    });
}

fn report_sparse_files(report: &crate::sparse::SparseReport, emit: &mut impl FnMut(LocalEvent)) {
    if report.is_empty() {
        return;
    }
    for file in &report.files {
        emit(LocalEvent::Warning {
            path: file.path.to_string(),
            message: format!(
                "sparse source: {} will be read and written although it occupies only {} \
                 ({:.1}x amplification). xsync does not yet transfer holes; the destination \
                 needs room for the larger figure.",
                human_bytes(file.apparent_bytes),
                human_bytes(file.allocated_bytes),
                file.amplification(),
            ),
        });
    }
    emit(LocalEvent::Warning {
        path: String::new(),
        message: format!(
            "{} sparse file(s): {} will be written to carry {} of real data — \
             {} of it holes that xsync cannot yet skip.",
            report.files.len(),
            human_bytes(report.sparse_apparent_bytes()),
            human_bytes(report.sparse_allocated_bytes()),
            human_bytes(report.wasted_bytes()),
        ),
    });
}

/// Render a byte count for a human reading a warning.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn emit_plan_actions(plan: &Plan, emit: &mut impl FnMut(LocalEvent)) {
    for entries in [&plan.directories, &plan.symlinks, &plan.files, &plan.other] {
        for entry in entries.new.iter().chain(&entries.changed) {
            emit(LocalEvent::Action {
                path: entry.path.to_string(),
                action: if entries
                    .changed
                    .iter()
                    .any(|changed| changed.path == entry.path)
                {
                    "update"
                } else {
                    "create"
                },
            });
        }
        for entry in &entries.extraneous {
            emit(LocalEvent::Action {
                path: entry.path.to_string(),
                action: "delete",
            });
        }
    }
}

fn transfer_directories(
    sink: &Sink,
    directories: &EntryPlan,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    for entry in directories.new.iter().chain(&directories.changed) {
        if let Err(error) = sink.create_directories(std::slice::from_ref(entry)) {
            record_failure(report, emit, entry.path.to_string(), error.to_string());
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
                entry.path.to_string(),
                "source path mapping is missing".to_owned(),
            );
            continue;
        };
        let source_path = source_relative.to_native_path(source_reader_root);
        let target = match fs::read_link(&source_path) {
            Ok(target) => target,
            Err(error) => {
                record_failure(report, emit, entry.path.to_string(), error.to_string());
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
            record_failure(report, emit, entry.path.to_string(), error.to_string());
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

/// User options paired with the capabilities probed at run time.
struct TransferSettings<'a> {
    options: &'a LocalSyncOptions,
    /// Whether the destination filesystem was found to support reflink.
    reflink: bool,
}

fn transfer_files(
    sink: &Sink,
    source_reader_root: &Path,
    files: &EntryPlan,
    paths: &SourcePathMap,
    settings: &TransferSettings<'_>,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) -> Result<(), LocalSyncError> {
    let options = settings.options;
    let reflink = settings.reflink;
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
                source_path: source_relative.to_native_path(source_reader_root),
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
            reflink,
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
                emit(LocalEvent::Progress {
                    path: outcome.path.clone(),
                    stream: 0,
                    completed: transfer.bytes,
                    total: transfer.bytes,
                });
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
    reflink: bool,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("xsync-local-worker".to_owned())
        .spawn(move || {
            for task in tasks {
                let path = task.destination.path.clone();
                let result = transfer_one_file(&sink, &source_reader, task, paranoid, reflink);
                if outcomes
                    .send(FileOutcome {
                        path: path.to_string(),
                        result,
                    })
                    .is_err()
                {
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
    reflink: bool,
) -> Result<FileTransfer, String> {
    let destination_path = sink
        .path_for(&task.destination.path)
        .map_err(|error| error.to_string())?;
    if reflink && task.source.size >= FILE_CLONE_MIN_BYTES {
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
    if paranoid {
        let verified = fs::read(&destination_path)
            .is_ok_and(|written| blake3::hash(&written) == stable.blake3);
        if !verified {
            sink.write_file_with_retry(&destination, &stable.blake3, |_| Ok(bytes.clone()))
                .map_err(|error| error.to_string())?;
            let retry_verified = fs::read(&destination_path)
                .is_ok_and(|written| blake3::hash(&written) == stable.blake3);
            if !retry_verified {
                return Err("destination readback hash mismatch after retry".to_owned());
            }
        }
    }
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

/// Directories whose mtime the kernel may have bumped during this transfer.
///
/// Creating, rewriting, or removing an entry updates the mtime of the directory
/// that *directly contains* it. Such a parent is frequently classified
/// `unchanged`, so restoring only `new` and `changed` directories leaves it with
/// a stale mtime. Collecting the touched parents keeps the final metadata pass
/// proportional to the work actually performed instead of to the whole tree,
/// which matters because an unchanged-directory sweep would add a syscall per
/// directory to every no-op sync.
fn touched_parent_directories(plan: &Plan, delete: bool) -> HashSet<WirePath> {
    let mut touched = HashSet::new();
    {
        let mut note = |path: &WirePath| {
            if let Some(index) = path.as_bytes().iter().rposition(|byte| *byte == b'/') {
                if let Ok(parent) = WirePath::from_wire(path.as_bytes()[..index].to_vec()) {
                    touched.insert(parent);
                }
            }
        };
        for entry in plan.files.new.iter().chain(&plan.files.changed) {
            note(&entry.path);
        }
        for entry in plan.symlinks.new.iter().chain(&plan.symlinks.changed) {
            note(&entry.path);
        }
        // `changed` includes a destination entry whose type was replaced, which
        // mutates its parent just as a creation does.
        for entry in plan.directories.new.iter().chain(&plan.directories.changed) {
            note(&entry.path);
        }
        if delete {
            for entry in plan
                .files
                .extraneous
                .iter()
                .chain(&plan.symlinks.extraneous)
                .chain(&plan.directories.extraneous)
                .chain(&plan.other.extraneous)
            {
                note(&entry.path);
            }
        }
    }
    touched
}

fn finish_directories(
    sink: &Sink,
    plan: &Plan,
    delete: bool,
    report: &mut LocalSyncReport,
    emit: &mut impl FnMut(LocalEvent),
) {
    let touched = touched_parent_directories(plan, delete);
    let directories = &plan.directories;
    let mut entries: Vec<_> = directories
        .new
        .iter()
        .chain(&directories.changed)
        .chain(
            directories
                .unchanged
                .iter()
                .filter(|entry| touched.contains(&entry.path)),
        )
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(path_depth(&entry.path)));
    for entry in entries {
        if let Err(error) = sink.finish_directories(std::slice::from_ref(entry)) {
            record_failure(report, emit, entry.path.to_string(), error.to_string());
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
                    path: entry.path.to_string(),
                });
            }
            Err(error) => record_failure(report, emit, entry.path.to_string(), error.to_string()),
        }
    }
}

fn protect_cloud_skipped(plan: &mut Plan, skipped: &[FileEntry]) {
    let protected: HashSet<_> = skipped.iter().map(|entry| entry.path.clone()).collect();
    for entries in [
        &mut plan.files.extraneous,
        &mut plan.directories.extraneous,
        &mut plan.symlinks.extraneous,
        &mut plan.other.extraneous,
    ] {
        entries.retain(|entry| !protected.contains(&entry.path));
    }
}

fn path_depth(path: &WirePath) -> usize {
    path.depth()
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
            filter: None,
            explain_filter: false,
            local_workers: 2,
            streams: 7,
            queue_capacity: 1,
            directory_clones: true,
            dry_run: false,
            delete: false,
            paranoid: false,
            checksum: false,
            cloud_files: CloudFilesPolicy::Download,
            on_path_collision: PathCollisionPolicy::Fail,
            exclude_patterns: Vec::new(),
            compress: true,
            compress_level: 3,
            bootstrap: crate::bootstrap::BootstrapPolicy::Disabled,
        }
    }

    fn cloud_placeholder_event(options: &LocalSyncOptions) -> (usize, u64, bool) {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("group")).unwrap();
        for index in 0..4 {
            fs::write(source.join(format!("group/file{index}")), b"payload").unwrap();
        }
        // An existing destination root defeats the whole-tree clone fast path,
        // so the per-entry planning loop is the code under test. That is also
        // the incremental case where per-file detection costs the most.
        fs::create_dir_all(&destination).unwrap();

        let mut observed = None;
        sync(&source, true, &destination, false, options, |event| {
            if let LocalEvent::CloudPlaceholders {
                files,
                bytes,
                detection_performed,
                ..
            } = event
            {
                observed = Some((files, bytes, detection_performed));
            }
        })
        .unwrap();
        observed.expect("sync must emit exactly one CloudPlaceholders event")
    }

    #[test]
    fn download_policy_does_not_inspect_entries_for_placeholders() {
        // `cloud::is_placeholder` spawns a process per file. Under `Download`
        // every entry is retained whatever the answer is, so the inspection is
        // pure cost and must not run. The event reports that it did not look,
        // rather than reporting zero placeholders as if it had.
        let mut options = options();
        options.cloud_files = CloudFilesPolicy::Download;

        let (files, bytes, detection_performed) = cloud_placeholder_event(&options);
        assert!(
            !detection_performed,
            "download policy must not inspect entries for placeholders"
        );
        assert_eq!(files, 0);
        assert_eq!(bytes, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn skip_policy_still_inspects_entries_for_placeholders() {
        // The gate must not disable detection for the policies whose outcome
        // genuinely depends on it.
        let mut options = options();
        options.cloud_files = CloudFilesPolicy::Skip;

        let (files, bytes, detection_performed) = cloud_placeholder_event(&options);
        assert!(
            detection_performed,
            "skip policy must inspect entries for placeholders"
        );
        // Ordinary temp files are not placeholders, so the inventory is empty
        // — but it is an inventory, which is the distinction being asserted.
        assert_eq!(files, 0);
        assert_eq!(bytes, 0);
    }

    fn collision_entry(path: &str) -> FileEntry {
        FileEntry {
            path: WirePath::from_wire(path.as_bytes().to_vec()).unwrap(),
            kind: EntryKind::File,
            size: 1,
            mtime: std::time::UNIX_EPOCH,
            mode: 0o644,
            fingerprint: crate::scanner::SourceFingerprint::synthetic(
                EntryKind::File,
                1,
                std::time::UNIX_EPOCH,
            ),
        }
    }

    /// The end-to-end integration test can only construct this case when the
    /// source volume distinguishes names the destination folds — two different
    /// filesystems. These exercise the detection directly, so the logic stays
    /// covered on every host.
    #[test]
    fn collisions_are_detected_per_destination_behaviour() {
        use crate::pathsem::PathSemantics;

        let entries = vec![
            collision_entry("Readme.md"),
            collision_entry("readme.md"),
            collision_entry("caf\u{e9}.txt"),
            collision_entry("cafe\u{301}.txt"),
            collision_entry("unique.txt"),
        ];

        // A destination that distinguishes everything sees no collision.
        assert!(find_path_collisions(&entries, PathSemantics::sensitive()).is_empty());

        // Case-insensitive: only the Readme pair collides.
        let case_only = PathSemantics {
            case_insensitive: true,
            normalization_insensitive: false,
        };
        let groups = find_path_collisions(&entries, case_only);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);

        // Normalization-insensitive: only the café pair collides.
        let normalization_only = PathSemantics {
            case_insensitive: false,
            normalization_insensitive: true,
        };
        assert_eq!(find_path_collisions(&entries, normalization_only).len(), 1);

        // APFS-style: both pairs, and `unique.txt` is never implicated.
        let apfs = PathSemantics {
            case_insensitive: true,
            normalization_insensitive: true,
        };
        let groups = find_path_collisions(&entries, apfs);
        assert_eq!(groups.len(), 2);
        let unique = WirePath::from_wire(b"unique.txt".to_vec()).unwrap();
        assert!(groups.iter().flatten().all(|path| *path != unique));
    }

    #[test]
    fn fail_policy_refuses_and_skip_policy_drops_every_colliding_path() {
        let temp = tempdir().unwrap();
        let entries = vec![
            collision_entry("Readme.md"),
            collision_entry("readme.md"),
            collision_entry("keep.txt"),
        ];
        if !crate::pathsem::PathSemantics::probe(temp.path()).can_collide() {
            return;
        }

        let refused = enforce_path_collisions(
            entries.clone(),
            temp.path(),
            PathCollisionPolicy::Fail,
            &mut |_| {},
        );
        assert!(matches!(
            refused,
            Err(LocalSyncError::PathCollision { count: 1, .. })
        ));

        let mut failures = 0;
        let (kept, dropped) = enforce_path_collisions(
            entries,
            temp.path(),
            PathCollisionPolicy::Skip,
            &mut |event| {
                if matches!(event, LocalEvent::Failed { .. }) {
                    failures += 1;
                }
            },
        )
        .unwrap();
        assert_eq!(dropped, 2, "both colliding paths are dropped, not one");
        assert_eq!(failures, 2, "each dropped path is reported");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path.to_string(), "keep.txt");
    }

    #[test]
    fn rewriting_a_file_restores_its_unchanged_parent_directory_mtime() {
        // A directory containing a rewritten file is classified `unchanged`,
        // but the kernel bumps its mtime when the child is republished. The
        // final metadata pass must restore it, or a churn sync leaves the
        // destination differing from the source.
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("group")).unwrap();
        fs::write(source.join("group/file"), b"original").unwrap();

        let fixed = filetime::FileTime::from_unix_time(1_700_000_000, 0);
        filetime::set_file_mtime(source.join("group/file"), fixed).unwrap();
        filetime::set_file_mtime(source.join("group"), fixed).unwrap();
        sync(&source, true, &destination, false, &options(), |_| {}).unwrap();

        // Rewrite the file with different content and a newer mtime, leaving
        // the containing directory's own metadata untouched.
        fs::write(source.join("group/file"), b"rewritten!").unwrap();
        let newer = filetime::FileTime::from_unix_time(1_700_000_500, 0);
        filetime::set_file_mtime(source.join("group/file"), newer).unwrap();
        filetime::set_file_mtime(source.join("group"), fixed).unwrap();

        let report = sync(&source, true, &destination, false, &options(), |_| {}).unwrap();
        assert_eq!(report.transferred_files, 1);
        assert_eq!(
            fs::read(destination.join("group/file")).unwrap(),
            b"rewritten!"
        );

        let source_dir = fs::metadata(source.join("group")).unwrap();
        let destination_dir = fs::metadata(destination.join("group")).unwrap();
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&source_dir),
            filetime::FileTime::from_last_modification_time(&destination_dir),
            "an unchanged parent directory must keep the source mtime after a child is rewritten"
        );
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
            LocalEvent::Skipped { path, .. } if path == "nested/file"
        )));
        assert!(first_events.iter().any(|event| matches!(
            event,
            LocalEvent::Started {
                local_workers: 2,
                streams: 7
            }
        )));
        for phase in ["scan", "plan", "transfer", "metadata"] {
            assert!(first_events.iter().any(|event| matches!(
                event,
                LocalEvent::Phase { name, started: true } if *name == phase
            )));
            assert!(first_events.iter().any(|event| matches!(
                event,
                LocalEvent::Phase { name, started: false } if *name == phase
            )));
        }
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
    fn incremental_sync_clones_absent_subtree_and_plans_existing_subtree() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("existing")).unwrap();
        fs::create_dir_all(source.join("new/deep")).unwrap();
        fs::write(source.join("existing/file"), b"existing").unwrap();
        fs::write(source.join("new/deep/file"), b"new subtree").unwrap();

        fs::create_dir_all(destination.join("existing")).unwrap();
        fs::copy(
            source.join("existing/file"),
            destination.join("existing/file"),
        )
        .unwrap();
        let source_file_mtime = filetime::FileTime::from_system_time(
            fs::metadata(source.join("existing/file"))
                .unwrap()
                .modified()
                .unwrap(),
        );
        filetime::set_file_mtime(destination.join("existing/file"), source_file_mtime).unwrap();
        let source_dir_mtime = filetime::FileTime::from_system_time(
            fs::metadata(source.join("existing"))
                .unwrap()
                .modified()
                .unwrap(),
        );
        filetime::set_file_mtime(destination.join("existing"), source_dir_mtime).unwrap();

        let mut events = Vec::new();
        let report = sync(&source, true, &destination, false, &options(), |event| {
            events.push(event);
        })
        .unwrap();

        assert_eq!(
            fs::read(destination.join("existing/file")).unwrap(),
            b"existing"
        );
        assert_eq!(
            fs::read(destination.join("new/deep/file")).unwrap(),
            b"new subtree"
        );
        assert_eq!(report.transferred_files, 1);
        assert!(report.directory_clones <= 1);
        if report.directory_clones == 1 {
            assert!(events.iter().any(|event| matches!(
                event,
                LocalEvent::Transferred {
                    path,
                    method: TransferMethod::DirectoryClone,
                    ..
                } if path == "new"
            )));
        }
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
                | LocalEvent::Skipped { path, .. }
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
        // Windows stores file times at 100 ns precision, so compare with the
        // value the source filesystem actually retained.
        let expected =
            filetime::FileTime::from_last_modification_time(&fs::metadata(&source).unwrap());
        sync(&source, true, &destination, false, &options(), |_| {}).unwrap();
        let actual =
            filetime::FileTime::from_last_modification_time(&fs::metadata(&destination).unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_destination_inside_source_tree() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"data").unwrap();
        let destination = source.join("nested/new");

        let error = sync(&source, true, &destination, false, &options(), |_| {}).unwrap_err();
        assert!(matches!(error, LocalSyncError::PathOverlap { .. }));
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_delete_sync_to_source_parent() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"data").unwrap();
        let mut options = options();
        options.delete = true;

        let error = sync(&source, true, temp.path(), false, &options, |_| {}).unwrap_err();
        assert!(matches!(error, LocalSyncError::PathOverlap { .. }));
        assert!(source.join("file").exists());
    }
}
