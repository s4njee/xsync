//! Remote bootstrap: place a verified xsync binary on a host that lacks one.
//!
//! Story D5.2. rsync's real advantage is being installed everywhere already; a
//! tool that requires itself on both ends has a distribution problem no amount
//! of packaging solves. This module closes that gap for the case where the
//! operator can reach the host over SSH but cannot install software on it.
//!
//! It is opt-in. Uploading an executable to a machine and running it is exactly
//! the shape of a supply-chain attack, so it never happens implicitly: the
//! caller must ask, the binary is checksummed on the remote before it is
//! executed, nothing is installed system-wide, and nothing requires root.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::server::{base_remote_invocation, quote_remote_arg, RemoteShell};

/// What to do about a remote that has no xsync binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BootstrapPolicy {
    /// Never upload. The remote must already have xsync.
    #[default]
    Disabled,
    /// Upload for this session and remove it afterwards.
    Ephemeral,
    /// Upload and leave it in place for later runs.
    Persist,
}

impl BootstrapPolicy {
    /// Whether an upload may happen at all.
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Failures specific to provisioning a remote binary.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    /// The platform probe could not be run or its output was unusable.
    #[error("cannot determine the remote platform: {0}")]
    Probe(String),
    /// The remote reported a platform xsync does not build for.
    #[error("{0}")]
    UnsupportedPlatform(String),
    /// No local binary matches the remote's platform.
    #[error("{0}")]
    NoMatchingBinary(String),
    /// The upload itself failed.
    #[error("cannot upload the xsync binary to the remote: {0}")]
    Upload(String),
    /// The uploaded bytes did not match what was sent.
    #[error("uploaded binary failed verification on the remote: {0}")]
    Verify(String),
}

/// Operating-system family, as far as choosing a binary goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOs {
    /// Any Linux distribution.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

/// CPU architecture of the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteArch {
    /// 64-bit x86.
    X86_64,
    /// 64-bit ARM.
    Aarch64,
}

/// C runtime in use, which only distinguishes Linux builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLibc {
    /// GNU libc.
    Gnu,
    /// musl libc.
    Musl,
}

/// Enough of the remote's identity to pick a binary for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemotePlatform {
    /// Operating-system family.
    pub os: RemoteOs,
    /// CPU architecture.
    pub arch: RemoteArch,
    /// C runtime, meaningful on Linux only.
    pub libc: Option<RemoteLibc>,
}

impl RemotePlatform {
    /// The Rust target triple xsync must be built for to run here.
    #[must_use]
    pub const fn target_triple(self) -> &'static str {
        match (self.os, self.arch, self.libc) {
            (RemoteOs::Linux, RemoteArch::X86_64, Some(RemoteLibc::Musl)) => {
                "x86_64-unknown-linux-musl"
            }
            (RemoteOs::Linux, RemoteArch::Aarch64, Some(RemoteLibc::Musl)) => {
                "aarch64-unknown-linux-musl"
            }
            (RemoteOs::Linux, RemoteArch::X86_64, _) => "x86_64-unknown-linux-gnu",
            (RemoteOs::Linux, RemoteArch::Aarch64, _) => "aarch64-unknown-linux-gnu",
            (RemoteOs::MacOs, RemoteArch::X86_64, _) => "x86_64-apple-darwin",
            (RemoteOs::MacOs, RemoteArch::Aarch64, _) => "aarch64-apple-darwin",
            (RemoteOs::Windows, RemoteArch::X86_64, _) => "x86_64-pc-windows-msvc",
            (RemoteOs::Windows, RemoteArch::Aarch64, _) => "aarch64-pc-windows-msvc",
        }
    }
}

/// The triple this binary was built for.
///
/// Derived from `cfg!` rather than a build script so it cannot drift from the
/// binary actually running.
#[must_use]
pub fn host_target_triple() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc"
        } else {
            "x86_64-pc-windows-msvc"
        }
    } else if cfg!(target_env = "musl") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-musl"
        } else {
            "x86_64-unknown-linux-musl"
        }
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

