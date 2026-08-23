//! Owned benchmark scratch directories with defensive cleanup.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MARKER: &str = ".xsync-bench-owned";
const MARKER_SCHEMA: &str = "xsync.bench.scratch.v1";
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One marker-owned benchmark run directory.
#[derive(Debug)]
pub struct OwnedScratch {
    path: PathBuf,
    base: PathBuf,
    id: String,
}

impl OwnedScratch {
    /// Create a unique, marker-owned run below `base`.
    ///
    /// # Errors
    ///
    /// Returns an error when the base or run directory cannot be created.
    pub fn create(base: impl AsRef<Path>) -> Result<Self, ScratchError> {
        fs::create_dir_all(base.as_ref()).map_err(|source| ScratchError::Io {
            path: base.as_ref().to_path_buf(),
            source,
        })?;
        let base = fs::canonicalize(base.as_ref()).map_err(|source| ScratchError::Io {
            path: base.as_ref().to_path_buf(),
            source,
        })?;
        for _ in 0..100 {
            let id = run_id()?;
            let path = base.join(format!("run-{id}"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let marker_path = path.join(MARKER);
                    let mut marker = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&marker_path)
                        .map_err(|source| ScratchError::Io {
                            path: marker_path.clone(),
                            source,
                        })?;
                    writeln!(marker, "{MARKER_SCHEMA}\n{id}").map_err(|source| {
                        ScratchError::Io {
                            path: marker_path,
                            source,
                        }
                    })?;
                    return Ok(Self { path, base, id });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(ScratchError::Io {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }
        Err(ScratchError::ExhaustedIds)
    }

    /// Owned run path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Canonical scratch base.
    #[must_use]
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Unique marker identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Remove this owned run after revalidating every cleanup guard.
    ///
    /// # Errors
    ///
    /// Returns an error instead of deleting when containment, marker, or
    /// dangerous-target checks fail.
    pub fn clean(self) -> Result<(), ScratchError> {
        clean_owned(&self.path, &self.base)
    }
}

/// Defensively remove a marker-owned run below an expected base.
///
/// # Errors
///
/// Returns an error instead of deleting an unmarked, escaped, broad, home, or
/// repository target.
pub fn clean_owned(root: impl AsRef<Path>, base: impl AsRef<Path>) -> Result<(), ScratchError> {
    let requested = fs::canonicalize(root.as_ref()).map_err(|source| ScratchError::Io {
        path: root.as_ref().to_path_buf(),
        source,
    })?;
    let base = fs::canonicalize(base.as_ref()).map_err(|source| ScratchError::Io {
        path: base.as_ref().to_path_buf(),
        source,
    })?;
    if requested == Path::new("/") || requested == base {
        return Err(ScratchError::UnsafeTarget(requested));
    }
    if requested
        .strip_prefix(&base)
        .ok()
        .is_none_or(|relative| relative.components().count() != 1)
    {
        return Err(ScratchError::OutsideBase {
            root: requested,
            base,
        });
    }
    if is_home_or_repository(&requested) {
        return Err(ScratchError::UnsafeTarget(requested));
    }

    let marker_path = requested.join(MARKER);
    let marker_metadata =
        fs::symlink_metadata(&marker_path).map_err(|_| ScratchError::MissingMarker {
            root: requested.clone(),
        })?;
    if !marker_metadata.file_type().is_file() || marker_metadata.file_type().is_symlink() {
        return Err(ScratchError::MissingMarker { root: requested });
    }
    let marker = fs::read_to_string(&marker_path).map_err(|source| ScratchError::Io {
        path: marker_path,
        source,
    })?;
    let mut lines = marker.lines();
    let schema = lines.next();
    let marker_id = lines.next();
    let expected_name = marker_id.map(|id| format!("run-{id}"));
    if schema != Some(MARKER_SCHEMA)
        || marker_id.is_none_or(str::is_empty)
        || lines.next().is_some()
        || requested.file_name().and_then(|name| name.to_str()) != expected_name.as_deref()
    {
        return Err(ScratchError::InvalidMarker { root: requested });
    }
    fs::remove_dir_all(&requested).map_err(|source| ScratchError::Io {
        path: requested,
        source,
    })
}

/// Scratch ownership and cleanup failures.
#[derive(Debug, thiserror::Error)]
pub enum ScratchError {
    /// Filesystem error.
    #[error("scratch I/O failed at '{}': {source}", path.display())]
    Io {
        /// Failing path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Unique run allocation repeatedly collided.
    #[error("could not allocate a unique benchmark scratch id")]
    ExhaustedIds,
    /// Cleanup target is too broad or special.
    #[error("refusing unsafe scratch cleanup target '{}'", .0.display())]
    UnsafeTarget(PathBuf),
    /// Cleanup target escaped or was not a direct run child.
    #[error("scratch root '{}' is not one direct child of base '{}'", root.display(), base.display())]
    OutsideBase {
        /// Requested root.
        root: PathBuf,
        /// Expected base.
        base: PathBuf,
    },
    /// Ownership marker is absent or not a regular file.
    #[error("scratch root '{}' has no valid ownership marker", root.display())]
    MissingMarker {
        /// Requested root.
        root: PathBuf,
    },
    /// Ownership marker contents are invalid.
    #[error("scratch root '{}' has an invalid ownership marker", root.display())]
    InvalidMarker {
        /// Requested root.
        root: PathBuf,
    },
    /// System clock cannot produce an identifier.
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
}

fn run_id() -> Result<String, ScratchError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ScratchError::InvalidClock)?
        .as_nanos();
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{}-{nanos}-{sequence}", std::process::id()))
}

fn is_home_or_repository(path: &Path) -> bool {
    let home = std::env::var_os("HOME")
        .and_then(|value| fs::canonicalize(value).ok())
        .is_some_and(|home| path == home);
    let repository = std::env::current_dir()
        .ok()
        .and_then(|value| fs::canonicalize(value).ok())
        .is_some_and(|repository| path == repository);
    home || repository
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn creates_marker_owned_run_and_cleans_only_it() {
        let parent = tempdir().unwrap();
        let base = parent.path().join("scratch");
        let owned = OwnedScratch::create(&base).unwrap();
        let path = owned.path().to_path_buf();
        assert!(path.join(MARKER).is_file());
        fs::write(path.join("payload"), b"x").unwrap();

        owned.clean().unwrap();
        assert!(!path.exists());
        assert!(base.exists());
    }

    #[test]
    fn refuses_unmarked_base_nested_and_outside_targets() {
        let parent = tempdir().unwrap();
        let base = parent.path().join("scratch");
        fs::create_dir(&base).unwrap();
        let unmarked = base.join("unmarked");
        fs::create_dir(&unmarked).unwrap();
        assert!(matches!(
            clean_owned(&unmarked, &base),
            Err(ScratchError::MissingMarker { .. })
        ));
        assert!(matches!(
            clean_owned(&base, &base),
            Err(ScratchError::UnsafeTarget(_))
        ));

        let nested = base.join("parent/child");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(MARKER), format!("{MARKER_SCHEMA}\nid\n")).unwrap();
        assert!(matches!(
            clean_owned(&nested, &base),
            Err(ScratchError::OutsideBase { .. })
        ));

        let outside = parent.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join(MARKER), format!("{MARKER_SCHEMA}\nid\n")).unwrap();
        assert!(matches!(
            clean_owned(&outside, &base),
            Err(ScratchError::OutsideBase { .. })
        ));
    }

    #[test]
    fn refuses_tampered_marker() {
        let parent = tempdir().unwrap();
        let owned = OwnedScratch::create(parent.path()).unwrap();
        fs::write(
            owned.path().join(MARKER),
            format!("{MARKER_SCHEMA}\na-different-run-id\n"),
        )
        .unwrap();
        assert!(matches!(
            owned.clean(),
            Err(ScratchError::InvalidMarker { .. })
        ));
    }
}
