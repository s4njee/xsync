//! Parallel, bounded-memory filesystem scanning.
//!
//! Directory roots are not emitted. Their descendants are streamed with paths
//! relative to the root; a file or symlink root is emitted under its basename.
//! Entry order is intentionally unspecified because the walk is parallel.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::SystemTime;
#[cfg(any(unix, windows))]
use std::time::{Duration, UNIX_EPOCH};

use crossbeam_channel::{bounded, Receiver, Sender};
use ignore::{WalkBuilder, WalkState};

/// Maximum number of discovered entries waiting for a consumer by default.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1_024;

/// The filesystem object represented by a scanned entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link. Its target is never traversed by the scanner.
    Symlink,
    /// A platform-specific object such as a socket, FIFO, or device.
    Other,
}

/// Stable identity components used to detect pathname replacement between
/// discovery and source reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    /// Device or volume identifier.
    pub device: u64,
    /// Inode or file-index identifier.
    pub file: u64,
}

/// Metadata captured during discovery and compared around every source read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFingerprint {
    /// Stable filesystem identity where the platform exposes one.
    pub identity: FileIdentity,
    /// Object kind at discovery time.
    pub kind: EntryKind,
    /// Logical length at discovery time.
    pub size: u64,
    /// Precise modification time at discovery time.
    pub mtime: SystemTime,
    /// Change time where the platform exposes one.
    pub ctime: Option<SystemTime>,
}

impl SourceFingerprint {
    /// Construct a synthetic fingerprint for metadata-only callers and tests.
    #[must_use]
    pub fn synthetic(kind: EntryKind, size: u64, mtime: SystemTime) -> Self {
        Self {
            identity: FileIdentity { device: 0, file: 0 },
            kind,
            size,
            mtime,
            ctime: None,
        }
    }
}

/// Metadata discovered for one filesystem object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Protocol-canonical path relative to the scan root, using `/` separators.
    pub path: String,
    /// The object's filesystem kind.
    pub kind: EntryKind,
    /// The metadata length in bytes.
    pub size: u64,
    /// The object's last-modified timestamp.
    pub mtime: SystemTime,
    /// Unix permission and special mode bits. On non-Unix hosts this is a
    /// portable approximation based on the read-only attribute.
    pub mode: u32,
    /// Source identity and metadata captured by discovery.
    pub fingerprint: SourceFingerprint,
}

/// An item produced by a parallel scan.
pub type ScanResult = Result<FileEntry, ScanError>;

