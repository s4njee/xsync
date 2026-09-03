//! The async client against the real synchronous server, over a real socket.
//!
//! A loopback socket rather than an in-process pipe: it exercises the same
//! framing a deployment uses, and it is the only way to have the synchronous
//! server and the async client both run as themselves.

use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use tempfile::tempdir;
use xsync_client::{
    attr_presence, Access, Client, Error, ErrorCode, RenameMode, TimeChange, CREATE, DIRECTORY,
    READ, WRITE,
};
use xsync_core::server::Server;

/// Serve one session from `root`, returning the address to connect to.
fn serve(root: PathBuf, read_only: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let address = listener.local_addr().expect("local address").to_string();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let reader = stream.try_clone().expect("clone the socket");
        let mut server = Server::new(root).read_only(read_only);
        // The session ends when the client drops the connection, which is a
        // clean finish rather than a failure.
        let _ = server.run(reader, stream);
    });
    address
}

async fn connect(address: &str) -> Client {
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    Client::from_stream(stream, 0).await.expect("negotiate v3")
}

#[tokio::test]
async fn mounts_and_reports_what_the_export_allows() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("notes.txt"), b"hello world").unwrap();

    let client = connect(&serve(root.path().to_path_buf(), false)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();

    assert!(mount.info().writable, "a fresh temp dir is writable");
    assert!(mount.info().reason.is_empty());
    assert!(mount.info().max_read > 0 && mount.info().max_write > 0);

    let usage = mount.statfs().await.unwrap();
    assert!(usage.total_bytes > 0);
    assert!(usage.available_bytes <= usage.free_bytes);
    assert!(!usage.read_only);

    let attrs = mount
        .stat(b"notes.txt", true, attr_presence::IDENTITY)
        .await
        .unwrap();
    assert_eq!(attrs.kind, 1);
    assert_eq!(attrs.size, 11);
    assert!(attrs.identity.is_some());
}

#[tokio::test]
async fn reads_and_writes_a_file_through_a_handle() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"0123456789").unwrap();

    let client = connect(&serve(root.path().to_path_buf(), false)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();

    let file = mount.open(b"file", READ | WRITE).await.unwrap();
    assert_eq!(file.attrs().size, 10);

    // Verified: the client checks the server's digest before handing the bytes
    // over, which is the whole reason `want_digest` exists.
    let chunk = file.read_verified(2, 4).await.unwrap();
    assert_eq!(chunk.data, b"2345");
    assert_eq!(chunk.offset, 2);
    assert!(!chunk.eof);

    let written = file.write(0, b"AB").await.unwrap();
    assert_eq!(written.bytes, 2);
    assert_eq!(written.new_size, 10);
    file.flush().await.unwrap();

    let after = file.read(0, 32).await.unwrap();
    assert_eq!(after.data, b"AB23456789");
    assert!(after.eof, "a short read at the end is marked");
    file.close().await.unwrap();

    assert_eq!(
        std::fs::read(root.path().join("file")).unwrap(),
        b"AB23456789"
    );
}

#[tokio::test]
async fn lists_a_directory_across_pages() {
    let root = tempdir().unwrap();
    let listing = root.path().join("listing");
    std::fs::create_dir(&listing).unwrap();
    for index in 0..300 {
        std::fs::write(listing.join(format!("e{index:03}")), b"x").unwrap();
    }

    let client = connect(&serve(root.path().to_path_buf(), false)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();
    let directory = mount.open(b"listing", READ | DIRECTORY).await.unwrap();

    // Paged by hand, to see the cursor work.
    let mut names = Vec::new();
    let mut cursor = 0;
    loop {
        let page = directory.read_dir(cursor, 64, 0).await.unwrap();
        names.extend(page.entries.into_iter().map(|entry| entry.name));
        if page.final_page {
            break;
        }
        cursor = page.cursor;
    }
    assert_eq!(names.len(), 300);

    // And the convenience that hides the paging.
    let all = directory
        .read_dir_all(attr_presence::IDENTITY)
        .await
        .unwrap();
    assert_eq!(all.len(), 300);
    assert!(all.iter().all(|entry| entry.attrs.identity.is_some()));
    directory.close().await.unwrap();
}

#[tokio::test]
async fn a_read_only_export_is_visible_before_anything_is_attempted() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"x").unwrap();

    let client = connect(&serve(root.path().to_path_buf(), true)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();

    // The point of advertising writability: a UI disables its write verbs from
    // this, without a failed attempt.
    assert!(!mount.info().writable);
    assert_eq!(mount.info().reason, "export is read-only");
    assert_eq!(mount.info().access, Access::ReadOnly);

    // And an attempt anyway is refused with the mount's own reason.
    let error = mount
        .open(b"file", WRITE | CREATE)
        .await
        .expect_err("a write-class open must be refused");
    match &error {
        Error::Server { code, message, .. } => {
            assert_eq!(*code, ErrorCode::ReadOnly);
            assert_eq!(message, "export is read-only");
        }
        other => panic!("expected a server error, got {other:?}"),
    }
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);

    // Reading still works.
    let file = mount.open(b"file", READ).await.unwrap();
    assert_eq!(file.read(0, 8).await.unwrap().data, b"x");
}

