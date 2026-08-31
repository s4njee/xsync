//! Metadata-only diff classification for source and destination scans.
//!
//! Destination entries are kept in a bounded in-memory index until an
//! explicit budget is reached. The index then spills to an owned per-run
//! store. Source entries use the same store format so a parallel discovery
//! producer never needs to retain a complete source vector.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions, ReadDir};
use std::io::{self, BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tempfile::{Builder, TempDir};

use crate::path::WirePath;
use crate::scanner::{EntryKind, FileEntry, FileIdentity, SourceFingerprint, UnixMetadata};

/// The default amount of memory reserved for the destination index.
pub const DEFAULT_INDEX_MEMORY_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// Maximum path bytes accepted by the planning store format.
pub const MAX_STORED_PATH_BYTES: usize = 1024 * 1024;

const STORE_PREFIX: &str = ".xsync-planner-";
const STORE_MARKER: &[u8] = b"xsync-planner-store\n";
const STORE_SCHEMA_VERSION: u32 = 2;
// Path length, kind, size, mtime, mode, identity, and the two presence flags
// for the optional change-time and Unix blocks.
const RECORD_FIXED_BYTES: usize = 47;
const CTIME_BYTES: usize = 12;
/// uid, gid, and link count, written only when the source host has them.
const UNIX_BYTES: usize = 16;
const MAX_RECORD_BYTES: usize =
    RECORD_FIXED_BYTES + CTIME_BYTES + UNIX_BYTES + MAX_STORED_PATH_BYTES;
const DEFAULT_SOURCE_SORT_MEMORY_BYTES: usize = 8 * 1024 * 1024;
const MERGE_FAN_IN: usize = 32;

static ACTIVE_STORES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// Configuration shared by a destination index and a source planning spool.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// Maximum estimated in-memory index bytes before spilling.
    pub memory_budget_bytes: usize,
    /// Parent directory for owned per-run stores.
    pub temp_root: PathBuf,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            memory_budget_bytes: DEFAULT_INDEX_MEMORY_BUDGET_BYTES,
            temp_root: std::env::temp_dir(),
        }
    }
}

impl IndexConfig {
    /// Create a configuration using the process temporary directory.
    #[must_use]
    pub fn with_budget(memory_budget_bytes: usize) -> Self {
        Self {
            memory_budget_bytes,
            ..Self::default()
        }
    }

    /// Set the parent directory for per-run stores.
    #[must_use]
    pub fn with_temp_root(mut self, temp_root: impl AsRef<Path>) -> Self {
        self.temp_root = temp_root.as_ref().to_path_buf();
        self
    }
}

/// Errors raised while creating, filling, sorting, or reading a planning
/// store.
#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    /// A filesystem operation failed.
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Relevant filesystem path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A source or destination scan emitted a duplicate canonical path.
    #[error("duplicate planning path '{0}'")]
    DuplicatePath(String),
    /// A path cannot fit in the on-disk record format.
    #[error("planning path exceeds the {MAX_STORED_PATH_BYTES}-byte limit: '{0}'")]
    PathTooLong(String),
    /// A run file is malformed or has an unsupported schema.
    #[error("invalid planning store '{}': {reason}", path.display())]
    CorruptStore {
        /// Corrupt store file.
        path: PathBuf,
        /// Reason the file was rejected.
        reason: String,
    },
    /// A timestamp could not be represented in the portable record format.
    #[error("mtime cannot be represented in the planning store")]
    TimestampOutOfRange,
}

fn io_error(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> PlannerError {
    PlannerError::Io {
        operation,
        path: path.into(),
        source,
    }
}

/// Destination entries keyed by protocol-canonical relative path.
pub struct DestinationIndex {
    backend: DestinationBackend,
    config: IndexConfig,
    estimated_bytes: usize,
}

enum DestinationBackend {
    Memory(BTreeMap<WirePath, FileEntry>),
    Disk(DiskIndex),
}

impl std::fmt::Debug for DestinationIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DestinationIndex")
            .field("len", &self.len())
            .field("disk_backed", &self.is_disk_backed())
            .field("memory_budget_bytes", &self.config.memory_budget_bytes)
            .finish_non_exhaustive()
    }
}

impl Default for DestinationIndex {
    fn default() -> Self {
        Self::new(IndexConfig::default()).expect("default planning store configuration cannot fail")
    }
}

impl DestinationIndex {
    /// Create an index that remains in memory unless explicitly populated with
    /// [`Self::with_budget`]. This is the compatibility constructor for small
    /// unit-test inputs.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            backend: DestinationBackend::Memory(BTreeMap::new()),
            config: IndexConfig {
                memory_budget_bytes: usize::MAX,
                ..IndexConfig::default()
            },
            estimated_bytes: 0,
        }
    }

    /// Create a budget-selected index using the process temporary directory.
    ///
    /// # Errors
    /// Returns an error when the configured temporary directory is invalid.
    pub fn with_budget(memory_budget_bytes: usize) -> Result<Self, PlannerError> {
        Self::new(IndexConfig::with_budget(memory_budget_bytes))
    }

    /// Create a budget-selected index with an explicit temporary directory.
    ///
    /// # Errors
    /// Returns an error when the configured temporary directory is invalid.
    pub fn with_config(config: IndexConfig) -> Result<Self, PlannerError> {
        Self::new(config)
    }

    /// Create a budget-selected index. The per-run directory is created on
    /// demand, so an in-memory run does not leave temporary files behind.
    ///
    /// # Errors
    /// Returns an error when the configured temporary directory is invalid.
    pub fn new(config: IndexConfig) -> Result<Self, PlannerError> {
        if config.temp_root.as_os_str().is_empty() {
            return Err(io_error(
                "use temporary directory",
                &config.temp_root,
                io::Error::new(io::ErrorKind::InvalidInput, "empty temporary directory"),
            ));
        }
        Ok(Self {
            backend: DestinationBackend::Memory(BTreeMap::new()),
            config,
            estimated_bytes: 0,
        })
    }

    /// Insert one scanned destination entry.
    ///
    /// # Errors
    /// Returns an error for duplicate or oversized paths, or when spilling to
    /// the per-run store fails.
    pub fn insert(&mut self, entry: FileEntry) -> Result<(), PlannerError> {
        if entry.path.len() > MAX_STORED_PATH_BYTES {
            return Err(PlannerError::PathTooLong(entry.path.to_string()));
        }

        let duplicate = match &self.backend {
            DestinationBackend::Memory(entries) => entries.contains_key(&entry.path),
            DestinationBackend::Disk(_) => false,
        };
        if duplicate {
            return Err(PlannerError::DuplicatePath(entry.path.to_string()));
        }

        let entry_bytes = estimated_entry_bytes(&entry);
        let should_spill = matches!(&self.backend, DestinationBackend::Memory(_))
            && self.estimated_bytes.saturating_add(entry_bytes) > self.config.memory_budget_bytes;

        if should_spill {
            self.spill_to_disk()?;
        }

        match &mut self.backend {
            DestinationBackend::Memory(entries) => {
                let path = entry.path.clone();
                if entries.insert(path.clone(), entry).is_some() {
                    return Err(PlannerError::DuplicatePath(path.to_string()));
                }
                self.estimated_bytes = self.estimated_bytes.saturating_add(entry_bytes);
            }
            DestinationBackend::Disk(index) => index.append(&entry)?,
        }
        Ok(())
    }

    /// Number of entries accepted by the index.
    #[must_use]
    pub fn len(&self) -> u64 {
        match &self.backend {
            DestinationBackend::Memory(entries) => entries.len() as u64,
            DestinationBackend::Disk(index) => index.len,
        }
    }

    /// Whether the index contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether this run has spilled to disk.
    #[must_use]
    pub fn is_disk_backed(&self) -> bool {
        matches!(self.backend, DestinationBackend::Disk(_))
    }

    fn spill_to_disk(&mut self) -> Result<(), PlannerError> {
        let mut index = DiskIndex::new(&self.config, self.sort_memory_bytes())?;
        let old_entries = match std::mem::replace(
            &mut self.backend,
            DestinationBackend::Memory(BTreeMap::new()),
        ) {
            DestinationBackend::Memory(entries) => entries,
            DestinationBackend::Disk(existing) => {
                self.backend = DestinationBackend::Disk(existing);
                return Ok(());
            }
        };
        for entry in old_entries.into_values() {
            index.append(&entry)?;
        }
        self.estimated_bytes = 0;
        self.backend = DestinationBackend::Disk(index);
        Ok(())
    }

    fn sort_memory_bytes(&self) -> usize {
        self.config
            .memory_budget_bytes
            .clamp(1, DEFAULT_SOURCE_SORT_MEMORY_BYTES)
    }

    fn finish(self) -> Result<SortedEntries, PlannerError> {
        match self.backend {
            DestinationBackend::Memory(entries) => {
                SortedEntries::memory(entries.into_values().collect::<Vec<_>>())
            }
            DestinationBackend::Disk(index) => index.finish(),
        }
    }

    fn config(&self) -> IndexConfig {
        self.config.clone()
    }
}

