//! Durable resume checkpoint journal (Story 3.4).
//!
//! The remote receiver persists the set of *verified* large-file ranges it has
//! written to its staging file, in a compact versioned record stored *outside*
//! the published destination tree. A killed sender, receiver, or transport
//! therefore restarts from the last durable checkpoint instead of re-sending
//! ranges that were already verified and acknowledged.
//!
//! The journal is keyed by a deterministic job root (derived from the session
//! job ID) plus a stable file identity (reversible relative path + source
//! fingerprint). If the source fingerprint changes between attempts, the old
//! ranges are invalidated and that file restarts fresh; ranges from two source
//! versions are never combined.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::protocol::ByteRange;
use crate::scanner::{SourceFingerprint, EntryKind};

/// Journal record format version. Bumped on any incompatible layout change.
pub const RESUME_JOURNAL_VERSION: u32 = 1;

const MAGIC: &[u8; 4] = b"XSRJ";
const FILE_SUFFIX: &str = ".js";
/// Maximum ranges persisted for one file, matching the wire resume-page budget.
const MAX_JOURNAL_RANGES: usize = 65_536;

/// Errors produced by resume-journal I/O.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// A filesystem operation needed by the journal failed.
    #[error("resume journal {operation} '{}' failed: {source}", path.display())]
    Io {
        /// Short operation description.
        operation: &'static str,
        /// Path involved in the operation.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// A stored journal record was malformed and is being ignored as stale.
    #[error("resume journal record for '{path}' is malformed: {reason}")]
    Corrupt {
        /// File path the record belonged to.
        path: String,
        /// Why the record was rejected.
        reason: &'static str,
    },
}

/// Stable identity used to key resume ranges for one file.
#[derive(Debug, Clone)]
pub struct ResumeIdentity {
    /// Reversible relative protocol path (raw bytes).
    pub path: Vec<u8>,
    /// Source fingerprint that must match across attempts.
    pub fingerprint: SourceFingerprint,
}

impl ResumeIdentity {
    /// Whether a stored record's fingerprint still matches this identity.
    #[must_use]
    pub fn matches(&self, record: &JournalRecord) -> bool {
        record.size == self.fingerprint.size
            && record.mtime_ns == timestamp_nanos(self.fingerprint.mtime)
            && record.ctime_ns
                == self
                    .fingerprint
                    .ctime
                    .map_or(0, timestamp_nanos)
            && record.identity_device == self.fingerprint.identity.device
            && record.identity_file == self.fingerprint.identity.file
            && record.kind == self.fingerprint.kind
    }
}

/// A validated set of verified ranges persisted for one file.
#[derive(Debug, Clone)]
pub struct JournalRecord {
    /// Declared full file size.
    pub size: u64,
    /// Modification time in nanoseconds since Unix epoch.
    pub mtime_ns: i64,
    /// Change time in nanoseconds, zero when unavailable.
    pub ctime_ns: i64,
    /// Platform device identity.
    pub identity_device: u64,
    /// Platform file identity (inode).
    pub identity_file: u64,
    /// Entry kind.
    pub kind: EntryKind,
    /// Sorted, non-overlapping verified ranges within `size`.
    pub ranges: Vec<ByteRange>,
}

/// A durable, versioned, atomically-updated checkpoint store.
#[derive(Debug, Clone)]
pub struct ResumeJournal {
    /// Directory that owns this job's journal records.
    root: PathBuf,
}

impl ResumeJournal {
    /// Compute the deterministic journal root for a session job ID.
    #[must_use]
    pub fn root_for(job_id: &[u8; 16]) -> PathBuf {
        let digest = blake3::hash(job_id).to_hex().to_string();
        std::env::temp_dir().join(format!("xsync-resume-{}", &digest[..16]))
    }

    /// Open the journal for a job ID, creating its root directory if needed.
    ///
    /// # Errors
    /// Returns [`JournalError::Io`] if the root directory cannot be created.
    pub fn new(job_id: &[u8; 16]) -> Result<Self, JournalError> {
        let root = Self::root_for(job_id);
        fs::create_dir_all(&root).map_err(|source| {
            journal_io("create resume journal root", &root, source)
        })?;
        Ok(Self { root })
    }