#[tokio::test]
async fn server_errors_arrive_as_typed_errors() {
    let root = tempdir().unwrap();
    let client = connect(&serve(root.path().to_path_buf(), false)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();

    let missing = mount
        .stat(b"nope", true, 0)
        .await
        .expect_err("a missing path is an error");
    assert!(matches!(
        missing,
        Error::Server {
            code: ErrorCode::NoEntry,
            ..
        }
    ));
    assert_eq!(missing.kind(), ErrorKind::NotFound);

    // A path that leaves the export never reaches the filesystem.
    let escape = mount
        .stat(b"../secret", true, 0)
        .await
        .expect_err("traversal must be refused");
    assert!(matches!(escape, Error::Server { .. }));

    // The session survives both, so a third request still works.
    assert!(mount.statfs().await.is_ok());
}

#[tokio::test]
async fn requests_are_multiplexed_over_one_connection() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("file"), vec![7_u8; 64 * 1024]).unwrap();

    let client = connect(&serve(root.path().to_path_buf(), false)).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();
    let file = Arc::new(mount.open(b"file", READ).await.unwrap());

    // Thirty-two reads issued without awaiting any of them. They are answered
    // in whatever order the server finishes; the client routes each reply to
    // the caller that asked for it.
    let mut tasks = Vec::new();
    for index in 0..32_u64 {
        let file = Arc::clone(&file);
        tasks.push(tokio::spawn(async move {
            let chunk = file.read_verified(index * 2048, 2048).await.unwrap();
            (index, chunk.data.len())
        }));
    }
    let mut answered = Vec::new();
    for task in tasks {
        answered.push(task.await.unwrap());
    }
    answered.sort_unstable();
    assert_eq!(answered.len(), 32);
    for (index, (answered_index, length)) in answered.into_iter().enumerate() {
        assert_eq!(
            answered_index, index as u64,
            "a reply went to the wrong caller"
        );
        assert_eq!(length, 2048);
    }
}

#[tokio::test]
async fn a_dropped_connection_fails_waiting_requests_rather_than_hanging() {
    let root = tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    // A peer that completes the handshake and then goes away.
    let root_path = root.path().to_path_buf();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let reader = stream.try_clone().expect("clone");
        let mut server = Server::new(root_path);
        let _ = server.run(reader, stream);
        // Dropped here: the socket closes under the client.
    });

    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.unwrap();
    // Force the server thread to finish and the socket to close, then keep
    // asking. Whatever happens, it must not hang.
    drop(mount);
    let mount = client.mount(b"", Access::ReadWrite).await;
    // Either the second mount was refused (the session mounts once) or the
    // connection had already gone; both are errors, neither is a hang.
    assert!(mount.is_err());
}

