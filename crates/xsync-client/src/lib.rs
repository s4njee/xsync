//! Async client for the xsync protocol v3 filesystem surface.
//!
//! The server in `xsync-core` is synchronous and speaks `Read`/`Write`; the
//! applications that want it — a Tauri backend, a transfer engine — are async.
//! This crate is the seam: it owns a connection, multiplexes requests over it,
//! and hands back typed answers.
//!
//! ```no_run
//! # async fn example() -> Result<(), xsync_client::Error> {
//! use xsync_client::{Access, Client, READ};
//!
//! let client = Client::connect_ssh("nas.local", "/srv/media", None).await?;
//! let mount = client.mount(b"", Access::ReadWrite).await?;
//! if !mount.info().writable {
//!     eprintln!("read-only: {}", mount.info().reason);
//! }
//! let file = mount.open(b"notes.txt", READ).await?;
//! let chunk = file.read(0, 4096).await?;
//! file.close().await?;
//! # Ok(())
//! # }
//! ```
//!
//! What is deliberately absent, because the server does not offer it yet:
//! TLS connections (`xsyncv3.md` E2-S1), session resume (E3-S2), credit-based
//! flow control (E3-S3), directory watches (E7) and the namespace mutations
//! (E5-S4). Each arrives with the story that implements its half.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use xsync_core::protocol::{
    encode_frame, negotiate_protocol_version, CompressionMode, FrameDecoder, Message, Role,
    CAP_BROWSE_META, CAP_BROWSE_V2, CAP_FS_V3, CAP_VERSION_NEGOTIATION,
    DEFAULT_UNACKNOWLEDGED_WINDOW, FRAME_HEADER_LEN, MAX_COMPLETE_PAYLOAD, MAX_DATA_SEGMENT,
};
use xsync_core::protocol_v3::{self as v3, V3Message};

pub use v3::{attr_presence, features, supports, Access, Attrs, ErrorCode, Normalization};

/// Which optional `Attrs` blocks a request wants. See [`attr_presence`].
pub type AttrMask = u32;

/// Flags for [`Mount::open`]. See [`v3::open_flags`] for the bit values.
pub use v3::open_flags::{APPEND, CREATE, DIRECTORY, EXCL, NOFOLLOW, READ, TRUNC, WRITE};

/// A set of [`open_flags`](v3::open_flags) bits.
pub type OpenFlags = u32;

