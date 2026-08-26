//! Server mode and protocol session drivers for remote synchronization.
//!
//! Story 3.2: `xsync --server` speaking the v1 wire protocol over stdin/stdout.
//! ALL diagnostics and logs go exclusively to stderr; stdout is protocol-only.

#![allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::manual_flatten,
    clippy::assigning_clones,
    clippy::identity_op,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::redundant_closure_for_method_calls
)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use filetime::{set_file_mtime, FileTime};
use thiserror::Error;

use crate::hash_cache::{HashCache, HashFingerprint};
use crate::local::{LocalEvent, LocalSyncOptions, LocalSyncReport, TransferMethod};
use crate::path::WirePath;
use crate::planner::{
    try_plan, try_plan_with_fingerprint, DestinationIndex, IndexConfig, PlannerError,
};
use crate::protocol::{
    common_capabilities, encode_frame, encode_frame_with_compression, negotiate_compression,
    negotiate_protocol_version, ByteRange, CompressionMode, EntryRecord, FrameDecoder, Message,
    MetadataOperation, ProtocolError, Role, CAP_BROWSE_V2, CAP_VERSION_NEGOTIATION, CAP_ZSTD,
    DEFAULT_UNACKNOWLEDGED_WINDOW, MAX_COLLECTION_COUNT, MAX_COMPLETE_PAYLOAD, MAX_DATA_SEGMENT,
};
use crate::protocol_v2::{self, V2CodecError, V2Frame, V2Message};
use crate::protocol_v2::{BrowseEntry, MutationStatus};
use crate::scanner::{
    fingerprint_from_metadata, permission_mode, scan, EntryKind as ScanEntryKind, FileEntry,
    FileIdentity, ScanError, SourceFingerprint,
};
use crate::sink::{Sink, SinkError, SymlinkTargetKind};
use crate::source::{SourceReadError, SourceReader};
use crate::strategy::{BATCH_TARGET_SIZE, MAX_BATCH_FILES, SMALL_FILE_LIMIT};

/// Emit server lifecycle diagnostics without contaminating the binary
/// protocol on stdout. Stderr is intentionally safe for SSH diagnostics.
fn server_log(message: impl std::fmt::Display) {
    eprintln!("[xsync server] {message}");
}

/// Errors produced by server operations and remote protocol sessions.
#[derive(Debug, Error)]
pub enum ServerError {
    /// A protocol error occurred during encoding, decoding, or negotiation.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    /// An underlying filesystem or I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A scanner error occurred.
    #[error("scanner error: {0}")]
    Scan(#[from] ScanError),
    /// A planner error occurred.
    #[error("planner error: {0}")]
    Planner(#[from] PlannerError),
    /// A sink error occurred.
    #[error("sink error: {0}")]
    Sink(#[from] SinkError),
    /// A source read error occurred.
    #[error("source read error: {0}")]
    SourceRead(#[from] SourceReadError),
    /// A resume checkpoint journal error occurred.
    #[error("resume journal error: {0}")]
    Journal(#[from] crate::journal::JournalError),
    /// The destination path is invalid or attempts to escape the root.
    #[error("invalid destination path '{0}'")]
    InvalidPath(String),
    /// A pre-existing symlink or escape attempt was detected.
    #[error("symlink traversal escape detected for path '{0}'")]
    SymlinkEscape(String),
    /// Duplicate normalized destination path received in the same session.
    #[error("duplicate destination path '{0}'")]
    DuplicatePath(String),
    /// Unexpected message received for the current session state.
    #[error("unexpected protocol message: {0}")]
    UnexpectedMessage(String),
    /// The remote server reported an error.
    #[error("remote error (code {code}): {message}")]
    RemoteError {
        /// Remote error code.
        code: u16,
        /// Remote diagnostic message.
        message: String,
    },
    /// A browse-session frame was malformed or used the wrong message shape.
    #[error("v2 session error: {0}")]
    Browse(#[from] V2CodecError),
    /// The remote shell positively reported that xsync is unavailable.
    #[error("xs not found on remote host — install it or check PATH")]
    MissingRemoteXsync,
    /// The remote was reached but its shell could not parse the server command.
    ///
    /// Raised for stock Windows OpenSSH, which hands the command to `cmd.exe`.
    /// Internal: callers retry once with the Windows command form.
    #[error("remote shell could not run the xsync server command")]
    RemoteShellMismatch,
    /// A transport error occurred on the named stream.
    #[error("transport error on stream {stream}: {message}")]
    Transport {
        /// Stream identifier.
        stream: usize,
        /// Diagnostic message.
        message: String,
    },
    /// The persistent browse peer closed its stream cleanly.
    #[error("browse peer disconnected")]
    PeerDisconnected,
}

/// Result of probing a peer before opening a browse session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeStatus {
    /// The peer supports the current browse protocol.
    Ready,
    /// The peer answered, but is older and cannot provide browse v2.
    OlderPeer { selected_version: u32 },
    /// The peer answered with an unusable role or protocol state.
    Unusable { detail: String },
}

impl ProbeStatus {
    /// Action a caller can present for this probe result.
    #[must_use]
    pub const fn action(&self) -> &'static str {
        match self {
            Self::Ready => "open the browse session",
            Self::OlderPeer { .. } => "upgrade the remote xsync binary before browsing",
            Self::Unusable { .. } => "check the remote xsync installation and protocol settings",
        }
    }
}

/// Handshake facts exposed by a pre-session probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProbe {
    /// Local application version.
    pub local_version: &'static str,
    /// Remote application version when the peer exposes one.
    pub remote_version: Option<String>,
    /// Capabilities advertised by the remote peer.
    pub remote_capabilities: u32,
    /// Capability intersection computed during the probe.
    pub common_capabilities: u32,
    /// Selected wire version.
    pub selected_version: u32,
    /// Typed probe outcome.
    pub status: ProbeStatus,
}

/// A successful probe with the connection still available for reuse.
pub struct ProbedConnection<R, W> {
    /// Probe facts.
    pub probe: AgentProbe,
    /// Already-handshaken reader.
    pub reader: R,
    /// Already-handshaken writer.
    pub writer: W,
}

impl<R: Read, W: Write> ProbedConnection<R, W> {
    /// Continue directly into a browse session without another connection or handshake.
    pub fn into_browse_session(self) -> Result<BrowseSession<R, W>, ServerError> {
        if !matches!(self.probe.status, ProbeStatus::Ready) {
            return Err(ServerError::UnexpectedMessage(format!(
                "cannot open browse session after probe: {:?}",
                self.probe.status
            )));
        }
        Ok(BrowseSession {
            reader: self.reader,
            writer: self.writer,
            next_message_id: 2,
            remote_capabilities: self.probe.remote_capabilities,
            common_capabilities: self.probe.common_capabilities,
        })
    }
}

/// Perform the v1-compatible opening handshake without starting a session operation.
pub fn probe_session<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    job_id: [u8; 16],
) -> Result<ProbedConnection<R, W>, ServerError> {
    let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
    let handshake = Message::Handshake {
        role: Role::Session,
        capabilities,
        max_payload: MAX_COMPLETE_PAYLOAD as u32,
        max_segment: MAX_DATA_SEGMENT as u32,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        job_id,
        compression: CompressionMode::None,
        compression_level: 3,
    };
    writer.write_all(&encode_frame(1, &handshake)?)?;
    writer.flush()?;
    let mut decoder = FrameDecoder::new();
    let server_handshake = decoder.read(&mut reader)?;
    let remote_capabilities = match server_handshake.message {
        Message::Handshake {
            role: Role::Session,
            capabilities,
            ..
        } => capabilities,
        other => {
            return Ok(ProbedConnection {
                probe: AgentProbe {
                    local_version: crate::version(),
                    remote_version: None,
                    remote_capabilities: 0,
                    common_capabilities: 0,
                    selected_version: 0,
                    status: ProbeStatus::Unusable {
                        detail: format!("expected Session handshake, got {other:?}"),
                    },
                },
                reader,
                writer,
            })
        }
    };
    let ack = decoder.read(&mut reader)?;
    if !matches!(ack.message, Message::Ack { .. }) {
        return Ok(ProbedConnection {
            probe: AgentProbe {
                local_version: crate::version(),
                remote_version: None,
                remote_capabilities,
                common_capabilities: common_capabilities(capabilities, remote_capabilities),
                selected_version: 0,
                status: ProbeStatus::Unusable {
                    detail: format!("expected handshake acknowledgement, got {:?}", ack.message),
                },
            },
            reader,
            writer,
        });
    }
    let selected_version = negotiate_protocol_version(capabilities, remote_capabilities);
    let status = if selected_version == 2 {
        ProbeStatus::Ready
    } else {
        ProbeStatus::OlderPeer { selected_version }
    };
    Ok(ProbedConnection {
        probe: AgentProbe {
            local_version: crate::version(),
            remote_version: None,
            remote_capabilities,
            common_capabilities: common_capabilities(capabilities, remote_capabilities),
            selected_version,
            status,
        },
        reader,
        writer,
    })
}

/// Client-side driver for a persistent v2 browse session.
///
/// The constructor performs the v1-compatible opening handshake and commits
/// to v2 before returning. Requests and responses thereafter use only the v2
/// browse envelope; a malformed frame is returned as an error and is never
/// retried as v1.
pub struct BrowseSession<R, W> {
    reader: R,
    writer: W,
    next_message_id: u64,
    remote_capabilities: u32,
    common_capabilities: u32,
}

/// Metadata returned after a single-file fetch is verified locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchedFile {
    /// Number of bytes published.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
    /// Source filesystem identity at the stable read.
    pub identity: FileIdentity,
    /// BLAKE3 digest of the fetched bytes.
    pub digest: [u8; 32],
}

impl<R: Read, W: Write> BrowseSession<R, W> {
    /// Establish a browse session over an already-connected stream pair.
    ///
    /// # Errors
    /// Returns [`ServerError`] when the peer does not negotiate v2 or the
    /// opening handshake is malformed.
    pub fn connect(reader: R, writer: W, job_id: [u8; 16]) -> Result<Self, ServerError> {
        probe_session(reader, writer, job_id)?.into_browse_session()
    }

    /// Capabilities advertised by the remote browse peer.
    #[must_use]
    pub const fn remote_capabilities(&self) -> u32 {
        self.remote_capabilities
    }

    /// Known capabilities shared by both endpoints.
    #[must_use]
    pub const fn common_capabilities(&self) -> u32 {
        self.common_capabilities
    }

    /// Return the underlying stream pair, consuming the session driver.
    #[must_use]
    pub fn into_parts(self) -> (R, W) {
        (self.reader, self.writer)
    }

    /// Send one v2 request and return its envelope ID.
    ///
    /// # Errors
    /// Returns [`ServerError`] when encoding or writing fails.
    pub fn send(&mut self, message: &V2Message) -> Result<u64, ServerError> {
        let message_id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        self.writer
            .write_all(&protocol_v2::encode_frame(message_id, message)?)?;
        self.writer.flush()?;
        Ok(message_id)
    }

    /// Receive the next v2 response in stream order.
    ///
    /// # Errors
    /// Returns [`ServerError`] when decoding or reading fails.
    pub fn receive(&mut self) -> Result<V2Frame, ServerError> {
        protocol_v2::read_frame(&mut self.reader)?.ok_or(ServerError::PeerDisconnected)
    }

    /// Send one request and wait for its next response.
    ///
    /// # Errors
    /// Returns [`ServerError`] when the request or response cannot be sent or
    /// decoded.
    pub fn request(&mut self, message: &V2Message) -> Result<V2Frame, ServerError> {
        self.send(message)?;
        self.receive()
    }

    /// Request one bounded directory page.
    ///
    /// The returned token is zero when the page is final; otherwise pass it
    /// back unchanged to retrieve the next page.
    ///
    /// # Errors
    /// Returns [`ServerError`] when the peer returns a non-list response or
    /// the request cannot be sent.
    pub fn list_page(
        &mut self,
        path: Vec<u8>,
        page_token: u64,
        page_size: u32,
    ) -> Result<(u64, bool, Vec<BrowseEntry>), ServerError> {
        let response = self.request(&V2Message::ListRequest {
            path,
            page_token,
            page_size,
        })?;
        match response.message {
            V2Message::ListPage {
                page_token,
                final_page,
                entries,
                ..
            } => Ok((page_token, final_page, entries)),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected ListPage, got {other:?}"
            ))),
        }
    }

    /// Request metadata for one path without following a final symlink.
    ///
    /// # Errors
    /// Returns [`ServerError`] when the peer returns a non-stat response or
    /// the request cannot be sent.
    pub fn stat(&mut self, path: Vec<u8>, include_digest: bool) -> Result<V2Message, ServerError> {
        let response = self.request(&V2Message::StatRequest {
            path,
            include_digest,
        })?;
        match response.message {
            V2Message::StatResponse { .. } => Ok(response.message),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected StatResponse, got {other:?}"
            ))),
        }
    }

    /// Cancel a request and require its cancellation acknowledgement.
    ///
    /// Cancelling an already completed or unknown request is a no-op
    /// acknowledgement, not a session error.
    pub fn cancel(&mut self, related_id: u64) -> Result<(), ServerError> {
        let response = self.request(&V2Message::CancelRequest { related_id })?;
        match response.message {
            V2Message::BrowseError {
                related_id: response_id,
                code: 1,
                ..
            } if response_id == related_id => Ok(()),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected cancellation acknowledgement, got {other:?}"
            ))),
        }
    }

    /// Send a keepalive and require the matching acknowledgement.
    ///
    /// # Errors
    /// Returns [`ServerError`] when the peer closes, sends another response,
    /// or acknowledges a different nonce.
    pub fn keepalive(&mut self, nonce: u64) -> Result<(), ServerError> {
        let response = self.request(&V2Message::Keepalive { nonce })?;
        if response.message == (V2Message::KeepaliveAck { nonce }) {
            Ok(())
        } else {
            Err(ServerError::UnexpectedMessage(format!(
                "expected KeepaliveAck, got {:?}",
                response.message
            )))
        }
    }

    /// Rename a remote path without replacing an existing destination.
    pub fn rename(&mut self, source: Vec<u8>, destination: Vec<u8>) -> Result<(), ServerError> {
        let response = self.request(&V2Message::RenameRequest {
            source,
            destination,
        })?;
        match response.message {
            V2Message::RenameResponse {
                status: MutationStatus::Ok,
                ..
            } => Ok(()),
            V2Message::RenameResponse { status, error, .. } => Err(ServerError::RemoteError {
                code: status as u16,
                message: String::from_utf8_lossy(&error).into_owned(),
            }),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected RenameResponse, got {other:?}"
            ))),
        }
    }

    /// Create one remote directory, without creating missing parents.
    pub fn create_directory(&mut self, path: Vec<u8>) -> Result<(), ServerError> {
        let response = self.request(&V2Message::CreateDirectoryRequest { path })?;
        match response.message {
            V2Message::CreateDirectoryResponse {
                status: MutationStatus::Ok,
                ..
            } => Ok(()),
            V2Message::CreateDirectoryResponse { status, error, .. } => {
                Err(ServerError::RemoteError {
                    code: status as u16,
                    message: String::from_utf8_lossy(&error).into_owned(),
                })
            }
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected CreateDirectoryResponse, got {other:?}"
            ))),
        }
    }

    /// Delete a remote tree, calling `progress` once for every attempted item.
    pub fn delete_with_progress<F>(
        &mut self,
        path: Vec<u8>,
        mut progress: F,
    ) -> Result<V2Message, ServerError>
    where
        F: FnMut(&V2Message),
    {
        let related_id = self.send(&V2Message::DeleteRequest { path })?;
        loop {
            let response = self.receive()?;
            match &response.message {
                V2Message::DeleteProgress { related_id: id, .. } if *id == related_id => {
                    progress(&response.message);
                }
                V2Message::DeleteResponse { related_id: id, .. } if *id == related_id => {
                    return Ok(response.message);
                }
                V2Message::BrowseError { code: 1, .. } => {}
                other => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected delete progress or response, got {other:?}"
                    )))
                }
            }
        }
    }

    /// Fetch one remote regular file to a local path, publishing atomically.
    pub fn fetch(
        &mut self,
        remote_path: Vec<u8>,
        local_path: impl AsRef<Path>,
    ) -> Result<FetchedFile, ServerError> {
        let related_id = self.send(&V2Message::FetchRequest { path: remote_path })?;
        let start = loop {
            let response = self.receive()?;
            match response.message {
                V2Message::FetchStart { related_id: id, .. } if id == related_id => {
                    break response.message
                }
                V2Message::BrowseError { code, message, .. } => {
                    return Err(ServerError::RemoteError {
                        code,
                        message: String::from_utf8_lossy(&message).into_owned(),
                    })
                }
                other => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected FetchStart, got {other:?}"
                    )))
                }
            }
        };
        let V2Message::FetchStart {
            size,
            mtime_ns,
            device,
            file,
            digest,
            ..
        } = start
        else {
            unreachable!()
        };
        let local_path = local_path.as_ref();
        let parent = local_path.parent().unwrap_or_else(|| Path::new("."));
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(ServerError::Io)?;
        let mut expected_offset = 0_u64;
        let mut hasher = blake3::Hasher::new();
        while expected_offset < size {
            let response = self.receive()?;
            let V2Message::FetchChunk {
                related_id: id,
                offset,
                data,
            } = response.message
            else {
                return Err(ServerError::UnexpectedMessage(
                    "expected FetchChunk while fetching".to_owned(),
                ));
            };
            if id != related_id || offset != expected_offset {
                return Err(ServerError::UnexpectedMessage(
                    "fetch chunk is out of order".to_owned(),
                ));
            }
            expected_offset = expected_offset
                .checked_add(data.len() as u64)
                .ok_or_else(|| ServerError::UnexpectedMessage("fetch size overflow".to_owned()))?;
            if expected_offset > size {
                return Err(ServerError::UnexpectedMessage(
                    "fetch exceeded advertised size".to_owned(),
                ));
            }
            hasher.update(&data);
            staged.write_all(&data)?;
        }
        if *hasher.finalize().as_bytes() != digest {
            return Err(ServerError::UnexpectedMessage(
                "fetched digest does not match source".to_owned(),
            ));
        }
        staged
            .persist(local_path)
            .map_err(|error| ServerError::Io(error.error))?;
        Ok(FetchedFile {
            size,
            mtime_ns,
            identity: FileIdentity { device, file },
            digest,
        })
    }

    /// Publish a locally edited file only when the fetched remote identity is unchanged.
    pub fn publish(
        &mut self,
        remote_path: Vec<u8>,
        local_path: impl AsRef<Path>,
        fetched: FetchedFile,
    ) -> Result<V2Message, ServerError> {
        let bytes = fs::read(local_path).map_err(ServerError::Io)?;
        if bytes.len() as u64 != fetched.size || *blake3::hash(&bytes).as_bytes() != fetched.digest
        {
            return Err(ServerError::UnexpectedMessage(
                "local file no longer matches fetched identity".to_owned(),
            ));
        }
        let related_id = self.send(&V2Message::PublishRequest {
            path: remote_path,
            size: fetched.size,
            mtime_ns: fetched.mtime_ns,
            device: fetched.identity.device,
            file: fetched.identity.file,
            digest: fetched.digest,
        })?;
        match self.receive()?.message {
            V2Message::PublishReady { related_id: id } if id == related_id => {
                for (offset, data) in bytes.chunks(1024 * 1024).enumerate() {
                    self.send(&V2Message::PublishChunk {
                        related_id,
                        offset: (offset * 1024 * 1024) as u64,
                        data: data.to_vec(),
                    })?;
                }
                match self.receive()?.message {
                    response @ V2Message::PublishResponse { .. } => Ok(response),
                    other => Err(ServerError::UnexpectedMessage(format!(
                        "expected PublishResponse, got {other:?}"
                    ))),
                }
            }
            response @ V2Message::PublishResponse { .. } => Ok(response),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected PublishReady or PublishResponse, got {other:?}"
            ))),
        }
    }
}

#[must_use]
fn to_protocol_kind(kind: ScanEntryKind) -> crate::protocol::EntryKind {
    match kind {
        ScanEntryKind::File => crate::protocol::EntryKind::File,
        ScanEntryKind::Directory => crate::protocol::EntryKind::Directory,
        ScanEntryKind::Symlink => crate::protocol::EntryKind::Symlink,
        ScanEntryKind::Other => crate::protocol::EntryKind::Other,
    }
}

#[must_use]
fn from_protocol_kind(kind: crate::protocol::EntryKind) -> ScanEntryKind {
    match kind {
        crate::protocol::EntryKind::File => ScanEntryKind::File,
        crate::protocol::EntryKind::Directory => ScanEntryKind::Directory,
        crate::protocol::EntryKind::Symlink => ScanEntryKind::Symlink,
        crate::protocol::EntryKind::Other => ScanEntryKind::Other,
    }
}

/// Convert a [`SystemTime`] timestamp into nanoseconds since Unix epoch.
#[must_use]
pub fn system_time_to_nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX),
        Err(err) => {
            let nanos = err.duration().as_nanos();
            let neg = i64::try_from(nanos).unwrap_or(i64::MAX);
            -neg
        }
    }
}

/// Convert nanoseconds since Unix epoch into a [`SystemTime`].
#[must_use]
pub fn nanos_to_system_time(nanos: i64) -> SystemTime {
    if nanos >= 0 {
        UNIX_EPOCH + Duration::from_nanos(nanos as u64)
    } else {
        let neg = nanos.unsigned_abs();
        UNIX_EPOCH - Duration::from_nanos(neg)
    }
}

/// Convert a [`FileEntry`] into an [`EntryRecord`] for wire transmission.
#[must_use]
pub fn entry_record_from_file_entry(entry: &FileEntry) -> EntryRecord {
    let mtime_ns = system_time_to_nanos(entry.mtime);
    let mut fp = [0u8; 32];
    fp[0..8].copy_from_slice(&entry.fingerprint.identity.device.to_le_bytes());
    fp[8..16].copy_from_slice(&entry.fingerprint.identity.file.to_le_bytes());
    EntryRecord {
        path: entry.path.as_bytes().to_vec(),
        kind: to_protocol_kind(entry.kind),
        size: entry.size,
        mtime_ns,
        mode: entry.mode,
        fingerprint: fp,
    }
}

fn native_symlink_target(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        return PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
    }
    #[cfg(windows)]
    {
        return PathBuf::from(String::from_utf8_lossy(bytes).into_owned());
    }
    #[allow(unreachable_code)]
    PathBuf::new()
}

fn hash_file_streaming(path: &Path) -> io::Result<blake3::Hash> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(hasher.finalize());
        }
        hasher.update(&buffer[..read]);
    }
}

fn content_entry_record(
    root: &Path,
    entry: &FileEntry,
    cache: Option<&HashCache>,
) -> Result<EntryRecord, ServerError> {
    let mut record = entry_record_from_file_entry(entry);
    if entry.kind == ScanEntryKind::File {
        let path = entry.path.to_native_path(root);
        let identity = cached_content_identity(&path, entry, cache)?;
        record.fingerprint[..8].copy_from_slice(&identity.device.to_le_bytes());
        record.fingerprint[8..16].copy_from_slice(&identity.file.to_le_bytes());
    }
    Ok(record)
}

fn cached_content_identity(
    path: &Path,
    entry: &FileEntry,
    cache: Option<&HashCache>,
) -> Result<FileIdentity, ServerError> {
    let digest = match cache.and_then(|cache| {
        cache
            .hash_file(
                path,
                HashFingerprint {
                    device: entry.fingerprint.identity.device,
                    file: entry.fingerprint.identity.file,
                    size: entry.fingerprint.size,
                    mtime: entry.fingerprint.mtime,
                    ctime: entry.fingerprint.ctime,
                },
            )
            .ok()
    }) {
        Some(digest) => digest,
        None => blake3::hash(&fs::read(path).map_err(ServerError::Io)?),
    };
    Ok(FileIdentity {
        device: u64::from_le_bytes(digest.as_bytes()[..8].try_into().unwrap_or([0; 8])),
        file: u64::from_le_bytes(digest.as_bytes()[8..16].try_into().unwrap_or([0; 8])),
    })
}

/// Convert an [`EntryRecord`] received from the wire into a [`FileEntry`].
///
/// # Errors
/// Returns [`ServerError::InvalidPath`] if the path bytes are unsafe.
pub fn file_entry_from_entry_record(record: &EntryRecord) -> Result<FileEntry, ServerError> {
    let path = WirePath::from_wire(record.path.clone())
        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
    let mtime = nanos_to_system_time(record.mtime_ns);
    let device = u64::from_le_bytes(record.fingerprint[0..8].try_into().unwrap_or([0; 8]));
    let file = u64::from_le_bytes(record.fingerprint[8..16].try_into().unwrap_or([0; 8]));
    let kind = from_protocol_kind(record.kind);
    Ok(FileEntry {
        path,
        kind,
        size: record.size,
        mtime,
        mode: record.mode,
        fingerprint: SourceFingerprint {
            identity: FileIdentity { device, file },
            kind,
            size: record.size,
            mtime,
            ctime: None,
        },
    })
}

/// Validate a relative mutation path before touching its destination.
///
/// Every existing ancestor is checked with `symlink_metadata`, and any
/// symlink in the parent chain is refused. This keeps mutation operations
/// inside the configured root even when a link points outside it.
///
/// # Errors
/// Returns [`ServerError::InvalidPath`] or [`ServerError::SymlinkEscape`].
pub fn validate_destination_path(
    root: &Path,
    relative_path: impl Into<WirePath>,
) -> Result<PathBuf, ServerError> {
    let relative_path = relative_path.into();
    let relative_path = WirePath::from_wire(relative_path.as_bytes().to_vec())
        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
    if relative_path.is_empty() {
        return Err(ServerError::InvalidPath("empty path".to_owned()));
    }
    let current = relative_path.to_native_path(root);
    let mut ancestor = root.to_path_buf();
    let components: Vec<&[u8]> = relative_path
        .as_bytes()
        .split(|byte| *byte == b'/')
        .collect();
    for (index, component) in components.iter().enumerate() {
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            ancestor.push(OsString::from_vec(component.to_vec()));
        }
        #[cfg(not(unix))]
        ancestor.push(String::from_utf8_lossy(component).as_ref());
        // The final component may itself be replaced by a mutation, but an
        // existing symlink in the parent chain can redirect that mutation.
        if let Ok(metadata) = fs::symlink_metadata(&ancestor) {
            if metadata.file_type().is_symlink() && index + 1 < components.len() {
                return Err(ServerError::SymlinkEscape(relative_path.to_string()));
            }
        }
    }
    Ok(current)
}

/// Validate and register one normalized destination for a mutation session.
///
/// `WirePath` is the protocol's normalized representation, so equivalent
/// destinations are compared as paths rather than as presentation strings.
pub fn validate_unique_destination_path(
    root: &Path,
    path: WirePath,
    seen: &mut HashSet<WirePath>,
) -> Result<PathBuf, ServerError> {
    let native = validate_destination_path(root, path.clone())?;
    if !seen.insert(path.clone()) {
        return Err(ServerError::DuplicatePath(path.to_string()));
    }
    Ok(native)
}

fn browse_directory_path(root: &Path, path: &[u8]) -> Result<PathBuf, ServerError> {
    let relative = WirePath::from_wire(path.to_vec())
        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
    if relative.is_empty() {
        return Ok(root.to_path_buf());
    }
    let mut ancestor = root.to_path_buf();
    for component in relative.as_bytes().split(|byte| *byte == b'/') {
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            ancestor.push(OsString::from_vec(component.to_vec()));
        }
        #[cfg(not(unix))]
        ancestor.push(String::from_utf8_lossy(component).as_ref());
        if let Ok(metadata) = fs::symlink_metadata(&ancestor) {
            if metadata.file_type().is_symlink() {
                return Err(ServerError::SymlinkEscape(relative.to_string()));
            }
        }
    }
    Ok(relative.to_native_path(root))
}

