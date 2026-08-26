//! rsync-compatible path specification parsing.
//!
//! An argument is either a local path or, if it matches `[user@]host:path`,
//! a remote (SSH) location. Windows drive letters like `C:\foo` or `C:/foo`
//! are treated as local paths, never as remote hosts. Trailing-slash semantics
//! are captured: `path/` sends the directory's *contents*, `path` sends the
//! directory itself.

use std::path::{Path, PathBuf};

/// A validated, protocol-relative path represented by raw component bytes.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WirePath(Vec<u8>);

impl std::fmt::Debug for WirePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("WirePath").field(&self.0).finish()
    }
}

impl std::fmt::Display for WirePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl WirePath {
    /// Construct a validated relative path from slash-separated wire bytes.
    ///
    /// # Errors
    /// Returns an error when the bytes contain NUL, traversal, or rooted path
    /// components.
    pub fn from_wire(bytes: Vec<u8>) -> Result<Self, WirePathError> {
        if bytes.contains(&0) {
            return Err(WirePathError::Nul);
        }
        if bytes.is_empty() {
            return Ok(Self(bytes));
        }
        if bytes[0] == b'/'
            || bytes
                .split(|byte| *byte == b'/')
                .any(|component| component.is_empty() || component == b"." || component == b"..")
        {
            return Err(WirePathError::Unsafe);
        }
        Ok(Self(bytes))
    }

    /// Construct a path from a native relative path.
    ///
    /// # Errors
    /// Returns an error when the native path is rooted or contains a non-normal
    /// component or unsupported platform encoding.
    pub fn from_native_relative(path: &Path) -> Result<Self, WirePathError> {
        let mut bytes = Vec::new();
        for component in path.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(WirePathError::Unsafe);
            };
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                if !bytes.is_empty() {
                    bytes.push(b'/');
                }
                bytes.extend_from_slice(component.as_bytes());
            }
            #[cfg(windows)]
            {
                let text = component.to_str().ok_or(WirePathError::Encoding)?;
                if !bytes.is_empty() {
                    bytes.push(b'/');
                }
                bytes.extend_from_slice(text.as_bytes());
            }
        }
        Self::from_wire(bytes)
    }

    /// Raw slash-separated wire bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Number of encoded path bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this is the empty transfer-root path.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert this relative path to a native path below `root`.
    #[must_use]
    pub fn to_native_path(&self, root: &Path) -> PathBuf {
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            return root.join(OsString::from_vec(self.0.clone()));
        }
        #[cfg(windows)]
        {
            return root.join(String::from_utf8_lossy(&self.0).as_ref());
        }
        #[allow(unreachable_code)]
        root.to_path_buf()
    }

    /// Number of components in the path.
    #[must_use]
    #[allow(clippy::naive_bytecount)]
    pub fn depth(&self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            self.0.iter().filter(|byte| **byte == b'/').count() + 1
        }
    }

    /// Add a complete component prefix.
    #[must_use]
    pub fn with_prefix(&self, prefix: &Self) -> Self {
        if prefix.0.is_empty() {
            return self.clone();
        }
        if self.0.is_empty() {
            return prefix.clone();
        }
        let mut bytes = prefix.0.clone();
        bytes.push(b'/');
        bytes.extend_from_slice(&self.0);
        Self(bytes)
    }

    /// Remove a complete component prefix.
    #[must_use]
    pub fn strip_prefix<P: WirePathPattern>(&self, prefix: P) -> Option<Self> {
        let prefix = prefix.wire_bytes();
        if prefix.is_empty() {
            return Some(self.clone());
        }
        let suffix = self.0.strip_prefix(prefix)?.strip_prefix(b"/")?;
        Some(Self(suffix.to_vec()))
    }

    /// Whether `prefix` is a complete component prefix.
    #[must_use]
    pub fn starts_with<P: WirePathPattern>(&self, prefix: P) -> bool {
        let prefix = prefix.wire_bytes();
        self == &Self(prefix.to_vec()) || self.strip_prefix(prefix).is_some()
    }

    /// Consume the path into its wire bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Byte source accepted by [`WirePath::starts_with`] and [`WirePath::strip_prefix`].
