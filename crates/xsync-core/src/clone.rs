//! Staged local clone/reflink attempts.
//!
//! Clone capability failures are represented by `Ok(None)`, allowing the
//! caller to use the verified byte-copy path without exposing a partial stage.
//! A successful clone is still published through a deterministic sibling stage.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use filetime::FileTime;

use crate::scanner::{fingerprint_from_metadata, EntryKind, FileEntry};

/// The object kind produced by a successful fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneKind {
    /// A complete directory tree was cloned.
    Directory,
    /// One regular file was cloned/reflinked.
    File,
}

/// Result of a successful staged clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneOutcome {
    /// Fast-path object kind.
    pub kind: CloneKind,
    /// Whether paranoid content readback was performed.
    pub paranoid_verified: bool,
}

/// Errors that prevent a staged clone from being published.
#[derive(Debug, thiserror::Error)]
pub enum CloneError {
    /// A filesystem operation failed.
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        /// Short operation description.
        operation: &'static str,
        /// Relevant filesystem path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// The source object is not suitable for the requested clone.
    #[error("clone source '{}' is not a {expected}", path.display())]
    WrongKind {
        /// Source path.
        path: PathBuf,
        /// Required object kind.
        expected: &'static str,
    },
    /// A staged or paranoid verification failed.
    #[error("clone verification failed at '{}': {reason}", path.display())]
    Verification {
        /// Stage or published path.
        path: PathBuf,
        /// Failure detail.
        reason: String,
    },
}

/// Try a file clone/reflink without falling back to a byte copy.
///
/// `Ok(None)` means the platform or filesystem cannot provide the requested
/// clone, or the source changed during the attempt. In that case the caller
/// must perform its normal stable source read. No destination mutation remains.
///
/// # Errors
/// Returns an error for source metadata, staging, publication, or paranoid
/// readback failures.
pub fn try_clone_file(
    source: &Path,
    destination: &Path,
    expected: &FileEntry,
    paranoid: bool,
) -> Result<Option<CloneOutcome>, CloneError> {
    let source_metadata = inspect(source, "inspect clone source")?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err(CloneError::WrongKind {
            path: source.to_path_buf(),
            expected: "regular file",
        });
    }
    if !matches_expected(source, &source_metadata, expected)? {
        return Ok(None);
    }

    create_parent(destination)?;
    let stage = staging_path(destination, "file");
    remove_existing(&stage)?;
    let expected_hash = paranoid.then(|| hash_file(source)).transpose()?;
    if platform_clone_file(source, &stage).is_err() {
        remove_existing(&stage)?;
        return Ok(None);
    }

    let Ok(source_after) = inspect(source, "inspect clone source") else {
        remove_existing(&stage)?;
        return Ok(None);
    };
    if !matches_expected(source, &source_after, expected).unwrap_or(false) {
        remove_existing(&stage)?;
        return Ok(None);
    }
    let Ok(stage_metadata) = inspect(&stage, "inspect cloned file") else {
        remove_existing(&stage)?;
        return Ok(None);
    };
    if !stage_metadata.is_file() || stage_metadata.len() != expected.size {
        remove_existing(&stage)?;
        return Ok(None);
    }
    if apply_file_metadata(&stage, expected).is_err() {
        remove_existing(&stage)?;
        return Ok(None);
    }
    if let Some(expected_hash) = expected_hash.as_ref() {
        let Ok(stage_hash) = hash_file(&stage) else {
            remove_existing(&stage)?;
            return Ok(None);
        };
        if stage_hash != *expected_hash {
            remove_existing(&stage)?;
            return Ok(None);
        }
    }

    if let Err(error) = publish_file(&stage, destination) {
        let _ = remove_existing(&stage);
        return Err(error);
    }
    if paranoid {
        let Some(expected_hash) = expected_hash else {
            return Err(CloneError::Verification {
                path: destination.to_path_buf(),
                reason: "paranoid source hash was unavailable".to_owned(),
            });
        };
        if hash_file(destination)? != expected_hash {
            return Err(CloneError::Verification {
                path: destination.to_path_buf(),
                reason: "published file hash differs from source".to_owned(),
            });
        }
    }
    Ok(Some(CloneOutcome {
        kind: CloneKind::File,
        paranoid_verified: paranoid,
    }))
}

