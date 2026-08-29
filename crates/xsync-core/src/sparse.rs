//! Preflight inspection: what a transfer will cost, and what it will not preserve.
//!
//! Both questions are answered from one `symlink_metadata` per planned file, so
//! adding the second cost nothing beyond the first.
//!
//! # Sparse sources
//!
//! A sparse file reports a large apparent size while occupying far fewer blocks:
//! the unwritten regions are holes. xsync has no concept of a hole, so it reads
//! and writes every zero. A Docker VM disk measured on this project reports
//! 3,721.9 GB apparent against 130.2 GB allocated — a 28.6x amplification that
//! does not merely run slowly, it exhausts the destination and fails after
//! hours.
//!
//! Sparse-aware transfer is TUNING-TASKS Epic T2. Until then the honest thing is
//! to say so before starting work that cannot finish, which is what this module
//! provides.

use std::collections::HashMap;
use std::path::Path;

use crate::path::WirePath;
use crate::scanner::{EntryKind, FileEntry};

/// Metadata that xsync v1 does not carry to the destination.
///
/// The README documents these as limitations, but a run that says nothing leaves
/// a destination that *looks* complete. Counting them costs nothing on the stat
/// already being made, so the run can say what it is dropping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedMetadata {
    /// Files with more than one link. Each becomes an independent copy, so the
    /// destination holds more bytes than the source and the links are gone.
    pub hardlinked: usize,
    /// Bytes written for the extra copies of hardlinked files.
    pub hardlink_extra_bytes: u64,
    /// Entries carrying extended attributes. On macOS this includes resource
    /// forks, Finder info, and quarantine flags.
    pub with_xattrs: usize,
    /// Sparse files that will be written dense because holes are not preserved.
    ///
    /// Populated on Windows, where the sparse *attribute* is visible through
    /// stable std but the allocated size is not, so the byte saving cannot be
    /// reported the way [`SparseReport`] does on Unix.
    pub sparse_written_dense: usize,
    /// Entries owned by a user or group other than the one running xsync, whose
    /// ownership therefore cannot be reproduced.
    pub foreign_owner: usize,
    /// Reparse points (Windows). Junctions and symlinks share this attribute and
    /// cannot be told apart without reading the reparse tag, so both are counted
    /// together — a junction is recreated as a symlink, which is not equivalent.
    pub reparse_points: usize,
}

impl DroppedMetadata {
    /// Whether anything at all will be lost.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hardlinked == 0
            && self.with_xattrs == 0
            && self.foreign_owner == 0
            && self.sparse_written_dense == 0
            && self.reparse_points == 0
    }
}

/// Files below this apparent size are never inspected for holes.
///
/// Holes are allocated in filesystem blocks, so a small file cannot be
/// meaningfully sparse, and probing every entry of a large tree would cost a
/// second stat per file for no benefit.
pub const SPARSE_PROBE_MIN_BYTES: u64 = 1024 * 1024;

/// A file is reported when it occupies less than this fraction of its apparent
/// size. Chosen loosely: the concern is order-of-magnitude amplification, not a
/// few per cent of tail padding or filesystem compression.
const SPARSE_RATIO: f64 = 0.5;

/// One source file whose allocated size is far below its apparent size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseFile {
    /// Destination-relative path.
    pub path: WirePath,
    /// Size xsync will read and write.
    pub apparent_bytes: u64,
    /// Bytes the file actually occupies on the source.
    pub allocated_bytes: u64,
}

impl SparseFile {
    /// How many times more xsync will write than the source occupies.
    #[must_use]
    pub fn amplification(&self) -> f64 {
        if self.allocated_bytes == 0 {
            return f64::INFINITY;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.apparent_bytes as f64 / self.allocated_bytes as f64
        }
    }
}

/// Everything the preflight pass found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preflight {
    /// Files whose apparent size far exceeds their allocation.
    pub sparse: SparseReport,
    /// Metadata the transfer will not preserve.
    pub dropped: DroppedMetadata,
    /// Metadata categories this platform cannot inspect, so xsync can neither
    /// preserve them nor tell you whether you had any.
    ///
    /// Reporting this is the difference between "you have no hardlinks" and "I
    /// cannot see hardlinks here" — silence would imply the first while meaning
    /// the second.
    pub unchecked: Vec<&'static str>,
}