pub trait WirePathPattern {
    /// Return the slash-separated bytes to match.
    fn wire_bytes(&self) -> &[u8];
}

impl WirePathPattern for WirePath {
    fn wire_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl WirePathPattern for str {
    fn wire_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl WirePathPattern for String {
    fn wire_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl WirePathPattern for [u8] {
    fn wire_bytes(&self) -> &[u8] {
        self
    }
}

impl<T: WirePathPattern + ?Sized> WirePathPattern for &T {
    fn wire_bytes(&self) -> &[u8] {
        (*self).wire_bytes()
    }
}

/// Why a native path cannot be represented as a safe relative wire path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WirePathError {
    /// The path contains a NUL byte.
    #[error("path contains NUL")]
    Nul,
    /// The path is absolute or contains traversal components.
    #[error("path is absolute or contains an unsafe component")]
    Unsafe,
    /// The platform path cannot be represented by the documented encoding.
    #[error("path cannot be represented by the platform wire encoding")]
    Encoding,
}

impl From<&str> for WirePath {
    fn from(value: &str) -> Self {
        // Callers that cross a filesystem or protocol boundary validate with
        // `from_wire`; this conversion also supports hostile-path rejection tests.
        Self(value.as_bytes().to_vec())
    }
}

impl From<&WirePath> for WirePath {
    fn from(value: &WirePath) -> Self {
        value.clone()
    }
}

impl PartialEq<str> for WirePath {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for WirePath {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl PartialEq<String> for WirePath {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.as_bytes()
    }
}

#[cfg(unix)]
impl AsRef<std::ffi::OsStr> for WirePath {
    fn as_ref(&self) -> &std::ffi::OsStr {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(&self.0)
    }
}

#[cfg(unix)]
impl AsRef<Path> for WirePath {
    fn as_ref(&self) -> &Path {
        Path::new(<Self as AsRef<std::ffi::OsStr>>::as_ref(self))
    }
}

/// Whether a path specification refers to a local or a remote (SSH) location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A path on the local machine.
    Local,
    /// A path reachable over SSH, at `host` (optionally as `user`).
    Remote { user: Option<String>, host: String },
}

/// A parsed source or destination argument with rsync conventions applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSpec {
    location: Location,
    /// The path portion: the local path, or the remote side's path.
    pub path: String,
    /// True when the original ended in `/` (send the directory's contents).
    pub trailing_slash: bool,
}

impl PathSpec {
    /// Whether this spec points at a remote (SSH) location.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.location, Location::Remote { .. })
    }

    /// The host this spec addresses, if remote.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        match &self.location {
            Location::Remote { host, .. } => Some(host),
            Location::Local => None,
        }
    }

    /// SSH authority including an optional user (`user@host`).
    #[must_use]
    pub fn authority(&self) -> Option<String> {
        match &self.location {
            Location::Remote { user, host } => Some(
                user.as_ref()
                    .map_or_else(|| host.clone(), |user| format!("{user}@{host}")),
            ),
            Location::Local => None,
        }
    }
}

/// Errors produced when parsing or validating path specifications.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    /// The argument was empty.
    #[error("empty path")]
    Empty,
    /// A remote spec had no host before the `:`.
    #[error("missing host before ':'")]
    EmptyHost,
    /// A remote spec had no path after the `:`.
    #[error("remote path must be non-empty (e.g. 'host:dir')")]
    EmptyRemotePath,
    /// Both the source and destination are remote; not supported in v1.
    #[error("remote-to-remote sync is not supported in v1")]
    RemoteToRemote,
}

