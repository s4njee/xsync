//! Correctness-first local clone/reflink spike.
//!
//! Clone attempts always target a deterministic staging path. A capability
//! failure removes any partial stage and falls back to an ordinary copy. Basic
//! validation precedes publication; paranoid mode adds content readback. The
//! benchmark runner independently manifests every published result.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::{Command, Stdio};

use filetime::{set_file_times, set_symlink_file_times, FileTime};
use serde::{Deserialize, Serialize};
use xsync_bench::manifest::{build_manifest, verify_manifest, ManifestError};

/// Which data path produced the verified destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneDisposition {
    /// The platform clone/reflink capability succeeded.
    Cloned,
    /// Clone was unavailable or invalid and ordinary copy completed instead.
    BufferedFallback,
}

/// Result of a staged, verified clone attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneOutcome {
    /// Selected data path.
    pub disposition: CloneDisposition,
    /// Whether final-name readback was requested and passed.
    pub paranoid_verified: bool,
}

/// Conditions that must all hold before cloning an entire directory root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectoryClonePolicy {
    /// The request has at least one exclusion.
    pub has_exclusions: bool,
    /// Delete semantics are requested.
    pub delete: bool,
    /// The destination must be merged with an existing tree.
    pub merge: bool,
}

/// Clone/copy spike failure.
#[derive(Debug, thiserror::Error)]
pub enum CloneError {
    /// Filesystem operation failure.
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        /// Short operation description.
        operation: &'static str,
        /// Failing path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Source is not the expected object kind.
    #[error("clone source '{}' is not a {expected}", path.display())]
    WrongKind {
        /// Invalid source.
        path: PathBuf,
        /// Required kind.
        expected: &'static str,
    },
    /// Staged or paranoid verification failed.
    #[error("verified clone/copy mismatch at '{}': {reason}", path.display())]
    Verification {
        /// Invalid result path.
        path: PathBuf,
        /// Mismatch description.
        reason: String,
    },
    /// Directory root is not eligible for a whole-tree clone.
    #[error("directory clone is ineligible for this request")]
    IneligibleDirectory,
    /// Independent manifest failure.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

/// Return the observable destination tree root for trailing-slash semantics.
///
/// `source_contents=true` models `source/ destination` and returns
/// `destination`. Otherwise it models `source destination` and returns
/// `destination/source-basename`.
#[must_use]
pub fn directory_clone_target(source: &Path, destination: &Path, source_contents: bool) -> PathBuf {
    if source_contents {
        destination.to_path_buf()
    } else {
        source
            .file_name()
            .map_or_else(|| destination.to_path_buf(), |name| destination.join(name))
    }
}

/// Whether a whole-tree clone can preserve the complete requested semantics.
#[must_use]
pub fn directory_clone_eligible(target: &Path, policy: DirectoryClonePolicy) -> bool {
    !target.exists() && !policy.has_exclusions && !policy.delete && !policy.merge
}

/// Attempt a platform file clone, then transparently fall back to physical copy.
///
/// Existing destination content remains until the staged result is verified.
/// `paranoid` additionally reads back and hashes the published final name.
///
/// # Errors
///
/// Returns an error for an invalid source, a copy/metadata/publication failure,
/// or any verification mismatch.
pub fn clone_file_or_fallback(
    source: &Path,
    destination: &Path,
    paranoid: bool,
) -> Result<CloneOutcome, CloneError> {
    clone_file_or_fallback_with(source, destination, paranoid, platform_clone_file)
}

/// Produce an ordinary staged physical file copy for the paired baseline.
///
/// # Errors
///
/// Returns the same source, copy, metadata, publication, and verification
/// errors as [`clone_file_or_fallback`].
pub fn copy_file_baseline(
    source: &Path,
    destination: &Path,
    paranoid: bool,
) -> Result<CloneOutcome, CloneError> {
    clone_file_or_fallback_with(source, destination, paranoid, |_, _| {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ordinary-copy baseline",
        ))
    })
}

