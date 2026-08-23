//! Shared benchmark environment identification.

use std::path::Path;
use std::process::Command;

use crate::report::Environment;

/// Capture machine, revision, build, and source-filesystem identity.
#[must_use]
pub fn environment(root: &Path, binary_name: &str) -> Environment {
    Environment {
        source_revision: command_value("git", &["rev-parse", "HEAD"]),
        build_id: format!(
            "{binary_name}-{}-{}",
            env!("CARGO_PKG_VERSION"),
            command_value("rustc", &["--version"])
        ),
        host: std::env::var("HOSTNAME")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| command_value("uname", &["-n"])),
        hardware: hardware_value(),
        os: std::env::consts::OS.to_owned(),
        kernel: command_value("uname", &["-sr"]),
        filesystem: filesystem_value(root),
    }
}

fn command_value(command: &str, arguments: &[&str]) -> String {
    Command::new(command)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(target_os = "macos")]
fn hardware_value() -> String {
    command_value("sysctl", &["-n", "machdep.cpu.brand_string"])
}

#[cfg(target_os = "linux")]
fn hardware_value() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|value| value.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn hardware_value() -> String {
    "unknown".to_owned()
}

#[cfg(target_os = "macos")]
fn filesystem_value(root: &Path) -> String {
    let mount_point = Command::new("df")
        .args(["-P"])
        .arg(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| {
            output
                .lines()
                .last()
                .and_then(|line| line.split_whitespace().last())
                .map(str::to_owned)
        });
    let mounts = command_value("mount", &[]);
    mount_point
        .and_then(|mount_point| {
            let marker = format!(" on {mount_point} (");
            mounts.lines().find_map(|line| {
                line.split_once(&marker)
                    .and_then(|(_, options)| options.split(',').next())
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(target_os = "linux")]
fn filesystem_value(root: &Path) -> String {
    Command::new("findmnt")
        .arg("-T")
        .arg(root)
        .args(["-n", "-o", "FSTYPE"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn filesystem_value(_root: &Path) -> String {
    "unknown".to_owned()
}
