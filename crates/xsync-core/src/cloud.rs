//! Platform capability interface for cloud/dataless file placeholders.

use std::io;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

/// Whether this platform can identify cloud placeholders.
#[must_use]
pub const fn detection_available() -> bool {
    cfg!(target_os = "macos")
}

/// Return whether `path` is a provider placeholder.
///
/// Unsupported platforms deliberately return `Ok(false)` only for the
/// default download policy; callers must use [`detection_available`] before
/// applying skip/error policies.
///
/// # Errors
/// Returns an I/O error if the platform metadata query cannot be executed.
#[cfg(target_os = "macos")]
pub fn is_placeholder(path: &Path) -> io::Result<bool> {
    // File Provider marks evicted/dataless items with this xattr. The CLI is
    // used instead of an unsafe FFI call so the core crate retains its
    // no-unsafe-code contract; only stdout is discarded because presence is
    // sufficient and the value is provider-specific.
    let status = Command::new("/usr/bin/xattr")
        .args(["-p", "com.apple.fileprovider.fpfs#P"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

#[cfg(not(target_os = "macos"))]
///
/// # Errors
/// This implementation never returns an error.
pub fn is_placeholder(_path: &Path) -> io::Result<bool> {
    Ok(false)
}