/// Metadata categories that cannot be inspected on this platform.
///
/// Windows: `number_of_links` is behind the unstable `windows_by_handle`
/// feature, and enumerating alternate data streams needs `FindFirstStreamW`.
/// Both would require an unstable feature or `unsafe` FFI, which this crate
/// denies outside one documented exemption. Measured consequences on NTFS: a
/// hardlinked pair arrives as two independent copies, and an alternate data
/// stream is dropped entirely.
#[must_use]
pub fn unchecked_categories() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["hardlinks", "alternate-data-streams"]
    } else {
        Vec::new()
    }
}

/// What a planned transfer will actually cost in written bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SparseReport {
    /// Files whose apparent size far exceeds their allocation.
    pub files: Vec<SparseFile>,
    /// Bytes xsync will write across every inspected file.
    pub apparent_bytes: u64,
    /// Bytes those files occupy on the source.
    pub allocated_bytes: u64,
}

impl SparseFile {
    /// Bytes written purely to materialize this file's holes.
    #[must_use]
    pub fn wasted_bytes(&self) -> u64 {
        self.apparent_bytes.saturating_sub(self.allocated_bytes)
    }
}

impl SparseReport {
    /// Whether anything worth reporting was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Apparent size of the sparse files alone.
    #[must_use]
    pub fn sparse_apparent_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.apparent_bytes).sum()
    }

    /// Allocation of the sparse files alone.
    #[must_use]
    pub fn sparse_allocated_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.allocated_bytes).sum()
    }

    /// Extra bytes written purely because holes are materialized. Counts only
    /// the sparse files, so a dense file alongside them does not inflate it.
    #[must_use]
    pub fn wasted_bytes(&self) -> u64 {
        self.files.iter().map(SparseFile::wasted_bytes).sum()
    }
}

/// Inspect planned regular files for holes.
///
/// Only files at or above [`SPARSE_PROBE_MIN_BYTES`] are stat'd. Entries that
/// cannot be inspected are skipped rather than guessed at: a missing file is the
/// transfer's problem to report, not this check's.
#[must_use]
pub fn inspect(entries: &[FileEntry], source_root: &Path, owner: Option<Owner>) -> Preflight {
    inspect_with_workers(entries, source_root, owner, 1)
}

/// Inspect `entries`, spreading the per-file syscalls across `workers` threads.
///
/// This pass is two syscalls per transferred file — a `stat` and a `listxattr` —
/// and on a large tree that is not a rounding error: measured serially at 16.4%
/// of a 109,615-file local NVMe-to-NVMe copy. It is also embarrassingly
/// parallel, being pure reporting with no ordering requirement of its own.
///
/// Entries are split into contiguous chunks and merged in chunk order, so the
/// result is byte-identical to the serial version regardless of how the threads
/// interleave — including the hardlink accounting, which depends on which name
/// for an inode is seen first.
#[must_use]
pub fn inspect_with_workers(
    entries: &[FileEntry],
    source_root: &Path,
    owner: Option<Owner>,
    workers: usize,
) -> Preflight {
    let workers = workers.max(1);
    if workers == 1 || entries.len() < PARALLEL_THRESHOLD {
        return merge(
            vec![inspect_chunk(entries, source_root, owner)],
            entries.len(),
        );
    }

    let chunk_size = entries.len().div_ceil(workers);
    let partials: Vec<Partial> = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || inspect_chunk(chunk, source_root, owner)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    merge(partials, entries.len())
}

/// Files below this count are not worth the thread spawns.
const PARALLEL_THRESHOLD: usize = 2_048;

/// One chunk's findings, still carrying enough to merge deterministically.
#[derive(Default)]
struct Partial {
    sparse: SparseReport,
    with_xattrs: usize,
    foreign_owner: usize,
    sparse_written_dense: usize,
    reparse_points: usize,
    /// `(device, inode, size)` for each entry whose inode has more than one
    /// name, in encounter order.
    links: Vec<(u64, u64, u64)>,
}

