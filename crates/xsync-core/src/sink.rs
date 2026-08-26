//! Verified destination writes with deterministic temporary files.
//!
//! Received bytes are written under `.xsync.tmp.<hash-of-relpath>`, verified,
//! assigned their final metadata, and atomically renamed into place. A failed
//! verification is requested once more before becoming a file-level failure.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use filetime::FileTime;

use crate::path::WirePath;
use crate::scanner::{EntryKind, FileEntry};

/// Number of receive attempts allowed after one verification failure.
pub const MAX_VERIFICATION_ATTEMPTS: u8 = 2;

/// Whether a symlink points to a file or directory.
///
/// Unix does not need this distinction. Windows requires it when creating the
/// link, including for links whose targets do not exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkTargetKind {
    /// A file or another symlink.
    File,
    /// A directory.
    Directory,
}

/// Errors produced by verified sink operations.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// A protocol path was empty, absolute, or contained traversal components.
    #[error("invalid protocol path '{path}'")]
    InvalidPath {
        /// The rejected protocol path.
        path: String,
    },
    /// An operation received the wrong filesystem object kind.
    #[error("expected {expected} entry for '{path}', got {actual:?}")]
    WrongKind {
        /// Protocol-canonical entry path.
        path: String,
        /// Kind required by the operation.
        expected: &'static str,
        /// Kind supplied by the caller.
        actual: EntryKind,
    },
    /// A requested chunk fell outside its declared file size.
    #[error(
        "invalid chunk range for '{path}': offset {offset}, length {length}, file size {size}"
    )]
    InvalidChunkRange {
        /// Protocol-canonical file path.
        path: String,
        /// Requested byte offset.
        offset: u64,
        /// Requested byte count.
        length: u64,
        /// Declared full file size.
        size: u64,
    },
    /// Receiving bytes from the source failed.
    #[error("failed to receive '{path}' on attempt {attempt}: {source}")]
    Receive {
        /// Protocol-canonical file path.
        path: String,
        /// One-based receive attempt.
        attempt: u8,
        /// Underlying transport or source-read error.
        #[source]
        source: io::Error,
    },
    /// Both the initial payload and retransmission failed verification.
    #[error("verification failed for '{path}' after {attempts} attempts")]
    VerificationFailed {
        /// Protocol-canonical file path.
        path: String,
        /// Number of payloads that failed verification.
        attempts: u8,
    },
    /// A filesystem operation failed.
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        /// Short operation description.
        operation: &'static str,
        /// Filesystem path involved.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
}

/// A destination root that applies verified write operations.
#[derive(Debug, Clone)]
pub struct Sink {
    root: PathBuf,
    temporary_hashes: Arc<Mutex<HashMap<WirePath, String>>>,
}