#[tokio::test]
async fn the_mutation_set_works_end_to_end_over_a_real_socket() {
    // One test walking the whole set, because the interesting thing is that
    // they compose: make a directory, put something in it, move it, link it,
    // stamp it, and take it all away again.
    let root = tempdir().unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let made = mount.mkdir(b"box", 0o755).await.expect("mkdir");
    assert_eq!(made.kind, 2, "mkdir did not report a directory");

    // `mkdir`, not `mkdir -p`: the parent has to exist.
    let deep = mount.mkdir(b"absent/child", 0o755).await;
    assert!(
        matches!(
            deep,
            Err(Error::Server {
                code: ErrorCode::NoEntry,
                ..
            })
        ),
        "{deep:?}"
    );

    for name in [&b"box/note.txt"[..], b"box/other.txt"] {
        let file = mount
            .open(name, READ | WRITE | CREATE)
            .await
            .expect("create");
        file.write(0, b"hello").await.expect("write");
        file.close().await.expect("close");
    }

    mount
        .link(b"box/note.txt", b"box/hardlink")
        .await
        .expect("link");
    mount
        .symlink(b"note.txt", b"box/pointer")
        .await
        .expect("symlink");

    // Renaming one hard link onto another is *not* a way to remove a name.
    // POSIX says a rename whose two arguments resolve to the same file
    // succeeds and does nothing, so both names survive — which is why the
    // rename below uses a file with an inode of its own.
    mount
        .rename(b"box/note.txt", b"box/hardlink", RenameMode::Replace)
        .await
        .expect("same-inode rename is a successful no-op");
    assert!(root.path().join("box/note.txt").exists());
    assert!(root.path().join("box/hardlink").exists());

    // NoReplace refuses an existing destination; Replace takes it.
    let refused = mount
        .rename(b"box/other.txt", b"box/note.txt", RenameMode::NoReplace)
        .await;
    assert!(
        matches!(
            refused,
            Err(Error::Server {
                code: ErrorCode::Exists,
                ..
            })
        ),
        "{refused:?}"
    );
    mount
        .rename(b"box/other.txt", b"box/note.txt", RenameMode::Replace)
        .await
        .expect("rename replace");
    assert!(!root.path().join("box/other.txt").exists());

    let stamped = mount
        .set_times(
            b"box/note.txt",
            TimeChange::Omit,
            TimeChange::Set {
                seconds: 1_000_000,
                nanos: 0,
            },
            true,
        )
        .await
        .expect("set times");
    assert_eq!(stamped.mtime_ns, 1_000_000 * 1_000_000_000);

    let moded = mount
        .set_permissions(b"box/note.txt", 0o640, true)
        .await
        .expect("chmod");
    assert_eq!(moded.mode & 0o777, 0o640);

    // A directory with entries will not rmdir, and a directory will not unlink.
    assert!(matches!(
        mount.rmdir(b"box").await,
        Err(Error::Server {
            code: ErrorCode::NotEmpty,
            ..
        })
    ));
    assert!(matches!(
        mount.unlink(b"box").await,
        Err(Error::Server {
            code: ErrorCode::IsDirectory,
            ..
        })
    ));

    for name in [&b"box/note.txt"[..], b"box/hardlink", b"box/pointer"] {
        mount.unlink(name).await.expect("unlink");
    }
    mount.rmdir(b"box").await.expect("rmdir");
    assert!(!root.path().join("box").exists());
}

/// Whether a call was refused because the mount is read-only.
///
/// Generic over the success type so the two removals, which answer nothing, go
/// through the same check as the seven that answer with attributes.
fn read_only<T>(result: &Result<T, Error>) -> bool {
    matches!(
        result,
        Err(Error::Server {
            code: ErrorCode::ReadOnly,
            ..
        })
    )
}

#[tokio::test]
async fn a_read_only_export_refuses_every_mutation() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"x").unwrap();
    std::fs::create_dir(root.path().join("dir")).unwrap();
    let address = serve(root.path().to_path_buf(), true);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");
    assert!(!mount.info().writable);

    assert!(read_only(&mount.mkdir(b"new", 0o755).await));
    assert!(read_only(&mount.unlink(b"file").await));
    assert!(read_only(&mount.rmdir(b"dir").await));
    assert!(read_only(&mount.symlink(b"file", b"link").await));
    assert!(read_only(&mount.link(b"file", b"hard").await));
    assert!(read_only(&mount.chown(b"file", Some(0), None, true).await));
    assert!(read_only(
        &mount
            .set_times(b"file", TimeChange::Now, TimeChange::Now, true)
            .await
    ));
    assert!(read_only(
        &mount.set_permissions(b"file", 0o777, true).await
    ));
    assert!(read_only(
        &mount.rename(b"file", b"other", RenameMode::Replace).await
    ));

    // Nothing happened.
    assert!(root.path().join("file").exists());
    assert!(!root.path().join("new").exists());
}