/// Parse an rsync-style path argument, applying trailing-slash and
/// remote-vs-local (with Windows drive-letter) rules.
///
/// # Errors
///
/// Returns [`PathError::Empty`] for an empty argument,
/// [`PathError::EmptyRemotePath`] when a remote spec has no path after `:`,
/// or [`PathError::EmptyHost`] when the part before `:` is empty.
pub fn parse(spec: &str) -> Result<PathSpec, PathError> {
    if spec.is_empty() {
        return Err(PathError::Empty);
    }

    // A local path on Windows may end with either separator. A remote path is
    // interpreted by the remote shell, where '\\' is a legal filename byte, so
    // only '/' terminates a remote spec regardless of the local platform.
    let is_remote_spec = find_remote_colon(spec).is_some();
    let trailing_slash =
        spec.ends_with('/') || (cfg!(windows) && !is_remote_spec && spec.ends_with('\\'));
    let spec = if trailing_slash {
        &spec[..spec.len() - 1]
    } else {
        spec
    };

    if let Some(colon) = find_remote_colon(spec) {
        let (user_host, raw_path) = spec.split_at(colon);
        let raw_path = &raw_path[1..]; // strip the ':'
        if raw_path.is_empty() {
            return Err(PathError::EmptyRemotePath);
        }
        let (user, host) = match user_host.rsplit_once('@') {
            Some((u, h)) => (Some(u.to_string()), h.to_string()),
            None => (None, user_host.to_string()),
        };
        if host.is_empty() {
            return Err(PathError::EmptyHost);
        }
        Ok(PathSpec {
            location: Location::Remote { user, host },
            path: raw_path.to_string(),
            trailing_slash,
        })
    } else {
        Ok(PathSpec {
            location: Location::Local,
            path: spec.to_string(),
            trailing_slash,
        })
    }
}

/// Validate that a `(src, dest)` pair is supported in v1. The only forbidden
/// combination is remote-to-remote.
///
/// # Errors
///
/// Returns [`PathError::RemoteToRemote`] when both specs are remote.
pub fn validate_pair(src: &PathSpec, dest: &PathSpec) -> Result<(), PathError> {
    if src.is_remote() && dest.is_remote() {
        return Err(PathError::RemoteToRemote);
    }
    Ok(())
}