/// Create a budget-selected destination index from an entry stream.
///
/// This infallible function is retained for the original planner API and uses
/// [`IndexConfig::default`]. New scan pipelines should use
/// [`try_build_destination_index`] so filesystem failures while spilling are
/// returned instead of being hidden.
///
/// # Panics
/// Panics if the compatibility input contains an invalid or duplicate path.
#[must_use]
pub fn build_destination_index(entries: impl IntoIterator<Item = FileEntry>) -> DestinationIndex {
    let mut index = DestinationIndex::new(IndexConfig::default())
        .expect("default planning store configuration cannot fail");
    for entry in entries {
        index
            .insert(entry)
            .expect("in-memory destination index cannot fail");
    }
    index
}

/// Build a destination index using the explicit memory budget and run-store
/// configuration.
///
/// # Errors
/// Returns an error for invalid paths, duplicate paths, or run-store I/O
/// failures.
pub fn try_build_destination_index(
    entries: impl IntoIterator<Item = FileEntry>,
    config: IndexConfig,
) -> Result<DestinationIndex, PlannerError> {
    let mut index = DestinationIndex::new(config)?;
    for entry in entries {
        index.insert(entry)?;
    }
    Ok(index)
}

/// An owned source discovery spool. Entries are appended immediately and
/// externally sorted during [`Self::finish`], keeping source discovery memory
/// bounded independently of the number of filesystem entries.
pub struct PlanningSpool {
    store: RunStore,
    records_path: PathBuf,
    writer: Option<BufWriter<File>>,
    sort_memory_bytes: usize,
    len: u64,
}

impl std::fmt::Debug for PlanningSpool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanningSpool")
            .field("len", &self.len)
            .field("sort_memory_bytes", &self.sort_memory_bytes)
            .finish_non_exhaustive()
    }
}

impl PlanningSpool {
    /// Create a source spool using the default budget and temporary directory.
    ///
    /// # Errors
    /// Returns an error when the temporary store cannot be created.
    pub fn new() -> Result<Self, PlannerError> {
        Self::with_config(IndexConfig::default())
    }

    /// Create a source spool with a bounded sort budget.
    ///
    /// # Errors
    /// Returns an error when the temporary store cannot be created.
    pub fn with_budget(memory_budget_bytes: usize) -> Result<Self, PlannerError> {
        Self::with_config(IndexConfig::with_budget(memory_budget_bytes))
    }

    /// Create a source spool with an explicit temporary directory.
    ///
    /// # Errors
    /// Returns an error when the temporary store cannot be created.
    pub fn with_config(config: IndexConfig) -> Result<Self, PlannerError> {
        let IndexConfig {
            memory_budget_bytes,
            temp_root,
        } = config;
        let sort_memory_bytes = memory_budget_bytes.clamp(1, DEFAULT_SOURCE_SORT_MEMORY_BYTES);
        let mut store = RunStore::new(&temp_root)?;
        let records_path = store.new_path("source-records")?;
        let file = create_new_file(&records_path)?;
        Ok(Self {
            store,
            records_path: records_path.clone(),
            writer: Some(BufWriter::new(file)),
            sort_memory_bytes,
            len: 0,
        })
    }

    /// Append one discovered source entry.
    ///
    /// # Errors
    /// Returns an error for an oversized path or a write failure.
    #[allow(clippy::needless_pass_by_value)]
    pub fn push(&mut self, entry: FileEntry) -> Result<(), PlannerError> {
        write_entry(
            self.writer
                .as_mut()
                .ok_or_else(|| corrupt(&self.records_path, "spool writer is closed"))?,
            &entry,
        )?;
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    /// Number of entries appended to this spool.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether no entries have been appended to this spool.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Finish discovery and return a deterministic, streaming source cursor.
    ///
    /// # Errors
    /// Returns an error when the spool cannot be flushed, sorted, or read.
    pub fn finish(mut self) -> Result<PlanningSource, PlannerError> {
        flush_writer(&mut self.writer, &self.records_path)?;
        let sorted_path =
            sort_records(&mut self.store, &self.records_path, self.sort_memory_bytes)?;
        Ok(PlanningSource {
            entries: SortedEntries::disk(sorted_path, self.store)?,
        })
    }
}

/// Streaming source cursor returned by [`PlanningSpool::finish`].
pub struct PlanningSource {
    entries: SortedEntries,
}

impl Iterator for PlanningSource {
    type Item = Result<FileEntry, PlannerError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }
}

/// Classify source entries against a bounded destination index.
///
/// Source entries are first written to an owned planning spool. Classification
/// then merges sorted source and destination streams, which makes every bucket
/// deterministic and keeps working memory independent of corpus size. The
/// returned [`Plan`] necessarily owns its classification output; callers that
/// need a streaming transfer path can use [`classify_stream`] instead.
///
/// # Errors
/// Returns an error for malformed entries, duplicate paths, or run-store I/O
/// failures.
pub fn try_plan(
    source_entries: impl IntoIterator<Item = FileEntry>,
    destination: DestinationIndex,
) -> Result<Plan, PlannerError> {
    try_plan_with_fingerprint(source_entries, destination, false, cfg!(unix))
}

/// Classify entries using the content fingerprint when both sides provide one.
///
/// # Errors
/// Returns an error for malformed entries, duplicate paths, or run-store I/O
/// failures.
pub fn try_plan_with_fingerprint(
    source_entries: impl IntoIterator<Item = FileEntry>,
    destination: DestinationIndex,
    compare_fingerprint: bool,
    compare_modes: bool,
) -> Result<Plan, PlannerError> {
    let config = destination.config();
    let mut source_spool = PlanningSpool::with_config(IndexConfig {
        memory_budget_bytes: config
            .memory_budget_bytes
            .clamp(1, DEFAULT_SOURCE_SORT_MEMORY_BYTES),
        temp_root: config.temp_root,
    })?;
    for entry in source_entries {
        source_spool.push(entry)?;
    }
    try_plan_spooled_with_fingerprint(source_spool, destination, compare_fingerprint, compare_modes)
}

/// Classify a source spool after both source and destination discovery have
/// completed successfully.
///
/// # Errors
/// Returns an error when either spool is malformed or cannot be read, or when
/// duplicate paths are encountered.
pub fn try_plan_spooled(
    source_spool: PlanningSpool,
    destination: DestinationIndex,
) -> Result<Plan, PlannerError> {
    try_plan_spooled_with_fingerprint(source_spool, destination, false, cfg!(unix))
}

fn try_plan_spooled_with_fingerprint(
    source_spool: PlanningSpool,
    destination: DestinationIndex,
    compare_fingerprint: bool,
    compare_modes: bool,
) -> Result<Plan, PlannerError> {
    let mut source = source_spool.finish()?;
    let mut destination = destination.finish()?;
    let mut plan = Plan::default();
    classify_cursors(
        &mut source,
        &mut destination,
        compare_fingerprint,
        compare_modes,
        |entry, action| {
            push_classification(&mut plan, entry, action);
            Ok(())
        },
    )?;
    Ok(plan)
}

/// Classify entries and invoke a callback for each result in canonical path
/// order. The callback is called only after source spooling and destination
/// sorting have succeeded, so a destination scan can be completed and checked
/// before transfer work starts.
///
/// # Errors
/// Returns an error from spooling, sorting, duplicate detection, or the
/// callback.
pub fn classify_stream(
    source_entries: impl IntoIterator<Item = FileEntry>,
    destination: DestinationIndex,
    compare_modes: bool,
    mut callback: impl FnMut(FileEntry, Classification) -> Result<(), PlannerError>,
) -> Result<(), PlannerError> {
    let config = destination.config();
    let mut source_spool = PlanningSpool::with_config(IndexConfig {
        memory_budget_bytes: config
            .memory_budget_bytes
            .clamp(1, DEFAULT_SOURCE_SORT_MEMORY_BYTES),
        temp_root: config.temp_root,
    })?;
    for entry in source_entries {
        source_spool.push(entry)?;
    }
    let mut source = source_spool.finish()?;
    let mut destination = destination.finish()?;
    classify_cursors(&mut source, &mut destination, false, compare_modes, |entry, action| {
        callback(entry, action)
    })
}