impl Sink {
    /// Create a sink, creating its destination root when needed.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::Io`] if the destination root cannot be created.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, SinkError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|source| io_error("create destination root", &root, source))?;
        Ok(Self {
            root,
            temporary_hashes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Return this sink's destination root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a validated protocol-relative path under this sink.
    ///
    /// # Errors
    /// Returns [`SinkError::InvalidPath`] for an unsafe relative path.
    pub fn path_for(&self, relative_path: impl Into<WirePath>) -> Result<PathBuf, SinkError> {
        self.destination_path(&relative_path.into())
    }

    /// Apply metadata to this sink's root directory after all child writes.
    ///
    /// # Errors
    /// Returns an error when `entry` is not a directory or metadata cannot be
    /// applied.
    pub fn finish_root_directory(&self, entry: &FileEntry) -> Result<(), SinkError> {
        require_kind(entry, EntryKind::Directory, "directory")?;
        apply_path_metadata(&self.root, entry)
    }

    /// Remove one validated destination entry after a successful transfer.
    /// Missing paths are treated as already deleted.
    ///
    /// # Errors
    /// Returns an error for an unsafe path or filesystem failure.
    pub fn delete_entry(&self, entry: &FileEntry) -> Result<(), SinkError> {
        let path = self.destination_path(&entry.path)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => remove_existing(&path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(io_error("inspect extraneous destination", &path, source)),
        }
    }

    /// Return the deterministic temporary path for a protocol-relative path.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::InvalidPath`] for an unsafe protocol path.
    pub fn temporary_path(&self, relative_path: impl Into<WirePath>) -> Result<PathBuf, SinkError> {
        let relative_path = relative_path.into();
        let final_path = self.destination_path(&relative_path)?;
        let mut temporary_hashes = match self.temporary_hashes.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let hash = temporary_hashes
            .entry(relative_path.clone())
            .or_insert_with(|| blake3::hash(relative_path.as_bytes()).to_hex().to_string())
            .clone();
        let parent = final_path
            .parent()
            .ok_or_else(|| invalid_path(&relative_path.to_string()))?;
        Ok(parent.join(format!(".xsync.tmp.{hash}")))
    }

    /// Receive, verify, and atomically commit a complete regular file.
    ///
    /// `receive` is called a second time only when the first payload has the
    /// wrong length or BLAKE3 hash. Existing destination content remains in
    /// place until a verified temporary file is ready.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::VerificationFailed`] after two bad payloads, or a
    /// contextual receive/filesystem error.
    pub fn write_file_with_retry<F>(
        &self,
        entry: &FileEntry,
        expected_hash: &blake3::Hash,
        mut receive: F,
    ) -> Result<(), SinkError>
    where
        F: FnMut(u8) -> io::Result<Vec<u8>>,
    {
        require_kind(entry, EntryKind::File, "file")?;
        let final_path = self.destination_path(&entry.path)?;
        let temp_path = self.temporary_path(&entry.path)?;
        create_parent(&final_path)?;

        for attempt in 1..=MAX_VERIFICATION_ATTEMPTS {
            let data = receive(attempt).map_err(|source| SinkError::Receive {
                path: entry.path.to_string(),
                attempt,
                source,
            })?;
            write_new_temp(&temp_path, &data)?;
            if data.len() as u64 == entry.size && blake3::hash(&data) == *expected_hash {
                apply_file_metadata(&temp_path, entry)?;
                commit_temp(&temp_path, &final_path)?;
                return Ok(());
            }
        }

        Err(SinkError::VerificationFailed {
            path: entry.path.to_string(),
            attempts: MAX_VERIFICATION_ATTEMPTS,
        })
    }

    /// Ensure the deterministic temp file for chunked writes exists at the
    /// declared size.
    ///
    /// Idempotent: if a temp file already exists for this relative path at the
    /// correct size it is preserved, so a surviving stage (from a prior run or
    /// another stream) is not destroyed. Otherwise any stale temp is removed and
    /// the file is recreated and preallocated to `entry.size`. This is the
    /// contract shared by resume (Story 3.4) and future multi-stream striping
    /// (Story 4.2): all writers agree on the same stage path and only overwrite
    /// the ranges they own.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-file entry, unsafe path, or filesystem
    /// failure.
    pub fn prepare_large(&self, entry: &FileEntry) -> Result<(), SinkError> {
        require_kind(entry, EntryKind::File, "file")?;
        let final_path = self.destination_path(&entry.path)?;
        let temp_path = self.temporary_path(&entry.path)?;
        create_parent(&final_path)?;
        // Do not follow a pre-existing symlink at the staging path. Besides
        // invalidating the resume guarantee, doing so would let a malicious
        // or interrupted run redirect chunk writes outside the destination
        // root.
        if let Ok(metadata) = fs::symlink_metadata(&temp_path) {
            if metadata.len() == entry.size && !metadata.file_type().is_symlink() {
                return Ok(());
            }
        }
        remove_existing(&temp_path)?;
        let file = File::create(&temp_path)
            .map_err(|source| io_error("create temp file", &temp_path, source))?;
        file.set_len(entry.size)
            .map_err(|source| io_error("preallocate temp file", &temp_path, source))
    }

    /// Receive and verify one chunk, retrying once before writing its range.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::InvalidChunkRange`] for an out-of-bounds range,
    /// [`SinkError::VerificationFailed`] after two bad payloads, or a
    /// contextual receive/filesystem error.
    pub fn write_chunk_with_retry<F>(
        &self,
        entry: &FileEntry,
        offset: u64,
        length: u64,
        expected_hash: &blake3::Hash,
        mut receive: F,
    ) -> Result<(), SinkError>
    where
        F: FnMut(u8) -> io::Result<Vec<u8>>,
    {
        require_kind(entry, EntryKind::File, "file")?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > entry.size)
        {
            return Err(SinkError::InvalidChunkRange {
                path: entry.path.to_string(),
                offset,
                length,
                size: entry.size,
            });
        }
        let temp_path = self.temporary_path(&entry.path)?;

        for attempt in 1..=MAX_VERIFICATION_ATTEMPTS {
            let data = receive(attempt).map_err(|source| SinkError::Receive {
                path: entry.path.to_string(),
                attempt,
                source,
            })?;
            if data.len() as u64 == length && blake3::hash(&data) == *expected_hash {
                write_at(&temp_path, offset, &data)?;
                return Ok(());
            }
        }

        Err(SinkError::VerificationFailed {
            path: entry.path.to_string(),
            attempts: MAX_VERIFICATION_ATTEMPTS,
        })
    }

    /// Apply metadata and commit a fully acknowledged chunked file.
    ///
    /// The caller must invoke this only after every disjoint range has passed
    /// [`Self::write_chunk_with_retry`].
    ///
    /// # Errors
    ///
    /// Returns an error for a non-file entry, unsafe path, missing temp file,
    /// wrong temp length, or filesystem failure.
    pub fn finish_large(&self, entry: &FileEntry) -> Result<(), SinkError> {
        require_kind(entry, EntryKind::File, "file")?;
        let final_path = self.destination_path(&entry.path)?;
        let temp_path = self.temporary_path(&entry.path)?;
        let actual_size = fs::metadata(&temp_path)
            .map_err(|source| io_error("inspect temp file", &temp_path, source))?
            .len();
        if actual_size != entry.size {
            return Err(SinkError::InvalidChunkRange {
                path: entry.path.to_string(),
                offset: 0,
                length: actual_size,
                size: entry.size,
            });
        }
        apply_file_metadata(&temp_path, entry)?;
        commit_temp(&temp_path, &final_path)
    }

    /// Create all directories, including empty ones and on-demand parents.
    ///
    /// Directory mode and mtime are intentionally deferred to
    /// [`Self::finish_directories`] so child creation cannot disturb them.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-directory entry, unsafe path, or filesystem
    /// failure.
    pub fn create_directories(&self, entries: &[FileEntry]) -> Result<(), SinkError> {
        for entry in entries {
            require_kind(entry, EntryKind::Directory, "directory")?;
            let path = self.destination_path(&entry.path)?;
            if let Ok(metadata) = fs::symlink_metadata(&path) {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    remove_existing(&path)?;
                }
            }
            fs::create_dir_all(&path)
                .map_err(|source| io_error("create directory", &path, source))?;
        }
        Ok(())
    }