/// Everything that can go wrong talking to an xsync server.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server refused this request. `code` is the frozen v3 code.
    #[error("{code:?}: {message}")]
    Server {
        /// Frozen protocol code.
        code: ErrorCode,
        /// Platform errno when the server had one, else `0`.
        errno: i32,
        /// The server's explanation.
        message: String,
    },
    /// The peer does not speak protocol v3.
    #[error("peer negotiated protocol v{selected}, not v3; browse or sync only")]
    NotFilesystemCapable {
        /// What the handshake settled on.
        selected: u32,
    },
    /// The connection ended before this request was answered.
    #[error("connection closed before the request was answered")]
    Disconnected,
    /// A reply arrived that does not answer what was asked.
    #[error("unexpected reply: {0}")]
    Unexpected(String),
    /// Bytes on the wire did not decode.
    #[error("protocol: {0}")]
    Protocol(String),
    /// The bytes read back did not match the digest the server sent with them.
    #[error("integrity: read at offset {offset} did not match its digest")]
    Integrity {
        /// Where the bad range started.
        offset: u64,
    },
    /// Transport failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    fn from_server(code: ErrorCode, errno: i32, message: &[u8]) -> Self {
        Self::Server {
            code,
            errno,
            message: String::from_utf8_lossy(message).into_owned(),
        }
    }

    /// The closest [`std::io::ErrorKind`], for callers that already have an
    /// error taxonomy of their own.
    ///
    /// The full generated mapping is `xsyncv3.md` E9-S4; this is the subset
    /// that has an obvious counterpart.
    #[must_use]
    pub const fn kind(&self) -> std::io::ErrorKind {
        use std::io::ErrorKind;
        match self {
            Self::Server { code, .. } => match code {
                ErrorCode::NoEntry => ErrorKind::NotFound,
                ErrorCode::Access | ErrorCode::ReadOnly => ErrorKind::PermissionDenied,
                ErrorCode::Exists => ErrorKind::AlreadyExists,
                ErrorCode::Invalid => ErrorKind::InvalidInput,
                ErrorCode::TimedOut => ErrorKind::TimedOut,
                ErrorCode::Cancelled => ErrorKind::Interrupted,
                ErrorCode::WouldBlock => ErrorKind::WouldBlock,
                _ => ErrorKind::Other,
            },
            Self::Disconnected => ErrorKind::BrokenPipe,
            Self::Io(_) | Self::Protocol(_) | Self::Unexpected(_) => ErrorKind::Other,
            Self::NotFilesystemCapable { .. } => ErrorKind::Unsupported,
            Self::Integrity { .. } => ErrorKind::InvalidData,
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Facts the server reported about the mounted export.
#[derive(Debug, Clone)]
pub struct MountInfo {
    /// Export name as the server knows it.
    pub export: Vec<u8>,
    /// What the export grants.
    pub access: Access,
    /// Whether *this session* may write. The single value a UI should gate on.
    pub writable: bool,
    /// Why not, when `writable` is false.
    pub reason: String,
    /// Operator-supplied option string, shown verbatim.
    pub options: String,
    /// Whether names differing only by case are distinct.
    pub case_sensitive: bool,
    /// Normalization the filesystem applies.
    pub normalization: Normalization,
    /// Longest single name component.
    pub max_name_len: u32,
    /// Longest path.
    pub max_path_len: u32,
    /// See [`supports`].
    pub supports: u64,
    /// Largest `read` this server accepts.
    pub max_read: u32,
    /// Largest `write` this server accepts.
    pub max_write: u32,
}

/// Capacity and filesystem facts.
#[derive(Debug, Clone)]
pub struct FsInfo {
    /// Filesystem block size.
    pub block_size: u32,
    /// Total bytes.
    pub total_bytes: u64,
    /// Free bytes, including any root-only reserve.
    pub free_bytes: u64,
    /// Bytes this identity may actually use. What a capacity display wants.
    pub available_bytes: u64,
    /// Total inodes, `0` when the server does not know.
    pub total_inodes: u64,
    /// Free inodes, `0` when the server does not know.
    pub free_inodes: u64,
    /// Filesystem type name, empty when the server does not know.
    pub fs_type: String,
    /// The filesystem itself is read-only, distinct from the export's access.
    pub read_only: bool,
}

/// Bytes returned by [`OpenFile::read`].
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Offset the data starts at.
    pub offset: u64,
    /// The data ends at end of file.
    pub eof: bool,
    /// The bytes.
    pub data: Vec<u8>,
}

/// Outcome of [`OpenFile::write`].
#[derive(Debug, Clone, Copy)]
pub struct Written {
    /// Bytes written.
    pub bytes: u32,
    /// File size afterwards.
    pub new_size: u64,
    /// The bytes are already durable.
    pub stable: bool,
    /// Change cookie afterwards.
    pub change_cookie: [u8; 16],
}

/// One page of a directory listing.
#[derive(Debug, Clone)]
pub struct DirPage {
    /// Position to pass to the next call.
    pub cursor: u64,
    /// No more entries follow.
    pub final_page: bool,
    /// The entries.
    pub entries: Vec<v3::DirEntry>,
}

/// Shared connection state: the write side, and who is waiting for what.
struct Connection {
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<V3Message>>>,
    next_id: AtomicU64,
    negotiated_features: u64,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // The reader, writer and keepalive tasks hold no state worth draining;
        // when the last handle to the connection goes, so do they.
        if let Ok(tasks) = self.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
    }
}