    /// The sunburst journal root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Load a validated record for `identity`, ignoring a missing or stale
    /// record.
    ///
    /// # Errors
    /// Returns [`JournalError::Io`] on a read failure.
    pub fn load(&self, identity: &ResumeIdentity) -> Result<Option<JournalRecord>, JournalError> {
        self.load_unlocked(identity)
    }

    /// Load without acquiring the journal lock (for use inside a locked
    /// critical section). Reads of an atomically-renamed record are consistent.
    fn load_unlocked(
        &self,
        identity: &ResumeIdentity,
    ) -> Result<Option<JournalRecord>, JournalError> {
        let path = self.path_for(&identity.path);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(journal_io("read resume journal record", &path, source))
            }
        };
        match decode_record(&bytes) {
            Ok(record) => {
                if identity.matches(&record) {
                    Ok(Some(record))
                } else {
                    // Stale record from a different source version; do not use.
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    /// Persist `ranges` for `identity`, atomically, so the new record is
    /// durable before the corresponding acknowledgement is sent.
    ///
    /// Under a cross-process lock the stored ranges are merged with whatever a
    /// concurrent writer (e.g. another stream in a multi-stream job) has already
    /// persisted, so the record is the *union* of every writer's verified
    /// ranges rather than the last writer's list. `load → merge → write` is not
    /// atomic on its own, which is why every checkpoint, clear, and invalidate
    /// takes the same lock.
    ///
    /// # Errors
    /// Returns [`JournalError::Io`] on a write, lock, or durability failure.
    pub fn checkpoint(
        &self,
        identity: &ResumeIdentity,
        ranges: &[ByteRange],
    ) -> Result<(), JournalError> {
        self.with_lock(|| {
            let existing = self.load_unlocked(identity)?;
            let mut merged_ranges =
                existing.map_or_else(Vec::new, |record| record.ranges);
            let mut merged = merge_ranges(&merged_ranges, ranges);
            std::mem::swap(&mut merged_ranges, &mut merged);
            let mut sorted_ranges = merged_ranges;
            sorted_ranges.sort_by_key(|range| range.offset);

            let record = JournalRecord {
                size: identity.fingerprint.size,
                mtime_ns: timestamp_nanos(identity.fingerprint.mtime),
                ctime_ns: identity.fingerprint.ctime.map_or(0, timestamp_nanos),
                identity_device: identity.fingerprint.identity.device,
                identity_file: identity.fingerprint.identity.file,
                kind: identity.fingerprint.kind,
                ranges: sorted_ranges,
            };
            let bytes = encode_record(&record)?;

            let path = self.path_for(&identity.path);
            let tmp = path.with_extension(format!("tmp{FILE_SUFFIX}"));
            {
                let mut file = File::create(&tmp)
                    .map_err(|source| journal_io("create resume journal temp", &tmp, source))?;
                write_all(&mut file, &tmp, &bytes)?;
                file.sync_all()
                    .map_err(|source| journal_io("sync resume journal temp", &tmp, source))?;
            }
            fs::rename(&tmp, &path)
                .map_err(|source| journal_io("commit resume journal record", &path, source))?;
            sync_parent(&path)
        })
    }

    /// Remove the stored record for `identity` after a file finishes.
    ///
    /// # Errors
    /// Returns [`JournalError::Io`] on a removal or lock failure.
    pub fn clear(&self, identity: &ResumeIdentity) -> Result<(), JournalError> {
        self.with_lock(|| {
            let path = self.path_for(&identity.path);
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(source) => {
                    Err(journal_io("remove resume journal record", &path, source))
                }
            }
        })
    }

    /// Remove a stale record whose fingerprint no longer matches `identity`.
    ///
    /// # Errors
    /// Returns [`JournalError::Io`] on a removal or lock failure.
    pub fn invalidate(&self, identity: &ResumeIdentity) -> Result<(), JournalError> {
        self.clear(identity)
    }

    /// Run `f` with an exclusive lock on this journal's shared lock file, so
    /// cross-process load-merge-write is atomic. The lock file itself is the
    /// only artifact that is never removed.
    fn with_lock<T>(&self, f: impl FnOnce() -> Result<T, JournalError>) -> Result<T, JournalError> {
        let lock_path = self.root.join("lock");
        let mut lock = fslock::LockFile::open(&lock_path)
            .map_err(|source| journal_io("open resume journal lock", &lock_path, source))?;
        lock.lock()
            .map_err(|source| journal_io("acquire resume journal lock", &lock_path, source))?;
        let result = f();
        drop(lock); // fslock releases the lock on drop
        result
    }

    fn path_for(&self, path: &[u8]) -> PathBuf {
        self.root
            .join(format!("{}.{}", blake3::hash(path).to_hex(), FILE_SUFFIX))
    }
}

/// Return the union of the two range lists as a sorted, disjoint byte set.
#[must_use]
pub fn merge_ranges(existing: &[ByteRange], added: &[ByteRange]) -> Vec<ByteRange> {
    // Treat ranges as byte intervals and merge any that touch or overlap.
    let mut points = Vec::new();
    for r in existing.iter().chain(added) {
        if r.length == 0 {
            continue;
        }
        points.push((r.offset, true));
        points.push((r.offset.saturating_add(r.length), false));
    }
    points.sort_unstable();
    let mut merged: Vec<ByteRange> = Vec::new();
    let mut depth = 0usize;
    let mut start = 0u64;
    for (pos, is_start) in points {
        if is_start {
            if depth == 0 {
                start = pos;
            }
            depth += 1;
        } else {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                if let Some(last) = merged.last_mut() {
                    if last.offset.saturating_add(last.length) == pos {
                        last.length = pos - last.offset;
                        continue;
                    }
                }
                merged.push(ByteRange {
                    offset: start,
                    length: pos - start,
                });
            }
        }
    }
    merged
}