/// Share of a destination that may be deleted before the run is refused.
///
/// The accident this exists to prevent is a mirror job whose *source* failed to
/// mount: the scan finds nothing, every destination entry classifies as
/// extraneous, and `--delete` wipes the backup. A source that legitimately lost
/// most of its contents is rare; a source that failed to appear is not.
const SUSPICIOUS_DELETE_SHARE: f64 = 0.5;

/// Below this many entries the share test is not applied.
///
/// Small destinations hit high percentages honestly — emptying a four-file
/// directory is 100% and completely ordinary.
const SUSPICIOUS_DELETE_FLOOR: usize = 100;

/// What a `--delete` run would remove, and how much of the destination that is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeletionAudit {
    /// Entries that would be removed.
    pub to_delete: usize,
    /// Destination entries the plan knows about, deletions included.
    pub destination_entries: usize,
}

impl DeletionAudit {
    /// Fraction of the known destination this deletion would remove.
    #[must_use]
    pub fn share(&self) -> f64 {
        if self.destination_entries == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.to_delete as f64 / self.destination_entries as f64
        }
    }
}

/// Why a deletion set was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DeletionRefused {
    /// More entries than `--max-delete` allows.
    #[error("--delete would remove {to_delete} entries, over the --max-delete limit of {limit}; nothing was deleted")]
    OverLimit {
        /// Entries the run wanted to remove.
        to_delete: usize,
        /// The limit the user set.
        limit: usize,
    },
    /// A suspicious share of the destination, with no explicit authorization.
    #[error(
        "--delete would remove {to_delete} of {destination_entries} destination entries ({percent:.0}%); \
         refusing without authorization. If the source is intentionally this much smaller, re-run with \
         --max-delete {to_delete} (or higher). A source that failed to mount looks exactly like this."
    )]
    Suspicious {
        /// Entries the run wanted to remove.
        to_delete: usize,
        /// Destination entries the plan knows about.
        destination_entries: usize,
        /// Share, as a percentage, for the message.
        percent: f64,
    },
}

/// Decide whether a plan's deletions may proceed.
///
/// Called before the first removal on every route, so a refusal costs nothing
/// but the transfer that already happened.
///
/// # Errors
///
/// Returns [`DeletionRefused`] when the set exceeds `--max-delete`, or when it
/// covers a suspicious share of the destination and no limit was given.
pub fn authorize_deletions(
    plan: &Plan,
    max_delete: Option<usize>,
) -> Result<DeletionAudit, DeletionRefused> {
    let kinds = [&plan.files, &plan.directories, &plan.symlinks, &plan.other];
    let to_delete: usize = kinds.iter().map(|k| k.extraneous.len()).sum();
    let destination_entries: usize = kinds
        .iter()
        .map(|k| k.extraneous.len() + k.unchanged.len() + k.changed.len() + k.metadata.len())
        .sum();
    let audit = DeletionAudit {
        to_delete,
        destination_entries,
    };
    if let Some(limit) = max_delete {
        // An explicit limit is also an explicit authorization: honour it and
        // skip the share test entirely.
        if to_delete > limit {
            return Err(DeletionRefused::OverLimit { to_delete, limit });
        }
        return Ok(audit);
    }
    if to_delete >= SUSPICIOUS_DELETE_FLOOR && audit.share() >= SUSPICIOUS_DELETE_SHARE {
        return Err(DeletionRefused::Suspicious {
            to_delete,
            destination_entries,
            percent: audit.share() * 100.0,
        });
    }
    Ok(audit)
}

/// The classification applied to one source or destination entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The source entry has no destination counterpart.
    New,
    /// The source entry differs in kind, size, or mtime.
    Changed,
    /// The source and destination metadata match.
    Unchanged,
    /// Content matches, but the permission bits differ and can be repaired
    /// without moving any data.
    MetadataOnly,
    /// The destination entry has no source counterpart.
    Extraneous,
}

/// Classify using the original infallible planner API.
///
/// The compatibility wrapper is suitable for already-materialized in-memory
/// inputs. New filesystem pipelines should use [`try_plan`] or
/// [`classify_stream`] and handle [`PlannerError`].
///
/// # Panics
/// Panics if planning encounters malformed storage or filesystem I/O. Use
/// [`try_plan`] when those errors must be handled.
#[must_use]
pub fn plan(
    source_entries: impl IntoIterator<Item = FileEntry>,
    destination: DestinationIndex,
) -> Plan {
    try_plan(source_entries, destination).expect("planning failed")
}

fn classify_cursors(
    source: &mut PlanningSource,
    destination: &mut SortedEntries,
    compare_fingerprint: bool,
    compare_modes: bool,
    mut emit: impl FnMut(FileEntry, Classification) -> Result<(), PlannerError>,
) -> Result<(), PlannerError> {
    let mut source_entry = next_unique(source)?;
    let mut destination_entry = destination.next_unique()?;
    loop {
        match (&source_entry, &destination_entry) {
            (None, None) => return Ok(()),
            (Some(_), None) => {
                let entry = source_entry
                    .take()
                    .expect("source entry was matched as present");
                emit(entry, Classification::New)?;
                source_entry = next_unique(source)?;
            }
            (None, Some(_)) => {
                let entry = destination_entry
                    .take()
                    .expect("destination entry was matched as present");
                emit(entry, Classification::Extraneous)?;
                destination_entry = destination.next_unique()?;
            }
            (Some(source_value), Some(destination_value)) => {
                match source_value.path.cmp(&destination_value.path) {
                    std::cmp::Ordering::Less => {
                        let entry = source_entry
                            .take()
                            .expect("source entry was matched as present");
                        emit(entry, Classification::New)?;
                        source_entry = next_unique(source)?;
                    }
                    std::cmp::Ordering::Greater => {
                        let entry = destination_entry
                            .take()
                            .expect("destination entry was matched as present");
                        emit(entry, Classification::Extraneous)?;
                        destination_entry = destination.next_unique()?;
                    }
                    std::cmp::Ordering::Equal => {
                        let source_value = source_entry
                            .take()
                            .expect("source entry was matched as present");
                        let destination_value = destination_entry
                            .take()
                            .expect("destination entry was matched as present");
                        let classification = match difference(
                            &source_value,
                            &destination_value,
                            compare_fingerprint,
                            compare_modes,
                        ) {
                            Difference::None => Classification::Unchanged,
                            Difference::ModeOnly => Classification::MetadataOnly,
                            Difference::Content => Classification::Changed,
                        };
                        emit(source_value, classification)?;
                        source_entry = next_unique(source)?;
                        destination_entry = destination.next_unique()?;
                    }
                }
            }
        }
    }
}

fn next_unique(source: &mut PlanningSource) -> Result<Option<FileEntry>, PlannerError> {
    source.entries.next_unique()
}

fn push_classification(plan: &mut Plan, entry: FileEntry, classification: Classification) {
    let entries = plan.entries_mut(entry.kind);
    match classification {
        Classification::New => entries.new.push(entry),
        Classification::Changed => entries.changed.push(entry),
        Classification::Unchanged => entries.unchanged.push(entry),
        Classification::MetadataOnly => entries.metadata.push(entry),
        Classification::Extraneous => entries.extraneous.push(entry),
    }
}

/// Whether two modification times refer to the same instant, allowing for the
/// fact that filesystems disagree about how finely they store one.
///
/// APFS and ext4 keep nanoseconds, NTFS keeps 100-nanosecond ticks, HFS+ and
/// ext3 keep whole seconds. A timestamp therefore does not survive a round trip
/// between two different filesystems: writing an APFS mtime of `...126080149`
/// to NTFS reads back as `...126080100`. Comparing at full precision reported
/// every such file as modified on every run, so an incremental sync between
/// unlike filesystems degenerated into a full re-transfer.
///
/// Whole seconds is the granularity rsync compares at, and it is coarse enough
/// to absorb every quantisation in the table above except FAT's two-second
/// resolution, which needs an explicit `--modify-window`.
fn mtimes_match(source: SystemTime, destination: SystemTime) -> bool {
    whole_seconds(source) == whole_seconds(destination)
}