fn clone_file_or_fallback_with<F>(
    source: &Path,
    destination: &Path,
    paranoid: bool,
    clone_attempt: F,
) -> Result<CloneOutcome, CloneError>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|source_error| io_error("inspect clone source", source, source_error))?;
    if !source_metadata.is_file() {
        return Err(CloneError::WrongKind {
            path: source.to_path_buf(),
            expected: "regular file",
        });
    }
    let expected_length = source_metadata.len();
    let expected_hash = paranoid.then(|| hash_file(source)).transpose()?;
    create_parent(destination)?;
    let stage = staging_path(destination, "file");
    remove_file_if_present(&stage)?;

    let clone_valid = clone_attempt(source, &stage).is_ok()
        && validate_staged_file(source, &stage, expected_length, expected_hash.as_ref())
            .unwrap_or(false);
    let disposition = if clone_valid {
        CloneDisposition::Cloned
    } else {
        remove_file_if_present(&stage)?;
        buffered_copy_file(source, &stage)?;
        if !validate_staged_file(source, &stage, expected_length, expected_hash.as_ref())? {
            remove_file_if_present(&stage)?;
            return Err(CloneError::Verification {
                path: stage,
                reason: "buffered fallback length or BLAKE3 differs".to_owned(),
            });
        }
        CloneDisposition::BufferedFallback
    };
    if let Err(error) = publish_staged_file(destination, &stage, expected_hash.as_ref()) {
        let _ = remove_file_if_present(&stage);
        return Err(error);
    }
    Ok(CloneOutcome {
        disposition,
        paranoid_verified: paranoid,
    })
}

/// Clone a complete eligible directory root, falling back to physical copy.
///
/// The target must not exist and the policy must have no exclusions, delete,
/// or merge semantics. Both clone and fallback operate on a sibling staging
/// tree. Paranoid mode independently manifests both the stage and final name.
///
/// # Errors
///
/// Returns an error when the request is ineligible, source is not a directory,
/// the clone/copy cannot be produced, or paranoid verification fails.
pub fn clone_directory_or_fallback(
    source: &Path,
    target: &Path,
    policy: DirectoryClonePolicy,
    paranoid: bool,
) -> Result<CloneOutcome, CloneError> {
    clone_directory_with(source, target, policy, paranoid, true)
}

/// Produce an ordinary staged physical directory copy for the paired baseline.
///
/// # Errors
///
/// Returns the same eligibility, source, copy, metadata, publication, and
/// verification errors as [`clone_directory_or_fallback`].
pub fn copy_directory_baseline(
    source: &Path,
    target: &Path,
    policy: DirectoryClonePolicy,
    paranoid: bool,
) -> Result<CloneOutcome, CloneError> {
    clone_directory_with(source, target, policy, paranoid, false)
}

fn clone_directory_with(
    source: &Path,
    target: &Path,
    policy: DirectoryClonePolicy,
    paranoid: bool,
    attempt_clone: bool,
) -> Result<CloneOutcome, CloneError> {
    if !directory_clone_eligible(target, policy) {
        return Err(CloneError::IneligibleDirectory);
    }
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|source_error| io_error("inspect directory clone source", source, source_error))?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(CloneError::WrongKind {
            path: source.to_path_buf(),
            expected: "directory",
        });
    }
    create_parent(target)?;
    let stage = staging_path(target, "tree");
    remove_tree_if_present(&stage)?;
    let expected = paranoid.then(|| build_manifest(source)).transpose()?;

    let mut disposition = CloneDisposition::Cloned;
    let clone_valid = attempt_clone
        && platform_clone_directory(source, &stage).is_ok()
        && expected.as_ref().is_none_or(|manifest| {
            verify_manifest(&stage, manifest).is_ok_and(|verification| verification.passed)
        });
    if !clone_valid {
        disposition = CloneDisposition::BufferedFallback;
        remove_tree_if_present(&stage)?;
        copy_tree(source, &stage)?;
    }
    let verification_passed = expected
        .as_ref()
        .is_none_or(|manifest| verify_manifest(&stage, manifest).is_ok_and(|result| result.passed));
    if !verification_passed {
        remove_tree_if_present(&stage)?;
        return Err(CloneError::Verification {
            path: stage,
            reason: "staged tree manifest differs".to_owned(),
        });
    }
    fs::rename(&stage, target)
        .map_err(|source_error| io_error("publish staged directory", target, source_error))?;
    if let Some(expected) = expected {
        let readback = verify_manifest(target, &expected)?;
        if !readback.passed {
            return Err(CloneError::Verification {
                path: target.to_path_buf(),
                reason: format!("{} paranoid mismatch(es)", readback.mismatch_count),
            });
        }
    }
    Ok(CloneOutcome {
        disposition,
        paranoid_verified: paranoid,
    })
}