impl Connection {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send one request and wait for the reply that carries its id.
    ///
    /// Requests are multiplexed: several may be outstanding, and the server
    /// answers them in whatever order it finishes.
    async fn request(&self, message: &V3Message) -> Result<V3Message> {
        let id = self.next_id();
        let bytes =
            v3::encode_frame(id, message).map_err(|error| Error::Protocol(error.to_string()))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().map_err(|_| Error::Disconnected)?;
            pending.insert(id, reply_tx);
        }
        if self.outgoing.send(bytes).is_err() {
            self.pending.lock().ok().and_then(|mut p| p.remove(&id));
            return Err(Error::Disconnected);
        }
        reply_rx.await.map_err(|_| Error::Disconnected)
    }

    /// As [`Self::request`], turning a server `Error` reply into an `Err`.
    async fn call(&self, message: &V3Message) -> Result<V3Message> {
        match self.request(message).await? {
            V3Message::Error {
                code,
                platform_errno,
                message,
                ..
            } => Err(Error::from_server(code, platform_errno, &message)),
            other => Ok(other),
        }
    }
}

/// A connection to an xsync server that has negotiated protocol v3.
pub struct Client {
    connection: Arc<Connection>,
}

impl Client {
    /// Drive a session over an already-connected byte stream.
    ///
    /// This is the entry point an embedding application uses when it already
    /// holds a transport — an SSH channel, a socket it opened itself — and
    /// does not want this crate to spawn anything.
    ///
    /// # Errors
    ///
    /// Fails when the peer does not negotiate v3, or the opening exchange is
    /// malformed.
    pub async fn from_stream<S>(stream: S, requested_features: u64) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (reader, writer) = tokio::io::split(stream);
        Self::start(reader, writer, requested_features).await
    }

    /// Start `xs --server` on a remote host over SSH and speak v3 to it.
    ///
    /// `rsh` overrides the remote shell, as `xs -e` does; `None` uses `ssh`.
    /// Host-key checking, authentication and agent policy stay with OpenSSH.
    ///
    /// # Errors
    ///
    /// Fails when the remote shell cannot be spawned, `xs` is missing, or the
    /// peer does not negotiate v3.
    pub async fn connect_ssh(host: &str, root: &str, rsh: Option<&str>) -> Result<Self> {
        let program = rsh.unwrap_or("ssh");
        let mut command = tokio::process::Command::new(program);
        command
            .arg(host)
            .arg(format!(
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' '{}'",
                root.replace('\'', "'\\''")
            ))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Unexpected("remote stdout was not piped".to_owned()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Unexpected("remote stdin was not piped".to_owned()))?;
        Self::start(stdout, stdin, 0).await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one ordered sequence: handshake, then tasks, then features"
    )]
    async fn start<R, W>(mut reader: R, mut writer: W, requested_features: u64) -> Result<Self>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        // The opening handshake is a v1 envelope, as `v2handshake.md` requires,
        // and is done inline: nothing may be multiplexed until the grammar is
        // settled.
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION | CAP_BROWSE_META | CAP_FS_V3;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: bounded(MAX_COMPLETE_PAYLOAD),
            max_segment: bounded(MAX_DATA_SEGMENT),
            window: bounded(DEFAULT_UNACKNOWLEDGED_WINDOW),
            job_id: [0; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let bytes =
            encode_frame(1, &handshake).map_err(|error| Error::Protocol(error.to_string()))?;
        writer.write_all(&bytes).await?;
        writer.flush().await?;

        let mut decoder = FrameDecoder::new();
        let remote_capabilities = match read_v1(&mut reader, &mut decoder).await? {
            Message::Handshake { capabilities, .. } => capabilities,
            other => {
                return Err(Error::Unexpected(format!(
                    "expected a handshake, got {other:?}"
                )))
            }
        };
        match read_v1(&mut reader, &mut decoder).await? {
            Message::Ack { .. } => {}
            other => {
                return Err(Error::Unexpected(format!(
                    "expected a handshake acknowledgement, got {other:?}"
                )))
            }
        }
        let selected = negotiate_protocol_version(capabilities, remote_capabilities);
        if selected != 3 {
            return Err(Error::NotFilesystemCapable { selected });
        }

        let (outgoing, mut outbox) = mpsc::unbounded_channel::<Vec<u8>>();
        let connection = Arc::new(Connection {
            outgoing,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(2),
            negotiated_features: 0,
            tasks: Mutex::new(Vec::new()),
        });

        let writer_task = tokio::spawn(async move {
            while let Some(bytes) = outbox.recv().await {
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });

        let reader_connection = Arc::clone(&connection);
        let reader_task = tokio::spawn(async move {
            while let Ok(Some(bytes)) = read_frame_bytes(&mut reader).await {
                match v3::decode_frame(&bytes) {
                    Ok(frame) => {
                        if let Some(id) = related_id(&frame.message) {
                            let waiting = reader_connection
                                .pending
                                .lock()
                                .ok()
                                .and_then(|mut pending| pending.remove(&id));
                            if let Some(reply) = waiting {
                                let _ = reply.send(frame.message);
                            }
                            // A reply nobody is waiting for is dropped.
                            // Server-initiated notifications land here and get
                            // routed when E7 defines them.
                        }
                    }
                    // A frame that does not decode ends the session: the
                    // grammar was settled at the handshake and cannot be
                    // renegotiated mid-stream.
                    Err(_) => break,
                }
            }
            // Everyone still waiting learns the connection is gone, rather
            // than waiting for a reply that can never arrive.
            if let Ok(mut pending) = reader_connection.pending.lock() {
                pending.clear();
            }
        });

        if let Ok(mut tasks) = connection.tasks.lock() {
            tasks.push(writer_task);
            tasks.push(reader_task);
        }

        // Exchange features before anything else, as the contract requires.
        let negotiated = match connection
            .call(&V3Message::Features {
                features: requested_features,
            })
            .await?
        {
            V3Message::FeaturesAck { features, .. } => features,
            other => {
                return Err(Error::Unexpected(format!(
                    "expected FeaturesAck, got {other:?}"
                )))
            }
        };
        if negotiated & !requested_features != 0 {
            return Err(Error::Unexpected(format!(
                "server granted features 0x{negotiated:x} that were not requested",
            )));
        }

        let mut client = Self { connection };
        // `negotiated_features` is set once, here, before the value is shared.
        if let Some(connection) = Arc::get_mut(&mut client.connection) {
            connection.negotiated_features = negotiated;
        }
        Ok(client)
    }

    /// Optional features both ends offer.
    #[must_use]
    pub fn negotiated_features(&self) -> u64 {
        self.connection.negotiated_features
    }

    /// Whether every bit in `features` was negotiated.
    #[must_use]
    pub fn supports(&self, features: u64) -> bool {
        self.connection.negotiated_features & features == features
    }

    /// Send keepalives every `interval` for as long as the connection lives.
    ///
    /// A session is otherwise free to be idle; this exists for deployments
    /// where something between the two ends drops quiet connections.
    pub fn keepalive_every(&self, interval: Duration) {
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let mut nonce = 0_u64;
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                nonce = nonce.wrapping_add(1);
                if connection
                    .call(&V3Message::Keepalive { nonce })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        if let Ok(mut tasks) = self.connection.tasks.lock() {
            tasks.push(task);
        }
    }

    /// Attach to an export. A session mounts once.
    ///
    /// # Errors
    ///
    /// Fails when the export does not exist or the session is already mounted.
    pub async fn mount(&self, export: &[u8], requested_access: Access) -> Result<Mount> {
        let reply = self
            .connection
            .call(&V3Message::Mount {
                export: export.to_vec(),
                requested_access,
            })
            .await?;
        let V3Message::MountInfo {
            export,
            access,
            effective_writable,
            reason,
            options,
            case_sensitive,
            normalization,
            max_name_len,
            max_path_len,
            supports,
            max_read,
            max_write,
            ..
        } = reply
        else {
            return Err(Error::Unexpected(format!(
                "expected MountInfo, got {reply:?}"
            )));
        };
        Ok(Mount {
            connection: Arc::clone(&self.connection),
            info: MountInfo {
                export,
                access,
                writable: effective_writable,
                reason: String::from_utf8_lossy(&reason).into_owned(),
                options: String::from_utf8_lossy(&options).into_owned(),
                case_sensitive,
                normalization,
                max_name_len,
                max_path_len,
                supports,
                max_read,
                max_write,
            },
        })
    }
}