/// Try a complete directory clone/reflink without falling back to a byte copy.
///
/// The target must be absent. The supplied scan entries are checked before and
/// after the clone so a metadata-visible source race cannot publish an
/// unverified tree. `paranoid` additionally hashes source, stage, and final
/// tree content.
///
/// # Errors
/// Returns an error for source inspection, staging, publication, or paranoid
/// readback failures.
pub fn try_clone_directory(
    source: &Path,
    target: &Path,
    root: &FileEntry,
    entries: &[FileEntry],
    paranoid: bool,
) -> Result<Option<CloneOutcome>, CloneError> {
    let source_metadata = inspect(source, "inspect directory clone source")?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(CloneError::WrongKind {
            path: source.to_path_buf(),
            expected: "directory",
        });
    }
    if target.exists() || !matches_expected(source, &source_metadata, root)? {
        return Ok(None);
    }
    if !entries_match(source, entries)? {
        return Ok(None);
    }

    create_parent(target)?;
    let stage = staging_path(target, "tree");
    remove_existing(&stage)?;
    let expected_hash = paranoid.then(|| tree_hash(source)).transpose()?;
    if platform_clone_directory(source, &stage).is_err() {
        remove_existing(&stage)?;
        return Ok(None);
    }
    if !entries_match(source, entries)? {
        remove_existing(&stage)?;
        return Ok(None);
    }
    let Ok(stage_metadata) = inspect(&stage, "inspect cloned directory") else {
        remove_existing(&stage)?;
        return Ok(None);
    };
    if !stage_metadata.is_dir() || stage_metadata.file_type().is_symlink() {
        remove_existing(&stage)?;
        return Ok(None);
    }
    if let Some(expected_hash) = expected_hash.as_ref() {
        let Ok(stage_hash) = tree_hash(&stage) else {
            remove_existing(&stage)?;
            return Ok(None);
        };
        if stage_hash != *expected_hash {
            remove_existing(&stage)?;
            return Ok(None);
        }
    }

    if let Err(source_error) = fs::rename(&stage, target) {
        let _ = remove_existing(&stage);
        return Err(io_error("publish cloned directory", target, source_error));
    }
    if paranoid {
        let Some(expected_hash) = expected_hash else {
            return Err(CloneError::Verification {
                path: target.to_path_buf(),
                reason: "paranoid source hash was unavailable".to_owned(),
            });
        };
        if tree_hash(target)? != expected_hash {
            return Err(CloneError::Verification {
                path: target.to_path_buf(),
                reason: "published directory hash differs from source".to_owned(),
            });
        }
    }
    Ok(Some(CloneOutcome {
        kind: CloneKind::Directory,
        paranoid_verified: paranoid,
    }))
}

fn entries_match(source: &Path, entries: &[FileEntry]) -> Result<bool, CloneError> {
    for entry in entries {
        let path = entry.path.to_native_path(source);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source_error) => return Err(io_error("inspect clone entry", &path, source_error)),
        };
        if !matches_expected(&path, &metadata, entry)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn matches_expected(
    path: &Path,
    metadata: &fs::Metadata,
    expected: &FileEntry,
) -> Result<bool, CloneError> {
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
        .map_err(|source| io_error("read clone entry timestamp", path, source))?;
    let fingerprint = fingerprint_from_metadata(metadata, kind, mtime)
        .map_err(|source| io_error("read clone entry fingerprint", path, source))?;
    Ok(fingerprint == expected.fingerprint)
}

fn inspect(path: &Path, operation: &'static str) -> Result<fs::Metadata, CloneError> {
    fs::symlink_metadata(path).map_err(|source| io_error(operation, path, source))
}

fn apply_file_metadata(path: &Path, entry: &FileEntry) -> Result<(), CloneError> {
    filetime::set_file_mtime(path, FileTime::from_system_time(entry.mtime))
        .map_err(|source| io_error("set cloned file mtime", path, source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(entry.mode))
            .map_err(|source| io_error("set cloned file permissions", path, source))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(path)
            .map_err(|source| io_error("inspect cloned file permissions", path, source))?
            .permissions();
        permissions.set_readonly(entry.mode & 0o222 == 0);
        fs::set_permissions(path, permissions)
            .map_err(|source| io_error("set cloned file permissions", path, source))?;
    }
    Ok(())
}

