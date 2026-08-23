//! Stable source reads with scan/open/read/verify mutation detection.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use crate::scanner::{
    fingerprint_from_metadata, permission_mode, EntryKind, FileEntry, SourceFingerprint,
};

/// Number of complete source-read attempts, including one retry after change.
pub const MAX_SOURCE_READ_ATTEMPTS: u8 = 2;
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// A verified, stable source payload and the metadata version that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableRead {
    /// Metadata from the stable scan/read attempt.
    pub entry: FileEntry,
    /// Bytes read from that stable version.
    pub bytes: Vec<u8>,
    /// BLAKE3 digest computed while reading.
    pub blake3: blake3::Hash,
    /// One for the initial read, two when a change caused a retry.
    pub attempts: u8,
}

/// Errors produced while reading a source file under a stable-read contract.
#[derive(Debug, thiserror::Error)]
pub enum SourceReadError {
    /// The requested source entry is not a regular file.
    #[error("cannot read non-file source '{path}' ({kind:?})")]
    WrongKind {
        /// Protocol-relative source path.
        path: String,
        /// Discovered source kind.
        kind: EntryKind,
    },
    /// The protocol-relative source path is unsafe.
    #[error("invalid source path '{path}'")]
    InvalidPath {
        /// Rejected source path.
        path: String,
    },
    /// The source disappeared during a read or refresh.
    #[error("source file '{path}' vanished during read")]
    Vanished {
        /// Protocol-relative source path.
        path: String,
    },
    /// The source changed twice and cannot be read as one stable version.
    #[error("source file '{path}' changed during read after {attempts} attempts")]
    Unstable {
        /// Protocol-relative source path.
        path: String,
        /// Number of complete read attempts made.
        attempts: u8,
    },
    /// Opening the source failed for a reason other than a detected race.
    #[error("cannot open source '{path}': {source}")]
    Open {
        /// Filesystem source path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Reading the source failed.
    #[error("cannot read source '{path}': {source}")]
    Read {
        /// Filesystem source path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A source metadata fingerprint could not be obtained.
    #[error("cannot fingerprint source '{path}': {source}")]
    Fingerprint {
        /// Filesystem source path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
}

/// Reads source files relative to one root without following symlinks.
#[derive(Debug, Clone)]
pub struct SourceReader {
    root: PathBuf,
}

impl SourceReader {
    /// Create a reader rooted at `root`.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Read one scanned file, retrying once if its pathname or descriptor
    /// changes.
    ///
    /// # Errors
    /// Returns [`SourceReadError::Unstable`] after two changed versions,
    /// [`SourceReadError::Vanished`] for a disappeared file, or a contextual
    /// path/filesystem error.
    pub fn read(&self, entry: &FileEntry) -> Result<StableRead, SourceReadError> {
        self.read_with_observer(entry, |_| {})
    }

    /// Read one scanned file while observing bytes completed after each read
    /// buffer. The observer is intended for tests and race-injection harnesses;
    /// it cannot alter the reader's retry policy.
    ///
    /// # Errors
    /// Returns the same errors as [`Self::read`].
    pub fn read_with_observer<F>(
        &self,
        entry: &FileEntry,
        mut observer: F,
    ) -> Result<StableRead, SourceReadError>
    where
        F: FnMut(usize),
    {
        if entry.kind != EntryKind::File {
            return Err(SourceReadError::WrongKind {
                path: entry.path.clone(),
                kind: entry.kind,
            });
        }
        let path = self.source_path(&entry.path)?;
        let mut current = entry.clone();
        for attempt in 1..=MAX_SOURCE_READ_ATTEMPTS {
            match Self::read_attempt(&path, &current, &mut observer) {
                Ok((bytes, blake3)) => {
                    return Ok(StableRead {
                        entry: current,
                        bytes,
                        blake3,
                        attempts: attempt,
                    });
                }
                Err(AttemptFailure::Vanished) => {
                    return Err(SourceReadError::Vanished {
                        path: entry.path.clone(),
                    });
                }
                Err(AttemptFailure::Io(error)) => return Err(error),
                Err(AttemptFailure::Changed) if attempt == MAX_SOURCE_READ_ATTEMPTS => {
                    return Err(SourceReadError::Unstable {
                        path: entry.path.clone(),
                        attempts: attempt,
                    });
                }
                Err(AttemptFailure::Changed) => {
                    current = match Self::refresh_entry(&path, &current)? {
                        RefreshResult::File(entry) => entry,
                        RefreshResult::Changed => current,
                        RefreshResult::Vanished => {
                            return Err(SourceReadError::Vanished {
                                path: entry.path.clone(),
                            });
                        }
                    };
                }
            }
        }
        unreachable!("source read attempts are nonempty")
    }

    fn source_path(&self, relative: &str) -> Result<PathBuf, SourceReadError> {
        let mut path = self.root.clone();
        if relative.is_empty() {
            return Err(SourceReadError::InvalidPath {
                path: relative.to_owned(),
            });
        }
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(component) => path.push(component),
                Component::CurDir
                | Component::ParentDir
                | Component::RootDir
                | Component::Prefix(_) => {
                    return Err(SourceReadError::InvalidPath {
                        path: relative.to_owned(),
                    });
                }
            }
        }
        Ok(path)
    }

    fn read_attempt<F>(
        path: &Path,
        entry: &FileEntry,
        observer: &mut F,
    ) -> Result<(Vec<u8>, blake3::Hash), AttemptFailure>
    where
        F: FnMut(usize),
    {
        let file = match open_without_following(path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(AttemptFailure::Vanished);
            }
            Err(source) if is_symlink_open_error(&source) => {
                return Err(AttemptFailure::Changed);
            }
            Err(source) => {
                return Err(AttemptFailure::Io(SourceReadError::Open {
                    path: path.to_path_buf(),
                    source,
                }))
            }
        };

        let opened = metadata_fingerprint(&file, path)?;
        if opened != entry.fingerprint {
            return Err(AttemptFailure::Changed);
        }

        let mut hasher = blake3::Hasher::new();
        let mut bytes = Vec::new();
        let mut buffer = vec![0u8; READ_BUFFER_BYTES];
        loop {
            let read = (&file).read(&mut buffer).map_err(|source| {
                AttemptFailure::Io(SourceReadError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            bytes.extend_from_slice(&buffer[..read]);
            observer(bytes.len());
        }

        let after_read = metadata_fingerprint(&file, path)?;
        if after_read != entry.fingerprint {
            return Err(AttemptFailure::Changed);
        }
        let pathname = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(AttemptFailure::Vanished);
            }
            Err(source) => {
                return Err(AttemptFailure::Io(SourceReadError::Fingerprint {
                    path: path.to_path_buf(),
                    source,
                }));
            }
        };
        let pathname_fingerprint =
            metadata_fingerprint_from_metadata(&pathname, path).map_err(AttemptFailure::Io)?;
        if pathname_fingerprint != after_read {
            return Err(AttemptFailure::Changed);
        }
        Ok((bytes, hasher.finalize()))
    }

    fn refresh_entry(path: &Path, previous: &FileEntry) -> Result<RefreshResult, SourceReadError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(RefreshResult::Vanished);
            }
            Err(source) => {
                return Err(SourceReadError::Fingerprint {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Ok(RefreshResult::Changed);
        }
        let mtime = metadata
            .modified()
            .map_err(|source| SourceReadError::Fingerprint {
                path: path.to_path_buf(),
                source,
            })?;
        let fingerprint = metadata_fingerprint_from_metadata(&metadata, path)?;
        Ok(RefreshResult::File(FileEntry {
            path: previous.path.clone(),
            kind: EntryKind::File,
            size: metadata.len(),
            mtime,
            mode: permission_mode(&metadata),
            fingerprint,
        }))
    }
}

enum AttemptFailure {
    Changed,
    Vanished,
    Io(SourceReadError),
}

impl From<AttemptFailure> for SourceReadError {
    fn from(failure: AttemptFailure) -> Self {
        match failure {
            AttemptFailure::Io(error) => error,
            AttemptFailure::Changed | AttemptFailure::Vanished => {
                unreachable!("race failures are handled by SourceReader")
            }
        }
    }
}

enum RefreshResult {
    File(FileEntry),
    Changed,
    Vanished,
}

fn metadata_fingerprint(file: &File, path: &Path) -> Result<SourceFingerprint, AttemptFailure> {
    let metadata = file.metadata().map_err(|source| {
        AttemptFailure::Io(SourceReadError::Fingerprint {
            path: path.to_path_buf(),
            source,
        })
    })?;
    metadata_fingerprint_from_metadata(&metadata, path).map_err(AttemptFailure::Io)
}

fn metadata_fingerprint_from_metadata(
    metadata: &fs::Metadata,
    path: &Path,
) -> Result<SourceFingerprint, SourceReadError> {
    let kind = if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    };
    let mtime = metadata
        .modified()
        .map_err(|source| SourceReadError::Fingerprint {
            path: path.to_path_buf(),
            source,
        })?;
    fingerprint_from_metadata(metadata, kind, mtime).map_err(|source| {
        SourceReadError::Fingerprint {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(unix)]
fn open_without_following(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_without_following(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    // FILE_FLAG_OPEN_REPARSE_POINT keeps a reparse-point pathname unopened as
    // its target. The stable std API exposes the flag setter but not the value.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_without_following(path: &Path) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "refusing to follow source symlink",
        ));
    }
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn is_symlink_open_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_symlink_open_error(_error: &io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::scanner::scan;

    fn scanned_file(root: &Path) -> FileEntry {
        let scan = scan(root).unwrap();
        let entry = scan
            .entries()
            .iter()
            .find_map(|result| {
                let entry = result.unwrap();
                (entry.kind == EntryKind::File).then_some(entry)
            })
            .unwrap();
        scan.finish().unwrap();
        entry
    }

    #[test]
    fn stable_read_hashes_and_returns_the_scanned_version() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("data"), b"stable bytes").unwrap();
        let entry = scanned_file(temp.path());
        let result = SourceReader::new(temp.path()).read(&entry).unwrap();
        assert_eq!(result.bytes, b"stable bytes");
        assert_eq!(result.blake3, blake3::hash(b"stable bytes"));
        assert_eq!(result.attempts, 1);
    }

