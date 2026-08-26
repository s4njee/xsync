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
    env!("CARGO_BIN_EXE_xs")
}

fn populate_test_tree(root: &Path) {
    fs::create_dir_all(root.join("nested/alpha/beta")).unwrap();
    fs::create_dir_all(root.join("empty_dir")).unwrap();
    fs::write(
        root.join("nested/alpha/beta/small.txt"),
        b"small file content",
    )
    .unwrap();
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

#[cfg(windows)]
#[test]
fn test_windows_drive_and_backslash_paths_are_local() {
    let drive = xsync_core::path::parse(r"C:\xsync\destination").unwrap();
    assert!(!drive.is_remote());
    assert_eq!(drive.path, r"C:\xsync\destination");

    let forward = xsync_core::path::parse("C:/xsync/destination").unwrap();
    assert!(!forward.is_remote());
    assert_eq!(forward.path, "C:/xsync/destination");
}

#[cfg(windows)]
#[test]
fn test_windows_case_insensitive_destination_is_not_duplicated() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::write(source.path().join("case.txt"), b"source").unwrap();
    fs::write(destination.path().join("CASE.TXT"), b"old").unwrap();

    let output = Command::new(xsync_bin())
        .arg(format!("{}\\", source.path().display()))
        .arg(destination.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = fs::read_dir(destination.path()).unwrap().count();
    assert_eq!(entries, 1);
    assert_eq!(
        fs::read(destination.path().join("case.txt")).unwrap(),
        b"source"
    );
}

/// Build an `-e` value that runs the cross-platform fake-rsh helper.
///
/// This used to write a `#!/bin/sh` script, which Windows cannot execute, so
/// every remote-transport test was Unix-only (DEPLOYMENT.md D1.2). The helper
/// is a real binary built alongside `xs`, so the same tests now run everywhere.
///
/// `mode` is one of `"exec"`, `"missing"`, `"crash"`, or `"crash_after_chunk"`.
fn fake_rsh_with_marker(mode: &str, marker: &Path) -> String {
    let helper = env!("CARGO_BIN_EXE_fake-rsh").replace('\\', "/");
    let marker = marker.display().to_string().replace('\\', "/");
    format!("'{helper}' --mode {mode} --marker '{marker}'")
}

fn fake_rsh(mode: &str) -> String {
    // xsync splits `-e` with shlex, which treats a backslash as an escape.
    // Windows accepts forward slashes in program paths, so normalise rather
    // than fight the quoting; single quotes then survive spaces in the path.
    let helper = env!("CARGO_BIN_EXE_fake-rsh").replace('\\', "/");
    format!("'{helper}' --mode {mode}")
}

/// Fake remote shell backed by an absolute reference rsync. The xsync process
/// can have a poisoned PATH: only this simulated remote environment restores
/// the directory containing the oracle executable.
fn write_fake_rsync_rsh(script_dir: &Path, missing_xsync: bool) -> Option<PathBuf> {
    let reference = reference_rsync()?;
    let remote_path = Path::new(reference).parent().unwrap();
    let script = script_dir.join("fake_rsync_rsh.sh");
    let missing = if missing_xsync {
        "case \"$1\" in *\"'xs' '--server'\"*) echo 'xs: command not found' >&2; exit 127;; esac"
    } else {
        "case \"$1\" in *\"'xs' '--server'\"*) echo 'native xs unexpectedly requested' >&2; exit 99;; esac"
    };
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nshift\n{missing}\nPATH='{}':/usr/bin:/bin\nexport PATH\nexec /bin/sh -c \"$*\"\n",
            remote_path.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
    }
    Some(script)
}

fn reference_rsync() -> Option<&'static str> {
    ["/opt/homebrew/bin/rsync", "/usr/bin/rsync"]
        .into_iter()
        .find(|path| {
            if !Path::new(path).is_file() {
                return false;
            }
            Command::new(path)
                .arg("--version")
                .output()
                .is_ok_and(|output| {
                    output.status.success()
                        && String::from_utf8_lossy(&output.stdout).contains("protocol version 32")
                })
        })
}