fn publish_file(stage: &Path, destination: &Path) -> Result<(), CloneError> {
    remove_existing(destination)?;
    fs::rename(stage, destination)
        .map_err(|source| io_error("publish cloned file", destination, source))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn platform_clone_file(source: &Path, destination: &Path) -> io::Result<()> {
    let status = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("/bin/cp")
                .args(["-c", "-p"])
                .arg(source)
                .arg(destination)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("cp")
                .args(["--reflink=always", "--preserve=mode,timestamps"])
                .arg(source)
                .arg(destination)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file clone/reflink command failed",
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_clone_file(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file clone is unavailable on this platform",
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn platform_clone_directory(source: &Path, destination: &Path) -> io::Result<()> {
    let status = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("/bin/cp")
                .args(["-c", "-p", "-R"])
                .arg(source)
                .arg(destination)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("cp")
                .args(["-a", "--reflink=always"])
                .arg(source)
                .arg(destination)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory clone/reflink command failed",
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_clone_directory(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory clone is unavailable on this platform",
    ))
}

fn tree_hash(path: &Path) -> Result<blake3::Hash, CloneError> {
    fn visit(path: &Path, relative: &Path, hasher: &mut blake3::Hasher) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        let kind: &[u8] = if metadata.file_type().is_symlink() {
            b"symlink"
        } else if metadata.is_dir() {
            b"directory"
        } else if metadata.is_file() {
            b"file"
        } else {
            b"other"
        };
        hasher.update(kind);
        hasher.update(relative.to_string_lossy().as_bytes());
        if metadata.file_type().is_symlink() {
            hasher.update(fs::read_link(path)?.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            let mut file = File::open(path)?;
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
        } else if metadata.is_dir() {
            let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                visit(&child.path(), &relative.join(child.file_name()), hasher)?;
            }
        }
        Ok(())
    }

    let mut hasher = blake3::Hasher::new();
    visit(path, Path::new("."), &mut hasher)
        .map_err(|source| io_error("hash cloned tree", path, source))?;
    Ok(hasher.finalize())
}

fn hash_file(path: &Path) -> Result<blake3::Hash, CloneError> {
    let mut file = File::open(path).map_err(|source| io_error("hash cloned file", path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| io_error("hash cloned file", path, source))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize())
}

fn staging_path(destination: &Path, kind: &str) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let digest = blake3::hash(destination.as_os_str().to_string_lossy().as_bytes()).to_hex();
    parent.join(format!(".xsync.tmp.clone-{kind}-{digest}"))
}

fn create_parent(path: &Path) -> Result<(), CloneError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error("create clone parent", parent, source))
}

fn remove_existing(path: &Path) -> Result<(), CloneError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("inspect clone stage", path, source)),
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove clone stage", path, source)),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> CloneError {
    CloneError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::scanner::{permission_mode, scan};

    fn scanned_entries(root: &Path) -> Vec<FileEntry> {
        let scan = scan(root).unwrap();
        let entries = scan
            .entries()
            .iter()
            .map(|result| result.unwrap())
            .collect();
        scan.finish().unwrap();
        entries
    }

    fn root_entry(path: &Path) -> FileEntry {
        let metadata = fs::symlink_metadata(path).unwrap();
        let mtime = metadata.modified().unwrap();
        FileEntry {
            path: crate::path::WirePath::default(),
            kind: EntryKind::Directory,
            size: metadata.len(),
            mtime,
            mode: permission_mode(&metadata),
            fingerprint: fingerprint_from_metadata(&metadata, EntryKind::Directory, mtime).unwrap(),
        }
    }

    #[test]
    fn file_clone_is_atomic_and_capability_failure_leaves_no_destination() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"clone me").unwrap();
        let entry = scanned_entries(temp.path())
            .into_iter()
            .find(|entry| entry.path == "source")
            .unwrap();

        let outcome = try_clone_file(&source, &destination, &entry, true).unwrap();
        if let Some(outcome) = outcome {
            assert_eq!(outcome.kind, CloneKind::File);
            assert!(outcome.paranoid_verified);
            assert_eq!(fs::read(&destination).unwrap(), b"clone me");
            fs::write(&destination, b"destination only").unwrap();
            assert_eq!(fs::read(&source).unwrap(), b"clone me");
        } else {
            assert!(!destination.exists());
        }
    }

    #[test]
    fn directory_clone_preserves_tree_or_reports_unavailable_without_mutation() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("nested/file"), b"tree").unwrap();
        let entries = scanned_entries(&source);
        let root = root_entry(&source);

        let outcome = try_clone_directory(&source, &target, &root, &entries, true).unwrap();
        if let Some(outcome) = outcome {
            assert_eq!(outcome.kind, CloneKind::Directory);
            assert!(outcome.paranoid_verified);
            assert_eq!(fs::read(target.join("nested/file")).unwrap(), b"tree");
            fs::write(target.join("nested/file"), b"changed destination").unwrap();
            assert_eq!(fs::read(source.join("nested/file")).unwrap(), b"tree");
        } else {
            assert!(!target.exists());
        }
    }

    #[test]
    fn directory_clone_requires_an_absent_target() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let entries = scanned_entries(&source);
        let root = root_entry(&source);

        assert!(
            try_clone_directory(&source, &target, &root, &entries, false)
                .unwrap()
                .is_none()
        );
        assert!(target.is_dir());
    }
}