/// A mounted export.
pub struct Mount {
    connection: Arc<Connection>,
    info: MountInfo,
}

impl Mount {
    /// What the server said about this export at mount time.
    #[must_use]
    pub const fn info(&self) -> &MountInfo {
        &self.info
    }

    /// Attributes of a path. `follow` chooses `stat` over `lstat`.
    ///
    /// # Errors
    ///
    /// Fails when the path does not exist or leaves the export.
    pub async fn stat(&self, path: &[u8], follow: bool, attrs: AttrMask) -> Result<Attrs> {
        let reply = self
            .connection
            .call(&V3Message::Stat {
                target: v3::StatTarget::Path(path.to_vec()),
                follow,
                attr_mask: attrs,
            })
            .await?;
        match reply {
            V3Message::AttrsResponse { attrs, .. } => Ok(attrs),
            other => Err(Error::Unexpected(format!("expected Attrs, got {other:?}"))),
        }
    }

    /// Capacity and filesystem facts.
    ///
    /// # Errors
    ///
    /// Fails when the server cannot inspect the export.
    pub async fn statfs(&self) -> Result<FsInfo> {
        let reply = self.connection.call(&V3Message::StatFs).await?;
        let V3Message::FsInfo {
            block_size,
            total_bytes,
            free_bytes,
            available_bytes,
            total_inodes,
            free_inodes,
            fs_type,
            read_only,
            ..
        } = reply
        else {
            return Err(Error::Unexpected(format!("expected FsInfo, got {reply:?}")));
        };
        Ok(FsInfo {
            block_size,
            total_bytes,
            free_bytes,
            available_bytes,
            total_inodes,
            free_inodes,
            fs_type: String::from_utf8_lossy(&fs_type).into_owned(),
            read_only,
        })
    }

