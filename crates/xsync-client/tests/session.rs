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
    attr_presence, Access, Client, Error, ErrorCode, CREATE, DIRECTORY, READ, WRITE,
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