#[tokio::test]
async fn a_staged_upload_publishes_atomically_and_verified() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("target.bin"), b"old contents").unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    // A repeating byte pattern, so a range written at the wrong offset shows up
    // as wrong content rather than as a plausible-looking file.
    let payload: Vec<u8> = (0..40_000_u32)
        .map(|byte| u8::try_from(byte % 251).unwrap_or(0))
        .collect();
    let mut stage = mount
        .stage(b"target.bin", payload.len() as u64, 0o644, b"")
        .await
        .expect("stage");
    assert_eq!(stage.staged_bytes(), 0);

    // Out of order and overlapping, which is what a resuming client does.
    stage.write(20_000, &payload[20_000..]).await.expect("tail");
    stage.write(0, &payload[..25_000]).await.expect("head");

    // The destination is untouched until the commit: that is the whole point
    // of staging, and a reader mid-upload must not see a half-written file.
    assert_eq!(
        std::fs::read(root.path().join("target.bin")).unwrap(),
        b"old contents"
    );

    let ranges = stage.ranges().await.expect("ranges");
    assert_eq!(ranges, vec![(0, payload.len() as u64)], "gaps: {ranges:?}");

    let published = stage
        .commit(*blake3::hash(&payload).as_bytes(), None, None)
        .await
        .expect("commit")
        .expect("published");
    assert_eq!(published.size, payload.len() as u64);
    assert_eq!(
        std::fs::read(root.path().join("target.bin")).unwrap(),
        payload
    );
}

#[tokio::test]
async fn a_stage_with_a_hole_is_refused_rather_than_published() {
    let root = tempdir().unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let mut stage = mount
        .stage(b"gappy.bin", 100, 0o644, b"")
        .await
        .expect("stage");
    // Byte 0..10 and 50..60: the middle was never sent. Publishing would give
    // a file of zeroes where the client believes its data is.
    stage.write(0, &[1_u8; 10]).await.expect("head");
    stage.write(50, &[2_u8; 10]).await.expect("tail");
    assert_eq!(
        stage.ranges().await.expect("ranges"),
        vec![(0, 10), (50, 60)]
    );

    let refused = stage.commit([0_u8; 32], None, None).await;
    assert!(
        matches!(
            refused,
            Err(Error::Server {
                code: ErrorCode::Invalid,
                ..
            })
        ),
        "{refused:?}"
    );
    assert!(!root.path().join("gappy.bin").exists());
}

#[tokio::test]
async fn a_stage_that_fails_its_digest_publishes_nothing() {
    let root = tempdir().unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let mut stage = mount
        .stage(b"wrong.bin", 4, 0o644, b"")
        .await
        .expect("stage");
    stage.write(0, b"data").await.expect("write");
    let refused = stage.commit([0xAB_u8; 32], None, None).await;
    assert!(
        matches!(
            refused,
            Err(Error::Server {
                code: ErrorCode::Integrity,
                ..
            })
        ),
        "{refused:?}"
    );
    assert!(!root.path().join("wrong.bin").exists());
}

#[tokio::test]
async fn a_stage_resumes_on_a_brand_new_connection() {
    // The property E4-S6 exists for, and the reason the range set lives in a
    // sidecar rather than in session state: this is a *different session*
    // against a *different server object*, with no session resume involved.
    let root = tempdir().unwrap();
    let payload: Vec<u8> = (0..30_000_u32)
        .map(|byte| u8::try_from(byte % 251).unwrap_or(0))
        .collect();

    let token = {
        let address = serve(root.path().to_path_buf(), false);
        let client = connect(&address).await;
        let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");
        let mut stage = mount
            .stage(b"resumed.bin", payload.len() as u64, 0o644, b"")
            .await
            .expect("stage");
        stage
            .write(0, &payload[..10_000])
            .await
            .expect("first half");
        stage.resume_token().to_vec()
        // The connection drops here.
    };

    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");
    let mut stage = mount
        .stage(b"resumed.bin", payload.len() as u64, 0o644, &token)
        .await
        .expect("resume");

    // It knows what it already has, so the client sends only the rest.
    assert_eq!(stage.staged_bytes(), 10_000);
    assert_eq!(stage.ranges().await.expect("ranges"), vec![(0, 10_000)]);

    stage.write(10_000, &payload[10_000..]).await.expect("rest");
    let published = stage
        .commit(*blake3::hash(&payload).as_bytes(), None, None)
        .await
        .expect("commit")
        .expect("published");
    assert_eq!(published.size, payload.len() as u64);
    assert_eq!(
        std::fs::read(root.path().join("resumed.bin")).unwrap(),
        payload
    );
}