fn merge(partials: Vec<Partial>, capacity_hint: usize) -> Preflight {
    let mut result = Preflight {
        unchecked: unchecked_categories(),
        ..Preflight::default()
    };
    let mut seen: HashMap<(u64, u64), usize> = HashMap::with_capacity(capacity_hint / 16 + 1);
    for partial in partials {
        result.sparse.apparent_bytes = result
            .sparse
            .apparent_bytes
            .saturating_add(partial.sparse.apparent_bytes);
        result.sparse.allocated_bytes = result
            .sparse
            .allocated_bytes
            .saturating_add(partial.sparse.allocated_bytes);
        result.sparse.files.extend(partial.sparse.files);
        result.dropped.with_xattrs += partial.with_xattrs;
        result.dropped.foreign_owner += partial.foreign_owner;
        result.dropped.sparse_written_dense += partial.sparse_written_dense;
        result.dropped.reparse_points += partial.reparse_points;
        for (device, inode, size) in partial.links {
            result.dropped.hardlinked += 1;
            let count = seen.entry((device, inode)).or_insert(0_usize);
            *count += 1;
            // Extra bytes are only real when a second name for the same inode
            // is itself in the transfer: that is when one shared inode becomes
            // two independent copies.
            if *count > 1 {
                result.dropped.hardlink_extra_bytes =
                    result.dropped.hardlink_extra_bytes.saturating_add(size);
            }
        }
    }
    result
        .sparse
        .files
        .sort_by_key(|file| std::cmp::Reverse(file.apparent_bytes));
    result
}

fn inspect_chunk(entries: &[FileEntry], source_root: &Path, owner: Option<Owner>) -> Partial {
    let mut partial = Partial::default();
    for entry in entries {
        let path = entry.path.to_native_path(source_root);
        note_dropped_metadata(&mut partial, entry, &path, owner);
        #[cfg(windows)]
        note_reparse_point(&mut partial, &path);

        if entry.kind != EntryKind::File || entry.size < SPARSE_PROBE_MIN_BYTES {
            continue;
        }
        // Allocation is not in the fingerprint and is only asked of files large
        // enough to be worth a stat.
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(allocated) = allocated_bytes(&metadata) else {
            continue;
        };
        partial.sparse.apparent_bytes = partial.sparse.apparent_bytes.saturating_add(entry.size);
        partial.sparse.allocated_bytes = partial.sparse.allocated_bytes.saturating_add(allocated);
        #[allow(clippy::cast_precision_loss)]
        let sparse = (allocated as f64) < (entry.size as f64) * SPARSE_RATIO;
        if sparse {
            partial.sparse.files.push(SparseFile {
                path: entry.path.clone(),
                apparent_bytes: entry.size,
                allocated_bytes: allocated,
            });
        }
    }
    partial
}

/// The user and group a transfer runs as, used to tell whether ownership can be
/// reproduced. `None` on platforms without Unix ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    /// Effective user id.
    pub uid: u32,
    /// Effective group id.
    pub gid: u32,
}

impl Owner {
    /// Determine the running user by creating a file and reading back its owner.
    ///
    /// Avoids a second `unsafe` block for `geteuid`, and answers the question
    /// that actually matters — who newly created files will belong to —
    /// rather than inferring it.
    #[must_use]
    pub fn probe(directory: &Path) -> Option<Self> {
        let probe = directory.join(".xsync.tmp.owner-probe");
        let _ = std::fs::remove_file(&probe);
        std::fs::write(&probe, b"").ok()?;
        let owner = owner_of(&std::fs::symlink_metadata(&probe).ok()?);
        let _ = std::fs::remove_file(&probe);
        owner
    }
}

#[cfg(unix)]
// The non-Unix arm of this function has nothing to report, so the
// `Option` is load-bearing there even though this arm always fills it.
#[allow(clippy::unnecessary_wraps)]
fn owner_of(metadata: &std::fs::Metadata) -> Option<Owner> {
    use std::os::unix::fs::MetadataExt;
    Some(Owner {
        uid: metadata.uid(),
        gid: metadata.gid(),
    })
}