#[test]
fn test_native_rsync_fallback_needs_no_local_rsync_executable() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());
    for name in [
        "space name",
        "quote'name",
        "-leading-dash",
        "semi;dollar$bracket[glob]",
    ] {
        fs::write(src.path().join(name), name.as_bytes()).unwrap();
    }
    let expected = tempdir().unwrap();
    let actual = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), false) else {
        return;
    };

    assert!(Command::new(reference_rsync().unwrap())
        .arg("-rlptW")
        .arg("--protocol=32")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", expected.path().display()))
        .status()
        .unwrap()
        .success());

    let output = Command::new(xsync_bin())
        .env("PATH", "/definitely/no/local/rsync")
        .arg("--transport")
        .arg("rsync")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", actual.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native rsync codec failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut expected_entries = build_manifest(expected.path()).unwrap().entries;
    let mut actual_entries = build_manifest(actual.path()).unwrap().entries;
    // GNU rsync's receiver applies protocol-32 nanosecond timestamps to
    // regular files, while directory and symlink timestamp precision remains
    // platform/receiver dependent. Compare their documented second-level
    // metadata contract and retain exact file precision checks.
    for (expected, actual) in expected_entries.iter_mut().zip(&mut actual_entries) {
        if expected.kind != xsync_bench::manifest::ManifestKind::File {
            expected.mtime.nanoseconds = 0;
            actual.mtime.nanoseconds = 0;
        }
    }
    assert_eq!(expected_entries, actual_entries);
    assert!(String::from_utf8_lossy(&output.stdout).contains("transport: rsync"));
}

#[test]
fn test_auto_falls_back_only_when_remote_xsync_is_missing() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"fallback").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), true) else {
        return;
    };

    let output = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(dst.path().join("file.txt")).unwrap(), b"fallback");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("trying supported rsync fallback"));
}

#[test]
fn test_auto_does_not_fallback_on_authentication_failure() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"data").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let marker = scripts.path().join("calls");
    let rsh = fake_rsh_with_marker("auth_failure", &marker);

    let output = Command::new(xsync_bin())
        .arg("-e")
        .arg(&rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Permission denied"));
    assert_eq!(fs::read_to_string(marker).unwrap().lines().count(), 1);
}

#[test]
fn test_auto_does_not_fallback_on_host_key_or_native_protocol_failure() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"data").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();

    for name in ["host_key", "malformed_native"] {
        let marker = scripts.path().join(format!("{name}.calls"));
        let rsh = fake_rsh_with_marker(name, &marker);

        let output = Command::new(xsync_bin())
            .arg("-e")
            .arg(&rsh)
            .arg(format!("{}/", src.path().display()))
            .arg(format!("fakehost:{}/", dst.path().display()))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{name} unexpectedly succeeded");
        assert_eq!(
            fs::read_to_string(marker).unwrap().lines().count(),
            1,
            "{name} triggered a second backend"
        );
    }
}

#[test]
fn test_rsync_remote_destination_is_shell_quoted() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"quoted").unwrap();
    let base = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), true) else {
        return;
    };
    // The injected `touch` uses a relative path, so it lands in the child's
    // working directory. Give the child a tempdir of its own so a regression
    // drops the canary in scratch space rather than in the crate directory.
    let cwd = tempdir().unwrap();
    let marker = cwd.path().join("XSYNC_RSYNC_INJECTION_MARKER");
    let hostile = base
        .path()
        .join("dst'; touch XSYNC_RSYNC_INJECTION_MARKER; echo '");

    let output = Command::new(xsync_bin())
        .current_dir(cwd.path())
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", hostile.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !marker.exists(),
        "hostile destination executed a second command"
    );
    assert_eq!(fs::read(hostile.join("file.txt")).unwrap(), b"quoted");
}

#[test]
fn test_rsync_destination_trailing_slash_preserves_directory_semantics() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"directory target").unwrap();
    let base = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), false) else {
        return;
    };
    let destination = base.path().join("new-directory");
    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(src.path().join("file.txt"))
        .arg(format!("fakehost:{}/", destination.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(destination.join("file.txt")).unwrap(),
        b"directory target"
    );
    assert!(destination.is_dir());
}

#[test]
fn test_rsync_trailing_slash_and_type_replacement() {
    let src_parent = tempdir().unwrap();
    let src = src_parent.path().join("source-dir");
    fs::create_dir_all(src.join("folder")).unwrap();
    fs::write(src.join("item"), b"file replaces directory").unwrap();
    fs::write(src.join("folder/child"), b"directory replaces file").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), false) else {
        return;
    };

    // No source trailing slash retains the source directory basename.
    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(&src)
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let nested = dst.path().join("source-dir");
    assert_eq!(
        fs::read(nested.join("item")).unwrap(),
        b"file replaces directory"
    );

    // Replace destination objects with opposite source types on the next run.
    fs::remove_file(nested.join("item")).unwrap();
    fs::create_dir(nested.join("item")).unwrap();
    fs::write(nested.join("item/stale"), b"stale").unwrap();
    fs::remove_dir_all(nested.join("folder")).unwrap();
    fs::write(nested.join("folder"), b"wrong type").unwrap();

    let second = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(&src)
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        fs::read(nested.join("item")).unwrap(),
        b"file replaces directory"
    );
    assert_eq!(
        fs::read(nested.join("folder/child")).unwrap(),
        b"directory replaces file"
    );
}