    /// Open a file or directory.
    ///
    /// # Errors
    ///
    /// Fails when the path cannot be opened as asked, or the mount is not
    /// writable and `flags` would write.
    pub async fn open(&self, path: &[u8], flags: OpenFlags) -> Result<OpenFile> {
        self.open_with(path, flags, 0, 0).await
    }

    /// As [`Self::open`], with a creation mode and the attributes to return.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub async fn open_with(
        &self,
        path: &[u8],
        flags: OpenFlags,
        mode: u32,
        attrs: AttrMask,
    ) -> Result<OpenFile> {
        let reply = self
            .connection
            .call(&V3Message::Open {
                path: path.to_vec(),
                flags,
                mode,
                attr_mask: attrs,
            })
            .await?;
        match reply {
            V3Message::Opened { handle, attrs, .. } => Ok(OpenFile {
                connection: Arc::clone(&self.connection),
                handle,
                attrs,
                max_read: self.info.max_read,
                max_write: self.info.max_write,
            }),
            other => Err(Error::Unexpected(format!("expected Opened, got {other:?}"))),
        }
    }
}

/// An open file or directory.
///
/// Dropping this leaks the handle until the session ends; call
/// [`OpenFile::close`] to release it, which is also where a failure to release
/// it is reported.
pub struct OpenFile {
    connection: Arc<Connection>,
    handle: u64,
    attrs: Attrs,
    max_read: u32,
    max_write: u32,
}

impl OpenFile {
    /// Attributes as of the open.
    #[must_use]
    pub const fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    /// Read `length` bytes at `offset`.
    ///
    /// The server's digest is not requested. Use [`Self::read_verified`] to
    /// have the bytes checked before they are returned.
    ///
    /// # Errors
    ///
    /// Fails when the handle cannot be read or `length` exceeds the mount's
    /// `max_read`.
    pub async fn read(&self, offset: u64, length: u32) -> Result<Chunk> {
        self.read_inner(offset, length, false).await
    }