/// Errors that can occur while starting or consuming a scan.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The scan root could not be inspected.
    #[error("cannot inspect scan root '{}': {source}", path.display())]
    Root {
        /// The scan root.
        path: PathBuf,
        /// The underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// The requested channel capacity was zero.
    #[error("scan channel capacity must be at least 1")]
    ZeroChannelCapacity,
    /// The coordinator thread could not be started.
    #[error("cannot start scanner thread: {0}")]
    Start(#[source] std::io::Error),
    /// The parallel walker could not inspect part of the tree.
    #[error("filesystem walk failed: {0}")]
    Walk(String),
    /// Metadata could not be read for a discovered path.
    #[error("cannot read metadata for '{}': {source}", path.display())]
    Metadata {
        /// The discovered filesystem path.
        path: PathBuf,
        /// The underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// A discovered relative path cannot be represented by the string-based
    /// wire protocol.
    #[error("path is not valid UTF-8: '{}'", path.display())]
    NonUtf8Path {
        /// The discovered filesystem path.
        path: PathBuf,
    },
    /// A walker path unexpectedly fell outside its scan root.
    #[error("path '{}' is outside scan root '{}'", path.display(), root.display())]
    OutsideRoot {
        /// The discovered filesystem path.
        path: PathBuf,
        /// The configured scan root.
        root: PathBuf,
    },
    /// The scanner coordinator panicked.
    #[error("scanner thread panicked")]
    WorkerPanicked,
}

/// A running parallel scan and its bounded result stream.
pub struct Scan {
    entries: Receiver<ScanResult>,
    worker: JoinHandle<()>,
    channel_capacity: usize,
    queue_high_water: Arc<AtomicUsize>,
}

impl Scan {
    /// Receive scanned entries until the channel disconnects.
    #[must_use]
    pub fn entries(&self) -> &Receiver<ScanResult> {
        &self.entries
    }

    /// The maximum number of entries that can wait in the result channel.
    #[must_use]
    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    /// Highest result-channel occupancy observed by scanner producers.
    ///
    /// The value is bounded by [`Self::channel_capacity`] and is intended for
    /// diagnostics and benchmark evidence, not synchronization.
    #[must_use]
    pub fn queue_high_water_mark(&self) -> usize {
        self.queue_high_water.load(Ordering::Relaxed)
    }

    /// Stop receiving entries and wait for the scanner to finish.
    ///
    /// Any entries that have not yet been consumed are discarded.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::WorkerPanicked`] if the coordinator thread panics.
    pub fn finish(self) -> Result<(), ScanError> {
        let Self {
            entries, worker, ..
        } = self;
        drop(entries);
        worker.join().map_err(|_| ScanError::WorkerPanicked)
    }
}

/// Start a parallel scan using [`DEFAULT_CHANNEL_CAPACITY`].
///
/// All standard `ignore` filters are disabled, so dotfiles, `.git` trees, and
/// ignored files are included. Symbolic links are emitted but never followed.
///
/// # Errors
///
/// Returns an error when the root cannot be inspected or the coordinator
/// thread cannot be started.
pub fn scan(root: impl AsRef<Path>) -> Result<Scan, ScanError> {
    scan_with_capacity(root, DEFAULT_CHANNEL_CAPACITY)
}

/// Start a parallel scan with an explicit bounded-channel capacity.
///
/// # Errors
///
/// Returns an error when `channel_capacity` is zero, the root cannot be
/// inspected, or the coordinator thread cannot be started.
pub fn scan_with_capacity(
    root: impl AsRef<Path>,
    channel_capacity: usize,
) -> Result<Scan, ScanError> {
    if channel_capacity == 0 {
        return Err(ScanError::ZeroChannelCapacity);
    }

    let root = root.as_ref().to_path_buf();
    let root_metadata = fs::symlink_metadata(&root).map_err(|source| ScanError::Root {
        path: root.clone(),
        source,
    })?;
    let root_is_directory = root_metadata.is_dir();

    let mut builder = WalkBuilder::new(&root);
    builder.standard_filters(false).follow_links(false);
    let walker = builder.build_parallel();
    let (sender, entries) = bounded(channel_capacity);
    let queue_high_water = Arc::new(AtomicUsize::new(0));

    let worker = thread::Builder::new()
        .name("xsync-scanner".to_owned())
        .spawn({
            let root = root.clone();
            let queue_high_water = Arc::clone(&queue_high_water);
            move || {
                run_walker(walker, &root, root_is_directory, &sender, &queue_high_water);
            }
        })
        .map_err(ScanError::Start)?;

    Ok(Scan {
        entries,
        worker,
        channel_capacity,
        queue_high_water,
    })
}

fn run_walker(
    walker: ignore::WalkParallel,
    root: &Path,
    root_is_directory: bool,
    sender: &Sender<ScanResult>,
    queue_high_water: &AtomicUsize,
) {
    walker.run(|| {
        let root = root.to_path_buf();
        let sender = sender.clone();
        Box::new(move |result| {
            let result = match result {
                Ok(entry) if entry.depth() == 0 && root_is_directory => return WalkState::Continue,
                Ok(entry) => make_entry(&root, root_is_directory, &entry),
                Err(error) => Err(ScanError::Walk(error.to_string())),
            };

            if sender.send(result).is_ok() {
                observe_high_water(queue_high_water, sender.len());
                WalkState::Continue
            } else {
                WalkState::Quit
            }
        })
    });
}

fn observe_high_water(high_water: &AtomicUsize, occupancy: usize) {
    high_water.fetch_max(occupancy, Ordering::Relaxed);
}

fn make_entry(root: &Path, root_is_directory: bool, entry: &ignore::DirEntry) -> ScanResult {
    let path = entry.path();
    let metadata = fs::symlink_metadata(path).map_err(|source| ScanError::Metadata {
        path: path.to_path_buf(),
        source,
    })?;
    let mtime = metadata.modified().map_err(|source| ScanError::Metadata {
        path: path.to_path_buf(),
        source,
    })?;
    let kind = if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    };

    let relative = if root_is_directory {
        path.strip_prefix(root)
            .map_err(|_| ScanError::OutsideRoot {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
            })?
    } else {
        path.file_name().map_or(path, Path::new)
    };

    Ok(FileEntry {
        path: protocol_path(relative, path)?,
        kind,
        size: metadata.len(),
        mtime,
        mode: permission_mode(&metadata),
        fingerprint: source_fingerprint(&metadata, kind, mtime)?,
    })
}

fn source_fingerprint(
    metadata: &fs::Metadata,
    kind: EntryKind,
    mtime: SystemTime,
) -> Result<SourceFingerprint, ScanError> {
    fingerprint_from_metadata(metadata, kind, mtime).map_err(|source| ScanError::Metadata {
        path: PathBuf::from("<timestamp>"),
        source,
    })
}

pub(crate) fn fingerprint_from_metadata(
    metadata: &fs::Metadata,
    kind: EntryKind,
    mtime: SystemTime,
) -> std::io::Result<SourceFingerprint> {
    Ok(SourceFingerprint {
        identity: file_identity(metadata),
        kind,
        size: metadata.len(),
        mtime,
        ctime: change_time(metadata)?,
    })
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> FileIdentity {
    // Stable volume/file-index access is not available through stable std on
    // all non-Unix targets; size, mtime, and ctime still detect normal races.
    FileIdentity { device: 0, file: 0 }
}

#[cfg(unix)]
fn change_time(metadata: &fs::Metadata) -> std::io::Result<Option<SystemTime>> {
    use std::os::unix::fs::MetadataExt;

    signed_time(metadata.ctime(), metadata.ctime_nsec()).map(Some)
}

#[cfg(windows)]
fn change_time(metadata: &fs::Metadata) -> std::io::Result<Option<SystemTime>> {
    use std::os::windows::fs::MetadataExt;

    const WINDOWS_TO_UNIX_100NS: u64 = 116_444_736_000_000_000;
    let unix_100ns = metadata
        .creation_time()
        .checked_sub(WINDOWS_TO_UNIX_100NS)
        .ok_or_else(|| timestamp_error("Windows timestamp is before Unix epoch"))?;
    let seconds = unix_100ns / 10_000_000;
    let nanoseconds = (unix_100ns % 10_000_000) * 100;
    Ok(Some(
        UNIX_EPOCH
            .checked_add(Duration::new(
                seconds,
                u32::try_from(nanoseconds)
                    .map_err(|_| timestamp_error("Windows timestamp is out of range"))?,
            ))
            .ok_or_else(|| timestamp_error("Windows timestamp is out of range"))?,
    ))
}

#[cfg(not(any(unix, windows)))]
fn change_time(_metadata: &fs::Metadata) -> std::io::Result<Option<SystemTime>> {
    Ok(None)
}

#[cfg(unix)]
fn signed_time(seconds: i64, nanoseconds: i64) -> std::io::Result<SystemTime> {
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return Err(timestamp_error(
            "filesystem timestamp nanoseconds out of range",
        ));
    }
    let nanoseconds = u32::try_from(nanoseconds)
        .map_err(|_| timestamp_error("filesystem timestamp nanoseconds out of range"))?;
    if seconds >= 0 {
        UNIX_EPOCH
            .checked_add(Duration::new(
                u64::try_from(seconds)
                    .map_err(|_| timestamp_error("filesystem timestamp is out of range"))?,
                nanoseconds,
            ))
            .ok_or_else(|| timestamp_error("filesystem timestamp is out of range"))
    } else {
        let magnitude = seconds
            .checked_abs()
            .ok_or_else(|| timestamp_error("filesystem timestamp is out of range"))?;
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(u64::try_from(magnitude).map_err(
                |_| timestamp_error("filesystem timestamp is out of range"),
            )?))
            .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanoseconds))))
            .ok_or_else(|| timestamp_error("filesystem timestamp is out of range"))
    }
}