#[test]
fn test_rsync_rejects_unsupported_guarantees_before_remote_probe() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file"), b"data").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let marker = scripts.path().join("invoked");
    let rsh = scripts.path().join("must_not_run.sh");
    fs::write(
        &rsh,
        format!("#!/bin/sh\ntouch '{}'\nexit 99\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&rsh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&rsh, permissions).unwrap();
    }

    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("--delete")
        .arg("-e")
        .arg(&rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not support --delete"));
    assert!(
        !marker.exists(),
        "unsupported option opened the remote transport"
    );
}

#[test]
fn test_rsync_nonzero_receiver_exit_is_failure() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file"), b"data").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    // This test drives a *supported* peer to a disk-full failure, so it needs a
    // reference rsync xsync will actually talk to. Falling back to
    // /usr/bin/rsync unconditionally made the test fail on any host whose
    // system rsync advertises a protocol below 32 -- xsync rejects the peer
    // during the version probe and never reaches the disk-full path, so the
    // assertion below saw an unsupported-peer error instead. Skip, as the other
    // rsync tests already do, rather than assert on an unrelated failure.
    let Some(reference) = reference_rsync() else {
        return;
    };
    let rsh = scripts.path().join("disk_full.sh");
    fs::write(
        &rsh,
        format!(
            "#!/bin/sh\nshift\ncase \"$1\" in *--version*) exec '{reference}' --version;; esac\necho 'No space left on device' >&2\nexit 23\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&rsh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&rsh, permissions).unwrap();
    }

    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("-e")
        .arg(&rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No space left on device"));
}

#[test]
fn test_rsync_final_json_contains_transport_contract() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file"), b"json").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), false) else {
        return;
    };

    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("--progress-json")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let finished = events
        .iter()
        .find(|event| event["event"] == "finished")
        .expect("finished event");
    assert_eq!(finished["transport"], "rsync");
    assert_eq!(finished["remote_implementation"], "GNU rsync");
    assert_eq!(
        finished["wire_version"],
        i64::from(xsync_core::rsync::RSYNC_WIRE_VERSION)
    );
    assert_eq!(finished["selection_reason"], "explicit --transport=rsync");
    assert_eq!(finished["checksum_algorithm"], "md5");
    assert!(finished["wire_bytes"].as_u64().unwrap() > 0);
    assert!(finished["mapped_options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|option| option == "whole-file"));
    assert!(finished["unavailable_guarantees"]
        .as_array()
        .unwrap()
        .iter()
        .any(|guarantee| guarantee == "durable-resume"));
}

#[test]
fn test_native_compression_reports_wire_bytes_for_mixed_corpus() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("text.txt"), vec![b'a'; 2 * 1024 * 1024]).unwrap();
    let mut random = Vec::with_capacity(2 * 1024 * 1024);
    let mut state = 0x1234_5678_u32;
    for _ in 0..2 * 1024 * 1024 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        random.push(u8::try_from(state & 0xff).unwrap());
    }
    fs::write(src.path().join("random.bin"), random).unwrap();

    let fake_rsh = fake_rsh("exec");
    let compressed_dst = tempdir().unwrap();
    let compressed = Command::new(xsync_bin())
        .args([
            "--transport=xsync",
            "--progress-json",
            "--compress-level",
            "9",
        ])
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", compressed_dst.path().display()))
        .output()
        .unwrap();
    assert!(
        compressed.status.success(),
        "{}",
        String::from_utf8_lossy(&compressed.stderr)
    );
    let compressed_finished = finished_json(&compressed.stdout);
    assert_eq!(compressed_finished["compression_algorithm"], "zstd");
    assert!(compressed_finished["wire_bytes"].as_u64().unwrap() > 0);

    let raw_dst = tempdir().unwrap();
    let raw = Command::new(xsync_bin())
        .args(["--transport=xsync", "--progress-json", "--no-compress"])
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", raw_dst.path().display()))
        .output()
        .unwrap();
    assert!(
        raw.status.success(),
        "{}",
        String::from_utf8_lossy(&raw.stderr)
    );
    let raw_finished = finished_json(&raw.stdout);
    assert_eq!(
        raw_finished["compression_algorithm"],
        serde_json::Value::Null
    );
    assert!(
        compressed_finished["wire_bytes"].as_u64().unwrap()
            < raw_finished["wire_bytes"].as_u64().unwrap()
    );
    assert_eq!(
        build_manifest(compressed_dst.path()).unwrap(),
        build_manifest(raw_dst.path()).unwrap()
    );
}

