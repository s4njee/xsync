//! Filesystem manifest oracle implemented independently from `xsync-core`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Versioned manifest schema.
pub const MANIFEST_SCHEMA: &str = "xsync.manifest.v1";
/// Maximum mismatch details retained in a verification result.
pub const MAX_MISMATCH_DETAILS: usize = 100;

/// Reversible platform path components, encoded as lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EncodedPath {
    /// One encoded native component per relative path component.
    pub components_hex: Vec<String>,
}

impl EncodedPath {
    fn root() -> Self {
        Self {
            components_hex: Vec::new(),
        }
    }

    fn display(&self) -> String {
        if self.components_hex.is_empty() {
            ".".to_owned()
        } else {
            self.components_hex.join("/")
        }
    }
}

/// Filesystem object kind captured by the oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
    /// FIFO, socket, device, or another platform-specific object.
    Other,
}

/// Exact modification timestamp representation used by the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ManifestTime {
    /// Signed seconds relative to the Unix epoch.
    pub seconds: i64,
    /// Nanoseconds within the represented second.
    pub nanoseconds: u32,
}

/// One independently inspected filesystem entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Path relative to the inspected root. Empty components represent the root itself.
    pub path: EncodedPath,
    /// Filesystem object kind.
    pub kind: ManifestKind,
    /// Logical file length; zero for non-files.
    pub length: u64,
    /// BLAKE3 content digest for regular files.
    pub content_blake3: Option<String>,
    /// Permission and special mode bits where the platform exposes them.
    pub mode: u32,
    /// Last modification timestamp.
    pub mtime: ManifestTime,
    /// Raw native symlink target encoding.
    pub symlink_target_hex: Option<String>,
}

/// Content-pinned, deterministic filesystem manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema identifier.
    pub schema: String,
    /// Path byte encoding used for this manifest.
    pub path_encoding: String,
    /// Digest algorithm used for file content and the manifest.
    pub digest_algorithm: String,
    /// Number of entries, including the inspected root.
    pub item_count: u64,
    /// Sum of regular-file logical lengths.
    pub logical_bytes: u64,
    /// Canonical digest of all entry fields.
    pub manifest_digest: String,
    /// Entries sorted by native encoded relative path.
    pub entries: Vec<ManifestEntry>,
}

/// A bounded explanation of one oracle mismatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestMismatch {
    /// Encoded path, or `.` for the root.
    pub path: String,
    /// Human-readable mismatch class.
    pub reason: String,
}

/// Result of comparing a destination to an expected manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// True only when every entry and manifest digest matches.
    pub passed: bool,
    /// Expected manifest digest.
    pub expected_manifest_digest: String,
    /// Actual destination manifest digest.
    pub actual_manifest_digest: String,
    /// Actual item count.
    pub item_count: u64,
    /// Actual logical bytes.
    pub logical_bytes: u64,
    /// Total mismatches, which may exceed `mismatches.len()`.
    pub mismatch_count: u64,
    /// First bounded mismatch details.
    pub mismatches: Vec<ManifestMismatch>,
}