fn browse_stat_path(root: &Path, path: &[u8]) -> Result<PathBuf, ServerError> {
    let relative = WirePath::from_wire(path.to_vec())
        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
    if relative.is_empty() {
        return Ok(root.to_path_buf());
    }
    let components: Vec<&[u8]> = relative.as_bytes().split(|byte| *byte == b'/').collect();
    let mut ancestor = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            ancestor.push(OsString::from_vec(component.to_vec()));
        }
        #[cfg(not(unix))]
        ancestor.push(String::from_utf8_lossy(component).as_ref());
        if index + 1 != components.len() {
            if let Ok(metadata) = fs::symlink_metadata(&ancestor) {
                if metadata.file_type().is_symlink() {
                    return Err(ServerError::SymlinkEscape(relative.to_string()));
                }
            }
        }
    }
    Ok(relative.to_native_path(root))
}

fn native_path_bytes(path: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        return path.as_bytes().to_vec();
    }
    #[cfg(not(unix))]
    path.to_string_lossy().as_bytes().to_vec()
}

fn mutation_failure(error: &ServerError) -> (MutationStatus, String) {
    (MutationStatus::Error, error.to_string())
}

fn rename_status(error: &io::Error) -> MutationStatus {
    match error.kind() {
        io::ErrorKind::AlreadyExists => MutationStatus::AlreadyExists,
        io::ErrorKind::PermissionDenied => MutationStatus::PermissionDenied,
        io::ErrorKind::NotFound => MutationStatus::ParentMissing,
        _ if error.raw_os_error() == Some(libc::EXDEV) => MutationStatus::CrossDevice,
        _ => MutationStatus::Error,
    }
}

fn mkdir_status(error: &io::Error) -> MutationStatus {
    match error.kind() {
        io::ErrorKind::AlreadyExists => MutationStatus::AlreadyExists,
        io::ErrorKind::PermissionDenied => MutationStatus::PermissionDenied,
        io::ErrorKind::NotFound => MutationStatus::ParentMissing,
        _ => MutationStatus::Error,
    }
}

fn mutation_response(
    related_id: u64,
    result: Result<(), (MutationStatus, String)>,
    rename: bool,
) -> V2Message {
    let (status, error) = match result {
        Ok(()) => (MutationStatus::Ok, Vec::new()),
        Err((status, error)) => (status, error.into_bytes()),
    };
    if rename {
        V2Message::RenameResponse {
            related_id,
            status,
            error,
        }
    } else {
        V2Message::CreateDirectoryResponse {
            related_id,
            status,
            error,
        }
    }
}

fn publish_changed_response(related_id: u64, current: Option<(u64, i64, u64, u64)>) -> V2Message {
    let (current_present, size, mtime_ns, device, file) = current
        .map_or((false, 0, 0, 0, 0), |(size, mtime_ns, device, file)| {
            (true, size, mtime_ns, device, file)
        });
    V2Message::PublishResponse {
        related_id,
        status: protocol_v2::PublishStatus::Changed,
        current_present,
        size,
        mtime_ns,
        device,
        file,
        error: b"remote file changed underneath the editor".to_vec(),
    }
}

fn publish_error_response(related_id: u64, error: &str) -> V2Message {
    V2Message::PublishResponse {
        related_id,
        status: protocol_v2::PublishStatus::Error,
        current_present: false,
        size: 0,
        mtime_ns: 0,
        device: 0,
        file: 0,
        error: error.as_bytes().to_vec(),
    }
}

/// A server instance executing Source, Sink, or long-lived Session roles over
/// framed streams.
#[derive(Debug)]
pub struct Server {
    root: PathBuf,
    next_message_id: u64,
    decoder: FrameDecoder,
    seen_destinations: HashSet<WirePath>,
    journal: Option<crate::journal::ResumeJournal>,
    compression: CompressionMode,
    compression_level: i32,
    capabilities: u32,
}

