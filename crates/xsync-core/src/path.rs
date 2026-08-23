//! rsync-compatible path specification parsing.
//!
//! An argument is either a local path or, if it matches `[user@]host:path`,
//! a remote (SSH) location. Windows drive letters like `C:\foo` or `C:/foo`
//! are treated as local paths, never as remote hosts. Trailing-slash semantics
//! are captured: `path/` sends the directory's *contents*, `path` sends the
//! directory itself.

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

    let trailing_slash = spec.ends_with('/');
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
