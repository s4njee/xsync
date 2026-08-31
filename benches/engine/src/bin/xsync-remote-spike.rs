//! Remote-transfer spike.
//!
//! The spike drives `ssh`, reads `st_mode`/`st_mtime` directly, and `exec`s
//! `rsync` for its comparison arm -- all Unix-only. It used to take those
//! imports unconditionally, which failed `cargo build --workspace` on
//! `x86_64-pc-windows-msvc` and so failed CI on a target we release for. The
//! implementation now compiles only where it can run.

use std::process::ExitCode;

#[cfg(unix)]
#[path = "../remote_spike_unix.rs"]
mod imp;

#[cfg(unix)]
fn main() -> ExitCode {
    imp::main()
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!("xsync-remote-spike is a Unix-only benchmark and does nothing here");
    ExitCode::FAILURE
}