impl Server {
    /// Create a new server rooted at `root`.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            next_message_id: 1000,
            decoder: FrameDecoder::new(),
            seen_destinations: HashSet::new(),
            journal: None,
            compression: CompressionMode::None,
            compression_level: 3,
            capabilities: CAP_ZSTD | CAP_VERSION_NEGOTIATION,
        }
    }

    /// Create a server with an explicit capability set.
    ///
    /// This is also used to exercise interoperability with older or
    /// feature-reduced peers that do not advertise compression.
    #[must_use]
    pub fn new_with_capabilities(root: impl AsRef<Path>, capabilities: u32) -> Self {
        let mut server = Self::new(root);
        server.capabilities = capabilities;
        server
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id
    }

    /// Run the server state machine over `reader` and `writer`.
    ///
    /// # Errors
    /// Returns [`ServerError`] on protocol, I/O, or state errors.
    pub fn run<R: Read + Send + 'static, W: Write>(
        &mut self,
        mut reader: R,
        mut writer: W,
    ) -> Result<(), ServerError> {
        // 1. Receive Handshake from client.
        let frame = self.decoder.read(&mut reader)?;
        let (client_role, client_capabilities, job_id, compression, compression_level) =
            match frame.message {
                Message::Handshake {
                    role,
                    capabilities,
                    job_id,
                    compression,
                    compression_level,
                    ..
                } => (role, capabilities, job_id, compression, compression_level),
                other => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Handshake, got {other:?}"
                    )))
                }
            };
        server_log(format_args!(
            "received handshake: frame_id={}, role={client_role:?}, capabilities=0x{client_capabilities:x}",
            frame.message_id
        ));
        let advertised_capabilities = if client_role == Role::Session {
            self.capabilities | CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION
        } else {
            self.capabilities
        };
        self.compression =
            negotiate_compression(compression, advertised_capabilities, client_capabilities);
        self.compression_level = compression_level;

        let selected_version =
            negotiate_protocol_version(advertised_capabilities, client_capabilities);
        if client_role == Role::Session && selected_version != 2 {
            return Err(ServerError::UnexpectedMessage(
                "session role requires negotiated protocol v2".to_owned(),
            ));
        }

        // Browse sessions do not own sync state or a resume journal.
        if client_role != Role::Session {
            self.journal = Some(crate::journal::ResumeJournal::new(&job_id)?);
        }

        // A data-only session (multi-stream) skips the destination scan and
        // only writes segment traffic; the control session owns metadata and
        // the journal.
        let data_only = client_capabilities & crate::protocol::CAP_DATA_ONLY != 0;

        // Determine server's role.
        let server_role = match client_role {
            Role::Source => Role::Sink,
            Role::Sink => Role::Source,
            Role::Session => Role::Session,
        };

        // Send Server Handshake and Ack.
        let server_handshake = Message::Handshake {
            role: server_role,
            capabilities: advertised_capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id,
            compression: self.compression,
            compression_level,
        };
        let msg_id = self.next_id();
        let bytes = encode_frame(msg_id, &server_handshake)?;
        writer.write_all(&bytes)?;

        let ack = Message::Ack {
            acknowledged_id: frame.message_id,
            acknowledged_type: 1, // Handshake
        };
        let msg_id = self.next_id();
        let bytes = encode_frame(msg_id, &ack)?;
        writer.write_all(&bytes)?;
        writer.flush()?;

        if client_role == Role::Session {
            return self.run_browse_session(reader, &mut writer);
        }

        // 2. Receive SessionConfig from client.
        let frame = self.decoder.read(&mut reader)?;
        let (paranoid, delete, checksum, dry_run, exclude_patterns) = match frame.message {
            Message::SessionConfig {
                paranoid,
                delete,
                checksum,
                dry_run,
                exclude_patterns,
                ..
            } => (paranoid, delete, checksum, dry_run, exclude_patterns),
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected SessionConfig, got {other:?}"
                )))
            }
        };
        server_log(format_args!(
            "received session config: frame_id={}, data_only={}, paranoid={}, delete={}, checksum={}, dry_run={}, excludes={}",
            frame.message_id,
            data_only,
            paranoid,
            delete,
            checksum,
            dry_run,
            exclude_patterns.len()
        ));

        // Send Ack for SessionConfig.
        let ack = Message::Ack {
            acknowledged_id: frame.message_id,
            acknowledged_type: 2, // SessionConfig
        };
        let msg_id = self.next_id();
        let bytes = encode_frame(msg_id, &ack)?;
        writer.write_all(&bytes)?;
        writer.flush()?;

        match server_role {
            Role::Sink => {
                if data_only {
                    self.run_data_sink(&mut reader, &mut writer)
                } else {
                    self.run_sink(
                        &mut reader,
                        &mut writer,
                        paranoid,
                        delete,
                        checksum,
                        dry_run,
                        &exclude_patterns,
                    )
                }
            }
            Role::Source => self.run_source(&mut reader, &mut writer),
            Role::Session => unreachable!("browse sessions return before sync dispatch"),
        }
    }

    fn run_browse_session<R: Read + Send + 'static, W: Write>(
        &mut self,
        mut reader: R,
        writer: &mut W,
    ) -> Result<(), ServerError> {
        let (frames, incoming) = mpsc::channel();
        thread::spawn(move || loop {
            let frame = protocol_v2::read_frame(&mut reader);
            let done = matches!(frame, Ok(None) | Err(_));
            if frames.send(frame).is_err() || done {
                break;
            }
        });
        let mut seen_ids = HashSet::new();
        let mut active_requests = HashSet::new();
        let mut pending = VecDeque::new();
        loop {
            let frame = if let Some(frame) = pending.pop_front() {
                frame
            } else {
                match incoming.recv() {
                    Ok(Ok(Some(frame))) => frame,
                    Ok(Ok(None)) => return Ok(()),
                    Ok(Err(error)) => return Err(ServerError::Browse(error)),
                    Err(_) => return Ok(()),
                }
            };
            if !seen_ids.insert(frame.message_id) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "duplicate v2 session message ID {}",
                    frame.message_id
                )));
            }
            let response: Option<V2Message> = match frame.message {
                V2Message::Keepalive { nonce } => Some(V2Message::KeepaliveAck { nonce }),
                V2Message::CancelRequest { related_id } => {
                    let message = if active_requests.remove(&related_id) {
                        b"request cancelled".to_vec()
                    } else {
                        b"request already complete".to_vec()
                    };
                    Some(V2Message::BrowseError {
                        related_id,
                        code: 1,
                        message,
                    })
                }
                V2Message::ListRequest {
                    path,
                    page_token,
                    page_size,
                } => match self.browse_list_page(&path, page_token, page_size, frame.message_id) {
                    Ok(page) => Some(page),
                    Err(error)
                        if matches!(
                            &error,
                            ServerError::InvalidPath(_) | ServerError::SymlinkEscape(_)
                        ) =>
                    {
                        Some(V2Message::BrowseError {
                            related_id: frame.message_id,
                            code: 2,
                            message: error.to_string().into_bytes(),
                        })
                    }
                    Err(error) => Some(V2Message::BrowseError {
                        related_id: frame.message_id,
                        code: 3,
                        message: error.to_string().into_bytes(),
                    }),
                },
                V2Message::StatRequest {
                    path,
                    include_digest,
                } => match self.browse_stat_response(&path, include_digest, frame.message_id) {
                    Ok(response) => Some(response),
                    Err(error)
                        if matches!(
                            &error,
                            ServerError::InvalidPath(_) | ServerError::SymlinkEscape(_)
                        ) =>
                    {
                        Some(V2Message::BrowseError {
                            related_id: frame.message_id,
                            code: 2,
                            message: error.to_string().into_bytes(),
                        })
                    }
                    Err(error) => Some(V2Message::StatResponse {
                        related_id: frame.message_id,
                        status: protocol_v2::StatStatus::Error,
                        entry: None,
                        digest: None,
                        error: error.to_string().into_bytes(),
                    }),
                },
                V2Message::RenameRequest {
                    source,
                    destination,
                } => Some(self.browse_rename_response(&source, &destination, frame.message_id)),
                V2Message::CreateDirectoryRequest { path } => {
                    Some(self.browse_create_directory_response(&path, frame.message_id))
                }
                V2Message::DeleteRequest { path } => {
                    active_requests.insert(frame.message_id);
                    Some(self.browse_delete(
                        &path,
                        frame.message_id,
                        &incoming,
                        &mut pending,
                        writer,
                    )?)
                }
                V2Message::FetchRequest { path } => {
                    match self.browse_fetch(&path, frame.message_id, writer) {
                        Ok(()) => None,
                        Err(error) => Some(V2Message::BrowseError {
                            related_id: frame.message_id,
                            code: if matches!(
                                &error,
                                ServerError::InvalidPath(_) | ServerError::SymlinkEscape(_)
                            ) {
                                2
                            } else {
                                4
                            },
                            message: error.to_string().into_bytes(),
                        }),
                    }
                }
                V2Message::PublishRequest {
                    path,
                    size,
                    mtime_ns,
                    device,
                    file,
                    digest,
                } => match self.browse_publish(
                    &path,
                    frame.message_id,
                    size,
                    mtime_ns,
                    device,
                    file,
                    digest,
                    &incoming,
                    &mut pending,
                    writer,
                ) {
                    Ok(response) => Some(response),
                    Err(error) => {
                        Some(publish_error_response(frame.message_id, &error.to_string()))
                    }
                },
                V2Message::ListPage { .. }
                | V2Message::StatResponse { .. }
                | V2Message::RenameResponse { .. }
                | V2Message::CreateDirectoryResponse { .. }
                | V2Message::DeleteProgress { .. }
                | V2Message::DeleteResponse { .. }
                | V2Message::FetchStart { .. }
                | V2Message::FetchChunk { .. }
                | V2Message::PublishReady { .. }
                | V2Message::PublishChunk { .. }
                | V2Message::PublishResponse { .. }
                | V2Message::KeepaliveAck { .. }
                | V2Message::BrowseError { .. } => {
                    return Err(ServerError::UnexpectedMessage(
                        "v2 session received a response message".to_owned(),
                    ));
                }
            };
            let Some(response) = response else {
                continue;
            };
            if let V2Message::ListPage {
                related_id,
                final_page,
                ..
            } = &response
            {
                if *final_page {
                    active_requests.remove(related_id);
                } else {
                    active_requests.insert(*related_id);
                }
            }
            if let V2Message::DeleteResponse { related_id, .. } = &response {
                active_requests.remove(related_id);
            }
            let bytes = protocol_v2::encode_frame(self.next_id(), &response)?;
            writer.write_all(&bytes)?;
            writer.flush()?;
        }
    }

    fn browse_fetch(
        &mut self,
        path: &[u8],
        related_id: u64,
        writer: &mut impl Write,
    ) -> Result<(), ServerError> {
        let relative = WirePath::from_wire(path.to_vec())
            .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
        let native = validate_destination_path(&self.root, relative.clone())?;
        let metadata = fs::symlink_metadata(&native)?;
        if !metadata.file_type().is_file() {
            return Err(ServerError::UnexpectedMessage(
                "fetch source is not a regular file".to_owned(),
            ));
        }
        let mtime = metadata.modified().map_err(ServerError::Io)?;
        let fingerprint = fingerprint_from_metadata(&metadata, ScanEntryKind::File, mtime)
            .map_err(ServerError::Io)?;
        let entry = FileEntry {
            path: relative,
            kind: ScanEntryKind::File,
            size: metadata.len(),
            mtime,
            mode: permission_mode(&metadata),
            fingerprint,
        };
        let stable = SourceReader::new(&self.root).read(&entry)?;
        let start = V2Message::FetchStart {
            related_id,
            size: stable.entry.size,
            mtime_ns: system_time_to_nanos(stable.entry.mtime),
            device: stable.entry.fingerprint.identity.device,
            file: stable.entry.fingerprint.identity.file,
            digest: *stable.blake3.as_bytes(),
        };
        writer.write_all(&protocol_v2::encode_frame(self.next_id(), &start)?)?;
        for (offset, data) in stable.bytes.chunks(1024 * 1024).enumerate() {
            let chunk = V2Message::FetchChunk {
                related_id,
                offset: (offset * 1024 * 1024) as u64,
                data: data.to_vec(),
            };
            writer.write_all(&protocol_v2::encode_frame(self.next_id(), &chunk)?)?;
        }
        writer.flush()?;
        Ok(())
    }

    fn browse_publish(
        &mut self,
        path: &[u8],
        related_id: u64,
        size: u64,
        mtime_ns: i64,
        device: u64,
        file: u64,
        digest: [u8; 32],
        incoming: &mpsc::Receiver<Result<Option<V2Frame>, V2CodecError>>,
        pending: &mut VecDeque<V2Frame>,
        writer: &mut impl Write,
    ) -> Result<V2Message, ServerError> {
        let relative = WirePath::from_wire(path.to_vec())
            .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
        let native = validate_destination_path(&self.root, relative.clone())?;
        let metadata = match fs::symlink_metadata(&native) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => {
                return Ok(publish_error_response(
                    related_id,
                    "remote target is not a regular file",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(publish_changed_response(related_id, None));
            }
            Err(error) => return Ok(publish_error_response(related_id, &error.to_string())),
        };
        let current_mtime = metadata.modified().map_err(ServerError::Io)?;
        let current_fp = fingerprint_from_metadata(&metadata, ScanEntryKind::File, current_mtime)
            .map_err(ServerError::Io)?;
        let current = (
            metadata.len(),
            system_time_to_nanos(current_mtime),
            current_fp.identity.device,
            current_fp.identity.file,
        );
        if current != (size, mtime_ns, device, file) {
            return Ok(publish_changed_response(related_id, Some(current)));
        }
        let ready = V2Message::PublishReady { related_id };
        writer.write_all(&protocol_v2::encode_frame(self.next_id(), &ready)?)?;
        writer.flush()?;

        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        while offset < size {
            let frame = if let Some(frame) = pending.pop_front() {
                frame
            } else {
                match incoming.recv() {
                    Ok(Ok(Some(frame))) => frame,
                    Ok(Ok(None)) => return Err(ServerError::PeerDisconnected),
                    Ok(Err(error)) => return Err(ServerError::Browse(error)),
                    Err(_) => return Err(ServerError::PeerDisconnected),
                }
            };
            match frame.message {
                V2Message::PublishChunk {
                    related_id: id,
                    offset: chunk_offset,
                    data,
                } if id == related_id && chunk_offset == offset => {
                    offset = offset.saturating_add(data.len() as u64);
                    if offset > size {
                        return Ok(publish_error_response(
                            related_id,
                            "publish exceeded advertised size",
                        ));
                    }
                    bytes.extend_from_slice(&data);
                }
                other => {
                    return Ok(publish_error_response(
                        related_id,
                        &format!("unexpected publish frame: {other:?}"),
                    ));
                }
            }
        }
        if blake3::hash(&bytes).as_bytes() != &digest {
            return Ok(publish_error_response(
                related_id,
                "publish digest does not match",
            ));
        }
        let latest = fs::symlink_metadata(&native).ok().and_then(|metadata| {
            let mtime = metadata.modified().ok()?;
            let fp = fingerprint_from_metadata(&metadata, ScanEntryKind::File, mtime).ok()?;
            Some((
                metadata.len(),
                system_time_to_nanos(mtime),
                fp.identity.device,
                fp.identity.file,
            ))
        });
        if latest != Some(current) {
            return Ok(publish_changed_response(related_id, latest));
        }
        let entry = FileEntry {
            path: relative,
            kind: ScanEntryKind::File,
            size,
            mtime: nanos_to_system_time(mtime_ns),
            mode: permission_mode(&metadata),
            fingerprint: SourceFingerprint {
                identity: FileIdentity { device, file },
                kind: ScanEntryKind::File,
                size,
                mtime: nanos_to_system_time(mtime_ns),
                ctime: None,
            },
        };
        match Sink::new(&self.root)
            .map_err(ServerError::Sink)
            .and_then(|sink| {
                sink.write_file_with_retry(&entry, &blake3::Hash::from_bytes(digest), |_| {
                    Ok(bytes.clone())
                })
                .map_err(ServerError::Sink)
            }) {
            Ok(()) => Ok(V2Message::PublishResponse {
                related_id,
                status: protocol_v2::PublishStatus::Ok,
                current_present: true,
                size,
                mtime_ns,
                device,
                file,
                error: Vec::new(),
            }),
            Err(error) => Ok(publish_error_response(related_id, &error.to_string())),
        }
    }

    fn browse_delete(
        &mut self,
        path: &[u8],
        related_id: u64,
        incoming: &mpsc::Receiver<Result<Option<V2Frame>, V2CodecError>>,
        pending: &mut VecDeque<V2Frame>,
        writer: &mut impl Write,
    ) -> Result<V2Message, ServerError> {
        let relative = match WirePath::from_wire(path.to_vec()) {
            Ok(relative) => relative,
            Err(_error) => {
                return Ok(V2Message::DeleteResponse {
                    related_id,
                    status: protocol_v2::DeleteStatus::Partial,
                    removed_count: 0,
                    failures: vec![protocol_v2::DeleteFailure {
                        path: path.to_vec(),
                        errno: libc::EINVAL,
                    }],
                    irreversible: true,
                });
            }
        };
        if relative.is_empty() {
            return Ok(V2Message::DeleteResponse {
                related_id,
                status: protocol_v2::DeleteStatus::Partial,
                removed_count: 0,
                failures: vec![protocol_v2::DeleteFailure {
                    path: Vec::new(),
                    errno: libc::EINVAL,
                }],
                irreversible: true,
            });
        }
        let native = match validate_destination_path(&self.root, relative.clone()) {
            Ok(native) => native,
            Err(_) => {
                return Ok(V2Message::DeleteResponse {
                    related_id,
                    status: protocol_v2::DeleteStatus::Partial,
                    removed_count: 0,
                    failures: vec![protocol_v2::DeleteFailure {
                        path: path.to_vec(),
                        errno: libc::EINVAL,
                    }],
                    irreversible: true,
                });
            }
        };
        let mut stack = vec![(native, relative, false)];
        let mut failures = Vec::new();
        let mut removed_count = 0_u64;
        let mut cancelled = false;

        while let Some((native, relative, after_children)) = stack.pop() {
            while let Ok(message) = incoming.try_recv() {
                match message {
                    Ok(Some(frame)) if matches!(frame.message, V2Message::CancelRequest { related_id: id } if id == related_id) =>
                    {
                        let acknowledgement = V2Message::BrowseError {
                            related_id,
                            code: 1,
                            message: b"request cancelled".to_vec(),
                        };
                        writer.write_all(&protocol_v2::encode_frame(
                            self.next_id(),
                            &acknowledgement,
                        )?)?;
                        writer.flush()?;
                        cancelled = true;
                    }
                    Ok(Some(frame)) => pending.push_back(frame),
                    Ok(None) | Err(_) => break,
                }
                if cancelled {
                    break;
                }
            }
            if cancelled {
                break;
            }

            if !after_children {
                let metadata = match fs::symlink_metadata(&native) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        failures.push(protocol_v2::DeleteFailure {
                            path: relative.as_bytes().to_vec(),
                            errno: error.raw_os_error().unwrap_or(0),
                        });
                        self.write_delete_progress(
                            writer,
                            related_id,
                            &relative,
                            false,
                            &error.to_string(),
                        )?;
                        continue;
                    }
                };
                if metadata.file_type().is_dir() {
                    let mut children = Vec::new();
                    match fs::read_dir(&native) {
                        Ok(entries) => {
                            for entry in entries {
                                match entry {
                                    Ok(entry) => {
                                        let child = WirePath::from_wire(
                                            [
                                                relative.as_bytes(),
                                                b"/",
                                                &native_path_bytes(&entry.file_name()),
                                            ]
                                            .concat(),
                                        );
                                        match child {
                                            Ok(child) => children.push((entry.path(), child)),
                                            Err(_error) => {
                                                failures.push(protocol_v2::DeleteFailure {
                                                    path: relative.as_bytes().to_vec(),
                                                    errno: libc::EINVAL,
                                                })
                                            }
                                        }
                                    }
                                    Err(error) => failures.push(protocol_v2::DeleteFailure {
                                        path: relative.as_bytes().to_vec(),
                                        errno: error.raw_os_error().unwrap_or(0),
                                    }),
                                }
                            }
                        }
                        Err(error) => {
                            failures.push(protocol_v2::DeleteFailure {
                                path: relative.as_bytes().to_vec(),
                                errno: error.raw_os_error().unwrap_or(0),
                            });
                            self.write_delete_progress(
                                writer,
                                related_id,
                                &relative,
                                false,
                                &error.to_string(),
                            )?;
                            continue;
                        }
                    }
                    stack.push((native, relative, true));
                    for child in children.into_iter().rev() {
                        stack.push((child.0, child.1, false));
                    }
                    continue;
                }
            }

            let result = if after_children {
                fs::remove_dir(&native)
            } else {
                fs::remove_file(&native)
            };
            match result {
                Ok(()) => {
                    removed_count = removed_count.saturating_add(1);
                    self.write_delete_progress(writer, related_id, &relative, true, "")?;
                }
                Err(error) => {
                    failures.push(protocol_v2::DeleteFailure {
                        path: relative.as_bytes().to_vec(),
                        errno: error.raw_os_error().unwrap_or(0),
                    });
                    self.write_delete_progress(
                        writer,
                        related_id,
                        &relative,
                        false,
                        &error.to_string(),
                    )?;
                }
            }
        }

        let status = if cancelled {
            protocol_v2::DeleteStatus::Cancelled
        } else if failures.is_empty() {
            protocol_v2::DeleteStatus::Complete
        } else {
            protocol_v2::DeleteStatus::Partial
        };
        Ok(V2Message::DeleteResponse {
            related_id,
            status,
            removed_count,
            failures,
            irreversible: true,
        })
    }

    fn write_delete_progress(
        &mut self,
        writer: &mut impl Write,
        related_id: u64,
        path: &WirePath,
        removed: bool,
        error: &str,
    ) -> Result<(), ServerError> {
        let progress = V2Message::DeleteProgress {
            related_id,
            path: path.as_bytes().to_vec(),
            removed,
            error: error.as_bytes().to_vec(),
        };
        writer.write_all(&protocol_v2::encode_frame(self.next_id(), &progress)?)?;
        writer.flush()?;
        Ok(())
    }

    fn browse_list_page(
        &self,
        path: &[u8],
        page_token: u64,
        page_size: u32,
        related_id: u64,
    ) -> Result<V2Message, ServerError> {
        let directory = browse_directory_path(&self.root, path)?;
        let start = usize::try_from(page_token)
            .map_err(|_| ServerError::InvalidPath("page token is too large".to_owned()))?;
        let mut entries = Vec::with_capacity(page_size as usize);
        let mut index = 0usize;
        let mut directory_entries = fs::read_dir(directory)?.peekable();

        while index < start {
            if directory_entries.next().is_none() {
                return Ok(V2Message::ListPage {
                    related_id,
                    page_token: 0,
                    final_page: true,
                    entries,
                });
            }
            index += 1;
        }

        while entries.len() < page_size as usize {
            let Some(item) = directory_entries.next() else {
                return Ok(V2Message::ListPage {
                    related_id,
                    page_token: 0,
                    final_page: true,
                    entries,
                });
            };
            index += 1;
            let item = match item {
                Ok(item) => item,
                Err(_) => continue,
            };
            let metadata = match fs::symlink_metadata(item.path()) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            let file_type = metadata.file_type();
            let (kind, symlink_target) = if file_type.is_file() {
                (1, Vec::new())
            } else if file_type.is_dir() {
                (2, Vec::new())
            } else if file_type.is_symlink() {
                let target = fs::read_link(item.path())
                    .map(|target| native_path_bytes(target.as_os_str()))
                    .unwrap_or_default();
                (3, target)
            } else {
                (4, Vec::new())
            };
            entries.push(BrowseEntry {
                name: native_path_bytes(&item.file_name()),
                kind,
                size: metadata.len(),
                mtime_ns: metadata
                    .modified()
                    .map(system_time_to_nanos)
                    .unwrap_or_default(),
                mode: permission_mode(&metadata),
                symlink_target,
            });
        }

        let final_page = directory_entries.peek().is_none();
        Ok(V2Message::ListPage {
            related_id,
            page_token: if final_page { 0 } else { index as u64 },
            final_page,
            entries,
        })
    }

    fn browse_stat_response(
        &self,
        path: &[u8],
        include_digest: bool,
        related_id: u64,
    ) -> Result<V2Message, ServerError> {
        let native_path = browse_stat_path(&self.root, path)?;
        let metadata = match fs::symlink_metadata(&native_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(V2Message::StatResponse {
                    related_id,
                    status: protocol_v2::StatStatus::Missing,
                    entry: None,
                    digest: None,
                    error: Vec::new(),
                });
            }
            Err(error) => return Err(ServerError::Io(error)),
        };
        let file_type = metadata.file_type();
        let (kind, symlink_target) = if file_type.is_file() {
            (1, Vec::new())
        } else if file_type.is_dir() {
            (2, Vec::new())
        } else if file_type.is_symlink() {
            let target = fs::read_link(&native_path)
                .map(|target| native_path_bytes(target.as_os_str()))
                .unwrap_or_default();
            (3, target)
        } else {
            (4, Vec::new())
        };
        let name = path
            .rsplit(|byte| *byte == b'/')
            .next()
            .unwrap_or_default()
            .to_vec();
        let entry = BrowseEntry {
            name,
            kind,
            size: metadata.len(),
            mtime_ns: metadata
                .modified()
                .map(system_time_to_nanos)
                .unwrap_or_default(),
            mode: permission_mode(&metadata),
            symlink_target,
        };
        let digest = if include_digest && kind == 1 {
            let hash = hash_file_streaming(&native_path)?;
            Some(*hash.as_bytes())
        } else {
            None
        };
        Ok(V2Message::StatResponse {
            related_id,
            status: protocol_v2::StatStatus::Ok,
            entry: Some(entry),
            digest,
            error: Vec::new(),
        })
    }

    fn browse_rename_response(
        &self,
        source: &[u8],
        destination: &[u8],
        related_id: u64,
    ) -> V2Message {
        let result = (|| -> Result<(), (MutationStatus, String)> {
            let source = validate_destination_path(
                &self.root,
                WirePath::from_wire(source.to_vec())
                    .map_err(|error| (MutationStatus::Error, error.to_string()))?,
            )
            .map_err(|error| mutation_failure(&error))?;
            let destination = validate_destination_path(
                &self.root,
                WirePath::from_wire(destination.to_vec())
                    .map_err(|error| (MutationStatus::Error, error.to_string()))?,
            )
            .map_err(|error| mutation_failure(&error))?;
            if fs::symlink_metadata(&destination).is_ok() {
                return Err((
                    MutationStatus::AlreadyExists,
                    "destination already exists".to_owned(),
                ));
            }
            fs::rename(source, destination)
                .map_err(|error| (rename_status(&error), error.to_string()))
        })();
        mutation_response(related_id, result, true)
    }

    fn browse_create_directory_response(&self, path: &[u8], related_id: u64) -> V2Message {
        let result = (|| -> Result<(), (MutationStatus, String)> {
            let path = WirePath::from_wire(path.to_vec())
                .map_err(|error| (MutationStatus::Error, error.to_string()))?;
            let path = validate_destination_path(&self.root, path)
                .map_err(|error| mutation_failure(&error))?;
            fs::create_dir(path).map_err(|error| (mkdir_status(&error), error.to_string()))
        })();
        mutation_response(related_id, result, false)
    }

    /// A data-only receiver session (multi-stream Story 4.2): skips the
    /// destination scan and only writes small/whole files and verified chunk
    /// ranges into the shared stage, checkpointing each range into the union
    /// journal. The control session owns metadata, prepare/finish, and clearing
    /// the journal.
    fn run_data_sink<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), ServerError> {
        let sink = Sink::new(&self.root)?;
        // file_id -> EntryRecord for small/whole files.
        let mut active_files: HashMap<u64, EntryRecord> = HashMap::new();
        // file_id -> FileEntry for chunked large files, plus the ranges this
        // session has written (for merge-on-checkpoint).
        let mut large_files: HashMap<u64, FileEntry> = HashMap::new();
        let mut large_ranges: HashMap<u64, Vec<ByteRange>> = HashMap::new();

        loop {
            let frame = match self.decoder.read(reader) {
                Ok(frame) => frame,
                Err(ProtocolError::Read(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(err) => return Err(ServerError::Protocol(err)),
            };

            match frame.message {
                Message::FileBatch {
                    batch_id: _,
                    entries,
                } => {
                    for (i, entry) in entries.into_iter().enumerate() {
                        let file_id = (frame.message_id << 16) | (i as u64);
                        active_files.insert(file_id, entry);
                    }
                    self.ack(writer, frame.message_id, 3)?;
                }
                Message::FileSegment {
                    file_id,
                    offset,
                    data,
                } => {
                    if let Some(record) = active_files.remove(&file_id) {
                        if offset != 0 {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "non-zero offset {offset} for complete file {file_id}"
                            )));
                        }
                        let file_entry = file_entry_from_entry_record(&record)?;
                        validate_unique_destination_path(
                            &self.root,
                            file_entry.path.clone(),
                            &mut self.seen_destinations,
                        )?;
                        let hash = blake3::hash(&data);
                        sink.write_file_with_retry(
                            &file_entry,
                            &hash,
                            |_attempt| Ok(data.clone()),
                        )?;
                        self.ack(writer, frame.message_id, 4)?;
                    } else if let Some(file_entry) = large_files.get(&file_id) {
                        let length = data.len() as u64;
                        let hash = blake3::hash(&data);
                        sink.write_chunk_with_retry(
                            file_entry,
                            offset,
                            length,
                            &hash,
                            |_attempt| Ok(data.clone()),
                        )?;
                        // Durably merge this verified range into the union journal.
                        let range = ByteRange { offset, length };
                        let track = large_ranges
                            .get_mut(&file_id)
                            .expect("large file range tracker is initialized");
                        track.push(range);
                        let journal = self
                            .journal
                            .as_ref()
                            .expect("journal is initialized during handshake");
                        let identity = crate::journal::ResumeIdentity {
                            path: file_entry.path.clone().into_bytes(),
                            fingerprint: file_entry.fingerprint,
                        };
                        journal.checkpoint(&identity, track)?;
                        self.ack(writer, frame.message_id, 4)?;
                    } else {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "FileSegment for unregistered file_id {file_id}"
                        )));
                    }
                }
                Message::LargeFilePrepare {
                    file_id,
                    path,
                    size,
                    mtime_ns,
                    mode,
                    fingerprint,
                } => {
                    let rel_path = WirePath::from_wire(path)
                        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
                    validate_unique_destination_path(
                        &self.root,
                        rel_path.clone(),
                        &mut self.seen_destinations,
                    )?;
                    let device = u64::from_le_bytes(fingerprint[0..8].try_into().unwrap_or([0; 8]));
                    let file = u64::from_le_bytes(fingerprint[8..16].try_into().unwrap_or([0; 8]));
                    let mtime = nanos_to_system_time(mtime_ns);
                    let entry = FileEntry {
                        path: rel_path,
                        kind: ScanEntryKind::File,
                        size,
                        mtime,
                        mode,
                        fingerprint: SourceFingerprint {
                            identity: FileIdentity { device, file },
                            kind: ScanEntryKind::File,
                            size,
                            mtime,
                            ctime: None,
                        },
                    };
                    // Idempotent prepare: preserves a matching-size stage the
                    // control session (or another data session) already created.
                    sink.prepare_large(&entry)?;
                    large_files.insert(file_id, entry.clone());
                    large_ranges.insert(file_id, Vec::new());
                    self.ack(writer, frame.message_id, 5)?;
                }
                Message::LargeFileRange {
                    file_id: _,
                    range: _,
                } => {
                    self.ack(writer, frame.message_id, 6)?;
                }
                other => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "data session does not handle {other:?}"
                    )));
                }
            }
        }

        Ok(())
    }

    /// Write an acknowledgement frame, flushing after use.
    fn ack<W: Write>(&mut self, writer: &mut W, id: u64, ack_type: u8) -> Result<(), ServerError> {
        let ack = Message::Ack {
            acknowledged_id: id,
            acknowledged_type: ack_type,
        };
        let msg_id = self.next_id();
        let bytes = encode_frame(msg_id, &ack)?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        Ok(())
    }

    #[allow(clippy::fn_params_excessive_bools)]
    fn run_sink<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        paranoid: bool,
        delete: bool,
        checksum: bool,
        dry_run: bool,
        exclude_patterns: &[Vec<u8>],
    ) -> Result<(), ServerError> {
        let hash_cache = checksum
            .then(|| HashCache::open(HashCache::default_path()).ok())
            .flatten();
        // Destination scan phase: if destination exists, scan and send Scan frames.
        let mut entries = Vec::new();
        if self.root.exists() {
            if let Ok(scan_result) = scan(&self.root) {
                for item in scan_result.entries() {
                    if let Ok(entry) = item {
                        if !excluded_path(exclude_patterns, &entry.path) {
                            entries.push(if checksum {
                                content_entry_record(&self.root, &entry, hash_cache.as_ref())?
                            } else {
                                entry_record_from_file_entry(&entry)
                            });
                        }
                    }
                }
                let _ = scan_result.finish();
            }
        }

        // Send Scan frames in chunks <= MAX_COLLECTION_COUNT.
        if entries.is_empty() {
            let scan_msg = Message::Scan {
                scan_id: 1,
                final_page: true,
                entries: Vec::new(),
            };
            let msg_id = self.next_id();
            let bytes = encode_frame(msg_id, &scan_msg)?;
            writer.write_all(&bytes)?;
            writer.flush()?;

            // Wait for client Ack.
            let frame = self.decoder.read(reader)?;
            if !matches!(frame.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for Scan, got {:?}",
                    frame.message
                )));
            }
        } else {
            let chunks: Vec<Vec<EntryRecord>> = entries
                .chunks(MAX_COLLECTION_COUNT)
                .map(|c| c.to_vec())
                .collect();
            let total_chunks = chunks.len();
            for (idx, chunk) in chunks.into_iter().enumerate() {
                let is_final = idx + 1 == total_chunks;
                let scan_msg = Message::Scan {
                    scan_id: 1,
                    final_page: is_final,
                    entries: chunk,
                };
                let msg_id = self.next_id();
                let bytes = encode_frame(msg_id, &scan_msg)?;
                writer.write_all(&bytes)?;
                writer.flush()?;

                // Wait for client Ack.
                let frame = self.decoder.read(reader)?;
                if !matches!(frame.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for Scan, got {:?}",
                        frame.message
                    )));
                }
            }
        }

        // Initialize sink.
        let sink = Sink::new(&self.root)?;

        // Map of upcoming file_id -> EntryRecord for small/medium and large files.
        let mut active_files: HashMap<u64, EntryRecord> = HashMap::new();
        let mut large_files: HashMap<u64, FileEntry> = HashMap::new();
        // Verified large-file ranges per file_id, for durable checkpointing.
        let mut large_ranges: HashMap<u64, Vec<ByteRange>> = HashMap::new();

        // Process incoming transfer operations.
        loop {
            let frame = match self.decoder.read(reader) {
                Ok(frame) => frame,
                Err(ProtocolError::Read(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(err) => return Err(ServerError::Protocol(err)),
            };

            if dry_run
                && matches!(
                    &frame.message,
                    Message::Metadata { .. }
                        | Message::FileBatch { .. }
                        | Message::FileSegment { .. }
                        | Message::LargeFilePrepare { .. }
                        | Message::LargeFileRange { .. }
                        | Message::LargeFileFinish { .. }
                )
            {
                return Err(ServerError::UnexpectedMessage(
                    "dry-run receiver rejected a mutation message".to_owned(),
                ));
            }

            match frame.message {
                Message::Metadata {
                    operation,
                    path,
                    target,
                    mode,
                    mtime_ns,
                } => {
                    let rel_path = WirePath::from_wire(path)
                        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
                    if !rel_path.is_empty() && !self.seen_destinations.insert(rel_path.clone()) {
                        // Check if duplicate for same operation; allow set directory after creation
                        if operation != MetadataOperation::SetDirectory {
                            return Err(ServerError::DuplicatePath(rel_path.to_string()));
                        }
                    }

                    match operation {
                        MetadataOperation::CreateDirectory => {
                            validate_destination_path(&self.root, &rel_path)?;
                            let entry = FileEntry {
                                path: rel_path.clone(),
                                kind: ScanEntryKind::Directory,
                                size: 0,
                                mtime: nanos_to_system_time(mtime_ns),
                                mode,
                                fingerprint: SourceFingerprint::synthetic(
                                    ScanEntryKind::Directory,
                                    0,
                                    nanos_to_system_time(mtime_ns),
                                ),
                            };
                            sink.create_directories(&[entry])?;
                        }
                        MetadataOperation::CreateSymlink => {
                            validate_destination_path(&self.root, &rel_path)?;
                            let entry = FileEntry {
                                path: rel_path.clone(),
                                kind: ScanEntryKind::Symlink,
                                size: 0,
                                mtime: nanos_to_system_time(mtime_ns),
                                mode,
                                fingerprint: SourceFingerprint::synthetic(
                                    ScanEntryKind::Symlink,
                                    0,
                                    nanos_to_system_time(mtime_ns),
                                ),
                            };
                            sink.create_symlink(
                                &entry,
                                &native_symlink_target(&target),
                                SymlinkTargetKind::File,
                            )?;
                        }
                        MetadataOperation::SetFile => {
                            validate_destination_path(&self.root, &rel_path)?;
                            let dest_file = sink.path_for(&rel_path)?;
                            let time = FileTime::from_system_time(nanos_to_system_time(mtime_ns));
                            set_file_mtime(&dest_file, time)?;
                        }
                        MetadataOperation::SetDirectory => {
                            if rel_path.is_empty() {
                                let entry = FileEntry {
                                    path: WirePath::default(),
                                    kind: ScanEntryKind::Directory,
                                    size: 0,
                                    mtime: nanos_to_system_time(mtime_ns),
                                    mode,
                                    fingerprint: SourceFingerprint::synthetic(
                                        ScanEntryKind::Directory,
                                        0,
                                        nanos_to_system_time(mtime_ns),
                                    ),
                                };
                                sink.finish_root_directory(&entry)?;
                            } else {
                                validate_destination_path(&self.root, &rel_path)?;
                                let entry = FileEntry {
                                    path: rel_path.clone(),
                                    kind: ScanEntryKind::Directory,
                                    size: 0,
                                    mtime: nanos_to_system_time(mtime_ns),
                                    mode,
                                    fingerprint: SourceFingerprint::synthetic(
                                        ScanEntryKind::Directory,
                                        0,
                                        nanos_to_system_time(mtime_ns),
                                    ),
                                };
                                sink.finish_directories(&[entry])?;
                            }
                        }
                        MetadataOperation::Delete => {
                            if delete {
                                validate_destination_path(&self.root, &rel_path)?;
                                let entry = FileEntry {
                                    path: rel_path.clone(),
                                    kind: ScanEntryKind::File,
                                    size: 0,
                                    mtime: UNIX_EPOCH,
                                    mode: 0,
                                    fingerprint: SourceFingerprint::synthetic(
                                        ScanEntryKind::File,
                                        0,
                                        UNIX_EPOCH,
                                    ),
                                };
                                if let Err(error) = sink.delete_entry(&entry) {
                                    let error = Message::Error {
                                        code: 1001,
                                        related_id: frame.message_id,
                                        message: format!("delete '{rel_path}' failed: {error}"),
                                    };
                                    let msg_id = self.next_id();
                                    writer.write_all(&encode_frame(msg_id, &error)?)?;
                                    writer.flush()?;
                                    continue;
                                }
                            }
                        }
                    }

                    // Acknowledge Metadata operation.
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 8, // Metadata
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::FileBatch {
                    batch_id: _,
                    entries,
                } => {
                    for (i, entry) in entries.into_iter().enumerate() {
                        let file_id = (frame.message_id << 16) | (i as u64);
                        active_files.insert(file_id, entry);
                    }
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 3, // FileBatch
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::FileSegment {
                    file_id,
                    offset,
                    data,
                } => {
                    // Check if regular batch file or large file range.
                    if let Some(record) = active_files.remove(&file_id) {
                        // A batched file is a complete-file transfer.  The
                        // offset is not part of this operation and must be
                        // zero; accepting a non-zero offset would silently
                        // turn a malformed/racy sender into a successful
                        // transfer with different semantics.
                        if offset != 0 {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "non-zero offset {offset} for complete file {file_id}"
                            )));
                        }
                        let file_entry = file_entry_from_entry_record(&record)?;
                        validate_unique_destination_path(
                            &self.root,
                            file_entry.path.clone(),
                            &mut self.seen_destinations,
                        )?;
                        let hash = blake3::hash(&data);
                        sink.write_file_with_retry(
                            &file_entry,
                            &hash,
                            |_attempt| Ok(data.clone()),
                        )?;

                        if paranoid {
                            let committed_path = sink.path_for(&file_entry.path)?;
                            let readback = fs::read(&committed_path)?;
                            if blake3::hash(&readback) != hash {
                                return Err(ServerError::Sink(SinkError::VerificationFailed {
                                    path: file_entry.path.to_string(),
                                    attempts: 2,
                                }));
                            }
                        }

                        let ack = Message::Ack {
                            acknowledged_id: frame.message_id,
                            acknowledged_type: 4, // FileSegment
                        };
                        let msg_id = self.next_id();
                        let bytes = encode_frame(msg_id, &ack)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;
                    } else if let Some(file_entry) = large_files.get(&file_id) {
                        let hash = blake3::hash(&data);
                        let length = data.len() as u64;
                        sink.write_chunk_with_retry(
                            file_entry,
                            offset,
                            length,
                            &hash,
                            |_attempt| Ok(data.clone()),
                        )?;

                        // Durably checkpoint this verified range before the ack
                        // that makes it "durably acknowledged".
                        let range = ByteRange { offset, length };
                        let track = large_ranges
                            .get_mut(&file_id)
                            .expect("large file range tracker is initialized");
                        track.push(range);
                        let journal = self
                            .journal
                            .as_ref()
                            .expect("journal is initialized during handshake");
                        let identity = crate::journal::ResumeIdentity {
                            path: file_entry.path.clone().into_bytes(),
                            fingerprint: file_entry.fingerprint,
                        };
                        journal.checkpoint(&identity, track)?;

                        let ack = Message::Ack {
                            acknowledged_id: frame.message_id,
                            acknowledged_type: 4, // FileSegment
                        };
                        let msg_id = self.next_id();
                        let bytes = encode_frame(msg_id, &ack)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;
                    } else {
                        // A data segment with no registered source must not be
                        // silently acknowledged: with a singleton session it is
                        // a protocol-ordering bug; under multi-stream it is the
                        // drop path for a mis-sequenced data session. Make it a
                        // loud failure instead of silent data loss.
                        return Err(ServerError::UnexpectedMessage(format!(
                            "FileSegment for unregistered file_id {file_id}"
                        )));
                    }
                }
                Message::LargeFilePrepare {
                    file_id,
                    path,
                    size,
                    mtime_ns,
                    mode,
                    fingerprint,
                } => {
                    let rel_path = WirePath::from_wire(path)
                        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
                    validate_unique_destination_path(
                        &self.root,
                        rel_path.clone(),
                        &mut self.seen_destinations,
                    )?;
                    let device = u64::from_le_bytes(fingerprint[0..8].try_into().unwrap_or([0; 8]));
                    let file = u64::from_le_bytes(fingerprint[8..16].try_into().unwrap_or([0; 8]));
                    let mtime = nanos_to_system_time(mtime_ns);
                    let entry = FileEntry {
                        path: rel_path.clone(),
                        kind: ScanEntryKind::File,
                        size,
                        mtime,
                        mode,
                        fingerprint: SourceFingerprint {
                            identity: FileIdentity { device, file },
                            kind: ScanEntryKind::File,
                            size,
                            mtime,
                            ctime: None,
                        },
                    };

                    // Durable resume: load prior verified ranges for this exact
                    // file identity from the receiver-side journal.
                    let identity = crate::journal::ResumeIdentity {
                        path: rel_path.clone().into_bytes(),
                        fingerprint: entry.fingerprint,
                    };
                    let loaded = self
                        .journal
                        .as_ref()
                        .expect("journal is initialized during handshake")
                        .load(&identity)?;
                    let existing_ranges: Vec<ByteRange> = loaded
                        .as_ref()
                        .map(|record| record.ranges.clone())
                        .unwrap_or_default();

                    let temp_path = sink.temporary_path(&rel_path)?;
                    let temp_present = temp_path.exists();
                    let busy_ranges;
                    if !existing_ranges.is_empty() && temp_present {
                        // Keep the surviving staging file; the previously
                        // verified ranges are already written to it. Do not
                        // recreate the temp, which would wipe that progress.
                        busy_ranges = existing_ranges;
                    } else {
                        // Fresh or invalidated file: discard stale ranges and
                        // (re)create the staging file.
                        if !existing_ranges.is_empty() {
                            self.journal
                                .as_ref()
                                .expect("journal is initialized during handshake")
                                .invalidate(&identity)?;
                        }
                        busy_ranges = Vec::new();
                        sink.prepare_large(&entry)?;
                    }
                    large_files.insert(file_id, entry.clone());
                    large_ranges.insert(file_id, busy_ranges.clone());

                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 5, // LargeFilePrepare
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;

                    // Send the verified-range resume pages the client uses to
                    // skip already-acknowledged chunks. Always at least one
                    // (possibly empty) final page.
                    let all = busy_ranges.clone();
                    let total_pages = (all.len().div_ceil(MAX_COLLECTION_COUNT)).max(1);
                    let mut p = 0usize;
                    while p < total_pages {
                        let start = p * MAX_COLLECTION_COUNT;
                        let end = (start + MAX_COLLECTION_COUNT).min(all.len());
                        let is_final = p + 1 == total_pages;
                        let ranges = all[start..end].to_vec();
                        let rp = Message::ResumePage {
                            file_id,
                            page: p as u32,
                            final_page: is_final,
                            ranges,
                        };
                        let msg_id = self.next_id();
                        let bytes = encode_frame(msg_id, &rp)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;
                        p += 1;
                    }
                }
                Message::LargeFileRange {
                    file_id: _,
                    range: _,
                } => {
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 6, // LargeFileRange
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::LargeFileFinish { file_id, digest } => {
                    let Some(entry) = large_files.remove(&file_id) else {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "LargeFileFinish for unregistered file_id {file_id}"
                        )));
                    };
                    {
                        // `prepare_large` preallocates the staging file, so a
                        // finish without complete coverage would otherwise
                        // publish zero-filled holes as if they were received.
                        // Require one merged range covering the entire file
                        // before making the atomic commit.
                        let mut ranges = large_ranges.get(&file_id).cloned().unwrap_or_default();
                        // In multi-stream mode, data sessions checkpoint into
                        // the shared journal after this control session has
                        // received Prepare. Refresh from disk so the barrier
                        // validates ranges actually written by those peers.
                        let identity = crate::journal::ResumeIdentity {
                            path: entry.path.as_bytes().to_vec(),
                            fingerprint: entry.fingerprint,
                        };
                        if let Some(record) = self
                            .journal
                            .as_ref()
                            .expect("journal is initialized during handshake")
                            .load(&identity)?
                        {
                            ranges = crate::journal::merge_ranges(&ranges, &record.ranges);
                        }
                        let fully_covered = ranges_cover_file(entry.size, &ranges);
                        if !fully_covered {
                            large_ranges.remove(&file_id);
                            return Err(ServerError::UnexpectedMessage(format!(
                                "LargeFileFinish for file_id {file_id} has incomplete byte coverage"
                            )));
                        }
                        sink.finish_large(&entry)?;
                        // The file is committed; discard its resume record.
                        let journal = self
                            .journal
                            .as_ref()
                            .expect("journal is initialized during handshake");
                        let identity = crate::journal::ResumeIdentity {
                            path: entry.path.as_bytes().to_vec(),
                            fingerprint: entry.fingerprint,
                        };
                        journal.clear(&identity)?;
                        large_ranges.remove(&file_id);
                        if paranoid {
                            let committed_path = sink.path_for(&entry.path)?;
                            let readback = fs::read(&committed_path)?;
                            if *blake3::hash(&readback).as_bytes() != digest {
                                return Err(ServerError::Sink(SinkError::VerificationFailed {
                                    path: entry.path.to_string(),
                                    attempts: 2,
                                }));
                            }
                        }
                    }
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 7, // LargeFileFinish
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::Stats {
                    files,
                    bytes: byte_count,
                    skipped,
                    warnings,
                    failed,
                } => {
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 10, // Stats
                    };
                    let msg_id = self.next_id();
                    let b = encode_frame(msg_id, &ack)?;
                    writer.write_all(&b)?;

                    let reply_stats = Message::Stats {
                        files,
                        bytes: byte_count,
                        skipped,
                        warnings,
                        failed,
                    };
                    let msg_id = self.next_id();
                    let b = encode_frame(msg_id, &reply_stats)?;
                    writer.write_all(&b)?;
                    writer.flush()?;
                    break;
                }
                Message::Error { code, message, .. } => {
                    eprintln!("server received error {code}: {message}");
                    return Err(ServerError::RemoteError { code, message });
                }
                other => {
                    return Err(ServerError::UnexpectedMessage(format!("{other:?}")));
                }
            }
        }

        Ok(())
    }

    fn run_source<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), ServerError> {
        // Source scan phase: scan source root and stream Scan frames to client.
        let mut entries = Vec::new();
        if self.root.exists() {
            let root_meta = fs::symlink_metadata(&self.root)?;
            if root_meta.is_dir() && !root_meta.file_type().is_symlink() {
                let mtime = root_meta.modified()?;
                let mode = permission_mode(&root_meta);
                entries.push(EntryRecord {
                    path: Vec::new(),
                    kind: crate::protocol::EntryKind::Directory,
                    size: 0,
                    mtime_ns: system_time_to_nanos(mtime),
                    mode,
                    fingerprint: [0u8; 32],
                });
            }
            let scan_result = scan(&self.root)?;
            for item in scan_result.entries() {
                let entry = item?;
                entries.push(entry_record_from_file_entry(&entry));
            }
            scan_result.finish()?;
        }

        let chunks: Vec<Vec<EntryRecord>> = if entries.is_empty() {
            vec![Vec::new()]
        } else {
            entries
                .chunks(MAX_COLLECTION_COUNT)
                .map(|c| c.to_vec())
                .collect()
        };
        let total_chunks = chunks.len();
        for (idx, chunk) in chunks.into_iter().enumerate() {
            let is_final = idx + 1 == total_chunks;
            let scan_msg = Message::Scan {
                scan_id: 1,
                final_page: is_final,
                entries: chunk,
            };
            let msg_id = self.next_id();
            let bytes = encode_frame(msg_id, &scan_msg)?;
            writer.write_all(&bytes)?;
            writer.flush()?;

            // Wait for client Ack.
            let frame = self.decoder.read(reader)?;
            if !matches!(frame.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for Scan, got {:?}",
                    frame.message
                )));
            }
        }

        let source_reader = SourceReader::new(&self.root);
        let mut large_source_files: HashMap<u64, FileEntry> = HashMap::new();

        // Process request messages from client.
        loop {
            let frame = match self.decoder.read(reader) {
                Ok(frame) => frame,
                Err(ProtocolError::Read(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(err) => return Err(ServerError::Protocol(err)),
            };

            match frame.message {
                Message::FileBatch {
                    batch_id: _,
                    entries,
                } => {
                    // Segments stream out without stopping for each
                    // acknowledgement; replies are drained on a bounded window
                    // so the client's pending acknowledgements always fit in
                    // the channel buffer.
                    let mut outstanding = 0usize;
                    for entry_rec in entries {
                        let file_entry = file_entry_from_entry_record(&entry_rec)?;
                        let stable_read = source_reader.read(&file_entry)?;
                        let file_id = frame.message_id;

                        // Send file segment.
                        let seg = Message::FileSegment {
                            file_id,
                            offset: 0,
                            data: stable_read.bytes,
                        };
                        let msg_id = self.next_id();
                        write_data_frame(
                            writer,
                            msg_id,
                            &seg,
                            self.compression == CompressionMode::Zstd,
                            self.compression_level,
                        )?;
                        outstanding += 1;
                        if outstanding >= MAX_PIPELINED_FRAMES {
                            drain_acks(
                                &mut self.decoder,
                                reader,
                                &mut outstanding,
                                MAX_PIPELINED_FRAMES / 2,
                            )?;
                        }
                    }
                    drain_acks(&mut self.decoder, reader, &mut outstanding, 0)?;

                    // Acknowledge the batch.
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 3,
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::LargeFilePrepare {
                    file_id,
                    path,
                    size,
                    mtime_ns,
                    mode,
                    fingerprint,
                } => {
                    let rel_path = WirePath::from_wire(path)
                        .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
                    let device = u64::from_le_bytes(fingerprint[0..8].try_into().unwrap_or([0; 8]));
                    let file = u64::from_le_bytes(fingerprint[8..16].try_into().unwrap_or([0; 8]));
                    let entry = FileEntry {
                        path: rel_path,
                        kind: ScanEntryKind::File,
                        size,
                        mtime: nanos_to_system_time(mtime_ns),
                        mode,
                        fingerprint: SourceFingerprint {
                            identity: FileIdentity { device, file },
                            kind: ScanEntryKind::File,
                            size,
                            mtime: nanos_to_system_time(mtime_ns),
                            ctime: None,
                        },
                    };
                    large_source_files.insert(file_id, entry);

                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 5,
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::LargeFileRange { file_id, range } => {
                    if let Some(entry) = large_source_files.get(&file_id) {
                        let stable = source_reader.read(entry)?;
                        let start = usize::try_from(range.offset).unwrap_or(0);
                        let end = usize::try_from(range.offset.saturating_add(range.length))
                            .unwrap_or(stable.bytes.len());
                        let slice = &stable.bytes[start..std::cmp::min(end, stable.bytes.len())];

                        let seg = Message::FileSegment {
                            file_id,
                            offset: range.offset,
                            data: slice.to_vec(),
                        };
                        let msg_id = self.next_id();
                        write_data_frame(
                            writer,
                            msg_id,
                            &seg,
                            self.compression == CompressionMode::Zstd,
                            self.compression_level,
                        )?;

                        // Wait for client Ack for segment.
                        let ack_frame = self.decoder.read(reader)?;
                        if !matches!(ack_frame.message, Message::Ack { .. }) {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "expected Ack for segment, got {:?}",
                                ack_frame.message
                            )));
                        }
                    }

                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 6,
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::LargeFileFinish { file_id, digest: _ } => {
                    large_source_files.remove(&file_id);
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 7,
                    };
                    let msg_id = self.next_id();
                    let bytes = encode_frame(msg_id, &ack)?;
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Message::Metadata {
                    operation,
                    path,
                    target: _,
                    mode: _,
                    mtime_ns: _,
                } => {
                    // Client requesting symlink metadata in Pull mode.
                    if operation == MetadataOperation::CreateSymlink {
                        let rel_path = WirePath::from_wire(path)
                            .map_err(|error| ServerError::InvalidPath(error.to_string()))?;
                        let symlink_path = rel_path.to_native_path(&self.root);
                        let target_path = fs::read_link(&symlink_path)?;
                        let target_bytes = target_path.into_os_string().into_encoded_bytes();
                        let metadata = fs::symlink_metadata(&symlink_path)?;
                        let mode = permission_mode(&metadata);
                        let mtime = metadata.modified()?;

                        let reply = Message::Metadata {
                            operation: MetadataOperation::CreateSymlink,
                            path: rel_path.into_bytes(),
                            target: target_bytes,
                            mode,
                            mtime_ns: system_time_to_nanos(mtime),
                        };
                        let msg_id = self.next_id();
                        let bytes = encode_frame(msg_id, &reply)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;
                    } else {
                        let ack = Message::Ack {
                            acknowledged_id: frame.message_id,
                            acknowledged_type: 8,
                        };
                        let msg_id = self.next_id();
                        let bytes = encode_frame(msg_id, &ack)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;
                    }
                }
                Message::Stats {
                    files,
                    bytes: byte_count,
                    skipped,
                    warnings,
                    failed,
                } => {
                    let ack = Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 10,
                    };
                    let msg_id = self.next_id();
                    let b = encode_frame(msg_id, &ack)?;
                    writer.write_all(&b)?;

                    let reply_stats = Message::Stats {
                        files,
                        bytes: byte_count,
                        skipped,
                        warnings,
                        failed,
                    };
                    let msg_id = self.next_id();
                    let b = encode_frame(msg_id, &reply_stats)?;
                    writer.write_all(&b)?;
                    writer.flush()?;
                    break;
                }
                Message::Error { code, message, .. } => {
                    eprintln!("server received error {code}: {message}");
                    return Err(ServerError::RemoteError { code, message });
                }
                other => {
                    return Err(ServerError::UnexpectedMessage(format!("{other:?}")));
                }
            }
        }

        Ok(())
    }
}