/// Manifest creation and validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// An inspected path could not be read.
    #[error("cannot inspect '{}': {source}", path.display())]
    Inspect {
        /// Failing path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// A regular file could not be hashed.
    #[error("cannot hash '{}': {source}", path.display())]
    Hash {
        /// Failing path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// An expected manifest was modified or decoded incorrectly.
    #[error("manifest digest is invalid: recorded {recorded}, computed {computed}")]
    InvalidDigest {
        /// Digest stored in the manifest.
        recorded: String,
        /// Digest computed from its entries.
        computed: String,
    },
    /// An entry count cannot be represented by the manifest schema.
    #[error("manifest contains too many entries")]
    TooManyEntries,
}

/// Inspect a filesystem tree and return a content-pinned manifest.
///
/// The root itself is included. Symlinks are recorded but never followed.
///
/// # Errors
///
/// Returns an error when metadata, directory contents, link targets, or file
/// content cannot be read consistently enough to create the manifest.
pub fn build_manifest(root: impl AsRef<Path>) -> Result<Manifest, ManifestError> {
    let root = root.as_ref();
    let mut entries = Vec::new();
    visit(root, &EncodedPath::root(), &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let item_count = u64::try_from(entries.len()).map_err(|_| ManifestError::TooManyEntries)?;
    let logical_bytes = entries
        .iter()
        .filter(|entry| entry.kind == ManifestKind::File)
        .map(|entry| entry.length)
        .sum();
    let manifest_digest = digest_entries(&entries);
    Ok(Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        path_encoding: platform_path_encoding().to_owned(),
        digest_algorithm: "blake3".to_owned(),
        item_count,
        logical_bytes,
        manifest_digest,
        entries,
    })
}

/// Verify a filesystem tree against an independently generated manifest.
///
/// # Errors
///
/// Returns an error when the expected manifest is internally invalid or the
/// destination cannot be inspected.
pub fn verify_manifest(
    root: impl AsRef<Path>,
    expected: &Manifest,
) -> Result<Verification, ManifestError> {
    let expected_digest = digest_entries(&expected.entries);
    if expected_digest != expected.manifest_digest {
        return Err(ManifestError::InvalidDigest {
            recorded: expected.manifest_digest.clone(),
            computed: expected_digest,
        });
    }

    let actual = build_manifest(root)?;
    let expected_by_path: BTreeMap<_, _> = expected
        .entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect();
    let actual_by_path: BTreeMap<_, _> = actual
        .entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect();
    let paths: BTreeSet<_> = expected_by_path
        .keys()
        .chain(actual_by_path.keys())
        .copied()
        .collect();
    let mut mismatch_count = 0_u64;
    let mut mismatches = Vec::new();

    for path in paths {
        let reason = match (expected_by_path.get(path), actual_by_path.get(path)) {
            (Some(_), None) => Some("missing destination entry".to_owned()),
            (None, Some(_)) => Some("unexpected destination entry".to_owned()),
            (Some(expected_entry), Some(actual_entry)) if expected_entry != actual_entry => {
                Some(entry_difference(expected_entry, actual_entry))
            }
            _ => None,
        };
        if let Some(reason) = reason {
            mismatch_count += 1;
            if mismatches.len() < MAX_MISMATCH_DETAILS {
                mismatches.push(ManifestMismatch {
                    path: path.display(),
                    reason,
                });
            }
        }
    }

    Ok(Verification {
        passed: mismatch_count == 0 && actual.manifest_digest == expected.manifest_digest,
        expected_manifest_digest: expected.manifest_digest.clone(),
        actual_manifest_digest: actual.manifest_digest,
        item_count: actual.item_count,
        logical_bytes: actual.logical_bytes,
        mismatch_count,
        mismatches,
    })
}

fn visit(
    path: &Path,
    encoded_path: &EncodedPath,
    entries: &mut Vec<ManifestEntry>,
) -> Result<(), ManifestError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| ManifestError::Inspect {
        path: path.to_path_buf(),
        source,
    })?;
    let kind = manifest_kind(&metadata);
    let content_blake3 = if kind == ManifestKind::File {
        Some(hash_file(path)?)
    } else {
        None
    };
    let symlink_target_hex = if kind == ManifestKind::Symlink {
        let target = fs::read_link(path).map_err(|source| ManifestError::Inspect {
            path: path.to_path_buf(),
            source,
        })?;
        Some(hex(&os_bytes(target.as_os_str())))
    } else {
        None
    };
    entries.push(ManifestEntry {
        path: encoded_path.clone(),
        kind,
        length: if kind == ManifestKind::File {
            metadata.len()
        } else {
            0
        },
        content_blake3,
        mode: metadata_mode(&metadata),
        mtime: metadata_mtime(&metadata).map_err(|source| ManifestError::Inspect {
            path: path.to_path_buf(),
            source,
        })?,
        symlink_target_hex,
    });

    if kind == ManifestKind::Directory {
        let directory = fs::read_dir(path).map_err(|source| ManifestError::Inspect {
            path: path.to_path_buf(),
            source,
        })?;
        let mut children = directory
            .map(|result| {
                result
                    .map(|entry| {
                        let name = entry.file_name();
                        let key = os_bytes(&name);
                        (key, name, entry.path())
                    })
                    .map_err(|source| ManifestError::Inspect {
                        path: path.to_path_buf(),
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, _name, child_path) in children {
            let mut components = encoded_path.components_hex.clone();
            components.push(hex(&key));
            visit(
                &child_path,
                &EncodedPath {
                    components_hex: components,
                },
                entries,
            )?;
        }
    }
    Ok(())
}

fn manifest_kind(metadata: &Metadata) -> ManifestKind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        ManifestKind::Symlink
    } else if file_type.is_file() {
        ManifestKind::File
    } else if file_type.is_dir() {
        ManifestKind::Directory
    } else {
        ManifestKind::Other
    }
}