    /// Apply directory mode and mtime deepest-first after all writes finish.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-directory entry, unsafe path, or filesystem
    /// failure.
    pub fn finish_directories(&self, entries: &[FileEntry]) -> Result<(), SinkError> {
        let mut entries: Vec<_> = entries.iter().collect();
        entries.sort_by_key(|entry| Reverse(path_depth(&entry.path)));
        for entry in entries {
            require_kind(entry, EntryKind::Directory, "directory")?;
            let path = self.destination_path(&entry.path)?;
            apply_path_metadata(&path, entry)?;
        }
        Ok(())
    }

    /// Create a symlink through its deterministic temporary path and commit it.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-symlink entry, unsafe path, or filesystem
    /// failure.
    pub fn create_symlink(
        &self,
        entry: &FileEntry,
        target: &Path,
        target_kind: SymlinkTargetKind,
    ) -> Result<(), SinkError> {
        require_kind(entry, EntryKind::Symlink, "symlink")?;
        let final_path = self.destination_path(&entry.path)?;
        let temp_path = self.temporary_path(&entry.path)?;
        create_parent(&final_path)?;
        remove_existing(&temp_path)?;
        create_platform_symlink(target, &temp_path, target_kind)
            .map_err(|source| io_error("create temp symlink", &temp_path, source))?;
        let time = FileTime::from_system_time(entry.mtime);
        filetime::set_symlink_file_times(&temp_path, time, time)
            .map_err(|source| io_error("set symlink mtime", &temp_path, source))?;
        commit_temp(&temp_path, &final_path)
    }