fn publish_staged_file(
    destination: &Path,
    stage: &Path,
    expected_hash: Option<&blake3::Hash>,
) -> Result<(), CloneError> {
    fs::rename(stage, destination)
        .map_err(|source_error| io_error("publish staged file", destination, source_error))?;
    if let Some(expected_hash) = expected_hash {
        if hash_file(destination)? != *expected_hash {
            return Err(CloneError::Verification {
                path: destination.to_path_buf(),
                reason: "paranoid final-name BLAKE3 differs".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_staged_file(
    source: &Path,
    stage: &Path,
    expected_length: u64,
    expected_hash: Option<&blake3::Hash>,
) -> Result<bool, CloneError> {
    apply_metadata(source, stage)?;
    let stage_metadata = fs::metadata(stage)
        .map_err(|source_error| io_error("inspect staged file", stage, source_error))?;
    if stage_metadata.len() != expected_length {
        return Ok(false);
    }
    expected_hash.map_or(Ok(true), |hash| Ok(hash_file(stage)? == *hash))
}

fn platform_clone_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("/bin/cp")
        .args(["-c", "-p"])
        .arg(source)
        .arg(destination)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(target_os = "linux")]
    let status = Command::new("cp")
        .args(["--reflink=always", "--preserve=mode,timestamps"])
        .arg(source)
        .arg(destination)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "platform clone/reflink command failed",
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "platform clone probe unavailable",
        ))
    }
}

fn platform_clone_directory(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("/bin/cp")
        .args(["-c", "-p", "-R"])
        .arg(source)
        .arg(destination)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(target_os = "linux")]
    let status = Command::new("cp")
        .args(["-a", "--reflink=always"])
        .arg(source)
        .arg(destination)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "platform directory clone/reflink command failed",
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "platform directory clone probe unavailable",
        ))
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), CloneError> {
    fs::create_dir(destination)
        .map_err(|source_error| io_error("create fallback tree", destination, source_error))?;
    let mut directories = vec![(source.to_path_buf(), destination.to_path_buf())];
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source_dir, destination_dir)) = pending.pop() {
        let mut children = fs::read_dir(&source_dir)
            .map_err(|source_error| io_error("read fallback source", &source_dir, source_error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source_error| io_error("read fallback entry", &source_dir, source_error))?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let source_path = child.path();
            let destination_path = destination_dir.join(child.file_name());
            let metadata = fs::symlink_metadata(&source_path).map_err(|source_error| {
                io_error("inspect fallback source", &source_path, source_error)
            })?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::create_dir(&destination_path).map_err(|source_error| {
                    io_error("create fallback directory", &destination_path, source_error)
                })?;
                directories.push((source_path.clone(), destination_path.clone()));
                pending.push((source_path, destination_path));
            } else if metadata.file_type().is_symlink() {
                copy_symlink(&source_path, &destination_path)?;
            } else if metadata.is_file() {
                buffered_copy_file(&source_path, &destination_path)?;
                apply_metadata(&source_path, &destination_path)?;
            }
        }
    }
    directories.sort_by_key(|(_, path)| std::cmp::Reverse(path.components().count()));
    for (source_dir, destination_dir) in directories {
        apply_metadata(&source_dir, &destination_dir)?;
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), CloneError> {
    use std::os::unix::fs::symlink;

    let target = fs::read_link(source)
        .map_err(|source_error| io_error("read fallback symlink", source, source_error))?;
    symlink(target, destination)
        .map_err(|source_error| io_error("create fallback symlink", destination, source_error))?;
    let metadata = fs::symlink_metadata(source)
        .map_err(|source_error| io_error("inspect fallback symlink", source, source_error))?;
    let accessed = FileTime::from_last_access_time(&metadata);
    let modified = FileTime::from_last_modification_time(&metadata);
    set_symlink_file_times(destination, accessed, modified)
        .map_err(|source_error| io_error("set fallback symlink times", destination, source_error))
}