/// Run `xsync --server` on standard I/O.
///
/// # Errors
/// Returns [`ServerError`] on failure.
pub fn run_server_stdio(root: PathBuf) -> Result<(), ServerError> {
    server_log(format_args!(
        "process started: pid={}, root={}",
        std::process::id(),
        root.display()
    ));
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut server = Server::new(root);
    let reader = BufReader::new(stdin);
    let mut writer = BufWriter::new(stdout);
    server_log("waiting for client handshake");
    let result = server.run(reader, &mut writer);
    match &result {
        Ok(()) => server_log("session finished successfully"),
        Err(error) => server_log(format_args!("session failed: {error}")),
    }
    result
}

/// Frames the client may leave unacknowledged before it drains replies.
///
/// The receiver acknowledges every frame and blocks once its own writes fill the
/// socket buffer, so a client that writes without ever reading can deadlock
/// against it. An acknowledgement frame is 41 bytes, so this window keeps the
/// peer's pending replies near 10 KiB — comfortably inside an ordinary pipe or
/// SSH channel buffer — while still removing the per-file round trip.
pub const MAX_PIPELINED_FRAMES: usize = 256;

/// Read acknowledgements until at most `limit` frames remain outstanding.
fn drain_acks<R: Read>(
    decoder: &mut FrameDecoder,
    reader: &mut R,
    outstanding: &mut usize,
    limit: usize,
) -> Result<(), ServerError> {
    while *outstanding > limit {
        let frame = decoder
            .read(reader)
            .map_err(|error| map_transport_error(error, 0))?;
        match frame.message {
            Message::Ack { .. } => *outstanding -= 1,
            Message::Error { code, message, .. } => {
                return Err(ServerError::RemoteError { code, message })
            }
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack while draining pipelined writes, got {other:?}"
                )))
            }
        }
    }
    Ok(())
}

/// Run a client-side Push session (local source to remote destination).
///
/// # Errors
/// Returns [`ServerError`] on transfer or protocol failures.
#[allow(clippy::too_many_lines)]
pub fn run_client_push<R: Read, W: Write, F: FnMut(LocalEvent)>(
    source_path: &Path,
    source_trailing_slash: bool,
    dest_path: &str,
    _dest_trailing_slash: bool,
    options: &LocalSyncOptions,
    mut reader: R,
    mut writer: W,
    mut emit: F,
) -> Result<LocalSyncReport, ServerError> {
    let mut decoder = FrameDecoder::new();
    let mut next_message_id = 1u64;
    let mut alloc_id = || {
        let id = next_message_id;
        next_message_id = next_message_id.saturating_add(1);
        id
    };

    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams: options.streams,
    });
    // The remote routes do not implement placeholder detection at all: the
    // counts are structurally zero, not an inventory. See the note on
    // `LocalEvent::CloudPlaceholders::detection_performed`.
    emit(LocalEvent::CloudPlaceholders {
        files: 0,
        bytes: 0,
        detection_available: cfg!(target_os = "macos"),
        detection_performed: false,
    });

    // 1. Send Handshake (Client is Source).
    let local_capabilities = CAP_ZSTD | CAP_VERSION_NEGOTIATION;
    let handshake = Message::Handshake {
        role: Role::Source,
        capabilities: local_capabilities,
        max_payload: MAX_COMPLETE_PAYLOAD as u32,
        max_segment: MAX_DATA_SEGMENT as u32,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        job_id: session_job_id(source_path.to_string_lossy().as_ref(), dest_path),
        compression: if options.compress {
            CompressionMode::Zstd
        } else {
            CompressionMode::None
        },
        compression_level: options.compress_level,
    };
    let hs_id = alloc_id();
    let bytes = encode_frame(hs_id, &handshake)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Server Handshake and Ack.
    let frame1 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    let (negotiated_compression, negotiated_level, remote_capabilities) = match frame1.message {
        Message::Handshake {
            compression,
            compression_level,
            capabilities,
            ..
        } => (compression, compression_level, capabilities),
        other => {
            return Err(ServerError::UnexpectedMessage(format!(
                "expected Server Handshake, got {other:?}"
            )))
        }
    };
    let frame2 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame2.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Handshake, got {:?}",
            frame2.message
        )));
    }
    emit(LocalEvent::Negotiated {
        compression_algorithm: if negotiated_compression == CompressionMode::Zstd {
            "zstd"
        } else {
            "none"
        },
        compression_reason: if !options.compress {
            "compression disabled by user"
        } else if remote_capabilities & CAP_ZSTD == 0 {
            "remote peer does not advertise zstd"
        } else {
            "both peers advertise zstd"
        },
    });
    let selected_version = negotiate_protocol_version(local_capabilities, remote_capabilities);
    emit(LocalEvent::ProtocolNegotiated {
        selected_version,
        remote_capabilities,
        common_capabilities: common_capabilities(local_capabilities, remote_capabilities),
        browse_available: selected_version >= 2,
    });

    // 2. Send SessionConfig.
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: options.checksum,
        paranoid: options.paranoid,
        dry_run: options.dry_run,
        exclude_patterns: encode_exclude_patterns(&options.exclude_patterns),
    };
    let sc_id = alloc_id();
    let bytes = encode_frame(sc_id, &session_config)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Ack for SessionConfig.
    let frame3 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame3.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for SessionConfig, got {:?}",
            frame3.message
        )));
    }

    // 3. Receive Scan pages from server.
    emit(LocalEvent::Phase {
        name: "scan",
        started: true,
    });
    let mut dest_entries = Vec::new();
    loop {
        let frame = decoder
            .read(&mut reader)
            .map_err(|e| map_transport_error(e, 0))?;
        match frame.message {
            Message::Scan {
                scan_id: _,
                final_page,
                entries,
            } => {
                for rec in entries {
                    let entry = file_entry_from_entry_record(&rec)?;
                    if !excluded_path(
                        &encode_exclude_patterns(&options.exclude_patterns),
                        &entry.path,
                    ) {
                        dest_entries.push(entry);
                    }
                }
                // Send Ack for Scan page.
                let ack = Message::Ack {
                    acknowledged_id: frame.message_id,
                    acknowledged_type: 9,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &ack)?;
                writer.write_all(&b)?;
                writer.flush()?;

                if final_page {
                    break;
                }
            }
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Scan, got {other:?}"
                )))
            }
        }
    }

    // Build DestinationIndex.
    let mut dest_index = DestinationIndex::with_config(IndexConfig {
        memory_budget_bytes: 32 * 1024 * 1024,
        temp_root: std::env::temp_dir(),
    })?;
    for entry in dest_entries {
        dest_index.insert(entry)?;
    }

    // Scan local source root. A single-file source is scanned relative to its
    // parent directory, so all subsequent reads must use that same root;
    // otherwise `file` is incorrectly reopened as `file/file`.
    let source_metadata = fs::symlink_metadata(source_path)?;
    let source_is_dir = source_metadata.is_dir() && !source_metadata.file_type().is_symlink();
    let source_reader_root = if source_is_dir {
        source_path.to_path_buf()
    } else {
        source_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let hash_cache = options
        .checksum
        .then(|| HashCache::open(HashCache::default_path()).ok())
        .flatten();
    let source_scan = scan(source_path)?;
    let mut source_entries = Vec::new();
    for item in source_scan.entries() {
        let mut entry = item?;
        if options.checksum && entry.kind == ScanEntryKind::File {
            entry.fingerprint.identity = cached_content_identity(
                &entry.path.to_native_path(&source_reader_root),
                &entry,
                hash_cache.as_ref(),
            )?;
        }
        if !excluded_path(
            &encode_exclude_patterns(&options.exclude_patterns),
            &entry.path,
        ) {
            source_entries.push(entry);
        }
    }
    source_scan.finish()?;
    emit(LocalEvent::Phase {
        name: "scan",
        started: false,
    });

    // Map source entries relative to destination root according to trailing-slash rules.
    let prefix = if source_is_dir {
        if source_trailing_slash {
            String::new()
        } else {
            source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_owned()
        }
    } else {
        // A single-file scan already reports the basename as its relative
        // path. Adding another basename here creates `file/file` when the
        // destination is a directory.
        String::new()
    };

    let mut mapped_source = Vec::new();
    for mut entry in source_entries {
        if !prefix.is_empty() {
            entry.path = entry.path.with_prefix(&WirePath::from(prefix.as_str()));
        }
        mapped_source.push(entry);
    }

    // Plan differences.
    emit(LocalEvent::Phase {
        name: "plan",
        started: true,
    });
    let plan = try_plan_with_fingerprint(mapped_source, dest_index, options.checksum)?;
    emit(LocalEvent::Phase {
        name: "plan",
        started: false,
    });

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams: options.streams,
        checksum_cache_hits: hash_cache.as_ref().map_or(0, |cache| cache.stats().0),
        checksum_cache_misses: hash_cache.as_ref().map_or(0, |cache| cache.stats().1),
        ..LocalSyncReport::default()
    };
    report.skipped_files = plan.files.unchanged.len();
    // Resume accounting across all transferred files.
    let mut resumed_bytes_total = 0u64;
    let mut restarted_files_total = 0usize;
    let mut retransmitted_bytes_total = 0u64;
    let mut checkpoint_bytes_total = 0u64;

    let total_plan_files = plan.files.new.len() + plan.files.changed.len();
    let total_plan_bytes: u64 = plan
        .files
        .new
        .iter()
        .chain(&plan.files.changed)
        .map(|e| e.size)
        .sum();
    emit(LocalEvent::Planned {
        files: total_plan_files,
        bytes: total_plan_bytes,
    });

    for entry in &plan.files.unchanged {
        emit(LocalEvent::Skipped {
            path: entry.path.to_string(),
            bytes: entry.size,
        });
    }
    if options.dry_run {
        crate::local::emit_plan_actions(&plan, &mut emit);
    }

    emit(LocalEvent::Phase {
        name: "transfer",
        started: true,
    });
    if !options.dry_run {
        // Create directories.
        // `changed` covers a destination entry whose *type* differs — a file
        // where the source now has a directory. The sink replaces it, but only
        // if the client actually announces it, so both buckets are created.
        let mut dirs_to_create = plan.directories.new.clone();
        dirs_to_create.extend(plan.directories.changed.iter().cloned());
        dirs_to_create.sort_by_key(|d| d.path.len());
        let mut pending = 0usize;
        for dir in dirs_to_create {
            let meta_msg = Message::Metadata {
                operation: MetadataOperation::CreateDirectory,
                path: dir.path.as_bytes().to_vec(),
                target: Vec::new(),
                mode: dir.mode,
                mtime_ns: system_time_to_nanos(dir.mtime),
            };
            let msg_id = alloc_id();
            let b = encode_frame(msg_id, &meta_msg)?;
            writer.write_all(&b)?;
            pending += 1;
            if pending >= MAX_PIPELINED_FRAMES {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    MAX_PIPELINED_FRAMES / 2,
                )?;
            }
        }
        writer.flush()?;
        drain_acks(&mut decoder, &mut reader, &mut pending, 0)?;

        // Create symlinks.
        let mut pending = 0usize;
        for sym in plan.symlinks.new.iter().chain(&plan.symlinks.changed) {
            let local_sym_path = if prefix.is_empty() {
                sym.path.to_native_path(&source_reader_root)
            } else {
                let stripped = sym
                    .path
                    .strip_prefix(format!("{prefix}/"))
                    .unwrap_or_else(|| sym.path.clone());
                stripped.to_native_path(&source_reader_root)
            };
            let target = fs::read_link(&local_sym_path)?;
            let target_bytes = target.into_os_string().into_encoded_bytes();

            let meta_msg = Message::Metadata {
                operation: MetadataOperation::CreateSymlink,
                path: sym.path.as_bytes().to_vec(),
                target: target_bytes,
                mode: sym.mode,
                mtime_ns: system_time_to_nanos(sym.mtime),
            };
            let msg_id = alloc_id();
            let b = encode_frame(msg_id, &meta_msg)?;
            writer.write_all(&b)?;
            pending += 1;
            if pending >= MAX_PIPELINED_FRAMES {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    MAX_PIPELINED_FRAMES / 2,
                )?;
            }
        }
        writer.flush()?;
        drain_acks(&mut decoder, &mut reader, &mut pending, 0)?;

        // Transfer files.
        let source_reader = SourceReader::new(&source_reader_root);

        // Small files are coalesced and pipelined: one metadata frame describes
        // many files, and their segments are written without stopping for each
        // acknowledgement. This removes the two serialized round trips per file
        // that previously dominated small-file transfers over a real link.
        let small_files: Vec<&FileEntry> = plan
            .files
            .new
            .iter()
            .chain(&plan.files.changed)
            .filter(|file| file.size <= SMALL_FILE_LIMIT)
            .collect();
        let mut cursor = 0usize;
        while cursor < small_files.len() {
            let mut loaded: Vec<(&FileEntry, Vec<u8>)> = Vec::new();
            let mut batch_bytes = 0u64;
            while cursor < small_files.len()
                && loaded.len() < MAX_BATCH_FILES
                && (loaded.is_empty()
                    || batch_bytes.saturating_add(small_files[cursor].size) <= BATCH_TARGET_SIZE)
            {
                let file = small_files[cursor];
                cursor += 1;
                let mut file_to_read = file.clone();
                if !prefix.is_empty() {
                    file_to_read.path = file
                        .path
                        .strip_prefix(format!("{prefix}/"))
                        .unwrap_or_else(|| file.path.clone())
                        .clone();
                }
                // Read before the file is announced, so a read failure never
                // leaves the receiver waiting for a segment that never arrives.
                match source_reader.read(&file_to_read) {
                    Ok(stable) => {
                        batch_bytes = batch_bytes.saturating_add(file.size);
                        loaded.push((file, stable.bytes));
                    }
                    Err(err) => {
                        emit(LocalEvent::Failed {
                            path: file.path.to_string(),
                            message: err.to_string(),
                        });
                        report.failed_entries = report.failed_entries.saturating_add(1);
                    }
                }
            }
            if loaded.is_empty() {
                continue;
            }

            let transferred: Vec<(String, u64)> = loaded
                .iter()
                .map(|(file, _)| (file.path.to_string(), file.size))
                .collect();
            let entries = loaded
                .iter()
                .map(|(file, _)| {
                    let mut rec = entry_record_from_file_entry(file);
                    rec.path = file.path.as_bytes().to_vec();
                    rec
                })
                .collect();

            let batch_id = alloc_id();
            let bytes = encode_frame(
                batch_id,
                &Message::FileBatch {
                    batch_id: 1,
                    entries,
                },
            )?;
            writer.write_all(&bytes)?;
            let mut outstanding = 1usize;

            for (index, (_, data)) in loaded.into_iter().enumerate() {
                // The receiver derives the same identity from the batch frame's
                // message ID and the entry's position within it.
                let file_id = (batch_id << 16) | index as u64;
                let seg_msg = Message::FileSegment {
                    file_id,
                    offset: 0,
                    data,
                };
                let msg_id = alloc_id();
                let wire_bytes = write_data_frame(
                    &mut writer,
                    msg_id,
                    &seg_msg,
                    negotiated_compression == CompressionMode::Zstd,
                    negotiated_level,
                )?;
                report.wire_bytes = report.wire_bytes.saturating_add(wire_bytes as u64);
                outstanding += 1;
                if outstanding >= MAX_PIPELINED_FRAMES {
                    writer.flush()?;
                    drain_acks(
                        &mut decoder,
                        &mut reader,
                        &mut outstanding,
                        MAX_PIPELINED_FRAMES / 2,
                    )?;
                }
            }
            writer.flush()?;
            drain_acks(&mut decoder, &mut reader, &mut outstanding, 0)?;

            for (path, size) in transferred {
                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(size);
                report.physical_bytes = report.physical_bytes.saturating_add(size);
                report.byte_copies = report.byte_copies.saturating_add(1);
                emit(LocalEvent::Transferred {
                    path,
                    bytes: size,
                    physical_bytes: size,
                    method: TransferMethod::ByteCopy,
                });
            }
        }

        for file in plan.files.new.iter().chain(&plan.files.changed) {
            if file.size <= SMALL_FILE_LIMIT {
                // Already sent by the coalesced small-file pass above.
                continue;
            }
            let mut file_to_read = file.clone();
            if !prefix.is_empty() {
                file_to_read.path = file
                    .path
                    .strip_prefix(format!("{prefix}/"))
                    .unwrap_or_else(|| file.path.clone())
                    .clone();
            }

            let stable = match source_reader.read(&file_to_read) {
                Ok(s) => s,
                Err(err) => {
                    emit(LocalEvent::Failed {
                        path: file.path.to_string(),
                        message: err.to_string(),
                    });
                    report.failed_entries = report.failed_entries.saturating_add(1);
                    continue;
                }
            };

            if file.size <= MAX_DATA_SEGMENT as u64 {
                // Small / medium file: send batch + segment.
                let mut rec = entry_record_from_file_entry(file);
                rec.path = file.path.as_bytes().to_vec();
                let batch_msg = Message::FileBatch {
                    batch_id: 1,
                    entries: vec![rec],
                };
                let batch_id = alloc_id();
                let b = encode_frame(batch_id, &batch_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for FileBatch, got {:?}",
                        ack.message
                    )));
                }

                let file_id = (batch_id << 16) | 0;
                let seg_msg = Message::FileSegment {
                    file_id,
                    offset: 0,
                    data: stable.bytes,
                };
                let msg_id = alloc_id();
                let wire_bytes = write_data_frame(
                    &mut writer,
                    msg_id,
                    &seg_msg,
                    negotiated_compression == CompressionMode::Zstd,
                    negotiated_level,
                )?;
                report.wire_bytes = report.wire_bytes.saturating_add(wire_bytes as u64);

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for FileSegment, got {:?}",
                        ack.message
                    )));
                }

                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
                report.physical_bytes = report.physical_bytes.saturating_add(file.size);
                report.byte_copies = report.byte_copies.saturating_add(1);

                emit(LocalEvent::Transferred {
                    path: file.path.to_string(),
                    bytes: file.size,
                    physical_bytes: file.size,
                    method: TransferMethod::ByteCopy,
                });
            } else {
                // Large file: LargeFilePrepare + ranges + LargeFileFinish.
                let mut rec = entry_record_from_file_entry(file);
                rec.path = file.path.as_bytes().to_vec();
                let file_id = alloc_id();
                let prep_msg = Message::LargeFilePrepare {
                    file_id,
                    path: rec.path,
                    size: file.size,
                    mtime_ns: rec.mtime_ns,
                    mode: file.mode,
                    fingerprint: rec.fingerprint,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &prep_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for LargeFilePrepare, got {:?}",
                        ack.message
                    )));
                }

                // Receive resume pages describing already-verified ranges.
                let verified_ranges: Vec<ByteRange> = {
                    let mut all = Vec::new();
                    let mut final_page = false;
                    while !final_page {
                        let frame = decoder
                            .read(&mut reader)
                            .map_err(|e| map_transport_error(e, 0))?;
                        match frame.message {
                            Message::ResumePage {
                                final_page: fp,
                                ranges,
                                ..
                            } => {
                                all.extend(ranges);
                                final_page = fp;
                            }
                            other => {
                                return Err(ServerError::UnexpectedMessage(format!(
                                    "expected ResumePage after LargeFilePrepare, got {other:?}"
                                )));
                            }
                        }
                    }
                    all
                };
                let resumed_bytes: u64 = verified_ranges.iter().map(|range| range.length).sum();
                resumed_bytes_total = resumed_bytes_total.saturating_add(resumed_bytes);
                if resumed_bytes > 0 {
                    restarted_files_total += 1;
                }

                // Send only the chunks that are not yet durably verified.
                let missing =
                    crate::journal::missing_chunks(file.size, 8 * 1024 * 1024, &verified_ranges);
                let mut sent_bytes = 0u64;
                for range in missing {
                    let start = usize::try_from(range.offset).unwrap_or(0);
                    let len = usize::try_from(range.length).unwrap_or(0);
                    sent_bytes = sent_bytes.saturating_add(range.length);
                    let range_msg = Message::LargeFileRange {
                        file_id,
                        range: ByteRange {
                            offset: range.offset,
                            length: range.length,
                        },
                    };
                    let msg_id = alloc_id();
                    let b = encode_frame(msg_id, &range_msg)?;
                    writer.write_all(&b)?;

                    let seg_msg = Message::FileSegment {
                        file_id,
                        offset: range.offset,
                        data: stable.bytes[start..(start + len)].to_vec(),
                    };
                    let msg_id = alloc_id();
                    let wire_bytes = write_data_frame(
                        &mut writer,
                        msg_id,
                        &seg_msg,
                        negotiated_compression == CompressionMode::Zstd,
                        negotiated_level,
                    )?;
                    report.wire_bytes = report.wire_bytes.saturating_add(wire_bytes as u64);

                    let ack1 = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    let ack2 = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    if !matches!(ack1.message, Message::Ack { .. })
                        || !matches!(ack2.message, Message::Ack { .. })
                    {
                        return Err(ServerError::UnexpectedMessage(
                            "expected Ack for LargeFileRange/Segment".to_owned(),
                        ));
                    }
                    emit(LocalEvent::Progress {
                        path: file.path.to_string(),
                        stream: 0,
                        completed: resumed_bytes.saturating_add(sent_bytes),
                        total: file.size,
                    });
                }
                retransmitted_bytes_total = retransmitted_bytes_total.saturating_add(sent_bytes);
                checkpoint_bytes_total =
                    checkpoint_bytes_total.saturating_add(resumed_bytes.saturating_add(sent_bytes));

                let finish_msg = Message::LargeFileFinish {
                    file_id,
                    digest: *stable.blake3.as_bytes(),
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &finish_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for LargeFileFinish, got {:?}",
                        ack.message
                    )));
                }

                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
                report.physical_bytes = report.physical_bytes.saturating_add(sent_bytes);
                report.byte_copies = report.byte_copies.saturating_add(1);

                emit(LocalEvent::Transferred {
                    path: file.path.to_string(),
                    bytes: file.size,
                    physical_bytes: sent_bytes,
                    method: TransferMethod::ByteCopy,
                });
            }
        }

        // Deletions update parent directory mtimes, so perform them before the
        // final directory metadata pass.
        if options.delete && !report.partial_failure() {
            let mut to_delete = Vec::new();
            to_delete.extend(plan.files.extraneous.clone());
            to_delete.extend(plan.symlinks.extraneous.clone());
            to_delete.extend(plan.other.extraneous.clone());
            let mut ext_dirs = plan.directories.extraneous.clone();
            ext_dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
            to_delete.extend(ext_dirs);

            for entry in to_delete {
                let meta_msg = Message::Metadata {
                    operation: MetadataOperation::Delete,
                    path: entry.path.as_bytes().to_vec(),
                    target: Vec::new(),
                    mode: 0,
                    mtime_ns: 0,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &meta_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;
                let deleted = if let Some(message) =
                    expect_ack_or_delete_warning(&mut decoder, &mut reader)?
                {
                    record_delete_failure(
                        &mut report,
                        &mut emit,
                        &entry,
                        ServerError::RemoteError {
                            code: 1001,
                            message,
                        },
                    );
                    false
                } else {
                    emit(LocalEvent::Deleted {
                        path: entry.path.to_string(),
                    });
                    true
                };
                if deleted {
                    report.deleted_entries = report.deleted_entries.saturating_add(1);
                }
            }
        }

        // Finish directories metadata deepest first.
        let mut dirs_to_finish: Vec<_> = plan
            .directories
            .new
            .iter()
            .chain(&plan.directories.changed)
            .chain(&plan.directories.unchanged)
            .collect();
        dirs_to_finish.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
        let mut pending = 0usize;
        for dir in dirs_to_finish {
            let meta_msg = Message::Metadata {
                operation: MetadataOperation::SetDirectory,
                path: dir.path.as_bytes().to_vec(),
                target: Vec::new(),
                mode: dir.mode,
                mtime_ns: system_time_to_nanos(dir.mtime),
            };
            let msg_id = alloc_id();
            let b = encode_frame(msg_id, &meta_msg)?;
            writer.write_all(&b)?;
            pending += 1;
            if pending >= MAX_PIPELINED_FRAMES {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    MAX_PIPELINED_FRAMES / 2,
                )?;
            }
        }
        writer.flush()?;
        drain_acks(&mut decoder, &mut reader, &mut pending, 0)?;

        // Apply root directory metadata if source is a directory.
        if source_is_dir {
            let meta_msg = Message::Metadata {
                operation: MetadataOperation::SetDirectory,
                path: Vec::new(), // root
                target: Vec::new(),
                mode: permission_mode(&source_metadata),
                mtime_ns: system_time_to_nanos(source_metadata.modified()?),
            };
            let msg_id = alloc_id();
            let b = encode_frame(msg_id, &meta_msg)?;
            writer.write_all(&b)?;
            writer.flush()?;

            let ack = decoder
                .read(&mut reader)
                .map_err(|e| map_transport_error(e, 0))?;
            if !matches!(ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for root SetDirectory, got {:?}",
                    ack.message
                )));
            }
        }
    }

    emit(LocalEvent::Phase {
        name: "transfer",
        started: false,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: true,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: false,
    });

    // Send Stats.
    let stats_msg = Message::Stats {
        files: report.transferred_files as u64,
        bytes: report.transferred_bytes,
        skipped: report.skipped_files as u64,
        warnings: report.warnings as u64,
        failed: report.failed_entries as u64,
    };
    let msg_id = alloc_id();
    let b = encode_frame(msg_id, &stats_msg)?;
    writer.write_all(&b)?;
    writer.flush()?;

    let ack = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(ack.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Stats, got {:?}",
            ack.message
        )));
    }
    let server_stats = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    let _ = server_stats;

    report.resumed_bytes = resumed_bytes_total;
    report.restarted_files = restarted_files_total;
    report.retransmitted_bytes = retransmitted_bytes_total;
    report.checkpoint_bytes = checkpoint_bytes_total;

    emit(LocalEvent::Finished {
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        deleted_entries: report.deleted_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
        directory_clones: 0,
        file_clones: 0,
        byte_copies: report.byte_copies,
        restarted_files: report.restarted_files,
        resumed_bytes: report.resumed_bytes,
        retransmitted_bytes: report.retransmitted_bytes,
        checkpoint_bytes: report.checkpoint_bytes,
        checksum_cache_hits: report.checksum_cache_hits,
        checksum_cache_misses: report.checksum_cache_misses,
    });

    Ok(report)
}

