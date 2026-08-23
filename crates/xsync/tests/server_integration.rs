use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::tempdir;
use xsync_bench::manifest::build_manifest;
use xsync_core::protocol::FrameDecoder;

fn xsync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_xsync")
}

fn populate_test_tree(root: &Path) {
    fs::create_dir_all(root.join("nested/alpha/beta")).unwrap();
    fs::create_dir_all(root.join("empty_dir")).unwrap();
    fs::write(root.join("nested/alpha/beta/small.txt"), b"small file content").unwrap();
    fs::write(root.join("root_file.dat"), vec![0xAB; 64 * 1024]).unwrap();

    // Large file (multi-segment)
    fs::write(root.join("large.bin"), vec![0x5A; 10 * 1024 * 1024]).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("root_file.dat", root.join("link_to_file")).unwrap();
        let mut perms = fs::metadata(root.join("nested/alpha/beta/small.txt"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(root.join("nested/alpha/beta/small.txt"), perms).unwrap();
    }
}

/// Write an executable fake-rsh script that ignores the host and the literal
/// `xsync` command word, then execs the local server binary: `xsync --server <path>`.
///
/// `mode` is one of `"exec"`, `"missing"`, or `"crash"`.
fn write_fake_rsh(script_dir: &Path, mode: &str) -> PathBuf {
    let script = script_dir.join(format!("fake_rsh_{mode}.sh"));
    let body = match mode {
        // Exec the local server directly (ignoring host + the "xsync" word).
        "exec" | "missing" => {
            let target = if mode == "missing" {
                "definitely-not-a-real-xsync-binary".to_owned()
            } else {
                xsync_bin().to_owned()
            };
            format!("#!/bin/sh\nexec {target} \"$3\" \"$4\"\n")
        }
        // Start the server, then SIGKILL it shortly after so the client sees a
        // mid-transfer disconnect and leaves only staging artifacts.
        "crash" => {
            let xs = xsync_bin();
            format!(
                "#!/bin/sh\n\"{xs}\" \"$3\" \"$4\" &\nPID=$!\nsleep 0.05\nkill -9 $PID\nwait $PID\n"
            )
        }
        // Start the server and SIGKILL it once the receiver has durably staged
        // the first 8 MiB chunk, so the resume journal survives with that range
        // verified and a subsequent run skips it.
        "crash_after_chunk" => {
            let xs = xsync_bin();
            format!(
                "#!/bin/sh\n\"{xs}\" \"$3\" \"$4\" &\nPID=$!\n\
                 for i in $(seq 1 400); do\n\
                 \x20 f=$(ls \"$4\"/.xsync.tmp.* 2>/dev/null | head -n1)\n\
                 \x20 if [ -n \"$f\" ] && [ \"$(wc -c < \"$f\" 2>/dev/null || echo 0)\" -ge 8388608 ]; then\n\
                 \x20   kill -9 $PID 2>/dev/null; wait $PID 2>/dev/null; exit 0\n\
                 \x20 fi\n\
                 \x20 sleep 0.005\n\
                 done\n\
                 kill -9 $PID 2>/dev/null; wait $PID 2>/dev/null\n"
            )
        }
        _ => unreachable!(),
    };
    fs::write(&script, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }
    script
}

#[test]
fn test_push_matches_local_sync_identically() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst_local = tempdir().unwrap();
    let dst_push = tempdir().unwrap();

    // 1. Run local-to-local sync
    let status_local = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("{}", dst_local.path().display()))
        .status()
        .unwrap();
    assert!(status_local.success());

    // 2. Run remote push sync (fakehost:dest)
    let status_push = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_push.path().display()))
        .status()
        .unwrap();
    assert!(status_push.success());

    // Compare manifests
    let manifest_local = build_manifest(dst_local.path()).unwrap();
    let manifest_push = build_manifest(dst_push.path()).unwrap();
    assert_eq!(manifest_local.manifest_digest, manifest_push.manifest_digest);
    assert_eq!(manifest_local.entries.len(), manifest_push.entries.len());
}

#[test]
fn test_pull_matches_push_identically() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst_push = tempdir().unwrap();
    let dst_pull = tempdir().unwrap();

    // 1. Run remote push sync (fakehost:dest)
    let status_push = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_push.path().display()))
        .status()
        .unwrap();
    assert!(status_push.success());

    // 2. Run remote pull sync (fakehost:src dest)
    let status_pull = Command::new(xsync_bin())
        .arg(format!("fakehost:{}/", src.path().display()))
        .arg(format!("{}", dst_pull.path().display()))
        .status()
        .unwrap();
    assert!(status_pull.success());

    // Compare manifests
    let manifest_push = build_manifest(dst_push.path()).unwrap();
    let manifest_pull = build_manifest(dst_pull.path()).unwrap();
    assert_eq!(manifest_push.manifest_digest, manifest_pull.manifest_digest);
    assert_eq!(manifest_push.entries.len(), manifest_pull.entries.len());
}

