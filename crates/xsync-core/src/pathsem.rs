//! How a destination filesystem compares path names.
//!
//! Two source paths that are distinct on the source can be the *same* path on
//! the destination. APFS and NTFS are case-insensitive by default; APFS also
//! treats canonically-equivalent Unicode forms as one name. Publishing both
//! entries then keeps whichever wrote last, which is silent data loss: a
//! four-file Linux source pulled to macOS was measured landing as two files
//! with a success exit code, one of them holding another file's contents.
//!
//! These properties belong to the destination *volume*, not to the operating
//! system — a macOS volume can be formatted case-sensitive, and Linux can mount
//! NTFS — so they are probed rather than inferred.

use std::fs;
use std::path::Path;

use unicode_normalization::UnicodeNormalization;

use crate::path::WirePath;

/// Probe suffixes. A unique temporary prefix is added for every probe so we
/// never touch a user-owned, predictable basename.
const CASE_PROBE_UPPER: &str = "PathProbe";
const CASE_PROBE_LOWER: &str = "pathprobe";
/// "é" as one codepoint (U+00E9) and as "e" + U+0301. Canonically equivalent.
const NFC_PROBE: &str = "probe-\u{e9}";
const NFD_PROBE: &str = "probe-e\u{301}";

/// What a destination filesystem considers to be the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathSemantics {
    /// `Readme.md` and `readme.md` are one file.
    pub case_insensitive: bool,
    /// Canonically-equivalent Unicode forms are one file.
    pub normalization_insensitive: bool,
}

impl PathSemantics {
    /// Assume the destination distinguishes every distinct byte sequence.
    ///
    /// Used when a destination cannot be probed; it never reports a collision
    /// that would not occur, so it fails open rather than blocking a transfer.
    #[must_use]
    pub const fn sensitive() -> Self {
        Self {
            case_insensitive: false,
            normalization_insensitive: false,
        }
    }

    /// Whether any two distinct paths could collide here.
    #[must_use]
    pub const fn can_collide(self) -> bool {
        self.case_insensitive || self.normalization_insensitive
    }

    /// Probe a destination directory by creating names and observing them.
    ///
    /// Probe files are removed before returning. A directory that cannot be
    /// written is reported as [`Self::sensitive`]: the transfer will fail on its
    /// own merits, and guessing a stricter answer would reject valid work.
    #[must_use]
    pub fn probe(root: &Path) -> Self {
        // The destination often does not exist yet when a transfer is planned.
        // These are properties of the volume, so the nearest existing ancestor
        // answers the same question.
        let Some(existing) = nearest_existing_directory(root) else {
            return Self::sensitive();
        };
        Self {
            case_insensitive: probe_pair(&existing, CASE_PROBE_UPPER, CASE_PROBE_LOWER),
            normalization_insensitive: probe_pair(&existing, NFC_PROBE, NFD_PROBE),
        }
    }

    /// A key that is equal for any two paths this destination cannot tell apart.
    ///
    /// Non-UTF-8 paths are returned unchanged: they cannot be normalized, and a
    /// filesystem storing raw bytes compares them byte-wise.
    #[must_use]
    pub fn collision_key(self, path: &WirePath) -> Vec<u8> {
        let bytes = path.as_bytes();
        if !self.can_collide() {
            return bytes.to_vec();
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return bytes.to_vec();
        };
        // Normalization first: case folding a decomposed sequence must not
        // separate the base character from its combining marks.
        let normalized = if self.normalization_insensitive {
            text.nfc().collect::<String>()
        } else {
            text.to_owned()
        };
        let folded = if self.case_insensitive {
            normalized.to_lowercase()
        } else {
            normalized
        };
        folded.into_bytes()
    }
}

/// Walk up from `path` to the first component that exists and is a directory.
///
/// A destination is frequently created by the transfer itself, but its volume —
/// and therefore its naming behaviour — is fixed by an existing ancestor.
fn nearest_existing_directory(path: &Path) -> Option<std::path::PathBuf> {
    let mut candidate = path.to_path_buf();
    loop {
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !candidate.pop() {
            return None;
        }
        if candidate.as_os_str().is_empty() {
            return None;
        }
    }
}