#[cfg(not(unix))]
fn owner_of(_metadata: &std::fs::Metadata) -> Option<Owner> {
    None
}

/// Record any metadata on `entry` that will not reach the destination.
#[cfg(unix)]
fn note_dropped_metadata(
    partial: &mut Partial,
    entry: &FileEntry,
    path: &Path,
    owner: Option<Owner>,
) {
    use std::os::unix::fs::MetadataExt;

    // The scan's own stat cannot be reused here: plan entries reach this point
    // reconstructed from the index encoding, which carries identity, size and
    // times but not ownership or link count, and that encoding is shared with
    // the frozen v1 wire format.
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        // A vanished or unreadable entry is the transfer's problem to report;
        // guessing about it here would only add noise.
        return;
    };

    if entry.kind == EntryKind::File && metadata.nlink() > 1 {
        partial
            .links
            .push((metadata.dev(), metadata.ino(), entry.size));
    }
    if let Some(owner) = owner {
        if metadata.uid() != owner.uid || metadata.gid() != owner.gid {
            partial.foreign_owner += 1;
        }
    }
    if xattr::list(path).is_ok_and(|mut names| names.any(|name| xattr_is_significant(&name))) {
        partial.with_xattrs += 1;
    }
}

/// Record dropped metadata on platforms without Unix ownership.
///
/// On Windows this reports sparse files, which are otherwise written out dense
/// in silence — a 10 MB sparse file occupying almost nothing on disk becomes
/// 10 MB of real zeros at the destination, with no warning at all before this.
///
/// Two things NTFS loses are deliberately *not* reported, because stable Rust
/// cannot see them and this crate denies `unsafe` outside one documented
/// exemption:
///
/// - **Hardlinks.** `std::os::windows::fs::MetadataExt::number_of_links` is
///   behind the unstable `windows_by_handle` feature. Measured behaviour: a
///   hardlinked pair arrives as two independent copies.
/// - **Alternate data streams.** Enumerating them needs `FindFirstStreamW`.
///   Measured behaviour: the stream is silently dropped.
///
/// Junctions are also converted to symlinks, which `file_attributes` cannot
/// distinguish from a real symlink without reading the reparse tag.
#[cfg(windows)]
fn note_dropped_metadata(
    partial: &mut Partial,
    entry: &FileEntry,
    path: &Path,
    _owner: Option<Owner>,
) {
    use std::os::windows::fs::MetadataExt;

    /// `FILE_ATTRIBUTE_SPARSE_FILE`.
    const SPARSE: u32 = 0x0000_0200;

    if entry.kind != EntryKind::File {
        return;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_attributes() & SPARSE != 0 {
        partial.sparse_written_dense += 1;
    }
}

/// Count reparse points, which are visible even though their *kind* is not.
#[cfg(windows)]
fn note_reparse_point(partial: &mut Partial, path: &Path) {
    use std::os::windows::fs::MetadataExt;

    /// `FILE_ATTRIBUTE_REPARSE_POINT`.
    const REPARSE: u32 = 0x0000_0400;

    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_attributes() & REPARSE != 0 {
            partial.reparse_points += 1;
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn note_dropped_metadata(
    _partial: &mut Partial,
    _entry: &FileEntry,
    _path: &Path,
    _owner: Option<Owner>,
) {
}

/// Whether an extended attribute carries anything a user would miss.
///
/// macOS stamps `com.apple.provenance` onto essentially every file it creates,
/// so counting it would make the warning fire on every run and turn it into
/// noise. Resource forks, Finder info, quarantine flags and Spotlight metadata
/// are real user data and are counted.
#[cfg(unix)]
fn xattr_is_significant(name: &std::ffi::OsStr) -> bool {
    const KERNEL_MAINTAINED: [&str; 2] = ["com.apple.provenance", "com.apple.lastuseddate#PS"];
    name.to_str()
        .is_none_or(|name| !KERNEL_MAINTAINED.contains(&name))
}

#[cfg(unix)]
// The non-Unix arm of this function has nothing to report, so the
// `Option` is load-bearing there even though this arm always fills it.
#[allow(clippy::unnecessary_wraps)]
fn allocated_bytes(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    // `st_blocks` counts 512-byte units by POSIX definition, independent of the
    // filesystem's own block size.
    Some(metadata.blocks().saturating_mul(512))
}

#[cfg(not(unix))]
fn allocated_bytes(_metadata: &std::fs::Metadata) -> Option<u64> {
    // Windows exposes allocation through `GetCompressedFileSize`, which std does
    // not wrap. Reporting nothing is correct here: it suppresses the warning
    // rather than inventing a number.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::SourceFingerprint;
    use std::time::UNIX_EPOCH;

    fn entry(path: &str, size: u64) -> FileEntry {
        FileEntry {
            path: WirePath::from_wire(path.as_bytes().to_vec()).unwrap(),
            kind: EntryKind::File,
            size,
            mtime: UNIX_EPOCH,
            mode: 0o644,
            fingerprint: SourceFingerprint::synthetic(EntryKind::File, size, UNIX_EPOCH),
        }
    }

    /// Build an entry for a file that exists on disk, sized from it.
    fn scanned_entry(root: &Path, name: &str) -> FileEntry {
        let metadata = std::fs::symlink_metadata(root.join(name)).unwrap();
        entry(name, metadata.len())
    }

    #[cfg(unix)]
    #[test]
    fn a_sparse_file_is_reported_with_both_sizes() {
        use std::io::{Seek, SeekFrom, Write};

        let temp = tempfile::tempdir().unwrap();
        // 64 MiB apparent, one byte written at the end: almost entirely hole.
        let apparent = 64 * 1024 * 1024;
        let path = temp.path().join("sparse.img");
        let mut file = std::fs::File::create(&path).unwrap();
        file.seek(SeekFrom::Start(apparent - 1)).unwrap();
        file.write_all(b"x").unwrap();
        file.sync_all().unwrap();

        let report = inspect(&[entry("sparse.img", apparent)], temp.path(), None).sparse;
        if report.is_empty() {
            // Some filesystems (and some CI volumes) do not support holes.
            return;
        }
        assert_eq!(report.files.len(), 1);
        let found = &report.files[0];
        assert_eq!(found.apparent_bytes, apparent);
        assert!(
            found.allocated_bytes < apparent / 2,
            "a hole-punched file should occupy far less than its length"
        );
        assert!(found.amplification() > 2.0);
        assert!(report.wasted_bytes() > apparent / 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_dense_file_is_not_reported() {
        let temp = tempfile::tempdir().unwrap();
        let size = 2 * 1024 * 1024_usize;
        std::fs::write(temp.path().join("dense.bin"), vec![7u8; size]).unwrap();
        let report = inspect(&[entry("dense.bin", size as u64)], temp.path(), None).sparse;
        assert!(report.is_empty(), "a fully written file has no holes");
        assert_eq!(report.apparent_bytes, size as u64);
    }

    #[test]
    fn small_files_are_never_inspected() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tiny.txt"), b"x").unwrap();
        let report = inspect(&[entry("tiny.txt", 1)], temp.path(), None).sparse;
        assert!(report.is_empty());
        assert_eq!(
            report.apparent_bytes, 0,
            "below the threshold, nothing is stat'd"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_parallel_pass_agrees_with_the_serial_one() {
        // The chunked pass must be byte-identical to the serial one, including
        // hardlink accounting, which depends on which name for an inode is seen
        // first. Enough entries to clear PARALLEL_THRESHOLD.
        let temp = tempfile::tempdir().unwrap();
        let count = PARALLEL_THRESHOLD + 500;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let name = format!("f{i:05}.bin");
            let path = temp.path().join(&name);
            std::fs::write(&path, vec![0u8; 32]).unwrap();
            // Every seventh file gets a second name, so hardlink groups span
            // chunk boundaries.
            if i % 7 == 0 && i > 0 {
                let link = format!("l{i:05}.bin");
                std::fs::hard_link(&path, temp.path().join(&link)).unwrap();
                entries.push(scanned_entry(temp.path(), &link));
            }
            entries.push(scanned_entry(temp.path(), &name));
        }

        let serial = inspect_with_workers(&entries, temp.path(), None, 1);
        for workers in [2, 3, 8, 16] {
            let parallel = inspect_with_workers(&entries, temp.path(), None, workers);
            assert_eq!(
                parallel.dropped, serial.dropped,
                "dropped metadata differs at {workers} workers"
            );
            assert_eq!(
                parallel.sparse.files, serial.sparse.files,
                "sparse findings differ at {workers} workers"
            );
            assert_eq!(parallel.sparse.apparent_bytes, serial.sparse.apparent_bytes);
        }
        assert!(
            serial.dropped.hardlinked > 0,
            "fixture must exercise hardlinks"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_are_counted_with_the_bytes_they_will_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.bin"), vec![1u8; 1024]).unwrap();
        std::fs::hard_link(temp.path().join("a.bin"), temp.path().join("b.bin")).unwrap();

        let found = inspect(
            &[
                scanned_entry(temp.path(), "a.bin"),
                scanned_entry(temp.path(), "b.bin"),
            ],
            temp.path(),
            None,
        );
        assert_eq!(
            found.dropped.hardlinked, 2,
            "both names carry the extra link"
        );
        assert_eq!(
            found.dropped.hardlink_extra_bytes, 1024,
            "one shared inode becoming two copies costs one extra copy, not two"
        );
        assert!(!found.dropped.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn extended_attributes_are_counted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tagged.txt");
        std::fs::write(&path, b"x").unwrap();
        if xattr::set(&path, "user.xsync.test", b"1").is_err() {
            // Some filesystems reject user xattrs; nothing to assert then.
            return;
        }
        let found = inspect(
            &[scanned_entry(temp.path(), "tagged.txt")],
            temp.path(),
            None,
        );
        assert_eq!(found.dropped.with_xattrs, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_plain_tree_reports_nothing_dropped() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("plain.txt"), b"x").unwrap();
        let owner = Owner::probe(temp.path());
        let found = inspect(
            &[scanned_entry(temp.path(), "plain.txt")],
            temp.path(),
            owner,
        );
        assert!(
            found.dropped.is_empty(),
            "silence is reserved for the case where nothing is lost: {:?}",
            found.dropped
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_probe_matches_a_file_this_process_creates() {
        let temp = tempfile::tempdir().unwrap();
        let owner = Owner::probe(temp.path()).expect("unix always reports an owner");
        std::fs::write(temp.path().join("mine.txt"), b"x").unwrap();
        let found = inspect(
            &[scanned_entry(temp.path(), "mine.txt")],
            temp.path(),
            Some(owner),
        );
        assert_eq!(
            found.dropped.foreign_owner, 0,
            "a file this process created is not foreign-owned"
        );
        // And the probe must not leave itself behind.
        assert!(!temp.path().join(".xsync.tmp.owner-probe").exists());
    }

    #[test]
    fn amplification_of_a_fully_hole_file_is_infinite_not_a_panic() {
        let file = SparseFile {
            path: WirePath::from_wire(b"x".to_vec()).unwrap(),
            apparent_bytes: 1 << 40,
            allocated_bytes: 0,
        };
        assert!(file.amplification().is_infinite());
    }

    #[cfg(unix)]
    #[test]
    fn a_lone_name_for_a_linked_inode_costs_no_extra_bytes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.bin"), vec![1u8; 1024]).unwrap();
        std::fs::hard_link(temp.path().join("a.bin"), temp.path().join("b.bin")).unwrap();

        // Only one of the two names is being transferred.
        let found = inspect(&[scanned_entry(temp.path(), "a.bin")], temp.path(), None);
        assert_eq!(
            found.dropped.hardlinked, 1,
            "the lost link is still worth saying"
        );
        assert_eq!(found.dropped.hardlink_extra_bytes, 0);
    }
}