#[cfg(unix)]
fn timestamp_error(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

#[cfg(windows)]
fn timestamp_error(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn protocol_path(relative: &Path, full_path: &Path) -> Result<String, ScanError> {
    relative
        .iter()
        .map(|component| {
            component.to_str().ok_or_else(|| ScanError::NonUtf8Path {
                path: full_path.to_path_buf(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

#[cfg(unix)]
pub(crate) fn permission_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
pub(crate) fn permission_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;

    fn collect(root: &Path) -> HashMap<String, FileEntry> {
        let scan = scan(root).unwrap();
        let entries = scan
            .entries()
            .iter()
            .map(|result| {
                let entry = result.unwrap();
                (entry.path.clone(), entry)
            })
            .collect();
        scan.finish().unwrap();
        entries
    }

    #[test]
    fn includes_hidden_git_and_ignored_entries() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/object"), b"object").unwrap();
        fs::write(temp.path().join(".gitignore"), b"ignored.txt\n").unwrap();
        fs::write(temp.path().join("ignored.txt"), b"included").unwrap();
        fs::write(temp.path().join(".hidden"), b"included").unwrap();

        let entries = collect(temp.path());
        for path in [
            ".git",
            ".git/object",
            ".gitignore",
            "ignored.txt",
            ".hidden",
        ] {
            assert!(entries.contains_key(path), "missing {path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn reports_symlinks_without_following_loops() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("dir")).unwrap();
        fs::write(temp.path().join("dir/file"), b"data").unwrap();
        symlink("..", temp.path().join("dir/loop")).unwrap();
        symlink("dir/file", temp.path().join("file-link")).unwrap();

        let scan = scan(temp.path()).unwrap();
        let mut entries = HashMap::new();
        loop {
            match scan.entries().recv_timeout(Duration::from_secs(2)) {
                Ok(result) => {
                    let entry = result.unwrap();
                    entries.insert(entry.path.clone(), entry);
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    panic!("scan did not terminate after encountering a symlink loop");
                }
            }
        }
        scan.finish().unwrap();

        assert_eq!(entries["dir/loop"].kind, EntryKind::Symlink);
        assert_eq!(entries["file-link"].kind, EntryKind::Symlink);
        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn uses_protocol_canonical_relative_paths() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("one/two")).unwrap();
        fs::write(temp.path().join("one/two/file"), b"data").unwrap();

        let entries = collect(temp.path());
        assert!(entries.contains_key("one/two/file"));
        assert!(entries.keys().all(|path| !path.starts_with('/')));
    }

    #[test]
    fn streams_through_the_configured_bounded_channel() {
        let temp = tempdir().unwrap();
        for index in 0..2_048 {
            fs::write(temp.path().join(format!("file-{index}")), b"x").unwrap();
        }

        let scan = scan_with_capacity(temp.path(), 2).unwrap();
        assert_eq!(scan.channel_capacity(), 2);
        assert_eq!(scan.entries().iter().count(), 2_048);
        assert!((1..=2).contains(&scan.queue_high_water_mark()));
        scan.finish().unwrap();
    }

    #[test]
    #[ignore = "stress test creates 100,000 filesystem entries"]
    fn scans_100k_files_through_a_bounded_channel() {
        let temp = tempdir().unwrap();
        for index in 0..100_000 {
            fs::write(temp.path().join(format!("file-{index}")), []).unwrap();
        }

        let scan = scan_with_capacity(temp.path(), 32).unwrap();
        assert_eq!(scan.channel_capacity(), 32);
        assert_eq!(scan.entries().iter().count(), 100_000);
        scan.finish().unwrap();
    }

    #[test]
    fn emits_a_single_file_root_under_its_basename() {
        let temp = tempdir().unwrap();
        let file = temp.path().join("source.txt");
        fs::write(&file, b"contents").unwrap();

        let entries = collect(&file);
        assert_eq!(entries["source.txt"].kind, EntryKind::File);
        assert_eq!(entries["source.txt"].size, 8);
    }

    #[test]
    fn rejects_zero_channel_capacity() {
        let temp = tempdir().unwrap();
        assert!(matches!(
            scan_with_capacity(temp.path(), 0),
            Err(ScanError::ZeroChannelCapacity)
        ));
    }
}