/// Locate the `:` that separates `[user@]host` from `path`, or `None` when
/// the colon is a Windows drive letter (e.g. `C:\foo`, `C:/foo`).
fn find_remote_colon(spec: &str) -> Option<usize> {
    let idx = spec.find(':')?;
    let before_is_drive_letter = idx == 1 && spec.as_bytes()[0].is_ascii_alphabetic();
    let after_is_slash = spec[idx + 1..].starts_with(['/', '\\']);
    if before_is_drive_letter && after_is_slash {
        return None;
    }
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_plain_dir() {
        let p = parse("dir").unwrap();
        assert_eq!(p.location, Location::Local);
        assert_eq!(p.path, "dir");
        assert!(!p.trailing_slash);
    }

    #[cfg(windows)]
    #[test]
    fn windows_local_trailing_backslash_is_a_trailing_separator() {
        // rsync semantics: a trailing separator means "copy the contents of the
        // directory", not the directory itself. On Windows '\\' is the native
        // separator and must behave exactly like '/'.
        let back = parse(r"C:\data\").unwrap();
        assert!(!back.is_remote());
        assert!(back.trailing_slash);
        assert_eq!(back.path, r"C:\data");

        let forward = parse("C:/data/").unwrap();
        assert!(forward.trailing_slash);
        assert_eq!(forward.path, "C:/data");

        // No trailing separator keeps directory-itself semantics.
        let bare = parse(r"C:\data").unwrap();
        assert!(!bare.trailing_slash);
        assert_eq!(bare.path, r"C:\data");
    }

    #[test]
    fn remote_trailing_backslash_is_not_a_separator() {
        // The remote shell interprets this path; '\\' is a legal filename byte
        // there, so it must survive as part of the path on every platform.
        let remote = parse(r"host:/data\").unwrap();
        assert!(remote.is_remote());
        assert!(!remote.trailing_slash);
        assert_eq!(remote.path, r"/data\");
    }

    #[test]
    fn local_dir_trailing_slash() {
        let p = parse("dir/").unwrap();
        assert_eq!(p.location, Location::Local);
        assert_eq!(p.path, "dir");
        assert!(p.trailing_slash);
    }

    #[test]
    fn remote_host_dir() {
        let p = parse("host:dir").unwrap();
        assert_eq!(
            p.location,
            Location::Remote {
                user: None,
                host: "host".into()
            }
        );
        assert_eq!(p.path, "dir");
        assert!(!p.trailing_slash);
        assert!(p.is_remote());
        assert_eq!(p.host(), Some("host"));
    }

    #[test]
    fn remote_user_host_dir_trailing_slash() {
        let p = parse("user@host:dir/").unwrap();
        assert_eq!(
            p.location,
            Location::Remote {
                user: Some("user".into()),
                host: "host".into()
            }
        );
        assert_eq!(p.path, "dir");
        assert!(p.trailing_slash);
    }

    #[test]
    fn windows_drive_letters_are_local() {
        let backslash = parse(r"C:\Users\x").unwrap();
        assert_eq!(backslash.location, Location::Local);
        assert_eq!(backslash.path, r"C:\Users\x");

        let forward = parse("C:/Users/x").unwrap();
        assert_eq!(forward.location, Location::Local);
        assert_eq!(forward.path, "C:/Users/x");
    }

    #[test]
    fn windows_long_drive_paths_are_not_truncated() {
        let path = format!("C:\\{}", "x".repeat(300));
        let parsed = parse(&path).unwrap();
        assert!(!parsed.is_remote());
        assert_eq!(parsed.path, path);
    }

    #[test]
    fn relative_and_single_file_are_local() {
        let rel = parse("./relative").unwrap();
        assert_eq!(rel.location, Location::Local);
        assert_eq!(rel.path, "./relative");

        let single = parse("Makefile").unwrap();
        assert_eq!(single.location, Location::Local);
        assert_eq!(single.path, "Makefile");
    }

    #[test]
    fn empty_path_is_an_error() {
        assert_eq!(parse(""), Err(PathError::Empty));
    }

    #[test]
    fn empty_remote_path_is_an_error() {
        assert_eq!(parse("host:"), Err(PathError::EmptyRemotePath));
    }

    #[test]
    fn wire_paths_reject_hostile_components() {
        for bytes in [
            b"/absolute".as_slice(),
            b"a//b",
            b"a/./b",
            b"a/../b",
            b"a\0b",
        ] {
            assert!(WirePath::from_wire(bytes.to_vec()).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn wire_paths_preserve_unix_bytes_and_prefixes() {
        let raw = WirePath::from_wire(b"dir/bad-\xff".to_vec()).unwrap();
        assert_eq!(raw.as_bytes(), b"dir/bad-\xff");
        assert_eq!(raw.strip_prefix("dir").unwrap().as_bytes(), b"bad-\xff");
        assert!(raw.starts_with("dir"));
    }

    #[test]
    fn trailing_slash_without_remote_split_point_is_local() {
        // A bare `host:/` still has content before the colon; host is remote,
        // but the path is empty -> error.
        assert_eq!(parse("host:/"), Err(PathError::EmptyRemotePath));
    }

    #[test]
    fn validate_pair_rejects_remote_to_remote() {
        let r1 = parse("h1:a").unwrap();
        let r2 = parse("h2:b").unwrap();
        assert_eq!(validate_pair(&r1, &r2), Err(PathError::RemoteToRemote));

        // All other combinations are fine.
        let l = parse("dest").unwrap();
        assert!(validate_pair(&l, &r1).is_ok());
        assert!(validate_pair(&r1, &l).is_ok());
        assert!(validate_pair(&l, &l).is_ok());
    }
}