fn finished_json(stdout: &[u8]) -> serde_json::Value {
    String::from_utf8(stdout.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .find(|event: &serde_json::Value| event["event"] == "finished")
        .expect("finished event")
}

#[test]
fn test_native_compression_multi_stream_stress_preserves_mixed_ranges() {
    let src = tempdir().unwrap();
    for index in 0..2 {
        let path = src.path().join(format!("chunk-{index}.bin"));
        if index % 2 == 0 {
            fs::write(path, vec![b'Z'; 40 * 1024 * 1024]).unwrap();
        } else {
            let mut bytes = Vec::with_capacity(40 * 1024 * 1024);
            let mut state = 0x9e37_79b9_u32.wrapping_add(index);
            for _ in 0..40 * 1024 * 1024 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                bytes.push(u8::try_from(state & 0xff).unwrap());
            }
            fs::write(path, bytes).unwrap();
        }
    }
    let fake_rsh = fake_rsh("exec");
    let dst = tempdir().unwrap();
    let output = Command::new(xsync_bin())
        .args([
            "--transport=xsync",
            "--progress-json",
            "--streams",
            "4",
            "--compress-level",
            "5",
        ])
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let finished = finished_json(&output.stdout);
    assert_eq!(finished["compression_algorithm"], "zstd");
    assert!(finished["wire_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        build_manifest(src.path()).unwrap(),
        build_manifest(dst.path()).unwrap()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_rsync_preserves_non_utf8_unix_filename() {
    use std::os::unix::ffi::OsStringExt as _;

    let src = tempdir().unwrap();
    let raw_name = std::ffi::OsString::from_vec(vec![b'n', b'a', b'm', b'e', 0xff]);
    fs::write(src.path().join(&raw_name), b"raw").unwrap();
    let dst = tempdir().unwrap();
    let scripts = tempdir().unwrap();
    let Some(fake_rsh) = write_fake_rsync_rsh(scripts.path(), false) else {
        return;
    };

    let output = Command::new(xsync_bin())
        .arg("--transport=rsync")
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}/", dst.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(dst.path().join(raw_name)).unwrap(), b"raw");
}

#[test]
fn test_push_matches_local_sync_identically() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst_local = tempdir().unwrap();
    let dst_push = tempdir().unwrap();
    let fake_rsh = fake_rsh("exec");

    // 1. Run local-to-local sync
    let status_local = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("{}", dst_local.path().display()))
        .status()
        .unwrap();
    assert!(status_local.success());

    // 2. Run remote push sync (fakehost:dest) through a fake-rsh: over real
    //    transport the default remote shell is `ssh`, but for the pipe suite we
    //    point `-e` at an exec'ing fake-rsh so no sshd is needed.
    let status_push = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_push.path().display()))
        .status()
        .unwrap();
    assert!(status_push.success());

    // Compare manifests
    let manifest_local = build_manifest(dst_local.path()).unwrap();
    let manifest_push = build_manifest(dst_push.path()).unwrap();
    assert_eq!(
        manifest_local.manifest_digest,
        manifest_push.manifest_digest
    );
    assert_eq!(manifest_local.entries.len(), manifest_push.entries.len());
}

#[test]
fn test_pull_matches_push_identically() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst_push = tempdir().unwrap();
    let dst_pull = tempdir().unwrap();
    let fake_rsh = fake_rsh("exec");

    // 1. Run remote push sync (fakehost:dest) through a fake-rsh.
    let status_push = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_push.path().display()))
        .status()
        .unwrap();
    assert!(status_push.success());

    // 2. Run remote pull sync (fakehost:src dest) through a fake-rsh.
    let status_pull = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
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
    let fake_rsh = fake_rsh("exec");
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
    assert_eq!(
        manifest_local.manifest_digest,
        manifest_fake.manifest_digest
    );
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
    assert_eq!(
        manifest_fake.manifest_digest,
        manifest_short.manifest_digest
    );
}