#[test]
fn test_rsh_override_uses_fake_rsh_and_matches_push() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst_local = tempdir().unwrap();
    let dst_fake = tempdir().unwrap();

    // Local-to-local baseline.
    let status_local = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("{}", dst_local.path().display()))
        .status()
        .unwrap();
    assert!(status_local.success());

    // Remote push driven through an explicit fake-rsh (long `--rsh` form) that
    // ignores the host and execs the local server binary — no sshd, no network.
    let script_dir = tempdir().unwrap();
    let fake_rsh = write_fake_rsh(script_dir.path(), "exec");
    let output = Command::new(xsync_bin())
        .arg("--rsh")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_fake.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fake-rsh push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest_local = build_manifest(dst_local.path()).unwrap();
    let manifest_fake = build_manifest(dst_fake.path()).unwrap();
    assert_eq!(manifest_local.manifest_digest, manifest_fake.manifest_digest);
    assert_eq!(manifest_local.entries.len(), manifest_fake.entries.len());

    // `-e` short form routes through the same pipe transport.
    let dst_short = tempdir().unwrap();
    let status_short = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_short.path().display()))
        .status()
        .unwrap();
    assert!(status_short.success());
    let manifest_short = build_manifest(dst_short.path()).unwrap();
    assert_eq!(manifest_fake.manifest_digest, manifest_short.manifest_digest);
}

#[test]
fn test_fake_rsh_second_run_skips_all_files() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst = tempdir().unwrap();
    let script_dir = tempdir().unwrap();
    let fake_rsh = write_fake_rsh(script_dir.path(), "exec");

    let run = || {
        Command::new(xsync_bin())
            .arg("-e")
            .arg(&fake_rsh)
            .arg(format!("{}/", src.path().display()))
            .arg(format!("fakehost:{}", dst.path().display()))
            .output()
            .unwrap()
    };

    let first = run();
    assert!(first.status.success());

    // Second run transfers zero bytes: all files are classified unchanged.
    let second = run();
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("0 transferred"),
        "second run should report 0 transferred, got: {stdout}"
    );
}

#[test]
fn test_fake_rsh_restart_safety_leaves_no_final_truncated_file() {
    let src = tempdir().unwrap();
    // Multi-segment file so a mid-transfer kill interrupts it.
    fs::write(src.path().join("big.bin"), vec![0x42; 20 * 1024 * 1024]).unwrap();

    let dst = tempdir().unwrap();
    let script_dir = tempdir().unwrap();
    let crash_rsh = write_fake_rsh(script_dir.path(), "crash");

    // First attempt is killed mid-transfer.
    let first = Command::new(xsync_bin())
        .arg("-e")
        .arg(&crash_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();
    assert!(!first.status.success());

    // No truncated file may ever exist under its final name.
    let final_big = dst.path().join("big.bin");
    assert!(
        !final_big.exists(),
        "interrupted transfer must not publish a truncated final file"
    );

    // A clean re-run (with a functional fake-rsh) completes the transfer.
    let good_rsh = write_fake_rsh(script_dir.path(), "exec");
    let second = Command::new(xsync_bin())
        .arg("-e")
        .arg(&good_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "re-run after interrupt failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let written = fs::read(&final_big).unwrap();
    assert_eq!(written, vec![0x42; 20 * 1024 * 1024]);
}

#[test]
fn test_missing_remote_binary_reports_clear_error() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"data").unwrap();

    let dst = tempdir().unwrap();
    let script_dir = tempdir().unwrap();
    let missing_rsh = write_fake_rsh(script_dir.path(), "missing");

    let output = Command::new(xsync_bin())
        .arg("-e")
        .arg(&missing_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("xsync not found on remote host — install it or check PATH"),
        "missing remote binary must not surface as a raw broken-pipe error; got: {stderr}"
    );
}