/// One command string that identifies the remote under either shell family.
///
/// `uname` is absent on Windows and `%OS%` does not expand on POSIX, so running
/// both and keeping whichever produced output avoids needing to know the family
/// first. `ldd --version` separates glibc from musl and is allowed to fail on
/// macOS, which has no `ldd`.
fn probe_command(shell: RemoteShell) -> String {
    match shell {
        // Home comes third, before the variable-length libc line, so the
        // fixed fields can be read positionally.
        RemoteShell::Posix => {
            "uname -s; uname -m; printf '%s\\n' \"$HOME\"; (ldd --version 2>&1 | head -n 1) || true"
                .to_owned()
        }
        RemoteShell::Windows => {
            "echo %OS%& echo %PROCESSOR_ARCHITECTURE%& echo %USERPROFILE%".to_owned()
        }
    }
}

/// The remote's home directory, as reported by the probe.
///
/// Needed as an absolute path because the upload goes over `scp`, which does no
/// shell expansion: `$HOME` and `%USERPROFILE%` arrive as literal text.
#[must_use]
pub fn parse_home(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(2)
        .map(str::to_owned)
        .filter(|home| !home.is_empty())
}

/// Ask the remote what it is.
///
/// # Errors
///
/// Returns [`BootstrapError::Probe`] if the probe cannot run, and
/// [`BootstrapError::UnsupportedPlatform`] for a platform outside the target
/// matrix — refusing is the point, since shipping a binary that cannot execute
/// is worse than saying so.
pub fn detect_remote_platform(
    rsh: Option<&str>,
    host: &str,
    shell: RemoteShell,
) -> Result<(RemotePlatform, String), BootstrapError> {
    let (program, args) = base_remote_invocation(rsh, host, &probe_command(shell));
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| BootstrapError::Probe(error.to_string()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let platform = parse_platform(&text)?;
    let home = parse_home(&text).ok_or_else(|| {
        BootstrapError::Probe(format!(
            "the remote did not report a home directory: {}",
            text.trim()
        ))
    })?;
    Ok((platform, home))
}

/// Interpret probe output. Split out so the mapping is testable without SSH.
///
/// # Errors
///
/// Returns [`BootstrapError::UnsupportedPlatform`] when the OS or architecture
/// is outside the supported matrix, naming what was reported.
pub fn parse_platform(text: &str) -> Result<RemotePlatform, BootstrapError> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let first = lines.first().copied().unwrap_or_default();
    let second = lines.get(1).copied().unwrap_or_default();
    // Index 2 is the home directory; the libc banner follows it.
    let rest = lines.get(3..).unwrap_or_default().join(" ").to_lowercase();

    let os = if first.eq_ignore_ascii_case("Windows_NT") {
        RemoteOs::Windows
    } else if first.eq_ignore_ascii_case("Darwin") {
        RemoteOs::MacOs
    } else if first.eq_ignore_ascii_case("Linux") {
        RemoteOs::Linux
    } else {
        return Err(BootstrapError::UnsupportedPlatform(format!(
            "remote reports operating system {first:?}, which xsync does not build for; \
             supported: Linux, Darwin, Windows_NT"
        )));
    };

    let machine = second.to_lowercase();
    let arch = match machine.as_str() {
        "x86_64" | "amd64" => RemoteArch::X86_64,
        "aarch64" | "arm64" => RemoteArch::Aarch64,
        other => {
            return Err(BootstrapError::UnsupportedPlatform(format!(
                "remote reports architecture {other:?}; xsync builds only for x86_64 and \
                 aarch64 (see docs/TARGET-MATRIX.md)"
            )))
        }
    };

    // Only Linux needs the distinction, and only the musl case is positively
    // identifiable: `ldd --version` names musl, while glibc names itself in a
    // dozen locale-dependent ways. Default to gnu and let verification catch a
    // wrong guess rather than guessing musl.
    let libc = (os == RemoteOs::Linux).then(|| {
        if rest.contains("musl") {
            RemoteLibc::Musl
        } else {
            RemoteLibc::Gnu
        }
    });

    Ok(RemotePlatform { os, arch, libc })
}