#[cfg(windows)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), CloneError> {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    let target = fs::read_link(source)
        .map_err(|source_error| io_error("read fallback symlink", source, source_error))?;
    let target_is_directory = source.metadata().is_ok_and(|metadata| metadata.is_dir());
    let result = if target_is_directory {
        symlink_dir(target, destination)
    } else {
        symlink_file(target, destination)
    };
    result.map_err(|source_error| io_error("create fallback symlink", destination, source_error))
}

fn apply_metadata(source: &Path, destination: &Path) -> Result<(), CloneError> {
    let metadata = fs::metadata(source)
        .map_err(|source_error| io_error("inspect source metadata", source, source_error))?;
    fs::set_permissions(destination, metadata.permissions()).map_err(|source_error| {
        io_error(
            "preserve destination permissions",
            destination,
            source_error,
        )
    })?;
    set_file_times(
        destination,
        FileTime::from_last_access_time(&metadata),
        FileTime::from_last_modification_time(&metadata),
    )
    .map_err(|source_error| io_error("preserve destination times", destination, source_error))
}

fn hash_file(path: &Path) -> Result<blake3::Hash, CloneError> {
    let mut file = File::open(path)
        .map_err(|source_error| io_error("hash clone/copy file", path, source_error))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source_error| io_error("hash clone/copy file", path, source_error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize())
}

fn buffered_copy_file(source: &Path, destination: &Path) -> Result<(), CloneError> {
    let mut input = File::open(source)
        .map_err(|source_error| io_error("open buffered-copy source", source, source_error))?;
    let mut output = File::create(destination).map_err(|source_error| {
        io_error(
            "create buffered-copy destination",
            destination,
            source_error,
        )
    })?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|source_error| io_error("read buffered-copy source", source, source_error))?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).map_err(|source_error| {
            io_error("write buffered-copy destination", destination, source_error)
        })?;
    }
    Ok(())
}

fn staging_path(destination: &Path, kind: &str) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let identity = destination.as_os_str().to_string_lossy();
    let digest = blake3::hash(identity.as_bytes()).to_hex();
    parent.join(format!(".xsync.tmp.clone-{kind}-{digest}"))
}

fn create_parent(path: &Path) -> Result<(), CloneError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|source_error| io_error("create clone destination parent", parent, source_error))
}

fn remove_file_if_present(path: &Path) -> Result<(), CloneError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source_error) => Err(io_error("remove stale file stage", path, source_error)),
    }
}