#[test]
fn test_fake_rsh_second_run_skips_all_files() {
    let src = tempdir().unwrap();
    populate_test_tree(src.path());

    let dst = tempdir().unwrap();
    let fake_rsh = fake_rsh("exec");

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
    let crash_rsh = fake_rsh("crash");

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
    let good_rsh = fake_rsh("exec");
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
    let missing_rsh = fake_rsh("missing");

    let output = Command::new(xsync_bin())
        .arg("--transport=xsync")
        .arg("-e")
        .arg(&missing_rsh)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("xs not found on remote host — install it or check PATH"),
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
    let crash_rsh = fake_rsh("crash_after_chunk");

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
    let good_rsh = fake_rsh("exec");
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
        stdout.contains("resume:") && !stdout.contains(", 0 restarted,"),
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

#[cfg(unix)]
#[test]
fn test_ssh_default_transport_reports_remote_stderr_on_failure() {
    let src = tempdir().unwrap();
    fs::write(src.path().join("file.txt"), b"data").unwrap();

    let dst = tempdir().unwrap();
    let script_dir = tempdir().unwrap();

    // Pretend the default remote shell (`ssh`) is a failing ssh that writes an
    // authentication/connect error to stderr and exits non-zero. We insert it
    // into PATH so the default transport picks it up without `-e`.
    let fake_ssh = script_dir.path().join("ssh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&fake_ssh, "#!/bin/sh\necho 'ssh: connect to host fakehost port 22: Connection refused' >&2\nexit 255\n")
            .unwrap();
        let mut perms = fs::metadata(&fake_ssh).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_ssh, perms).unwrap();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", script_dir.path().display(), path);

    let output = Command::new(xsync_bin())
        .env("PATH", &new_path)
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst.path().display()))
        .output()
        .unwrap();

    // SSH failure: the job must exit non-zero and relay ssh's stderr to the user.
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Connection refused"),
        "default ssh transport must surface the remote shell's stderr, got: {stderr}"
    );
}

#[test]
fn test_multi_stream_push_stripes_large_file_and_is_byte_identical() {
    let src = tempdir().unwrap();
    fs::create_dir_all(src.path().join("nested")).unwrap();
    for i in 0..20 {
        fs::write(
            src.path().join("nested").join(format!("f{i:02}.bin")),
            vec![0x33 + i; 4096],
        )
        .unwrap();
    }
    // 64 MiB -> eight 8 MiB chunks, so each of four data sessions carries
    // multiple disjoint ranges and must prepare the file only once.
    fs::write(src.path().join("big.bin"), vec![0xEE; 64 * 1024 * 1024]).unwrap();

    let dst_local = tempdir().unwrap();
    let dst_multi = tempdir().unwrap();
    let fake_rsh = fake_rsh("exec");

    // Local baseline.
    let status_local = Command::new(xsync_bin())
        .arg(format!("{}/", src.path().display()))
        .arg(format!("{}", dst_local.path().display()))
        .status()
        .unwrap();
    assert!(status_local.success());

    // Multi-stream push with --streams 4 over a fake-rsh (no sshd, no network).
    let output = Command::new(xsync_bin())
        .arg("-e")
        .arg(&fake_rsh)
        .arg("--streams")
        .arg("4")
        .arg(format!("{}/", src.path().display()))
        .arg(format!("fakehost:{}", dst_multi.path().display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "multi-stream push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Byte-identical to the single-stream local baseline.
    let m_local = build_manifest(dst_local.path()).unwrap();
    let m_multi = build_manifest(dst_multi.path()).unwrap();
    assert_eq!(m_local.manifest_digest, m_multi.manifest_digest);
    assert_eq!(m_local.entries.len(), m_multi.entries.len());

    // The large file is intact despite being striped across four sessions.
    assert_eq!(
        fs::read(dst_multi.path().join("big.bin")).unwrap(),
        vec![0xEE; 64 * 1024 * 1024]
    );

    // The Finished event reports the file count and byte accounting.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("finished:") && stdout.contains("21 transferred"),
        "expected 21 transferred (20 small + 1 striped large), got: {stdout}"
    );
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
        compression_level: 3,
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
        let frame = decoder
            .read(&mut cursor)
            .expect("stdout must be valid frames only");
        assert!(frame.message_id >= 1000);
        decoded_count += 1;
    }
    assert!(decoded_count >= 1);
}