    fn destination_path(&self, relative_path: &WirePath) -> Result<PathBuf, SinkError> {
        let relative_path = WirePath::from_wire(relative_path.as_bytes().to_vec())
            .map_err(|_| invalid_path(&relative_path.to_string()))?;
        if relative_path.is_empty() {
            return Err(invalid_path(&relative_path.to_string()));
        }
        // Never resolve an existing symlink in an ancestor. The final
        // component may legitimately be replaced (for example, a file being
        // changed into a directory), but an ancestor would redirect the
        // operation outside the destination root.
        let mut ancestor = self.root.clone();
        let components = relative_path.as_bytes().split(|byte| *byte == b'/');
        let count = relative_path.depth();
        for (index, component) in components.enumerate() {
            if index + 1 == count {
                break;
            }
            let component_path = WirePath::from_wire(component.to_vec())
                .expect("components of a validated path are safe")
                .to_native_path(Path::new(""));
            ancestor.push(component_path);
            if let Ok(metadata) = fs::symlink_metadata(&ancestor) {
                if metadata.file_type().is_symlink() {
                    return Err(invalid_path(&relative_path.to_string()));
                }
            }
        }
        Ok(relative_path.to_native_path(&self.root))
    }
}

fn require_kind(
    entry: &FileEntry,
    required: EntryKind,
    expected: &'static str,
) -> Result<(), SinkError> {
    if entry.kind == required {
        Ok(())
    } else {
        Err(SinkError::WrongKind {
            path: entry.path.to_string(),
            expected,
            actual: entry.kind,
        })
    }
}

fn invalid_path(path: &str) -> SinkError {
    SinkError::InvalidPath {
        path: path.to_owned(),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> SinkError {
    SinkError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn create_parent(path: &Path) -> Result<(), SinkError> {
    let parent = path
        .parent()
        .expect("validated destination paths always have a parent");
    fs::create_dir_all(parent).map_err(|source| io_error("create parent directory", parent, source))
}

fn write_new_temp(path: &Path, data: &[u8]) -> Result<(), SinkError> {
    remove_existing(path)?;
    let mut file =
        File::create(path).map_err(|source| io_error("create temp file", path, source))?;
    file.write_all(data)
        .map_err(|source| io_error("write temp file", path, source))
}

fn write_at(path: &Path, offset: u64, data: &[u8]) -> Result<(), SinkError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| io_error("open chunked temp file", path, source))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| io_error("seek chunked temp file", path, source))?;
    file.write_all(data)
        .map_err(|source| io_error("write chunked temp file", path, source))?;
    // The caller records the completed range in its durable receiver journal
    // immediately after this function returns. Flush the staged bytes first
    // so a checkpoint can never acknowledge data that exists only in cache.
    file.sync_data()
        .map_err(|source| io_error("sync chunked temp file", path, source))
}

fn apply_file_metadata(path: &Path, entry: &FileEntry) -> Result<(), SinkError> {
    apply_path_metadata(path, entry)
}

fn apply_path_metadata(path: &Path, entry: &FileEntry) -> Result<(), SinkError> {
    let time = FileTime::from_system_time(entry.mtime);
    filetime::set_file_mtime(path, time)
        .map_err(|source| io_error("set modification time", path, source))?;
    set_permissions(path, entry.mode)
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), SinkError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|source| io_error("set permissions", path, source))
}