/// Compute which 8 MiB-aligned chunks of a file are *not* yet verified.
///
/// `verified` ranges may cover arbitrary offsets; the returned list is the
/// chunk-sized gaps that still require transmission.
#[must_use]
pub fn missing_chunks(
    size: u64,
    chunk_size: u64,
    verified: &[ByteRange],
) -> Vec<ByteRange> {
    let merged = merge_ranges(verified, &[]);
    let mut missing = Vec::new();
    let mut cursor = 0u64;
    for range in merged {
        let range_end = range.offset.saturating_add(range.length);
        if range.offset > cursor {
            append_chunks(&mut missing, size, chunk_size, cursor, range.offset);
        }
        cursor = cursor.max(range_end);
    }
    if cursor < size {
        append_chunks(&mut missing, size, chunk_size, cursor, size);
    }
    missing
}

fn append_chunks(
    out: &mut Vec<ByteRange>,
    size: u64,
    chunk_size: u64,
    start: u64,
    end: u64,
) {
    let mut offset = start;
    while offset < end {
        let length = chunk_size.min(size.saturating_sub(offset)).min(end - offset);
        if length > 0 {
            out.push(ByteRange { offset, length });
        }
        offset += chunk_size;
    }
}

/// Test helper: the set of chunk offsets represented by a range list.
#[must_use]
pub fn covered_chunk_offsets(ranges: &[ByteRange], chunk_size: u64) -> HashSet<u64> {
    let mut set = HashSet::new();
    for range in ranges {
        let mut offset = range.offset;
        let end = range.offset.saturating_add(range.length);
        while offset < end {
            set.insert(offset - (offset % chunk_size));
            offset += chunk_size.min(end - offset);
        }
    }
    set
}

fn timestamp_nanos(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX),
        Err(err) => {
            let nanos = err.duration().as_nanos();
            let neg = i64::try_from(nanos).unwrap_or(i64::MAX);
            -neg
        }
    }
}