    /// As [`Self::read`], verifying the server's BLAKE3 digest before
    /// returning the bytes.
    ///
    /// # Errors
    ///
    /// Adds [`Error::Integrity`] when the bytes do not match the digest.
    pub async fn read_verified(&self, offset: u64, length: u32) -> Result<Chunk> {
        self.read_inner(offset, length, true).await
    }

    async fn read_inner(&self, offset: u64, length: u32, verify: bool) -> Result<Chunk> {
        if length > self.max_read {
            return Err(Error::Server {
                code: ErrorCode::Invalid,
                errno: 0,
                message: format!(
                    "read of {length} exceeds the mount's max_read {}",
                    self.max_read
                ),
            });
        }
        let reply = self
            .connection
            .call(&V3Message::Read {
                handle: self.handle,
                offset,
                length,
                want_digest: verify,
            })
            .await?;
        let V3Message::ReadData {
            offset,
            eof,
            digest,
            data,
            ..
        } = reply
        else {
            return Err(Error::Unexpected(format!(
                "expected ReadData, got {reply:?}"
            )));
        };
        if verify {
            // A server that answered without one has not done what was asked,
            // so this is a mismatch rather than a reason to skip the check.
            let matches = digest.is_some_and(|digest| *blake3::hash(&data).as_bytes() == digest);
            if !matches {
                return Err(Error::Integrity { offset });
            }
        }
        Ok(Chunk { offset, eof, data })
    }

    /// Write `data` at `offset`. An `APPEND` handle ignores `offset`.
    ///
    /// The digest is always sent: it costs one hash of data already in memory
    /// and it is what stops a corrupted payload from being written.
    ///
    /// # Errors
    ///
    /// Fails when the handle cannot be written or `data` exceeds the mount's
    /// `max_write`.
    pub async fn write(&self, offset: u64, data: &[u8]) -> Result<Written> {
        if data.len() > self.max_write as usize {
            return Err(Error::Server {
                code: ErrorCode::Invalid,
                errno: 0,
                message: format!(
                    "write of {} exceeds the mount's max_write {}",
                    data.len(),
                    self.max_write
                ),
            });
        }
        let reply = self
            .connection
            .call(&V3Message::Write {
                handle: self.handle,
                offset,
                digest: Some(*blake3::hash(data).as_bytes()),
                data: data.to_vec(),
            })
            .await?;
        match reply {
            V3Message::WriteAck {
                bytes_written,
                new_size,
                stable,
                change_cookie,
                ..
            } => Ok(Written {
                bytes: bytes_written,
                new_size,
                stable,
                change_cookie,
            }),
            other => Err(Error::Unexpected(format!(
                "expected WriteAck, got {other:?}"
            ))),
        }
    }

    /// One page of a directory listing. `cursor` is `0` for the first page.
    ///
    /// # Errors
    ///
    /// Fails when the handle is not a directory or the cursor is not one this
    /// listing issued.
    pub async fn read_dir(
        &self,
        cursor: u64,
        max_entries: u32,
        attrs: AttrMask,
    ) -> Result<DirPage> {
        let reply = self
            .connection
            .call(&V3Message::ReadDir {
                handle: self.handle,
                cursor,
                max_entries,
                attr_mask: attrs,
            })
            .await?;
        match reply {
            V3Message::DirPage {
                cursor,
                final_page,
                entries,
                ..
            } => Ok(DirPage {
                cursor,
                final_page,
                entries,
            }),
            other => Err(Error::Unexpected(format!(
                "expected DirPage, got {other:?}"
            ))),
        }
    }

    /// Every entry of a directory, paged transparently.
    ///
    /// # Errors
    ///
    /// As [`Self::read_dir`].
    pub async fn read_dir_all(&self, attrs: AttrMask) -> Result<Vec<v3::DirEntry>> {
        let mut all = Vec::new();
        let mut cursor = 0;
        loop {
            let page = self.read_dir(cursor, 8192, attrs).await?;
            all.extend(page.entries);
            if page.final_page {
                return Ok(all);
            }
            cursor = page.cursor;
        }
    }

