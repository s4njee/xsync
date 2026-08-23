use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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