fn journal_io(operation: &'static str, path: &Path, source: io::Error) -> JournalError {
    JournalError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn write_all(file: &mut File, path: &Path, bytes: &[u8]) -> Result<(), JournalError> {
    file.write_all(bytes)
        .map_err(|source| journal_io("write resume journal temp", path, source))
}

/// Sync the parent directory so a rename is durable.
fn sync_parent(path: &Path) -> Result<(), JournalError> {
    #[cfg(unix)]
    {
        let dir = File::open(
            path.parent()
                .expect("journal paths always have a parent"),
        )
        .map_err(|source| journal_io("open resume journal directory", path, source))?;
        dir.sync_all()
            .map_err(|source| journal_io("sync resume journal directory", path, source))?;
    }
    Ok(())
}

/// Encode a record into its compact, versioned binary form.
fn encode_record(record: &JournalRecord) -> Result<Vec<u8>, JournalError> {
    let mut out = Vec::with_capacity(48 + record.ranges.len() * 16);
    out.extend_from_slice(MAGIC);
    push_u32(&mut out, RESUME_JOURNAL_VERSION);
    push_u64(&mut out, record.size);
    push_i64(&mut out, record.mtime_ns);
    push_i64(&mut out, record.ctime_ns);
    push_u64(&mut out, record.identity_device);
    push_u64(&mut out, record.identity_file);
    push_u8(&mut out, kind_to_byte(record.kind));
    let count = u32::try_from(record.ranges.len())
        .map_err(|_| JournalError::Io {
            operation: "encode resume journal range count",
            path: PathBuf::new(),
            source: io::Error::new(io::ErrorKind::InvalidData, "too many ranges"),
        })?;
    push_u32(&mut out, count);
    for range in &record.ranges {
        push_u64(&mut out, range.offset);
        push_u64(&mut out, range.length);
    }
    Ok(out)
}

/// Decode and validate a stored record.
fn decode_record(bytes: &[u8]) -> Result<JournalRecord, JournalError> {
    if bytes.len() < 48 || &bytes[0..4] != MAGIC {
        return Err(JournalError::Corrupt {
            path: String::new(),
            reason: "bad magic or short header",
        });
    }
    let mut pos = 4usize;
    let version = read_u32(bytes, &mut pos)?;
    if version != RESUME_JOURNAL_VERSION {
        return Err(JournalError::Corrupt {
            path: String::new(),
            reason: "unsupported version",
        });
    }
    let size = read_u64(bytes, &mut pos)?;
    let mtime_ns = read_i64(bytes, &mut pos)?;
    let ctime_ns = read_i64(bytes, &mut pos)?;
    let identity_device = read_u64(bytes, &mut pos)?;
    let identity_file = read_u64(bytes, &mut pos)?;
    let kind = byte_to_kind(bytes[pos]);
    pos += 1;
    let count = read_u32(bytes, &mut pos)? as usize;
    if count > MAX_JOURNAL_RANGES {
        return Err(JournalError::Corrupt {
            path: String::new(),
            reason: "too many ranges",
        });
    }
    if bytes.len() != pos + count * 16 {
        return Err(JournalError::Corrupt {
            path: String::new(),
            reason: "trailing or truncated range data",
        });
    }
    let mut ranges: Vec<ByteRange> = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = read_u64(bytes, &mut pos)?;
        let length = read_u64(bytes, &mut pos)?;
        let end = offset.checked_add(length).ok_or(JournalError::Corrupt {
            path: String::new(),
            reason: "range overflow",
        })?;
        if length == 0 || end > size {
            return Err(JournalError::Corrupt {
                path: String::new(),
                reason: "out-of-file range",
            });
        }
        if let Some(prev) = ranges.last() {
            let prev_end = prev.offset.saturating_add(prev.length);
            if offset < prev_end {
                return Err(JournalError::Corrupt {
                    path: String::new(),
                    reason: "overlapping ranges",
                });
            }
        }
        ranges.push(ByteRange { offset, length });
    }
    Ok(JournalRecord {
        size,
        mtime_ns,
        ctime_ns,
        identity_device,
        identity_file,
        kind,
        ranges,
    })
}