#[tokio::test]
async fn compare_and_swap_refuses_a_destination_that_moved() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("doc.txt"), b"original").unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    // What an editor would have: the cookie as of when it read the file.
    let opened = mount.open(b"doc.txt", READ).await.expect("open");
    let cookie = opened.attrs().change_cookie;
    opened.close().await.expect("close");

    // Somebody else edits it. Sleep so the mtime actually differs — the cookie
    // hashes mtime, and a same-second write on a coarse clock would not move it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(root.path().join("doc.txt"), b"someone else's edit").unwrap();

    let mut stage = mount.stage(b"doc.txt", 8, 0o644, b"").await.expect("stage");
    stage.write(0, b"my edits").await.expect("write");
    let outcome = stage
        .commit(*blake3::hash(b"my edits").as_bytes(), Some(cookie), None)
        .await
        .expect("commit resolves");

    // A refusal is an answer, not an error, and it carries the destination as
    // it now stands so the caller can offer replace / keep both / diff.
    let current = outcome.expect_err("the guard should have refused");
    assert_ne!(current.change_cookie, cookie);
    assert_eq!(
        std::fs::read(root.path().join("doc.txt")).unwrap(),
        b"someone else's edit",
        "the CAS refusal must not have published"
    );
}

#[tokio::test]
async fn an_all_zero_cookie_means_create_and_only_create() {
    // The case v2's PublishRequest could not express: it answered Changed for
    // a destination that did not exist, so there was no create-only mode.
    let root = tempdir().unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let mut stage = mount.stage(b"new.txt", 5, 0o644, b"").await.expect("stage");
    stage.write(0, b"fresh").await.expect("write");
    stage
        .commit(*blake3::hash(b"fresh").as_bytes(), Some([0_u8; 16]), None)
        .await
        .expect("commit")
        .expect("created");
    assert_eq!(
        std::fs::read(root.path().join("new.txt")).unwrap(),
        b"fresh"
    );

    // Now it exists, so the same create-only commit must refuse.
    let mut second = mount.stage(b"new.txt", 6, 0o644, b"").await.expect("stage");
    second.write(0, b"second").await.expect("write");
    let refused = second
        .commit(*blake3::hash(b"second").as_bytes(), Some([0_u8; 16]), None)
        .await
        .expect("commit resolves");
    assert!(refused.is_err(), "create-only overwrote an existing file");
    assert_eq!(
        std::fs::read(root.path().join("new.txt")).unwrap(),
        b"fresh"
    );
}

#[tokio::test]
async fn a_guarded_write_refuses_a_file_that_changed() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("small.txt"), b"aaaa").unwrap();
    let address = serve(root.path().to_path_buf(), false);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let file = mount.open(b"small.txt", READ | WRITE).await.expect("open");
    let cookie = file.attrs().change_cookie;

    // A guarded write with the right cookie goes through...
    let written = file
        .write_if_unchanged(0, cookie, b"bbbb")
        .await
        .expect("guarded write");
    assert_eq!(written.bytes, 4);

    // ...and the same cookie is now stale, so a second one is refused.
    let refused = file.write_if_unchanged(0, cookie, b"cccc").await;
    assert!(
        matches!(
            refused,
            Err(Error::Server {
                code: ErrorCode::Changed,
                ..
            })
        ),
        "{refused:?}"
    );
    file.close().await.expect("close");
    assert_eq!(
        std::fs::read(root.path().join("small.txt")).unwrap(),
        b"bbbb"
    );
}

#[tokio::test]
async fn staging_is_refused_on_a_read_only_export() {
    let root = tempdir().unwrap();
    let address = serve(root.path().to_path_buf(), true);
    let client = connect(&address).await;
    let mount = client.mount(b"", Access::ReadWrite).await.expect("mount");

    let refused = mount.stage(b"nope.bin", 4, 0o644, b"").await;
    assert!(read_only(&refused));
    // And nothing was staged: no temporary file was created.
    let leftovers: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".xsync."))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