/// Seconds since the Unix epoch, rounding toward negative infinity so that
/// times on either side of the epoch compare consistently.
fn whole_seconds(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            let seconds = i128::from(duration.as_secs());
            if duration.subsec_nanos() == 0 {
                -seconds
            } else {
                -(seconds + 1)
            }
        }
    }
}

/// How a source entry differs from the destination entry with the same path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Difference {
    /// Nothing to do.
    None,
    /// Content is identical; only the permission bits drifted.
    ModeOnly,
    /// Kind, size, time or content differs.
    Content,
}

/// Compare one pair, including permissions when both ends can represent them.
///
/// `compare_modes` is false whenever either endpoint synthesizes its mode —
/// `permission_mode` invents `0o755`/`0o644` on non-Unix hosts, and comparing an
/// invented mode with a real one would classify every file as drifted forever.
fn difference(
    source: &FileEntry,
    destination: &FileEntry,
    compare_fingerprint: bool,
    compare_modes: bool,
) -> Difference {
    if !metadata_matches(source, destination, compare_fingerprint) {
        return Difference::Content;
    }
    // Symlink permission bits are not portably settable, so drift on them is
    // not repairable and reporting it would only produce noise.
    let mode_bearing = matches!(source.kind, EntryKind::File | EntryKind::Directory);
    if compare_modes && mode_bearing && source.mode != destination.mode {
        return Difference::ModeOnly;
    }
    Difference::None
}

fn metadata_matches(
    source: &FileEntry,
    destination: &FileEntry,
    compare_fingerprint: bool,
) -> bool {
    if source.kind != destination.kind || source.size != destination.size {
        return false;
    }
    // `--checksum` classifies by content hash *instead of* size+mtime, so the
    // timestamp heuristic is skipped entirely rather than added to. Retaining it
    // would make the flag useless exactly where it is needed most: a source and
    // destination whose filesystems store timestamps at different granularity.
    if compare_fingerprint && source.kind == EntryKind::File {
        return source.fingerprint.identity == destination.fingerprint.identity;
    }
    mtimes_match(source.mtime, destination.mtime)
}

/// Entries of one filesystem kind grouped by their required action.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EntryPlan {
    /// Entries absent from the destination.
    pub new: Vec<FileEntry>,
    /// Entries whose type, size, or modification time differs.
    pub changed: Vec<FileEntry>,
    /// Entries with matching type, size, and modification time.
    pub unchanged: Vec<FileEntry>,
    /// Entries whose content matches but whose permission bits drifted.
    ///
    /// Repaired with a metadata-only operation; no content is retransferred.
    pub metadata: Vec<FileEntry>,
    /// Destination entries absent from the source.
    pub extraneous: Vec<FileEntry>,
}

/// A complete metadata diff, separated by filesystem object kind.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Regular file actions.
    pub files: EntryPlan,
    /// Directory actions.
    pub directories: EntryPlan,
    /// Symbolic-link actions.
    pub symlinks: EntryPlan,
    /// Actions for platform-specific objects unsupported by v1 transfer.
    pub other: EntryPlan,
}

impl Plan {
    fn entries_mut(&mut self, kind: EntryKind) -> &mut EntryPlan {
        match kind {
            EntryKind::File => &mut self.files,
            EntryKind::Directory => &mut self.directories,
            EntryKind::Symlink => &mut self.symlinks,
            EntryKind::Other => &mut self.other,
        }
    }
}

struct DiskIndex {
    store: RunStore,
    records_path: PathBuf,
    writer: Option<BufWriter<File>>,
    sort_memory_bytes: usize,
    len: u64,
}

impl DiskIndex {
    fn new(config: &IndexConfig, sort_memory_bytes: usize) -> Result<Self, PlannerError> {
        let mut store = RunStore::new(&config.temp_root)?;
        let records_path = store.new_path("destination-records")?;
        let file = create_new_file(&records_path)?;
        Ok(Self {
            store,
            records_path: records_path.clone(),
            writer: Some(BufWriter::new(file)),
            sort_memory_bytes,
            len: 0,
        })
    }

    fn append(&mut self, entry: &FileEntry) -> Result<(), PlannerError> {
        write_entry(
            self.writer
                .as_mut()
                .expect("destination index writer is present"),
            entry,
        )?;
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    fn finish(mut self) -> Result<SortedEntries, PlannerError> {
        flush_writer(&mut self.writer, &self.records_path)?;
        let sorted_path =
            sort_records(&mut self.store, &self.records_path, self.sort_memory_bytes)?;
        SortedEntries::disk(sorted_path, self.store)
    }
}

struct RunStore {
    directory: TempDir,
    next_id: u64,
}

impl RunStore {
    fn new(root: &Path) -> Result<Self, PlannerError> {
        fs::create_dir_all(root).map_err(|source| io_error("create store parent", root, source))?;
        let directory = Builder::new()
            .prefix(STORE_PREFIX)
            .tempdir_in(root)
            .map_err(|source| io_error("create planning store", root, source))?;
        let marker = directory.path().join("marker");
        let mut file = create_new_file(&marker)?;
        file.write_all(STORE_MARKER)
            .and_then(|()| file.write_all(&STORE_SCHEMA_VERSION.to_le_bytes()))
            .map_err(|source| io_error("write planning store marker", marker, source))?;
        file.sync_all()
            .map_err(|source| io_error("sync planning store marker", directory.path(), source))?;
        let store_path = directory.path().to_path_buf();
        active_stores()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(store_path);
        Ok(Self {
            directory,
            next_id: 0,
        })
    }

    fn new_path(&mut self, stem: &str) -> Result<PathBuf, PlannerError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or_else(|| {
            io_error(
                "allocate planning store path",
                self.directory.path(),
                io::Error::other("planning store path counter overflow"),
            )
        })?;
        Ok(self.directory.path().join(format!("{stem}-{id}")))
    }
}

impl Drop for RunStore {
    fn drop(&mut self) {
        active_stores()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.directory.path());
    }
}

fn active_stores() -> &'static Mutex<HashSet<PathBuf>> {
    ACTIVE_STORES.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Remove valid, marker-owned stores left by an interrupted process.
///
/// Stores with an unknown prefix, missing marker, or unsupported schema are
/// left untouched. Active stores in the current process are skipped; stores
/// from an interrupted process can be recovered on the next startup.
///
/// # Errors
/// Returns an error when the store parent or a marker-owned stale store cannot
/// be inspected or removed.
pub fn cleanup_stale_stores(root: impl AsRef<Path>) -> Result<usize, PlannerError> {
    let root = root.as_ref();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(source) => return Err(io_error("read planning store parent", root, source)),
    };
    cleanup_store_entries(entries)
}

fn cleanup_store_entries(entries: ReadDir) -> Result<usize, PlannerError> {
    let mut removed = 0;
    for entry in entries {
        let entry =
            entry.map_err(|source| io_error("read planning store entry", "<directory>", source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect planning store entry", &path, source))?;
        if !file_type.is_dir()
            || !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(STORE_PREFIX))
        {
            continue;
        }
        if !valid_store_marker(&path)? {
            continue;
        }
        if active_stores()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&path)
        {
            continue;
        }
        fs::remove_dir_all(&path)
            .map_err(|source| io_error("remove stale planning store", &path, source))?;
        removed += 1;
    }
    Ok(removed)
}

fn valid_store_marker(directory: &Path) -> Result<bool, PlannerError> {
    let marker = directory.join("marker");
    let bytes = match fs::read(&marker) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io_error("read planning store marker", marker, source)),
    };
    Ok(bytes == marker_bytes())
}

fn marker_bytes() -> Vec<u8> {
    let mut bytes = STORE_MARKER.to_vec();
    bytes.extend_from_slice(&STORE_SCHEMA_VERSION.to_le_bytes());
    bytes
}

struct SortedEntries {
    source: EntrySource,
    _store: Option<RunStore>,
    last_path: Option<WirePath>,
}

enum EntrySource {
    Memory(std::vec::IntoIter<FileEntry>),
    Disk(BufReader<File>, PathBuf),
}

impl SortedEntries {
    fn memory(mut entries: Vec<FileEntry>) -> Result<Self, PlannerError> {
        entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        validate_sorted_entries(&entries)?;
        Ok(Self {
            source: EntrySource::Memory(entries.into_iter()),
            _store: None,
            last_path: None,
        })
    }

    fn disk(path: PathBuf, store: RunStore) -> Result<Self, PlannerError> {
        validate_sorted_file(&path)?;
        Ok(Self {
            source: EntrySource::Disk(open_reader(&path)?, path),
            _store: Some(store),
            last_path: None,
        })
    }