fn hash_file(path: &Path) -> Result<String, ManifestError> {
    let mut file = File::open(path).map_err(|source| ManifestError::Hash {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| ManifestError::Hash {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn digest_entries(entries: &[ManifestEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, MANIFEST_SCHEMA.as_bytes());
    hash_field(&mut hasher, platform_path_encoding().as_bytes());
    for entry in entries {
        hash_u64(&mut hasher, entry.path.components_hex.len() as u64);
        for component in &entry.path.components_hex {
            hash_field(&mut hasher, component.as_bytes());
        }
        hasher.update(&[entry.kind as u8]);
        hash_u64(&mut hasher, entry.length);
        hash_field(
            &mut hasher,
            entry.content_blake3.as_deref().unwrap_or("").as_bytes(),
        );
        hash_u64(&mut hasher, u64::from(entry.mode));
        hasher.update(&entry.mtime.seconds.to_le_bytes());
        hash_u64(&mut hasher, u64::from(entry.mtime.nanoseconds));
        hash_field(
            &mut hasher,
            entry.symlink_target_hex.as_deref().unwrap_or("").as_bytes(),
        );
    }
    hasher.finalize().to_hex().to_string()
}

fn hash_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hash_u64(hasher, value.len() as u64);
    hasher.update(value);
}

fn hash_u64(hasher: &mut blake3::Hasher, value: u64) {
    hasher.update(&value.to_le_bytes());
}

fn entry_difference(expected: &ManifestEntry, actual: &ManifestEntry) -> String {
    let mut fields = Vec::new();
    if expected.kind != actual.kind {
        fields.push("kind");
    }
    if expected.length != actual.length {
        fields.push("length");
    }
    if expected.content_blake3 != actual.content_blake3 {
        fields.push("content");
    }
    if expected.mode != actual.mode {
        fields.push("mode");
    }
    if expected.mtime != actual.mtime {
        fields.push("mtime");
    }
    if expected.symlink_target_hex != actual.symlink_target_hex {
        fields.push("symlink target");
    }
    format!("metadata/content mismatch ({})", fields.join(", "))
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(windows)]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

#[cfg(unix)]
fn platform_path_encoding() -> &'static str {
    "unix-bytes-hex-components"
}

#[cfg(windows)]
fn platform_path_encoding() -> &'static str {
    "windows-utf16le-hex-components"
}

#[cfg(unix)]
fn metadata_mode(metadata: &Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    metadata.mode() & 0o7777
}

#[cfg(windows)]
fn metadata_mode(metadata: &Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}

#[cfg(unix)]
fn metadata_mtime(metadata: &Metadata) -> io::Result<ManifestTime> {
    use std::os::unix::fs::MetadataExt;
    Ok(ManifestTime {
        seconds: metadata.mtime(),
        nanoseconds: u32::try_from(metadata.mtime_nsec()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "negative mtime nanoseconds")
        })?,
    })
}