macro_rules! read_impls {
    ($($name:ident, $t:ty, $size:expr);*) => {
        $(
            fn $name(bytes: &[u8], pos: &mut usize) -> Result<$t, JournalError> {
                if bytes.len() < *pos + $size {
                    return Err(JournalError::Corrupt {
                        path: String::new(),
                        reason: "truncated field",
                    });
                }
                let value = <$t>::from_le_bytes(
                    bytes[*pos..*pos + $size].try_into().unwrap(),
                );
                *pos += $size;
                Ok(value)
            }
        )*
    };
}
read_impls!(read_u32, u32, 4; read_u64, u64, 8; read_i64, i64, 8);

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}
fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn kind_to_byte(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::File => 0,
        EntryKind::Directory => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

fn byte_to_kind(byte: u8) -> EntryKind {
    match byte {
        0 => EntryKind::File,
        1 => EntryKind::Directory,
        2 => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn file(path: &str, mtime_secs: u64) -> ResumeIdentity {
        ResumeIdentity {
            path: path.as_bytes().to_vec(),
            fingerprint: SourceFingerprint::synthetic(
                EntryKind::File,
                24 * 1024 * 1024,
                UNIX_EPOCH + Duration::from_secs(mtime_secs),
            ),
        }
    }

    #[test]
    fn checkpoint_and_load_round_trip() {
        let job_id = [7u8; 16];
        let journal = ResumeJournal::new(&job_id).unwrap();
        let identity = file("large.bin", 100);
        let ranges = vec![
            ByteRange {
                offset: 0,
                length: 8 * 1024 * 1024,
            },
            ByteRange {
                offset: 8 * 1024 * 1024,
                length: 8 * 1024 * 1024,
            },
        ];
        journal.checkpoint(&identity, &ranges).unwrap();

        let loaded = journal.load(&identity).unwrap().expect("record present");
        assert_eq!(loaded.size, 24 * 1024 * 1024);
        assert_eq!(loaded.ranges, ranges);

        journal.clear(&identity).unwrap();
        assert!(journal.load(&identity).unwrap().is_none());
    }

    #[test]
    fn changed_fingerprint_invalidates_stored_ranges() {
        let journal = ResumeJournal::new(&[3u8; 16]).unwrap();
        let identity_a = file("a.bin", 100);
        let identity_b = file("a.bin", 200); // different mtime
        journal
            .checkpoint(
                &identity_a,
                &[ByteRange {
                    offset: 0,
                    length: 1024,
                }],
            )
            .unwrap();

        // Same path, changed source fingerprint: ranges must not be reused.
        assert!(journal.load(&identity_b).unwrap().is_none());
        assert!(journal.load(&identity_a).unwrap().is_some());
    }

    #[test]
    fn checkpoint_merges_across_concurrent_writers() {
        let job_id = [11u8; 16];
        let journal = ResumeJournal::new(&job_id).unwrap();
        let identity = file("big.bin", 100);
        let chunk = 8 * 1024 * 1024u64;

        // Two "writers" (streams) checkpoint disjoint ranges; the stored record
        // must be the union, not the last writer's list.
        journal
            .checkpoint(
                &identity,
                &[ByteRange {
                    offset: 0,
                    length: chunk,
                }],
            )
            .unwrap();
        journal
            .checkpoint(
                &identity,
                &[ByteRange {
                    offset: chunk,
                    length: chunk,
                }],
            )
            .unwrap();

        let loaded = journal.load(&identity).unwrap().expect("record present");
        assert_eq!(loaded.ranges.len(), 2);
        assert_eq!(loaded.ranges[0].offset, 0);
        assert_eq!(loaded.ranges[1].offset, chunk);
    }

    #[test]
    fn missing_chunks_respects_verified_ranges() {
        let size = 24 * 1024 * 1024u64;
        let chunk = 8 * 1024 * 1024u64;

        // Nothing verified -> all chunks.
        let all = missing_chunks(size, chunk, &[]);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].offset, 0);

        // First chunk verified -> remaining two chunks missing.
        let verified = vec![ByteRange {
            offset: 0,
            length: chunk,
        }];
        let missing = missing_chunks(size, chunk, &verified);
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0].offset, chunk);
        assert_eq!(missing[1].offset, 2 * chunk);

        // Two chunks verified -> only the tail missing.
        let verified = vec![
            ByteRange {
                offset: 0,
                length: chunk,
            },
            ByteRange {
                offset: chunk,
                length: chunk,
            },
        ];
        let missing = missing_chunks(size, chunk, &verified);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].offset, 2 * chunk);
    }

    #[test]
    fn covered_chunk_offsets_maps_ranges_to_chunks() {
        let ranges = vec![
            ByteRange {
                offset: 0,
                length: 1024,
            },
            ByteRange {
                offset: 8 * 1024 * 1024,
                length: 4096,
            },
        ];
        let covered = covered_chunk_offsets(&ranges, 8 * 1024 * 1024);
        assert!(covered.contains(&0));
        assert!(covered.contains(&(8 * 1024 * 1024)));
    }
}