/// Create `first`, then report whether `second` names the same file.
fn probe_pair(root: &Path, first: &str, second: &str) -> bool {
    let seed = tempfile::Builder::new()
        .prefix(".xsync-probe-")
        .tempfile_in(root)
        .ok();
    let Some(seed) = seed else {
        return false;
    };
    let Some(prefix) = seed.path().file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let prefix = prefix.to_owned();
    drop(seed);
    let first_path = root.join(format!("{prefix}{first}"));
    let second_path = root.join(format!("{prefix}{second}"));
    let first_created = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&first_path)
        .is_ok();
    if !first_created {
        return false;
    }
    let collides = fs::symlink_metadata(&second_path).is_ok();
    let _ = fs::remove_file(&first_path);
    // On a case/normalization-insensitive volume this is the same inode;
    // removing the first name is sufficient. Never unlink the second path:
    // another process may have created it after our metadata check.
    collides
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(path: &str) -> WirePath {
        WirePath::from_wire(path.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn a_sensitive_destination_never_folds() {
        let semantics = PathSemantics::sensitive();
        assert!(!semantics.can_collide());
        assert_ne!(
            semantics.collision_key(&wire("Readme.md")),
            semantics.collision_key(&wire("readme.md"))
        );
        assert_ne!(
            semantics.collision_key(&wire("caf\u{e9}.txt")),
            semantics.collision_key(&wire("cafe\u{301}.txt"))
        );
    }

    #[test]
    fn case_folding_collapses_only_case() {
        let semantics = PathSemantics {
            case_insensitive: true,
            normalization_insensitive: false,
        };
        assert_eq!(
            semantics.collision_key(&wire("Readme.md")),
            semantics.collision_key(&wire("readme.md"))
        );
        assert_ne!(
            semantics.collision_key(&wire("caf\u{e9}.txt")),
            semantics.collision_key(&wire("cafe\u{301}.txt"))
        );
        assert_ne!(
            semantics.collision_key(&wire("a.txt")),
            semantics.collision_key(&wire("b.txt"))
        );
    }

    #[test]
    fn normalization_folding_collapses_equivalent_forms() {
        let semantics = PathSemantics {
            case_insensitive: false,
            normalization_insensitive: true,
        };
        assert_eq!(
            semantics.collision_key(&wire("caf\u{e9}.txt")),
            semantics.collision_key(&wire("cafe\u{301}.txt"))
        );
        assert_ne!(
            semantics.collision_key(&wire("Readme.md")),
            semantics.collision_key(&wire("readme.md"))
        );
    }

    #[test]
    fn apfs_style_folds_both_together() {
        let semantics = PathSemantics {
            case_insensitive: true,
            normalization_insensitive: true,
        };
        // Differing in case *and* normalization at once, as APFS would fold.
        assert_eq!(
            semantics.collision_key(&wire("CAF\u{c9}.txt")),
            semantics.collision_key(&wire("cafe\u{301}.txt"))
        );
    }

    #[test]
    fn non_utf8_paths_are_compared_byte_wise() {
        let semantics = PathSemantics {
            case_insensitive: true,
            normalization_insensitive: true,
        };
        let raw = WirePath::from_wire(vec![b'n', b'a', b'm', b'e', 0xff]).unwrap();
        let other = WirePath::from_wire(vec![b'n', b'a', b'm', b'e', 0xfe]).unwrap();
        assert_eq!(
            semantics.collision_key(&raw),
            vec![b'n', b'a', b'm', b'e', 0xff]
        );
        assert_ne!(
            semantics.collision_key(&raw),
            semantics.collision_key(&other)
        );
    }

    #[test]
    fn probe_uses_the_nearest_existing_ancestor() {
        // A destination that does not exist yet must still be probed, via the
        // volume it will be created on.
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("not-created-yet").join("nor-this");
        assert!(!missing.exists());
        assert_eq!(
            PathSemantics::probe(&missing),
            PathSemantics::probe(temp.path()),
            "an absent destination must inherit its ancestor's semantics"
        );
    }

    #[test]
    fn probe_reports_the_real_destination_behaviour() {
        let temp = tempfile::tempdir().unwrap();
        let semantics = PathSemantics::probe(temp.path());
        // Whatever this host's temp volume does, probing must leave nothing behind.
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "probe left files behind: {leftovers:?}"
        );
        // On macOS the default volume folds both; on Linux/ext4 neither. Both are
        // valid, so assert only internal consistency.
        if semantics.case_insensitive || semantics.normalization_insensitive {
            assert!(semantics.can_collide());
        }
    }

    #[test]
    fn probe_does_not_touch_predictable_user_files() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join(".xsync.tmp.PathProbe");
        fs::write(&sentinel, b"user data").unwrap();

        let _ = PathSemantics::probe(temp.path());

        assert_eq!(fs::read(sentinel).unwrap(), b"user data");
    }
}