/// Run a client-side Pull session (remote source to local destination).
///
/// # Errors
/// Returns [`ServerError`] on transfer or protocol failures.
#[allow(clippy::too_many_lines)]
pub fn run_client_pull<R: Read, W: Write, F: FnMut(LocalEvent)>(
    src_path: &str,
    src_trailing_slash: bool,
    dest_path: &Path,
    _dest_trailing_slash: bool,
    options: &LocalSyncOptions,
    mut reader: R,
    mut writer: W,
    mut emit: F,
) -> Result<LocalSyncReport, ServerError> {
    let mut decoder = FrameDecoder::new();
    let mut next_message_id = 1u64;
    let mut alloc_id = || {
        let id = next_message_id;
        next_message_id = next_message_id.saturating_add(1);
        id
    };

    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams: options.streams,
    });
    // The remote routes do not implement placeholder detection at all: the
    // counts are structurally zero, not an inventory. See the note on
    // `LocalEvent::CloudPlaceholders::detection_performed`.
    emit(LocalEvent::CloudPlaceholders {
        files: 0,
        bytes: 0,
        detection_available: cfg!(target_os = "macos"),
        detection_performed: false,
    });

    // 1. Send Handshake (Client is Sink).
    let job_id = session_job_id(src_path, dest_path.to_string_lossy().as_ref());
    let resume_journal = crate::journal::ResumeJournal::new(&job_id)?;
    let local_capabilities = CAP_ZSTD | CAP_VERSION_NEGOTIATION;
    let handshake = Message::Handshake {
        role: Role::Sink,
        capabilities: local_capabilities,
        max_payload: MAX_COMPLETE_PAYLOAD as u32,
        max_segment: MAX_DATA_SEGMENT as u32,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        job_id,
        compression: if options.compress {
            CompressionMode::Zstd
        } else {
            CompressionMode::None
        },
        compression_level: options.compress_level,
    };
    let hs_id = alloc_id();
    let bytes = encode_frame(hs_id, &handshake)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Server Handshake and Ack.
    let frame1 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    let (negotiated_compression, remote_capabilities) = match frame1.message {
        Message::Handshake {
            compression,
            capabilities,
            ..
        } => (compression, capabilities),
        other => {
            return Err(ServerError::UnexpectedMessage(format!(
                "expected Server Handshake, got {other:?}"
            )))
        }
    };
    let frame2 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame2.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Handshake, got {:?}",
            frame2.message
        )));
    }
    emit(LocalEvent::Negotiated {
        compression_algorithm: if negotiated_compression == CompressionMode::Zstd {
            "zstd"
        } else {
            "none"
        },
        compression_reason: if !options.compress {
            "compression disabled by user"
        } else if remote_capabilities & CAP_ZSTD == 0 {
            "remote peer does not advertise zstd"
        } else {
            "both peers advertise zstd"
        },
    });
    let selected_version = negotiate_protocol_version(local_capabilities, remote_capabilities);
    emit(LocalEvent::ProtocolNegotiated {
        selected_version,
        remote_capabilities,
        common_capabilities: common_capabilities(local_capabilities, remote_capabilities),
        browse_available: selected_version >= 2,
    });

    // 2. Send SessionConfig.
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: false,
        paranoid: options.paranoid,
        dry_run: options.dry_run,
        exclude_patterns: encode_exclude_patterns(&options.exclude_patterns),
    };
    let sc_id = alloc_id();
    let bytes = encode_frame(sc_id, &session_config)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Ack for SessionConfig.
    let frame3 = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame3.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for SessionConfig, got {:?}",
            frame3.message
        )));
    }

    // 3. Receive source Scan pages from server.
    emit(LocalEvent::Phase {
        name: "scan",
        started: true,
    });
    let mut source_entries = Vec::new();
    let mut source_root_entry: Option<FileEntry> = None;
    loop {
        let frame = decoder
            .read(&mut reader)
            .map_err(|e| map_transport_error(e, 0))?;
        match frame.message {
            Message::Scan {
                scan_id: _,
                final_page,
                entries,
            } => {
                for rec in entries {
                    if rec.path.is_empty() {
                        let mtime = nanos_to_system_time(rec.mtime_ns);
                        source_root_entry = Some(FileEntry {
                            path: WirePath::default(),
                            kind: ScanEntryKind::Directory,
                            size: 0,
                            mtime,
                            mode: rec.mode,
                            fingerprint: SourceFingerprint::synthetic(
                                ScanEntryKind::Directory,
                                0,
                                mtime,
                            ),
                        });
                    } else {
                        let entry = file_entry_from_entry_record(&rec)?;
                        if !excluded_path(
                            &encode_exclude_patterns(&options.exclude_patterns),
                            &entry.path,
                        ) {
                            source_entries.push(entry);
                        }
                    }
                }
                // Send Ack for Scan page.
                let ack = Message::Ack {
                    acknowledged_id: frame.message_id,
                    acknowledged_type: 9,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &ack)?;
                writer.write_all(&b)?;
                writer.flush()?;

                if final_page {
                    break;
                }
            }
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Scan, got {other:?}"
                )))
            }
        }
    }

    // Build local DestinationIndex.
    let mut dest_index = DestinationIndex::with_config(IndexConfig {
        memory_budget_bytes: 32 * 1024 * 1024,
        temp_root: std::env::temp_dir(),
    })?;
    if dest_path.exists() {
        if let Ok(dest_scan) = scan(dest_path) {
            for item in dest_scan.entries() {
                if let Ok(entry) = item {
                    if !excluded_path(
                        &encode_exclude_patterns(&options.exclude_patterns),
                        &entry.path,
                    ) {
                        dest_index.insert(entry)?;
                    }
                }
            }
            let _ = dest_scan.finish();
        }
    }

    emit(LocalEvent::Phase {
        name: "scan",
        started: false,
    });

    // Map source entries according to trailing-slash rules.
    let source_basename = Path::new(src_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let prefix = if src_trailing_slash {
        String::new()
    } else {
        source_basename.to_owned()
    };

    let mut mapped_source = Vec::new();
    for mut entry in source_entries {
        if !prefix.is_empty() {
            entry.path = entry.path.with_prefix(&WirePath::from(prefix.as_str()));
        }
        mapped_source.push(entry);
    }

    // Plan differences.
    emit(LocalEvent::Phase {
        name: "plan",
        started: true,
    });
    let plan = try_plan(mapped_source, dest_index)?;
    emit(LocalEvent::Phase {
        name: "plan",
        started: false,
    });

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams: options.streams,
        ..LocalSyncReport::default()
    };
    report.skipped_files = plan.files.unchanged.len();
    // Resume accounting across all transferred files.
    let mut resumed_bytes_total = 0u64;
    let mut restarted_files_total = 0usize;
    let mut retransmitted_bytes_total = 0u64;
    let mut checkpoint_bytes_total = 0u64;

    let total_plan_files = plan.files.new.len() + plan.files.changed.len();
    let total_plan_bytes: u64 = plan
        .files
        .new
        .iter()
        .chain(&plan.files.changed)
        .map(|e| e.size)
        .sum();
    emit(LocalEvent::Planned {
        files: total_plan_files,
        bytes: total_plan_bytes,
    });

    for entry in &plan.files.unchanged {
        emit(LocalEvent::Skipped {
            path: entry.path.to_string(),
            bytes: entry.size,
        });
    }
    if options.dry_run {
        crate::local::emit_plan_actions(&plan, &mut emit);
    }

    emit(LocalEvent::Phase {
        name: "transfer",
        started: true,
    });
    if !options.dry_run {
        let sink = Sink::new(dest_path)?;

        // Create directories.
        let mut dirs_to_create = plan.directories.new.clone();
        dirs_to_create.sort_by_key(|d| d.path.len());
        sink.create_directories(&dirs_to_create)?;

        // Fetch symlinks.
        for sym in plan.symlinks.new.iter().chain(&plan.symlinks.changed) {
            let raw_path = if prefix.is_empty() {
                sym.path.clone()
            } else {
                sym.path
                    .strip_prefix(format!("{prefix}/"))
                    .unwrap_or_else(|| sym.path.clone())
                    .clone()
            };

            let req = Message::Metadata {
                operation: MetadataOperation::CreateSymlink,
                path: raw_path.into_bytes(),
                target: Vec::new(),
                mode: sym.mode,
                mtime_ns: system_time_to_nanos(sym.mtime),
            };
            let msg_id = alloc_id();
            let b = encode_frame(msg_id, &req)?;
            writer.write_all(&b)?;
            writer.flush()?;

            let reply = decoder
                .read(&mut reader)
                .map_err(|e| map_transport_error(e, 0))?;
            if let Message::Metadata { target, .. } = reply.message {
                sink.create_symlink(
                    sym,
                    &native_symlink_target(&target),
                    SymlinkTargetKind::File,
                )?;
            }
        }

        // Request files from server. Small files are requested as one
        // coalesced batch, so their segments stream back continuously instead
        // of costing a request round trip each.
        let remote_path = |file: &FileEntry| -> String {
            if prefix.is_empty() {
                file.path.to_string()
            } else {
                file.path
                    .strip_prefix(format!("{prefix}/"))
                    .unwrap_or_else(|| file.path.clone())
                    .to_string()
            }
        };
        let small_files: Vec<&FileEntry> = plan
            .files
            .new
            .iter()
            .chain(&plan.files.changed)
            .filter(|file| file.size <= SMALL_FILE_LIMIT)
            .collect();
        let mut cursor = 0usize;
        while cursor < small_files.len() {
            let mut batch: Vec<&FileEntry> = Vec::new();
            let mut batch_bytes = 0u64;
            while cursor < small_files.len()
                && batch.len() < MAX_BATCH_FILES
                && (batch.is_empty()
                    || batch_bytes.saturating_add(small_files[cursor].size) <= BATCH_TARGET_SIZE)
            {
                batch_bytes = batch_bytes.saturating_add(small_files[cursor].size);
                batch.push(small_files[cursor]);
                cursor += 1;
            }

            let entries = batch
                .iter()
                .map(|file| {
                    let mut rec = entry_record_from_file_entry(file);
                    rec.path = remote_path(file).into_bytes();
                    rec
                })
                .collect();
            let batch_id = alloc_id();
            let bytes = encode_frame(
                batch_id,
                &Message::FileBatch {
                    batch_id: 1,
                    entries,
                },
            )?;
            writer.write_all(&bytes)?;
            writer.flush()?;

            for file in &batch {
                let seg_frame = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                report.wire_bytes = report
                    .wire_bytes
                    .saturating_add(decoder.last_wire_bytes() as u64);
                let data = match seg_frame.message {
                    Message::FileSegment { data, .. } => data,
                    other => {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected FileSegment, got {other:?}"
                        )))
                    }
                };
                let hash = blake3::hash(&data);
                sink.write_file_with_retry(file, &hash, |_attempt| Ok(data.clone()))?;
                if options.paranoid {
                    let committed_path = sink.path_for(&file.path)?;
                    let readback = fs::read(&committed_path)?;
                    if blake3::hash(&readback) != hash {
                        return Err(ServerError::Sink(SinkError::VerificationFailed {
                            path: file.path.to_string(),
                            attempts: 2,
                        }));
                    }
                }
                let ack = Message::Ack {
                    acknowledged_id: seg_frame.message_id,
                    acknowledged_type: 4,
                };
                let msg_id = alloc_id();
                let bytes = encode_frame(msg_id, &ack)?;
                writer.write_all(&bytes)?;
                writer.flush()?;

                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
                report.physical_bytes = report.physical_bytes.saturating_add(file.size);
                report.byte_copies = report.byte_copies.saturating_add(1);
                emit(LocalEvent::Transferred {
                    path: file.path.to_string(),
                    bytes: file.size,
                    physical_bytes: file.size,
                    method: TransferMethod::ByteCopy,
                });
            }

            let batch_ack = decoder
                .read(&mut reader)
                .map_err(|e| map_transport_error(e, 0))?;
            if !matches!(batch_ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for FileBatch, got {:?}",
                    batch_ack.message
                )));
            }
        }

        for file in plan.files.new.iter().chain(&plan.files.changed) {
            if file.size <= SMALL_FILE_LIMIT {
                // Already retrieved by the coalesced small-file pass above.
                continue;
            }
            let raw_path = remote_path(file);

            if file.size <= MAX_DATA_SEGMENT as u64 {
                let mut rec = entry_record_from_file_entry(file);
                rec.path = raw_path.into_bytes();

                let batch_msg = Message::FileBatch {
                    batch_id: 1,
                    entries: vec![rec],
                };
                let batch_id = alloc_id();
                let b = encode_frame(batch_id, &batch_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                // Server sends FileSegment.
                let seg_frame = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                report.wire_bytes = report
                    .wire_bytes
                    .saturating_add(decoder.last_wire_bytes() as u64);
                let data = match seg_frame.message {
                    Message::FileSegment { data, .. } => data,
                    other => {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected FileSegment, got {other:?}"
                        )))
                    }
                };

                // Write and commit file locally with Sink.
                let hash = blake3::hash(&data);
                sink.write_file_with_retry(file, &hash, |_attempt| Ok(data.clone()))?;

                if options.paranoid {
                    let committed_path = sink.path_for(&file.path)?;
                    let readback = fs::read(&committed_path)?;
                    if blake3::hash(&readback) != hash {
                        return Err(ServerError::Sink(SinkError::VerificationFailed {
                            path: file.path.to_string(),
                            attempts: 2,
                        }));
                    }
                }

                // Send Ack for segment.
                let ack = Message::Ack {
                    acknowledged_id: seg_frame.message_id,
                    acknowledged_type: 4,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &ack)?;
                writer.write_all(&b)?;
                writer.flush()?;

                // Read batch Ack.
                let batch_ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(batch_ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for FileBatch, got {:?}",
                        batch_ack.message
                    )));
                }

                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
                report.physical_bytes = report.physical_bytes.saturating_add(file.size);
                report.byte_copies = report.byte_copies.saturating_add(1);

                emit(LocalEvent::Transferred {
                    path: file.path.to_string(),
                    bytes: file.size,
                    physical_bytes: file.size,
                    method: TransferMethod::ByteCopy,
                });
            } else {
                // Durable resume: load locally-verified ranges for this file
                // from the receiver-side journal so already-committed chunks
                // are not retransmitted after a crash.
                let identity = crate::journal::ResumeIdentity {
                    path: file.path.as_bytes().to_vec(),
                    fingerprint: file.fingerprint,
                };
                let loaded = resume_journal.load(&identity)?;
                let verified_ranges: Vec<ByteRange> = loaded
                    .as_ref()
                    .map(|record| record.ranges.clone())
                    .unwrap_or_default();
                let temp_path = sink.temporary_path(&file.path)?;
                if verified_ranges.is_empty() || !temp_path.exists() {
                    if !verified_ranges.is_empty() {
                        resume_journal.invalidate(&identity)?;
                    }
                    sink.prepare_large(file)?;
                }
                let mut track = verified_ranges.clone();
                let resumed_bytes: u64 = verified_ranges.iter().map(|r| r.length).sum();
                resumed_bytes_total = resumed_bytes_total.saturating_add(resumed_bytes);
                if resumed_bytes > 0 {
                    restarted_files_total += 1;
                }

                let mut rec = entry_record_from_file_entry(file);
                rec.path = raw_path.into_bytes();
                let file_id = alloc_id();
                let prep_msg = Message::LargeFilePrepare {
                    file_id,
                    path: rec.path,
                    size: file.size,
                    mtime_ns: rec.mtime_ns,
                    mode: file.mode,
                    fingerprint: rec.fingerprint,
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &prep_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for LargeFilePrepare, got {:?}",
                        ack.message
                    )));
                }

                // Request and receive only the still-missing chunks.
                let missing =
                    crate::journal::missing_chunks(file.size, 8 * 1024 * 1024, &verified_ranges);
                let mut sent_bytes = 0u64;
                for range in missing {
                    let offset = range.offset;
                    let length = range.length;
                    sent_bytes = sent_bytes.saturating_add(length);
                    let range_msg = Message::LargeFileRange {
                        file_id,
                        range: ByteRange { offset, length },
                    };
                    let msg_id = alloc_id();
                    let b = encode_frame(msg_id, &range_msg)?;
                    writer.write_all(&b)?;
                    writer.flush()?;

                    let seg_frame = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    report.wire_bytes = report
                        .wire_bytes
                        .saturating_add(decoder.last_wire_bytes() as u64);
                    let data = match seg_frame.message {
                        Message::FileSegment { data, .. } => data,
                        other => {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "expected FileSegment, got {other:?}"
                            )))
                        }
                    };

                    let hash = blake3::hash(&data);
                    sink.write_chunk_with_retry(file, offset, length, &hash, |_attempt| {
                        Ok(data.clone())
                    })?;

                    // Durably checkpoint this verified range before its ack.
                    track.push(ByteRange { offset, length });
                    resume_journal.checkpoint(&identity, &track)?;

                    // Send Ack for segment.
                    let ack = Message::Ack {
                        acknowledged_id: seg_frame.message_id,
                        acknowledged_type: 4,
                    };
                    let msg_id = alloc_id();
                    let b = encode_frame(msg_id, &ack)?;
                    writer.write_all(&b)?;
                    writer.flush()?;

                    // Read range Ack.
                    let range_ack = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    if !matches!(range_ack.message, Message::Ack { .. }) {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected Ack for LargeFileRange, got {:?}",
                            range_ack.message
                        )));
                    }
                    emit(LocalEvent::Progress {
                        path: file.path.to_string(),
                        stream: 0,
                        completed: verified_ranges
                            .iter()
                            .map(|r| r.length)
                            .sum::<u64>()
                            .saturating_add(sent_bytes),
                        total: file.size,
                    });
                }
                retransmitted_bytes_total = retransmitted_bytes_total.saturating_add(sent_bytes);
                checkpoint_bytes_total =
                    checkpoint_bytes_total.saturating_add(resumed_bytes.saturating_add(sent_bytes));

                sink.finish_large(file)?;
                resume_journal.clear(&identity)?;

                if options.paranoid {
                    let committed_path = sink.path_for(&file.path)?;
                    let readback = fs::read(&committed_path)?;
                    let _ = readback;
                }

                let finish_msg = Message::LargeFileFinish {
                    file_id,
                    digest: [0u8; 32],
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &finish_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for LargeFileFinish, got {:?}",
                        ack.message
                    )));
                }

                report.transferred_files = report.transferred_files.saturating_add(1);
                report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
                report.physical_bytes = report.physical_bytes.saturating_add(sent_bytes);
                report.byte_copies = report.byte_copies.saturating_add(1);

                emit(LocalEvent::Transferred {
                    path: file.path.to_string(),
                    bytes: file.size,
                    physical_bytes: sent_bytes,
                    method: TransferMethod::ByteCopy,
                });
            }
        }

        // Finish local directory metadata.
        let dirs_to_finish: Vec<_> = plan
            .directories
            .new
            .iter()
            .chain(&plan.directories.changed)
            .chain(&plan.directories.unchanged)
            .cloned()
            .collect();
        sink.finish_directories(&dirs_to_finish)?;

        if let Some(ref root_entry) = source_root_entry {
            sink.finish_root_directory(root_entry)?;
        }

        // Delete extraneous entries if enabled.
        if options.delete && !report.partial_failure() {
            for entry in &plan.files.extraneous {
                match sink.delete_entry(entry) {
                    Ok(()) => emit(LocalEvent::Deleted {
                        path: entry.path.to_string(),
                    }),
                    Err(error) => record_delete_failure(&mut report, &mut emit, entry, error),
                }
            }
            for entry in &plan.symlinks.extraneous {
                match sink.delete_entry(entry) {
                    Ok(()) => emit(LocalEvent::Deleted {
                        path: entry.path.to_string(),
                    }),
                    Err(error) => record_delete_failure(&mut report, &mut emit, entry, error),
                }
            }
            let mut ext_dirs = plan.directories.extraneous.clone();
            ext_dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
            for entry in &ext_dirs {
                match sink.delete_entry(entry) {
                    Ok(()) => emit(LocalEvent::Deleted {
                        path: entry.path.to_string(),
                    }),
                    Err(error) => record_delete_failure(&mut report, &mut emit, entry, error),
                }
            }
        }
    }

    emit(LocalEvent::Phase {
        name: "transfer",
        started: false,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: true,
    });
    emit(LocalEvent::Phase {
        name: "metadata",
        started: false,
    });

    // Send Stats.
    let stats_msg = Message::Stats {
        files: report.transferred_files as u64,
        bytes: report.transferred_bytes,
        skipped: report.skipped_files as u64,
        warnings: report.warnings as u64,
        failed: report.failed_entries as u64,
    };
    let msg_id = alloc_id();
    let b = encode_frame(msg_id, &stats_msg)?;
    writer.write_all(&b)?;
    writer.flush()?;

    let ack = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(ack.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Stats, got {:?}",
            ack.message
        )));
    }
    let _server_stats = decoder
        .read(&mut reader)
        .map_err(|e| map_transport_error(e, 0))?;

    report.resumed_bytes = resumed_bytes_total;
    report.restarted_files = restarted_files_total;
    report.retransmitted_bytes = retransmitted_bytes_total;
    report.checkpoint_bytes = checkpoint_bytes_total;

    emit(LocalEvent::Finished {
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        deleted_entries: report.deleted_entries,
        warnings: report.warnings,
        local_workers: report.local_workers,
        streams: report.streams,
        partial_failure: report.partial_failure(),
        directory_clones: 0,
        file_clones: 0,
        byte_copies: report.byte_copies,
        restarted_files: report.restarted_files,
        resumed_bytes: report.resumed_bytes,
        retransmitted_bytes: report.retransmitted_bytes,
        checkpoint_bytes: report.checkpoint_bytes,
        checksum_cache_hits: report.checksum_cache_hits,
        checksum_cache_misses: report.checksum_cache_misses,
    });

    Ok(report)
}

/// Derive a stable 16-byte job ID from the remote invocation so a retried job
/// (after a killed sender, receiver, or transport) reuses the same resume
/// journal, while distinct invocations use distinct journals.
#[must_use]
pub fn session_job_id(left: &str, right: &str) -> [u8; 16] {
    let digest = blake3::hash(format!("{left}\u{0}{right}").as_bytes());
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

fn encode_exclude_patterns(patterns: &[String]) -> Vec<Vec<u8>> {
    patterns
        .iter()
        .map(|pattern| pattern.as_bytes().to_vec())
        .collect()
}

fn ranges_cover_file(size: u64, ranges: &[ByteRange]) -> bool {
    if size == 0 {
        return true;
    }
    let mut ordered = ranges.to_vec();
    ordered.sort_by_key(|range| range.offset);
    let mut cursor = 0_u64;
    for range in ordered {
        if range.offset > cursor {
            return false;
        }
        cursor = cursor.max(range.offset.saturating_add(range.length));
        if cursor >= size {
            return true;
        }
    }
    false
}

fn excluded_path(patterns: &[Vec<u8>], path: &WirePath) -> bool {
    let display = path.to_string();
    let mut candidate = Some(display.as_str());
    while let Some(value) = candidate {
        if patterns.iter().any(|pattern| {
            std::str::from_utf8(pattern)
                .ok()
                .and_then(|glob| globset::Glob::new(glob).ok())
                .is_some_and(|glob| glob.compile_matcher().is_match(value))
        }) {
            return true;
        }
        candidate = value.rsplit_once('/').map(|(parent, _)| parent);
    }
    false
}

fn map_transport_error(err: ProtocolError, stream: usize) -> ServerError {
    match err {
        ProtocolError::Read(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => {
            ServerError::Transport {
                stream,
                message: "server stream disconnected / process exited unexpectedly".to_owned(),
            }
        }
        other => ServerError::Protocol(other),
    }
}

/// Spawn a local child `xsync --server <path>` process and execute push.
///
/// # Errors
/// Returns [`ServerError`] on failure.
pub fn sync_push_server<F: FnMut(LocalEvent)>(
    source_path: &Path,
    source_trailing_slash: bool,
    dest_path: &str,
    dest_trailing_slash: bool,
    options: &LocalSyncOptions,
    rsh: Option<&str>,
    host: Option<&str>,
    emit: F,
) -> Result<LocalSyncReport, ServerError> {
    if options.streams > 1 {
        return sync_push_server_streams(
            source_path,
            source_trailing_slash,
            dest_path,
            dest_trailing_slash,
            options,
            rsh,
            host,
            emit,
        );
    }
    let mut emit = emit;
    spawn_and_run_session(dest_path, rsh, host, |reader, writer| {
        run_client_push(
            source_path,
            source_trailing_slash,
            dest_path,
            dest_trailing_slash,
            options,
            reader,
            writer,
            &mut emit,
        )
    })
}

/// Encode and flush one frame to a client `writer`.
///
/// # Errors
/// Returns [`ServerError`] on encode or I/O failure.
fn write_frame<W: Write>(writer: &mut W, id: u64, msg: &Message) -> Result<(), ServerError> {
    let bytes = encode_frame(id, msg)?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn write_data_frame<W: Write>(
    writer: &mut W,
    id: u64,
    msg: &Message,
    compress: bool,
    level: i32,
) -> Result<usize, ServerError> {
    let mode = if compress {
        CompressionMode::Zstd
    } else {
        CompressionMode::None
    };
    let bytes = encode_frame_with_compression(id, msg, mode, level)?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(bytes.len())
}

/// Read and verify one `Ack` frame.
///
/// # Errors
/// Returns [`ServerError`] if the next frame is not an `Ack`, or a protocol/
/// transport error.
fn expect_ack<R: Read>(decoder: &mut FrameDecoder, reader: &mut R) -> Result<(), ServerError> {
    let frame = decoder
        .read(reader)
        .map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack, got {:?}",
            frame.message
        )));
    }
    Ok(())
}