#[cfg(not(unix))]
fn set_permissions(path: &Path, mode: u32) -> Result<(), SinkError> {
    let mut permissions = fs::metadata(path)
        .map_err(|source| io_error("inspect permissions", path, source))?
        .permissions();
    permissions.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, permissions)
        .map_err(|source| io_error("set permissions", path, source))
}

fn commit_temp(temp_path: &Path, final_path: &Path) -> Result<(), SinkError> {
    if let Ok(metadata) = fs::symlink_metadata(final_path) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            remove_existing(final_path)?;
        }
    }

    #[cfg(windows)]
    remove_existing(final_path)?;

    fs::rename(temp_path, final_path)
        .map_err(|source| io_error("commit verified temp file", final_path, source))
}

fn remove_existing(path: &Path) -> Result<(), SinkError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("inspect existing path", path, source)),
    };

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
            .map_err(|source| io_error("remove existing directory", path, source))
    } else {
        remove_non_directory(path)
    }
}

#[cfg(not(windows))]
fn remove_non_directory(path: &Path) -> Result<(), SinkError> {
    fs::remove_file(path).map_err(|source| io_error("remove existing file", path, source))
}

#[cfg(windows)]
fn remove_non_directory(path: &Path) -> Result<(), SinkError> {
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .map_err(|source| io_error("remove existing path", path, source))
}

fn path_depth(path: &WirePath) -> usize {
    path.depth()
}