fn remove_tree_if_present(path: &Path) -> Result<(), CloneError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source_error) => {
            return Err(io_error("inspect stale tree stage", path, source_error));
        }
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source_error) => Err(io_error("remove stale tree stage", path, source_error)),
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
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn failed_clone_falls_back_without_exposing_partial_or_losing_existing() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"new complete contents").unwrap();
        fs::write(&destination, b"old destination").unwrap();

        let outcome = clone_file_or_fallback_with(&source, &destination, true, |_, stage| {
            fs::write(stage, b"partial")?;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "injected capability failure",
            ))
        })
        .unwrap();

        assert_eq!(outcome.disposition, CloneDisposition::BufferedFallback);
        assert!(outcome.paranoid_verified);
        assert_eq!(fs::read(&destination).unwrap(), b"new complete contents");
        assert!(!staging_path(&destination, "file").exists());
    }

    #[test]
    fn invalid_successful_clone_falls_back_before_publication() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"complete contents").unwrap();

        let outcome = clone_file_or_fallback_with(&source, &destination, true, |_, stage| {
            fs::write(stage, b"corrupt but successful")
        })
        .unwrap();

        assert_eq!(outcome.disposition, CloneDisposition::BufferedFallback);
        assert_eq!(fs::read(destination).unwrap(), b"complete contents");
    }

    #[test]
    fn file_clone_or_fallback_preserves_metadata_and_copy_independence() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"original").unwrap();
        let time = FileTime::from_unix_time(1_700_000_000, 0);
        set_file_times(&source, time, time).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        }

        let outcome = clone_file_or_fallback(&source, &destination, true).unwrap();
        assert!(matches!(
            outcome.disposition,
            CloneDisposition::Cloned | CloneDisposition::BufferedFallback
        ));
        let source_metadata = fs::metadata(&source).unwrap();
        let destination_metadata = fs::metadata(&destination).unwrap();
        assert_eq!(
            FileTime::from_last_modification_time(&source_metadata),
            FileTime::from_last_modification_time(&destination_metadata)
        );
        assert_eq!(
            source_metadata.permissions(),
            destination_metadata.permissions()
        );

        let mut source_file = fs::OpenOptions::new().write(true).open(&source).unwrap();
        source_file.write_all(b"changed!").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"original");
    }

    #[test]
    fn ordinary_file_baseline_reports_buffered_fallback() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"baseline").unwrap();

        let outcome = copy_file_baseline(&source, &destination, true).unwrap();

        assert_eq!(outcome.disposition, CloneDisposition::BufferedFallback);
        assert_eq!(fs::read(destination).unwrap(), b"baseline");
    }

    #[test]
    fn whole_tree_gate_covers_trailing_slash_excludes_delete_merge_and_existing() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        let without_slash = directory_clone_target(&source, &destination, false);
        let with_slash = directory_clone_target(&source, &destination, true);
        assert_eq!(without_slash, destination.join("source"));
        assert_eq!(with_slash, destination);
        assert!(directory_clone_eligible(
            &without_slash,
            DirectoryClonePolicy::default()
        ));
        assert!(!directory_clone_eligible(
            &without_slash,
            DirectoryClonePolicy {
                has_exclusions: true,
                ..DirectoryClonePolicy::default()
            }
        ));
        assert!(!directory_clone_eligible(
            &without_slash,
            DirectoryClonePolicy {
                delete: true,
                ..DirectoryClonePolicy::default()
            }
        ));
        assert!(!directory_clone_eligible(
            &without_slash,
            DirectoryClonePolicy {
                merge: true,
                ..DirectoryClonePolicy::default()
            }
        ));
        fs::create_dir_all(&without_slash).unwrap();
        assert!(!directory_clone_eligible(
            &without_slash,
            DirectoryClonePolicy::default()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn complete_tree_clone_or_fallback_is_manifest_exact_and_independent() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::write(source.join("file"), b"original").unwrap();
        symlink("file", source.join("link")).unwrap();
        fs::set_permissions(source.join("file"), fs::Permissions::from_mode(0o640)).unwrap();
        let time = FileTime::from_unix_time(1_700_000_000, 0);
        for path in [source.join("file"), source.join("empty"), source.clone()] {
            set_file_times(path, time, time).unwrap();
        }
        set_symlink_file_times(source.join("link"), time, time).unwrap();

        let outcome =
            clone_directory_or_fallback(&source, &target, DirectoryClonePolicy::default(), true)
                .unwrap();
        assert!(matches!(
            outcome.disposition,
            CloneDisposition::Cloned | CloneDisposition::BufferedFallback
        ));
        fs::write(source.join("file"), b"mutated!").unwrap();
        assert_eq!(fs::read(target.join("file")).unwrap(), b"original");
        assert!(target.join("empty").is_dir());
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            Path::new("file")
        );
    }
}