    fn next_unique(&mut self) -> Result<Option<FileEntry>, PlannerError> {
        let entry = match self.next() {
            None => None,
            Some(Ok(entry)) => Some(entry),
            Some(Err(error)) => return Err(error),
        };
        if let Some(entry) = &entry {
            if self.last_path.as_ref() == Some(&entry.path) {
                return Err(PlannerError::DuplicatePath(entry.path.to_string()));
            }
            self.last_path = Some(entry.path.clone());
        }
        Ok(entry)
    }
}

fn validate_sorted_entries(entries: &[FileEntry]) -> Result<(), PlannerError> {
    for pair in entries.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(PlannerError::DuplicatePath(pair[0].path.to_string()));
        }
    }
    Ok(())
}

fn validate_sorted_file(path: &Path) -> Result<(), PlannerError> {
    let mut reader = open_reader(path)?;
    let mut last_path: Option<WirePath> = None;
    while let Some(entry) = read_entry(&mut reader, path)? {
        if last_path.as_ref() == Some(&entry.path) {
            return Err(PlannerError::DuplicatePath(entry.path.to_string()));
        }
        last_path = Some(entry.path);
    }
    Ok(())
}

impl Iterator for SortedEntries {
    type Item = Result<FileEntry, PlannerError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.source {
            EntrySource::Memory(entries) => entries.next().map(Ok),
            EntrySource::Disk(reader, path) => match read_entry(reader, path) {
                Ok(entry) => entry.map(Ok),
                Err(error) => Some(Err(error)),
            },
        }
    }
}

fn sort_records(
    store: &mut RunStore,
    input_path: &Path,
    memory_budget_bytes: usize,
) -> Result<PathBuf, PlannerError> {
    let mut reader = BufReader::new(open_file(input_path)?);
    let mut runs = Vec::new();
    let mut chunk = Vec::new();
    let mut chunk_bytes = 0usize;
    while let Some(entry) = read_entry(&mut reader, input_path)? {
        let entry_bytes = estimated_entry_bytes(&entry);
        if !chunk.is_empty() && chunk_bytes.saturating_add(entry_bytes) > memory_budget_bytes {
            runs.push(write_sorted_run(store, &mut chunk)?);
            chunk_bytes = 0;
        }
        chunk_bytes = chunk_bytes.saturating_add(entry_bytes);
        chunk.push(entry);
    }
    if !chunk.is_empty() {
        runs.push(write_sorted_run(store, &mut chunk)?);
    }
    drop(reader);
    remove_if_present(input_path)?;

    if runs.is_empty() {
        let empty = store.new_path("sorted-records")?;
        create_new_file(&empty)?;
        return Ok(empty);
    }

    while runs.len() > 1 {
        let mut merged = Vec::new();
        for group in runs.chunks(MERGE_FAN_IN) {
            let output = store.new_path("merged-records")?;
            merge_runs(group, &output)?;
            for path in group {
                remove_if_present(path)?;
            }
            merged.push(output);
        }
        runs = merged;
    }
    Ok(runs.pop().expect("nonempty sorted run list"))
}

fn write_sorted_run(
    store: &mut RunStore,
    entries: &mut Vec<FileEntry>,
) -> Result<PathBuf, PlannerError> {
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    let path = store.new_path("sorted-run")?;
    let mut writer = BufWriter::new(create_new_file(&path)?);
    for entry in entries.drain(..) {
        write_entry(&mut writer, &entry)?;
    }
    writer
        .flush()
        .map_err(|source| io_error("write sorted planning run", &path, source))?;
    Ok(path)
}

fn merge_runs(paths: &[PathBuf], output: &Path) -> Result<(), PlannerError> {
    let mut readers = paths
        .iter()
        .map(|path| {
            let reader = BufReader::new(open_file(path)?);
            Ok((reader, path.clone(), None))
        })
        .collect::<Result<Vec<(BufReader<File>, PathBuf, Option<FileEntry>)>, PlannerError>>()?;
    for (reader, path, current) in &mut readers {
        *current = read_entry(reader, path)?;
    }
    let mut writer = BufWriter::new(create_new_file(output)?);
    loop {
        let next_index = readers
            .iter()
            .enumerate()
            .filter_map(|(index, (_, _, entry))| entry.as_ref().map(|entry| (index, &entry.path)))
            .min_by(|left, right| left.1.cmp(right.1))
            .map(|(index, _)| index);
        let Some(index) = next_index else {
            break;
        };
        let (reader, path, current) = &mut readers[index];
        let entry = current.take().expect("selected run has an entry");
        write_entry(&mut writer, &entry)?;
        *current = read_entry(reader, path)?;
    }
    writer
        .flush()
        .map_err(|source| io_error("write merged planning run", output, source))
}

fn estimated_entry_bytes(entry: &FileEntry) -> usize {
    entry.path.len().saturating_add(128)
}

fn create_new_file(path: &Path) -> Result<File, PlannerError> {
    OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error("create planning store file", path, source))
}

fn open_file(path: &Path) -> Result<File, PlannerError> {
    File::open(path).map_err(|source| io_error("open planning store file", path, source))
}

fn open_reader(path: &Path) -> Result<BufReader<File>, PlannerError> {
    Ok(BufReader::new(open_file(path)?))
}

fn flush_writer(writer: &mut Option<BufWriter<File>>, path: &Path) -> Result<(), PlannerError> {
    if let Some(mut writer) = writer.take() {
        writer
            .flush()
            .map_err(|source| io_error("flush planning store", path, source))?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), PlannerError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove planning store file", path, source)),
    }
}

fn write_entry(writer: &mut impl Write, entry: &FileEntry) -> Result<(), PlannerError> {
    let path = entry.path.as_bytes();
    if path.len() > MAX_STORED_PATH_BYTES {
        return Err(PlannerError::PathTooLong(entry.path.to_string()));
    }
    let (mtime_seconds, mtime_nanos) = encode_mtime(entry.mtime)?;
    let ctime = entry.fingerprint.ctime.map(encode_mtime).transpose()?;
    let unix = entry.fingerprint.unix;
    let body_len = RECORD_FIXED_BYTES
        .checked_add(path.len())
        .and_then(|length| length.checked_add(optional_bytes(ctime.is_some(), unix.is_some())))
        .ok_or(PlannerError::TimestampOutOfRange)?;
    let body_len_u32 =
        u32::try_from(body_len).map_err(|_| PlannerError::PathTooLong(entry.path.to_string()))?;
    let path_len =
        u32::try_from(path.len()).map_err(|_| PlannerError::PathTooLong(entry.path.to_string()))?;
    let mut body = Vec::with_capacity(body_len);
    body.extend_from_slice(&path_len.to_le_bytes());
    body.extend_from_slice(path);
    body.push(entry_kind_byte(entry.kind));
    body.extend_from_slice(&entry.size.to_le_bytes());
    body.extend_from_slice(&mtime_seconds.to_le_bytes());
    body.extend_from_slice(&mtime_nanos.to_le_bytes());
    body.extend_from_slice(&entry.mode.to_le_bytes());
    body.extend_from_slice(&entry.fingerprint.identity.device.to_le_bytes());
    body.extend_from_slice(&entry.fingerprint.identity.file.to_le_bytes());
    if let Some((seconds, nanos)) = ctime {
        body.push(1);
        body.extend_from_slice(&seconds.to_le_bytes());
        body.extend_from_slice(&nanos.to_le_bytes());
    } else {
        body.push(0);
    }
    if let Some(unix) = unix {
        body.push(1);
        body.extend_from_slice(&unix.uid.to_le_bytes());
        body.extend_from_slice(&unix.gid.to_le_bytes());
        body.extend_from_slice(&unix.nlink.to_le_bytes());
    } else {
        body.push(0);
    }
    writer
        .write_all(&body_len_u32.to_le_bytes())
        .and_then(|()| writer.write_all(&body))
        .map_err(|source| io_error("write planning record", "<open store>", source))
}

/// Bytes the optional trailing blocks add to a record.
const fn optional_bytes(has_ctime: bool, has_unix: bool) -> usize {
    (if has_ctime { CTIME_BYTES } else { 0 }) + (if has_unix { UNIX_BYTES } else { 0 })
}