/// Directories searched for a binary matching `triple`.
fn search_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("XSYNC_BOOTSTRAP_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join(".cache/xsync/binaries"));
    }
    dirs
}

/// Find a local binary that will run on `platform`.
///
/// The running executable is used when its own triple matches, which covers the
/// common same-platform case without any staging. Otherwise a binary must have
/// been placed under one of the search directories, because xsync has no
/// release channel to fetch one from yet (D2.2).
///
/// # Errors
///
/// Returns [`BootstrapError::NoMatchingBinary`] naming the required triple and
/// every path searched. Refusing here is deliberate: uploading a binary for the
/// wrong architecture or libc produces a confusing failure on the remote
/// instead of a clear one locally.
pub fn locate_binary(platform: RemotePlatform) -> Result<PathBuf, BootstrapError> {
    let triple = platform.target_triple();
    let file = if platform.os == RemoteOs::Windows {
        "xs.exe"
    } else {
        "xs"
    };

    if triple == host_target_triple() {
        if let Ok(exe) = std::env::current_exe() {
            if exe.is_file() {
                return Ok(exe);
            }
        }
    }

    let mut searched = Vec::new();
    for dir in search_directories() {
        let candidate = dir.join(triple).join(file);
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate.display().to_string());
    }

    Err(BootstrapError::NoMatchingBinary(format!(
        "no xsync binary for the remote's platform ({triple}). This machine runs {}, so its \
         own binary cannot be used. Place one at one of: {}; or set XSYNC_BOOTSTRAP_DIR to a \
         directory laid out as <triple>/{file}.",
        host_target_triple(),
        if searched.is_empty() {
            "<no search directory: set XSYNC_BOOTSTRAP_DIR or HOME>".to_owned()
        } else {
            searched.join(", ")
        }
    )))
}

/// Where the uploaded binary lives on the remote, per shell family.
///
/// Always under the invoking user's own directory: bootstrap never installs
/// system-wide and never needs root.
#[must_use]
pub fn remote_binary_path(
    shell: RemoteShell,
    home: &str,
    tag: &str,
    policy: BootstrapPolicy,
) -> String {
    // Forward slashes work throughout the Windows API and avoid a second layer
    // of backslash escaping through cmd and scp.
    let home = home.trim_end_matches(['/', '\\']).replace('\\', "/");
    let extension = if shell == RemoteShell::Windows {
        ".exe"
    } else {
        ""
    };
    if policy == BootstrapPolicy::Persist {
        // `.local/bin` is exactly the directory the remote command already
        // prepends to PATH, so a persisted binary is found as a plain `xs` by
        // later runs -- including runs from a different machine, which have no
        // memory of where this one put it. A digest-tagged path would leave the
        // file in place while remaining undiscoverable, which is worse than not
        // persisting at all.
        return format!("{home}/.local/bin/xs{extension}");
    }
    // Ephemeral uploads are tagged by digest so concurrent and repeat runs
    // converge on one file rather than racing over a single name.
    format!("{home}/.cache/xsync/xs-{tag}{extension}")
}