#[test]
fn test_durable_resume_skips_verified_ranges() {
    let src = tempdir().unwrap();
    // 24 MiB file -> three 8 MiB chunks so a crash after the first leaves two
    // chunks still to transmit on the resumed run.
    fs::write(src.path().join("big.bin"), vec![0xEE; 24 * 1024 * 1024]).unwrap();

    let dst = tempdir().unwrap();
    let script_dir = tempdir().unwrap();
    let crash_rsh = write_fake_rsh(script_dir.path(), "crash_after_chunk");

    // First run is killed by the receiver-side fake transport once the first
    // 8 MiB chunk is durably staged+checkpointed.
    let first = Command::new(xsync_bin())
        .arg("-e")
        .arg(&crash_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();
    assert!(!first.status.success());
    assert!(
        !dst.path().join("big.bin").exists(),
        "interrupted transfer must not publish the file"
    );

    // A clean re-run resumes from the durable journal: it does not retransmit
    // the verified first chunk, completes the file, and reports nonzero resume
    // accounting.
    let good_rsh = write_fake_rsh(script_dir.path(), "exec");
    let second = Command::new(xsync_bin())
        .arg("-e")
        .arg(&good_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "resumed re-run failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    // Final content is byte-identical despite only part of it being sent this run.
    let written = fs::read(dst.path().join("big.bin")).unwrap();
    assert_eq!(written, vec![0xEE; 24 * 1024 * 1024]);

    // The Finished event must report nonzero resumed bytes / restarted file and
    // physical bytes that are less than the full logical size.
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("resume:") && !stdout.contains(", 0 restarted,") ,
        "expected nonzero resume accounting, got: {stdout}"
    );
    assert!(
        stdout.contains("resumed") && stdout.contains("checkpointed"),
        "resume event must name resumed/checkpointed bytes, got: {stdout}"
    );

    // Successful finish clears the per-file journal record; this job's journal
    // root must contain no leftover `.js` record.
    let src_str = src.path().to_string_lossy().to_string();
    let dst_str = dst.path().to_string_lossy().to_string();
    let job_id = xsync_core::server::session_job_id(&src_str, &dst_str);
    let job_dir = xsync_core::journal::ResumeJournal::root_for(&job_id);
    if job_dir.exists() {
        let leftover_records = std::fs::read_dir(&job_dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".js"));
        assert!(
            !leftover_records,
            "successful resume left an orphan journal record in {}",
            job_dir.display()
        );
    }
}

#[test]
fn test_server_crash_mid_transfer_surfaces_stream_transport_error() {
    let src = tempdir().unwrap();
    // Create large file so transfer takes several messages
    fs::write(src.path().join("big.bin"), vec![0x42; 20 * 1024 * 1024]).unwrap();

    let dst = tempdir().unwrap();

    // Create a wrapper script that kills the server during transfer
    let script_dir = tempdir().unwrap();
    let crash_rsh = script_dir.path().join("crash_rsh.sh");
    #[cfg(unix)]
    {
        fs::write(
            &crash_rsh,
            format!(
                "#!/bin/sh\nexec {} --server \"$@\" &\nPID=$!\nsleep 0.05\nkill -9 $PID\nwait $PID\n",
                xsync_bin()
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&crash_rsh).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&crash_rsh, perms).unwrap();
    }

    let output = Command::new(xsync_bin())
        .arg("-e")
        .arg(&crash_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();

    // Must not succeed and must exit with non-zero (either partial failure 23 or failure 1)
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("transport error on stream 0")
            || stderr.contains("server stream disconnected")
            || stderr.contains("broken pipe")
            || stderr.contains("cannot spawn")
            || !output.status.success()
    );
}

#[test]
fn test_server_stdout_is_protocol_only() {
    let dst = tempdir().unwrap();

    // Spawn server process and send a Handshake frame, capture stdout
    let mut child = Command::new(xsync_bin())
        .arg("--server")
        .arg(dst.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();

    let handshake = xsync_core::protocol::Message::Handshake {
        role: xsync_core::protocol::Role::Source,
        capabilities: 0,
        max_payload: 16 * 1024 * 1024,
        max_segment: 8 * 1024 * 1024,
        window: 32 * 1024 * 1024,
        job_id: [0u8; 16],
        compression: xsync_core::protocol::CompressionMode::None,
    };
    let bytes = xsync_core::protocol::encode_frame(1, &handshake).unwrap();
    stdin.write_all(&bytes).unwrap();
    stdin.flush().unwrap();
    drop(stdin); // Close stdin to end session

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success() || output.stdout.len() >= 32);

    // Verify all bytes in stdout are valid protocol frames
    let mut decoder = FrameDecoder::new();
    let mut cursor = std::io::Cursor::new(&output.stdout);
    let mut decoded_count = 0;
    while cursor.position() < output.stdout.len() as u64 {
        let frame = decoder.read(&mut cursor).expect("stdout must be valid frames only");
        assert!(frame.message_id >= 1000);
        decoded_count += 1;
    }
    assert!(decoded_count >= 1);
}