    #[test]
    fn pathname_replacement_retries_without_mixing_versions() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("data");
        fs::write(&path, vec![b'a'; READ_BUFFER_BYTES * 4]).unwrap();
        let entry = scanned_file(temp.path());
        let mut replaced = false;
        let result = SourceReader::new(temp.path())
            .read_with_observer(&entry, |bytes| {
                if !replaced && bytes >= READ_BUFFER_BYTES {
                    fs::rename(&path, temp.path().join("old")).unwrap();
                    fs::write(&path, vec![b'b'; READ_BUFFER_BYTES * 4]).unwrap();
                    replaced = true;
                }
            })
            .unwrap();
        assert!(replaced);
        assert_eq!(result.attempts, 2);
        assert!(result.bytes.iter().all(|byte| *byte == b'b'));
    }

    #[test]
    fn rewrite_with_the_same_length_is_detected() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("data");
        fs::write(&path, vec![b'a'; READ_BUFFER_BYTES * 2]).unwrap();
        let entry = scanned_file(temp.path());
        let mut rewritten = false;
        let result = SourceReader::new(temp.path())
            .read_with_observer(&entry, |bytes| {
                if !rewritten && bytes >= READ_BUFFER_BYTES {
                    fs::write(&path, vec![b'c'; READ_BUFFER_BYTES * 2]).unwrap();
                    rewritten = true;
                }
            })
            .unwrap();
        assert_eq!(result.attempts, 2);
        assert!(result.bytes.iter().all(|byte| *byte == b'c'));
    }

    #[test]
    fn truncation_and_extension_in_place_are_retried() {
        for new_size in [READ_BUFFER_BYTES, READ_BUFFER_BYTES * 3] {
            let temp = tempdir().unwrap();
            let path = temp.path().join("data");
            fs::write(&path, vec![b'a'; READ_BUFFER_BYTES * 2]).unwrap();
            let entry = scanned_file(temp.path());
            let mut resized = false;
            let result = SourceReader::new(temp.path())
                .read_with_observer(&entry, |bytes| {
                    if !resized && bytes >= READ_BUFFER_BYTES {
                        fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .unwrap()
                            .set_len(u64::try_from(new_size).unwrap())
                            .unwrap();
                        resized = true;
                    }
                })
                .unwrap();
            assert!(resized);
            assert_eq!(result.attempts, 2);
            assert_eq!(result.bytes.len(), new_size);
        }
    }

    #[test]
    fn vanished_source_is_named_without_retrying_unrelated_work() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("data");
        fs::write(&path, vec![b'a'; READ_BUFFER_BYTES * 2]).unwrap();
        let entry = scanned_file(temp.path());
        let error = SourceReader::new(temp.path())
            .read_with_observer(&entry, |bytes| {
                if bytes >= READ_BUFFER_BYTES && path.exists() {
                    fs::remove_file(&path).unwrap();
                }
            })
            .unwrap_err();
        assert!(matches!(error, SourceReadError::Vanished { path } if path == "data"));
    }

    #[cfg(unix)]
    #[test]
    fn swapping_a_regular_file_for_a_symlink_is_unstable() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let path = temp.path().join("data");
        fs::write(&path, vec![b'a'; READ_BUFFER_BYTES * 2]).unwrap();
        let entry = scanned_file(temp.path());
        let mut swapped = false;
        let error = SourceReader::new(temp.path())
            .read_with_observer(&entry, |bytes| {
                if !swapped && bytes >= READ_BUFFER_BYTES {
                    fs::rename(&path, temp.path().join("old")).unwrap();
                    symlink("old", &path).unwrap();
                    swapped = true;
                }
            })
            .unwrap_err();
        assert!(matches!(
            error,
            SourceReadError::Unstable { attempts: 2, .. }
        ));
    }
}