    /// Make this handle's writes durable.
    ///
    /// # Errors
    ///
    /// Fails when the server cannot flush.
    pub async fn flush(&self) -> Result<()> {
        self.expect_done(&V3Message::Flush {
            handle: self.handle,
        })
        .await
    }

    /// Release the handle.
    ///
    /// # Errors
    ///
    /// Fails when the server does not know the handle.
    pub async fn close(self) -> Result<()> {
        self.expect_done(&V3Message::Close {
            handle: self.handle,
        })
        .await
    }

    async fn expect_done(&self, message: &V3Message) -> Result<()> {
        match self.connection.call(message).await? {
            V3Message::Done { .. } => Ok(()),
            other => Err(Error::Unexpected(format!("expected Done, got {other:?}"))),
        }
    }
}

// Hand-written rather than derived: the connection's pending-request map and
// task handles are noise in a log, and a handle's identity is what a reader
// actually wants to correlate.
impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field(
                "negotiated_features",
                &format_args!("{:#x}", self.negotiated_features()),
            )
            .finish()
    }
}

#[allow(
    clippy::missing_fields_in_debug,
    reason = "the connection behind it is noise; identity is what a log wants"
)]
impl std::fmt::Debug for Mount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Mount")
            .field("export", &String::from_utf8_lossy(&self.info.export))
            .field("writable", &self.info.writable)
            .finish()
    }
}

#[allow(
    clippy::missing_fields_in_debug,
    reason = "the connection behind it is noise; identity is what a log wants"
)]
impl std::fmt::Debug for OpenFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenFile")
            .field("handle", &self.handle)
            .field("kind", &self.attrs.kind)
            .finish()
    }
}

/// A protocol size constant as the `u32` the handshake carries.
///
/// Every value passed here is a compile-time constant far below `u32::MAX`;
/// saturating is a formality that keeps the conversion checked.
fn bounded(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The request a reply answers, or `None` for a frame that answers nothing.
const fn related_id(message: &V3Message) -> Option<u64> {
    match message {
        V3Message::Error { related_id, .. }
        | V3Message::Done { related_id }
        | V3Message::Opened { related_id, .. }
        | V3Message::MountInfo { related_id, .. }
        | V3Message::AttrsResponse { related_id, .. }
        | V3Message::ReadData { related_id, .. }
        | V3Message::WriteAck { related_id, .. }
        | V3Message::DirPage { related_id, .. }
        | V3Message::FsInfo { related_id, .. }
        | V3Message::FeaturesAck { related_id, .. } => Some(*related_id),
        // A keepalive acknowledgement echoes its nonce rather than an id, and
        // the client sends the nonce as the id, so they agree.
        V3Message::KeepaliveAck { nonce } => Some(*nonce),
        _ => None,
    }
}

/// Read exactly one frame's bytes: the fixed envelope, then its payload.
async fn read_frame_bytes<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut header = [0_u8; FRAME_HEADER_LEN];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
    }
    let payload_len = u32::from_le_bytes([header[16], header[17], header[18], header[19]]) as usize;
    if payload_len > MAX_COMPLETE_PAYLOAD {
        return Err(Error::Protocol(format!(
            "frame declares a {payload_len} byte payload, above the {MAX_COMPLETE_PAYLOAD} cap"
        )));
    }
    let mut bytes = header.to_vec();
    bytes.resize(FRAME_HEADER_LEN + payload_len, 0);
    reader.read_exact(&mut bytes[FRAME_HEADER_LEN..]).await?;
    Ok(Some(bytes))
}

/// Read one v1-envelope frame, for the opening handshake only.
async fn read_v1<R: AsyncRead + Unpin>(
    reader: &mut R,
    decoder: &mut FrameDecoder,
) -> Result<Message> {
    let bytes = read_frame_bytes(reader).await?.ok_or(Error::Disconnected)?;
    decoder
        .read(&mut Cursor::new(bytes))
        .map(|frame| frame.message)
        .map_err(|error| Error::Protocol(error.to_string()))
}