fn expect_ack_or_delete_warning<R: Read>(
    decoder: &mut FrameDecoder,
    reader: &mut R,
) -> Result<Option<String>, ServerError> {
    let frame = decoder
        .read(reader)
        .map_err(|e| map_transport_error(e, 0))?;
    match frame.message {
        Message::Ack { .. } => Ok(None),
        Message::Error {
            code: 1001,
            message,
            ..
        } => Ok(Some(message)),
        other => Err(ServerError::UnexpectedMessage(format!(
            "expected Ack or delete warning, got {other:?}"
        ))),
    }
}

fn record_delete_failure<F: FnMut(LocalEvent)>(
    report: &mut LocalSyncReport,
    emit: &mut F,
    entry: &FileEntry,
    error: impl std::fmt::Display,
) {
    let path = entry.path.to_string();
    let message = format!("delete '{path}' failed: {error}");
    report.failed_entries = report.failed_entries.saturating_add(1);
    report.warnings = report.warnings.saturating_add(1);
    emit(LocalEvent::Warning {
        path: path.clone(),
        message: message.clone(),
    });
    emit(LocalEvent::Failed { path, message });
}

/// Multi-stream PUSH (Story 4.2): one control session owns planning, metadata,
/// large-file prepare/finish, and journal clearing, while `streams` data-only
/// `--server` sessions strip a huge file's disjoint ranges (and small/medium
/// files are written by the control session). Every data range is durably
/// merged into the shared resume journal by its data session before the client
/// raises the finish barrier.
///
/// A global barrier is used: all data sessions run to completion, then a
/// coverage assertion guarantees every large file's bytes were durably written
/// before any `LargeFileFinish` commits it, converting any routing bug from
/// silent corruption into a loud failure.
///
/// # Errors
/// Returns [`ServerError`] on protocol, transport, or coverage failures.
#[allow(clippy::too_many_lines, clippy::type_complexity)]
pub fn sync_push_server_streams<F: FnMut(LocalEvent)>(
    source_path: &Path,
    source_trailing_slash: bool,
    dest_path: &str,
    _dest_trailing_slash: bool,
    options: &LocalSyncOptions,
    rsh: Option<&str>,
    host: Option<&str>,
    mut emit: F,
) -> Result<LocalSyncReport, ServerError> {
    let streams = options.streams.max(1);
    let job_id = session_job_id(source_path.to_string_lossy().as_ref(), dest_path);

    // ---- Control session: handshake, destination scan, plan ----
    let mut control = spawn_server_child(dest_path, rsh, host)?;
    let control_stderr = control.stderr.take();
    let cstderr_handle = control_stderr.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut buf);
            buf
        })
    });
    let cstdin = control.stdin.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open control stdin".to_owned(),
    })?;
    let cstdout = control
        .stdout
        .take()
        .ok_or_else(|| ServerError::Transport {
            stream: 0,
            message: "failed to open control stdout".to_owned(),
        })?;
    let mut cwriter = BufWriter::new(cstdin);
    let mut creader = BufReader::new(cstdout);
    let mut cdec = FrameDecoder::new();
    let mut cid = 1u64;
    let mut calloc = || {
        let id = cid;
        cid = cid.saturating_add(1);
        id
    };

    emit(LocalEvent::Started {
        local_workers: options.local_workers,
        streams,
    });

    write_frame(
        &mut cwriter,
        calloc(),
        &Message::Handshake {
            role: Role::Source,
            capabilities: 0,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id,
            compression: CompressionMode::None,
            compression_level: 3,
        },
    )?;
    if !matches!(
        cdec.read(&mut creader)
            .map_err(|e| map_transport_error(e, 0))?
            .message,
        Message::Handshake { .. }
    ) {
        return Err(ServerError::UnexpectedMessage(
            "control handshake".to_owned(),
        ));
    }
    expect_ack(&mut cdec, &mut creader)?;

    write_frame(
        &mut cwriter,
        calloc(),
        &Message::SessionConfig {
            streams: streams as u8,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            delete: options.delete,
            checksum: options.checksum,
            paranoid: options.paranoid,
            dry_run: options.dry_run,
            exclude_patterns: encode_exclude_patterns(&options.exclude_patterns),
        },
    )?;
    expect_ack(&mut cdec, &mut creader)?;

    // Destination scan pages from the control server.
    let mut dest_entries = Vec::new();
    loop {
        let frame = cdec
            .read(&mut creader)
            .map_err(|e| map_transport_error(e, 0))?;
        match frame.message {
            Message::Scan {
                final_page,
                entries,
                ..
            } => {
                for rec in entries {
                    dest_entries.push(file_entry_from_entry_record(&rec)?);
                }
                write_frame(
                    &mut cwriter,
                    calloc(),
                    &Message::Ack {
                        acknowledged_id: frame.message_id,
                        acknowledged_type: 9,
                    },
                )?;
                if final_page {
                    break;
                }
            }
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Scan, got {other:?}"
                )))
            }
        }
    }

    let mut dest_index = DestinationIndex::with_config(IndexConfig {
        memory_budget_bytes: 32 * 1024 * 1024,
        temp_root: std::env::temp_dir(),
    })?;
    for entry in dest_entries {
        dest_index.insert(entry)?;
    }

    // Scan local source and plan.
    let source_scan = scan(source_path)?;
    let mut source_entries = Vec::new();
    for item in source_scan.entries() {
        source_entries.push(item?);
    }
    source_scan.finish()?;

    let source_metadata = fs::symlink_metadata(source_path)?;
    let source_is_dir = source_metadata.is_dir() && !source_metadata.file_type().is_symlink();
    let prefix = if source_is_dir {
        if source_trailing_slash {
            String::new()
        } else {
            source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_owned()
        }
    } else {
        // A single-file scan already reports the basename as its relative
        // path; do not prepend it a second time.
        String::new()
    };
    let exclude_patterns = encode_exclude_patterns(&options.exclude_patterns);
    let hash_cache = options
        .checksum
        .then(|| HashCache::open(HashCache::default_path()).ok())
        .flatten();
    let source_reader_root = if source_is_dir {
        source_path.to_path_buf()
    } else {
        source_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let mut mapped = Vec::new();
    for mut entry in source_entries {
        if options.checksum && entry.kind == ScanEntryKind::File {
            entry.fingerprint.identity = cached_content_identity(
                &entry.path.to_native_path(&source_reader_root),
                &entry,
                hash_cache.as_ref(),
            )?;
        }
        if !prefix.is_empty() {
            entry.path = entry.path.with_prefix(&WirePath::from(prefix.as_str()));
        }
        if !excluded_path(&exclude_patterns, &entry.path) {
            mapped.push(entry);
        }
    }
    let plan = try_plan_with_fingerprint(mapped, dest_index, options.checksum)?;

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams,
        ..LocalSyncReport::default()
    };
    report.skipped_files = plan.files.unchanged.len();
    let plan_files = plan.files.new.len() + plan.files.changed.len();
    let plan_bytes: u64 = plan
        .files
        .new
        .iter()
        .chain(&plan.files.changed)
        .map(|e| e.size)
        .sum();
    emit(LocalEvent::Planned {
        files: plan_files,
        bytes: plan_bytes,
    });
    for entry in &plan.files.unchanged {
        emit(LocalEvent::Skipped {
            path: entry.path.to_string(),
            bytes: entry.size,
        });
    }

    // The control server has already sent its destination scan, so complete
    // the dry-run protocol transaction and reap it without opening any data
    // sessions or sending mutation frames.
    if options.dry_run {
        crate::local::emit_plan_actions(&plan, &mut emit);
        if let Some(cache) = hash_cache.as_ref() {
            let (hits, misses) = cache.stats();
            report.checksum_cache_hits = hits;
            report.checksum_cache_misses = misses;
        }
        write_frame(
            &mut cwriter,
            calloc(),
            &Message::Stats {
                files: 0,
                bytes: 0,
                skipped: report.skipped_files as u64,
                warnings: 0,
                failed: 0,
            },
        )?;
        expect_ack(&mut cdec, &mut creader)?;
        let _ = cdec
            .read(&mut creader)
            .map_err(|e| map_transport_error(e, 0))?;
        drop(cwriter);
        drop(creader);
        let _ = control.wait();
        if let Some(handle) = cstderr_handle {
            let _ = handle.join();
        }
        emit(LocalEvent::Finished {
            transport: None,
            transferred_files: 0,
            transferred_bytes: 0,
            skipped_files: report.skipped_files,
            failed_entries: 0,
            deleted_entries: 0,
            warnings: 0,
            physical_bytes: 0,
            wire_bytes: 0,
            directory_clones: 0,
            file_clones: 0,
            byte_copies: 0,
            local_workers: options.local_workers,
            streams,
            partial_failure: false,
            restarted_files: 0,
            resumed_bytes: 0,
            retransmitted_bytes: 0,
            checkpoint_bytes: 0,
            checksum_cache_hits: report.checksum_cache_hits,
            checksum_cache_misses: report.checksum_cache_misses,
        });
        return Ok(report);
    }

    // ---- Control: create directories and symlinks ----
    if !options.dry_run {
        let mut dirs = plan.directories.new.clone();
        dirs.sort_by_key(|d| d.path.len());
        for dir in dirs {
            write_frame(
                &mut cwriter,
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::CreateDirectory,
                    path: dir.path.as_bytes().to_vec(),
                    target: Vec::new(),
                    mode: dir.mode,
                    mtime_ns: system_time_to_nanos(dir.mtime),
                },
            )?;
            expect_ack(&mut cdec, &mut creader)?;
        }
        for sym in plan.symlinks.new.iter().chain(&plan.symlinks.changed) {
            let local = if prefix.is_empty() {
                sym.path.to_native_path(source_path)
            } else {
                sym.path
                    .strip_prefix(format!("{prefix}/"))
                    .unwrap_or_else(|| sym.path.clone())
                    .to_native_path(source_path)
            };
            let target = fs::read_link(&local)?;
            write_frame(
                &mut cwriter,
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::CreateSymlink,
                    path: sym.path.as_bytes().to_vec(),
                    target: target.into_os_string().into_encoded_bytes(),
                    mode: sym.mode,
                    mtime_ns: system_time_to_nanos(sym.mtime),
                },
            )?;
            expect_ack(&mut cdec, &mut creader)?;
        }
    }

    let source_reader = SourceReader::new(source_path);

    // ---- Partition files ----
    // Small/medium files: written by the control session itself.
    let mut small_files: Vec<FileEntry> = Vec::new();
    // Large files: striped across data sessions.
    let mut large_files: Vec<FileEntry> = Vec::new();
    for file in plan.files.new.iter().chain(&plan.files.changed) {
        if file.size <= MAX_DATA_SEGMENT as u64 {
            small_files.push(file.clone());
        } else {
            large_files.push(file.clone());
        }
    }

    // Control: prepare each large file and read its resume pages (verified ranges).
    let mut verified_by_path: HashMap<WirePath, Vec<ByteRange>> = HashMap::new();
    let mut control_large_ids: Vec<(u64, FileEntry)> = Vec::new();
    for file in &large_files {
        let file_id = calloc();
        let record = entry_record_from_file_entry(file);
        write_frame(
            &mut cwriter,
            calloc(),
            &Message::LargeFilePrepare {
                file_id,
                path: file.path.as_bytes().to_vec(),
                size: file.size,
                mtime_ns: record.mtime_ns,
                mode: file.mode,
                fingerprint: record.fingerprint,
            },
        )?;
        expect_ack(&mut cdec, &mut creader)?;
        let mut verified = Vec::new();
        let mut final_page = false;
        while !final_page {
            let frame = cdec
                .read(&mut creader)
                .map_err(|e| map_transport_error(e, 0))?;
            match frame.message {
                Message::ResumePage {
                    final_page: fp,
                    ranges,
                    ..
                } => {
                    verified.extend(ranges);
                    final_page = fp;
                }
                other => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected ResumePage, got {other:?}"
                    )))
                }
            }
        }
        verified_by_path.insert(file.path.clone(), verified);
        control_large_ids.push((file_id, file.clone()));
    }

    // Control: write small/medium files.
    for file in &small_files {
        let file_to_read = if prefix.is_empty() {
            file.clone()
        } else {
            let mut f = file.clone();
            f.path = file
                .path
                .strip_prefix(format!("{prefix}/"))
                .unwrap_or_else(|| file.path.clone())
                .clone();
            f
        };
        let stable = match source_reader.read(&file_to_read) {
            Ok(s) => s,
            Err(err) => {
                emit(LocalEvent::Failed {
                    path: file.path.to_string(),
                    message: err.to_string(),
                });
                report.failed_entries = report.failed_entries.saturating_add(1);
                continue;
            }
        };
        let mut rec = entry_record_from_file_entry(file);
        rec.path = file.path.as_bytes().to_vec();
        let batch_id = calloc();
        write_frame(
            &mut cwriter,
            batch_id,
            &Message::FileBatch {
                batch_id: 1,
                entries: vec![rec],
            },
        )?;
        expect_ack(&mut cdec, &mut creader)?;
        let sfile_id = (batch_id << 16) | 0;
        write_frame(
            &mut cwriter,
            calloc(),
            &Message::FileSegment {
                file_id: sfile_id,
                offset: 0,
                data: stable.bytes,
            },
        )?;
        expect_ack(&mut cdec, &mut creader)?;

        report.transferred_files = report.transferred_files.saturating_add(1);
        report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
        report.physical_bytes = report.physical_bytes.saturating_add(file.size);
        report.byte_copies = report.byte_copies.saturating_add(1);
        emit(LocalEvent::Transferred {
            path: file.path.to_string(),
            bytes: file.size,
            physical_bytes: file.size,
            method: TransferMethod::ByteCopy,
        });
    }

    // ---- Partition large-file missing ranges across data sessions ----
    let chunk = 8 * 1024 * 1024u64;
    // Per data-session work grouped by file path, so each session prepares each
    // file exactly once and then writes that file's disjoint ranges.
    let mut grouped: Vec<HashMap<WirePath, (FileEntry, Vec<ByteRange>)>> =
        (0..streams).map(|_| HashMap::new()).collect();
    let mut assigned_by_path: HashMap<WirePath, Vec<ByteRange>> = HashMap::new();
    let mut round = 0usize;
    for file in &large_files {
        let verified = verified_by_path
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        let missing = crate::journal::missing_chunks(file.size, chunk, &verified);
        for range in missing {
            let slot = grouped[round % streams]
                .entry(file.path.clone())
                .or_insert_with(|| (file.clone(), Vec::new()));
            slot.1.push(range);
            assigned_by_path
                .entry(file.path.clone())
                .or_default()
                .push(range);
            round += 1;
        }
    }
    let data_work: Vec<Vec<(FileEntry, Vec<ByteRange>)>> = grouped
        .into_iter()
        .map(|m| m.into_values().collect())
        .collect();

    // ---- Spawn data threads ----
    let source_path_buf = source_path.to_path_buf();
    let data_threads = {
        let dest = dest_path.to_owned();
        let job_id_copy = job_id;
        let compress = options.compress;
        let compression_level = options.compress_level;
        let mut handles = Vec::new();
        let work_by_thread: Vec<(std::process::Child, Vec<(FileEntry, Vec<ByteRange>)>)> =
            data_work
                .into_iter()
                .map(|work| {
                    let child = spawn_server_child(&dest, rsh, host)?;
                    Ok((child, work))
                })
                .collect::<Result<_, ServerError>>()?;
        for (child, work) in work_by_thread {
            let sp = source_path_buf.clone();
            let prefix_copy = prefix.clone();
            let job = job_id_copy;
            handles.push(std::thread::spawn(move || {
                run_data_thread(
                    child,
                    &sp,
                    &prefix_copy,
                    job,
                    work,
                    compress,
                    compression_level,
                )
            }));
        }
        handles
    };

    let written: Vec<Result<(Vec<(WirePath, Vec<ByteRange>)>, u64), ServerError>> = data_threads
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|_| {
                Err(ServerError::UnexpectedMessage(
                    "data thread panicked".to_owned(),
                ))
            })
        })
        .collect();
    let mut written_by_path: HashMap<WirePath, Vec<ByteRange>> = HashMap::new();
    for result in written {
        let (ranges, wire_bytes) = result?;
        report.wire_bytes = report.wire_bytes.saturating_add(wire_bytes);
        for (path, mut rs) in ranges {
            if let Some(file) = large_files.iter().find(|file| file.path == path) {
                emit(LocalEvent::Progress {
                    path: path.to_string(),
                    stream: 0,
                    completed: rs.iter().map(|range| range.length).sum(),
                    total: file.size,
                });
            }
            written_by_path.entry(path).or_default().append(&mut rs);
        }
    }

    // Coverage assertion: every large file must be fully, durably covered.
    let mut resumed_bytes_total = 0u64;
    let mut retransmitted_total = 0u64;
    for file in &large_files {
        let verified = verified_by_path
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        let assigned = assigned_by_path
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        let union = crate::journal::merge_ranges(&verified, &assigned);
        let covered: u64 = union.iter().map(|r| r.length).sum();
        if covered != file.size {
            return Err(ServerError::UnexpectedMessage(format!(
                "large file {} covered {covered} of {} bytes; ranges lost across streams",
                file.path, file.size
            )));
        }
        let resumed: u64 = verified.iter().map(|r| r.length).sum();
        resumed_bytes_total = resumed_bytes_total.saturating_add(resumed);
        retransmitted_total =
            retransmitted_total.saturating_add(assigned.iter().map(|r| r.length).sum::<u64>());

        // Count the large file once, reporting physical bytes actually sent.
        let physical: u64 = assigned.iter().map(|r| r.length).sum();
        report.transferred_files = report.transferred_files.saturating_add(1);
        report.transferred_bytes = report.transferred_bytes.saturating_add(file.size);
        report.physical_bytes = report.physical_bytes.saturating_add(physical);
        report.byte_copies = report.byte_copies.saturating_add(1);
        emit(LocalEvent::Transferred {
            path: file.path.to_string(),
            bytes: file.size,
            physical_bytes: physical,
            method: TransferMethod::ByteCopy,
        });
    }

    if !options.dry_run {
        // Barrier reached: all ranges are durably written. Finish large files.
        // Paranoid mode supplies a complete source digest so the receiver can
        // validate the published striped file after the final rename.
        for (file_id, file) in &control_large_ids {
            let digest = if options.paranoid {
                let source_relative = if prefix.is_empty() {
                    file.path.clone()
                } else {
                    file.path
                        .strip_prefix(format!("{prefix}/"))
                        .unwrap_or_else(|| file.path.clone())
                        .clone()
                };
                *hash_file_streaming(&source_relative.to_native_path(source_path))?.as_bytes()
            } else {
                [0u8; 32]
            };
            write_frame(
                &mut cwriter,
                calloc(),
                &Message::LargeFileFinish {
                    file_id: *file_id,
                    digest,
                },
            )?;
            expect_ack(&mut cdec, &mut creader)?;
        }

        // Finish directories deepest-first, then the root directory.
        let mut dirs: Vec<_> = plan
            .directories
            .new
            .iter()
            .chain(&plan.directories.changed)
            .chain(&plan.directories.unchanged)
            .collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
        for dir in dirs {
            write_frame(
                &mut cwriter,
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::SetDirectory,
                    path: dir.path.as_bytes().to_vec(),
                    target: Vec::new(),
                    mode: dir.mode,
                    mtime_ns: system_time_to_nanos(dir.mtime),
                },
            )?;
            expect_ack(&mut cdec, &mut creader)?;
        }
        if source_is_dir {
            write_frame(
                &mut cwriter,
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::SetDirectory,
                    path: Vec::new(),
                    target: Vec::new(),
                    mode: permission_mode(&source_metadata),
                    mtime_ns: system_time_to_nanos(source_metadata.modified()?),
                },
            )?;
            expect_ack(&mut cdec, &mut creader)?;
        }

        // Deletes, deepest first.
        if options.delete && !report.partial_failure() {
            let mut to_delete = Vec::new();
            to_delete.extend(plan.files.extraneous.clone());
            to_delete.extend(plan.symlinks.extraneous.clone());
            let mut ext_dirs = plan.directories.extraneous.clone();
            ext_dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
            to_delete.extend(ext_dirs);
            for entry in to_delete {
                write_frame(
                    &mut cwriter,
                    calloc(),
                    &Message::Metadata {
                        operation: MetadataOperation::Delete,
                        path: entry.path.as_bytes().to_vec(),
                        target: Vec::new(),
                        mode: 0,
                        mtime_ns: 0,
                    },
                )?;
                let deleted =
                    if let Some(message) = expect_ack_or_delete_warning(&mut cdec, &mut creader)? {
                        record_delete_failure(
                            &mut report,
                            &mut emit,
                            &entry,
                            ServerError::RemoteError {
                                code: 1001,
                                message,
                            },
                        );
                        false
                    } else {
                        emit(LocalEvent::Deleted {
                            path: entry.path.to_string(),
                        });
                        true
                    };
                if deleted {
                    report.deleted_entries = report.deleted_entries.saturating_add(1);
                }
            }
        }

        // Stats.
        write_frame(
            &mut cwriter,
            calloc(),
            &Message::Stats {
                files: report.transferred_files as u64,
                bytes: report.transferred_bytes,
                skipped: report.skipped_files as u64,
                warnings: report.warnings as u64,
                failed: report.failed_entries as u64,
            },
        )?;
        expect_ack(&mut cdec, &mut creader)?;
        let _server_stats = cdec
            .read(&mut creader)
            .map_err(|e| map_transport_error(e, 0))?;
    }

    report.resumed_bytes = resumed_bytes_total;
    report.restarted_files = large_files
        .iter()
        .filter(|f| verified_by_path.get(&f.path).is_some_and(|v| !v.is_empty()))
        .count();
    report.retransmitted_bytes = retransmitted_total;
    report.checkpoint_bytes = resumed_bytes_total.saturating_add(retransmitted_total);

    drop(cwriter);
    drop(creader);
    let _ = control.wait();
    if let Some(handle) = cstderr_handle {
        let text = handle.join().unwrap_or_default();
        let trimmed = text.trim_end().to_owned();
        if !trimmed.is_empty() {
            eprintln!("{trimmed}");
        }
    }

    emit(LocalEvent::Finished {
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
        deleted_entries: report.deleted_entries,
        warnings: report.warnings,
        local_workers: options.local_workers,
        streams,
        partial_failure: report.partial_failure(),
        directory_clones: 0,
        file_clones: 0,
        byte_copies: report.byte_copies,
        restarted_files: report.restarted_files,
        resumed_bytes: report.resumed_bytes,
        retransmitted_bytes: report.retransmitted_bytes,
        checkpoint_bytes: report.checkpoint_bytes,
        checksum_cache_hits: report.checksum_cache_hits,
        checksum_cache_misses: report.checksum_cache_misses,
    });

    Ok(report)
}

/// Drive one data-only `--server` session (Story 4.2), handshaking with the
/// `CAP_DATA_ONLY` capability bit and writing the assigned large-file ranges.
type DataThreadResult = (Vec<(WirePath, Vec<ByteRange>)>, u64);
///
/// # Errors
/// Returns [`ServerError`] on protocol or transport failure.
#[allow(clippy::needless_pass_by_value)]
fn run_data_thread(
    mut child: std::process::Child,
    source_path: &Path,
    prefix: &str,
    job_id: [u8; 16],
    work: Vec<(FileEntry, Vec<ByteRange>)>,
    compress: bool,
    compression_level: i32,
) -> Result<DataThreadResult, ServerError> {
    let stdin = child.stdin.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open data stdin".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open data stdout".to_owned(),
    })?;
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut buf);
            buf
        })
    });
    let result = run_data_inner(
        stdin,
        stdout,
        source_path,
        prefix,
        job_id,
        &work,
        compress,
        compression_level,
    );
    if let Some(handle) = stderr {
        let text = handle.join().unwrap_or_default();
        let trimmed = text.trim_end().to_owned();
        if !trimmed.is_empty() {
            eprintln!("{trimmed}");
        }
    }
    let _ = child.wait();
    result
}

/// Core of [`run_data_thread`], after its streams have been split out.
fn run_data_inner(
    stdin: std::process::ChildStdin,
    stdout: std::process::ChildStdout,
    source_path: &Path,
    prefix: &str,
    job_id: [u8; 16],
    work: &[(FileEntry, Vec<ByteRange>)],
    compress: bool,
    compression_level: i32,
) -> Result<DataThreadResult, ServerError> {
    let mut writer = BufWriter::new(stdin);
    let mut reader = BufReader::new(stdout);
    let mut decoder = FrameDecoder::new();
    let mut id = 1u64;
    let mut alloc = || {
        let x = id;
        id = id.saturating_add(1);
        x
    };

    write_frame(
        &mut writer,
        alloc(),
        &Message::Handshake {
            role: Role::Source,
            capabilities: crate::protocol::CAP_DATA_ONLY | CAP_ZSTD,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id,
            compression: if compress {
                CompressionMode::Zstd
            } else {
                CompressionMode::None
            },
            compression_level,
        },
    )?;
    if !matches!(
        decoder
            .read(&mut reader)
            .map_err(|e| map_transport_error(e, 0))?
            .message,
        Message::Handshake { .. }
    ) {
        return Err(ServerError::UnexpectedMessage("data handshake".to_owned()));
    }
    expect_ack(&mut decoder, &mut reader)?;
    write_frame(
        &mut writer,
        alloc(),
        &Message::SessionConfig {
            streams: 1,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            delete: false,
            checksum: false,
            paranoid: false,
            dry_run: false,
            exclude_patterns: Vec::new(),
        },
    )?;
    expect_ack(&mut decoder, &mut reader)?;

    let source_reader = SourceReader::new(source_path);
    let mut written: Vec<(WirePath, Vec<ByteRange>)> = Vec::new();
    let mut wire_bytes = 0_u64;
    for (file, ranges) in work {
        // Read the file once as the source for this session's slices.
        let rel = if prefix.is_empty() {
            file.path.clone()
        } else {
            file.path
                .strip_prefix(format!("{prefix}/"))
                .unwrap_or_else(|| file.path.clone())
                .clone()
        };
        let mut file_to_read = file.clone();
        file_to_read.path = rel;
        let stable = source_reader.read(&file_to_read)?;

        let record = entry_record_from_file_entry(file);
        let file_id = alloc();
        write_frame(
            &mut writer,
            alloc(),
            &Message::LargeFilePrepare {
                file_id,
                path: file.path.as_bytes().to_vec(),
                size: file.size,
                mtime_ns: record.mtime_ns,
                mode: file.mode,
                fingerprint: record.fingerprint,
            },
        )?;
        expect_ack(&mut decoder, &mut reader)?;

        for range in ranges {
            let start = usize::try_from(range.offset).unwrap_or(0);
            let len = usize::try_from(range.length).unwrap_or(0);
            write_frame(
                &mut writer,
                alloc(),
                &Message::LargeFileRange {
                    file_id,
                    range: *range,
                },
            )?;
            if !matches!(
                decoder
                    .read(&mut reader)
                    .map_err(|e| map_transport_error(e, 0))?
                    .message,
                Message::Ack { .. }
            ) {
                return Err(ServerError::UnexpectedMessage("data range ack".to_owned()));
            }
            wire_bytes = wire_bytes.saturating_add(write_data_frame(
                &mut writer,
                alloc(),
                &Message::FileSegment {
                    file_id,
                    offset: range.offset,
                    data: stable.bytes[start..start + len].to_vec(),
                },
                compress,
                compression_level,
            )? as u64);
            expect_ack(&mut decoder, &mut reader)?;
            written.push((file.path.clone(), vec![*range]));
        }
    }
    // Signal EOF to end the data session cleanly.
    drop(writer);
    drop(reader);
    Ok((written, wire_bytes))
}