fn read_entry(
    reader: &mut BufReader<File>,
    path: &Path,
) -> Result<Option<FileEntry>, PlannerError> {
    let mut length_bytes = [0u8; 4];
    let first_read = reader
        .read(&mut length_bytes)
        .map_err(|source| io_error("read planning record", path, source))?;
    if first_read == 0 {
        return Ok(None);
    }
    if first_read < length_bytes.len() {
        reader
            .read_exact(&mut length_bytes[first_read..])
            .map_err(|source| {
                if source.kind() == io::ErrorKind::UnexpectedEof {
                    corrupt(path, "truncated record length")
                } else {
                    io_error("read planning record", path, source)
                }
            })?;
    }
    let length = u32::from_le_bytes(length_bytes) as usize;
    if !(RECORD_FIXED_BYTES..=MAX_RECORD_BYTES).contains(&length) {
        return Err(corrupt(path, "record length is outside the schema bounds"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|source| {
        if source.kind() == io::ErrorKind::UnexpectedEof {
            corrupt(path, "truncated planning record")
        } else {
            io_error("read planning record", path, source)
        }
    })?;
    decode_entry(&body, path).map(Some)
}

fn decode_entry(body: &[u8], path: &Path) -> Result<FileEntry, PlannerError> {
    if body.len() < RECORD_FIXED_BYTES {
        return Err(corrupt(path, "record is shorter than its fixed fields"));
    }
    let mut cursor = Cursor::new(body);
    let path_len = read_u32(&mut cursor, path)? as usize;
    if path_len > MAX_STORED_PATH_BYTES
        || body.len() < RECORD_FIXED_BYTES + path_len
        || body.len() > RECORD_FIXED_BYTES + path_len + CTIME_BYTES + UNIX_BYTES
    {
        return Err(corrupt(path, "path length does not match record length"));
    }
    let mut path_bytes = vec![0u8; path_len];
    cursor
        .read_exact(&mut path_bytes)
        .map_err(|source| io_error("read planning path", path, source))?;
    let path_string =
        WirePath::from_wire(path_bytes).map_err(|_| corrupt(path, "invalid wire path"))?;
    let kind = entry_kind_from_byte(read_u8(&mut cursor, path)?, path)?;
    let size = read_u64(&mut cursor, path)?;
    let mtime_seconds = read_i64(&mut cursor, path)?;
    let mtime_nanos = read_u32(&mut cursor, path)?;
    let mode = read_u32(&mut cursor, path)?;
    let identity = FileIdentity {
        device: read_u64(&mut cursor, path)?,
        file: read_u64(&mut cursor, path)?,
    };
    let ctime = match read_u8(&mut cursor, path)? {
        0 => None,
        1 => Some(decode_mtime(
            read_i64(&mut cursor, path)?,
            read_u32(&mut cursor, path)?,
        )?),
        _ => return Err(corrupt(path, "invalid change-time marker")),
    };
    let unix = match read_u8(&mut cursor, path)? {
        0 => None,
        1 => Some(UnixMetadata {
            uid: read_u32(&mut cursor, path)?,
            gid: read_u32(&mut cursor, path)?,
            nlink: read_u64(&mut cursor, path)?,
        }),
        _ => return Err(corrupt(path, "invalid unix-metadata marker")),
    };
    let expected_body_len = RECORD_FIXED_BYTES
        .checked_add(path_len)
        .and_then(|length| length.checked_add(optional_bytes(ctime.is_some(), unix.is_some())))
        .ok_or_else(|| corrupt(path, "record length overflow"))?;
    if expected_body_len != body.len() {
        return Err(corrupt(
            path,
            "optional trailing fields do not match record length",
        ));
    }
    let mtime = decode_mtime(mtime_seconds, mtime_nanos)?;
    Ok(FileEntry {
        path: path_string,
        kind,
        size,
        mtime,
        mode,
        fingerprint: SourceFingerprint {
            identity,
            kind,
            size,
            mtime,
            ctime,
            unix,
        },
    })
}

fn corrupt(path: &Path, reason: &str) -> PlannerError {
    PlannerError::CorruptStore {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

fn read_u8(cursor: &mut Cursor<&[u8]>, path: &Path) -> Result<u8, PlannerError> {
    let mut bytes = [0u8; 1];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error("read planning record", path, source))?;
    Ok(bytes[0])
}

fn read_u32(cursor: &mut Cursor<&[u8]>, path: &Path) -> Result<u32, PlannerError> {
    let mut bytes = [0u8; 4];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error("read planning record", path, source))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i64(cursor: &mut Cursor<&[u8]>, path: &Path) -> Result<i64, PlannerError> {
    let mut bytes = [0u8; 8];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error("read planning record", path, source))?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_u64(cursor: &mut Cursor<&[u8]>, path: &Path) -> Result<u64, PlannerError> {
    let mut bytes = [0u8; 8];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error("read planning record", path, source))?;
    Ok(u64::from_le_bytes(bytes))
}

fn entry_kind_byte(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::File => 0,
        EntryKind::Directory => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

fn entry_kind_from_byte(byte: u8, path: &Path) -> Result<EntryKind, PlannerError> {
    match byte {
        0 => Ok(EntryKind::File),
        1 => Ok(EntryKind::Directory),
        2 => Ok(EntryKind::Symlink),
        3 => Ok(EntryKind::Other),
        _ => Err(corrupt(path, "unknown entry kind")),
    }
}

fn encode_mtime(mtime: SystemTime) -> Result<(i64, u32), PlannerError> {
    match mtime.duration_since(UNIX_EPOCH) {
        Ok(duration) => Ok((
            i64::try_from(duration.as_secs()).map_err(|_| PlannerError::TimestampOutOfRange)?,
            duration.subsec_nanos(),
        )),
        Err(error) => {
            let duration = error.duration();
            let seconds =
                i64::try_from(duration.as_secs()).map_err(|_| PlannerError::TimestampOutOfRange)?;
            if duration.subsec_nanos() == 0 {
                Ok((
                    seconds
                        .checked_neg()
                        .ok_or(PlannerError::TimestampOutOfRange)?,
                    0,
                ))
            } else {
                Ok((
                    seconds
                        .checked_add(1)
                        .and_then(i64::checked_neg)
                        .ok_or(PlannerError::TimestampOutOfRange)?,
                    1_000_000_000 - duration.subsec_nanos(),
                ))
            }
        }
    }
}

fn decode_mtime(seconds: i64, nanos: u32) -> Result<SystemTime, PlannerError> {
    if nanos >= 1_000_000_000 {
        return Err(PlannerError::TimestampOutOfRange);
    }
    if seconds >= 0 {
        let seconds = u64::try_from(seconds).map_err(|_| PlannerError::TimestampOutOfRange)?;
        UNIX_EPOCH
            .checked_add(Duration::new(seconds, nanos))
            .ok_or(PlannerError::TimestampOutOfRange)
    } else {
        let magnitude = seconds
            .checked_abs()
            .ok_or(PlannerError::TimestampOutOfRange)?;
        let magnitude = u64::try_from(magnitude).map_err(|_| PlannerError::TimestampOutOfRange)?;
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(magnitude))
            .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
            .ok_or(PlannerError::TimestampOutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::tempdir;

    use super::*;

    fn entry(path: &str, kind: EntryKind, size: u64, mtime_seconds: u64) -> FileEntry {
        FileEntry {
            path: WirePath::from(path),
            kind,
            size,
            mtime: UNIX_EPOCH + Duration::from_secs(mtime_seconds),
            mode: 0o644,
            fingerprint: SourceFingerprint::synthetic(
                kind,
                size,
                UNIX_EPOCH + Duration::from_secs(mtime_seconds),
            ),
        }
    }

    /// The planning spool spills to disk, so anything the record encoding drops
    /// vanishes for large trees while surviving for small ones — a silent,
    /// size-dependent bug. An earlier attempt to carry these fields was
    /// reverted for exactly that reason, so pin the round trip.
    #[test]
    fn unix_ownership_and_link_count_survive_the_record_encoding() {
        let mut source = entry("a.txt", EntryKind::File, 10, 5);
        source.fingerprint.unix = Some(UnixMetadata {
            uid: 501,
            gid: 20,
            nlink: 3,
        });
        source.fingerprint.ctime = Some(UNIX_EPOCH + Duration::from_secs(7));

        let mut buffer = Vec::new();
        write_entry(&mut buffer, &source).unwrap();
        // `write_entry` frames the body with a length prefix; decode the body.
        let decoded = decode_entry(&buffer[4..], Path::new("<test>")).unwrap();

        assert_eq!(decoded.fingerprint.unix, source.fingerprint.unix);
        assert_eq!(decoded.fingerprint.ctime, source.fingerprint.ctime);
        assert_eq!(decoded.fingerprint.identity, source.fingerprint.identity);
    }

    /// The two optional blocks are independent; every combination must frame to
    /// the exact record length or decoding rejects it as corrupt.
    #[test]
    fn every_combination_of_optional_blocks_round_trips() {
        for ctime in [None, Some(UNIX_EPOCH + Duration::from_secs(9))] {
            for unix in [
                None,
                Some(UnixMetadata {
                    uid: 0,
                    gid: 0,
                    nlink: 1,
                }),
            ] {
                let mut source = entry("b.txt", EntryKind::File, 3, 4);
                source.fingerprint.ctime = ctime;
                source.fingerprint.unix = unix;

                let mut buffer = Vec::new();
                write_entry(&mut buffer, &source).unwrap();
                let decoded = decode_entry(&buffer[4..], Path::new("<test>")).unwrap();

                assert_eq!(decoded.fingerprint.ctime, ctime, "ctime {ctime:?}");
                assert_eq!(decoded.fingerprint.unix, unix, "unix {unix:?}");
            }
        }
    }

    /// A chmod changes nothing a content comparison can see, so before this the
    /// destination kept the old mode forever and the run reported "skipped".
    #[test]
    fn a_mode_only_change_is_classified_for_metadata_repair() {
        let source = entry("a.txt", EntryKind::File, 10, 5);
        let mut destination = entry("a.txt", EntryKind::File, 10, 5);
        destination.mode = 0o600;

        assert_eq!(
            difference(&source, &destination, false, true),
            Difference::ModeOnly
        );
        // Content is identical, so this must never become a retransfer.
        assert_ne!(
            difference(&source, &destination, false, true),
            Difference::Content
        );
    }

    /// A peer that synthesizes modes would otherwise report every file as
    /// permanently drifted, re-chmod-ing the whole tree on every run.
    #[test]
    fn mode_drift_is_ignored_when_the_peer_cannot_represent_modes() {
        let source = entry("a.txt", EntryKind::File, 10, 5);
        let mut destination = entry("a.txt", EntryKind::File, 10, 5);
        destination.mode = 0o644 | 0o111;

        assert_eq!(
            difference(&source, &destination, false, false),
            Difference::None
        );
    }

    /// Symlink permission bits are not portably settable, so drift on them is
    /// not repairable and reporting it would be noise.
    #[test]
    fn symlink_mode_drift_is_not_reported() {
        let source = entry("l", EntryKind::Symlink, 0, 5);
        let mut destination = entry("l", EntryKind::Symlink, 0, 5);
        destination.mode = 0o777;

        assert_eq!(
            difference(&source, &destination, false, true),
            Difference::None
        );
    }

    /// Content differences still win: a mode change on top of a size change is
    /// a transfer, not a metadata repair.
    #[test]
    fn a_content_change_outranks_a_mode_change() {
        let source = entry("a.txt", EntryKind::File, 20, 5);
        let mut destination = entry("a.txt", EntryKind::File, 10, 5);
        destination.mode = 0o600;

        assert_eq!(
            difference(&source, &destination, false, true),
            Difference::Content
        );
    }

    fn plan_with(extraneous: usize, kept: usize) -> Plan {
        let mut plan = Plan::default();
        for i in 0..extraneous {
            plan.files
                .extraneous
                .push(entry(&format!("gone{i}"), EntryKind::File, 1, 1));
        }
        for i in 0..kept {
            plan.files
                .unchanged
                .push(entry(&format!("kept{i}"), EntryKind::File, 1, 1));
        }
        plan
    }

    /// The accident this guard exists for: the source fails to mount, so every
    /// destination entry classifies as extraneous and `--delete` wipes a backup.
    #[test]
    fn a_source_that_vanished_is_refused_rather_than_mirrored() {
        let plan = plan_with(500, 0);
        let err = authorize_deletions(&plan, None).unwrap_err();
        assert!(
            matches!(err, DeletionRefused::Suspicious { to_delete: 500, .. }),
            "{err:?}"
        );
    }

    /// An explicit limit is an explicit authorization, and suppresses the share
    /// test — otherwise a deliberate large prune could never be expressed.
    #[test]
    fn an_explicit_limit_authorizes_a_large_deletion() {
        let plan = plan_with(500, 0);
        let audit = authorize_deletions(&plan, Some(500)).expect("authorized");
        assert_eq!(audit.to_delete, 500);
    }

    #[test]
    fn a_limit_below_the_deletion_set_refuses() {
        let plan = plan_with(500, 0);
        let err = authorize_deletions(&plan, Some(100)).unwrap_err();
        assert!(
            matches!(
                err,
                DeletionRefused::OverLimit {
                    to_delete: 500,
                    limit: 100
                }
            ),
            "{err:?}"
        );
    }

    /// Small destinations reach high percentages honestly; emptying a handful of
    /// files is ordinary and must not require a flag.
    #[test]
    fn a_small_destination_is_below_the_floor_and_proceeds() {
        let plan = plan_with(8, 0);
        let audit = authorize_deletions(&plan, None).expect("under the floor");
        assert_eq!(audit.to_delete, 8);
    }

    /// A large but proportionate prune is not the accident and must not be
    /// blocked: 200 of 1,000 is 20%, well under the share threshold.
    #[test]
    fn a_proportionate_prune_of_a_large_destination_proceeds() {
        let plan = plan_with(200, 800);
        let audit = authorize_deletions(&plan, None).expect("under the share");
        assert_eq!(audit.to_delete, 200);
        assert!((audit.share() - 0.2).abs() < 1e-9, "{}", audit.share());
    }

    fn disk_config(root: &Path, budget: usize) -> IndexConfig {
        IndexConfig::with_budget(budget).with_temp_root(root)
    }

    fn assert_same_plan(source: Vec<FileEntry>, destination: Vec<FileEntry>) {
        let memory = try_build_destination_index(
            destination.clone(),
            IndexConfig::with_budget(usize::MAX).with_temp_root(tempdir().unwrap().path()),
        )
        .unwrap();
        let disk_root = tempdir().unwrap();
        let disk =
            try_build_destination_index(destination, disk_config(disk_root.path(), 1)).unwrap();
        let memory_plan = try_plan(source.clone(), memory).unwrap();
        let disk_plan = try_plan(source, disk).unwrap();
        assert_eq!(memory_plan, disk_plan);
    }

    #[test]
    fn identical_file_is_unchanged() {
        let source = entry("same.txt", EntryKind::File, 10, 100);
        let destination = build_destination_index([source.clone()]);

        let plan = plan([source.clone()], destination);

        assert_eq!(plan.files.unchanged, [source]);
        assert!(plan.files.new.is_empty());
        assert!(plan.files.changed.is_empty());
        assert!(plan.files.extraneous.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn disk_planning_preserves_invalid_utf8_path_bytes() {
        let mut source = entry("placeholder", EntryKind::File, 3, 100);
        source.path = WirePath::from_wire(b"bad-\xff-name".to_vec()).unwrap();
        let destination = try_build_destination_index(
            std::iter::empty(),
            IndexConfig::with_budget(1).with_temp_root(tempdir().unwrap().path()),
        )
        .unwrap();
        let plan = try_plan([source.clone()], destination).unwrap();
        assert_eq!(plan.files.new[0].path.as_bytes(), b"bad-\xff-name");
    }

    #[test]
    fn mtimes_matching_within_the_same_second_are_equal() {
        // NTFS truncates to 100 ns ticks, so an APFS source mtime cannot be
        // reproduced exactly on a Windows destination. Both must still compare
        // as unchanged or every sync becomes a full re-transfer.
        let apfs = UNIX_EPOCH + Duration::new(1_787_728_803, 126_080_149);
        let ntfs = UNIX_EPOCH + Duration::new(1_787_728_803, 126_080_100);
        assert!(mtimes_match(apfs, ntfs));

        // HFS+ and ext3 drop the sub-second part entirely.
        let seconds_only = UNIX_EPOCH + Duration::new(1_787_728_803, 0);
        assert!(mtimes_match(apfs, seconds_only));
    }

    #[test]
    fn mtimes_in_different_seconds_still_differ() {
        let earlier = UNIX_EPOCH + Duration::new(1_787_728_803, 999_999_999);
        let later = UNIX_EPOCH + Duration::new(1_787_728_804, 0);
        assert!(!mtimes_match(earlier, later));
    }

    #[test]
    fn mtimes_before_the_epoch_compare_consistently() {
        let before = UNIX_EPOCH - Duration::new(10, 0);
        let same = UNIX_EPOCH - Duration::new(10, 0);
        assert!(mtimes_match(before, same));

        // Rounding toward negative infinity puts -9.5 s and -10.0 s in the same
        // one-second bucket, exactly as 9.5 s and 9.0 s share a bucket after the
        // epoch. Without floor semantics the two sides of the epoch would round
        // in opposite directions.
        let within_same_bucket = UNIX_EPOCH - Duration::new(9, 500_000_000);
        assert!(mtimes_match(before, within_same_bucket));

        // A genuinely different second is still detected.
        let earlier = UNIX_EPOCH - Duration::new(11, 0);
        assert!(!mtimes_match(before, earlier));
    }

    #[test]
    fn checksum_classification_ignores_mtime_entirely() {
        // `--checksum` is documented as classifying by content hash *instead of*
        // size+mtime. A file with identical content but an mtime the destination
        // filesystem could not reproduce must compare as unchanged.
        let mut source = entry("f", EntryKind::File, 10, 1_000);
        let mut destination = entry("f", EntryKind::File, 10, 1_000);
        source.mtime = UNIX_EPOCH + Duration::new(1_787_728_803, 126_080_149);
        destination.mtime = UNIX_EPOCH + Duration::new(9_999, 0);
        source.fingerprint.identity = FileIdentity {
            device: 7,
            file: 42,
        };
        destination.fingerprint.identity = FileIdentity {
            device: 7,
            file: 42,
        };
        assert!(metadata_matches(&source, &destination, true));

        // Differing content still registers as changed.
        destination.fingerprint.identity = FileIdentity {
            device: 7,
            file: 43,
        };
        assert!(!metadata_matches(&source, &destination, true));
    }

    #[test]
    fn size_and_mtime_changes_are_detected() {
        let size_changed = entry("size.txt", EntryKind::File, 20, 100);
        let time_changed = entry("time.txt", EntryKind::File, 10, 200);
        let destination = build_destination_index([
            entry("size.txt", EntryKind::File, 10, 100),
            entry("time.txt", EntryKind::File, 10, 100),
        ]);

        let plan = plan([size_changed.clone(), time_changed.clone()], destination);

        assert_eq!(plan.files.changed, [size_changed, time_changed]);
        assert!(plan.files.unchanged.is_empty());
    }

    #[test]
    fn destination_only_entries_are_extraneous() {
        let destination_only = entry("old.txt", EntryKind::File, 10, 100);
        let destination = build_destination_index([destination_only.clone()]);

        let plan = plan([], destination);

        assert_eq!(plan.files.extraneous, [destination_only]);
    }

    #[test]
    fn type_changes_are_changed_replacements_in_the_source_bucket() {
        let directory = entry("file-to-dir", EntryKind::Directory, 0, 100);
        let symlink = entry("file-to-link", EntryKind::Symlink, 6, 100);
        let destination = build_destination_index([
            entry("file-to-dir", EntryKind::File, 0, 100),
            entry("file-to-link", EntryKind::File, 6, 100),
        ]);

        let plan = plan([directory.clone(), symlink.clone()], destination);

        assert_eq!(plan.directories.changed, [directory]);
        assert_eq!(plan.symlinks.changed, [symlink]);
        assert!(plan.files.extraneous.is_empty());
    }

    #[test]
    fn kinds_are_collected_separately_for_every_classification() {
        let new_file = entry("new-file", EntryKind::File, 1, 100);
        let unchanged_dir = entry("same-dir", EntryKind::Directory, 0, 100);
        let changed_link = entry("changed-link", EntryKind::Symlink, 4, 200);
        let extra_dir = entry("extra-dir", EntryKind::Directory, 0, 100);
        let destination = build_destination_index([
            unchanged_dir.clone(),
            entry("changed-link", EntryKind::Symlink, 3, 100),
            extra_dir.clone(),
        ]);

        let plan = plan(
            [
                new_file.clone(),
                unchanged_dir.clone(),
                changed_link.clone(),
            ],
            destination,
        );

        assert_eq!(plan.files.new, [new_file]);
        assert_eq!(plan.directories.unchanged, [unchanged_dir]);
        assert_eq!(plan.symlinks.changed, [changed_link]);
        assert_eq!(plan.directories.extraneous, [extra_dir]);
    }

    #[test]
    fn default_planning_requires_no_filesystem_access() {
        let metadata_only = entry("path/that/does/not/exist", EntryKind::File, 10, 100);
        let destination = build_destination_index([metadata_only.clone()]);

        let plan = plan([metadata_only.clone()], destination);

        assert_eq!(plan.files.unchanged, [metadata_only]);
    }

    #[test]
    fn tiny_budget_spills_and_matches_memory_plan() {
        let source = vec![
            entry("z.txt", EntryKind::File, 10, 100),
            entry("a.txt", EntryKind::File, 20, 200),
            entry("new.txt", EntryKind::File, 1, 1),
        ];
        let destination = vec![
            entry("a.txt", EntryKind::File, 20, 200),
            entry("z.txt", EntryKind::File, 9, 100),
            entry("old.txt", EntryKind::File, 1, 1),
        ];
        let root = tempdir().unwrap();
        let index = try_build_destination_index(destination, disk_config(root.path(), 1)).unwrap();
        assert!(index.is_disk_backed());
        let result = try_plan(source, index).unwrap();
        assert_eq!(result.files.changed[0].path, "z.txt");
        assert_eq!(result.files.unchanged[0].path, "a.txt");
        assert_eq!(result.files.extraneous[0].path, "old.txt");
    }

    #[test]
    fn disk_and_memory_backends_have_identical_deterministic_results() {
        assert_same_plan(
            vec![
                entry("z", EntryKind::File, 1, 1),
                entry("a", EntryKind::Directory, 0, 2),
                entry("m", EntryKind::Symlink, 3, 3),
            ],
            vec![
                entry("m", EntryKind::Symlink, 3, 3),
                entry("old", EntryKind::Other, 0, 4),
                entry("a", EntryKind::Directory, 0, 1),
            ],
        );
    }

    #[test]
    fn source_spool_round_trips_precise_and_pre_epoch_mtime() {
        let root = tempdir().unwrap();
        let mut spool = PlanningSpool::with_config(disk_config(root.path(), 1)).unwrap();
        let precise = FileEntry {
            path: WirePath::from("precise"),
            kind: EntryKind::Other,
            size: 42,
            mtime: UNIX_EPOCH - Duration::new(2, 500_000_000),
            mode: 0o1755,
            fingerprint: SourceFingerprint::synthetic(
                EntryKind::Other,
                42,
                UNIX_EPOCH - Duration::new(2, 500_000_000),
            ),
        };
        spool.push(precise.clone()).unwrap();
        let mut source = spool.finish().unwrap();
        assert_eq!(source.next().unwrap().unwrap(), precise);
        assert!(source.next().is_none());
    }

    #[test]
    fn duplicate_paths_are_rejected() {
        let root = tempdir().unwrap();
        let result = try_build_destination_index(
            [
                entry("same", EntryKind::File, 1, 1),
                entry("same", EntryKind::File, 2, 1),
            ],
            disk_config(root.path(), usize::MAX),
        );
        assert!(matches!(result, Err(PlannerError::DuplicatePath(path)) if path == "same"));
    }

    #[test]
    fn valid_stale_store_is_cleaned_but_unrelated_directory_is_not() {
        let root = tempdir().unwrap();
        let stale = Builder::new()
            .prefix(STORE_PREFIX)
            .tempdir_in(root.path())
            .unwrap();
        let marker = stale.path().join("marker");
        fs::write(&marker, marker_bytes()).unwrap();
        let stale_path = stale.keep();
        let unrelated = root.path().join(".xsync-planner-unrelated");
        fs::create_dir(&unrelated).unwrap();
        assert_eq!(cleanup_stale_stores(root.path()).unwrap(), 1);
        assert!(!stale_path.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn stale_cleanup_does_not_remove_an_active_store() {
        let root = tempdir().unwrap();
        let spool = PlanningSpool::with_config(disk_config(root.path(), 1)).unwrap();
        assert_eq!(cleanup_stale_stores(root.path()).unwrap(), 0);
        drop(spool);
        assert_eq!(cleanup_stale_stores(root.path()).unwrap(), 0);
    }
}