#[cfg(windows)]
fn metadata_mtime(metadata: &Metadata) -> io::Result<ManifestTime> {
    use std::time::UNIX_EPOCH;
    let duration = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(ManifestTime {
        seconds: i64::try_from(duration.as_secs())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        nanoseconds: duration.subsec_nanos(),
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use filetime::{set_file_mtime, FileTime};
    use tempfile::tempdir;

    use super::*;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> tempfile::TempDir {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("nested/file"), b"content").unwrap();
        fs::write(root.path().join("empty"), []).unwrap();
        set_file_mtime(
            root.path().join("nested/file"),
            FileTime::from_unix_time(1_700_000_000, 123),
        )
        .unwrap();
        root
    }

    #[test]
    fn manifest_is_deterministic_and_content_pinned() {
        let root = fixture();
        let first = build_manifest(root.path()).unwrap();
        let second = build_manifest(root.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.schema, MANIFEST_SCHEMA);
        assert_eq!(first.logical_bytes, 7);
        assert_eq!(first.item_count, 4);

        fs::write(root.path().join("nested/file"), b"changed").unwrap();
        let changed = build_manifest(root.path()).unwrap();
        assert_ne!(first.manifest_digest, changed.manifest_digest);
    }

    #[test]
    fn verification_reports_content_mode_mtime_missing_and_extra() {
        let expected_root = fixture();
        let actual_root = fixture();
        let expected = build_manifest(expected_root.path()).unwrap();

        fs::write(actual_root.path().join("nested/file"), b"xxxxxxx").unwrap();
        set_file_mtime(
            actual_root.path().join("nested/file"),
            FileTime::from_unix_time(1_700_000_001, 0),
        )
        .unwrap();
        fs::remove_file(actual_root.path().join("empty")).unwrap();
        fs::write(actual_root.path().join("extra"), b"x").unwrap();
        #[cfg(unix)]
        fs::set_permissions(
            actual_root.path().join("nested/file"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        let result = verify_manifest(actual_root.path(), &expected).unwrap();
        assert!(!result.passed);
        assert!(result.mismatch_count >= 3);
        let reasons = result
            .mismatches
            .iter()
            .map(|mismatch| mismatch.reason.as_str())
            .collect::<BTreeSet<_>>();
        assert!(reasons.contains("missing destination entry"));
        assert!(reasons.contains("unexpected destination entry"));
        assert!(reasons.iter().any(|reason| reason.contains("content")));
    }

    #[test]
    fn invalid_expected_digest_is_rejected() {
        let root = fixture();
        let mut expected = build_manifest(root.path()).unwrap();
        expected.manifest_digest = "tampered".to_owned();
        assert!(matches!(
            verify_manifest(root.path(), &expected),
            Err(ManifestError::InvalidDigest { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn verification_detects_type_and_symlink_target_changes() {
        let expected_root = fixture();
        let actual_root = fixture();
        fs::write(expected_root.path().join("node"), b"file").unwrap();
        fs::write(actual_root.path().join("node"), b"file").unwrap();
        symlink("nested/file", expected_root.path().join("link")).unwrap();
        symlink("empty", actual_root.path().join("link")).unwrap();
        let expected = build_manifest(expected_root.path()).unwrap();

        fs::remove_file(actual_root.path().join("node")).unwrap();
        fs::create_dir(actual_root.path().join("node")).unwrap();
        let result = verify_manifest(actual_root.path(), &expected).unwrap();
        assert!(!result.passed);
        assert!(result
            .mismatches
            .iter()
            .any(|mismatch| mismatch.reason.contains("kind")));
        assert!(result
            .mismatches
            .iter()
            .any(|mismatch| mismatch.reason.contains("symlink target")));
    }

    #[cfg(unix)]
    #[test]
    fn raw_names_and_symlink_targets_have_reversible_encoding() {
        let raw_name = std::ffi::OsString::from_vec(vec![b'r', 0xff, b'w']);
        let raw_target = std::ffi::OsString::from_vec(vec![b't', 0xfe]);
        assert_eq!(hex(&os_bytes(&raw_name)), "72ff77");
        assert_eq!(hex(&os_bytes(&raw_target)), "74fe");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn raw_names_round_trip_through_a_supporting_filesystem() {
        let root = fixture();
        let raw_name = std::ffi::OsString::from_vec(vec![b'r', 0xff, b'w']);
        fs::write(root.path().join(&raw_name), b"raw").unwrap();
        let raw_target = std::ffi::OsString::from_vec(vec![b't', 0xfe]);
        symlink(&raw_target, root.path().join("link")).unwrap();
        let manifest = build_manifest(root.path()).unwrap();
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.path.components_hex.last() == Some(&"72ff77".to_owned())));
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.symlink_target_hex.as_deref() == Some("74fe")));
    }
}