/// Spawn a local child `xsync --server <path>` process and execute pull.
///
/// # Errors
/// Returns [`ServerError`] on failure.
pub fn sync_pull_server<F: FnMut(LocalEvent)>(
    src_path: &str,
    src_trailing_slash: bool,
    dest_path: &Path,
    dest_trailing_slash: bool,
    options: &LocalSyncOptions,
    rsh: Option<&str>,
    host: Option<&str>,
    emit: F,
) -> Result<LocalSyncReport, ServerError> {
    let mut emit = emit;
    spawn_and_run_session(src_path, rsh, host, |reader, writer| {
        run_client_pull(
            src_path,
            src_trailing_slash,
            dest_path,
            dest_trailing_slash,
            options,
            reader,
            writer,
            &mut emit,
        )
    })
}

/// Spawn the remote server and run one client session against it, learning the
/// remote shell family from the attempt rather than probing for it first.
///
/// A stock Windows OpenSSH host hands the command to `cmd.exe`, which cannot
/// parse the POSIX form. That is only discoverable by trying, so the first
/// session against an unknown host may cost one extra connection -- but only
/// when the remote actually ran and rejected the command. An authentication or
/// host-key failure returns ssh's 255 and is never retried, so a bad credential
/// still produces exactly one connection attempt.
///
/// The learned family is cached per host, so later streams in the same job and
/// later jobs in the same process go straight to the right form.
fn spawn_and_run_session<F>(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
    mut f: F,
) -> Result<LocalSyncReport, ServerError>
where
    F: FnMut(
        &mut BufReader<CountingReader<std::process::ChildStdout>>,
        &mut BufWriter<std::process::ChildStdin>,
    ) -> Result<LocalSyncReport, ServerError>,
{
    let Some(host_name) = host else {
        let child = spawn_server_child_with_shell(remote_path, rsh, host, RemoteShell::Posix)?;
        return run_server_child_session(child, &mut f);
    };

    let first = remote_shell_for(rsh, host_name);
    let child = spawn_server_child_with_shell(remote_path, rsh, host, first)?;
    let result = run_server_child_session(child, &mut f);

    match result {
        Err(ServerError::RemoteShellMismatch) if first == RemoteShell::Posix => {
            let child = spawn_server_child_with_shell(
                remote_path,
                rsh,
                host,
                RemoteShell::Windows,
            )?;
            let retried = run_server_child_session(child, &mut f);
            if retried.is_ok() {
                remember_remote_shell(rsh, host_name, RemoteShell::Windows);
            }
            retried
        }
        // A mismatch when the Windows form was already in use is not a shell
        // problem; report it as the transport failure it actually is.
        Err(ServerError::RemoteShellMismatch) => Err(ServerError::Transport {
            stream: 0,
            message: "remote xsync server exited without completing the session".to_owned(),
        }),
        other => other,
    }
}

fn parse_rsh_command(rsh: &str) -> Vec<String> {
    shlex::split(rsh).unwrap_or_else(|| vec![rsh.to_owned()])
}

fn quote_remote_arg(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\\''"))
}

fn quote_remote_path(path: &str) -> String {
    if path == "~" {
        "\"$HOME\"".to_owned()
    } else if let Some(relative) = path.strip_prefix("~/") {
        format!("\"$HOME\"/{}", quote_remote_arg(relative))
    } else {
        quote_remote_arg(path)
    }
}

/// Which shell the remote `sshd` will hand the command string to.
///
/// The command is a single string interpreted by the remote login shell, so its
/// syntax is not portable: `PATH="$HOME/..." 'xs' '--server' '/p'` is correct
/// POSIX and complete nonsense to `cmd.exe`, which is what stock Windows
/// OpenSSH uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoteShell {
    /// A POSIX shell: `sh`, `bash`, `zsh`. The default everywhere but Windows.
    #[default]
    Posix,
    /// Windows `cmd.exe`, the stock OpenSSH-for-Windows default shell.
    Windows,
}

/// Quote one argument for `cmd.exe`.
///
/// Inside double quotes `cmd` treats `&`, `|`, `<`, `>` and `^` literally, so
/// the only genuine hazards are an embedded quote (which ends the run) and `%`
/// (which still expands `%VAR%` inside quotes and would leak or corrupt the
/// path). Neither can be escaped reliably across `cmd` versions, so they are
/// refused rather than mangled.
fn quote_windows_arg(argument: &str) -> Result<String, ServerError> {
    if argument.contains('"') || argument.contains('%') {
        return Err(ServerError::InvalidPath(format!(
            "remote Windows path may not contain '\"' or '%': {argument}"
        )));
    }
    Ok(format!("\"{argument}\""))
}

fn quote_windows_path(path: &str) -> Result<String, ServerError> {
    if path == "~" {
        return Ok("\"%USERPROFILE%\"".to_owned());
    }
    if let Some(relative) = path.strip_prefix("~/") {
        return Ok(format!("\"%USERPROFILE%\\{}\"", {
            let quoted = quote_windows_arg(relative)?;
            quoted.trim_matches('"').to_owned()
        }));
    }
    quote_windows_arg(path)
}

fn xsync_remote_command(
    remote_path: &str,
    shell: RemoteShell,
) -> Result<String, ServerError> {
    match shell {
        RemoteShell::Posix => Ok(format!(
            "PATH=\"$HOME/.local/bin:$PATH\" {} {} {}",
            quote_remote_arg("xs"),
            quote_remote_arg("--server"),
            quote_remote_path(remote_path)
        )),
        // `set "PATH=..." & xs --server "<path>"`. A single `&` rather than
        // `&&` so a `set` that reports failure still runs the server, matching
        // the POSIX form where the assignment prefix cannot fail independently.
        RemoteShell::Windows => Ok(format!(
            "set \"PATH=%USERPROFILE%\\.local\\bin;%PATH%\" & xs --server {}",
            quote_windows_path(remote_path)?
        )),
    }
}

/// Remote shell family remembered per `(rsh, host)` pair for this process.
///
/// Deliberately *not* a probe. An upfront probe would add a second SSH
/// connection to every job, and on a host that rejects authentication that
/// second connection means a second password prompt or another failed-login
/// attempt against a lockout policy. `xs` must contact a failing remote exactly
/// once, so the family is instead learned from the first attempt and reused.
fn remote_shell_cache() -> &'static std::sync::Mutex<HashMap<String, RemoteShell>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, RemoteShell>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn remote_shell_key(rsh: Option<&str>, host: &str) -> String {
    format!("{}\u{1f}{host}", rsh.unwrap_or_default())
}

/// The shell family to try first: whatever a previous session established for
/// this host, otherwise POSIX, which is correct everywhere but Windows.
fn remote_shell_for(rsh: Option<&str>, host: &str) -> RemoteShell {
    remote_shell_cache()
        .lock()
        .ok()
        .and_then(|map| map.get(&remote_shell_key(rsh, host)).copied())
        .unwrap_or_default()
}

fn remember_remote_shell(rsh: Option<&str>, host: &str, shell: RemoteShell) {
    if let Ok(mut map) = remote_shell_cache().lock() {
        map.insert(remote_shell_key(rsh, host), shell);
    }
}

/// `(program, args)` that runs one command string on `host`, without deciding
/// what that string should be. Shared by the shell probe and the server launch
/// so they can never disagree about how the remote shell is reached.
fn base_remote_invocation(
    rsh: Option<&str>,
    host: &str,
    command: &str,
) -> (String, Vec<String>) {
    if let Some(rsh_cmd) = rsh {
        let parts = parse_rsh_command(rsh_cmd);
        let program = parts.first().cloned().unwrap_or_else(|| rsh_cmd.to_owned());
        let mut args = if parts.is_empty() {
            Vec::new()
        } else {
            parts[1..].to_vec()
        };
        args.push(host.to_owned());
        args.push(command.to_owned());
        (program, args)
    } else {
        (
            DEFAULT_RSH.to_owned(),
            vec![host.to_owned(), command.to_owned()],
        )
    }
}

/// Default remote shell; replaced only by an explicit `-e/--rsh`.
const DEFAULT_RSH: &str = "ssh";

fn is_missing_xsync_stderr(stderr: &str, exit_code: Option<i32>) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("xs: command not found")
        || lower.contains("xs: not found")
        || (exit_code == Some(127) && lower.contains("xs"))
}

/// Compute `(program, args)` used to launch the remote `xsync --server`.
///
/// - Explicit `-e CMD`: shell request parsed (shlex), then `{host}` and one
///   injection-safe quoted remote command are appended.
/// - No `-e` but a host: the default `ssh {host} '<quoted command>'`.
/// - No host and no `-e`: an in-process/local child server via `current_exe`.
///
/// `shell` selects the syntax of the remote command string. Callers that have
/// not probed the remote should use [`remote_server_command`], which detects it.
///
/// # Errors
///
/// Returns [`ServerError::InvalidPath`] when a Windows remote path contains a
/// character that `cmd.exe` quoting cannot express safely.
pub fn remote_server_command_with_shell(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
    shell: RemoteShell,
) -> Result<(String, Vec<String>), ServerError> {
    match host {
        Some(h) => Ok(base_remote_invocation(
            rsh,
            h,
            &xsync_remote_command(remote_path, shell)?,
        )),
        // No host: the server runs locally as a child process with no shell in
        // between, so the arguments are passed through verbatim and none of the
        // quoting above applies.
        None => {
            if let Some(rsh_cmd) = rsh {
                let parts = parse_rsh_command(rsh_cmd);
                let program = parts.first().cloned().unwrap_or_else(|| rsh_cmd.to_owned());
                let mut args = if parts.is_empty() {
                    Vec::new()
                } else {
                    parts[1..].to_vec()
                };
                args.push("xs".to_owned());
                args.push("--server".to_owned());
                args.push(remote_path.to_owned());
                Ok((program, args))
            } else {
                let exe = std::env::current_exe()
                    .unwrap_or_else(|_| PathBuf::from("xsync"))
                    .to_string_lossy()
                    .into_owned();
                Ok((exe, vec!["--server".to_owned(), remote_path.to_owned()]))
            }
        }
    }
}

/// Compute `(program, args)` for the remote server, detecting the remote shell.
///
/// Probes the remote shell family once per `(rsh, host)` pair per process; see
/// [`remote_server_command_with_shell`] for the syntax rules.
///
/// # Errors
///
/// Returns [`ServerError::InvalidPath`] when a Windows remote path contains a
/// character that `cmd.exe` quoting cannot express safely.
pub fn remote_server_command(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
) -> Result<(String, Vec<String>), ServerError> {
    let shell = match host {
        Some(h) => remote_shell_for(rsh, h),
        None => RemoteShell::Posix,
    };
    remote_server_command_with_shell(remote_path, rsh, host, shell)
}

fn spawn_server_child(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
) -> Result<Child, ServerError> {
    let shell = host.map_or(RemoteShell::Posix, |h| remote_shell_for(rsh, h));
    spawn_server_child_with_shell(remote_path, rsh, host, shell)
}

fn spawn_server_child_with_shell(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
    shell: RemoteShell,
) -> Result<Child, ServerError> {
    if host.is_some_and(|value| value.starts_with('-')) {
        return Err(ServerError::Transport {
            stream: 0,
            message: "remote host must not start with '-' (would be parsed as an ssh option)"
                .to_owned(),
        });
    }
    let (program, args) = remote_server_command_with_shell(remote_path, rsh, host, shell)?;
    let mut cmd = Command::new(program);
    cmd.args(args);

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd.spawn().map_err(|err| ServerError::Transport {
        stream: 0,
        message: format!("cannot spawn xsync --server: {err}"),
    })
}

/// Run a client protocol session against a spawned remote child, draining the
/// child's stderr so a missing remote binary is reported as a typed selection
/// result rather than as a raw broken-pipe error.
/// Read adapter recording whether the remote ever produced a single byte.
///
/// This is the only dependable way to tell "the remote shell never ran our
/// server" from "the server ran and then failed". Exit codes cannot do it: on
/// stock Windows `cmd.exe`, `PATH` is a *builtin*, so the POSIX command string
/// `PATH="$HOME/.local/bin:$PATH" \'xs\' ...` is parsed as a `PATH` command that
/// sets the search path to that literal text and **exits 0**. The remote
/// reports success, emits nothing, and no status code distinguishes it.
struct CountingReader<R> {
    inner: R,
    bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.bytes.fetch_add(
            u64::try_from(read).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(read)
    }
}

