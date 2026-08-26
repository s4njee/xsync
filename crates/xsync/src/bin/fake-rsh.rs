//! Test-only stand-in for the remote shell, used by the integration harness.
//!
//! The harness previously wrote `#!/bin/sh` scripts and passed them to `-e`.
//! Windows cannot execute those, so every remote-transport test was unrunnable
//! there — the gap recorded against DEPLOYMENT.md story D1.2. This binary does
//! the same job as a real `ssh` would, on every platform.
//!
//! Invoked as `fake-rsh --mode <mode> <host> <command-string>`, matching how
//! `-e CMD` is expanded: xsync appends the host and one command string.
//! The host is ignored; the server root is taken from the command string.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Split a remote command string into words, honouring both quoting styles.
///
/// The string is whatever `xsync_remote_command` produced: POSIX
/// (`PATH="..." 'xs' '--server' '/p'`) or cmd (`set "..." & xs --server "C:/p"`).
/// Only enough quoting is handled to recover the arguments.
fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    // Tracks a quoted word that decoded to nothing, so `''` stays an argument.
    let mut started = false;
    for ch in command.chars() {
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() {
            if started || !current.is_empty() {
                out.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

/// The server root is the word after `--server`.
fn server_root(command: &str) -> Option<String> {
    let words = words(command);
    let index = words.iter().position(|word| word == "--server")?;
    words.get(index + 1).cloned()
}

/// Locate the `xs` binary built alongside this helper.
fn xsync_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("current_exe");
    path.pop();
    path.join(if cfg!(windows) { "xs.exe" } else { "xs" })
}

/// Whether a staged temporary under `root` has reached `threshold` bytes.
fn staged_at_least(root: &Path, threshold: u64) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(".xsync.tmp.")
            && entry.metadata().is_ok_and(|meta| meta.len() >= threshold)
    })
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.iter().position(|a| a == "--mode") {
        Some(i) => args.get(i + 1).cloned().unwrap_or_default(),
        None => "exec".to_owned(),
    };
    // Trailing positionals are the host and the single command string.
    let command = args.last().cloned().unwrap_or_default();

    // Some tests count how many times the remote shell was reached, to prove
    // xsync does not retry a failure it must not retry. Record every call.
    if let Some(i) = args.iter().position(|a| a == "--marker") {
        if let Some(path) = args.get(i + 1) {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{command}");
            }
        }
    }

    // Failures that belong to ssh itself rather than to the remote command.
    // ssh reports these as 255, which xsync must never retry.
    match mode.as_str() {
        "auth_failure" => {
            eprintln!("Permission denied (publickey)");
            return std::process::ExitCode::from(255);
        }
        "host_key" => {
            eprintln!("Host key verification failed.");
            return std::process::ExitCode::from(255);
        }
        // A peer that answers, but not with the protocol. Exits 0 having
        // written to stdout, so it must not be mistaken for a shell that never
        // ran the server.
        "malformed_native" => {
            print!("bad native protocol");
            let _ = std::io::stdout().flush();
            return std::process::ExitCode::SUCCESS;
        }
        _ => {}
    }

    // Emulate a remote shell that cannot find the binary: the exit status and
    // message are what `is_missing_xsync_stderr` keys on.
    if mode == "missing" {
        eprintln!("xs: command not found");
        return std::process::ExitCode::from(127);
    }

    let Some(root) = server_root(&command) else {
        eprintln!("fake-rsh: no --server argument in {command:?}");
        return std::process::ExitCode::from(2);
    };

    let mut child = match Command::new(xsync_binary())
        .arg("--server")
        .arg(&root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "fake-rsh: cannot spawn xs: {error}");
            return std::process::ExitCode::from(127);
        }
    };

    match mode.as_str() {
        // Kill shortly after start so the client sees a mid-transfer
        // disconnect and only staging artifacts remain.
        "crash" => {
            std::thread::sleep(Duration::from_millis(50));
            let _ = child.kill();
            let _ = child.wait();
            std::process::ExitCode::from(1)
        }
        // Kill once the receiver has durably staged the first 8 MiB chunk, so
        // the resume journal survives with that range verified.
        "crash_after_chunk" => {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if staged_at_least(Path::new(&root), 8 * 1024 * 1024) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = child.kill();
            let _ = child.wait();
            std::process::ExitCode::from(1)
        }
        _ => {
            let status = child.wait().expect("wait for xs --server");
            std::process::ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1))
        }
    }
}