#[cfg(unix)]
fn create_platform_symlink(
    target: &Path,
    link: &Path,
    _target_kind: SymlinkTargetKind,
) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_platform_symlink(
    target: &Path,
    link: &Path,
    target_kind: SymlinkTargetKind,
) -> io::Result<()> {
    match target_kind {
        SymlinkTargetKind::File => std::os::windows::fs::symlink_file(target, link),
        SymlinkTargetKind::Directory => std::os::windows::fs::symlink_dir(target, link),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::collections::HashMap;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::tempdir;

    use super::*;
    #[cfg(unix)]
    use crate::scanner;
    use crate::scanner::SourceFingerprint;

    fn entry(path: &str, kind: EntryKind, size: u64, mode: u32, seconds: u64) -> FileEntry {
        FileEntry {
            path: WirePath::from(path),
            kind,
            size,
            mtime: UNIX_EPOCH + Duration::from_secs(seconds),
            mode,
            fingerprint: SourceFingerprint::synthetic(
                kind,
                size,
                UNIX_EPOCH + Duration::from_secs(seconds),
            ),
        }
    }

    #[cfg(unix)]
    fn scanned(root: &Path) -> HashMap<WirePath, FileEntry> {
        let scan = scanner::scan(root).unwrap();
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
    fn retries_one_corrupt_file_then_commits_verified_bytes() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let good = b"verified contents";
        let expected = blake3::hash(good);
        let file = entry(
            "nested/file",
            EntryKind::File,
            good.len() as u64,
            0o640,
            100,
        );
        let mut calls = 0;

        sink.write_file_with_retry(&file, &expected, |_| {
            calls += 1;
            Ok(if calls == 1 {
                b"bad contents".to_vec()
            } else {
                good.to_vec()
            })
        })
        .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(fs::read(temp.path().join("nested/file")).unwrap(), good);
        assert!(!sink.temporary_path("nested/file").unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn writes_invalid_utf8_filename_without_lossy_conversion() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let temp = tempdir().unwrap();
        let raw_name = OsString::from_vec(b"raw-\xff-name".to_vec());
        let probe = temp.path().join(&raw_name);
        if fs::write(&probe, []).is_err() {
            // Some macOS filesystems reject non-UTF-8 names.
            return;
        }
        fs::remove_file(&probe).unwrap();

        let sink = Sink::new(temp.path()).unwrap();
        let mut file = entry("placeholder", EntryKind::File, 3, 0o644, 1);
        file.path = WirePath::from_wire(b"raw-\xff-name".to_vec()).unwrap();
        let payload = b"raw";
        sink.write_file_with_retry(&file, &blake3::hash(payload), |_| Ok(payload.to_vec()))
            .unwrap();

        assert_eq!(fs::read(temp.path().join(raw_name)).unwrap(), payload);
    }

    #[test]
    fn second_verification_failure_preserves_existing_destination() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        fs::write(temp.path().join("file"), b"old destination").unwrap();
        let expected_data = b"expected";
        let file = entry(
            "file",
            EntryKind::File,
            expected_data.len() as u64,
            0o644,
            100,
        );

        let error = sink
            .write_file_with_retry(&file, &blake3::hash(expected_data), |_| {
                Ok(b"corrupt".to_vec())
            })
            .unwrap_err();

        assert!(matches!(
            error,
            SinkError::VerificationFailed { attempts: 2, .. }
        ));
        assert_eq!(
            fs::read(temp.path().join("file")).unwrap(),
            b"old destination"
        );
        assert!(sink.temporary_path("file").unwrap().exists());
    }

    #[test]
    fn deterministic_leftover_is_overwritten_on_rerun() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let file = entry("nested/file", EntryKind::File, 8, 0o644, 100);
        let temp_path = sink.temporary_path(&file.path).unwrap();
        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(&temp_path, b"partial").unwrap();

        assert_eq!(temp_path, sink.temporary_path(&file.path).unwrap());
        assert!(!temp.path().join("nested/file").exists());
        sink.write_file_with_retry(&file, &blake3::hash(b"complete"), |_| {
            Ok(b"complete".to_vec())
        })
        .unwrap();

        assert_eq!(
            fs::read(temp.path().join("nested/file")).unwrap(),
            b"complete"
        );
        assert!(!temp_path.exists());
    }

    #[test]
    fn verifies_disjoint_chunks_before_large_file_commit() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let file = entry("large", EntryKind::File, 10, 0o600, 100);
        sink.prepare_large(&file).unwrap();
        sink.write_chunk_with_retry(&file, 0, 5, &blake3::hash(b"hello"), |_| {
            Ok(b"hello".to_vec())
        })
        .unwrap();
        let mut attempts = 0;
        sink.write_chunk_with_retry(&file, 5, 5, &blake3::hash(b"world"), |_| {
            attempts += 1;
            Ok(if attempts == 1 {
                b"wrong".to_vec()
            } else {
                b"world".to_vec()
            })
        })
        .unwrap();
        sink.finish_large(&file).unwrap();

        assert_eq!(attempts, 2);
        assert_eq!(fs::read(temp.path().join("large")).unwrap(), b"helloworld");
    }

    #[test]
    fn prepare_large_is_idempotent_and_preserves_a_matching_stage() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let file = entry("large", EntryKind::File, 10, 0o600, 100);
        sink.prepare_large(&file).unwrap();

        // Write a range into the stage, then call prepare again: an identical
        // idempotent prepare must NOT wipe the already-written data (the
        // contract shared by resume and multi-stream writers).
        sink.write_chunk_with_retry(&file, 0, 5, &blake3::hash(b"hello"), |_| {
            Ok(b"hello".to_vec())
        })
        .unwrap();
        sink.prepare_large(&file).unwrap();
        sink.write_chunk_with_retry(&file, 5, 5, &blake3::hash(b"world"), |_| {
            Ok(b"world".to_vec())
        })
        .unwrap();
        sink.finish_large(&file).unwrap();
        assert_eq!(fs::read(temp.path().join("large")).unwrap(), b"helloworld");

        // A differently-sized entry collapses the stage to the new size.
        let other = entry("large", EntryKind::File, 4, 0o600, 200);
        sink.prepare_large(&other).unwrap();
        assert_eq!(
            fs::metadata(sink.temporary_path("large").unwrap())
                .unwrap()
                .len(),
            4
        );
        // The committed final file is unaffected by a later prepare.
        assert_eq!(fs::read(temp.path().join("large")).unwrap(), b"helloworld");
    }

    #[cfg(unix)]
    #[test]
    fn prepare_large_replaces_a_symlinked_stage_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("target");
        fs::write(&outside_file, b"must remain unchanged").unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let file = entry("nested/large", EntryKind::File, 20, 0o600, 100);
        let stage = sink.temporary_path(&file.path).unwrap();
        fs::create_dir_all(stage.parent().unwrap()).unwrap();
        symlink(&outside_file, &stage).unwrap();

        sink.prepare_large(&file).unwrap();

        assert!(!fs::symlink_metadata(&stage)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&outside_file).unwrap(), b"must remain unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_destination_ancestors() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        symlink(outside.path(), temp.path().join("nested")).unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        let file = entry("nested/file", EntryKind::File, 4, 0o644, 1);

        let error = sink
            .write_file_with_retry(&file, &blake3::hash(b"data"), |_| Ok(b"data".to_vec()))
            .unwrap_err();
        assert!(matches!(error, SinkError::InvalidPath { .. }));
        assert!(!outside.path().join("file").exists());
    }

    #[cfg(unix)]
    #[test]
    fn recreates_tree_content_metadata_empty_dirs_and_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        fs::create_dir_all(source.path().join("nested/empty")).unwrap();
        fs::write(source.path().join("nested/file"), b"contents").unwrap();
        symlink("nested/file", source.path().join("link")).unwrap();

        fs::set_permissions(
            source.path().join("nested/file"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        fs::set_permissions(
            source.path().join("nested/empty"),
            fs::Permissions::from_mode(0o750),
        )
        .unwrap();
        let fixed = FileTime::from_unix_time(1_700_000_000, 0);
        filetime::set_file_mtime(source.path().join("nested/file"), fixed).unwrap();
        filetime::set_symlink_file_times(source.path().join("link"), fixed, fixed).unwrap();
        filetime::set_file_mtime(source.path().join("nested/empty"), fixed).unwrap();
        filetime::set_file_mtime(source.path().join("nested"), fixed).unwrap();

        let source_entries = scanned(source.path());
        let sink = Sink::new(destination.path()).unwrap();
        let directories: Vec<_> = source_entries
            .values()
            .filter(|entry| entry.kind == EntryKind::Directory)
            .cloned()
            .collect();
        sink.create_directories(&directories).unwrap();
        for entry in source_entries.values() {
            match entry.kind {
                EntryKind::File => {
                    let data = fs::read(entry.path.to_native_path(source.path())).unwrap();
                    let hash = blake3::hash(&data);
                    sink.write_file_with_retry(entry, &hash, |_| Ok(data.clone()))
                        .unwrap();
                }
                EntryKind::Symlink => {
                    let target = fs::read_link(entry.path.to_native_path(source.path())).unwrap();
                    sink.create_symlink(entry, &target, SymlinkTargetKind::File)
                        .unwrap();
                }
                EntryKind::Directory | EntryKind::Other => {}
            }
        }
        sink.finish_directories(&directories).unwrap();

        let destination_entries = scanned(destination.path());
        assert_eq!(source_entries.len(), destination_entries.len());
        for (path, source_entry) in &source_entries {
            let destination_entry = &destination_entries[path];
            assert_eq!(source_entry.path, destination_entry.path);
            assert_eq!(source_entry.kind, destination_entry.kind);
            assert_eq!(source_entry.size, destination_entry.size);
            assert_eq!(source_entry.mtime, destination_entry.mtime);
            assert_eq!(source_entry.mode, destination_entry.mode);
        }
        assert_eq!(
            fs::read_link(destination.path().join("link")).unwrap(),
            Path::new("nested/file")
        );
        assert_eq!(
            fs::read(destination.path().join("nested/file")).unwrap(),
            b"contents"
        );
    }

    #[test]
    fn rejects_paths_that_can_escape_the_destination() {
        let temp = tempdir().unwrap();
        let sink = Sink::new(temp.path()).unwrap();
        for path in ["", "/absolute", "../escape", "dir/../escape", "dir//file"] {
            assert!(matches!(
                sink.temporary_path(path),
                Err(SinkError::InvalidPath { .. })
            ));
        }
    }
}