fn run_server_child_session<F>(child: Child, f: F) -> Result<LocalSyncReport, ServerError>
where
    F: FnMut(
        &mut BufReader<CountingReader<std::process::ChildStdout>>,
        &mut BufWriter<std::process::ChildStdin>,
    ) -> Result<LocalSyncReport, ServerError>,
{
    let mut child = child;
    let stdin = child.stdin.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open child stdin".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open child stdout".to_owned(),
    })?;
    let stderr = child.stderr.take();

    // Drain stderr on a background thread: it both unblocks the child should its
    // own (little) stderr pipe fill, and lets us inspect a missing-binary message.
    let stderr_handle = stderr.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut buf);
            buf
        })
    });

    let bytes_from_remote = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut reader = BufReader::new(CountingReader {
        inner: stdout,
        bytes: std::sync::Arc::clone(&bytes_from_remote),
    });
    let mut writer = BufWriter::new(stdin);
    let mut f = f;
    let result = f(&mut reader, &mut writer);

    // Close both streams to signal EOF to the child before reaping it.
    drop(reader);
    drop(writer);

    let status = child.wait().ok();
    let stderr_text = stderr_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    // Relay the remote/ssh stderr so authentication, host-key, and other
    // diagnostics remain visible to the user (required for SSH UX). It is
    // empty for a clean in-process/fake-rsh session, so this adds no noise.
    let trimmed = stderr_text.trim_end().to_owned();
    if !trimmed.is_empty() {
        eprintln!("{trimmed}");
    }

    let exit_code = status.and_then(|s| s.code());

    if is_missing_xsync_stderr(&stderr_text, exit_code) {
        return Err(ServerError::MissingRemoteXsync);
    }

    // The remote shell reported success and yet never spoke a single byte of the
    // protocol. That pair is the exact, measured signature of the POSIX command
    // string reaching stock Windows `cmd.exe`: `PATH` is a cmd builtin, so
    // `PATH="$HOME/.local/bin:$PATH" \'xs\' ...` is read as a `PATH` command that
    // sets the search path to that literal text and exits 0. Nothing runs, and
    // no error is reported anywhere.
    //
    // Both halves are load-bearing:
    //
    //   * Requiring exit 0 excludes a server that started and then died. A
    //     transfer killed mid-flight exits non-zero, and its staged data and
    //     resume journal must survive -- retrying there would resume from the
    //     checkpoint and publish a file the caller was never told about.
    //   * Requiring zero bytes excludes a remote that spoke and then failed,
    //     including a peer that answers with a malformed protocol.
    //
    // ssh's own 255 (authentication, host key, connection) fails both tests, so
    // a rejected credential still costs exactly one connection attempt.
    if result.is_err()
        && exit_code == Some(0)
        && bytes_from_remote.load(std::sync::atomic::Ordering::Relaxed) == 0
    {
        return Err(ServerError::RemoteShellMismatch);
    }

    if status.is_some_and(|status| !status.success()) && result.is_ok() {
        return Err(ServerError::Transport {
            stream: 0,
            message: format!(
                "remote xsync exited with status {}",
                exit_code.map_or_else(|| "unknown".to_owned(), |code| code.to_string())
            ),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    #[test]
    fn entry_record_round_trips_invalid_utf8_path_bytes() {
        let record = EntryRecord {
            path: b"bad-\xff-name".to_vec(),
            kind: crate::protocol::EntryKind::File,
            size: 3,
            mtime_ns: 0,
            mode: 0o644,
            fingerprint: [0; 32],
        };
        let entry = file_entry_from_entry_record(&record).unwrap();
        assert_eq!(entry.path.as_bytes(), b"bad-\xff-name");
        assert_eq!(entry_record_from_file_entry(&entry).path, record.path);
    }

    #[test]
    fn probe_reports_ready_and_older_peers_without_reconnecting() {
        fn peer_bytes(capabilities: u32) -> Vec<u8> {
            let handshake = Message::Handshake {
                role: Role::Session,
                capabilities,
                max_payload: MAX_COMPLETE_PAYLOAD as u32,
                max_segment: MAX_DATA_SEGMENT as u32,
                window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                job_id: [14; 16],
                compression: CompressionMode::None,
                compression_level: 3,
            };
            let mut bytes = encode_frame(100, &handshake).unwrap();
            bytes.extend_from_slice(
                &encode_frame(
                    101,
                    &Message::Ack {
                        acknowledged_id: 1,
                        acknowledged_type: 1,
                    },
                )
                .unwrap(),
            );
            bytes
        }

        let ready = probe_session(
            Cursor::new(peer_bytes(CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION)),
            Vec::new(),
            [15; 16],
        )
        .unwrap();
        assert_eq!(ready.probe.status, ProbeStatus::Ready);
        assert_eq!(ready.probe.selected_version, 2);
        assert_eq!(ready.probe.status.action(), "open the browse session");
        assert!(ready.into_browse_session().is_ok());

        let older = probe_session(
            Cursor::new(peer_bytes(CAP_VERSION_NEGOTIATION)),
            Vec::new(),
            [16; 16],
        )
        .unwrap();
        assert_eq!(
            older.probe.status,
            ProbeStatus::OlderPeer {
                selected_version: 1
            }
        );
        assert_eq!(
            older.probe.status.action(),
            "upgrade the remote xsync binary before browsing"
        );
        assert!(older.into_browse_session().is_err());
    }

    #[test]
    fn browse_session_handles_multiple_requests_until_eof() {
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [9; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(2, &V2Message::Keepalive { nonce: 7 }).unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                3,
                &V2Message::ListRequest {
                    path: Vec::new(),
                    page_token: 0,
                    page_size: 10,
                },
            )
            .unwrap(),
        );

        let mut output = Vec::new();
        Server::new(tempdir().unwrap().path())
            .run(Cursor::new(input), &mut output)
            .unwrap();

        let mut v1_decoder = FrameDecoder::new();
        let mut output_cursor = Cursor::new(output);
        let server_handshake = v1_decoder.read(&mut output_cursor).unwrap();
        assert!(matches!(
            server_handshake.message,
            Message::Handshake {
                role: Role::Session,
                capabilities,
                ..
            } if capabilities & CAP_BROWSE_V2 != 0
        ));
        assert!(matches!(
            v1_decoder.read(&mut output_cursor).unwrap().message,
            Message::Ack { .. }
        ));
        let position = output_cursor.position() as usize;
        let bytes = output_cursor.into_inner();
        let mut v2_cursor = Cursor::new(bytes);
        v2_cursor.set_position(position as u64);
        let first = protocol_v2::read_frame(&mut v2_cursor).unwrap().unwrap();
        assert_eq!(first.message_id, 1002);
        assert_eq!(first.message, V2Message::KeepaliveAck { nonce: 7 });
        let second = protocol_v2::read_frame(&mut v2_cursor).unwrap().unwrap();
        assert_eq!(second.message_id, 1003);
        assert_eq!(
            second.message,
            V2Message::ListPage {
                related_id: 3,
                page_token: 0,
                final_page: true,
                entries: Vec::new(),
            }
        );
    }

    #[test]
    fn list_pages_are_bounded_and_report_symlinks_without_following() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("one"), b"one").unwrap();
        fs::write(temp.path().join("two"), b"two").unwrap();
        fs::write(temp.path().join("three"), b"three").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("one", temp.path().join("link")).unwrap();
        }

        let server = Server::new(temp.path());
        let first = server.browse_list_page(b"", 0, 2, 41).unwrap();
        let V2Message::ListPage {
            page_token,
            final_page,
            entries: first_entries,
            ..
        } = first
        else {
            panic!("expected first list page");
        };
        assert_eq!(first_entries.len(), 2);
        assert!(!final_page);
        assert!(page_token > 0);

        let second = server.browse_list_page(b"", page_token, 2, 41).unwrap();
        let V2Message::ListPage {
            final_page,
            entries: second_entries,
            ..
        } = second
        else {
            panic!("expected second list page");
        };
        assert!(final_page);
        assert!(!second_entries.is_empty());
        #[cfg(unix)]
        {
            let link = first_entries
                .iter()
                .chain(second_entries.iter())
                .find(|entry| entry.name == b"link")
                .unwrap();
            assert_eq!(link.kind, 3);
            assert_eq!(link.symlink_target, b"one");
        }
        assert!(matches!(
            server.browse_list_page(b"../", 0, 2, 41),
            Err(ServerError::InvalidPath(_))
        ));
    }

    #[test]
    #[ignore = "filesystem benchmark; run explicitly with --ignored --nocapture"]
    fn list_first_page_100k_entry_benchmark() {
        let temp = tempdir().unwrap();
        for index in 0..100_000 {
            fs::write(temp.path().join(format!("entry-{index:06}")), []).unwrap();
        }
        let server = Server::new(temp.path());
        let started = std::time::Instant::now();
        let page = server.browse_list_page(b"", 0, 100, 1).unwrap();
        let elapsed = started.elapsed();
        let V2Message::ListPage {
            entries,
            final_page,
            ..
        } = page
        else {
            panic!("expected list page");
        };
        assert_eq!(entries.len(), 100);
        assert!(!final_page);
        eprintln!("list first page: 100/100000 entries in {elapsed:?}");
    }

    #[test]
    fn stat_reports_file_directory_symlink_and_missing_without_following() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("file"), b"contents").unwrap();
        fs::create_dir(temp.path().join("directory")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("file", temp.path().join("link")).unwrap();

        let server = Server::new(temp.path());
        let file = server.browse_stat_response(b"file", true, 1).unwrap();
        let V2Message::StatResponse {
            status,
            entry,
            digest,
            ..
        } = file
        else {
            panic!("expected file stat");
        };
        assert_eq!(status, protocol_v2::StatStatus::Ok);
        assert_eq!(entry.as_ref().unwrap().kind, 1);
        assert_eq!(digest, Some(*blake3::hash(b"contents").as_bytes()));

        let directory = server.browse_stat_response(b"directory", true, 2).unwrap();
        let V2Message::StatResponse {
            status,
            entry,
            digest,
            ..
        } = directory
        else {
            panic!("expected directory stat");
        };
        assert_eq!(status, protocol_v2::StatStatus::Ok);
        assert_eq!(entry.as_ref().unwrap().kind, 2);
        assert_eq!(digest, None);

        #[cfg(unix)]
        {
            let link = server.browse_stat_response(b"link", true, 3).unwrap();
            let V2Message::StatResponse { entry, digest, .. } = link else {
                panic!("expected symlink stat");
            };
            let entry = entry.unwrap();
            assert_eq!(entry.kind, 3);
            assert_eq!(entry.symlink_target, b"file");
            assert_eq!(digest, None);
        }

        let missing = server.browse_stat_response(b"missing", true, 4).unwrap();
        assert!(matches!(
            missing,
            V2Message::StatResponse {
                status: protocol_v2::StatStatus::Missing,
                entry: None,
                digest: None,
                ..
            }
        ));
        assert!(matches!(
            server.browse_stat_response(b"../escape", false, 5),
            Err(ServerError::InvalidPath(_))
        ));
    }

    #[test]
    fn mutations_are_atomic_scoped_and_actionable() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("old"), b"data").unwrap();
        fs::write(temp.path().join("existing"), b"keep").unwrap();
        let server = Server::new(temp.path());

        assert_eq!(
            server.browse_rename_response(b"old", b"new", 1),
            V2Message::RenameResponse {
                related_id: 1,
                status: MutationStatus::Ok,
                error: Vec::new(),
            }
        );
        assert!(temp.path().join("new").is_file());
        assert_eq!(
            server.browse_rename_response(b"new", b"existing", 2),
            V2Message::RenameResponse {
                related_id: 2,
                status: MutationStatus::AlreadyExists,
                error: b"destination already exists".to_vec(),
            }
        );
        assert!(matches!(
            server.browse_create_directory_response(b"parent/child", 3),
            V2Message::CreateDirectoryResponse {
                related_id: 3,
                status: MutationStatus::ParentMissing,
                error,
            } if !error.is_empty()
        ));
        assert!(matches!(
            server.browse_create_directory_response(b"../escape", 4),
            V2Message::CreateDirectoryResponse {
                status: MutationStatus::Error,
                ..
            }
        ));
    }

    #[test]
    fn recursive_delete_reports_each_item_and_is_irreversible() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("tree")).unwrap();
        fs::create_dir(temp.path().join("tree/nested")).unwrap();
        fs::write(temp.path().join("tree/file"), b"data").unwrap();
        fs::write(temp.path().join("tree/nested/other"), b"data").unwrap();

        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [11; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                2,
                &V2Message::DeleteRequest {
                    path: b"tree".to_vec(),
                },
            )
            .unwrap(),
        );
        let mut output = Vec::new();
        Server::new(temp.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        assert!(!temp.path().join("tree").exists());

        let mut v1 = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        v1.read(&mut cursor).unwrap();
        v1.read(&mut cursor).unwrap();
        let position = cursor.position();
        let bytes = cursor.into_inner();
        let mut v2 = Cursor::new(bytes);
        v2.set_position(position);
        let mut progress = 0;
        let mut final_response = None;
        while let Some(frame) = protocol_v2::read_frame(&mut v2).unwrap() {
            match frame.message {
                V2Message::DeleteProgress { .. } => progress += 1,
                response @ V2Message::DeleteResponse { .. } => final_response = Some(response),
                other => panic!("unexpected delete response: {other:?}"),
            }
        }
        assert_eq!(progress, 4);
        assert!(matches!(
            final_response,
            Some(V2Message::DeleteResponse {
                status: protocol_v2::DeleteStatus::Complete,
                removed_count: 4,
                irreversible: true,
                ref failures,
                ..
            }) if failures.is_empty()
        ));
    }

    #[test]
    fn fetch_reads_a_stable_file_and_returns_identity_and_digest() {
        let temp = tempdir().unwrap();
        let contents = vec![b'x'; 1024 * 1024 + 3];
        fs::write(temp.path().join("edit.txt"), &contents).unwrap();
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [12; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                2,
                &V2Message::FetchRequest {
                    path: b"edit.txt".to_vec(),
                },
            )
            .unwrap(),
        );
        let mut output = Vec::new();
        Server::new(temp.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        let mut v1 = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        v1.read(&mut cursor).unwrap();
        v1.read(&mut cursor).unwrap();
        let position = cursor.position();
        let bytes = cursor.into_inner();
        let mut v2 = Cursor::new(bytes);
        v2.set_position(position);
        let start = protocol_v2::read_frame(&mut v2).unwrap().unwrap();
        let V2Message::FetchStart {
            size,
            digest,
            related_id,
            ..
        } = start.message
        else {
            panic!("expected fetch start");
        };
        assert_eq!(related_id, 2);
        assert_eq!(size, contents.len() as u64);
        assert_eq!(digest, *blake3::hash(&contents).as_bytes());
        let mut fetched = Vec::new();
        while let Some(frame) = protocol_v2::read_frame(&mut v2).unwrap() {
            match frame.message {
                V2Message::FetchChunk { data, .. } => fetched.extend(data),
                other => panic!("unexpected fetch frame: {other:?}"),
            }
        }
        assert_eq!(fetched, contents);
    }

    #[test]
    fn publish_refuses_changed_remote_identity_and_commits_matching_file_atomically() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("edit.txt");
        fs::write(&path, b"old").unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let mtime = metadata.modified().unwrap();
        let fingerprint = fingerprint_from_metadata(&metadata, ScanEntryKind::File, mtime).unwrap();
        let digest = *blake3::hash(b"new").as_bytes();
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [13; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                2,
                &V2Message::PublishRequest {
                    path: b"edit.txt".to_vec(),
                    size: 3,
                    mtime_ns: system_time_to_nanos(mtime),
                    device: fingerprint.identity.device,
                    file: fingerprint.identity.file,
                    digest,
                },
            )
            .unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                3,
                &V2Message::PublishChunk {
                    related_id: 2,
                    offset: 0,
                    data: b"new".to_vec(),
                },
            )
            .unwrap(),
        );
        let mut output = Vec::new();
        Server::new(temp.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let mut v1 = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        v1.read(&mut cursor).unwrap();
        v1.read(&mut cursor).unwrap();
        let position = cursor.position();
        let bytes = cursor.into_inner();
        let mut v2 = Cursor::new(bytes);
        v2.set_position(position);
        assert!(matches!(
            protocol_v2::read_frame(&mut v2).unwrap().unwrap().message,
            V2Message::PublishReady { related_id: 2 }
        ));
        assert!(matches!(
            protocol_v2::read_frame(&mut v2).unwrap().unwrap().message,
            V2Message::PublishResponse {
                status: protocol_v2::PublishStatus::Ok,
                ..
            }
        ));
    }

    #[test]
    fn browse_session_rejects_duplicate_ids_and_v1_frames_after_selection() {
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities: CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [8; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let opening = encode_frame(1, &handshake).unwrap();

        let mut duplicate = opening.clone();
        let keepalive = protocol_v2::encode_frame(2, &V2Message::Keepalive { nonce: 1 }).unwrap();
        duplicate.extend_from_slice(&keepalive);
        duplicate.extend_from_slice(&keepalive);
        let duplicate_error = Server::new(tempdir().unwrap().path())
            .run(Cursor::new(duplicate), &mut Vec::new())
            .unwrap_err();
        assert!(duplicate_error
            .to_string()
            .contains("duplicate v2 session message ID 2"));

        let mut mixed = opening;
        mixed.extend_from_slice(
            &encode_frame(
                2,
                &Message::Ack {
                    acknowledged_id: 1,
                    acknowledged_type: 1,
                },
            )
            .unwrap(),
        );
        let mixed_error = Server::new(tempdir().unwrap().path())
            .run(Cursor::new(mixed), &mut Vec::new())
            .unwrap_err();
        assert!(mixed_error
            .to_string()
            .contains("malformed v2 envelope: wrong version"));
    }

    #[test]
    fn browse_session_cancels_a_non_final_list_without_emitting_more_pages() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("one"), b"one").unwrap();
        fs::write(root.path().join("two"), b"two").unwrap();
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities: CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [7; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                2,
                &V2Message::ListRequest {
                    path: Vec::new(),
                    page_token: 0,
                    page_size: 1,
                },
            )
            .unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(3, &V2Message::CancelRequest { related_id: 2 }).unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(4, &V2Message::CancelRequest { related_id: 99 }).unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(5, &V2Message::Keepalive { nonce: 55 }).unwrap(),
        );
        let mut output = Vec::new();
        Server::new(root.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();

        let mut v1 = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        v1.read(&mut cursor).unwrap();
        v1.read(&mut cursor).unwrap();
        let first = protocol_v2::read_frame(&mut cursor).unwrap().unwrap();
        assert!(matches!(
            first.message,
            V2Message::ListPage {
                final_page: false,
                ..
            }
        ));
        let second = protocol_v2::read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(
            second.message,
            V2Message::BrowseError {
                related_id: 2,
                code: 1,
                message: b"request cancelled".to_vec(),
            }
        );
        let completed = protocol_v2::read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(
            completed.message,
            V2Message::BrowseError {
                related_id: 99,
                code: 1,
                message: b"request already complete".to_vec(),
            }
        );
        let keepalive = protocol_v2::read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(keepalive.message, V2Message::KeepaliveAck { nonce: 55 });
        assert!(protocol_v2::read_frame(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn browse_client_driver_negotiates_and_keeps_alive() {
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let server_handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [4; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut peer_bytes = encode_frame(1000, &server_handshake).unwrap();
        peer_bytes.extend_from_slice(
            &encode_frame(
                1001,
                &Message::Ack {
                    acknowledged_id: 1,
                    acknowledged_type: 1,
                },
            )
            .unwrap(),
        );
        peer_bytes.extend_from_slice(
            &protocol_v2::encode_frame(1002, &V2Message::KeepaliveAck { nonce: 11 }).unwrap(),
        );
        peer_bytes.extend_from_slice(
            &protocol_v2::encode_frame(
                1003,
                &V2Message::ListPage {
                    related_id: 3,
                    page_token: 0,
                    final_page: true,
                    entries: vec![BrowseEntry {
                        name: b"file".to_vec(),
                        kind: 1,
                        size: 4,
                        mtime_ns: 0,
                        mode: 0o644,
                        symlink_target: Vec::new(),
                    }],
                },
            )
            .unwrap(),
        );

        let mut session =
            BrowseSession::connect(Cursor::new(peer_bytes), Vec::new(), [3; 16]).unwrap();
        assert_eq!(session.remote_capabilities(), capabilities);
        assert_eq!(session.common_capabilities(), capabilities);
        session.keepalive(11).unwrap();
        let (next_token, final_page, entries) = session.list_page(Vec::new(), 0, 100).unwrap();
        assert_eq!(next_token, 0);
        assert!(final_page);
        assert_eq!(entries[0].name, b"file");
        let (reader, writer) = session.into_parts();
        assert!(reader.position() > 0);

        let mut sent = Cursor::new(writer);
        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.read(&mut sent).unwrap().message,
            Message::Handshake {
                role: Role::Session,
                ..
            }
        ));
        let v2_position = sent.position();
        let mut v2_sent = Cursor::new(sent.into_inner());
        v2_sent.set_position(v2_position);
        let request = protocol_v2::read_frame(&mut v2_sent).unwrap().unwrap();
        assert_eq!(request.message_id, 2);
        assert_eq!(request.message, V2Message::Keepalive { nonce: 11 });
        let list_request = protocol_v2::read_frame(&mut v2_sent).unwrap().unwrap();
        assert_eq!(list_request.message_id, 3);
        assert!(matches!(
            list_request.message,
            V2Message::ListRequest { .. }
        ));
    }

    #[test]
    fn browse_client_names_clean_peer_disconnect() {
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [5; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut peer_bytes = encode_frame(1000, &handshake).unwrap();
        peer_bytes.extend_from_slice(
            &encode_frame(
                1001,
                &Message::Ack {
                    acknowledged_id: 1,
                    acknowledged_type: 1,
                },
            )
            .unwrap(),
        );
        let mut session =
            BrowseSession::connect(Cursor::new(peer_bytes), Vec::new(), [6; 16]).unwrap();
        assert_eq!(
            session.receive().unwrap_err().to_string(),
            "browse peer disconnected"
        );
    }

    #[test]
    fn delete_failure_is_reported_as_a_warning_and_partial_failure() {
        let entry = FileEntry {
            path: WirePath::from("stale.txt"),
            kind: ScanEntryKind::File,
            size: 0,
            mtime: UNIX_EPOCH,
            mode: 0o644,
            fingerprint: SourceFingerprint::synthetic(ScanEntryKind::File, 0, UNIX_EPOCH),
        };
        let mut report = LocalSyncReport::default();
        let mut events = Vec::new();
        record_delete_failure(
            &mut report,
            &mut |event| events.push(event),
            &entry,
            ServerError::RemoteError {
                code: 1001,
                message: "permission denied".to_owned(),
            },
        );

        assert_eq!(report.failed_entries, 1);
        assert_eq!(report.warnings, 1);
        assert!(report.partial_failure());
        assert!(matches!(events.first(), Some(LocalEvent::Warning { .. })));
        assert!(matches!(events.get(1), Some(LocalEvent::Failed { .. })));
    }

    #[test]
    fn unrouted_segment_is_a_loud_error_not_a_silent_drop() {
        let dst = tempdir().unwrap();
        let mut input = Vec::new();

        // Handshake: client is the source (we are sending data at the sink).
        input.extend_from_slice(
            &encode_frame(
                1,
                &Message::Handshake {
                    role: Role::Source,
                    capabilities: 0,
                    max_payload: MAX_COMPLETE_PAYLOAD as u32,
                    max_segment: MAX_DATA_SEGMENT as u32,
                    window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                    job_id: [9u8; 16],
                    compression: CompressionMode::None,
                    compression_level: 3,
                },
            )
            .unwrap(),
        );

        // SessionConfig.
        input.extend_from_slice(
            &encode_frame(
                2,
                &Message::SessionConfig {
                    streams: 1,
                    batch_bytes: 32 * 1024 * 1024,
                    chunk_bytes: 16 * 1024 * 1024,
                    window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                    delete: false,
                    checksum: false,
                    paranoid: false,
                    dry_run: false,
                    exclude_patterns: Vec::new(),
                },
            )
            .unwrap(),
        );

        // Ack the (empty) destination Scan page the server sends after config.
        input.extend_from_slice(
            &encode_frame(
                3,
                &Message::Ack {
                    acknowledged_id: 1002,
                    acknowledged_type: 9,
                },
            )
            .unwrap(),
        );

        // A FileSegment whose file_id was never prepared/batched: this must fail
        // loudly rather than report success while dropping the bytes.
        input.extend_from_slice(
            &encode_frame(
                4,
                &Message::FileSegment {
                    file_id: 9_999,
                    offset: 0,
                    data: b"must not be silently dropped".to_vec(),
                },
            )
            .unwrap(),
        );

        let mut server = Server::new(dst.path());
        let mut output = Vec::new();
        let result = server.run(Cursor::new(input), &mut output);
        assert!(
            matches!(
                &result,
                Err(ServerError::UnexpectedMessage(msg)) if msg.contains("unregistered file_id")
            ),
            "unregistered FileSegment must be rejected, got {result:?}"
        );
        // No file may be published for the dropped segment.
        let count = dst.path().read_dir().map_or(0, |iter| iter.count());
        assert_eq!(count, 0);
    }

    #[test]
    fn data_only_session_writes_ranges_and_skips_the_scan() {
        let dst = tempdir().unwrap();
        let mut input = Vec::new();

        // Handshake requesting a data-only session.
        input.extend_from_slice(
            &encode_frame(
                1,
                &Message::Handshake {
                    role: Role::Source,
                    capabilities: crate::protocol::CAP_DATA_ONLY,
                    max_payload: MAX_COMPLETE_PAYLOAD as u32,
                    max_segment: MAX_DATA_SEGMENT as u32,
                    window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                    job_id: [12u8; 16],
                    compression: CompressionMode::None,
                    compression_level: 3,
                },
            )
            .unwrap(),
        );
        input.extend_from_slice(
            &encode_frame(
                2,
                &Message::SessionConfig {
                    streams: 1,
                    batch_bytes: 32 * 1024 * 1024,
                    chunk_bytes: 16 * 1024 * 1024,
                    window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
                    delete: false,
                    checksum: false,
                    paranoid: false,
                    dry_run: false,
                    exclude_patterns: Vec::new(),
                },
            )
            .unwrap(),
        );
        // Prepare a 9-byte large file, then write its first 5 bytes.
        input.extend_from_slice(
            &encode_frame(
                3,
                &Message::LargeFilePrepare {
                    file_id: 501,
                    path: b"big.bin".to_vec(),
                    size: 9,
                    mtime_ns: 1_000_000_000,
                    mode: 0o644,
                    fingerprint: [1u8; 32],
                },
            )
            .unwrap(),
        );
        input.extend_from_slice(
            &encode_frame(
                4,
                &Message::LargeFileRange {
                    file_id: 501,
                    range: ByteRange {
                        offset: 0,
                        length: 5,
                    },
                },
            )
            .unwrap(),
        );
        input.extend_from_slice(
            &encode_frame(
                5,
                &Message::FileSegment {
                    file_id: 501,
                    offset: 0,
                    data: b"hello".to_vec(),
                },
            )
            .unwrap(),
        );

        let mut server = Server::new(dst.path());
        let mut output = Vec::new();
        let result = server.run(Cursor::new(input), &mut output);
        assert!(
            result.is_ok(),
            "data session must terminate cleanly: {result:?}"
        );

        // The data-only server must not have sent a destination Scan, and it
        // must have acknowledged the segment.
        let mut dec = FrameDecoder::new();
        let mut cur = Cursor::new(&output);
        let mut saw_scan = false;
        let mut saw_seg_ack = false;
        while (cur.position() as usize) < output.len() {
            let frame = dec.read(&mut cur).unwrap();
            match frame.message {
                Message::Scan { .. } => saw_scan = true,
                Message::Ack {
                    acknowledged_type: 4,
                    ..
                } => saw_seg_ack = true,
                _ => {}
            }
        }
        assert!(
            !saw_scan,
            "data-only session must skip the destination scan"
        );
        assert!(saw_seg_ack, "segment must be acknowledged");

        // The range landed in the shared stage at offset 0.
        let sink = Sink::new(dst.path()).unwrap();
        let stage = fs::read(sink.temporary_path("big.bin").unwrap()).unwrap();
        assert_eq!(&stage[0..5], b"hello");
    }

    #[test]
    fn default_remote_shell_is_ssh_over_host() {
        let (program, args) =
            remote_server_command_with_shell("/dest", None, Some("user@mars"), RemoteShell::Posix)
                .unwrap();
        assert_eq!(program, "ssh");
        assert_eq!(
            args,
            [
                "user@mars",
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' '/dest'"
            ]
        );
    }

    #[test]
    fn explicit_rsh_replaces_the_shell_but_preserves_host_and_args() {
        let (program, args) = remote_server_command_with_shell(
            "/dest",
            Some("myrsh -oK=1"),
            Some("host"),
            RemoteShell::Posix,
        )
        .unwrap();
        assert_eq!(program, "myrsh");
        assert_eq!(
            args,
            [
                "-oK=1",
                "host",
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' '/dest'"
            ]
        );
    }

    #[test]
    fn remote_path_is_shell_quoted_as_one_command() {
        let (_, args) =
            remote_server_command_with_shell(
                "/dst'; touch XSYNC_INJECTION; echo '",
                None,
                Some("host"),
                RemoteShell::Posix,
            )
            .unwrap();
        assert_eq!(
            args,
            [
                "host",
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' '/dst'\\''; touch XSYNC_INJECTION; echo '\\'''"
            ]
        );
    }

    #[test]
    fn remote_home_path_is_expanded_by_the_remote_shell() {
        let (_, args) =
            remote_server_command_with_shell("~", None, Some("host"), RemoteShell::Posix).unwrap();
        assert_eq!(
            args,
            [
                "host",
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' \"$HOME\""
            ]
        );

        let (_, args) =
            remote_server_command_with_shell("~/nested", None, Some("host"), RemoteShell::Posix)
                .unwrap();
        assert_eq!(
            args,
            [
                "host",
                "PATH=\"$HOME/.local/bin:$PATH\" 'xs' '--server' \"$HOME\"/'nested'"
            ]
        );
    }

    #[test]
    fn remote_shell_is_remembered_per_host_and_rsh() {
        // The learned family must not leak between hosts: one Windows box in a
        // script must not push every later POSIX host down the cmd.exe path.
        remember_remote_shell(None, "winbox.example", RemoteShell::Windows);
        assert_eq!(
            remote_shell_for(None, "winbox.example"),
            RemoteShell::Windows
        );
        assert_eq!(
            remote_shell_for(None, "linuxbox.example"),
            RemoteShell::Posix,
            "an unrelated host must still start from the POSIX default"
        );
        // A different -e command against the same host is a different route and
        // does not inherit the answer.
        assert_eq!(
            remote_shell_for(Some("ssh -p 2222"), "winbox.example"),
            RemoteShell::Posix
        );
    }

    #[test]
    fn windows_remote_uses_cmd_syntax_not_posix() {
        // Stock Windows OpenSSH hands the command to cmd.exe, which cannot parse
        // a POSIX assignment prefix or single-quoted words.
        let (program, args) = remote_server_command_with_shell(
            "C:/backup",
            None,
            Some("winbox"),
            RemoteShell::Windows,
        )
        .unwrap();
        assert_eq!(program, "ssh");
        assert_eq!(
            args,
            [
                "winbox",
                "set \"PATH=%USERPROFILE%\\.local\\bin;%PATH%\" & xs --server \"C:/backup\""
            ]
        );
    }

    #[test]
    fn windows_remote_expands_home_with_userprofile() {
        let (_, args) =
            remote_server_command_with_shell("~", None, Some("winbox"), RemoteShell::Windows)
                .unwrap();
        assert!(args[1].ends_with("xs --server \"%USERPROFILE%\""), "{}", args[1]);

        let (_, args) = remote_server_command_with_shell(
            "~/backup",
            None,
            Some("winbox"),
            RemoteShell::Windows,
        )
        .unwrap();
        assert!(
            args[1].ends_with("xs --server \"%USERPROFILE%\\backup\""),
            "{}",
            args[1]
        );
    }

    #[test]
    fn windows_remote_refuses_paths_cmd_quoting_cannot_express() {
        // `%` still expands inside cmd double quotes, and a quote ends the run.
        // Both are refused rather than silently mangled into a different path.
        for hostile in [
            r"C:/%USERPROFILE%/evil",
            r#"C:/dest" & del /q C:\important & echo "#,
        ] {
            let result = remote_server_command_with_shell(
                hostile,
                None,
                Some("winbox"),
                RemoteShell::Windows,
            );
            assert!(
                matches!(result, Err(ServerError::InvalidPath(_))),
                "expected refusal for {hostile:?}"
            );
        }
    }

    #[test]
    fn windows_and_posix_forms_differ_for_the_same_path() {
        let posix =
            remote_server_command_with_shell("/data", None, Some("h"), RemoteShell::Posix).unwrap();
        let windows =
            remote_server_command_with_shell("/data", None, Some("h"), RemoteShell::Windows)
                .unwrap();
        assert_ne!(posix.1, windows.1);
        assert!(posix.1[1].starts_with("PATH="));
        assert!(windows.1[1].starts_with("set \"PATH="));
    }

    #[test]
    fn no_host_runs_an_in_process_local_server() {
        let (program, args) =
            remote_server_command_with_shell("/dest", None, None, RemoteShell::Posix).unwrap();
        assert_ne!(program, "ssh");
        assert_eq!(args, ["--server", "/dest"]);
    }

    #[test]
    fn server_sink_and_client_push_round_trip() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();

        // Create test source structure
        fs::create_dir_all(src_dir.path().join("sub")).unwrap();
        fs::write(src_dir.path().join("sub/hello.txt"), b"hello world").unwrap();
        fs::write(src_dir.path().join("large.bin"), vec![42u8; 100_000]).unwrap();

        // In-memory duplex pipe using threads
        let (client_tx, server_rx) = crossbeam_channel::bounded::<Vec<u8>>(128);
        let (server_tx, client_rx) = crossbeam_channel::bounded::<Vec<u8>>(128);

        struct ChannelWriter(crossbeam_channel::Sender<Vec<u8>>);
        impl Write for ChannelWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0
                    .send(buf.to_vec())
                    .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct ChannelReader {
            rx: crossbeam_channel::Receiver<Vec<u8>>,
            buffer: Vec<u8>,
            pos: usize,
        }
        impl Read for ChannelReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.pos >= self.buffer.len() {
                    match self.rx.recv() {
                        Ok(data) => {
                            self.buffer = data;
                            self.pos = 0;
                        }
                        Err(_) => return Ok(0),
                    }
                }
                let available = &self.buffer[self.pos..];
                let to_copy = std::cmp::min(buf.len(), available.len());
                buf[..to_copy].copy_from_slice(&available[..to_copy]);
                self.pos += to_copy;
                Ok(to_copy)
            }
        }

        let dst_path = dst_dir.path().to_path_buf();
        let server_thread = std::thread::spawn(move || {
            let mut server = Server::new_with_capabilities(dst_path, 0);
            let reader = ChannelReader {
                rx: server_rx,
                buffer: Vec::new(),
                pos: 0,
            };
            let writer = ChannelWriter(server_tx);
            server.run(reader, writer).unwrap();
        });

        // Client side
        let client_reader = ChannelReader {
            rx: client_rx,
            buffer: Vec::new(),
            pos: 0,
        };
        let client_writer = ChannelWriter(client_tx);

        let options = LocalSyncOptions {
            paranoid: true,
            ..LocalSyncOptions::default()
        };
        let mut events = Vec::new();
        let report = run_client_push(
            src_dir.path(),
            true,
            dst_dir.path().to_str().unwrap(),
            true,
            &options,
            client_reader,
            client_writer,
            |ev| events.push(ev),
        )
        .unwrap();

        server_thread.join().unwrap();

        assert_eq!(report.transferred_files, 2);
        assert!(!report.partial_failure());
        assert!(events.iter().any(|event| matches!(
            event,
            LocalEvent::Negotiated {
                compression_algorithm: "none",
                compression_reason: "remote peer does not advertise zstd"
            }
        )));

        // Verify destination matches
        let hello = fs::read(dst_dir.path().join("sub/hello.txt")).unwrap();
        assert_eq!(hello, b"hello world");
        let large = fs::read(dst_dir.path().join("large.bin")).unwrap();
        assert_eq!(large, vec![42u8; 100_000]);
    }

    #[test]
    fn server_source_and_client_pull_round_trip() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();

        // Create test source structure
        fs::create_dir_all(src_dir.path().join("nested/dir")).unwrap();
        fs::write(src_dir.path().join("nested/dir/data.bin"), b"pull data").unwrap();

        let (client_tx, server_rx) = crossbeam_channel::bounded::<Vec<u8>>(128);
        let (server_tx, client_rx) = crossbeam_channel::bounded::<Vec<u8>>(128);

        struct ChannelWriter(crossbeam_channel::Sender<Vec<u8>>);
        impl Write for ChannelWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0
                    .send(buf.to_vec())
                    .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        struct ChannelReader {
            rx: crossbeam_channel::Receiver<Vec<u8>>,
            buffer: Vec<u8>,
            pos: usize,
        }
        impl Read for ChannelReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.pos >= self.buffer.len() {
                    match self.rx.recv() {
                        Ok(data) => {
                            self.buffer = data;
                            self.pos = 0;
                        }
                        Err(_) => return Ok(0),
                    }
                }
                let available = &self.buffer[self.pos..];
                let to_copy = std::cmp::min(buf.len(), available.len());
                buf[..to_copy].copy_from_slice(&available[..to_copy]);
                self.pos += to_copy;
                Ok(to_copy)
            }
        }

        let src_path = src_dir.path().to_path_buf();
        let server_thread = std::thread::spawn(move || {
            let mut server = Server::new_with_capabilities(src_path, 0);
            let reader = ChannelReader {
                rx: server_rx,
                buffer: Vec::new(),
                pos: 0,
            };
            let writer = ChannelWriter(server_tx);
            server.run(reader, writer).unwrap();
        });

        // Client side
        let client_reader = ChannelReader {
            rx: client_rx,
            buffer: Vec::new(),
            pos: 0,
        };
        let client_writer = ChannelWriter(client_tx);

        let options = LocalSyncOptions {
            paranoid: true,
            ..LocalSyncOptions::default()
        };
        let mut events = Vec::new();
        let report = run_client_pull(
            src_dir.path().to_str().unwrap(),
            true,
            dst_dir.path(),
            true,
            &options,
            client_reader,
            client_writer,
            |ev| events.push(ev),
        )
        .unwrap();

        server_thread.join().unwrap();

        assert!(events.iter().any(|event| matches!(
            event,
            LocalEvent::Negotiated {
                compression_algorithm: "none",
                compression_reason: "remote peer does not advertise zstd"
            }
        )));

        assert_eq!(report.transferred_files, 1);
        let pull_data = fs::read(dst_dir.path().join("nested/dir/data.bin")).unwrap();
        assert_eq!(pull_data, b"pull data");
    }

    #[test]
    fn server_stdout_is_protocol_only_and_all_bytes_are_valid_frames() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();

        fs::write(src_dir.path().join("test.txt"), b"stdout validity").unwrap();

        // Capture all output written by server into a buffer
        let mut server_output = Vec::new();
        let mut client_to_server = Vec::new();

        // Encode Handshake from client
        let hs = Message::Handshake {
            role: Role::Source,
            capabilities: 0,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [0u8; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        client_to_server.extend_from_slice(&encode_frame(1, &hs).unwrap());

        // SessionConfig
        let sc = Message::SessionConfig {
            streams: 1,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            delete: false,
            checksum: false,
            paranoid: false,
            dry_run: false,
            exclude_patterns: Vec::new(),
        };
        client_to_server.extend_from_slice(&encode_frame(2, &sc).unwrap());

        // Scan Ack
        let ack_scan = Message::Ack {
            acknowledged_id: 1003, // server scan frame id
            acknowledged_type: 9,
        };
        client_to_server.extend_from_slice(&encode_frame(3, &ack_scan).unwrap());

        // FileBatch + FileSegment
        let rec = EntryRecord {
            path: b"test.txt".to_vec(),
            kind: crate::protocol::EntryKind::File,
            size: 15,
            mtime_ns: 1_000_000_000,
            mode: 0o644,
            fingerprint: [0u8; 32],
        };
        let fb = Message::FileBatch {
            batch_id: 1,
            entries: vec![rec],
        };
        client_to_server.extend_from_slice(&encode_frame(4, &fb).unwrap());

        let fs_msg = Message::FileSegment {
            file_id: (4 << 16) | 0,
            offset: 0,
            data: b"stdout validity".to_vec(),
        };
        client_to_server.extend_from_slice(&encode_frame(5, &fs_msg).unwrap());

        // Stats
        let stats = Message::Stats {
            files: 1,
            bytes: 15,
            skipped: 0,
            warnings: 0,
            failed: 0,
        };
        client_to_server.extend_from_slice(&encode_frame(6, &stats).unwrap());

        let mut server = Server::new(dst_dir.path());
        let reader = Cursor::new(client_to_server);
        let _ = server.run(reader, &mut server_output);

        // Verify every byte in server_output is part of a valid protocol frame
        let mut decoder = FrameDecoder::new();
        let mut cursor = Cursor::new(&server_output);
        let mut frame_count = 0;
        while cursor.position() < server_output.len() as u64 {
            let frame = decoder
                .read(&mut cursor)
                .expect("all bytes must form valid frames");
            frame_count += 1;
            assert!(frame.message_id >= 1000);
        }
        assert!(frame_count >= 5);
        assert_eq!(cursor.position(), server_output.len() as u64);
    }

    #[test]
    fn rejects_escape_and_symlink_traversal() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dest");
        fs::create_dir(&root).unwrap();

        assert!(validate_destination_path(&root, "../escape").is_err());
        assert!(validate_destination_path(&root, "/etc/passwd").is_err());
        assert!(validate_destination_path(&root, "a/../../b").is_err());
        assert!(validate_destination_path(&root, "").is_err());

        // Pre-existing directory symlink
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("link_dir")).unwrap();
            assert!(validate_destination_path(&root, "link_dir/file.txt").is_err());

            fs::write(&outside.join("file"), b"outside").unwrap();
            std::os::unix::fs::symlink(outside.join("file"), root.join("link_file")).unwrap();
            assert!(matches!(
                validate_destination_path(&root, "link_file/child"),
                Err(ServerError::SymlinkEscape(_))
            ));
        }
    }

    #[test]
    fn mutation_validation_registers_only_unique_normalized_paths() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dest");
        fs::create_dir(&root).unwrap();
        let mut seen = HashSet::new();

        validate_unique_destination_path(
            &root,
            WirePath::from_wire(b"same/path".to_vec()).unwrap(),
            &mut seen,
        )
        .unwrap();
        assert!(matches!(
            validate_unique_destination_path(
                &root,
                WirePath::from_wire(b"same/path".to_vec()).unwrap(),
                &mut seen,
            ),
            Err(ServerError::DuplicatePath(path)) if path == "same/path"
        ));
        assert_eq!(seen.len(), 1);
    }
}