/// SHA-256 of a local file, as lowercase hex.
///
/// SHA-256 rather than the BLAKE3 used on the wire, because every supported
/// remote can compute it with a preinstalled tool — `sha256sum`, `shasum`, or
/// `certutil` — and the remote has no xsync yet to compute anything else.
///
/// # Errors
///
/// Returns [`BootstrapError::Upload`] if the local file cannot be read.
pub fn file_sha256(path: &std::path::Path) -> Result<String, BootstrapError> {
    let mut file =
        std::fs::File::open(path).map_err(|error| BootstrapError::Upload(error.to_string()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|error| BootstrapError::Upload(error.to_string()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Shell command creating the parent directory of `path`.
fn mkdir_command(shell: RemoteShell, path: &str) -> String {
    let parent = path.rsplit_once('/').map_or(path, |(head, _)| head);
    match shell {
        RemoteShell::Posix => format!("mkdir -p {}", quote_remote_arg(parent)),
        // `md` fails when the directory exists, which is not an error here.
        RemoteShell::Windows => {
            format!("if not exist \"{0}\" md \"{0}\"", parent.replace('/', "\\"))
        }
    }
}

/// Shell command that prints the SHA-256 of `path`.
fn hash_command(shell: RemoteShell, path: &str) -> String {
    match shell {
        // sha256sum on Linux, shasum on macOS; try both.
        RemoteShell::Posix => format!(
            "sha256sum {0} 2>/dev/null || shasum -a 256 {0}",
            quote_remote_arg(path)
        ),
        RemoteShell::Windows => {
            format!("certutil -hashfile \"{}\" SHA256", path.replace('/', "\\"))
        }
    }
}

/// Shell command making `path` executable. Only meaningful on POSIX.
fn chmod_command(shell: RemoteShell, path: &str) -> Option<String> {
    match shell {
        RemoteShell::Posix => Some(format!("chmod 700 {}", quote_remote_arg(path))),
        RemoteShell::Windows => None,
    }
}

/// Shell command that removes `path`, ignoring a missing file.
fn remove_command(shell: RemoteShell, path: &str) -> String {
    match shell {
        RemoteShell::Posix => format!("rm -f {}", quote_remote_arg(path)),
        RemoteShell::Windows => format!("del /q \"{}\" 2>nul", path.replace('/', "\\")),
    }
}

/// Run one command on the remote and return its captured output.
fn run_remote(
    rsh: Option<&str>,
    host: &str,
    command: &str,
) -> Result<std::process::Output, BootstrapError> {
    let (program, args) = base_remote_invocation(rsh, host, command);
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| BootstrapError::Upload(error.to_string()))
}

/// Pull the first 64-character hex run out of a hashing tool's output.
///
/// `sha256sum` prints `<hash>  <path>`, `shasum` the same, and `certutil`
/// prints a banner then the digest. Scanning for the hex run handles all three
/// without parsing each format.
fn extract_sha256(output: &str) -> Option<String> {
    for token in output.split_whitespace() {
        if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    // Some certutil versions space the digest into byte pairs.
    let joined: String = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
        .map(|line| line.replace(' ', ""))
        .find(|line| line.len() == 64)?;
    Some(joined.to_ascii_lowercase())
}

/// Copy `binary` to `remote_path` with `scp`.
///
/// Binary stdin to a Windows remote is not dependable: measured against
/// OpenSSH-for-Windows, a 1 MB payload piped to the login shell arrived
/// truncated at varying lengths, while 50 KB and 200 KB arrived intact. `scp`
/// speaks the sftp protocol and transferred 3 MB byte-identical, so the file
/// copy uses the tool built for it rather than a shell pipeline.
fn scp_upload(
    binary: &std::path::Path,
    host: &str,
    remote_path: &str,
) -> Result<(), BootstrapError> {
    let output = Command::new("scp")
        .arg("-q")
        .arg(binary)
        .arg(format!("{host}:{remote_path}"))
        .stdin(Stdio::null())
        .output()
        .map_err(|error| BootstrapError::Upload(format!("cannot run scp: {error}")))?;
    if !output.status.success() {
        return Err(BootstrapError::Upload(format!(
            "scp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Upload `binary` to the remote, verify its checksum there, and return the
/// remote path to execute.
///
/// The order is deliberate: the file is placed, then hashed *on the remote*,
/// and only a matching digest allows it to be executed. A mismatch removes the
/// file rather than leaving an unverified executable behind.
///
/// # Errors
///
/// Returns [`BootstrapError::Upload`] if the transfer fails and
/// [`BootstrapError::Verify`] if the remote's digest does not match what was
/// sent.
pub fn upload_and_verify(
    rsh: Option<&str>,
    host: &str,
    shell: RemoteShell,
    home: &str,
    binary: &std::path::Path,
    policy: BootstrapPolicy,
) -> Result<String, BootstrapError> {
    // scp cannot be derived from an arbitrary `-e` command, and binary stdin to
    // Windows is unreliable, so say so rather than transfer a truncated
    // executable and let it fail confusingly on the remote.
    if rsh.is_some() && shell == RemoteShell::Windows {
        return Err(BootstrapError::Upload(
            "bootstrapping a Windows remote requires the default ssh transport, because the              upload uses scp; re-run without -e/--rsh, or install xsync on the remote"
                .to_owned(),
        ));
    }

    let expected = file_sha256(binary)?;
    // Tag the path with the digest so repeat runs converge on one file rather
    // than racing over a single name.
    let remote_path = remote_binary_path(shell, home, &expected[..16], policy);

    let output = run_remote(rsh, host, &mkdir_command(shell, &remote_path))?;
    if !output.status.success() {
        return Err(BootstrapError::Upload(format!(
            "cannot create the remote directory: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    scp_upload(binary, host, &remote_path)?;

    let output = run_remote(rsh, host, &hash_command(shell, &remote_path))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(actual) = extract_sha256(&text) else {
        return Err(BootstrapError::Verify(format!(
            "no SHA-256 in the remote's response: {}",
            text.trim()
        )));
    };
    if actual != expected {
        let _ = remove_remote(rsh, host, shell, &remote_path);
        return Err(BootstrapError::Verify(format!(
            "expected {expected}, remote computed {actual}; the upload was altered in transit              and has been removed"
        )));
    }

    if let Some(command) = chmod_command(shell, &remote_path) {
        let output = run_remote(rsh, host, &command)?;
        if !output.status.success() {
            return Err(BootstrapError::Upload(format!(
                "cannot make the uploaded binary executable: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }

    Ok(remote_path)
}

/// Delete a previously uploaded binary. Best-effort; a failure is reported but
/// is not fatal to a transfer that already succeeded.
///
/// # Errors
///
/// Returns [`BootstrapError::Upload`] if the removal command cannot be run.
pub fn remove_remote(
    rsh: Option<&str>,
    host: &str,
    shell: RemoteShell,
    remote_path: &str,
) -> Result<(), BootstrapError> {
    let (program, args) = base_remote_invocation(rsh, host, &remove_command(shell, remote_path));
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| BootstrapError::Upload(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_glibc_and_musl_distinctly() {
        let gnu = parse_platform("Linux\nx86_64\n/home/u\nldd (GNU libc) 2.39\n").unwrap();
        assert_eq!(gnu.target_triple(), "x86_64-unknown-linux-gnu");

        // A musl host must not be handed a glibc binary: it would fail to
        // execute with an error that says nothing useful about why.
        let musl = parse_platform("Linux\naarch64\n/home/u\nmusl libc (aarch64)\n").unwrap();
        assert_eq!(musl.target_triple(), "aarch64-unknown-linux-musl");
    }

    #[test]
    fn parses_macos_and_windows() {
        assert_eq!(
            parse_platform("Darwin\narm64\n/Users/u\n")
                .unwrap()
                .target_triple(),
            "aarch64-apple-darwin"
        );
        // cmd.exe reports AMD64/ARM64 rather than uname's spellings.
        assert_eq!(
            parse_platform("Windows_NT\nAMD64\nC:\\Users\\u\n")
                .unwrap()
                .target_triple(),
            "x86_64-pc-windows-msvc"
        );
        assert_eq!(
            parse_platform("Windows_NT\nARM64\nC:\\Users\\u\n")
                .unwrap()
                .target_triple(),
            "aarch64-pc-windows-msvc"
        );
    }

    #[test]
    fn refuses_platforms_outside_the_target_matrix() {
        // Shipping something that cannot run is worse than refusing, so both
        // the OS and the architecture are checked, and the message names what
        // the remote actually reported.
        let os = parse_platform("FreeBSD\nx86_64\n/home/u\n").unwrap_err();
        assert!(matches!(os, BootstrapError::UnsupportedPlatform(_)));
        assert!(os.to_string().contains("FreeBSD"), "{os}");

        let arch = parse_platform("Linux\narmv7l\n/home/u\n").unwrap_err();
        assert!(matches!(arch, BootstrapError::UnsupportedPlatform(_)));
        assert!(arch.to_string().contains("armv7l"), "{arch}");
    }

    #[test]
    fn extracts_a_digest_from_each_tools_output() {
        let digest = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        // sha256sum and shasum: "<hash>  <path>"
        assert_eq!(
            extract_sha256(&format!("{digest}  /home/u/.cache/xsync/xs-abc")).as_deref(),
            Some(digest)
        );
        // certutil: banner, digest, trailer -- and it upper-cases.
        let certutil =
            format!("SHA256 hash of C:\\x\\xs.exe:\n{}\nCertUtil: -hashfile command completed successfully.", digest.to_uppercase());
        assert_eq!(extract_sha256(&certutil).as_deref(), Some(digest));
    }

    #[test]
    fn no_digest_in_output_is_reported_rather_than_guessed() {
        assert_eq!(extract_sha256("sha256sum: no such file"), None);
    }

    #[test]
    fn uploads_only_under_the_users_own_directory() {
        // Bootstrap must never install system-wide or need root.
        for policy in [BootstrapPolicy::Ephemeral, BootstrapPolicy::Persist] {
            let posix = remote_binary_path(RemoteShell::Posix, "/home/u", "deadbeef", policy);
            assert!(posix.starts_with("/home/u/"), "{posix}");
            let windows =
                remote_binary_path(RemoteShell::Windows, "C:\\Users\\u", "deadbeef", policy);
            assert!(windows.starts_with("C:/Users/u/"), "{windows}");
            assert!(
                std::path::Path::new(&windows)
                    .extension()
                    .is_some_and(|e| e == "exe"),
                "{windows}"
            );
        }
    }

    #[test]
    fn persist_targets_a_location_later_runs_can_find() {
        // A persisted binary must land on the PATH the remote command prepends,
        // otherwise it survives the run but no later run can discover it, and
        // every run re-uploads.
        let persisted = remote_binary_path(
            RemoteShell::Posix,
            "/home/u",
            "deadbeef",
            BootstrapPolicy::Persist,
        );
        assert_eq!(persisted, "/home/u/.local/bin/xs");
        let windows = remote_binary_path(
            RemoteShell::Windows,
            "C:\\Users\\u",
            "deadbeef",
            BootstrapPolicy::Persist,
        );
        assert_eq!(windows, "C:/Users/u/.local/bin/xs.exe");

        // An ephemeral one is digest-tagged and lives out of the way.
        let ephemeral = remote_binary_path(
            RemoteShell::Posix,
            "/home/u",
            "deadbeef",
            BootstrapPolicy::Ephemeral,
        );
        assert_eq!(ephemeral, "/home/u/.cache/xsync/xs-deadbeef");
    }

    #[test]
    fn locate_binary_refuses_a_mismatched_platform_with_an_actionable_message() {
        // Pick a platform this build definitely is not.
        let other = if host_target_triple().contains("linux") {
            RemotePlatform {
                os: RemoteOs::MacOs,
                arch: RemoteArch::Aarch64,
                libc: None,
            }
        } else {
            RemotePlatform {
                os: RemoteOs::Linux,
                arch: RemoteArch::X86_64,
                libc: Some(RemoteLibc::Musl),
            }
        };
        let error = locate_binary(other).unwrap_err();
        assert!(matches!(error, BootstrapError::NoMatchingBinary(_)));
        let message = error.to_string();
        assert!(message.contains(other.target_triple()), "{message}");
        assert!(message.contains("XSYNC_BOOTSTRAP_DIR"), "{message}");
    }

    #[test]
    fn bootstrap_is_off_unless_asked_for() {
        assert!(!BootstrapPolicy::default().enabled());
        assert!(BootstrapPolicy::Ephemeral.enabled());
        assert!(BootstrapPolicy::Persist.enabled());
    }
}
