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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use filetime::{set_file_mtime, FileTime};
use thiserror::Error;

use crate::hash_cache::{HashCache, HashFingerprint};
use crate::local::{LocalEvent, LocalSyncOptions, LocalSyncReport, TransferMethod};
use crate::path::WirePath;
use crate::planner::{try_plan_with_fingerprint, DestinationIndex, IndexConfig, PlannerError};
use crate::protocol::{
    common_capabilities, encode_frame, encode_frame_with_compression, negotiate_compression,
    negotiate_protocol_version, ByteRange, CompressionMode, EntryRecord, FrameDecoder, Message,
    MetadataOperation, ProtocolError, Role, CAP_BROWSE_META, CAP_BROWSE_V2, CAP_FILTER_RULES,
    CAP_FS_V3, CAP_UNIX_MODES, CAP_VERSION_NEGOTIATION, CAP_ZSTD, DEFAULT_UNACKNOWLEDGED_WINDOW,
    MAX_COLLECTION_COUNT, MAX_COMPLETE_PAYLOAD, MAX_DATA_SEGMENT,
};
use crate::protocol_v2::{self, V2CodecError, V2Frame, V2Message};
use crate::protocol_v2::{BrowseEntry, MutationStatus, StatStatus};
use crate::protocol_v3::{
    self, ErrorCode as FsErrorCode, StatTarget, V3CodecError, V3Frame, V3Message,
};
use crate::scanner::{
    fingerprint_from_metadata, permission_mode, scan, scan_with_filter, EntryKind as ScanEntryKind,
    FileEntry, FileIdentity, ScanError, SourceFingerprint,
};
use crate::sink::{Sink, SinkError, SymlinkTargetKind};
use crate::source::{SourceReadError, SourceReader};
use crate::strategy::SMALL_FILE_LIMIT;
use crate::tuning::{apply_worker_count, APPLY_JOBS_PER_WORKER};

/// Emit server lifecycle diagnostics without contaminating the binary
/// protocol on stdout. Stderr is intentionally safe for SSH diagnostics.
///
/// With a failure log configured, the whole stream becomes JSON records rather
/// than a mix of JSON and bare text, so the client can parse every line it
/// relays instead of guessing which ones are structured.
fn server_log(message: impl std::fmt::Display) {
    if crate::faillog::is_enabled() {
        let text = message.to_string();
        crate::faillog::write(&crate::faillog::Record {
            severity: crate::faillog::Severity::Info,
            origin: crate::faillog::Origin::Server,
            kind: "lifecycle",
            path: None,
            host: None,
            message: &text,
        });
        return;
    }
    eprintln!("[xsync server] {message}");
}

/// Report a server-side failure in human-readable form.
///
/// Structured records are written by exactly one place -- the binary's
/// top-level error handler, which sees every failure including those raised
/// before a session starts. Emitting here as well produced two records for one
/// failure, so this reports only when nothing structured is being written.
fn server_fail(error: &ServerError) {
    if !crate::faillog::is_enabled() {
        eprintln!("[xsync server] session failed: {error}");
    }
}

impl ServerError {
    /// Stable machine-readable family for this error.
    ///
    /// Kept deliberately coarse and separate from the `Display` text: the
    /// message is for a human and may be reworded, while a consumer routing on
    /// failures needs something it can match on that will not change with
    /// wording. Grouping is by what an operator would *do* about it -- a path
    /// problem is fixed differently from a transport one.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Protocol(_)
            | Self::UnexpectedMessage(_)
            | Self::Browse(_)
            | Self::FsSession(_) => "protocol",
            Self::Io(_) | Self::Sink(_) | Self::SourceRead(_) | Self::Journal(_) => "io",
            Self::Scan(_) => "scan",
            Self::Planner(_) => "plan",
            Self::InvalidPath(_)
            | Self::SymlinkEscape(_)
            | Self::DuplicatePath(_)
            | Self::PathCollision(_) => "path",
            Self::RemoteError { .. } => "remote",
            Self::MissingRemoteXsync => "missing-remote-binary",
            Self::RemoteFlagRejected => "remote-flag-rejected",
            Self::Bootstrap(_) => "bootstrap",
            Self::RemoteShellMismatch => "remote-shell",
            Self::FilterUnrepresentable(_) => "filter-unrepresentable",
            Self::DeleteRefused(_) => "delete-refused",
            Self::Transport { .. } => "transport",
            Self::PeerDisconnected => "peer-disconnected",
        }
    }
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
    /// Two source paths would become one file on this destination.
    #[error("{0}")]
    PathCollision(String),
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

    /// The peer cannot represent the requested filter, so it is refused
    /// rather than approximated into a wider transfer.
    #[error("{0}")]
    FilterUnrepresentable(String),

    /// A `--delete` run was refused before removing anything.
    #[error("{0}")]
    DeleteRefused(String),
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
    /// A filesystem-session frame was malformed or used the wrong message shape.
    #[error("v3 session error: {0}")]
    FsSession(#[from] V3CodecError),
    /// The remote shell positively reported that xsync is unavailable.
    #[error("xs not found on remote host — install it or check PATH")]
    MissingRemoteXsync,
    /// The remote's argument parser refused a flag this build sent.
    ///
    /// Internal: the caller drops the flag for that host and retries, so a
    /// logging preference never costs a transfer.
    #[error("remote rejected a command-line flag")]
    RemoteFlagRejected,
    /// Provisioning a binary onto a remote that lacked one failed.
    #[error("remote bootstrap failed: {0}")]
    Bootstrap(String),
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
    /// The peer selected the v3 filesystem grammar.
    ///
    /// Only reachable from [`probe_fs_session`], which is the probe that
    /// advertises `CAP_FS_V3`; [`probe_session`] never produces it, so an
    /// existing browse consumer keeps its exact behaviour against a v3 server.
    ReadyV3,
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
            Self::ReadyV3 => "open the filesystem session",
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] when the probe did not settle
    /// on [`ProbeStatus::Ready`], since a session may not begin on a connection
    /// whose capability negotiation did not complete.
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

    /// Continue into a v3 filesystem session, performing the `Features`
    /// exchange before returning.
    ///
    /// `requested_features` is this client's optional-feature bitmap; the
    /// session keeps the intersection with the server's, which is the only set
    /// whose messages may be sent.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] when the probe did not settle
    /// on [`ProbeStatus::ReadyV3`], or when the peer answers the exchange with
    /// anything but `FeaturesAck`.
    pub fn into_fs_session(
        mut self,
        requested_features: u64,
    ) -> Result<FsSession<R, W>, ServerError> {
        if !matches!(self.probe.status, ProbeStatus::ReadyV3) {
            return Err(ServerError::UnexpectedMessage(format!(
                "cannot open filesystem session after probe: {:?}",
                self.probe.status
            )));
        }
        self.writer.write_all(&protocol_v3::encode_frame(
            2,
            &V3Message::Features {
                features: requested_features,
            },
        )?)?;
        self.writer.flush()?;
        let frame =
            protocol_v3::read_frame(&mut self.reader)?.ok_or(ServerError::PeerDisconnected)?;
        let V3Message::FeaturesAck { features, .. } = frame.message else {
            return Err(ServerError::UnexpectedMessage(format!(
                "expected FeaturesAck, got {:?}",
                frame.message
            )));
        };
        if features & !requested_features != 0 {
            return Err(ServerError::UnexpectedMessage(format!(
                "server granted v3 features 0x{features:x} the client did not request",
            )));
        }
        Ok(FsSession {
            reader: self.reader,
            writer: self.writer,
            next_message_id: 3,
            remote_capabilities: self.probe.remote_capabilities,
            common_capabilities: self.probe.common_capabilities,
            negotiated_features: features,
        })
    }
}

/// Perform the v1-compatible opening handshake without starting a session operation.
///
/// # Errors
///
/// Returns [`ServerError::UnexpectedMessage`] if the peer's reply is not a
/// handshake acknowledgement, and propagates transport and codec failures from
/// the underlying stream.
pub fn probe_session<R: Read, W: Write>(
    reader: R,
    writer: W,
    job_id: [u8; 16],
) -> Result<ProbedConnection<R, W>, ServerError> {
    probe_with_capabilities(
        reader,
        writer,
        job_id,
        CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION | CAP_BROWSE_META,
    )
}

/// Perform the opening handshake advertising `CAP_FS_V3` as well.
///
/// Selection prefers v3 and falls back to v2 browse against an older peer, so
/// the returned status is [`ProbeStatus::ReadyV3`], [`ProbeStatus::Ready`], or
/// an older/unusable peer. This is a separate entry point from
/// [`probe_session`] on purpose: an existing browse consumer must not start
/// selecting a different grammar because the server was upgraded.
///
/// # Errors
///
/// As [`probe_session`].
pub fn probe_fs_session<R: Read, W: Write>(
    reader: R,
    writer: W,
    job_id: [u8; 16],
) -> Result<ProbedConnection<R, W>, ServerError> {
    probe_with_capabilities(
        reader,
        writer,
        job_id,
        CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION | CAP_BROWSE_META | CAP_FS_V3,
    )
}

fn probe_with_capabilities<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    job_id: [u8; 16],
    capabilities: u32,
) -> Result<ProbedConnection<R, W>, ServerError> {
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
    let status = match selected_version {
        3 => ProbeStatus::ReadyV3,
        2 => ProbeStatus::Ready,
        _ => ProbeStatus::OlderPeer { selected_version },
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] if the peer answers with a
    /// frame other than a cancel acknowledgement. Transport failures are
    /// propagated.
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RemoteError`] carrying the peer's status code and
    /// message when the remote refuses the operation, and
    /// [`ServerError::UnexpectedMessage`] if the peer answers with a frame other
    /// than the matching response. Transport failures are propagated.
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RemoteError`] carrying the peer's status code and
    /// message when the remote refuses the operation, and
    /// [`ServerError::UnexpectedMessage`] if the peer answers with a frame other
    /// than the matching response. Transport failures are propagated.
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

    /// Whether the peer advertised [`CAP_BROWSE_META`] (chmod/mtime/readlink).
    #[must_use]
    pub const fn supports_browse_meta(&self) -> bool {
        self.remote_capabilities & CAP_BROWSE_META != 0
    }

    /// Set Unix permission bits on a remote path (follows a final symlink).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] without sending when the peer
    /// does not advertise [`CAP_BROWSE_META`], so an older decoder is never
    /// sent type 36. Remote refusals are [`ServerError::RemoteError`].
    pub fn set_permissions(&mut self, path: Vec<u8>, mode: u32) -> Result<(), ServerError> {
        if !self.supports_browse_meta() {
            return Err(ServerError::UnexpectedMessage(
                "peer does not advertise CAP_BROWSE_META".to_owned(),
            ));
        }
        let response = self.request(&V2Message::SetPermissionsRequest { path, mode })?;
        match response.message {
            V2Message::SetPermissionsResponse {
                status: MutationStatus::Ok,
                ..
            } => Ok(()),
            V2Message::SetPermissionsResponse { status, error, .. } => {
                Err(ServerError::RemoteError {
                    code: status as u16,
                    message: String::from_utf8_lossy(&error).into_owned(),
                })
            }
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected SetPermissionsResponse, got {other:?}"
            ))),
        }
    }

    /// Set modification time on a remote path (follows a final symlink).
    ///
    /// `mtime_ns` is signed nanoseconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] without sending when the peer
    /// does not advertise [`CAP_BROWSE_META`]. Remote refusals are
    /// [`ServerError::RemoteError`].
    pub fn set_mtime(&mut self, path: Vec<u8>, mtime_ns: i64) -> Result<(), ServerError> {
        if !self.supports_browse_meta() {
            return Err(ServerError::UnexpectedMessage(
                "peer does not advertise CAP_BROWSE_META".to_owned(),
            ));
        }
        let response = self.request(&V2Message::SetMtimeRequest { path, mtime_ns })?;
        match response.message {
            V2Message::SetMtimeResponse {
                status: MutationStatus::Ok,
                ..
            } => Ok(()),
            V2Message::SetMtimeResponse { status, error, .. } => Err(ServerError::RemoteError {
                code: status as u16,
                message: String::from_utf8_lossy(&error).into_owned(),
            }),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected SetMtimeResponse, got {other:?}"
            ))),
        }
    }

    /// Read a remote symlink's target without following it.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] without sending when the peer
    /// does not advertise [`CAP_BROWSE_META`]. A missing path is
    /// [`ServerError::InvalidPath`]; a non-symlink or filesystem error is
    /// [`ServerError::RemoteError`].
    pub fn read_link(&mut self, path: Vec<u8>) -> Result<Vec<u8>, ServerError> {
        if !self.supports_browse_meta() {
            return Err(ServerError::UnexpectedMessage(
                "peer does not advertise CAP_BROWSE_META".to_owned(),
            ));
        }
        let missing = String::from_utf8_lossy(&path).into_owned();
        let response = self.request(&V2Message::ReadLinkRequest { path })?;
        match response.message {
            V2Message::ReadLinkResponse {
                status: StatStatus::Ok,
                target,
                ..
            } => Ok(target),
            V2Message::ReadLinkResponse {
                status: StatStatus::Missing,
                ..
            } => Err(ServerError::RemoteError {
                code: StatStatus::Missing as u16,
                message: format!("not found: {missing}"),
            }),
            V2Message::ReadLinkResponse {
                status: StatStatus::Error,
                error,
                ..
            } => Err(ServerError::RemoteError {
                code: StatStatus::Error as u16,
                message: String::from_utf8_lossy(&error).into_owned(),
            }),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected ReadLinkResponse, got {other:?}"
            ))),
        }
    }

    /// Delete a remote tree, calling `progress` once for every attempted item.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RemoteError`] carrying the peer's status code and
    /// message when the remote refuses the operation, and
    /// [`ServerError::UnexpectedMessage`] if the peer answers with a frame other
    /// than the matching response. Transport failures are propagated.
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RemoteError`] when the remote refuses the read,
    /// [`ServerError::UnexpectedMessage`] on an out-of-protocol reply, and an
    /// I/O error if the local temporary file cannot be written or published. A
    /// digest mismatch fails the fetch and leaves the destination untouched.
    pub fn fetch(
        &mut self,
        remote_path: Vec<u8>,
        local_path: impl AsRef<Path>,
    ) -> Result<FetchedFile, ServerError> {
        let related_id = self.send(&V2Message::FetchRequest { path: remote_path })?;
        let start = {
            let response = self.receive()?;
            match response.message {
                V2Message::FetchStart { related_id: id, .. } if id == related_id => {
                    response.message
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RemoteError`] when the remote rejects the write —
    /// including the identity check failing, which means the file changed since
    /// it was fetched and the edit would silently overwrite newer content — and
    /// [`ServerError::UnexpectedMessage`] on an out-of-protocol reply.
    pub fn publish(
        &mut self,
        remote_path: Vec<u8>,
        local_path: impl AsRef<Path>,
        fetched: FetchedFile,
    ) -> Result<V2Message, ServerError> {
        let bytes = fs::read(local_path).map_err(ServerError::Io)?;
        let content_size = bytes.len() as u64;
        let content_digest = *blake3::hash(&bytes).as_bytes();
        let related_id = self.send(&V2Message::PublishRequest {
            path: remote_path,
            size: fetched.size,
            mtime_ns: fetched.mtime_ns,
            device: fetched.identity.device,
            file: fetched.identity.file,
            content_size,
            digest: content_digest,
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

/// Client-side driver for a persistent v3 filesystem session.
///
/// The session is committed to v3 before this type exists and never retries a
/// frame as v2 or v1. Requests are issued one at a time; concurrent dispatch
/// with out-of-order responses is `xsyncv3.md` E3-S1.
pub struct FsSession<R, W> {
    reader: R,
    writer: W,
    next_message_id: u64,
    remote_capabilities: u32,
    common_capabilities: u32,
    negotiated_features: u64,
}

impl<R: Read, W: Write> FsSession<R, W> {
    /// Open a filesystem session over an already-connected stream pair.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the peer does not negotiate v3 or the
    /// opening handshake or feature exchange is malformed.
    pub fn connect(
        reader: R,
        writer: W,
        job_id: [u8; 16],
        requested_features: u64,
    ) -> Result<Self, ServerError> {
        probe_fs_session(reader, writer, job_id)?.into_fs_session(requested_features)
    }

    /// Capabilities advertised by the remote peer.
    #[must_use]
    pub const fn remote_capabilities(&self) -> u32 {
        self.remote_capabilities
    }

    /// Known capabilities shared by both endpoints.
    #[must_use]
    pub const fn common_capabilities(&self) -> u32 {
        self.common_capabilities
    }

    /// Optional v3 features both endpoints offer.
    #[must_use]
    pub const fn negotiated_features(&self) -> u64 {
        self.negotiated_features
    }

    /// Whether every bit in `features` was negotiated.
    #[must_use]
    pub const fn supports(&self, features: u64) -> bool {
        self.negotiated_features & features == features
    }

    /// Refuse to send a message whose feature the server did not advertise.
    ///
    /// A fail-closed peer aborts the session on a message type it does not
    /// know, so sending an ungated request would cost the connection. This
    /// turns that into a local error naming the missing feature.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] when a bit is missing.
    pub fn require(&self, features: u64, name: &str) -> Result<(), ServerError> {
        if self.supports(features) {
            return Ok(());
        }
        Err(ServerError::UnexpectedMessage(format!(
            "remote peer did not negotiate the v3 {name} feature (0x{features:x}); \
             negotiated set is 0x{:x}",
            self.negotiated_features
        )))
    }

    /// Recover the underlying streams.
    pub fn into_parts(self) -> (R, W) {
        (self.reader, self.writer)
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        id
    }

    /// Send one request and return its message ID.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on encode or transport failure.
    pub fn send(&mut self, message: &V3Message) -> Result<u64, ServerError> {
        let id = self.next_id();
        self.writer
            .write_all(&protocol_v3::encode_frame(id, message)?)?;
        self.writer.flush()?;
        Ok(id)
    }

    /// Read the next frame from the peer.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::PeerDisconnected`] at clean EOF, and propagates
    /// codec and transport failures.
    pub fn receive(&mut self) -> Result<V3Frame, ServerError> {
        protocol_v3::read_frame(&mut self.reader)?.ok_or(ServerError::PeerDisconnected)
    }

    /// Send one request and read its reply.
    ///
    /// # Errors
    ///
    /// As [`FsSession::send`] and [`FsSession::receive`].
    pub fn request(&mut self, message: &V3Message) -> Result<V3Frame, ServerError> {
        self.send(message)?;
        self.receive()
    }

    /// Send a keepalive and check that the peer echoes the nonce.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::UnexpectedMessage`] when the reply is not the
    /// matching acknowledgement.
    pub fn keepalive(&mut self, nonce: u64) -> Result<(), ServerError> {
        let frame = self.request(&V3Message::Keepalive { nonce })?;
        match frame.message {
            V3Message::KeepaliveAck { nonce: echoed } if echoed == nonce => Ok(()),
            other => Err(ServerError::UnexpectedMessage(format!(
                "expected KeepaliveAck({nonce}), got {other:?}"
            ))),
        }
    }

    /// Abandon one in-flight request.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on encode or transport failure.
    pub fn cancel(&mut self, related_id: u64) -> Result<(), ServerError> {
        self.send(&V3Message::Cancel { related_id })?;
        Ok(())
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
            unix: None,
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
///
/// # Errors
///
/// Returns [`ServerError::DuplicatePath`] when `path` was already seen in this
/// transfer, and propagates [`ServerError::InvalidPath`] for a path that
/// escapes `root` or is otherwise unrepresentable.
pub fn validate_unique_destination_path<S: std::hash::BuildHasher>(
    root: &Path,
    path: &WirePath,
    seen: &mut HashSet<WirePath, S>,
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
        path.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    path.to_string_lossy().as_bytes().to_vec()
}

/// The inverse of [`native_path_bytes`], for turning a name captured from a
/// directory listing back into something that can be joined onto a path.
fn native_path_os_string(bytes: &[u8]) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(bytes.to_vec())
    }
    #[cfg(not(unix))]
    std::ffi::OsString::from(String::from_utf8_lossy(bytes).into_owned())
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

#[derive(Clone, Copy)]
enum MutationKind {
    Rename,
    CreateDirectory,
    SetPermissions,
    SetMtime,
}

fn mutation_response(
    related_id: u64,
    result: Result<(), (MutationStatus, String)>,
    kind: MutationKind,
) -> V2Message {
    let (status, error) = match result {
        Ok(()) => (MutationStatus::Ok, Vec::new()),
        Err((status, error)) => (status, error.into_bytes()),
    };
    match kind {
        MutationKind::Rename => V2Message::RenameResponse {
            related_id,
            status,
            error,
        },
        MutationKind::CreateDirectory => V2Message::CreateDirectoryResponse {
            related_id,
            status,
            error,
        },
        MutationKind::SetPermissions => V2Message::SetPermissionsResponse {
            related_id,
            status,
            error,
        },
        MutationKind::SetMtime => V2Message::SetMtimeResponse {
            related_id,
            status,
            error,
        },
    }
}

fn meta_status(error: &io::Error) -> MutationStatus {
    match error.kind() {
        io::ErrorKind::PermissionDenied => MutationStatus::PermissionDenied,
        io::ErrorKind::NotFound => MutationStatus::ParentMissing,
        _ => MutationStatus::Error,
    }
}

#[cfg(unix)]
fn set_path_permissions(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_path_permissions(path: &Path, mode: u32) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, permissions)
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
/// Shared state for one v3 filesystem session.
///
/// Every worker thread holds an `Arc` of this, so anything mutable lives behind
/// its own lock. The handle table lands with E4-S1; today it carries only the
/// export root and the set of cancelled requests.
struct FsSessionState {
    /// Export root every path in the session resolves below.
    root: PathBuf,
    /// The operator asked for a read-only export (`xs --server --read-only`).
    read_only: bool,
    cancelled: Mutex<HashSet<u64>>,
    /// Open handles, shared by every worker.
    ///
    /// Handles are `Arc`ed so a worker clones one out and releases the lock
    /// before doing any I/O: a slow read must not block an unrelated `Open`.
    handles: RwLock<HashMap<u64, Arc<OpenHandle>>>,
    /// Never reused within a session, and never `0`.
    next_handle: AtomicU64,
    /// Reserved handle slots: incremented *before* an open starts and released
    /// if it fails. Reading the table's length instead would let every worker
    /// see room at once and overshoot the cap by the width of the pool.
    reserved_handles: AtomicU64,
    max_handles: u64,
}

/// A reserved handle slot, released on drop unless the open committed.
struct HandleSlot<'a> {
    state: &'a FsSessionState,
    committed: bool,
}

impl Drop for HandleSlot<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.state.reserved_handles.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl FsSessionState {
    /// Reserve one slot, or `None` when the session is at its limit.
    fn reserve_handle(&self) -> Option<HandleSlot<'_>> {
        if self.reserved_handles.fetch_add(1, Ordering::AcqRel) >= self.max_handles {
            self.reserved_handles.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(HandleSlot {
            state: self,
            committed: false,
        })
    }

    fn release_handle(&self) {
        self.reserved_handles.fetch_sub(1, Ordering::AcqRel);
    }

    /// Clone one handle out of the table, releasing the lock immediately: the
    /// I/O that follows must not hold up an unrelated `Open` or `Close`.
    fn handle(&self, id: u64) -> Option<Arc<OpenHandle>> {
        self.handles.read().ok()?.get(&id).cloned()
    }
}

/// One open file or directory.
#[derive(Debug)]
struct OpenHandle {
    /// Resolved native path, already confined under the export root.
    path: PathBuf,
    /// `None` for a directory handle. Positional reads and writes use
    /// `FileExt`, which takes `&self`, so several workers can use one open
    /// file at once without a lock and without a shared seek offset.
    file: Option<fs::File>,
    /// The flags it was opened with. Only recoverable here: `Read` refuses a
    /// handle opened write-only, and `Write` needs to know an `APPEND` handle
    /// ignores its offset (E4-S3).
    flags: u32,
    /// Names captured when a directory listing started, so paging is a slice
    /// rather than a re-read. `None` until the first page asks for one.
    ///
    /// `Arc` so a page clones the list out and releases the lock before it
    /// starts stat-ing entries.
    listing: Mutex<Option<Arc<Vec<Vec<u8>>>>>,
}

impl FsSessionState {
    fn cancel(&self, related_id: u64) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.insert(related_id);
        }
    }

    /// Whether this request was cancelled. A handler doing long work should
    /// poll this and stop early; a short one may ignore it.
    fn is_cancelled(&self, related_id: u64) -> bool {
        self.cancelled
            .lock()
            .is_ok_and(|cancelled| cancelled.contains(&related_id))
    }
}

/// Executes one v3 filesystem request.
///
/// Called from several worker threads at once, so an implementation must be
/// internally synchronised. The real handlers arrive with E1-S4 (`Mount`),
/// E4 (`Open`/`Read`/`Write`/`Flush`/`Close`) and E5 (`Stat`/`ReadDir`/`StatFs`);
/// this seam exists so they are written against a dispatcher that is already
/// concurrent rather than retrofitted onto a serial one.
trait FsHandler: Send + Sync {
    fn handle(&self, state: &FsSessionState, related_id: u64, request: V3Message) -> V3Message;
}

/// The server's pooled filesystem handler.
///
/// `Mount` is not here: it is session setup, answered on the session thread
/// (like the `Features` exchange) because every other verb's admissibility
/// depends on its answer. The rest answer `EOPNOTSUPP` until E4 and E5 land.
struct ServerFsHandler;

impl FsHandler for ServerFsHandler {
    fn handle(&self, state: &FsSessionState, related_id: u64, request: V3Message) -> V3Message {
        match request {
            V3Message::Open {
                path,
                flags,
                mode,
                attr_mask,
            } => open_handle(state, related_id, &path, flags, mode, attr_mask),
            V3Message::Close { handle } => close_handle(state, related_id, handle),
            V3Message::Read {
                handle,
                offset,
                length,
                want_digest,
            } => read_handle(state, related_id, handle, offset, length, want_digest),
            V3Message::Write {
                handle,
                offset,
                digest,
                data,
            } => write_handle(state, related_id, handle, offset, digest, &data),
            V3Message::Flush { handle } => flush_handle(state, related_id, handle),
            V3Message::Stat {
                target,
                follow,
                attr_mask,
            } => stat_target(state, related_id, &target, follow, attr_mask),
            V3Message::ReadDir {
                handle,
                cursor,
                max_entries,
                attr_mask,
            } => read_dir_handle(state, related_id, handle, cursor, max_entries, attr_mask),
            other => V3Message::Error {
                related_id,
                code: FsErrorCode::NotSupported,
                platform_errno: 0,
                message: format!(
                    "v3 message type {} is negotiated but not implemented by this build",
                    protocol_v3::message_type(&other)
                )
                .into_bytes(),
            },
        }
    }
}

/// Why this session may not write, or `None` when it may.
///
/// Answered by creating and removing a uniquely-named probe file. That is a
/// side effect, but it is the only portable way to learn whether *this* user
/// may write on *this* mount, and it answers in one question what a read-only
/// mount flag, a denying mode and a denying ACL would each answer separately.
/// `PathSemantics::probe` already writes here for the same reason and with the
/// same unique-prefix discipline, so this adds no new class of side effect.
///
/// It is deliberately not a per-path evaluation; that is `Access` (E5-S7).
fn write_barrier(state: &FsSessionState) -> Option<&'static str> {
    if state.read_only {
        return Some("export is read-only");
    }
    match tempfile::Builder::new()
        .prefix(".xsync-write-probe-")
        .tempfile_in(&state.root)
    {
        Ok(_probe) => None,
        Err(error) if is_read_only_filesystem(&error) => Some("filesystem is mounted read-only"),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Some("no write permission on the export root")
        }
        Err(_) => Some("the export root does not accept writes"),
    }
}

#[cfg(unix)]
fn is_read_only_filesystem(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::EROFS)
}

#[cfg(not(unix))]
const fn is_read_only_filesystem(_error: &std::io::Error) -> bool {
    false
}

/// Conventional name and path limits for the export root.
///
/// These are the values every filesystem xsync serves today actually uses.
/// Reading the real `pathconf` limits needs an FFI call, and the workspace
/// denies `unsafe_code` for one documented exception; a per-filesystem limit is
/// not worth becoming the second. A client uses these to validate input before
/// a round trip, and the server still rejects an over-long name on its merits.
const fn name_and_path_limits() -> (u32, u32) {
    if cfg!(windows) {
        (255, 260)
    } else {
        (255, 4096)
    }
}

/// Facts about the export, computed once per `Mount`.
fn mount_info(
    state: &FsSessionState,
    related_id: u64,
    export: &[u8],
    requested_access: protocol_v3::Access,
) -> V3Message {
    // Phase 1 serves exactly the one root `xs --server` was given; named
    // exports arrive with the daemon (E1-S2), so anything but the empty name
    // is a client error rather than a missing export.
    if !export.is_empty() {
        return fs_error(
            related_id,
            FsErrorCode::NoEntry,
            "this server has one unnamed export; send an empty export name",
        );
    }
    match std::fs::metadata(&state.root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return fs_error(
                related_id,
                FsErrorCode::NotDirectory,
                "export root is not a directory",
            )
        }
        Err(error) => {
            return V3Message::Error {
                related_id,
                code: FsErrorCode::NoEntry,
                platform_errno: error.raw_os_error().unwrap_or(0),
                message: b"export root does not exist".to_vec(),
            }
        }
    }

    let barrier = write_barrier(state);
    // The client's request can only narrow what the export allows.
    let read_only =
        state.read_only || requested_access == protocol_v3::Access::ReadOnly || barrier.is_some();
    let access = if state.read_only {
        protocol_v3::Access::ReadOnly
    } else {
        protocol_v3::Access::ReadWrite
    };
    let effective_writable = !read_only;
    let reason = if effective_writable {
        Vec::new()
    } else if let Some(barrier) = barrier {
        barrier.as_bytes().to_vec()
    } else {
        // Nothing stops a write; the client simply asked for a read-only mount.
        b"mounted read-only at the client's request".to_vec()
    };

    let semantics = crate::pathsem::PathSemantics::probe(&state.root);
    let mut supports = 0;
    if cfg!(unix) {
        supports |= protocol_v3::supports::SYMLINKS;
    }
    if semantics.case_insensitive {
        supports |= protocol_v3::supports::CASE_INSENSITIVE;
    }
    if semantics.normalization_insensitive {
        supports |= protocol_v3::supports::NORMALIZATION_INSENSITIVE;
    }
    let (max_name_len, max_path_len) = name_and_path_limits();

    V3Message::MountInfo {
        related_id,
        export: Vec::new(),
        access,
        effective_writable,
        reason,
        // There is no operator-supplied option string without an exports file
        // (E1-S2); a client shows `reason` instead of inventing one here.
        options: Vec::new(),
        case_sensitive: !semantics.case_insensitive,
        // What the filesystem *applies*. Neither APFS nor ext4 rewrites the
        // name it is given, and the probe cannot observe a filesystem that
        // does, so claiming NFC or NFD here would be a guess. Whether two
        // forms collide is reported through `supports` instead.
        normalization: protocol_v3::Normalization::None,
        max_name_len,
        max_path_len,
        supports,
        max_read: DEFAULT_FS_MAX_TRANSFER,
        max_write: DEFAULT_FS_MAX_TRANSFER,
        // Cache lifetime hints need leases to be meaningful (E8-S1).
        attr_cache_ms: 0,
        dir_cache_ms: 0,
    }
}

/// Translate a filesystem failure into the frozen v3 code.
///
/// The errno is preserved alongside it, so a client can distinguish two
/// failures that share a code without this table having to grow.
#[cfg(unix)]
fn fs_code_for(error: &io::Error) -> FsErrorCode {
    match error.raw_os_error() {
        Some(libc::ENOENT) => FsErrorCode::NoEntry,
        Some(libc::EACCES | libc::EPERM) => FsErrorCode::Access,
        Some(libc::EEXIST) => FsErrorCode::Exists,
        Some(libc::EISDIR) => FsErrorCode::IsDirectory,
        Some(libc::ENOTDIR) => FsErrorCode::NotDirectory,
        Some(libc::ELOOP) => FsErrorCode::Loop,
        Some(libc::EROFS) => FsErrorCode::ReadOnly,
        Some(libc::ENOSPC) => FsErrorCode::NoSpace,
        Some(libc::EDQUOT) => FsErrorCode::Quota,
        Some(libc::ENAMETOOLONG) => FsErrorCode::NameTooLong,
        Some(libc::ENOTEMPTY) => FsErrorCode::NotEmpty,
        // Out of descriptors is this server's limit, not the caller's mistake.
        Some(libc::EMFILE | libc::ENFILE) => FsErrorCode::Limit,
        _ => FsErrorCode::Io,
    }
}

#[cfg(not(unix))]
fn fs_code_for(error: &io::Error) -> FsErrorCode {
    match error.kind() {
        io::ErrorKind::NotFound => FsErrorCode::NoEntry,
        io::ErrorKind::PermissionDenied => FsErrorCode::Access,
        io::ErrorKind::AlreadyExists => FsErrorCode::Exists,
        _ => FsErrorCode::Io,
    }
}

fn fs_io_error(related_id: u64, error: &io::Error, context: &str) -> V3Message {
    V3Message::Error {
        related_id,
        code: fs_code_for(error),
        platform_errno: error.raw_os_error().unwrap_or(0),
        message: format!("{context}: {error}").into_bytes(),
    }
}

/// A path that left the export, or was never representable, never reaches the
/// filesystem.
fn fs_path_error(related_id: u64, error: &ServerError) -> V3Message {
    let code = match error {
        ServerError::SymlinkEscape(_) => FsErrorCode::Access,
        _ => FsErrorCode::Invalid,
    };
    fs_error(related_id, code, &error.to_string())
}

/// An opaque 16-byte value that changes whenever the file does.
///
/// Derived from identity, length and both timestamps, so an edit that
/// preserves length still changes it. It is a digest rather than a packed
/// tuple because the contract says opaque, and a client that starts parsing
/// one would be relying on something this may change.
fn change_cookie(metadata: &fs::Metadata) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&metadata.len().to_le_bytes());
    hasher.update(
        &metadata
            .modified()
            .map(system_time_to_nanos)
            .unwrap_or_default()
            .to_le_bytes(),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(&metadata.dev().to_le_bytes());
        hasher.update(&metadata.ino().to_le_bytes());
        hasher.update(&metadata.ctime().to_le_bytes());
        hasher.update(&metadata.ctime_nsec().to_le_bytes());
    }
    let mut cookie = [0_u8; 16];
    cookie.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    cookie
}

/// Build an `Attrs` record, filling the optional blocks the mask asked for.
///
/// A block the mask did not ask for, or that this platform cannot answer, is
/// simply absent: the contract lets the response be a strict subset of the
/// mask, and a client must render with any of them missing.
fn attrs_from_metadata(metadata: &fs::Metadata, path: &Path, attr_mask: u32) -> protocol_v3::Attrs {
    use protocol_v3::attr_presence as presence;

    let file_type = metadata.file_type();
    let kind = if file_type.is_file() {
        1
    } else if file_type.is_dir() {
        2
    } else if file_type.is_symlink() {
        3
    } else {
        4
    };
    let mut attrs = protocol_v3::Attrs::minimal(
        kind,
        permission_mode(metadata),
        metadata.len(),
        metadata
            .modified()
            .map(system_time_to_nanos)
            .unwrap_or_default(),
        change_cookie(metadata),
    );
    let wants = |bit: u32| attr_mask & bit != 0;
    if wants(presence::ATIME) {
        attrs.atime_ns = metadata.accessed().ok().map(system_time_to_nanos);
    }
    if wants(presence::BTIME) {
        attrs.btime_ns = metadata.created().ok().map(system_time_to_nanos);
    }
    if wants(presence::SYMLINK_TARGET) && kind == 3 {
        attrs.symlink_target = fs::read_link(path)
            .ok()
            .map(|target| native_path_bytes(target.as_os_str()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if wants(presence::OWNER) {
            attrs.owner = Some((metadata.uid(), metadata.gid()));
        }
        if wants(presence::NLINK) {
            attrs.nlink = u32::try_from(metadata.nlink()).ok();
        }
        if wants(presence::CTIME) {
            attrs.ctime_ns = Some(
                metadata
                    .ctime()
                    .saturating_mul(1_000_000_000)
                    .saturating_add(metadata.ctime_nsec()),
            );
        }
        if wants(presence::IDENTITY) {
            attrs.identity = Some((metadata.dev(), metadata.ino()));
        }
        if wants(presence::RDEV) && kind == 4 {
            attrs.rdev = Some(metadata.rdev());
        }
        if wants(presence::ALLOCATED_SIZE) {
            attrs.allocated_size = Some(metadata.blocks().saturating_mul(512));
        }
    }
    // NAMES is never filled: resolving a uid to a name needs `getpwuid`, and
    // the server does not advertise the OWNER_NAMES feature that would let a
    // client ask for it.
    attrs
}

#[cfg(unix)]
fn apply_unix_open_flags(options: &mut fs::OpenOptions, flags: u32, mode: u32) {
    use protocol_v3::open_flags;
    use std::os::unix::fs::OpenOptionsExt;

    if flags & open_flags::CREATE != 0 {
        options.mode(mode & protocol_v3::MAX_MODE);
    }
    if flags & open_flags::NOFOLLOW != 0 {
        options.custom_flags(libc::O_NOFOLLOW);
    }
}

#[cfg(not(unix))]
fn apply_unix_open_flags(_options: &mut fs::OpenOptions, _flags: u32, _mode: u32) {}

/// Open a file or directory and put it in the session's handle table.
fn open_handle(
    state: &FsSessionState,
    related_id: u64,
    path: &[u8],
    flags: u32,
    mode: u32,
    attr_mask: u32,
) -> V3Message {
    use protocol_v3::open_flags;

    let native = match browse_stat_path(&state.root, path) {
        Ok(native) => native,
        Err(error) => return fs_path_error(related_id, &error),
    };

    // Reserved before the open, so a session cannot exhaust the process's
    // descriptors. The slot is released on every failure path below by the
    // guard's `Drop`.
    let Some(mut slot) = state.reserve_handle() else {
        return fs_error(
            related_id,
            FsErrorCode::Limit,
            "too many open handles on this session",
        );
    };

    let directory = flags & open_flags::DIRECTORY != 0;
    // `symlink_metadata` rather than `metadata`: with NOFOLLOW the answer must
    // describe the link itself, and without it the open below follows anyway.
    let existing = fs::symlink_metadata(&native);
    if let Ok(metadata) = &existing {
        if metadata.file_type().is_symlink() && flags & open_flags::NOFOLLOW != 0 {
            return fs_error(
                related_id,
                FsErrorCode::Loop,
                "path is a symbolic link and NOFOLLOW was requested",
            );
        }
    }

    let handle = if directory {
        let metadata = match fs::metadata(&native) {
            Ok(metadata) => metadata,
            Err(error) => return fs_io_error(related_id, &error, "open directory"),
        };
        if !metadata.is_dir() {
            return fs_error(
                related_id,
                FsErrorCode::NotDirectory,
                "DIRECTORY was requested but the path is not a directory",
            );
        }
        OpenHandle {
            path: native,
            file: None,
            flags,
            listing: Mutex::new(None),
        }
    } else {
        // A directory opened without DIRECTORY would produce a handle no read
        // or write could use, so refuse it here rather than at first use.
        if existing.is_ok_and(|metadata| metadata.is_dir()) {
            return fs_error(
                related_id,
                FsErrorCode::IsDirectory,
                "path is a directory; open it with the DIRECTORY flag",
            );
        }
        let mut options = fs::OpenOptions::new();
        options
            .read(flags & open_flags::READ != 0)
            .write(flags & open_flags::WRITE != 0 && flags & open_flags::APPEND == 0)
            .append(flags & open_flags::APPEND != 0)
            .truncate(flags & open_flags::TRUNC != 0);
        if flags & open_flags::EXCL != 0 {
            options.create_new(true);
        } else if flags & open_flags::CREATE != 0 {
            options.create(true);
        }
        apply_unix_open_flags(&mut options, flags, mode);
        match options.open(&native) {
            Ok(file) => OpenHandle {
                path: native,
                file: Some(file),
                flags,
                listing: Mutex::new(None),
            },
            Err(error) => return fs_io_error(related_id, &error, "open"),
        }
    };

    let metadata = match handle.file.as_ref() {
        Some(file) => file.metadata(),
        None => fs::metadata(&handle.path),
    };
    let metadata = match metadata {
        Ok(metadata) => metadata,
        Err(error) => return fs_io_error(related_id, &error, "stat after open"),
    };
    let attrs = attrs_from_metadata(&metadata, &handle.path, attr_mask);

    let id = state.next_handle.fetch_add(1, Ordering::Relaxed);
    match state.handles.write() {
        Ok(mut handles) => {
            handles.insert(id, Arc::new(handle));
            slot.committed = true;
        }
        Err(_) => return fs_error(related_id, FsErrorCode::Io, "handle table is poisoned"),
    }
    V3Message::Opened {
        related_id,
        handle: id,
        attrs,
    }
}

/// Positional read at `offset`, which is the offset in the file rather than a
/// cursor: the platform primitives take `&self`, so several reads on one
/// handle can be in flight at once with no shared seek position to race over.
#[cfg(unix)]
fn read_at(file: &fs::File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buffer, offset)
}

#[cfg(windows)]
fn read_at(file: &fs::File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, offset)
}

/// Read one byte range of an open file.
fn read_handle(
    state: &FsSessionState,
    related_id: u64,
    handle: u64,
    offset: u64,
    length: u32,
    want_digest: bool,
) -> V3Message {
    use protocol_v3::open_flags;

    let Some(open) = state.handle(handle) else {
        return fs_error(related_id, FsErrorCode::BadHandle, "no such handle");
    };
    let Some(file) = open.file.as_ref() else {
        return fs_error(
            related_id,
            FsErrorCode::IsDirectory,
            "handle is a directory; use ReadDir",
        );
    };
    if open.flags & open_flags::READ == 0 {
        return fs_error(
            related_id,
            FsErrorCode::Access,
            "handle was not opened for reading",
        );
    }
    // The bound the mount advertised, not the envelope's: a client that
    // ignores `max_read` gets a clear refusal rather than a larger allocation
    // than the server offered.
    if length > DEFAULT_FS_MAX_TRANSFER {
        return fs_error(
            related_id,
            FsErrorCode::Invalid,
            "read length exceeds the mount's max_read",
        );
    }

    let mut data = vec![0_u8; length as usize];
    let mut filled = 0_usize;
    // `read_at` may return a short count for reasons other than end of file,
    // so a short *response* is only honest once a read has actually returned
    // zero. Looping here is what makes `eof` mean end of file.
    while filled < data.len() {
        if state.is_cancelled(related_id) {
            return fs_error(related_id, FsErrorCode::Cancelled, "read cancelled");
        }
        match read_at(
            file,
            &mut data[filled..],
            offset.saturating_add(filled as u64),
        ) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return fs_io_error(related_id, &error, "read"),
        }
    }
    let eof = filled < data.len();
    data.truncate(filled);
    let digest = want_digest.then(|| *blake3::hash(&data).as_bytes());
    V3Message::ReadData {
        related_id,
        offset,
        eof,
        digest,
        data,
    }
}

#[cfg(unix)]
fn write_at(file: &fs::File, buffer: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.write_at(buffer, offset)
}

#[cfg(windows)]
fn write_at(file: &fs::File, buffer: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buffer, offset)
}

/// Write one byte range of an open file.
fn write_handle(
    state: &FsSessionState,
    related_id: u64,
    handle: u64,
    offset: u64,
    digest: Option<[u8; 32]>,
    data: &[u8],
) -> V3Message {
    use protocol_v3::open_flags;

    let Some(open) = state.handle(handle) else {
        return fs_error(related_id, FsErrorCode::BadHandle, "no such handle");
    };
    let Some(file) = open.file.as_ref() else {
        return fs_error(
            related_id,
            FsErrorCode::IsDirectory,
            "handle is a directory and cannot be written",
        );
    };
    if open.flags & open_flags::WRITE == 0 {
        return fs_error(
            related_id,
            FsErrorCode::Access,
            "handle was not opened for writing",
        );
    }
    if data.len() > DEFAULT_FS_MAX_TRANSFER as usize {
        return fs_error(
            related_id,
            FsErrorCode::Invalid,
            "write length exceeds the mount's max_write",
        );
    }
    // Verified before the first byte reaches the file: a corrupt payload must
    // leave the destination exactly as it was, not half-written.
    if let Some(expected) = digest {
        if *blake3::hash(data).as_bytes() != expected {
            return fs_error(
                related_id,
                FsErrorCode::Integrity,
                "write payload does not match its digest; nothing was written",
            );
        }
    }

    // An APPEND handle writes at the end no matter what offset says, so it
    // cannot use the positional call: `pwrite` against `O_APPEND` differs
    // between Linux and the BSDs, and `Write for &File` appends on both.
    let append = open.flags & open_flags::APPEND != 0;
    let mut written = 0_usize;
    while written < data.len() {
        let result = if append {
            io::Write::write(&mut &*file, &data[written..])
        } else {
            write_at(
                file,
                &data[written..],
                offset.saturating_add(written as u64),
            )
        };
        match result {
            Ok(0) => {
                return fs_error(
                    related_id,
                    FsErrorCode::Io,
                    "write made no progress before the range was complete",
                )
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            // A partial write has already reached the file. Reporting the
            // error rather than a short WriteAck is the honest answer: the
            // client must re-read the range to learn what landed.
            Err(error) => return fs_io_error(related_id, &error, "write"),
        }
    }

    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => return fs_io_error(related_id, &error, "stat after write"),
    };
    V3Message::WriteAck {
        related_id,
        bytes_written: u32::try_from(written).unwrap_or(u32::MAX),
        new_size: metadata.len(),
        // Write-through to the page cache; `Flush` is the durability barrier.
        // A `sync` export would make this true, and arrives with the exports
        // file (E1-S2).
        stable: false,
        change_cookie: change_cookie(&metadata),
    }
}

/// Make a handle's writes durable.
fn flush_handle(state: &FsSessionState, related_id: u64, handle: u64) -> V3Message {
    let Some(open) = state.handle(handle) else {
        return fs_error(related_id, FsErrorCode::BadHandle, "no such handle");
    };
    let Some(file) = open.file.as_ref() else {
        return fs_error(
            related_id,
            FsErrorCode::IsDirectory,
            "handle is a directory and buffers nothing",
        );
    };
    match file.sync_all() {
        Ok(()) => V3Message::Done { related_id },
        Err(error) => fs_io_error(related_id, &error, "flush"),
    }
}

/// Attributes of a path or an open handle.
fn stat_target(
    state: &FsSessionState,
    related_id: u64,
    target: &StatTarget,
    follow: bool,
    attr_mask: u32,
) -> V3Message {
    let (path, metadata) = match target {
        StatTarget::Path(path) => {
            let native = match browse_stat_path(&state.root, path) {
                Ok(native) => native,
                Err(error) => return fs_path_error(related_id, &error),
            };
            // `follow` is the whole difference between stat and lstat, and a
            // client listing a tree wants lstat so a link reads as a link.
            let metadata = if follow {
                fs::metadata(&native)
            } else {
                fs::symlink_metadata(&native)
            };
            match metadata {
                Ok(metadata) => (native, metadata),
                Err(error) => return fs_io_error(related_id, &error, "stat"),
            }
        }
        StatTarget::Handle(handle) => {
            let Some(open) = state.handle(*handle) else {
                return fs_error(related_id, FsErrorCode::BadHandle, "no such handle");
            };
            // A file handle answers from the descriptor, so it describes the
            // file this session opened even if the name has since been reused.
            // A directory handle has no descriptor to ask, so it restats the
            // path.
            let metadata = match open.file.as_ref() {
                Some(file) => file.metadata(),
                None => fs::metadata(&open.path),
            };
            match metadata {
                Ok(metadata) => (open.path.clone(), metadata),
                Err(error) => return fs_io_error(related_id, &error, "stat handle"),
            }
        }
    };
    V3Message::AttrsResponse {
        related_id,
        attrs: attrs_from_metadata(&metadata, &path, attr_mask),
    }
}

/// One page of a directory handle's listing.
fn read_dir_handle(
    state: &FsSessionState,
    related_id: u64,
    handle: u64,
    cursor: u64,
    max_entries: u32,
    attr_mask: u32,
) -> V3Message {
    let Some(open) = state.handle(handle) else {
        return fs_error(related_id, FsErrorCode::BadHandle, "no such handle");
    };
    if open.file.is_some() {
        return fs_error(
            related_id,
            FsErrorCode::NotDirectory,
            "handle is a file; open it with DIRECTORY to list it",
        );
    }

    // The names are captured once, at cursor zero, and every later page is a
    // slice of that. Re-reading the directory per page and skipping forward is
    // what makes paging quadratic in the number of entries.
    let names = {
        let Ok(mut listing) = open.listing.lock() else {
            return fs_error(related_id, FsErrorCode::Io, "listing state is poisoned");
        };
        if cursor == 0 || listing.is_none() {
            let directory = match fs::read_dir(&open.path) {
                Ok(directory) => directory,
                Err(error) => return fs_io_error(related_id, &error, "read directory"),
            };
            let mut names = Vec::new();
            for entry in directory {
                match entry {
                    // `read_dir` never yields "." or "..".
                    Ok(entry) => names.push(native_path_bytes(&entry.file_name())),
                    Err(error) => return fs_io_error(related_id, &error, "read directory"),
                }
            }
            *listing = Some(Arc::new(names));
        }
        Arc::clone(listing.as_ref().expect("just populated"))
    };

    let Ok(start) = usize::try_from(cursor) else {
        return fs_error(related_id, FsErrorCode::Invalid, "cursor is out of range");
    };
    if start > names.len() {
        return fs_error(
            related_id,
            FsErrorCode::Invalid,
            "cursor is past the end of this listing",
        );
    }
    let end = start.saturating_add(max_entries as usize).min(names.len());

    let mut entries = Vec::with_capacity(end - start);
    for name in &names[start..end] {
        let path = open.path.join(native_path_os_string(name));
        // A name from the snapshot that has since been removed is simply left
        // out: the contract allows an entry that changed mid-listing to appear
        // or not, and refusing the whole page would be worse.
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            entries.push(protocol_v3::DirEntry {
                name: name.clone(),
                attrs: attrs_from_metadata(&metadata, &path, attr_mask),
            });
        }
    }

    V3Message::DirPage {
        related_id,
        cursor: end as u64,
        final_page: end >= names.len(),
        entries,
    }
}

/// Drop a handle. Closing one that is not open is this request's error, never
/// the session's.
fn close_handle(state: &FsSessionState, related_id: u64, handle: u64) -> V3Message {
    match state.handles.write() {
        Ok(mut handles) => {
            if handles.remove(&handle).is_some() {
                state.release_handle();
                V3Message::Done { related_id }
            } else {
                fs_error(related_id, FsErrorCode::BadHandle, "no such handle")
            }
        }
        Err(_) => fs_error(related_id, FsErrorCode::Io, "handle table is poisoned"),
    }
}

/// Whether a request would modify the export, and so must be refused on a
/// mount that is not writable before it reaches the filesystem.
fn fs_is_write_class(request: &V3Message) -> bool {
    match request {
        V3Message::Open { flags, .. } => {
            use protocol_v3::open_flags;
            flags
                & (open_flags::WRITE
                    | open_flags::CREATE
                    | open_flags::EXCL
                    | open_flags::TRUNC
                    | open_flags::APPEND)
                != 0
        }
        V3Message::Write { .. } => true,
        _ => false,
    }
}

/// How a request uses its handle's ordering domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HandleKey {
    handle: u64,
    /// The request mutates the file, so it runs alone.
    exclusive: bool,
}

/// One unit of work for the session's pool.
struct FsJob {
    related_id: u64,
    /// The handle whose ordering domain this request belongs to, if any.
    key: Option<HandleKey>,
    request: V3Message,
}

/// What the session thread waits on: either a frame from the peer or a
/// completed job from a worker.
enum FsEvent {
    Incoming(Result<Option<V3Frame>, V3CodecError>),
    Completed {
        related_id: u64,
        key: Option<HandleKey>,
        response: V3Message,
    },
}

/// The ordering domain of a request, and whether it needs the domain to itself.
///
/// Requests naming one handle are *applied* in send order, so a `Write`
/// followed by a `Read` observes the write. Reads do not mutate and so cannot
/// observe each other, which is why several of them may overlap without
/// weakening that guarantee — and they must, because a streaming client keeps
/// several 1 MiB reads outstanding on one file and serialising them would cost
/// it a round trip per chunk.
fn fs_ordering_key(request: &V3Message) -> Option<HandleKey> {
    let (handle, exclusive) = match request {
        V3Message::Read { handle, .. }
        | V3Message::ReadDir { handle, .. }
        | V3Message::Stat {
            target: StatTarget::Handle(handle),
            ..
        } => (*handle, false),
        V3Message::Write { handle, .. }
        | V3Message::Flush { handle }
        | V3Message::Close { handle } => (*handle, true),
        _ => return None,
    };
    Some(HandleKey { handle, exclusive })
}

/// One handle's ordering domain: what is running on it and what is waiting.
#[derive(Default)]
struct HandleDomain {
    /// Dispatched on this handle and not yet answered.
    inflight: usize,
    /// The in-flight request is exclusive, so nothing else may start.
    exclusive: bool,
    queue: VecDeque<FsJob>,
}

/// Start whatever the head of `handle`'s queue allows, returning the jobs to
/// dispatch.
///
/// Shared requests start in a batch; an exclusive one waits for the batch to
/// drain and then runs alone. The queue is strictly FIFO, so a `Write` behind
/// a burst of reads is never starved by later reads jumping it.
fn fs_pump_handle(domains: &mut HashMap<u64, HandleDomain>, handle: u64) -> Vec<FsJob> {
    let mut ready = Vec::new();
    let Some(domain) = domains.get_mut(&handle) else {
        return ready;
    };
    while let Some(front) = domain.queue.front() {
        let exclusive = front.key.is_some_and(|key| key.exclusive);
        if domain.exclusive || (exclusive && domain.inflight > 0) {
            break;
        }
        let job = domain
            .queue
            .pop_front()
            .expect("front() just reported an entry");
        domain.inflight += 1;
        domain.exclusive = exclusive;
        ready.push(job);
        if exclusive {
            break;
        }
    }
    if domain.inflight == 0 && domain.queue.is_empty() {
        domains.remove(&handle);
    }
    ready
}

fn fs_error(related_id: u64, code: FsErrorCode, message: &str) -> V3Message {
    V3Message::Error {
        related_id,
        code,
        platform_errno: 0,
        message: message.as_bytes().to_vec(),
    }
}

fn write_v3<W: Write>(writer: &mut W, id: u64, message: &V3Message) -> Result<(), ServerError> {
    writer.write_all(&protocol_v3::encode_frame(id, message)?)?;
    writer.flush()?;
    Ok(())
}

pub struct Server {
    root: PathBuf,
    next_message_id: u64,
    decoder: FrameDecoder,
    seen_destinations: HashSet<WirePath>,
    journal: Option<crate::journal::ResumeJournal>,
    compression: CompressionMode,
    compression_level: i32,
    capabilities: u32,
    fs_features: u64,
    fs_max_in_flight: usize,
    fs_workers: usize,
    fs_read_only: bool,
    fs_max_handles: usize,
    fs_handler: Arc<dyn FsHandler>,
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
            capabilities: CAP_ZSTD
                | CAP_VERSION_NEGOTIATION
                | CAP_FILTER_RULES
                | CAP_BROWSE_META
                | CAP_FS_V3
                | if cfg!(unix) { CAP_UNIX_MODES } else { 0 },
            // Every optional v3 feature gates a later phase, so an honest
            // server offers none of them yet. A client that asks for one gets
            // it back cleared and must not send the messages it gates.
            fs_features: 0,
            fs_max_in_flight: DEFAULT_FS_MAX_IN_FLIGHT,
            fs_workers: DEFAULT_FS_WORKERS,
            fs_read_only: false,
            fs_max_handles: DEFAULT_FS_MAX_HANDLES,
            fs_handler: Arc::new(ServerFsHandler),
        }
    }

    /// Serve the export read-only.
    ///
    /// The mount reports `access = ro` with a reason, and every write-class
    /// request is refused with `EROFS` before it reaches the filesystem.
    #[must_use]
    pub const fn read_only(mut self, read_only: bool) -> Self {
        self.fs_read_only = read_only;
        self
    }

    /// Bound one v3 session's concurrency.
    ///
    /// `max_in_flight` is the number of accepted-but-unanswered requests; past
    /// it a request is refused with `ELIMIT` rather than stalling the session.
    /// `workers` is the pool that executes them.
    ///
    /// # Panics
    ///
    /// Panics when either bound is zero, which would deadlock the session.
    #[must_use]
    pub fn with_fs_limits(mut self, max_in_flight: usize, workers: usize) -> Self {
        assert!(
            max_in_flight > 0 && workers > 0,
            "v3 session limits must be non-zero"
        );
        self.fs_max_in_flight = max_in_flight;
        self.fs_workers = workers;
        self
    }

    /// Bound the open handles one v3 session may hold.
    ///
    /// # Panics
    ///
    /// Panics when the bound is zero, which would make every `Open` fail.
    #[must_use]
    pub fn with_fs_max_handles(mut self, max_handles: usize) -> Self {
        assert!(max_handles > 0, "a session must be allowed one handle");
        self.fs_max_handles = max_handles;
        self
    }

    /// Advertise a v3 optional-feature bitmap (see `protocol_v3::features`).
    ///
    /// The negotiated set is the intersection with the client's, so this can
    /// only ever narrow what a peer may send.
    #[must_use]
    pub const fn with_fs_features(mut self, features: u64) -> Self {
        self.fs_features = features;
        self
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
        if client_role == Role::Session && selected_version < 2 {
            return Err(ServerError::UnexpectedMessage(
                "session role requires negotiated protocol v2 or v3".to_owned(),
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
            // The grammar was committed above and is never revisited: a v3
            // session never falls back to browse v2 on a decode failure.
            return if selected_version >= 3 {
                self.run_fs_session(reader, &mut writer)
            } else {
                self.run_browse_session(reader, &mut writer)
            };
        }

        // 2. Receive SessionConfig from client.
        let frame = self.decoder.read(&mut reader)?;
        let (paranoid, delete, checksum, dry_run, exclude_patterns, filter_rules) =
            match frame.message {
                Message::SessionConfig {
                    paranoid,
                    delete,
                    checksum,
                    dry_run,
                    exclude_patterns,
                    filter_rules,
                    ..
                } => (
                    paranoid,
                    delete,
                    checksum,
                    dry_run,
                    exclude_patterns,
                    filter_rules,
                ),
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
            exclude_patterns.len() + filter_rules.len()
        ));
        let session_filter = filter_from_wire(&exclude_patterns, &filter_rules)?;

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
                        &session_filter,
                    )
                }
            }
            Role::Source => self.run_source(&mut reader, &mut writer, checksum),
            Role::Session => unreachable!("browse sessions return before sync dispatch"),
        }
    }

    /// Serve one v3 filesystem session.
    ///
    /// The session opens with the `Features` exchange, then dispatches requests
    /// to a bounded worker pool and writes each response as it completes, in
    /// whatever order that happens (`xsyncv3.md` E3-S1). Three things stay on
    /// this thread because they must never queue behind filesystem work: the
    /// feature exchange, `Keepalive`, and `Cancel`.
    ///
    /// Ordering: requests naming the same handle are executed one at a time in
    /// send order, so a `Write` then `Read` on one handle observes the write.
    /// Everything else runs concurrently. The writer is only ever touched from
    /// this thread, so responses cannot interleave on the wire.
    fn run_fs_session<R: Read + Send + 'static, W: Write>(
        &mut self,
        mut reader: R,
        writer: &mut W,
    ) -> Result<(), ServerError> {
        let (events, incoming) = crossbeam_channel::unbounded();
        let reader_events = events.clone();
        thread::spawn(move || loop {
            let frame = protocol_v3::read_frame(&mut reader);
            let done = matches!(frame, Ok(None) | Err(_));
            if reader_events.send(FsEvent::Incoming(frame)).is_err() || done {
                break;
            }
        });

        let state = Arc::new(FsSessionState {
            root: self.root.clone(),
            read_only: self.fs_read_only,
            cancelled: Mutex::new(HashSet::new()),
            handles: RwLock::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            reserved_handles: AtomicU64::new(0),
            max_handles: self.fs_max_handles as u64,
        });
        let (work, jobs) = crossbeam_channel::unbounded::<FsJob>();
        for _ in 0..self.fs_workers {
            let jobs = jobs.clone();
            let events = events.clone();
            let state = Arc::clone(&state);
            let handler = Arc::clone(&self.fs_handler);
            thread::spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let response = handler.handle(&state, job.related_id, job.request);
                    if events
                        .send(FsEvent::Completed {
                            related_id: job.related_id,
                            key: job.key,
                            response,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        // The workers hold the only remaining receivers, so dropping this one
        // lets them exit when `work` goes out of scope at the end of the
        // session.
        drop(jobs);

        let mut seen_ids = HashSet::new();
        let mut negotiated_features: Option<u64> = None;
        let mut in_flight: usize = 0;
        let mut running: HashSet<u64> = HashSet::new();
        let mut handle_domains: HashMap<u64, HandleDomain> = HashMap::new();
        // `None` until a `Mount` has answered; then the writability this
        // session was granted, with the reason a write is refused.
        let mut mount: Option<(bool, Vec<u8>)> = None;
        let mut eof = false;

        loop {
            // Everything the peer sent has been answered and no more is
            // coming: the session is over. Checked before blocking, because
            // this thread holds a sender and so `recv` would never return.
            if eof && in_flight == 0 {
                return Ok(());
            }
            let Ok(event) = incoming.recv() else {
                return Ok(());
            };
            let frame = match event {
                FsEvent::Incoming(Err(error)) => return Err(ServerError::FsSession(error)),
                FsEvent::Incoming(Ok(None)) => {
                    eof = true;
                    continue;
                }
                FsEvent::Incoming(Ok(Some(frame))) => frame,
                FsEvent::Completed {
                    related_id,
                    key,
                    response,
                } => {
                    running.remove(&related_id);
                    in_flight -= 1;
                    write_v3(writer, self.next_id(), &response)?;
                    // Release this handle's ordering domain to the next
                    // request queued behind it, if any.
                    if let Some(key) = key {
                        if let Some(domain) = handle_domains.get_mut(&key.handle) {
                            domain.inflight -= 1;
                            if domain.inflight == 0 {
                                domain.exclusive = false;
                            }
                        }
                        for job in fs_pump_handle(&mut handle_domains, key.handle) {
                            running.insert(job.related_id);
                            if work.send(job).is_err() {
                                return Err(ServerError::UnexpectedMessage(
                                    "v3 worker pool stopped".to_owned(),
                                ));
                            }
                        }
                    }
                    continue;
                }
            };

            if !seen_ids.insert(frame.message_id) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "duplicate v3 session message ID {}",
                    frame.message_id
                )));
            }
            let related_id = frame.message_id;
            match (negotiated_features, frame.message) {
                // The exchange is the first frame of the session, so a client
                // cannot send a feature-gated request before the server has
                // said whether it has the feature.
                (None, V3Message::Features { features }) => {
                    let common = features & self.fs_features;
                    negotiated_features = Some(common);
                    server_log(format_args!(
                        "v3 features: client=0x{features:x}, server=0x{:x}, common=0x{common:x}",
                        self.fs_features
                    ));
                    write_v3(
                        writer,
                        self.next_id(),
                        &V3Message::FeaturesAck {
                            related_id,
                            features: common,
                        },
                    )?;
                }
                (None, other) => {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "v3 session expects Features first, got {other:?}"
                    )))
                }
                (Some(_), V3Message::Features { .. }) => {
                    return Err(ServerError::UnexpectedMessage(
                        "v3 features exchanged twice".to_owned(),
                    ))
                }
                // Answered on this thread: a keepalive that queued behind a
                // large read would report the session dead while it is merely
                // busy.
                (Some(_), V3Message::Keepalive { nonce }) => {
                    write_v3(writer, self.next_id(), &V3Message::KeepaliveAck { nonce })?;
                }
                // Likewise a cancel, which exists to overtake queued work.
                (Some(_), V3Message::Cancel { related_id: target }) => {
                    let queued = handle_domains.values_mut().find_map(|domain| {
                        let position = domain
                            .queue
                            .iter()
                            .position(|job| job.related_id == target)?;
                        domain.queue.remove(position)
                    });
                    if queued.is_some() {
                        // Never started: this is the target's terminal response.
                        in_flight -= 1;
                        write_v3(
                            writer,
                            self.next_id(),
                            &fs_error(target, FsErrorCode::Cancelled, "cancelled before execution"),
                        )?;
                    } else if running.contains(&target) {
                        // Already executing: flag it and let the handler's own
                        // response be the terminal one, so the client never
                        // sees two answers to one request.
                        state.cancel(target);
                    } else {
                        write_v3(
                            writer,
                            self.next_id(),
                            &fs_error(target, FsErrorCode::Cancelled, "request already complete"),
                        )?;
                    }
                }
                // Session setup, so answered here rather than on the pool: it
                // runs once, nothing else may run before it, and handling it
                // inline is what lets the gates below be a simple check rather
                // than a race against an in-flight mount. It probes the export
                // (a temporary file and `pathsem`), which is the one place a
                // session blocks on I/O before its workers can start.
                (
                    Some(_),
                    V3Message::Mount {
                        export,
                        requested_access,
                    },
                ) => {
                    let response = if mount.is_some() {
                        fs_error(
                            related_id,
                            FsErrorCode::Invalid,
                            "this session is already mounted",
                        )
                    } else {
                        let info = mount_info(&state, related_id, &export, requested_access);
                        if let V3Message::MountInfo {
                            effective_writable,
                            reason,
                            ..
                        } = &info
                        {
                            mount = Some((*effective_writable, reason.clone()));
                        }
                        info
                    };
                    write_v3(writer, self.next_id(), &response)?;
                }
                // Every verb needs the mount's answer first: it is what says
                // whether the session may write at all.
                (Some(_), _) if mount.is_none() => {
                    write_v3(
                        writer,
                        self.next_id(),
                        &fs_error(
                            related_id,
                            FsErrorCode::Invalid,
                            "session is not mounted; send Mount and await MountInfo first",
                        ),
                    )?;
                }
                // Refused here rather than in a worker, so a write on a
                // read-only mount never reaches the filesystem at all.
                (Some(_), request)
                    if fs_is_write_class(&request)
                        && mount.as_ref().is_some_and(|(writable, _)| !writable) =>
                {
                    let reason = mount
                        .as_ref()
                        .map_or_else(Vec::new, |(_, reason)| reason.clone());
                    server_log(format_args!(
                        "refused write-class request {related_id} on a read-only mount"
                    ));
                    write_v3(
                        writer,
                        self.next_id(),
                        &V3Message::Error {
                            related_id,
                            code: FsErrorCode::ReadOnly,
                            platform_errno: 0,
                            message: reason,
                        },
                    )?;
                }
                (Some(_), request) => {
                    if in_flight >= self.fs_max_in_flight {
                        write_v3(
                            writer,
                            self.next_id(),
                            &fs_error(
                                related_id,
                                FsErrorCode::Limit,
                                "too many requests in flight on this session",
                            ),
                        )?;
                        continue;
                    }
                    in_flight += 1;
                    let key = fs_ordering_key(&request);
                    let job = FsJob {
                        related_id,
                        key,
                        request,
                    };
                    // A keyed request always goes through its handle's queue,
                    // even when the domain is idle, so send order is decided in
                    // exactly one place.
                    let ready = if let Some(key) = key {
                        handle_domains
                            .entry(key.handle)
                            .or_default()
                            .queue
                            .push_back(job);
                        fs_pump_handle(&mut handle_domains, key.handle)
                    } else {
                        vec![job]
                    };
                    for job in ready {
                        running.insert(job.related_id);
                        if work.send(job).is_err() {
                            return Err(ServerError::UnexpectedMessage(
                                "v3 worker pool stopped".to_owned(),
                            ));
                        }
                    }
                }
            }
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
                    Ok(Err(error)) => return Err(ServerError::Browse(error)),
                    // Clean EOF, or the reader thread's channel closed: both
                    // mean no further frames will arrive, which ends the
                    // session normally.
                    Ok(Ok(None)) | Err(_) => return Ok(()),
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
                V2Message::SetPermissionsRequest { path, mode } => {
                    if self.capabilities & CAP_BROWSE_META == 0 {
                        return Err(ServerError::UnexpectedMessage(
                            "v2 session received a browse-meta message without CAP_BROWSE_META"
                                .to_owned(),
                        ));
                    }
                    Some(self.browse_set_permissions_response(&path, mode, frame.message_id))
                }
                V2Message::SetMtimeRequest { path, mtime_ns } => {
                    if self.capabilities & CAP_BROWSE_META == 0 {
                        return Err(ServerError::UnexpectedMessage(
                            "v2 session received a browse-meta message without CAP_BROWSE_META"
                                .to_owned(),
                        ));
                    }
                    Some(self.browse_set_mtime_response(&path, mtime_ns, frame.message_id))
                }
                V2Message::ReadLinkRequest { path } => {
                    if self.capabilities & CAP_BROWSE_META == 0 {
                        return Err(ServerError::UnexpectedMessage(
                            "v2 session received a browse-meta message without CAP_BROWSE_META"
                                .to_owned(),
                        ));
                    }
                    Some(self.browse_read_link_response(&path, frame.message_id))
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
                    content_size,
                    digest,
                } => match self.browse_publish(
                    &path,
                    frame.message_id,
                    size,
                    mtime_ns,
                    device,
                    file,
                    content_size,
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
                | V2Message::SetPermissionsResponse { .. }
                | V2Message::SetMtimeResponse { .. }
                | V2Message::ReadLinkResponse { .. }
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
        content_size: u64,
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
        while offset < content_size {
            let frame = if let Some(frame) = pending.pop_front() {
                frame
            } else {
                match incoming.recv() {
                    Ok(Ok(Some(frame))) => frame,
                    Ok(Err(error)) => return Err(ServerError::Browse(error)),
                    // Mid-publish, an EOF and a closed reader channel are both
                    // the peer going away before the transfer completed.
                    Ok(Ok(None)) | Err(_) => return Err(ServerError::PeerDisconnected),
                }
            };
            match frame.message {
                V2Message::PublishChunk {
                    related_id: id,
                    offset: chunk_offset,
                    data,
                } if id == related_id && chunk_offset == offset => {
                    offset = offset.saturating_add(data.len() as u64);
                    if offset > content_size {
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
            size: content_size,
            mtime: nanos_to_system_time(mtime_ns),
            mode: permission_mode(&metadata),
            fingerprint: SourceFingerprint {
                identity: FileIdentity { device, file },
                kind: ScanEntryKind::File,
                size: content_size,
                mtime: nanos_to_system_time(mtime_ns),
                ctime: None,
                // A real local file with its metadata already in hand, so the
                // ownership and link count describe this host correctly.
                unix: crate::scanner::unix_metadata(&metadata),
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
        let Ok(native) = validate_destination_path(&self.root, relative.clone()) else {
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
                                                });
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
            let Ok(item) = item else { continue };
            let Ok(metadata) = fs::symlink_metadata(item.path()) else {
                continue;
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
        mutation_response(related_id, result, MutationKind::Rename)
    }

    fn browse_create_directory_response(&self, path: &[u8], related_id: u64) -> V2Message {
        let result = (|| -> Result<(), (MutationStatus, String)> {
            let path = WirePath::from_wire(path.to_vec())
                .map_err(|error| (MutationStatus::Error, error.to_string()))?;
            let path = validate_destination_path(&self.root, path)
                .map_err(|error| mutation_failure(&error))?;
            fs::create_dir(path).map_err(|error| (mkdir_status(&error), error.to_string()))
        })();
        mutation_response(related_id, result, MutationKind::CreateDirectory)
    }

    fn browse_set_permissions_response(
        &self,
        path: &[u8],
        mode: u32,
        related_id: u64,
    ) -> V2Message {
        let result = (|| -> Result<(), (MutationStatus, String)> {
            let path =
                browse_stat_path(&self.root, path).map_err(|error| mutation_failure(&error))?;
            set_path_permissions(&path, mode & 0o7777)
                .map_err(|error| (meta_status(&error), error.to_string()))
        })();
        mutation_response(related_id, result, MutationKind::SetPermissions)
    }

    fn browse_set_mtime_response(&self, path: &[u8], mtime_ns: i64, related_id: u64) -> V2Message {
        let result = (|| -> Result<(), (MutationStatus, String)> {
            let path =
                browse_stat_path(&self.root, path).map_err(|error| mutation_failure(&error))?;
            let seconds = mtime_ns.div_euclid(1_000_000_000);
            let nanos = u32::try_from(mtime_ns.rem_euclid(1_000_000_000)).unwrap_or(0);
            set_file_mtime(&path, FileTime::from_unix_time(seconds, nanos))
                .map_err(|error| (meta_status(&error), error.to_string()))
        })();
        mutation_response(related_id, result, MutationKind::SetMtime)
    }

    fn browse_read_link_response(&self, path: &[u8], related_id: u64) -> V2Message {
        let native = match browse_stat_path(&self.root, path) {
            Ok(path) => path,
            Err(error) => {
                return V2Message::ReadLinkResponse {
                    related_id,
                    status: StatStatus::Error,
                    target: Vec::new(),
                    error: error.to_string().into_bytes(),
                };
            }
        };
        match fs::read_link(&native) {
            Ok(target) => V2Message::ReadLinkResponse {
                related_id,
                status: StatStatus::Ok,
                target: native_path_bytes(target.as_os_str()),
                error: Vec::new(),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => V2Message::ReadLinkResponse {
                related_id,
                status: StatStatus::Missing,
                target: Vec::new(),
                error: Vec::new(),
            },
            Err(error) => V2Message::ReadLinkResponse {
                related_id,
                status: StatStatus::Error,
                target: Vec::new(),
                error: error.to_string().into_bytes(),
            },
        }
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
        // Shared with the apply pool, exactly as the control session does.
        let sink = Arc::new(Sink::new(&self.root)?);
        let mut apply = ApplyPool::new(&sink, false, apply_worker_count());
        let mut acks_unflushed = false;
        // file_id -> EntryRecord for small/whole files.
        let mut active_files: HashMap<u64, EntryRecord> = HashMap::new();
        // file_id -> FileEntry for chunked large files, plus the ranges this
        // session has written (for merge-on-checkpoint).
        let mut large_files: HashMap<u64, FileEntry> = HashMap::new();
        let mut large_ranges: HashMap<u64, Vec<ByteRange>> = HashMap::new();

        loop {
            if acks_unflushed {
                writer.flush()?;
                acks_unflushed = false;
            }
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
                    digest,
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
                            &file_entry.path,
                            &mut self.seen_destinations,
                        )?;
                        apply.submit(ApplyJob {
                            message_id: frame.message_id,
                            entry: file_entry,
                            hash: blake3::Hash::from_bytes(digest),
                            data,
                        })?;
                        // Drain fully at the batch boundary; the sender waits
                        // for every acknowledgement there.
                        let limit = if active_files.is_empty() {
                            0
                        } else {
                            apply.capacity()
                        };
                        apply.collect(limit, |id| {
                            acks_unflushed = true;
                            self.ack_buffered(writer, id, 4)
                        })?;
                    } else if let Some(file_entry) = large_files.get(&file_id) {
                        let length = data.len() as u64;
                        let hash = blake3::Hash::from_bytes(digest);
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
                        &rel_path,
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
                            unix: None,
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

        apply.finish(|id| {
            acks_unflushed = true;
            self.ack_buffered(writer, id, 4)
        })?;
        writer.flush()?;
        Ok(())
    }

    /// Write an acknowledgement frame, flushing after use.
    /// Acknowledge without flushing.
    ///
    /// Used where the caller controls the flush point — the apply pool
    /// acknowledges in bursts, and flushing each one would restore the
    /// per-file write syscall the pool exists to amortise.
    fn ack_buffered<W: Write>(
        &mut self,
        writer: &mut W,
        id: u64,
        ack_type: u8,
    ) -> Result<(), ServerError> {
        let ack = Message::Ack {
            acknowledged_id: id,
            acknowledged_type: ack_type,
        };
        let msg_id = self.next_id();
        let bytes = encode_frame(msg_id, &ack)?;
        writer.write_all(&bytes)?;
        Ok(())
    }

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
        filter: &crate::filter::FilterSet,
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
                        if filter.decide(&entry.path.to_string()).is_included() {
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
            let bytes = encode_meta_frame(
                msg_id,
                &scan_msg,
                self.compression == CompressionMode::Zstd,
                self.compression_level,
            )?;
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
                let bytes = encode_meta_frame(
                    msg_id,
                    &scan_msg,
                    self.compression == CompressionMode::Zstd,
                    self.compression_level,
                )?;
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

        // Initialize sink. Shared with the apply pool, which publishes files
        // off the decode thread.
        let sink = Arc::new(Sink::new(&self.root)?);
        let mut apply = ApplyPool::new(&sink, paranoid, apply_worker_count());
        // Acks are written buffered and flushed before this thread can block on
        // input. The sender drains to zero at every batch boundary, so an
        // unflushed acknowledgement there would deadlock both sides.
        let mut acks_unflushed = false;

        // Map of upcoming file_id -> EntryRecord for small/medium and large files.
        let mut active_files: HashMap<u64, EntryRecord> = HashMap::new();
        let mut large_files: HashMap<u64, FileEntry> = HashMap::new();
        // Verified large-file ranges per file_id, for durable checkpointing.
        let mut large_ranges: HashMap<u64, Vec<ByteRange>> = HashMap::new();
        // Chunks written but not yet flushed, per file. The journal may lag the
        // in-memory track; memory is lost in a crash, so that is correct while
        // a flush always precedes the checkpoint recording those ranges.
        let mut unsynced: HashMap<u64, usize> = HashMap::new();

        // Process incoming transfer operations.
        loop {
            if acks_unflushed {
                writer.flush()?;
                acks_unflushed = false;
            }
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
                            // Carries the mode too, which is the whole point of
                            // a metadata-only repair: a chmod on the source
                            // changes nothing a content comparison can see.
                            sink.apply_file_mode(&rel_path, mode)?;
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
                    digest,
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
                            &file_entry.path,
                            &mut self.seen_destinations,
                        )?;
                        // Publishing is independent per file, so it happens on
                        // the pool while this thread keeps decoding. The ack
                        // still follows the commit, so the durability contract
                        // is unchanged.
                        apply.submit(ApplyJob {
                            message_id: frame.message_id,
                            entry: file_entry,
                            hash: blake3::Hash::from_bytes(digest),
                            data,
                        })?;
                        // Inside a batch, publishing overlaps decoding. At the
                        // batch boundary it must not: the sender drains to zero
                        // there, so anything still in flight would be an
                        // acknowledgement it waits for and never receives.
                        // `active_files` empties exactly when the batch is done.
                        let limit = if active_files.is_empty() {
                            0
                        } else {
                            apply.capacity()
                        };
                        apply.collect(limit, |id| {
                            acks_unflushed = true;
                            self.ack_buffered(writer, id, 4)
                        })?;
                    } else if let Some(file_entry) = large_files.get(&file_id) {
                        let hash = blake3::Hash::from_bytes(digest);
                        let length = data.len() as u64;
                        // Flush cadence is `receiver_flush_chunks()`: 1 keeps
                        // today's per-chunk behaviour, N flushes every N, and 0
                        // defers everything to `LargeFileFinish`. The ordering
                        // invariant never changes -- a range is journalled only
                        // after the bytes are flushed -- so a wider cadence
                        // trades resume granularity, never correctness (4.66).
                        let cadence = crate::tuning::receiver_flush_chunks();
                        if cadence == 1 {
                            sink.write_chunk_with_retry(
                                file_entry,
                                offset,
                                length,
                                &hash,
                                |_attempt| Ok(data.clone()),
                            )?;
                        } else {
                            sink.write_chunk_deferred(
                                file_entry,
                                offset,
                                length,
                                &hash,
                                |_attempt| Ok(data.clone()),
                            )?;
                        }

                        let range = ByteRange { offset, length };
                        let track = large_ranges
                            .get_mut(&file_id)
                            .expect("large file range tracker is initialized");
                        track.push(range);

                        let pending = unsynced.entry(file_id).or_insert(0);
                        *pending += 1;
                        let due = cadence == 1 || (cadence > 1 && *pending >= cadence);
                        if due {
                            if cadence != 1 {
                                sink.sync_staged_chunks(file_entry)?;
                            }
                            *pending = 0;
                            let journal = self
                                .journal
                                .as_ref()
                                .expect("journal is initialized during handshake");
                            let identity = crate::journal::ResumeIdentity {
                                path: file_entry.path.clone().into_bytes(),
                                fingerprint: file_entry.fingerprint,
                            };
                            journal.checkpoint(&identity, track)?;
                        }

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
                        &rel_path,
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
                            unix: None,
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
                    // Flush anything the cadence left outstanding before the
                    // coverage check reads the journal or the commit renames.
                    // `commit_temp` does not sync, so an unflushed range would
                    // publish a hole and under-report coverage at once.
                    if unsynced.remove(&file_id).is_some_and(|p| p > 0) {
                        sink.sync_staged_chunks(&entry)?;
                        if let Some(track) = large_ranges.get(&file_id) {
                            let identity = crate::journal::ResumeIdentity {
                                path: entry.path.clone().into_bytes(),
                                fingerprint: entry.fingerprint,
                            };
                            self.journal
                                .as_ref()
                                .expect("journal is initialized during handshake")
                                .checkpoint(&identity, track)?;
                        }
                    }
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
                        if paranoid {
                            let staged_path = sink.temporary_path(&entry.path)?;
                            let readback = fs::read(&staged_path)?;
                            if *blake3::hash(&readback).as_bytes() != digest {
                                return Err(ServerError::Sink(SinkError::VerificationFailed {
                                    path: entry.path.to_string(),
                                    attempts: 2,
                                }));
                            }
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
                    // Every file must be published before the counts are
                    // reported, or the client is told about work still in
                    // flight.
                    apply.collect(0, |id| {
                        acks_unflushed = true;
                        self.ack_buffered(writer, id, 4)
                    })?;
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

        // Publish anything still in flight, then stop the workers.
        apply.finish(|id| {
            acks_unflushed = true;
            self.ack_buffered(writer, id, 4)
        })?;
        writer.flush()?;
        Ok(())
    }

    fn run_source<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        checksum: bool,
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
            let hash_cache = checksum
                .then(|| HashCache::open(HashCache::default_path()).ok())
                .flatten();
            let scan_result = scan(&self.root)?;
            for item in scan_result.entries() {
                let entry = item?;
                entries.push(if checksum {
                    content_entry_record(&self.root, &entry, hash_cache.as_ref())?
                } else {
                    entry_record_from_file_entry(&entry)
                });
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
                            digest: *stable_read.blake3.as_bytes(),
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
                        if outstanding >= crate::tuning::max_pipelined_frames() {
                            drain_acks(
                                &mut self.decoder,
                                reader,
                                &mut outstanding,
                                crate::tuning::max_pipelined_frames() / 2,
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
                            unix: None,
                        },
                    };
                    // The wire cannot carry ctime or the unix block, so a
                    // fingerprint rebuilt from it can never equal a real stat.
                    // Re-derive it from our own filesystem: this is our file,
                    // and accepting the peer's description of it would make the
                    // replacement check in `read_range` compare against a value
                    // that is wrong by construction. Falling back to the
                    // wire-derived entry keeps a file we cannot stat behaving
                    // as it did before rather than failing the transfer.
                    let entry =
                        match std::fs::symlink_metadata(entry.path.to_native_path(&self.root)) {
                            Ok(metadata) if metadata.is_file() => {
                                match metadata.modified().ok().and_then(|mtime| {
                                    crate::scanner::fingerprint_from_metadata(
                                        &metadata,
                                        ScanEntryKind::File,
                                        mtime,
                                    )
                                    .ok()
                                }) {
                                    Some(fingerprint) => FileEntry {
                                        size: metadata.len(),
                                        fingerprint,
                                        ..entry
                                    },
                                    None => entry,
                                }
                            }
                            _ => entry,
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
                        // Read only the requested range. This used to call
                        // `read`, which buffers and hashes the *whole* file,
                        // and then kept one 8 MB slice of it -- so serving a
                        // 500 MB file in 61 chunks read and BLAKE3'd about
                        // 30 GB to move 500 MB. The cost per chunk scaled with
                        // the file size rather than the chunk size, which is
                        // why pull ran at 29.7 MB/s against rsync's 109.9 on
                        // the same link (4.61).
                        //
                        // `read_range` checks the descriptor and pathname
                        // before and after the read, so a file replaced mid
                        // transfer is still reported rather than silently
                        // mixed in.
                        let length = range.length.min(entry.size.saturating_sub(range.offset));
                        let data = source_reader.read_range(entry, range.offset, length)?;

                        let seg = Message::FileSegment {
                            file_id,
                            offset: range.offset,
                            digest: *blake3::hash(&data).as_bytes(),
                            data,
                        };
                        let msg_id = self.next_id();
                        write_data_frame(
                            writer,
                            msg_id,
                            &seg,
                            self.compression == CompressionMode::Zstd,
                            self.compression_level,
                        )?;

                        // No wait for a per-segment acknowledgement here. It
                        // used to block until the client had written the chunk
                        // to disk and checkpointed the resume journal, which
                        // made the source idle for exactly as long as the
                        // receiver was busy, and prevented the client from
                        // requesting ahead at all (4.62).
                        //
                        // Backpressure is now the client's request window: it
                        // has at most `large_chunks_in_flight` requests
                        // outstanding, so no more than that many segments can
                        // be in flight toward it.
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
pub fn run_server_stdio(root: PathBuf, read_only: bool) -> Result<(), ServerError> {
    server_log(format_args!(
        "process started: pid={}, root={}, read_only={read_only}",
        std::process::id(),
        root.display()
    ));
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut server = Server::new(root).read_only(read_only);
    let reader = BufReader::new(stdin);
    let mut writer = BufWriter::with_capacity(TRANSPORT_WRITE_BUFFER, stdout);
    server_log("waiting for client handshake");
    let result = server.run(reader, &mut writer);
    match &result {
        Ok(()) => server_log("session finished successfully"),
        Err(error) => server_fail(error),
    }
    result
}

/// Default frames the client may leave unacknowledged before it drains replies.
///
/// The receiver acknowledges every frame and blocks once its own writes fill the
/// socket buffer, so a client that writes without ever reading can deadlock
/// against it. An acknowledgement frame is 41 bytes, so this window keeps the
/// peer's pending replies near 84 KiB: inside OpenSSH's 2 MiB channel window,
/// but *above* Linux's default 64 KiB pipe buffer, so the margin here comes
/// from the SSH transport rather than from a pipe.
///
/// This value was derived at a **5.3 ms** in-session round trip, measured over
/// a USB gigabit adapter, and the window that keeps a pipe full scales with the
/// bandwidth-delay product. Read it through
/// [`crate::tuning::max_pipelined_frames`], which allows a sweep to override it
/// without a rebuild, rather than referring to this constant directly.
pub const MAX_PIPELINED_FRAMES: usize = 2048;

/// Accepted-but-unanswered v3 requests allowed on one session (`xsyncv3.md`
/// E1-S5). Past it a request is refused with `ELIMIT`; it never stalls the
/// session, because a stalled reader cannot service a keepalive or a cancel.
pub const DEFAULT_FS_MAX_IN_FLIGHT: usize = 64;

/// Largest `Read.length` and `Write.data` a v3 session accepts, advertised in
/// `MountInfo`. The envelope caps this at `MAX_DATA_SEGMENT` (8 MiB); 1 MiB is
/// the point where a read is one frame and still fits comfortably in flight.
pub const DEFAULT_FS_MAX_TRANSFER: u32 = 1024 * 1024;

/// Open handles allowed on one session (`xsyncv3.md` E1-S5).
///
/// Bounded so one session cannot exhaust the process's descriptors; a server
/// serving many sessions is bounded by this times its session limit.
pub const DEFAULT_FS_MAX_HANDLES: usize = 1024;

/// Worker threads executing v3 requests for one session.
///
/// Requests are I/O-bound, so this is not sized from core count: it is the
/// number of filesystem operations worth having outstanding at once.
pub const DEFAULT_FS_WORKERS: usize = 8;

/// Bytes per chunk when a file is larger than one data segment.
///
/// Named because the pipelining depth in [`crate::tuning::large_chunks_in_flight`]
/// is derived from it and the negotiated byte window; the two must agree.
pub const LARGE_FILE_CHUNK: u64 = 8 * 1024 * 1024;

/// Write buffer for transport streams.
///
/// `BufWriter::new` defaults to 8 KB, which on a small-file corpus holds barely
/// one frame — congress averages 5,327 bytes per file — so the pipelining above
/// could never actually coalesce: the buffer filled and flushed every frame or
/// two regardless. A megabyte holds roughly 190 congress frames.
const TRANSPORT_WRITE_BUFFER: usize = 1024 * 1024;

/// Read acknowledgements until at most `limit` frames remain outstanding.
/// Send small files as coalesced, pipelined batches.
///
/// Extracted so the single-stream and multi-stream paths cannot drift again.
/// They already had: the multi-stream path sent every small file as its own
/// one-entry `FileBatch` plus one `FileSegment`, each followed by a blocking
/// ack — two synchronous round trips per file, which cost 12x on a 1.3 ms link
/// (congress-1k: 4.42 s against single-stream's 0.35 s).
///
/// Files are coalesced up to `BATCH_TARGET_SIZE` / `MAX_BATCH_FILES`, and their
/// segments are written without stopping for each acknowledgement.
///
/// # Errors
///
/// Returns a transport or remote error; individual file read failures are
/// reported through `emit` and counted, not propagated.
#[allow(clippy::too_many_arguments)]
/// Split the small files into batches without reading any of them.
///
/// Boundaries come from the recorded sizes alone so the loader can run ahead of
/// the writer. The previous inline version accumulated only successfully read
/// files, so a read failure could shift a boundary; batching is a performance
/// detail and the transferred set is identical either way.
fn plan_small_file_batches(small_files: &[FileEntry]) -> Vec<std::ops::Range<usize>> {
    let mut batches = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0u64;
    for (index, file) in small_files.iter().enumerate() {
        let count = index - start;
        if count > 0
            && (count >= crate::tuning::max_batch_files()
                || bytes.saturating_add(file.size) > crate::tuning::batch_target_size())
        {
            batches.push(start..index);
            start = index;
            bytes = 0;
        }
        bytes = bytes.saturating_add(file.size);
    }
    if start < small_files.len() {
        batches.push(start..small_files.len());
    }
    batches
}

/// Read one file, with the scan prefix stripped back off its path.
fn read_small_file(
    source_reader: &SourceReader,
    file: &FileEntry,
    prefix: &str,
) -> Result<Vec<u8>, String> {
    let mut file_to_read = file.clone();
    if !prefix.is_empty() {
        file_to_read.path = file
            .path
            .strip_prefix(format!("{prefix}/"))
            .unwrap_or_else(|| file.path.clone())
            .clone();
    }
    source_reader
        .read(&file_to_read)
        .map(|stable| stable.bytes)
        .map_err(|error| error.to_string())
}

/// Read one batch across `workers` threads, preserving plan order.
///
/// Results stay index-aligned with `files`, so the caller sees failures and
/// successes in exactly the order a serial read would have produced.
fn load_small_file_batch(
    source_reader: &SourceReader,
    files: &[FileEntry],
    prefix: &str,
    workers: usize,
) -> Vec<Result<Vec<u8>, String>> {
    let mut loaded: Vec<Result<Vec<u8>, String>> = files.iter().map(|_| Ok(Vec::new())).collect();
    if workers <= 1 || files.len() <= 1 {
        for (slot, file) in loaded.iter_mut().zip(files) {
            *slot = read_small_file(source_reader, file, prefix);
        }
        return loaded;
    }
    let chunk = files.len().div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (files_chunk, out_chunk) in files.chunks(chunk).zip(loaded.chunks_mut(chunk)) {
            scope.spawn(move || {
                for (file, slot) in files_chunk.iter().zip(out_chunk.iter_mut()) {
                    *slot = read_small_file(source_reader, file, prefix);
                }
            });
        }
    });
    loaded
}

/// Send the small files as pipelined batches, loading the next batch while the
/// current one is hashed, compressed, and written.
///
/// The loop used to be serial and phase-separated: it issued up to
/// `MAX_BATCH_FILES` blocking reads with the network idle, then hashed,
/// compressed, and framed with the disk idle. One thread alternating between
/// two resources put both endpoints near 50% CPU and made a Pi 5 receive within
/// 7% of a 7950X. A loader thread now runs one batch ahead over a bounded
/// channel, so at most two batches are resident.
fn send_small_files_batched<R: Read, W: Write, F: FnMut(LocalEvent)>(
    writer: &mut W,
    reader: &mut R,
    decoder: &mut FrameDecoder,
    source_reader: &SourceReader,
    small_files: &[FileEntry],
    prefix: &str,
    compress: bool,
    level: i32,
    workers: usize,
    next_id: &mut dyn FnMut() -> u64,
    report: &mut LocalSyncReport,
    emit: &mut F,
) -> Result<(), ServerError> {
    let batches = plan_small_file_batches(small_files);
    if batches.is_empty() {
        return Ok(());
    }
    std::thread::scope(|scope| -> Result<(), ServerError> {
        // One batch in flight plus one being built: enough to keep the disk and
        // the wire both busy without holding the corpus in memory.
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<Result<Vec<u8>, String>>>(1);
        let batch_ranges = &batches;
        scope.spawn(move || {
            for range in batch_ranges.clone() {
                let loaded =
                    load_small_file_batch(source_reader, &small_files[range], prefix, workers);
                // A send failure means the writer stopped early; so should this.
                if sender.send(loaded).is_err() {
                    return;
                }
            }
        });

        for (range, results) in batches.iter().cloned().zip(receiver) {
            let files = &small_files[range];
            let mut loaded: Vec<(&FileEntry, Vec<u8>)> = Vec::with_capacity(files.len());
            for (file, result) in files.iter().zip(results) {
                match result {
                    Ok(data) => loaded.push((file, data)),
                    Err(message) => {
                        emit(LocalEvent::Failed {
                            path: file.path.to_string(),
                            message,
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

            let batch_id = next_id();
            let bytes = encode_meta_frame(
                batch_id,
                &Message::FileBatch {
                    batch_id: 1,
                    entries,
                },
                compress,
                level,
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
                    digest: *blake3::hash(&data).as_bytes(),
                    data,
                };
                let msg_id = next_id();
                let wire_bytes =
                    write_data_frame_buffered(writer, msg_id, &seg_msg, compress, level)?;
                report.wire_bytes = report.wire_bytes.saturating_add(wire_bytes as u64);
                outstanding += 1;
                if outstanding >= crate::tuning::max_pipelined_frames() {
                    writer.flush()?;
                    drain_acks(
                        decoder,
                        reader,
                        &mut outstanding,
                        crate::tuning::max_pipelined_frames() * 3 / 4,
                    )?;
                }
            }
            writer.flush()?;
            drain_acks(decoder, reader, &mut outstanding, 0)?;

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
        Ok(())
    })
}

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
    writer: W,
    mut emit: F,
) -> Result<LocalSyncReport, ServerError> {
    // Counted at the boundary so the total cannot drift from what was sent.
    // The per-frame sum kept below is the *data* subset, not the total.
    let mut writer = crate::transport::CountingWriter::new(writer);
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
    let local_capabilities = CAP_ZSTD
        | CAP_VERSION_NEGOTIATION
        | CAP_FILTER_RULES
        | if cfg!(unix) { CAP_UNIX_MODES } else { 0 };
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
        // A push or pull never advertises CAP_FS_V3, so this route cannot
        // select v3 and has no feature bitmap to report.
        fs_v3_available: false,
        fs_v3_features: 0,
    });

    // 2. Send SessionConfig.
    let wire_filter = filter_for_peer(options, remote_capabilities)?;
    let active_filter = local_filter(options)?;
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: options.checksum,
        paranoid: options.paranoid,
        dry_run: options.dry_run,
        exclude_patterns: wire_filter.exclude_patterns,
        filter_rules: wire_filter.rules,
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
                    if active_filter.decide(&entry.path.to_string()).is_included() {
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
    let source_scan = options.filter.as_ref().map_or_else(
        || scan(source_path),
        |filter| scan_with_filter(source_path, Arc::new(filter.clone())),
    )?;
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
        if active_filter.decide(&entry.path.to_string()).is_included() {
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
    let plan = try_plan_with_fingerprint(
        mapped_source,
        dest_index,
        options.checksum,
        modes_comparable(remote_capabilities),
    )?;
    // The push source is local, so the same sparse inspection applies: a remote
    // destination will be asked to hold the apparent size, not the allocated one.
    let dropped;
    {
        let transferable: Vec<FileEntry> = plan
            .files
            .new
            .iter()
            .chain(&plan.files.changed)
            .cloned()
            .collect();
        // Ownership is deliberately not probed on this route: the destination
        // is another host, where uids mean something different, so comparing
        // them would produce a warning that cannot be acted on.
        let preflight = crate::sparse::inspect(&transferable, source_path, None);
        crate::local::report_preflight(
            &preflight,
            crate::local::OwnershipCheck::Unsupported,
            &mut emit,
        );
        dropped = preflight.dropped;
    }
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
            if pending >= crate::tuning::max_pipelined_frames() {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    crate::tuning::max_pipelined_frames() / 2,
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
            if pending >= crate::tuning::max_pipelined_frames() {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    crate::tuning::max_pipelined_frames() / 2,
                )?;
            }
        }
        writer.flush()?;
        drain_acks(&mut decoder, &mut reader, &mut pending, 0)?;

        // Transfer files.
        let source_reader = SourceReader::new(&source_reader_root);

        // Small files are coalesced and pipelined by the shared sender: one
        // metadata frame describes many files, and their segments are written
        // without stopping for each acknowledgement.
        let small_files: Vec<FileEntry> = plan
            .files
            .new
            .iter()
            .chain(&plan.files.changed)
            .filter(|file| file.size <= SMALL_FILE_LIMIT)
            .cloned()
            .collect();
        send_small_files_batched(
            &mut writer,
            &mut reader,
            &mut decoder,
            &source_reader,
            &small_files,
            &prefix,
            negotiated_compression == CompressionMode::Zstd,
            negotiated_level,
            options.local_workers,
            &mut alloc_id,
            &mut report,
            &mut emit,
        )?;

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

            // Only the small/medium path buffers the whole file, because it
            // sends the whole file as one segment anyway. Large files stream:
            // buffering them cost 555 ms and 517 ms of dead air per file before
            // a single chunk shipped, spent reading and hashing with the
            // network idle, and held the entire file in memory while doing it
            // (4.63, and the memory half of 4.23).
            if file.size <= MAX_DATA_SEGMENT as u64 {
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
                    digest: *stable.blake3.as_bytes(),
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
                    crate::journal::missing_chunks(file.size, LARGE_FILE_CHUNK, &verified_ranges);
                let mut sent_bytes = 0u64;
                // Resume needs every byte to compute the whole-file digest, and
                // `missing_chunks` can return ranges that are not chunk aligned
                // once some are already verified. Streaming covers the fresh
                // case -- the one that matters, and the one that is always
                // aligned. A resume falls back to buffering, which is what it
                // did before and is rare enough not to matter.
                let streaming = verified_ranges.is_empty();
                let mut whole = blake3::Hasher::new();
                let buffered = if streaming {
                    None
                } else {
                    match source_reader.read(&file_to_read) {
                        Ok(stable) => Some(stable),
                        Err(err) => {
                            emit(LocalEvent::Failed {
                                path: file.path.to_string(),
                                message: err.to_string(),
                            });
                            report.failed_entries = report.failed_entries.saturating_add(1);
                            continue;
                        }
                    }
                };
                // Chunks go out without stopping for each acknowledgement.
                // Before 4.60 this loop wrote one 8 MB chunk and blocked on
                // both of its acks, so the wire sat idle while the receiver
                // hashed and wrote, and the receiver sat idle while the next
                // chunk crossed. Measured against rsync over the same ssh on a
                // switched gigabit link that cost 1.8x, with *neither* end near
                // saturation -- which is what waiting looks like, not working.
                //
                // The depth is a byte budget expressed in chunks, deliberately
                // not the frame window used elsewhere: these frames are 8 MB
                // each, so 2048 in flight would be gigabytes.
                let chunk_window = crate::tuning::large_chunks_in_flight();
                let mut inflight_frames = 0usize;
                let mut inflight_chunks = 0usize;
                for range in missing {
                    let start = usize::try_from(range.offset).unwrap_or(0);
                    let len = usize::try_from(range.length).unwrap_or(0);
                    sent_bytes = sent_bytes.saturating_add(range.length);
                    let chunk = match &buffered {
                        Some(stable) => stable.bytes[start..(start + len)].to_vec(),
                        None => {
                            source_reader.read_range(&file_to_read, range.offset, range.length)?
                        }
                    };
                    // Streaming visits every chunk in order, so the whole-file
                    // digest can be accumulated as the data goes past instead
                    // of demanding a separate pass over the file.
                    if streaming {
                        whole.update(&chunk);
                    }
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
                        digest: *blake3::hash(&chunk).as_bytes(),
                        data: chunk,
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

                    inflight_frames += 2;
                    inflight_chunks += 1;
                    if inflight_chunks >= chunk_window {
                        writer.flush()?;
                        // Drain to half depth rather than to empty, so the wire
                        // stays busy while the receiver catches up. `drain_acks`
                        // surfaces a remote `Error` frame as `RemoteError` and
                        // rejects anything that is not an `Ack`, so pipelining
                        // does not weaken the checking the two blocking reads
                        // used to do.
                        let low_water = (chunk_window / 2) * 2;
                        drain_acks(&mut decoder, &mut reader, &mut inflight_frames, low_water)?;
                        inflight_chunks = inflight_frames / 2;
                    }
                    // Progress counts bytes *sent* rather than acknowledged.
                    // The two differ by at most `chunk_window` chunks, and all
                    // of them are acknowledged below before the file finishes.
                    emit(LocalEvent::Progress {
                        path: file.path.to_string(),
                        stream: 0,
                        completed: resumed_bytes.saturating_add(sent_bytes),
                        total: file.size,
                    });
                }
                // Every chunk must be acknowledged before LargeFileFinish goes
                // out. Two reasons, and the second is a silent-corruption
                // hazard: the receiver commits on Finish, so the durability the
                // resume journal assumes requires the chunks durable first --
                // and a straggling chunk ack is itself a `Message::Ack`, so it
                // would satisfy the Finish ack check below without complaint.
                writer.flush()?;
                drain_acks(&mut decoder, &mut reader, &mut inflight_frames, 0)?;

                retransmitted_bytes_total = retransmitted_bytes_total.saturating_add(sent_bytes);
                checkpoint_bytes_total =
                    checkpoint_bytes_total.saturating_add(resumed_bytes.saturating_add(sent_bytes));

                let file_digest = match &buffered {
                    Some(stable) => *stable.blake3.as_bytes(),
                    None => *whole.finalize().as_bytes(),
                };
                let finish_msg = Message::LargeFileFinish {
                    file_id,
                    digest: file_digest,
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
            // Gate before the first removal on every route.
            crate::planner::authorize_deletions(&plan, options.max_delete)
                .map_err(|refused| ServerError::DeleteRefused(refused.to_string()))?;
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
            .chain(&plan.directories.metadata)
            .collect();
        report.metadata_repaired += send_metadata_repairs(
            &mut writer,
            &mut reader,
            &mut decoder,
            &plan.files.metadata,
            &mut alloc_id,
        )?;
        // Drifted directories are repaired by the SetDirectory sweep below, but
        // they are still repairs and belong in the count.
        report.metadata_repaired += plan.directories.metadata.len();
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
            if pending >= crate::tuning::max_pipelined_frames() {
                writer.flush()?;
                drain_acks(
                    &mut decoder,
                    &mut reader,
                    &mut pending,
                    crate::tuning::max_pipelined_frames() / 2,
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

    // Everything accumulated into `wire_bytes` above was a data frame; the
    // exact total comes from the transport itself.
    report.data_wire_bytes = report.wire_bytes;
    report.wire_bytes = writer.byte_count();

    emit(LocalEvent::Finished {
        dropped_metadata: dropped,
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        data_wire_bytes: report.data_wire_bytes,
        skipped_files: report.skipped_files,
        metadata_repaired: report.metadata_repaired,
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
    reader: R,
    mut writer: W,
    mut emit: F,
) -> Result<LocalSyncReport, ServerError> {
    // Pull's payload arrives inbound, so its wire total is measured on the
    // read side. The per-frame sum kept below is the data subset.
    let mut reader = crate::transport::CountingReader::new(reader);
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
    let local_capabilities = CAP_ZSTD
        | CAP_VERSION_NEGOTIATION
        | CAP_FILTER_RULES
        | if cfg!(unix) { CAP_UNIX_MODES } else { 0 };
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
        // A push or pull never advertises CAP_FS_V3, so this route cannot
        // select v3 and has no feature bitmap to report.
        fs_v3_available: false,
        fs_v3_features: 0,
    });

    // 2. Send SessionConfig.
    let wire_filter = filter_for_peer(options, remote_capabilities)?;
    let active_filter = local_filter(options)?;
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: true,
        paranoid: options.paranoid,
        dry_run: options.dry_run,
        exclude_patterns: wire_filter.exclude_patterns,
        filter_rules: wire_filter.rules,
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
                        if active_filter.decide(&entry.path.to_string()).is_included() {
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
    let destination_hash_cache = HashCache::open(HashCache::default_path()).ok();
    let mut destination_entries = Vec::new();
    if dest_path.exists() {
        if let Ok(dest_scan) = scan(dest_path) {
            for item in dest_scan.entries() {
                if let Ok(mut entry) = item {
                    if active_filter.decide(&entry.path.to_string()).is_included() {
                        if entry.kind == ScanEntryKind::File {
                            let native = entry.path.to_native_path(dest_path);
                            entry.fingerprint.identity = cached_content_identity(
                                &native,
                                &entry,
                                destination_hash_cache.as_ref(),
                            )?;
                        }
                        destination_entries.push(entry.clone());
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
    // The pull destination is local, so the same probe the local engine uses
    // applies: two remote paths can name one file here.
    let (mapped_source, collision_failures) = crate::local::enforce_path_collisions(
        mapped_source,
        dest_path,
        options.on_path_collision,
        &mut emit,
    )
    .map_err(|error| ServerError::PathCollision(error.to_string()))?;
    let mut plan = try_plan_with_fingerprint(
        mapped_source,
        dest_index,
        true,
        modes_comparable(remote_capabilities),
    )?;
    // A content-identical file at another destination path is a rename, not a
    // new transfer. Apply the local rename before the transfer phase and leave
    // the old name out of the destination tree without reading remote bytes.
    let mut destination_by_identity: HashMap<(u64, u64), WirePath> = HashMap::new();
    for entry in destination_entries {
        if entry.kind == ScanEntryKind::File {
            destination_by_identity.insert(
                (
                    entry.fingerprint.identity.device,
                    entry.fingerprint.identity.file,
                ),
                entry.path,
            );
        }
    }
    let mut renamed = Vec::new();
    for entry in plan.files.new.clone() {
        let identity = (
            entry.fingerprint.identity.device,
            entry.fingerprint.identity.file,
        );
        let Some(old_path) = destination_by_identity.get(&identity) else {
            continue;
        };
        if old_path == &entry.path {
            continue;
        }
        let old_native = old_path.to_native_path(dest_path);
        let new_native = entry.path.to_native_path(dest_path);
        if !old_native.exists() || new_native.exists() {
            continue;
        }
        if let Some(parent) = new_native.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&old_native, &new_native)?;
        renamed.push(entry.path.clone());
    }
    if !renamed.is_empty() {
        plan.files
            .new
            .retain(|entry| !renamed.contains(&entry.path));
        for path in renamed {
            emit(LocalEvent::Skipped {
                path: path.to_string(),
                bytes: 0,
            });
        }
    }
    emit(LocalEvent::Phase {
        name: "plan",
        started: false,
    });

    let mut report = LocalSyncReport {
        local_workers: options.local_workers,
        streams: options.streams,
        failed_entries: collision_failures,
        partial_work: collision_failures > 0,
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
                && batch.len() < crate::tuning::max_batch_files()
                && (batch.is_empty()
                    || batch_bytes.saturating_add(small_files[cursor].size)
                        <= crate::tuning::batch_target_size())
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
            let bytes = encode_meta_frame(
                batch_id,
                &Message::FileBatch {
                    batch_id: 1,
                    entries,
                },
                negotiated_compression == CompressionMode::Zstd,
                options.compress_level,
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
                let (data, digest) = match seg_frame.message {
                    Message::FileSegment { data, digest, .. } => (data, digest),
                    other => {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected FileSegment, got {other:?}"
                        )))
                    }
                };
                let hash = blake3::Hash::from_bytes(digest);
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
                let (data, digest) = match seg_frame.message {
                    Message::FileSegment { data, digest, .. } => (data, digest),
                    other => {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected FileSegment, got {other:?}"
                        )))
                    }
                };

                // Write and commit file locally with Sink.
                let hash = blake3::Hash::from_bytes(digest);
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
                // Requests are issued ahead of the responses. This loop used to
                // be a full round trip per 8 MB -- request, segment, ack,
                // range-ack -- with the local write and a journal checkpoint
                // serialized in the middle, so the source idled for exactly as
                // long as this side was busy and vice versa. Pull ran at
                // 29.7 MB/s against rsync's 109.9 on the same link; 4.61 fixed
                // the quadratic re-read behind it, and this removes the
                // lockstep (4.62).
                //
                // The window is the request depth, and it is also the only
                // backpressure: the peer cannot have more segments in flight
                // than we have outstanding requests.
                let window = crate::tuning::large_chunks_in_flight();
                let ranges: Vec<ByteRange> = missing;
                // Ranges written but not yet flushed or checkpointed.
                let mut staged: Vec<ByteRange> = Vec::new();
                let mut issued = 0usize;
                let mut done = 0usize;
                while done < ranges.len() {
                    while issued < ranges.len() && issued - done < window {
                        let range_msg = Message::LargeFileRange {
                            file_id,
                            range: ByteRange {
                                offset: ranges[issued].offset,
                                length: ranges[issued].length,
                            },
                        };
                        let msg_id = alloc_id();
                        let b = encode_frame(msg_id, &range_msg)?;
                        writer.write_all(&b)?;
                        issued += 1;
                    }
                    writer.flush()?;

                    let offset = ranges[done].offset;
                    let length = ranges[done].length;
                    sent_bytes = sent_bytes.saturating_add(length);

                    // The peer answers each request with exactly two frames, in
                    // order: the segment, then the acknowledgement of the range.
                    let seg_frame = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    report.wire_bytes = report
                        .wire_bytes
                        .saturating_add(decoder.last_wire_bytes() as u64);
                    let (data, digest) = match seg_frame.message {
                        Message::FileSegment { data, digest, .. } => (data, digest),
                        other => {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "expected FileSegment, got {other:?}"
                            )))
                        }
                    };

                    let hash = blake3::Hash::from_bytes(digest);
                    sink.write_chunk_deferred(file, offset, length, &hash, |_attempt| {
                        Ok(data.clone())
                    })?;
                    staged.push(ByteRange { offset, length });

                    // Flush and checkpoint every `checkpoint_chunks` chunks
                    // rather than every chunk. The barriers cost ~21 ms per
                    // 8 MB against ~71 ms of wire time, which was the whole of
                    // pull's remaining gap to rsync (4.65).
                    //
                    // What must not change is the *order*: staged bytes are
                    // flushed before the checkpoint that records them, so a
                    // resume can never trust a range that exists only in page
                    // cache. Batching only widens how much an interruption
                    // redoes -- 64 MB at the default -- it never lets the
                    // journal run ahead of the disk.
                    let last = done + 1 == ranges.len();
                    if staged.len() >= crate::tuning::checkpoint_chunks() || last {
                        sink.sync_staged_chunks(file)?;
                        track.append(&mut staged);
                        resume_journal.checkpoint(&identity, &track)?;
                    }

                    let range_ack = decoder
                        .read(&mut reader)
                        .map_err(|e| map_transport_error(e, 0))?;
                    if !matches!(range_ack.message, Message::Ack { .. }) {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected Ack for LargeFileRange, got {:?}",
                            range_ack.message
                        )));
                    }
                    done += 1;
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
            .chain(&plan.directories.metadata)
            .cloned()
            .collect();
        sink.finish_directories(&dirs_to_finish)?;
        report.metadata_repaired += sink.repair_metadata(&plan.files.metadata)?;

        if let Some(ref root_entry) = source_root_entry {
            sink.finish_root_directory(root_entry)?;
        }

        // Delete extraneous entries if enabled.
        if options.delete && !report.partial_failure() {
            // Gate before the first removal on every route.
            crate::planner::authorize_deletions(&plan, options.max_delete)
                .map_err(|refused| ServerError::DeleteRefused(refused.to_string()))?;
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

    report.data_wire_bytes = report.wire_bytes;
    report.wire_bytes = reader.byte_count();

    emit(LocalEvent::Finished {
        dropped_metadata: crate::sparse::DroppedMetadata::default(),
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        data_wire_bytes: report.data_wire_bytes,
        skipped_files: report.skipped_files,
        metadata_repaired: report.metadata_repaired,
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

/// Rebuild the filter a session applies, from whichever representation arrived.
///
/// Fail-closed on both arms: a rule set that cannot be parsed exactly is an
/// error, never an approximation, because approximating either transfers files
/// the user excluded or skips files they asked for.
fn filter_from_wire(
    exclude_patterns: &[Vec<u8>],
    filter_rules: &[Vec<u8>],
) -> Result<crate::filter::FilterSet, ServerError> {
    if !filter_rules.is_empty() {
        return crate::filter::decode(filter_rules).map_err(|error| {
            ServerError::FilterUnrepresentable(format!("peer sent unusable filter rules: {error}"))
        });
    }
    let mut patterns = Vec::with_capacity(exclude_patterns.len());
    for pattern in exclude_patterns {
        let text = std::str::from_utf8(pattern).map_err(|_| {
            ServerError::FilterUnrepresentable(
                "peer sent an exclude pattern that is not valid UTF-8".to_owned(),
            )
        })?;
        patterns.push(text.to_owned());
    }
    crate::filter::from_exclude_patterns(&patterns).map_err(|error| {
        ServerError::FilterUnrepresentable(format!(
            "peer sent an unusable exclude pattern: {error}"
        ))
    })
}

/// Send mode-only repairs for files whose content already matches.
///
/// `SetFile` carries the mode, so no data moves. Repaired paths were never
/// transferred, so they do not collide with the receiver's duplicate check.
fn send_metadata_repairs<R: Read, W: Write>(
    writer: &mut W,
    reader: &mut R,
    decoder: &mut FrameDecoder,
    entries: &[FileEntry],
    next_id: &mut dyn FnMut() -> u64,
) -> Result<usize, ServerError> {
    if entries.is_empty() {
        return Ok(0);
    }
    let mut pending = 0usize;
    for entry in entries {
        let message = Message::Metadata {
            operation: MetadataOperation::SetFile,
            path: entry.path.as_bytes().to_vec(),
            target: Vec::new(),
            mode: entry.mode,
            mtime_ns: system_time_to_nanos(entry.mtime),
        };
        let id = next_id();
        writer.write_all(&encode_frame(id, &message)?)?;
        pending += 1;
        if pending >= crate::tuning::max_pipelined_frames() {
            writer.flush()?;
            drain_acks(
                decoder,
                reader,
                &mut pending,
                crate::tuning::max_pipelined_frames() / 2,
            )?;
        }
    }
    writer.flush()?;
    drain_acks(decoder, reader, &mut pending, 0)?;
    Ok(entries.len())
}

/// Whether permission bits may be compared against this peer.
///
/// Both ends must have real Unix modes. Either side synthesizing them makes
/// every file look permanently drifted, so the comparison is simply skipped.
const fn modes_comparable(remote_capabilities: u32) -> bool {
    cfg!(unix) && (remote_capabilities & CAP_UNIX_MODES != 0)
}

/// The filter this client applies to its own scans.
///
/// The ordered rule set when the user gave one, the flat excludes otherwise.
fn local_filter(options: &LocalSyncOptions) -> Result<crate::filter::FilterSet, ServerError> {
    match options.filter.as_ref() {
        Some(filter) => Ok(filter.clone()),
        None => filter_from_wire(&encode_exclude_patterns(&options.exclude_patterns), &[]),
    }
}

/// How a session's filter is represented on the wire.
///
/// Exactly one side is populated; the decoder rejects a message carrying both,
/// so a receiver never has to guess which describes the transfer.
#[derive(Debug)]
struct WireFilter {
    exclude_patterns: Vec<Vec<u8>>,
    rules: Vec<Vec<u8>>,
}

/// Choose the wire representation of the transfer's filter.
///
/// Returns `(exclude_patterns, filter_rules)`, exactly one of which is
/// populated. A peer advertising [`CAP_FILTER_RULES`] receives the ordered rule
/// set; one without it receives the flat exclude list, and is refused outright
/// if the filter contains an include rule — sending the excludes alone would
/// silently transfer a wider set of files than the user asked for.
fn filter_for_peer(
    options: &LocalSyncOptions,
    remote_capabilities: u32,
) -> Result<WireFilter, ServerError> {
    let Some(filter) = options.filter.as_ref() else {
        return Ok(WireFilter {
            exclude_patterns: encode_exclude_patterns(&options.exclude_patterns),
            rules: Vec::new(),
        });
    };
    if remote_capabilities & CAP_FILTER_RULES != 0 {
        return Ok(WireFilter {
            exclude_patterns: Vec::new(),
            rules: crate::filter::encode(filter),
        });
    }
    if filter.has_includes() {
        return Err(ServerError::FilterUnrepresentable(
            "--include is not supported against this remote: it is an older xsync that carries \
             only exclude patterns, and sending those alone would transfer more than you asked \
             for. Update the remote, use --exclude/--exclude-from, or run xsync on the remote \
             host so both ends of the filter are local."
                .to_owned(),
        ));
    }
    Ok(WireFilter {
        exclude_patterns: encode_exclude_patterns(&options.exclude_patterns),
        rules: Vec::new(),
    })
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
    spawn_and_run_session(dest_path, rsh, host, options.bootstrap, |reader, writer| {
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
/// Encode a frame, compressing the payload when the session negotiated zstd.
///
/// Mirrors [`encode_frame`] rather than `write_frame` so call sites keep their
/// own write-and-flush structure; the only difference is that the payload may
/// arrive compressed.
///
/// This exists for *metadata* frames. xsync compressed file payloads but never
/// the protocol chatter, while rsync's `-z` compresses its file list too — and
/// that chatter is dominated by paths, which share long prefixes and compress
/// about 15x on a real corpus. congress-1m carries roughly 49 MB of path bytes
/// that zstd-3 takes to about 3 MB.
///
/// No protocol change is needed: decompression keys off the frame header flags
/// before any message-type dispatch, so any peer that can decompress at all
/// decodes these, and the existing `CAP_ZSTD` negotiation still gates it.
fn encode_meta_frame(
    id: u64,
    message: &Message,
    compress: bool,
    level: i32,
) -> Result<Vec<u8>, ServerError> {
    let mode = if compress {
        CompressionMode::Zstd
    } else {
        CompressionMode::None
    };
    Ok(encode_frame_with_compression(id, message, mode, level)?)
}

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

/// Encode and write a data frame **without flushing**.
///
/// `write_data_frame` flushes every frame, which defeats the caller's buffering:
/// the batched small-file sender accumulates up to `MAX_PIPELINED_FRAMES` before
/// draining acks, but a per-frame flush turns that into one small write to the
/// SSH pipe per file. On congress-100k that is 109,615 flushes and the transfer
/// runs at 4.6% of the link.
///
/// Callers must flush before waiting on anything the peer sends, or the peer
/// will not have seen the frames it is being asked to acknowledge.
fn write_data_frame_buffered<W: Write>(
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
    // Settle the remote shell family first: the data threads below spawn their
    // own children and must agree with this one about how to quote the command.
    ensure_remote_shell_known(rsh, host);
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
    let mut cwriter = BufWriter::with_capacity(TRANSPORT_WRITE_BUFFER, cstdin);
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

    // The control session carries every small file and the whole plan, so it
    // needs the same capabilities as the data sessions below. Advertising none
    // meant it negotiated no compression and could not represent filter rules,
    // which silently emptied the destination index whenever an include rule was
    // in play -- every file then looked new and was re-sent on every run.
    let control_capabilities = CAP_ZSTD
        | CAP_VERSION_NEGOTIATION
        | CAP_FILTER_RULES
        | if cfg!(unix) { CAP_UNIX_MODES } else { 0 };
    write_frame(
        &mut cwriter,
        calloc(),
        &Message::Handshake {
            role: Role::Source,
            capabilities: control_capabilities,
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
        },
    )?;
    let Message::Handshake {
        compression: control_compression,
        capabilities: control_remote_capabilities,
        ..
    } = cdec
        .read(&mut creader)
        .map_err(|e| map_transport_error(e, 0))?
        .message
    else {
        return Err(ServerError::UnexpectedMessage(
            "control handshake".to_owned(),
        ));
    };
    // The negotiated mode still governs what the peer compresses back to us
    // (the destination scan pages). The control session no longer sends file
    // data itself — that is striped across the data sessions below — so there
    // is no outbound data-frame setting to derive from it here.
    debug_assert!(
        control_compression == CompressionMode::Zstd || !options.compress,
        "control session must negotiate zstd whenever compression is enabled"
    );
    expect_ack(&mut cdec, &mut creader)?;
    let control_filter = filter_for_peer(options, control_remote_capabilities)?;

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
            exclude_patterns: control_filter.exclude_patterns,
            filter_rules: control_filter.rules,
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
    let source_filter = match options.filter.as_ref() {
        Some(filter) => filter.clone(),
        None => filter_from_wire(&encode_exclude_patterns(&options.exclude_patterns), &[])?,
    };
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
        if source_filter.decide(&entry.path.to_string()).is_included() {
            mapped.push(entry);
        }
    }
    let plan = try_plan_with_fingerprint(
        mapped,
        dest_index,
        options.checksum,
        modes_comparable(control_remote_capabilities),
    )?;

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
            dropped_metadata: crate::sparse::DroppedMetadata::default(),
            transport: None,
            transferred_files: 0,
            transferred_bytes: 0,
            skipped_files: report.skipped_files,
            metadata_repaired: report.metadata_repaired,
            failed_entries: 0,
            deleted_entries: 0,
            warnings: 0,
            physical_bytes: 0,
            wire_bytes: 0,
            data_wire_bytes: 0,
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
        // Pipelined, like the single-stream path. One synchronous ack per
        // directory cost a full round trip each: on congress, which gives every
        // bill its own directory, that is nearly one round trip per file and made
        // `--streams` 30x slower than a single stream to a Raspberry Pi.
        let mut pending = 0usize;
        for dir in dirs {
            let bytes = encode_frame(
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::CreateDirectory,
                    path: dir.path.as_bytes().to_vec(),
                    target: Vec::new(),
                    mode: dir.mode,
                    mtime_ns: system_time_to_nanos(dir.mtime),
                },
            )?;
            cwriter.write_all(&bytes)?;
            pending += 1;
            if pending >= crate::tuning::max_pipelined_frames() {
                cwriter.flush()?;
                drain_acks(
                    &mut cdec,
                    &mut creader,
                    &mut pending,
                    crate::tuning::max_pipelined_frames() / 2,
                )?;
            }
        }
        cwriter.flush()?;
        drain_acks(&mut cdec, &mut creader, &mut pending, 0)?;
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
            let bytes = encode_frame(
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::CreateSymlink,
                    path: sym.path.as_bytes().to_vec(),
                    target: target.into_os_string().into_encoded_bytes(),
                    mode: sym.mode,
                    mtime_ns: system_time_to_nanos(sym.mtime),
                },
            )?;
            cwriter.write_all(&bytes)?;
            pending += 1;
            if pending >= crate::tuning::max_pipelined_frames() {
                cwriter.flush()?;
                drain_acks(
                    &mut cdec,
                    &mut creader,
                    &mut pending,
                    crate::tuning::max_pipelined_frames() / 2,
                )?;
            }
        }
        cwriter.flush()?;
        drain_acks(&mut cdec, &mut creader, &mut pending, 0)?;
    }

    // ---- Partition files ----
    // Small/medium files: striped across the data sessions (4.25).
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

    // Small files are striped across the data sessions rather than pushed down
    // the control session alone.
    //
    // Everything at or below `MAX_DATA_SEGMENT` used to ride the control
    // connection, which meant `--streams N` bought no parallelism at all on the
    // workload that is slowest — for congress that is 100% of the corpus.
    //
    // Balanced by **count, not bytes**: for files this small the per-file cost
    // dominates the payload, so equal counts share the work more evenly than
    // equal byte totals would.
    let small_shares: Vec<Vec<FileEntry>> = {
        let mut shares: Vec<Vec<FileEntry>> = (0..streams).map(|_| Vec::new()).collect();
        for (index, file) in small_files.iter().enumerate() {
            shares[index % streams].push(file.clone());
        }
        shares
    };

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
    // The data threads resolve each entry's relative path against this root.
    // Passing the raw `source_path` produced `<file>/<file>` for a single-file
    // source, because that path is already the file rather than its directory —
    // every `--streams N > 1` transfer of one file failed on it.
    let source_path_buf = source_reader_root.clone();
    let data_threads = {
        let dest = dest_path.to_owned();
        let job_id_copy = job_id;
        let compress = options.compress;
        let compression_level = options.compress_level;
        let mut handles = Vec::new();
        // Each thread opens its own connection, so establishment is concurrent
        // rather than a sequential loop -- but bounded, so a high stream count
        // does not trip the peer's `MaxStartups`.
        let gate = ConnectionGate::new(MAX_CONCURRENT_CONNECTIONS);
        let workers = options.local_workers;
        for (work, small) in data_work.into_iter().zip(small_shares) {
            let sp = source_path_buf.clone();
            let prefix_copy = prefix.clone();
            let job = job_id_copy;
            let dest_copy = dest.clone();
            let rsh_copy = rsh.map(str::to_owned);
            let host_copy = host.map(str::to_owned);
            let gate_copy = std::sync::Arc::clone(&gate);
            handles.push(std::thread::spawn(move || {
                let permit = ConnectionGate::acquire(&gate_copy);
                let child =
                    spawn_server_child(&dest_copy, rsh_copy.as_deref(), host_copy.as_deref())?;
                run_data_thread(
                    child,
                    &sp,
                    &prefix_copy,
                    job,
                    work,
                    small,
                    workers,
                    compress,
                    compression_level,
                    permit,
                )
            }));
        }
        handles
    };

    let written: Vec<Result<DataThreadResult, ServerError>> = data_threads
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
        let outcome = result?;
        report.wire_bytes = report.wire_bytes.saturating_add(outcome.wire_bytes);
        report.failed_entries = report.failed_entries.saturating_add(outcome.small_failed);
        // Small files published by this session. Accounted here so the counters
        // and the event stream stay on the caller's thread.
        for (path, bytes) in outcome.small_transferred {
            report.transferred_files = report.transferred_files.saturating_add(1);
            report.transferred_bytes = report.transferred_bytes.saturating_add(bytes);
            report.physical_bytes = report.physical_bytes.saturating_add(bytes);
            report.byte_copies = report.byte_copies.saturating_add(1);
            emit(LocalEvent::Transferred {
                path,
                bytes,
                physical_bytes: bytes,
                method: TransferMethod::ByteCopy,
            });
        }
        for (path, mut rs) in outcome.ranges {
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

        report.metadata_repaired += send_metadata_repairs(
            &mut cwriter,
            &mut creader,
            &mut cdec,
            &plan.files.metadata,
            &mut calloc,
        )?;
        report.metadata_repaired += plan.directories.metadata.len();

        // Finish directories deepest-first, then the root directory.
        let mut dirs: Vec<_> = plan
            .directories
            .new
            .iter()
            .chain(&plan.directories.changed)
            .chain(&plan.directories.unchanged)
            .chain(&plan.directories.metadata)
            .collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
        // Pipelined for the same reason as the creation pass above: one ack per
        // directory is a round trip per directory, and deep trees have roughly
        // as many directories as files.
        let mut meta_pending = 0usize;
        for dir in dirs {
            let bytes = encode_frame(
                calloc(),
                &Message::Metadata {
                    operation: MetadataOperation::SetDirectory,
                    path: dir.path.as_bytes().to_vec(),
                    target: Vec::new(),
                    mode: dir.mode,
                    mtime_ns: system_time_to_nanos(dir.mtime),
                },
            )?;
            cwriter.write_all(&bytes)?;
            meta_pending += 1;
            if meta_pending >= crate::tuning::max_pipelined_frames() {
                cwriter.flush()?;
                drain_acks(
                    &mut cdec,
                    &mut creader,
                    &mut meta_pending,
                    crate::tuning::max_pipelined_frames() / 2,
                )?;
            }
        }
        cwriter.flush()?;
        drain_acks(&mut cdec, &mut creader, &mut meta_pending, 0)?;
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
            // Gate before the first removal on every route.
            crate::planner::authorize_deletions(&plan, options.max_delete)
                .map_err(|refused| ServerError::DeleteRefused(refused.to_string()))?;
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
        dropped_metadata: crate::sparse::DroppedMetadata::default(),
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        data_wire_bytes: report.data_wire_bytes,
        skipped_files: report.skipped_files,
        metadata_repaired: report.metadata_repaired,
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
/// What one data session accomplished.
struct DataThreadResult {
    /// Large-file ranges this session durably wrote.
    ranges: Vec<(WirePath, Vec<ByteRange>)>,
    wire_bytes: u64,
    /// Small files this session published, for the caller to account and emit.
    small_transferred: Vec<(String, u64)>,
    small_failed: usize,
}
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
    small: Vec<FileEntry>,
    local_workers: usize,
    compress: bool,
    compression_level: i32,
    permit: ConnectionPermit,
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
        &small,
        local_workers,
        compress,
        compression_level,
        permit,
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
    small: &[FileEntry],
    local_workers: usize,
    compress: bool,
    compression_level: i32,
    permit: ConnectionPermit,
) -> Result<DataThreadResult, ServerError> {
    let mut writer = BufWriter::with_capacity(TRANSPORT_WRITE_BUFFER, stdin);
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
            filter_rules: Vec::new(),
        },
    )?;
    expect_ack(&mut decoder, &mut reader)?;
    // Authentication is finished, so the connection slot can go to the next
    // stream. Holding it for the whole transfer would cap concurrency instead
    // of just establishment.
    drop(permit);

    let source_reader = SourceReader::new(source_path);
    // This session's share of the small files. Batches are self-contained —
    // disjoint files, their own frames, their own acks — so they distribute
    // across the data connections the user already paid to open.
    let mut small_report = LocalSyncReport::default();
    let mut small_events: Vec<LocalEvent> = Vec::new();
    if !small.is_empty() {
        let mut collect = |event: LocalEvent| small_events.push(event);
        send_small_files_batched(
            &mut writer,
            &mut reader,
            &mut decoder,
            &source_reader,
            small,
            prefix,
            compress,
            compression_level,
            local_workers,
            &mut alloc,
            &mut small_report,
            &mut collect,
        )?;
    }
    let small_transferred: Vec<(String, u64)> = small_events
        .into_iter()
        .filter_map(|event| match event {
            LocalEvent::Transferred { path, bytes, .. } => Some((path, bytes)),
            _ => None,
        })
        .collect();
    let mut written: Vec<(WirePath, Vec<ByteRange>)> = Vec::new();
    let mut wire_bytes = small_report.wire_bytes;
    for (file, ranges) in work {
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
            let data = source_reader.read_range(&file_to_read, range.offset, range.length)?;
            wire_bytes = wire_bytes.saturating_add(write_data_frame(
                &mut writer,
                alloc(),
                &Message::FileSegment {
                    file_id,
                    offset: range.offset,
                    digest: *blake3::hash(&data).as_bytes(),
                    data,
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
    Ok(DataThreadResult {
        ranges: written,
        wire_bytes,
        small_transferred,
        small_failed: small_report.failed_entries,
    })
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
    spawn_and_run_session(src_path, rsh, host, options.bootstrap, |reader, writer| {
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
    bootstrap: crate::bootstrap::BootstrapPolicy,
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
        return run_server_child_session(child, RemoteShell::Posix, false, &mut f);
    };

    let mut shell = remote_shell_for(rsh, host_name);
    let child = spawn_server_child_with_shell(remote_path, rsh, host, shell)?;
    let mut result = run_server_child_session(child, shell, rsh.is_none(), &mut f);

    // The POSIX form silently succeeded without running anything, which only
    // stock Windows cmd.exe does. Try the cmd form.
    if matches!(result, Err(ServerError::RemoteShellMismatch)) && shell == RemoteShell::Posix {
        shell = RemoteShell::Windows;
        let child = spawn_server_child_with_shell(remote_path, rsh, host, shell)?;
        result = run_server_child_session(child, shell, rsh.is_none(), &mut f);
        // Remember the family whenever the cmd form got further than the POSIX
        // one did, not only on a completed transfer: "the binary is missing"
        // is itself proof that cmd parsed the command, and bootstrap needs the
        // right family to upload with.
        if !matches!(result, Err(ServerError::RemoteShellMismatch)) {
            remember_remote_shell(rsh, host_name, shell);
        }
    }

    // An older remote refused `--log-json`. Drop it for this host and retry:
    // structured remote records are a convenience, and a transfer must not fail
    // because the far end is a version that cannot provide them.
    if matches!(result, Err(ServerError::RemoteFlagRejected)) {
        remember_log_json_rejected(rsh, host_name);
        let child = spawn_server_child_with_shell(remote_path, rsh, host, shell)?;
        result = run_server_child_session(child, shell, rsh.is_none(), &mut f);
    }

    // The remote answered but has no xsync. With bootstrap enabled, provision a
    // verified binary and run against that. Attempted once per host: the
    // uploaded path is recorded before the retry, so a second failure is a real
    // failure rather than an upload loop.
    if matches!(result, Err(ServerError::MissingRemoteXsync))
        && bootstrap.enabled()
        && remote_program_for(rsh, host_name).is_none()
    {
        let (platform, home) = crate::bootstrap::detect_remote_platform(rsh, host_name, shell)
            .map_err(|error| ServerError::Bootstrap(error.to_string()))?;
        let binary = crate::bootstrap::locate_binary(platform)
            .map_err(|error| ServerError::Bootstrap(error.to_string()))?;
        // Reported unconditionally, including under --quiet: copying an
        // executable onto another machine and running it is a side effect the
        // operator should always see, not a progress detail.
        eprintln!(
            "bootstrap: {host_name} is {} and has no xsync; uploading {}",
            platform.target_triple(),
            binary.display()
        );
        let uploaded =
            crate::bootstrap::upload_and_verify(rsh, host_name, shell, &home, &binary, bootstrap)
                .map_err(|error| ServerError::Bootstrap(error.to_string()))?;
        eprintln!("bootstrap: checksum verified on {host_name}, running {uploaded}");
        remember_remote_program(rsh, host_name, uploaded);
        let child = spawn_server_child_with_shell(remote_path, rsh, host, shell)?;
        result = run_server_child_session(child, shell, rsh.is_none(), &mut f);
    }

    match result {
        // Still refused after the flag was dropped: not a logging problem.
        Err(ServerError::RemoteFlagRejected) => Err(ServerError::Transport {
            stream: 0,
            message: "remote rejected the server command line".to_owned(),
        }),
        // Still silent after the cmd form was tried: not a shell problem.
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

/// Quote a program name, keeping a leading `$HOME` expandable.
///
/// A bootstrap-uploaded binary lives under the remote user's home, which is
/// only known to the remote shell, so that one prefix must survive quoting
/// while the rest of the path is still protected.
fn quote_remote_arg_or_path(program: &str) -> String {
    program.strip_prefix("$HOME/").map_or_else(
        || quote_remote_arg(program),
        |rest| format!("\"$HOME\"/{}", quote_remote_arg(rest)),
    )
}

pub(crate) fn quote_remote_arg(argument: &str) -> String {
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
pub(crate) fn quote_windows_arg(argument: &str) -> Result<String, ServerError> {
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
    program: Option<&str>,
    log_json: bool,
) -> Result<String, ServerError> {
    // A bootstrap-provisioned binary is addressed by absolute path; otherwise
    // the bare name is resolved from the remote's PATH.
    let program = program.unwrap_or("xs");
    // `-` sends the remote's records to its stderr, which the client already
    // captures and relays. The remote's stdout is the binary protocol and can
    // never carry diagnostics.
    let logging = if log_json { " '--log-json' '-'" } else { "" };
    match shell {
        RemoteShell::Posix => Ok(format!(
            "PATH=\"$HOME/.local/bin:$PATH\" {}{logging} {} {}",
            quote_remote_arg_or_path(program),
            quote_remote_arg("--server"),
            quote_remote_path(remote_path)
        )),
        // `set "PATH=..." & "xs" --server "<path>"`. A single `&` rather than
        // `&&` so a `set` that reports failure still runs the server, matching
        // the POSIX form where the assignment prefix cannot fail independently.
        RemoteShell::Windows => Ok(format!(
            "set \"PATH=%USERPROFILE%\\.local\\bin;%PATH%\" & {}{} --server {}",
            quote_windows_arg(program)?,
            if log_json { " --log-json -" } else { "" },
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

/// Whether the remote server should be asked to emit structured records, and
/// whether relayed records should also be echoed to this process's stdout.
///
/// Process-global rather than threaded through every entry point because it is
/// a whole-run output preference, not a property of one transfer.
static REMOTE_JSON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

const REMOTE_JSON_REQUESTED: u8 = 1;
const REMOTE_JSON_ECHO_STDOUT: u8 = 2;

/// Ask the remote server for structured records, and say whether relayed
/// records should also go to this process's stdout event stream.
pub fn configure_remote_logging(requested: bool, echo_stdout: bool) {
    let mut bits = 0;
    if requested {
        bits |= REMOTE_JSON_REQUESTED;
    }
    if echo_stdout {
        bits |= REMOTE_JSON_ECHO_STDOUT;
    }
    REMOTE_JSON.store(bits, std::sync::atomic::Ordering::Relaxed);
}

fn remote_json_requested() -> bool {
    REMOTE_JSON.load(std::sync::atomic::Ordering::Relaxed) & REMOTE_JSON_REQUESTED != 0
}

fn remote_json_echo_stdout() -> bool {
    REMOTE_JSON.load(std::sync::atomic::Ordering::Relaxed) & REMOTE_JSON_ECHO_STDOUT != 0
}

/// Per-host opt-out, set when a remote rejected `--log-json`.
///
/// An older remote's argument parser refuses an unknown flag outright, so the
/// request is dropped for that host and the session retried rather than failing
/// a transfer over a logging preference.
fn remote_json_unsupported() -> &'static std::sync::Mutex<HashSet<String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

fn remote_accepts_log_json(rsh: Option<&str>, host: &str) -> bool {
    remote_json_requested()
        && remote_json_unsupported()
            .lock()
            .is_ok_and(|set| !set.contains(&remote_shell_key(rsh, host)))
}

fn remember_log_json_rejected(rsh: Option<&str>, host: &str) {
    if let Ok(mut set) = remote_json_unsupported().lock() {
        set.insert(remote_shell_key(rsh, host));
    }
}

/// Remote program to invoke, when bootstrap has uploaded one for this host.
///
/// Kept alongside the shell cache rather than threaded through every sync
/// entry point, because it is the same shape of fact: something learned about
/// one host that every later spawn in this process should reuse.
fn remote_program_cache() -> &'static std::sync::Mutex<HashMap<String, String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// The program name the remote command should invoke: `xs` from the remote's
/// own PATH unless bootstrap has placed one at a known path.
fn remote_program_for(rsh: Option<&str>, host: &str) -> Option<String> {
    remote_program_cache()
        .lock()
        .ok()
        .and_then(|map| map.get(&remote_shell_key(rsh, host)).cloned())
}

/// The path bootstrap uploaded for `host`, if any. The CLI uses this to remove
/// an ephemeral binary once the transfer is done.
#[must_use]
pub fn bootstrapped_program(rsh: Option<&str>, host: &str) -> Option<String> {
    remote_program_for(rsh, host)
}

/// Record the path bootstrap uploaded, so this and later sessions use it.
pub fn remember_remote_program(rsh: Option<&str>, host: &str, program: String) {
    if let Ok(mut map) = remote_program_cache().lock() {
        map.insert(remote_shell_key(rsh, host), program);
    }
}

fn remember_remote_shell(rsh: Option<&str>, host: &str, shell: RemoteShell) {
    if let Ok(mut map) = remote_shell_cache().lock() {
        map.insert(remote_shell_key(rsh, host), shell);
    }
}

/// `(program, args)` that runs one command string on `host`, without deciding
/// what that string should be. Shared by the shell probe and the server launch
/// so they can never disagree about how the remote shell is reached.
pub(crate) fn base_remote_invocation(
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

/// Whether a relayed line is one of the remote's structured records.
///
/// Deliberately shallow: a full parse would reject a record from a newer remote
/// carrying a field this build does not know, and relaying it verbatim is
/// exactly the right behaviour there.
fn is_json_object(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('{') && line.ends_with('}')
}

fn is_missing_xsync_stderr(stderr: &str, exit_code: Option<i32>) -> bool {
    let lower = stderr.to_lowercase();
    // 9009 is cmd.exe's "not recognized as an internal or external command".
    // It is matched on the code alone because cmd localises the message, so
    // matching text would only work on English installs.
    if exit_code == Some(9009) {
        return true;
    }
    lower.contains("xs: command not found")
        || lower.contains("xs: not found")
        || (exit_code == Some(127) && lower.contains("xs"))
}

/// The remote shell family learned for `host`, for callers that need to build
/// their own remote commands — bootstrap, which must upload and hash a file
/// using whichever shell the remote actually runs.
#[must_use]
pub fn learned_remote_shell(rsh: Option<&str>, host: &str) -> RemoteShell {
    remote_shell_for(rsh, host)
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
    let program = host.and_then(|h| remote_program_for(rsh, h));
    let program = program.as_deref();
    let log_json = host.is_some_and(|h| remote_accepts_log_json(rsh, h));
    match host {
        Some(h) => Ok(base_remote_invocation(
            rsh,
            h,
            &xsync_remote_command(remote_path, shell, program, log_json)?,
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

/// Determine the remote shell family before a multi-stream session spawns anything.
///
/// Only the single-stream path discovers this, by attempting the POSIX form and
/// retrying as `RemoteShell::Windows` when the remote turns out to be cmd.exe.
/// It caches the answer with `remember_remote_shell`, and later
/// `spawn_server_child` calls pick it up. A fresh process that goes straight to
/// `--streams N` never runs that discovery: it assumes POSIX, cmd.exe cannot
/// parse the single-quoted command, the child exits, and the transfer dies with
/// "server stream disconnected" rather than anything naming the cause.
///
/// The probe runs a harmless marker command, never the server itself. An earlier
/// version spawned a real `xs --server` against the destination and killed it,
/// which left the destination's lock and journal state behind and broke the
/// Linux path that had been working.
fn ensure_remote_shell_known(rsh: Option<&str>, host: Option<&str>) {
    let Some(host_name) = host else {
        return;
    };
    if remote_shell_cache()
        .lock()
        .ok()
        .is_some_and(|map| map.contains_key(&remote_shell_key(rsh, host_name)))
    {
        return;
    }
    // POSIX is tried first because its marker command cannot run under cmd.exe,
    // while the cmd form would also succeed under a POSIX shell and so cannot
    // discriminate on its own.
    for (shell, command) in [
        (RemoteShell::Posix, "printf 'XSYNCSHELLOK\n'"),
        (RemoteShell::Windows, "echo XSYNCSHELLOK"),
    ] {
        let (program, args) = base_remote_invocation(rsh, host_name, command);
        let Ok(output) = Command::new(program).args(args).output() else {
            continue;
        };
        if String::from_utf8_lossy(&output.stdout).contains("XSYNCSHELLOK") {
            remember_remote_shell(rsh, host_name, shell);
            return;
        }
    }
    // Neither form answered. Leave the cache alone and let the session report
    // the real failure rather than masking it here.
}

/// How many SSH connections may be authenticating at the same time.
///
/// OpenSSH's default `MaxStartups` is `10:30:100`: past ten concurrent
/// *unauthenticated* connections it starts refusing at random. `--streams 16`
/// opens seventeen at once counting the control session, which failed about
/// two runs in three with a disconnected stream. Every stream still runs; they
/// just do not all authenticate simultaneously.
const MAX_CONCURRENT_CONNECTIONS: usize = 8;

/// Bounds how many peer connections are being established concurrently.
struct ConnectionGate {
    available: std::sync::Mutex<usize>,
    released: std::sync::Condvar,
}

impl ConnectionGate {
    fn new(permits: usize) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            available: std::sync::Mutex::new(permits.max(1)),
            released: std::sync::Condvar::new(),
        })
    }

    /// Block until a connection slot is free, yielding a guard that frees it.
    fn acquire(gate: &std::sync::Arc<Self>) -> ConnectionPermit {
        let mut available = gate
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *available == 0 {
            available = gate
                .released
                .wait(available)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *available -= 1;
        ConnectionPermit {
            gate: std::sync::Arc::clone(gate),
        }
    }
}

/// Frees its connection slot when dropped, which the data path does as soon as
/// the peer's handshake proves authentication finished — not when the transfer
/// finishes, which would cap concurrency for the whole run.
struct ConnectionPermit {
    gate: std::sync::Arc<ConnectionGate>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut available = self
            .gate
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *available += 1;
        self.gate.released.notify_one();
    }
}

/// One received small file, ready to publish.
struct ApplyJob {
    message_id: u64,
    entry: FileEntry,
    hash: blake3::Hash,
    data: Vec<u8>,
}

/// Publishes received files across a pool, preserving the ack-on-commit
/// contract: a file is acknowledged only once it is durably renamed into place.
///
/// Acks may leave in a different order than the segments arrived. That is
/// already safe — the sender's `drain_acks` counts acknowledgements and never
/// matches them to ids.
struct ApplyPool {
    work: crossbeam_channel::Sender<ApplyJob>,
    done: crossbeam_channel::Receiver<Result<u64, ServerError>>,
    handles: Vec<std::thread::JoinHandle<()>>,
    in_flight: usize,
    capacity: usize,
}

impl ApplyPool {
    fn new(sink: &Arc<Sink>, paranoid: bool, workers: usize) -> Self {
        let (work_tx, work_rx) = crossbeam_channel::bounded::<ApplyJob>(workers * 4);
        let (done_tx, done_rx) = crossbeam_channel::unbounded();
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let rx = work_rx.clone();
            let tx = done_tx.clone();
            let sink = Arc::clone(sink);
            handles.push(std::thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    let outcome = publish_received_file(&sink, &job, paranoid);
                    if tx.send(outcome.map(|()| job.message_id)).is_err() {
                        break;
                    }
                }
            }));
        }
        Self {
            work: work_tx,
            done: done_rx,
            handles,
            in_flight: 0,
            capacity: workers * APPLY_JOBS_PER_WORKER,
        }
    }

    /// Hand a file to the pool, blocking if the queue is full.
    fn submit(&mut self, job: ApplyJob) -> Result<(), ServerError> {
        self.work.send(job).map_err(|_| {
            ServerError::UnexpectedMessage("apply worker pool stopped early".to_owned())
        })?;
        self.in_flight += 1;
        Ok(())
    }

    /// Collect finished files, blocking only when too many are outstanding.
    fn collect<F>(&mut self, block_until: usize, mut ack: F) -> Result<(), ServerError>
    where
        F: FnMut(u64) -> Result<(), ServerError>,
    {
        while self.in_flight > block_until {
            let done = self.done.recv().map_err(|_| {
                ServerError::UnexpectedMessage("apply worker pool stopped early".to_owned())
            })?;
            self.in_flight -= 1;
            ack(done?)?;
        }
        while let Ok(done) = self.done.try_recv() {
            self.in_flight -= 1;
            ack(done?)?;
        }
        Ok(())
    }

    /// Bound on outstanding work before `collect` blocks.
    const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Drain everything and stop the workers.
    fn finish<F>(mut self, ack: F) -> Result<(), ServerError>
    where
        F: FnMut(u64) -> Result<(), ServerError>,
    {
        self.collect(0, ack)?;
        drop(self.work);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
        Ok(())
    }
}

/// Write, verify and publish one received file.
fn publish_received_file(sink: &Sink, job: &ApplyJob, paranoid: bool) -> Result<(), ServerError> {
    sink.write_file_with_retry(&job.entry, &job.hash, |_attempt| Ok(job.data.clone()))?;
    if paranoid {
        let committed = sink.path_for(&job.entry.path)?;
        let readback = fs::read(&committed)?;
        if blake3::hash(&readback) != job.hash {
            return Err(ServerError::Sink(SinkError::VerificationFailed {
                path: job.entry.path.to_string(),
                attempts: 2,
            }));
        }
    }
    Ok(())
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

fn run_server_child_session<F>(
    child: Child,
    shell: RemoteShell,
    default_ssh: bool,
    f: F,
) -> Result<LocalSyncReport, ServerError>
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
    let mut writer = BufWriter::with_capacity(TRANSPORT_WRITE_BUFFER, stdin);
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
    //
    // A remote asked for structured records emits one JSON object per line.
    // Those are routed to the failure log and, when the client is emitting a
    // JSON event stream, echoed to stdout. Anything else -- ssh's own messages,
    // and everything an older remote produces -- is relayed as text exactly as
    // before, so this degrades cleanly rather than swallowing diagnostics it
    // does not recognise.
    for line in stderr_text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if is_json_object(line) {
            crate::faillog::write_raw(line);
            if remote_json_echo_stdout() {
                println!("{line}");
            }
        } else {
            eprintln!("{line}");
        }
    }

    let exit_code = status.and_then(|s| s.code());

    if is_missing_xsync_stderr(&stderr_text, exit_code) {
        return Err(ServerError::MissingRemoteXsync);
    }

    let silent = bytes_from_remote.load(std::sync::atomic::Ordering::Relaxed) == 0;

    // Nothing on the far end spoke the protocol, and ssh did not report one of
    // its own failures (255: authentication, host key, connection). ssh's code
    // is excluded so a rejected credential still costs exactly one connection.
    if result.is_err() && silent && exit_code != Some(255) {
        // clap exits 2 on an unrecognised argument: an older remote that does
        // not know a flag this build sends.
        if exit_code == Some(2) {
            return Err(ServerError::RemoteFlagRejected);
        }
        // Exit 0 with no output is the POSIX command reaching stock Windows
        // cmd.exe, where `PATH` is a builtin: the whole line is read as a `PATH`
        // invocation that sets the search path to that literal text and
        // succeeds, running nothing.
        //
        // A *non-zero* exit under the POSIX form means cmd tried to parse the
        // line and refused it. That happens whenever the destination path
        // contains a cmd metacharacter -- `C:/a<>b` makes cmd read `>` as
        // redirection and fail with "was unexpected at this time" long before
        // the PATH builtin swallows anything. Treating only the silent-success
        // case as a shell mismatch made detection depend on the destination
        // path happening to be benign under a shell we had not identified yet.
        //
        // Restricted to the default ssh transport: an explicit `-e` is a
        // transport the caller chose, not a stock Windows sshd, and a server
        // killed mid-transfer through such a helper also exits non-zero with no
        // bytes read. Retrying there would re-run a session whose partial work
        // and resume journal must be preserved.
        if exit_code == Some(0) || (default_ssh && shell == RemoteShell::Posix) {
            return Err(ServerError::RemoteShellMismatch);
        }
        if shell == RemoteShell::Windows {
            return Err(ServerError::MissingRemoteXsync);
        }
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
    fn include_filter() -> crate::filter::FilterSet {
        crate::filter::FilterSet::from_rules(vec![
            crate::filter::Rule::new(
                crate::filter::Action::Include,
                "keep/**",
                crate::filter::Origin::CommandLine,
            )
            .unwrap(),
            crate::filter::Rule::new(
                crate::filter::Action::Exclude,
                "*",
                crate::filter::Origin::CommandLine,
            )
            .unwrap(),
        ])
    }

    /// A peer that advertises the capability receives the ordered set, and the
    /// flat exclude list is left empty so the receiver cannot apply both.
    #[test]
    fn a_capable_peer_receives_the_ordered_rule_set() {
        let options = LocalSyncOptions {
            filter: Some(include_filter()),
            ..LocalSyncOptions::default()
        };
        let wire = filter_for_peer(&options, CAP_FILTER_RULES).unwrap();
        assert!(wire.exclude_patterns.is_empty());
        assert_eq!(wire.rules, vec![b"+ keep/**".to_vec(), b"- *".to_vec()]);
    }

    /// Sending the excludes alone would transfer a wider set than asked for, so
    /// an older peer is refused instead of approximated at.
    #[test]
    fn an_incapable_peer_is_refused_rather_than_sent_the_excludes_alone() {
        let options = LocalSyncOptions {
            filter: Some(include_filter()),
            ..LocalSyncOptions::default()
        };
        let error = filter_for_peer(&options, CAP_ZSTD).unwrap_err();
        assert_eq!(error.kind(), "filter-unrepresentable");
        assert!(error.to_string().contains("--include"), "{error}");
    }

    /// An exclude-only filter has always fit the flat list, so an older peer
    /// keeps working exactly as before.
    #[test]
    fn an_incapable_peer_still_accepts_an_exclude_only_filter() {
        let options = LocalSyncOptions {
            filter: Some(crate::filter::from_exclude_patterns(&["*.tmp".to_owned()]).unwrap()),
            exclude_patterns: vec!["*.tmp".to_owned()],
            ..LocalSyncOptions::default()
        };
        let wire = filter_for_peer(&options, CAP_ZSTD).unwrap();
        assert!(wire.rules.is_empty());
        assert_eq!(wire.exclude_patterns, vec![b"*.tmp".to_vec()]);
    }

    /// Rules that arrive unparseable are an error, never an approximation.
    #[test]
    fn unusable_rules_from_a_peer_are_refused() {
        let error = filter_from_wire(&[], &[b"keep/** with no sigil".to_vec()]).unwrap_err();
        assert_eq!(error.kind(), "filter-unrepresentable");
    }

    /// The order of the rules is what carries their meaning across the wire.
    #[test]
    fn rules_survive_the_wire_with_their_order_intact() {
        let filter = include_filter();
        let wire = filter_for_peer(
            &LocalSyncOptions {
                filter: Some(filter),
                ..LocalSyncOptions::default()
            },
            CAP_FILTER_RULES,
        )
        .unwrap();
        let rebuilt = filter_from_wire(&[], &wire.rules).unwrap();
        assert!(rebuilt.decide("keep/a.txt").is_included());
        assert!(!rebuilt.decide("other/a.txt").is_included());
    }

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

    fn fs_session_handshake(capabilities: u32) -> Message {
        Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [3; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        }
    }

    /// Client bytes: the v1 opening handshake, then v3 frames.
    fn fs_client_input(client_capabilities: u32, frames: &[(u64, V3Message)]) -> Vec<u8> {
        let mut input = encode_frame(1, &fs_session_handshake(client_capabilities)).unwrap();
        for (id, message) in frames {
            input.extend_from_slice(&protocol_v3::encode_frame(*id, message).unwrap());
        }
        input
    }

    /// Split the server's output into its v1 handshake pair and the v3 frames.
    fn fs_server_replies(output: Vec<u8>) -> (u32, Vec<V3Frame>) {
        let mut decoder = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        let advertised = match decoder.read(&mut cursor).unwrap().message {
            Message::Handshake { capabilities, .. } => capabilities,
            other => panic!("expected server handshake, got {other:?}"),
        };
        assert!(matches!(
            decoder.read(&mut cursor).unwrap().message,
            Message::Ack { .. }
        ));
        let position = cursor.position();
        let mut cursor = Cursor::new(cursor.into_inner());
        cursor.set_position(position);
        let mut frames = Vec::new();
        while let Some(frame) = protocol_v3::read_frame(&mut cursor).unwrap() {
            frames.push(frame);
        }
        (advertised, frames)
    }

    /// Peer bytes for a client-side probe, optionally answering the feature
    /// exchange with `granted`.
    fn fs_peer_bytes(peer_capabilities: u32, granted: Option<u64>) -> Vec<u8> {
        let mut bytes = encode_frame(100, &fs_session_handshake(peer_capabilities)).unwrap();
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
        if let Some(features) = granted {
            bytes.extend_from_slice(
                &protocol_v3::encode_frame(
                    102,
                    &V3Message::FeaturesAck {
                        related_id: 2,
                        features,
                    },
                )
                .unwrap(),
            );
        }
        bytes
    }

    const FS_V3_CLIENT: u32 = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION | CAP_FS_V3;
    const FS_LOCKS: u64 = crate::protocol_v3::features::LOCKS;
    const FS_NOTIFY: u64 = crate::protocol_v3::features::NOTIFY;

    #[test]
    fn fs_session_negotiates_features_then_serves_control_verbs() {
        let input = fs_client_input(
            FS_V3_CLIENT,
            &[
                (
                    2,
                    V3Message::Features {
                        features: FS_LOCKS | FS_NOTIFY,
                    },
                ),
                (3, V3Message::Keepalive { nonce: 7 }),
                (4, fs_mount()),
                (5, V3Message::StatFs),
            ],
        );

        let mut output = Vec::new();
        let root = tempdir().unwrap();
        Server::new(root.path())
            .with_fs_features(FS_LOCKS)
            .run(Cursor::new(input), &mut output)
            .unwrap();

        let (advertised, replies) = fs_server_replies(output);
        assert!(advertised & CAP_FS_V3 != 0);
        assert_eq!(replies.len(), 4);
        // The granted set is the intersection: the client asked for notify too.
        assert_eq!(
            replies[0].message,
            V3Message::FeaturesAck {
                related_id: 2,
                features: FS_LOCKS,
            }
        );
        assert_eq!(replies[1].message, V3Message::KeepaliveAck { nonce: 7 });
        assert!(matches!(replies[2].message, V3Message::MountInfo { .. }));
        // A verb whose handler has not landed yet is a per-request error, not
        // a dead session.
        match &replies[3].message {
            V3Message::Error {
                related_id, code, ..
            } => {
                assert_eq!(*related_id, 5);
                assert_eq!(*code, FsErrorCode::NotSupported);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn version_selection_covers_every_client_server_pair() {
        let root = tempdir().unwrap();

        // v3 client, v2 server: the server never advertises CAP_FS_V3, both
        // select v2, and browse still works.
        let mut output = Vec::new();
        let mut input = encode_frame(1, &fs_session_handshake(FS_V3_CLIENT)).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(2, &V2Message::Keepalive { nonce: 1 }).unwrap(),
        );
        Server::new_with_capabilities(root.path(), CAP_VERSION_NEGOTIATION | CAP_BROWSE_META)
            .run(Cursor::new(input), &mut output)
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        match decoder.read(&mut cursor).unwrap().message {
            Message::Handshake { capabilities, .. } => {
                assert_eq!(capabilities & CAP_FS_V3, 0);
                assert!(capabilities & CAP_BROWSE_V2 != 0);
            }
            other => panic!("expected handshake, got {other:?}"),
        }

        // v2 client, v3 server: the server advertises v3 but the client does
        // not, so v2 is selected and no v3 frame is ever emitted.
        let mut output = Vec::new();
        let mut input = encode_frame(
            1,
            &fs_session_handshake(CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION),
        )
        .unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(2, &V2Message::Keepalive { nonce: 5 }).unwrap(),
        );
        Server::new(root.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        assert!(matches!(
            decoder.read(&mut cursor).unwrap().message,
            Message::Handshake { .. }
        ));
        assert!(matches!(
            decoder.read(&mut cursor).unwrap().message,
            Message::Ack { .. }
        ));
        let position = cursor.position();
        let mut cursor = Cursor::new(cursor.into_inner());
        cursor.set_position(position);
        assert_eq!(
            protocol_v2::read_frame(&mut cursor)
                .unwrap()
                .unwrap()
                .message,
            V2Message::KeepaliveAck { nonce: 5 }
        );

        // A session peer with neither browse nor fs bits cannot be served.
        let input = encode_frame(1, &fs_session_handshake(CAP_VERSION_NEGOTIATION)).unwrap();
        let error = Server::new(root.path())
            .run(Cursor::new(input), &mut Vec::new())
            .unwrap_err();
        assert!(error.to_string().contains("v2 or v3"), "{error}");
    }

    #[test]
    fn fs_probe_selects_v3_and_leaves_the_browse_probe_alone() {
        let v3_peer = FS_V3_CLIENT;
        let v2_peer = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;

        let ready = probe_fs_session(
            Cursor::new(fs_peer_bytes(v3_peer, None)),
            Vec::new(),
            [1; 16],
        )
        .unwrap();
        assert_eq!(ready.probe.status, ProbeStatus::ReadyV3);
        assert_eq!(ready.probe.selected_version, 3);
        assert_eq!(ready.probe.status.action(), "open the filesystem session");
        // Ready for v3 is not ready for browse: the grammars are exclusive.
        assert!(ready.into_browse_session().is_err());

        let degraded = probe_fs_session(
            Cursor::new(fs_peer_bytes(v2_peer, None)),
            Vec::new(),
            [2; 16],
        )
        .unwrap();
        assert_eq!(degraded.probe.status, ProbeStatus::Ready);
        assert_eq!(degraded.probe.selected_version, 2);
        assert!(degraded.into_fs_session(0).is_err());

        // The regression guard for existing browse consumers: the v2 probe
        // must not start selecting v3 just because the server was upgraded.
        let unchanged = probe_session(
            Cursor::new(fs_peer_bytes(v3_peer, None)),
            Vec::new(),
            [3; 16],
        )
        .unwrap();
        assert_eq!(unchanged.probe.status, ProbeStatus::Ready);
        assert_eq!(unchanged.probe.selected_version, 2);
        assert!(unchanged.into_browse_session().is_ok());
    }

    #[test]
    fn fs_client_keeps_only_the_negotiated_feature_set() {
        let mut session = FsSession::connect(
            Cursor::new(fs_peer_bytes(FS_V3_CLIENT, Some(FS_LOCKS))),
            Vec::new(),
            [4; 16],
            FS_LOCKS | FS_NOTIFY,
        )
        .unwrap();
        assert_eq!(session.negotiated_features(), FS_LOCKS);
        assert!(session.supports(FS_LOCKS));
        assert!(!session.supports(FS_NOTIFY));
        assert!(!session.supports(FS_LOCKS | FS_NOTIFY));
        session.require(FS_LOCKS, "locks").unwrap();
        let error = session.require(FS_NOTIFY, "notify").unwrap_err();
        assert!(error.to_string().contains("notify"), "{error}");
        assert!(session.keepalive(1).is_err(), "peer sent no keepalive ack");

        // A server may only narrow the request; granting something unasked-for
        // means one side is not speaking this contract.
        let Err(error) = FsSession::connect(
            Cursor::new(fs_peer_bytes(FS_V3_CLIENT, Some(FS_NOTIFY))),
            Vec::new(),
            [5; 16],
            FS_LOCKS,
        ) else {
            panic!("accepted a feature the client did not request")
        };
        assert!(error.to_string().contains("did not request"), "{error}");
    }

    /// A handler that parks every request until `target` of them are running
    /// at once. A serial dispatcher can never release it, so the timeout is a
    /// clean failure rather than a hang.
    struct ConcurrencyProbe {
        target: usize,
        inside: Mutex<usize>,
        signal: std::sync::Condvar,
    }

    impl ConcurrencyProbe {
        fn new(target: usize) -> Arc<Self> {
            Arc::new(Self {
                target,
                inside: Mutex::new(0),
                signal: std::sync::Condvar::new(),
            })
        }
    }

    impl FsHandler for ConcurrencyProbe {
        fn handle(
            &self,
            _state: &FsSessionState,
            related_id: u64,
            _request: V3Message,
        ) -> V3Message {
            let mut inside = self.inside.lock().unwrap();
            *inside += 1;
            if *inside >= self.target {
                self.signal.notify_all();
                return V3Message::Done { related_id };
            }
            let (_guard, wait) = self
                .signal
                .wait_timeout_while(inside, std::time::Duration::from_secs(10), |inside| {
                    *inside < self.target
                })
                .unwrap();
            if wait.timed_out() {
                fs_error(
                    related_id,
                    FsErrorCode::TimedOut,
                    "requests never ran concurrently",
                )
            } else {
                V3Message::Done { related_id }
            }
        }
    }

    fn fs_stat(path: &[u8]) -> V3Message {
        V3Message::Stat {
            target: StatTarget::Path(path.to_vec()),
            follow: true,
            attr_mask: 0,
        }
    }

    fn fs_mount() -> V3Message {
        V3Message::Mount {
            export: Vec::new(),
            requested_access: protocol_v3::Access::ReadWrite,
        }
    }

    /// Run `requests` against `server`, returning the v3 responses in the order
    /// the server wrote them, with the `FeaturesAck` and `MountInfo` that open
    /// every session dropped.
    fn fs_run_server(mut server: Server, requests: &[(u64, V3Message)]) -> Vec<V3Frame> {
        // Id 1 for the mount: ids need only be unique, and tests own every id
        // from 3 up, including the thousands the leak test uses.
        let mut frames = vec![(2, V3Message::Features { features: 0 }), (1, fs_mount())];
        frames.extend(requests.iter().cloned());
        let mut output = Vec::new();
        server
            .run(
                Cursor::new(fs_client_input(FS_V3_CLIENT, &frames)),
                &mut output,
            )
            .unwrap();
        let (_, replies) = fs_server_replies(output);
        assert!(
            matches!(replies[1].message, V3Message::MountInfo { .. }),
            "session did not mount: {:?}",
            replies[1].message
        );
        replies[2..].to_vec()
    }

    /// As [`fs_run_server`], with the pooled handler replaced for the test.
    fn fs_run_with_handler(
        handler: Arc<dyn FsHandler>,
        limits: (usize, usize),
        requests: &[(u64, V3Message)],
    ) -> Vec<V3Frame> {
        // Held until `fs_run_server` returns: the mount probes this root, so
        // it has to still exist.
        let root = tempdir().unwrap();
        let mut server = Server::new(root.path()).with_fs_limits(limits.0, limits.1);
        server.fs_handler = handler;
        fs_run_server(server, requests)
    }

    /// Open a session, mount, and return the `MountInfo` verbatim.
    fn fs_mount_facts(mut server: Server, request: V3Message) -> V3Message {
        let frames = [(2, V3Message::Features { features: 0 }), (1000, request)];
        let mut output = Vec::new();
        server
            .run(
                Cursor::new(fs_client_input(FS_V3_CLIENT, &frames)),
                &mut output,
            )
            .unwrap();
        let (_, replies) = fs_server_replies(output);
        replies[1].message.clone()
    }

    #[test]
    fn fs_mount_reports_a_writable_export() {
        let root = tempdir().unwrap();
        let facts = fs_mount_facts(Server::new(root.path()), fs_mount());
        let V3Message::MountInfo {
            related_id,
            access,
            effective_writable,
            reason,
            max_read,
            max_write,
            max_name_len,
            max_path_len,
            case_sensitive,
            supports,
            ..
        } = facts
        else {
            panic!("expected MountInfo, got {facts:?}")
        };
        assert_eq!(related_id, 1000);
        assert_eq!(access, protocol_v3::Access::ReadWrite);
        assert!(effective_writable, "a fresh temp dir must be writable");
        // The contract ties these together: a reason exists only when a write
        // is refused.
        assert!(reason.is_empty());
        assert_eq!(max_read, DEFAULT_FS_MAX_TRANSFER);
        assert_eq!(max_write, DEFAULT_FS_MAX_TRANSFER);
        assert!(max_name_len > 0 && max_path_len > 0);
        // Case sensitivity is a property of whichever filesystem the temp dir
        // landed on, so assert only that it agrees with the probe rather than
        // hard-coding this machine's answer.
        let probed = crate::pathsem::PathSemantics::probe(root.path());
        assert_eq!(case_sensitive, !probed.case_insensitive);
        assert_eq!(
            supports & protocol_v3::supports::CASE_INSENSITIVE != 0,
            probed.case_insensitive
        );
    }

    #[test]
    fn fs_mount_reports_a_read_only_export_with_its_reason() {
        let root = tempdir().unwrap();
        let facts = fs_mount_facts(Server::new(root.path()).read_only(true), fs_mount());
        let V3Message::MountInfo {
            access,
            effective_writable,
            reason,
            ..
        } = facts
        else {
            panic!("expected MountInfo, got {facts:?}")
        };
        assert_eq!(access, protocol_v3::Access::ReadOnly);
        assert!(!effective_writable);
        assert_eq!(reason, b"export is read-only");
    }

    #[test]
    fn fs_mount_lets_a_client_ask_for_less_than_the_export_allows() {
        // The export is writable; the client asked not to be able to write, so
        // the mount is read-only and says why.
        let root = tempdir().unwrap();
        let facts = fs_mount_facts(
            Server::new(root.path()),
            V3Message::Mount {
                export: Vec::new(),
                requested_access: protocol_v3::Access::ReadOnly,
            },
        );
        let V3Message::MountInfo {
            access,
            effective_writable,
            reason,
            ..
        } = facts
        else {
            panic!("expected MountInfo, got {facts:?}")
        };
        // `access` is what the *export* grants; `effective_writable` is what
        // this session got.
        assert_eq!(access, protocol_v3::Access::ReadWrite);
        assert!(!effective_writable);
        assert_eq!(reason, b"mounted read-only at the client's request");
    }

    #[test]
    fn fs_mount_refuses_a_named_export_and_a_missing_root() {
        let root = tempdir().unwrap();
        let named = fs_mount_facts(
            Server::new(root.path()),
            V3Message::Mount {
                export: b"media".to_vec(),
                requested_access: protocol_v3::Access::ReadWrite,
            },
        );
        assert!(
            matches!(
                named,
                V3Message::Error {
                    code: FsErrorCode::NoEntry,
                    ..
                }
            ),
            "{named:?}"
        );

        let missing = fs_mount_facts(Server::new(root.path().join("gone")), fs_mount());
        assert!(
            matches!(
                missing,
                V3Message::Error {
                    code: FsErrorCode::NoEntry,
                    ..
                }
            ),
            "{missing:?}"
        );
    }

    /// Records every request that reached the pool.
    struct RecordingHandler {
        seen: Mutex<Vec<u8>>,
    }

    impl FsHandler for RecordingHandler {
        fn handle(
            &self,
            _state: &FsSessionState,
            related_id: u64,
            request: V3Message,
        ) -> V3Message {
            self.seen
                .lock()
                .unwrap()
                .push(protocol_v3::message_type(&request));
            V3Message::Done { related_id }
        }
    }

    #[test]
    fn fs_read_only_mount_refuses_writes_before_the_filesystem() {
        let root = tempdir().unwrap();
        let recorder = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
        });
        let mut server = Server::new(root.path()).read_only(true);
        server.fs_handler = Arc::clone(&recorder) as Arc<dyn FsHandler>;

        let replies = fs_run_server(
            server,
            &[
                (
                    3,
                    V3Message::Open {
                        path: b"new".to_vec(),
                        flags: protocol_v3::open_flags::WRITE | protocol_v3::open_flags::CREATE,
                        mode: 0o644,
                        attr_mask: 0,
                    },
                ),
                (
                    4,
                    V3Message::Write {
                        handle: 1,
                        offset: 0,
                        digest: None,
                        data: b"hi".to_vec(),
                    },
                ),
                (
                    5,
                    V3Message::Open {
                        path: b"existing".to_vec(),
                        flags: protocol_v3::open_flags::READ,
                        mode: 0,
                        attr_mask: 0,
                    },
                ),
            ],
        );

        assert_eq!(replies.len(), 3);
        for id in [3_u64, 4] {
            match fs_reply(&replies, id) {
                V3Message::Error { code, message, .. } => {
                    assert_eq!(*code, FsErrorCode::ReadOnly, "request {id}");
                    // The refusal carries the mount's own reason, so a client
                    // shows one explanation everywhere.
                    assert_eq!(message, b"export is read-only");
                }
                other => panic!("request {id}: expected EROFS, got {other:?}"),
            }
        }
        // The read-class open was not gated and did reach the pool; the two
        // write-class requests never did.
        assert_eq!(*fs_reply(&replies, 5), V3Message::Done { related_id: 5 });
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![protocol_v3::types::OPEN]
        );
    }

    #[test]
    fn fs_session_requires_exactly_one_mount_before_any_verb() {
        let root = tempdir().unwrap();
        let mut output = Vec::new();
        let frames = [
            (2, V3Message::Features { features: 0 }),
            // Before any mount.
            (3, V3Message::StatFs),
            (4, fs_mount()),
            // A second mount on a mounted session.
            (5, fs_mount()),
        ];
        Server::new(root.path())
            .run(
                Cursor::new(fs_client_input(FS_V3_CLIENT, &frames)),
                &mut output,
            )
            .unwrap();
        let (_, replies) = fs_server_replies(output);

        assert!(matches!(
            replies[1].message,
            V3Message::Error {
                related_id: 3,
                code: FsErrorCode::Invalid,
                ..
            }
        ));
        assert!(matches!(
            replies[2].message,
            V3Message::MountInfo { related_id: 4, .. }
        ));
        assert!(matches!(
            replies[3].message,
            V3Message::Error {
                related_id: 5,
                code: FsErrorCode::Invalid,
                ..
            }
        ));
    }

    fn fs_open(path: &[u8], flags: u32, attr_mask: u32) -> V3Message {
        V3Message::Open {
            path: path.to_vec(),
            flags,
            mode: 0,
            attr_mask,
        }
    }

    /// Run against a populated root, returning the replies after the mount.
    fn fs_run_in(root: &Path, requests: &[(u64, V3Message)]) -> Vec<V3Frame> {
        fs_run_server(Server::new(root), requests)
    }

    /// As [`fs_run_in`] with one worker, so pooled requests execute in send
    /// order. Only for tests whose *subject* is a sequence — the dispatcher
    /// gives no ordering between requests that do not name one handle.
    fn fs_run_serial(root: &Path, requests: &[(u64, V3Message)]) -> Vec<V3Frame> {
        fs_run_server(
            Server::new(root).with_fs_limits(requests.len().max(1) + 1, 1),
            requests,
        )
    }

    struct FsChannelWriter(crossbeam_channel::Sender<Vec<u8>>);
    impl Write for FsChannelWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0
                .send(buffer.to_vec())
                .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error))?;
            Ok(buffer.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FsChannelReader {
        rx: crossbeam_channel::Receiver<Vec<u8>>,
        buffer: Vec<u8>,
        position: usize,
    }
    impl FsChannelReader {
        const fn new(rx: crossbeam_channel::Receiver<Vec<u8>>) -> Self {
            Self {
                rx,
                buffer: Vec::new(),
                position: 0,
            }
        }
    }
    impl Read for FsChannelReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.position >= self.buffer.len() {
                let Ok(data) = self.rx.recv() else {
                    return Ok(0);
                };
                self.buffer = data;
                self.position = 0;
            }
            let available = &self.buffer[self.position..];
            let count = available.len().min(buffer.len());
            buffer[..count].copy_from_slice(&available[..count]);
            self.position += count;
            Ok(count)
        }
    }

    /// A live v3 session over in-process pipes.
    ///
    /// The scripted helpers feed a fixed byte stream, which cannot express
    /// "await this reply, then decide what to send" -- and a real client has no
    /// choice but to work that way, because it cannot name a handle until
    /// `Opened` tells it the number. Tests whose subject spans several round
    /// trips use this instead.
    struct FsLiveSession {
        to_server: Option<crossbeam_channel::Sender<Vec<u8>>>,
        reader: FsChannelReader,
        server: Option<thread::JoinHandle<Result<(), ServerError>>>,
        next_id: u64,
        /// Replies that arrived before the one being awaited. Responses are
        /// unordered, so this is normal rather than exceptional.
        pending: Vec<V3Frame>,
    }

    impl FsLiveSession {
        fn start(mut server: Server) -> Self {
            let (client_tx, server_rx) = crossbeam_channel::bounded::<Vec<u8>>(1024);
            let (server_tx, client_rx) = crossbeam_channel::bounded::<Vec<u8>>(1024);
            let thread = thread::spawn(move || {
                server.run(FsChannelReader::new(server_rx), FsChannelWriter(server_tx))
            });
            let mut session = Self {
                to_server: Some(client_tx),
                reader: FsChannelReader::new(client_rx),
                server: Some(thread),
                next_id: 2,
                pending: Vec::new(),
            };

            session.send_bytes(&encode_frame(1, &fs_session_handshake(FS_V3_CLIENT)).unwrap());
            let mut decoder = FrameDecoder::new();
            assert!(matches!(
                decoder.read(&mut session.reader).unwrap().message,
                Message::Handshake { .. }
            ));
            assert!(matches!(
                decoder.read(&mut session.reader).unwrap().message,
                Message::Ack { .. }
            ));

            let id = session.send(&V3Message::Features { features: 0 });
            assert!(matches!(
                session.await_reply(id),
                V3Message::FeaturesAck { .. }
            ));
            let id = session.send(&fs_mount());
            assert!(matches!(
                session.await_reply(id),
                V3Message::MountInfo { .. }
            ));
            session
        }

        fn send_bytes(&mut self, bytes: &[u8]) {
            self.to_server
                .as_ref()
                .expect("session is still open")
                .send(bytes.to_vec())
                .expect("server is still running");
        }

        /// Send one request without waiting for it, returning its id.
        fn send(&mut self, message: &V3Message) -> u64 {
            let id = self.next_id;
            self.next_id += 1;
            self.send_bytes(&protocol_v3::encode_frame(id, message).unwrap());
            id
        }

        fn await_reply(&mut self, related_id: u64) -> V3Message {
            if let Some(index) = self
                .pending
                .iter()
                .position(|frame| fs_related_id(&frame.message) == Some(related_id))
            {
                return self.pending.remove(index).message;
            }
            loop {
                let frame = protocol_v3::read_frame(&mut self.reader)
                    .expect("v3 decode")
                    .expect("server closed the session early");
                if fs_related_id(&frame.message) == Some(related_id) {
                    return frame.message;
                }
                self.pending.push(frame);
            }
        }

        fn request(&mut self, message: &V3Message) -> V3Message {
            let id = self.send(message);
            self.await_reply(id)
        }

        /// Open a path and return the handle the server assigned.
        fn open(&mut self, path: &[u8], flags: u32) -> u64 {
            match self.request(&fs_open(path, flags, 0)) {
                V3Message::Opened { handle, .. } => handle,
                other => panic!("open failed: {other:?}"),
            }
        }

        fn finish(mut self) {
            drop(self.to_server.take());
            self.server
                .take()
                .expect("joined once")
                .join()
                .expect("server thread panicked")
                .expect("session ended with an error");
        }
    }

    const fn fs_related_id(message: &V3Message) -> Option<u64> {
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
            _ => None,
        }
    }

    /// The reply to one request.
    ///
    /// Responses carry the id they answer precisely because they are not
    /// ordered. A test that indexes them positionally asserts something the
    /// dispatcher does not promise and fails whenever the pool happens to
    /// finish them in a different order.
    fn fs_reply(replies: &[V3Frame], related_id: u64) -> &V3Message {
        replies
            .iter()
            .map(|frame| &frame.message)
            .find(|message| fs_related_id(message) == Some(related_id))
            .unwrap_or_else(|| panic!("no reply to request {related_id} in {replies:?}"))
    }

    #[test]
    fn fs_open_returns_a_handle_and_the_attributes_of_the_file() {
        use protocol_v3::{attr_presence as presence, open_flags};

        let root = tempdir().unwrap();
        fs::write(root.path().join("file.txt"), b"hello").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (
                    3,
                    fs_open(
                        b"file.txt",
                        open_flags::READ,
                        presence::OWNER | presence::IDENTITY | presence::NLINK,
                    ),
                ),
                // The first handle of a session is 1; a test that needed to
                // discover it would need a live transport, not a fixed script.
                (4, V3Message::Close { handle: 1 }),
            ],
        );

        let V3Message::Opened {
            related_id,
            handle,
            attrs,
        } = fs_reply(&replies, 3)
        else {
            panic!("expected Opened, got {:?}", fs_reply(&replies, 3))
        };
        assert_eq!(*related_id, 3);
        // Zero is never a handle, so a client cannot mistake a default for one.
        assert_ne!(*handle, 0);
        assert_eq!(attrs.kind, 1);
        assert_eq!(attrs.size, 5);
        assert_ne!(attrs.change_cookie, [0; 16]);
        // Exactly the blocks the mask asked for, and no others.
        assert!(attrs.identity.is_some() && attrs.nlink.is_some());
        assert_eq!(attrs.atime_ns, None);
        assert_eq!(attrs.allocated_size, None);
        assert_eq!(attrs.names, None);
        if cfg!(unix) {
            assert!(attrs.owner.is_some());
        }
        assert_eq!(*fs_reply(&replies, 4), V3Message::Done { related_id: 4 });
    }

    #[test]
    fn fs_open_reports_a_changed_file_with_a_different_cookie() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        let path = root.path().join("file.txt");
        fs::write(&path, b"one").unwrap();
        let first = fs_run_in(
            root.path(),
            &[(3, fs_open(b"file.txt", open_flags::READ, 0))],
        );
        // Same length, different content: a cookie derived from size alone
        // would miss this.
        fs::write(&path, b"two").unwrap();
        let second = fs_run_in(
            root.path(),
            &[(3, fs_open(b"file.txt", open_flags::READ, 0))],
        );

        let cookie = |frame: &V3Frame| match &frame.message {
            V3Message::Opened { attrs, .. } => attrs.change_cookie,
            other => panic!("expected Opened, got {other:?}"),
        };
        assert_ne!(cookie(&first[0]), cookie(&second[0]));
    }

    #[test]
    fn fs_close_of_an_unknown_handle_is_this_requests_error() {
        let root = tempdir().unwrap();
        let replies = fs_run_in(
            root.path(),
            &[
                (3, V3Message::Close { handle: 4242 }),
                // The session is still healthy afterwards.
                (4, V3Message::Keepalive { nonce: 1 }),
            ],
        );
        // Searched rather than indexed: the keepalive is answered on the
        // session thread and legitimately overtakes the pooled close.
        assert!(
            replies.iter().any(|frame| matches!(
                frame.message,
                V3Message::Error {
                    related_id: 3,
                    code: FsErrorCode::BadHandle,
                    ..
                }
            )),
            "{replies:?}"
        );
        assert!(replies
            .iter()
            .any(|frame| frame.message == V3Message::KeepaliveAck { nonce: 1 }));
    }

    #[test]
    fn fs_open_maps_filesystem_failures_onto_the_frozen_codes() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("dir")).unwrap();
        fs::write(root.path().join("taken"), b"x").unwrap();

        let replies = fs_run_in(
            root.path(),
            &[
                (3, fs_open(b"missing", open_flags::READ, 0)),
                (
                    4,
                    fs_open(
                        b"taken",
                        open_flags::WRITE | open_flags::CREATE | open_flags::EXCL,
                        0,
                    ),
                ),
                // A directory without DIRECTORY yields a handle nothing could
                // use, so it is refused at open rather than at first read.
                (5, fs_open(b"dir", open_flags::READ, 0)),
                (
                    6,
                    fs_open(b"taken", open_flags::READ | open_flags::DIRECTORY, 0),
                ),
                (
                    7,
                    fs_open(b"dir", open_flags::READ | open_flags::DIRECTORY, 0),
                ),
            ],
        );

        for (id, expected) in [
            (3, FsErrorCode::NoEntry),
            (4, FsErrorCode::Exists),
            (5, FsErrorCode::IsDirectory),
            (6, FsErrorCode::NotDirectory),
        ] {
            match fs_reply(&replies, id) {
                V3Message::Error { code, .. } => assert_eq!(*code, expected, "request {id}"),
                other => panic!("request {id}: expected Error, got {other:?}"),
            }
        }
        assert!(matches!(fs_reply(&replies, 7), V3Message::Opened { .. }));
    }

    #[test]
    fn fs_open_confines_every_path_to_the_export() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"private").unwrap();
        fs::create_dir(root.path().join("inside")).unwrap();

        let mut requests = vec![
            // Rejected by the wire path type before any syscall.
            (3, fs_open(b"../secret", open_flags::READ, 0)),
            (4, fs_open(b"/etc/passwd", open_flags::READ, 0)),
            (5, fs_open(b"inside/../../secret", open_flags::READ, 0)),
        ];
        #[cfg(unix)]
        {
            // A symlink in a parent component may not redirect the open out of
            // the export, even though every component is individually legal.
            std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
            requests.push((6, fs_open(b"link/secret", open_flags::READ, 0)));
        }

        let replies = fs_run_in(root.path(), &requests);
        for frame in &replies {
            match &frame.message {
                V3Message::Error { code, .. } => assert!(
                    matches!(code, FsErrorCode::Invalid | FsErrorCode::Access),
                    "{frame:?}"
                ),
                other => panic!("path escaped the export: {other:?}"),
            }
        }
    }

    #[test]
    fn fs_open_is_bounded_by_the_session_handle_limit() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        for index in 0..4 {
            fs::write(root.path().join(format!("f{index}")), b"x").unwrap();
        }
        let requests: Vec<_> = (0..4)
            .map(|index| {
                (
                    index + 3,
                    fs_open(format!("f{index}").as_bytes(), open_flags::READ, 0),
                )
            })
            .collect();

        let replies = fs_run_server(Server::new(root.path()).with_fs_max_handles(2), &requests);
        let opened = replies
            .iter()
            .filter(|frame| matches!(frame.message, V3Message::Opened { .. }))
            .count();
        let limited = replies
            .iter()
            .filter(|frame| {
                matches!(
                    frame.message,
                    V3Message::Error {
                        code: FsErrorCode::Limit,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(opened, 2);
        assert_eq!(limited, 2);
    }

    #[test]
    fn fs_open_and_close_cycles_do_not_leak_descriptors() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();

        // Far more cycles than the process has descriptors: a handle that
        // outlived its Close would exhaust them and start failing EMFILE long
        // before the end. One session, so the table is the only thing keeping
        // the files open.
        let cycles = 10_000_u64;
        let mut requests = Vec::with_capacity(cycles as usize * 2);
        for index in 0..cycles {
            requests.push((index * 2 + 3, fs_open(b"file", open_flags::READ, 0)));
            requests.push((index * 2 + 4, V3Message::Close { handle: index + 1 }));
        }

        let replies = fs_run_server(
            // One worker, so each cycle's open and close run before the next
            // and the handle ids stay predictable. The whole script is fed at
            // once, so the in-flight bound has to admit all of it.
            Server::new(root.path()).with_fs_limits(requests.len() + 1, 1),
            &requests,
        );
        assert_eq!(replies.len() as u64, cycles * 2);
        for frame in &replies {
            assert!(
                matches!(
                    frame.message,
                    V3Message::Opened { .. } | V3Message::Done { .. }
                ),
                "cycle failed, likely a leaked descriptor: {frame:?}"
            );
        }
    }

    fn fs_read(handle: u64, offset: u64, length: u32, want_digest: bool) -> V3Message {
        V3Message::Read {
            handle,
            offset,
            length,
            want_digest,
        }
    }

    #[test]
    fn fs_read_returns_the_requested_range_and_marks_end_of_file() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"0123456789").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::READ, 0)),
                // A range in the middle: neither at the start nor reaching EOF.
                (4, fs_read(1, 2, 4, false)),
                // Asking past the end returns what exists and says so.
                (5, fs_read(1, 6, 100, false)),
                // Starting at the end returns nothing, and still says so.
                (6, fs_read(1, 10, 4, false)),
            ],
        );

        let read = |id: u64| match fs_reply(&replies, id) {
            V3Message::ReadData {
                offset, eof, data, ..
            } => (*offset, *eof, data.clone()),
            other => panic!("request {id}: expected ReadData, got {other:?}"),
        };
        assert_eq!(read(4), (2, false, b"2345".to_vec()));
        assert_eq!(read(5), (6, true, b"6789".to_vec()));
        assert_eq!(read(6), (10, true, Vec::new()));
    }

    #[test]
    fn fs_read_carries_a_digest_only_when_asked() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"hello").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::READ, 0)),
                (4, fs_read(1, 0, 5, true)),
                (5, fs_read(1, 0, 5, false)),
            ],
        );

        match fs_reply(&replies, 4) {
            V3Message::ReadData { digest, data, .. } => {
                assert_eq!(data, b"hello");
                // The digest covers exactly the bytes returned, so a client can
                // verify without knowing what it asked for.
                assert_eq!(*digest, Some(*blake3::hash(b"hello").as_bytes()));
            }
            other => panic!("expected ReadData, got {other:?}"),
        }
        match fs_reply(&replies, 5) {
            V3Message::ReadData { digest, .. } => assert_eq!(*digest, None),
            other => panic!("expected ReadData, got {other:?}"),
        }
    }

    #[test]
    fn fs_read_refuses_a_bad_handle_a_directory_and_an_oversized_length() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("dir")).unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (
                    3,
                    fs_open(b"dir", open_flags::READ | open_flags::DIRECTORY, 0),
                ),
                // Handle 1 is the directory.
                (4, fs_read(1, 0, 4, false)),
                (5, fs_read(999, 0, 4, false)),
                (6, fs_open(b"file", open_flags::WRITE, 0)),
                // Handle 2 was opened write-only.
                (7, fs_read(2, 0, 4, false)),
                (8, fs_open(b"file", open_flags::READ, 0)),
                // Handle 3 is readable, so the length is what is refused.
                (9, fs_read(3, 0, DEFAULT_FS_MAX_TRANSFER + 1, false)),
            ],
        );

        let code = |id: u64| match fs_reply(&replies, id) {
            V3Message::Error { code, .. } => *code,
            other => panic!("request {id}: expected Error, got {other:?}"),
        };
        assert_eq!(code(4), FsErrorCode::IsDirectory);
        assert_eq!(code(5), FsErrorCode::BadHandle);
        assert_eq!(code(7), FsErrorCode::Access);
        // Above what the mount advertised as max_read, even though the
        // envelope would carry it.
        assert_eq!(code(9), FsErrorCode::Invalid);
    }

    #[test]
    fn fs_reads_on_one_handle_run_concurrently() {
        use protocol_v3::open_flags;

        // Eight outstanding reads on one file is the streaming case, and the
        // point of letting reads share their handle's ordering domain. The
        // probe only releases when eight are inside it at once, so this fails
        // if reads on one handle are serialised.
        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), vec![7_u8; 4096]).unwrap();

        let mut requests = vec![(3, fs_open(b"file", open_flags::READ, 0))];
        for index in 0..8 {
            requests.push((index + 4, fs_read(1, index * 512, 512, false)));
        }

        let root_path = root.path().to_path_buf();
        let mut server = Server::new(&root_path).with_fs_limits(32, 8);
        // Opens must still complete, so only reads park in the probe.
        struct ReadProbe(Arc<ConcurrencyProbe>);
        impl FsHandler for ReadProbe {
            fn handle(
                &self,
                state: &FsSessionState,
                related_id: u64,
                request: V3Message,
            ) -> V3Message {
                if matches!(request, V3Message::Read { .. }) {
                    return self.0.handle(state, related_id, request);
                }
                ServerFsHandler.handle(state, related_id, request)
            }
        }
        server.fs_handler = Arc::new(ReadProbe(ConcurrencyProbe::new(8)));

        let replies = fs_run_server(server, &requests);
        for id in 4..12_u64 {
            assert_eq!(
                *fs_reply(&replies, id),
                V3Message::Done { related_id: id },
                "read {id} did not run concurrently with the others"
            );
        }
    }

    #[test]
    fn fs_a_write_class_request_still_waits_for_the_reads_before_it() {
        // Reads share the domain, but a Flush must not overtake them and must
        // not start while any is still running: that is the ordering guarantee
        // sharing is not allowed to weaken.
        struct OrderRecorder {
            seen: Mutex<Vec<&'static str>>,
            inside_reads: Mutex<usize>,
        }
        impl FsHandler for OrderRecorder {
            fn handle(
                &self,
                _state: &FsSessionState,
                related_id: u64,
                request: V3Message,
            ) -> V3Message {
                match request {
                    V3Message::Read { .. } => {
                        *self.inside_reads.lock().unwrap() += 1;
                        self.seen.lock().unwrap().push("read");
                        thread::sleep(std::time::Duration::from_millis(120));
                        *self.inside_reads.lock().unwrap() -= 1;
                        V3Message::Done { related_id }
                    }
                    V3Message::Flush { .. } => {
                        let overlapped = *self.inside_reads.lock().unwrap() > 0;
                        self.seen.lock().unwrap().push("flush");
                        if overlapped {
                            return fs_error(
                                related_id,
                                FsErrorCode::Busy,
                                "flush ran while a read was still in flight",
                            );
                        }
                        V3Message::Done { related_id }
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }

        let root = tempdir().unwrap();
        let recorder = Arc::new(OrderRecorder {
            seen: Mutex::new(Vec::new()),
            inside_reads: Mutex::new(0),
        });
        let mut server = Server::new(root.path()).with_fs_limits(16, 8);
        server.fs_handler = Arc::clone(&recorder) as Arc<dyn FsHandler>;

        let replies = fs_run_server(
            server,
            &[
                (3, fs_read(1, 0, 16, false)),
                (4, fs_read(1, 16, 16, false)),
                (5, V3Message::Flush { handle: 1 }),
                (6, fs_read(1, 32, 16, false)),
            ],
        );

        for id in [3_u64, 4, 5, 6] {
            assert_eq!(
                *fs_reply(&replies, id),
                V3Message::Done { related_id: id },
                "{:?}",
                fs_reply(&replies, id)
            );
        }
        // The flush sits between the two batches of reads, never inside one.
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec!["read", "read", "flush", "read"]
        );
    }

    fn fs_write(handle: u64, offset: u64, data: &[u8], digest: Option<[u8; 32]>) -> V3Message {
        V3Message::Write {
            handle,
            offset,
            digest,
            data: data.to_vec(),
        }
    }

    #[test]
    fn fs_write_lands_at_the_offset_and_reports_the_new_size() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"0123456789").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::READ | open_flags::WRITE, 0)),
                (4, fs_write(1, 2, b"AB", None)),
                // Past the end, so the file grows.
                (5, fs_write(1, 10, b"XY", None)),
                (6, fs_read(1, 0, 32, false)),
            ],
        );

        match fs_reply(&replies, 4) {
            V3Message::WriteAck {
                bytes_written,
                new_size,
                stable,
                ..
            } => {
                assert_eq!(*bytes_written, 2);
                assert_eq!(*new_size, 10, "an in-place write does not grow the file");
                // Write-through: Flush is the durability barrier until an
                // export can be configured `sync`.
                assert!(!*stable);
            }
            other => panic!("expected WriteAck, got {other:?}"),
        }
        match fs_reply(&replies, 5) {
            V3Message::WriteAck { new_size, .. } => assert_eq!(*new_size, 12),
            other => panic!("expected WriteAck, got {other:?}"),
        }
        match fs_reply(&replies, 6) {
            V3Message::ReadData { data, .. } => assert_eq!(data, b"01AB456789XY"),
            other => panic!("expected ReadData, got {other:?}"),
        }
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"01AB456789XY");
    }

    #[test]
    fn fs_write_verifies_its_digest_before_touching_the_file() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"original").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::WRITE, 0)),
                (4, fs_write(1, 0, b"corrupted", Some([0xab; 32]))),
                (
                    5,
                    fs_write(
                        1,
                        0,
                        b"REPLACED",
                        Some(*blake3::hash(b"REPLACED").as_bytes()),
                    ),
                ),
            ],
        );

        match fs_reply(&replies, 4) {
            V3Message::Error { code, .. } => assert_eq!(*code, FsErrorCode::Integrity),
            other => panic!("expected EINTEGRITY, got {other:?}"),
        }
        assert!(matches!(fs_reply(&replies, 5), V3Message::WriteAck { .. }));
        // The rejected write left nothing behind; the accepted one landed.
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"REPLACED");
    }

    #[test]
    fn fs_write_refuses_a_bad_handle_a_directory_and_a_read_only_handle() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("dir")).unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (
                    3,
                    fs_open(b"dir", open_flags::READ | open_flags::DIRECTORY, 0),
                ),
                (4, fs_open(b"file", open_flags::READ, 0)),
                (5, fs_write(999, 0, b"x", None)),
                (6, fs_write(1, 0, b"x", None)),
                (7, fs_write(2, 0, b"x", None)),
                (8, V3Message::Flush { handle: 1 }),
                (9, V3Message::Flush { handle: 2 }),
            ],
        );

        let code = |id: u64| match fs_reply(&replies, id) {
            V3Message::Error { code, .. } => *code,
            other => panic!("request {id}: expected Error, got {other:?}"),
        };
        assert_eq!(code(5), FsErrorCode::BadHandle);
        assert_eq!(code(6), FsErrorCode::IsDirectory);
        assert_eq!(code(7), FsErrorCode::Access);
        assert_eq!(code(8), FsErrorCode::IsDirectory);
        // Flushing a handle that was only read is harmless, not an error.
        assert_eq!(*fs_reply(&replies, 9), V3Message::Done { related_id: 9 });
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"x");
    }

    #[test]
    fn fs_append_handles_ignore_the_offset() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("log"), b"start:").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (
                    3,
                    fs_open(b"log", open_flags::WRITE | open_flags::APPEND, 0),
                ),
                // Offset 0 would overwrite the file if it were honoured.
                (4, fs_write(1, 0, b"one", None)),
                (5, fs_write(1, 0, b"two", None)),
            ],
        );

        for (id, expected_size) in [(4_u64, 9_u64), (5, 12)] {
            match fs_reply(&replies, id) {
                V3Message::WriteAck {
                    bytes_written,
                    new_size,
                    ..
                } => {
                    assert_eq!(*new_size, expected_size);
                    // WriteAck carries no offset field, so the offset an append
                    // landed at is `new_size - bytes_written`. Under the
                    // handle's exclusive domain that is exact.
                    assert_eq!(*new_size - u64::from(*bytes_written), expected_size - 3);
                }
                other => panic!("request {id}: expected WriteAck, got {other:?}"),
            }
        }
        assert_eq!(fs::read(root.path().join("log")).unwrap(), b"start:onetwo");
    }

    #[test]
    fn fs_interleaved_writes_on_two_handles_match_an_in_process_model() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        let path = root.path().join("file");
        fs::write(&path, vec![b'.'; 64]).unwrap();

        let mut session = FsLiveSession::start(Server::new(root.path()).with_fs_limits(64, 8));
        let first = session.open(b"file", open_flags::WRITE);
        let second = session.open(b"file", open_flags::WRITE);
        assert_ne!(first, second);

        // Two handles onto one file have separate ordering domains, so the
        // server may interleave these freely. All sixteen go out before any
        // reply is read, which is the pipelining the domains exist to allow.
        let mut model = vec![b'.'; 64];
        let mut ids = Vec::new();
        for index in 0..16_u64 {
            let handle = if index % 2 == 0 { first } else { second };
            let byte = b'a' + u8::try_from(index).unwrap();
            let offset = index * 4;
            ids.push(session.send(&fs_write(handle, offset, &[byte; 4], None)));
            model[offset as usize..offset as usize + 4].fill(byte);
        }
        for id in ids {
            assert!(
                matches!(session.await_reply(id), V3Message::WriteAck { .. }),
                "write {id} failed"
            );
        }
        session.finish();

        // Non-overlapping ranges, so whatever order the server chose, the file
        // must be exactly what the same writes produce locally.
        assert_eq!(fs::read(&path).unwrap(), model);
    }

    #[test]
    fn fs_flush_puts_the_bytes_on_disk() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        let path = root.path().join("file");
        fs::write(&path, b"old").unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::WRITE, 0)),
                (4, fs_write(1, 0, b"new", None)),
                (5, V3Message::Flush { handle: 1 }),
            ],
        );

        assert_eq!(*fs_reply(&replies, 5), V3Message::Done { related_id: 5 });
        // Proving durability across a crash needs a separate process to kill,
        // which arrives with the daemon (E1-S1). What is checkable here is that
        // the flush reported success and the bytes are readable from the file
        // rather than only through the handle.
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn fs_stat_answers_for_a_path_a_link_and_a_handle() {
        use protocol_v3::{attr_presence as presence, open_flags};

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"12345").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("file", root.path().join("link")).unwrap();

        let mask = presence::IDENTITY | presence::NLINK | presence::SYMLINK_TARGET;
        let mut requests = vec![
            (
                3,
                V3Message::Stat {
                    target: StatTarget::Path(b"file".to_vec()),
                    follow: true,
                    attr_mask: mask,
                },
            ),
            (4, fs_open(b"file", open_flags::READ, 0)),
            (
                5,
                V3Message::Stat {
                    target: StatTarget::Handle(1),
                    follow: false,
                    attr_mask: presence::IDENTITY,
                },
            ),
            (
                6,
                V3Message::Stat {
                    target: StatTarget::Path(b"missing".to_vec()),
                    follow: true,
                    attr_mask: 0,
                },
            ),
        ];
        #[cfg(unix)]
        {
            // lstat: the link itself.
            requests.push((
                7,
                V3Message::Stat {
                    target: StatTarget::Path(b"link".to_vec()),
                    follow: false,
                    attr_mask: mask,
                },
            ));
            // stat: what it points at.
            requests.push((
                8,
                V3Message::Stat {
                    target: StatTarget::Path(b"link".to_vec()),
                    follow: true,
                    attr_mask: mask,
                },
            ));
        }

        let replies = fs_run_serial(root.path(), &requests);
        let attrs = |id: u64| match fs_reply(&replies, id) {
            V3Message::AttrsResponse { attrs, .. } => attrs.clone(),
            other => panic!("request {id}: expected Attrs, got {other:?}"),
        };

        let file = attrs(3);
        assert_eq!(file.kind, 1);
        assert_eq!(file.size, 5);
        assert!(file.identity.is_some() && file.nlink.is_some());
        // Not a symlink, so no target even though the mask asked.
        assert_eq!(file.symlink_target, None);

        // fstat through the handle agrees with the path.
        assert_eq!(attrs(5).identity, file.identity);

        match fs_reply(&replies, 6) {
            V3Message::Error { code, .. } => assert_eq!(*code, FsErrorCode::NoEntry),
            other => panic!("expected ENOENT, got {other:?}"),
        }

        #[cfg(unix)]
        {
            let link = attrs(7);
            assert_eq!(link.kind, 3, "follow=false must describe the link");
            assert_eq!(link.symlink_target.as_deref(), Some(&b"file"[..]));
            let followed = attrs(8);
            assert_eq!(followed.kind, 1, "follow=true must describe the target");
            assert_eq!(followed.size, 5);
        }
    }

    #[test]
    fn fs_read_dir_pages_a_snapshot_without_repeating_or_losing_entries() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        let listing = root.path().join("listing");
        fs::create_dir(&listing).unwrap();
        for index in 0..250 {
            fs::write(listing.join(format!("entry-{index:04}")), b"x").unwrap();
        }
        fs::create_dir(listing.join("subdir")).unwrap();

        let mut session = FsLiveSession::start(Server::new(root.path()));
        let handle = session.open(b"listing", open_flags::READ | open_flags::DIRECTORY);

        let mut seen: Vec<Vec<u8>> = Vec::new();
        let mut cursor = 0_u64;
        let mut pages = 0;
        loop {
            let reply = session.request(&V3Message::ReadDir {
                handle,
                cursor,
                max_entries: 32,
                attr_mask: 0,
            });
            let V3Message::DirPage {
                cursor: next,
                final_page,
                entries,
                ..
            } = reply
            else {
                panic!("expected DirPage, got {reply:?}")
            };
            pages += 1;
            for entry in entries {
                // Never the directory's own links.
                assert_ne!(entry.name, b".");
                assert_ne!(entry.name, b"..");
                seen.push(entry.name);
            }
            if final_page {
                break;
            }
            assert_ne!(next, cursor, "a non-final page must advance the cursor");
            cursor = next;
        }
        session.finish();

        assert!(pages > 1, "the page size should have forced several pages");
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "an entry was listed twice");
        assert_eq!(seen.len(), 251, "an entry was lost across pages");
        assert!(seen.iter().any(|name| name == b"subdir"));
    }

    #[test]
    fn fs_read_dir_fills_the_attributes_the_mask_asked_for() {
        use protocol_v3::{attr_presence as presence, open_flags};

        let root = tempdir().unwrap();
        let listing = root.path().join("listing");
        fs::create_dir(&listing).unwrap();
        fs::write(listing.join("file"), b"abc").unwrap();
        fs::create_dir(listing.join("dir")).unwrap();

        let mut session = FsLiveSession::start(Server::new(root.path()));
        let handle = session.open(b"listing", open_flags::READ | open_flags::DIRECTORY);

        let page = |session: &mut FsLiveSession, mask: u32| {
            let reply = session.request(&V3Message::ReadDir {
                handle,
                cursor: 0,
                max_entries: 64,
                attr_mask: mask,
            });
            match reply {
                V3Message::DirPage { entries, .. } => entries,
                other => panic!("expected DirPage, got {other:?}"),
            }
        };

        // The fixed part always rides along -- that is what makes one round
        // trip a readdirplus rather than a listing plus a stat per entry.
        for entry in page(&mut session, 0) {
            assert!(entry.attrs.kind == 1 || entry.attrs.kind == 2);
            assert_ne!(entry.attrs.change_cookie, [0; 16]);
            assert_eq!(entry.attrs.identity, None);
        }
        for entry in page(&mut session, presence::IDENTITY | presence::OWNER) {
            assert!(entry.attrs.identity.is_some());
            if entry.name == b"file" {
                assert_eq!(entry.attrs.size, 3);
            }
            if cfg!(unix) {
                assert!(entry.attrs.owner.is_some());
            }
        }
        session.finish();
    }

    #[test]
    fn fs_read_dir_refuses_a_file_handle_and_a_cursor_past_the_end() {
        use protocol_v3::open_flags;

        let root = tempdir().unwrap();
        fs::write(root.path().join("file"), b"x").unwrap();
        fs::create_dir(root.path().join("dir")).unwrap();

        let replies = fs_run_serial(
            root.path(),
            &[
                (3, fs_open(b"file", open_flags::READ, 0)),
                (
                    4,
                    fs_open(b"dir", open_flags::READ | open_flags::DIRECTORY, 0),
                ),
                (
                    5,
                    V3Message::ReadDir {
                        handle: 1,
                        cursor: 0,
                        max_entries: 8,
                        attr_mask: 0,
                    },
                ),
                (
                    6,
                    V3Message::ReadDir {
                        handle: 2,
                        cursor: 9_999,
                        max_entries: 8,
                        attr_mask: 0,
                    },
                ),
                (
                    7,
                    V3Message::ReadDir {
                        handle: 999,
                        cursor: 0,
                        max_entries: 8,
                        attr_mask: 0,
                    },
                ),
            ],
        );
        let code = |id: u64| match fs_reply(&replies, id) {
            V3Message::Error { code, .. } => *code,
            other => panic!("request {id}: expected Error, got {other:?}"),
        };
        assert_eq!(code(5), FsErrorCode::NotDirectory);
        assert_eq!(code(6), FsErrorCode::Invalid);
        assert_eq!(code(7), FsErrorCode::BadHandle);
    }

    #[test]
    fn fs_read_dir_paging_does_not_grow_with_the_number_of_pages() {
        use protocol_v3::open_flags;

        // The bug this guards against re-read the directory on every page and
        // skipped forward, which Kestrel measured as 167 ms at page 256 against
        // 46 ms for one large page. Paging is a slice of one snapshot, so both
        // shapes do the same work; the bound is deliberately loose because the
        // failure it catches is ~40x, not 40%.
        let root = tempdir().unwrap();
        let listing = root.path().join("listing");
        fs::create_dir(&listing).unwrap();
        for index in 0..10_000 {
            fs::write(listing.join(format!("e{index:05}")), b"").unwrap();
        }

        let count_all = |page_size: u32| -> (usize, std::time::Duration) {
            let mut session = FsLiveSession::start(Server::new(root.path()));
            let handle = session.open(b"listing", open_flags::READ | open_flags::DIRECTORY);
            let start = std::time::Instant::now();
            let mut total = 0;
            let mut cursor = 0;
            loop {
                let reply = session.request(&V3Message::ReadDir {
                    handle,
                    cursor,
                    max_entries: page_size,
                    attr_mask: 0,
                });
                let V3Message::DirPage {
                    cursor: next,
                    final_page,
                    entries,
                    ..
                } = reply
                else {
                    panic!("expected DirPage, got {reply:?}")
                };
                total += entries.len();
                cursor = next;
                if final_page {
                    break;
                }
            }
            let elapsed = start.elapsed();
            session.finish();
            (total, elapsed)
        };

        let (single_total, single) = count_all(16_384);
        let (paged_total, paged) = count_all(256);
        assert_eq!(single_total, 10_000);
        assert_eq!(paged_total, 10_000);
        assert!(
            paged < single * 5 + std::time::Duration::from_millis(250),
            "paging looks quadratic: {paged:?} in pages of 256 against {single:?} in one page"
        );
    }

    #[test]
    fn fs_session_executes_independent_requests_concurrently() {
        // 64 requests issued without awaiting, against a pool of 8. The probe
        // only releases once 8 are inside it at the same time, so this passes
        // only if the dispatcher is genuinely concurrent.
        let requests: Vec<_> = (0..64)
            .map(|index| (index + 3, fs_stat(format!("f{index}").as_bytes())))
            .collect();
        let replies = fs_run_with_handler(ConcurrencyProbe::new(8), (64, 8), &requests);

        assert_eq!(replies.len(), 64);
        let mut answered: Vec<u64> = replies
            .iter()
            .map(|frame| match &frame.message {
                V3Message::Done { related_id } => *related_id,
                other => panic!("expected Done, got {other:?}"),
            })
            .collect();
        answered.sort_unstable();
        assert_eq!(answered, (3..67).collect::<Vec<_>>());
    }

    #[test]
    fn fs_session_answers_out_of_order() {
        /// `slow` finishes only after `fast` has, so the reply order must
        /// invert the request order.
        struct OrderProbe {
            fast_done: Mutex<bool>,
            signal: std::sync::Condvar,
        }

        impl FsHandler for OrderProbe {
            fn handle(
                &self,
                _state: &FsSessionState,
                related_id: u64,
                request: V3Message,
            ) -> V3Message {
                let V3Message::Stat {
                    target: StatTarget::Path(path),
                    ..
                } = request
                else {
                    panic!("unexpected request")
                };
                if path == b"fast" {
                    *self.fast_done.lock().unwrap() = true;
                    self.signal.notify_all();
                    return V3Message::Done { related_id };
                }
                let (_guard, wait) = self
                    .signal
                    .wait_timeout_while(
                        self.fast_done.lock().unwrap(),
                        std::time::Duration::from_secs(10),
                        |done| !*done,
                    )
                    .unwrap();
                if wait.timed_out() {
                    fs_error(related_id, FsErrorCode::TimedOut, "fast request never ran")
                } else {
                    V3Message::Done { related_id }
                }
            }
        }

        let replies = fs_run_with_handler(
            Arc::new(OrderProbe {
                fast_done: Mutex::new(false),
                signal: std::sync::Condvar::new(),
            }),
            (8, 4),
            &[(3, fs_stat(b"slow")), (4, fs_stat(b"fast"))],
        );

        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].message, V3Message::Done { related_id: 4 });
        assert_eq!(replies[1].message, V3Message::Done { related_id: 3 });
    }

    #[test]
    fn fs_session_serialises_write_class_requests_on_one_handle() {
        /// Records arrival order and fails loudly if a second request on the
        /// same handle is dispatched while the first is still running.
        struct HandleOrderProbe {
            arrivals: Mutex<Vec<u64>>,
        }

        impl FsHandler for HandleOrderProbe {
            fn handle(
                &self,
                _state: &FsSessionState,
                related_id: u64,
                _request: V3Message,
            ) -> V3Message {
                let first = {
                    let mut arrivals = self.arrivals.lock().unwrap();
                    arrivals.push(related_id);
                    arrivals.len() == 1
                };
                if first {
                    // Hold the handle's ordering domain. An unordered
                    // dispatcher would start the other two during this window.
                    thread::sleep(std::time::Duration::from_millis(250));
                    if self.arrivals.lock().unwrap().len() > 1 {
                        return fs_error(
                            related_id,
                            FsErrorCode::Busy,
                            "a later request on the same handle overtook this one",
                        );
                    }
                }
                V3Message::Done { related_id }
            }
        }

        let probe = Arc::new(HandleOrderProbe {
            arrivals: Mutex::new(Vec::new()),
        });
        // Flush is write-class, so it takes its handle's domain exclusively.
        // Reads deliberately do *not* serialise (see
        // `fs_reads_on_one_handle_run_concurrently`).
        let flush = || V3Message::Flush { handle: 1 };
        let replies = fs_run_with_handler(
            Arc::clone(&probe) as Arc<dyn FsHandler>,
            (8, 8),
            &[(3, flush()), (4, flush()), (5, flush())],
        );

        // Same handle, so send order is preserved end to end even though the
        // pool has eight free workers.
        assert_eq!(*probe.arrivals.lock().unwrap(), vec![3, 4, 5]);
        let ids: Vec<u64> = replies
            .iter()
            .map(|frame| match &frame.message {
                V3Message::Done { related_id } => *related_id,
                other => panic!("expected Done, got {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec![3, 4, 5]);
    }

    #[test]
    fn fs_session_refuses_requests_past_the_in_flight_cap() {
        // Two workers park until both are busy, so requests three and four
        // arrive with the cap full.
        let replies = fs_run_with_handler(
            ConcurrencyProbe::new(2),
            (2, 4),
            &[
                (3, fs_stat(b"a")),
                (4, fs_stat(b"b")),
                (5, fs_stat(b"c")),
                (6, fs_stat(b"d")),
            ],
        );

        assert_eq!(replies.len(), 4);
        let mut limited = Vec::new();
        let mut done = Vec::new();
        for frame in &replies {
            match &frame.message {
                V3Message::Done { related_id } => done.push(*related_id),
                V3Message::Error {
                    related_id, code, ..
                } if *code == FsErrorCode::Limit => limited.push(*related_id),
                other => panic!("unexpected reply {other:?}"),
            }
        }
        done.sort_unstable();
        limited.sort_unstable();
        assert_eq!(done, vec![3, 4]);
        assert_eq!(limited, vec![5, 6]);
    }

    #[test]
    fn fs_session_answers_keepalive_without_waiting_for_the_pool() {
        // Both stats park until the second is dispatched, which happens after
        // the keepalive is read -- so a keepalive queued behind pool work
        // could not be answered first.
        let replies = fs_run_with_handler(
            ConcurrencyProbe::new(2),
            (8, 2),
            &[
                (3, fs_stat(b"a")),
                (4, V3Message::Keepalive { nonce: 99 }),
                (5, fs_stat(b"b")),
            ],
        );

        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0].message, V3Message::KeepaliveAck { nonce: 99 });
    }

    #[test]
    fn fs_session_cancel_removes_queued_work_before_it_runs() {
        // Three requests on one handle: the second is cancelled while it waits
        // behind the first, so it must never reach the handler.
        struct SlowFirst {
            seen: Mutex<Vec<u64>>,
        }

        impl FsHandler for SlowFirst {
            fn handle(
                &self,
                _state: &FsSessionState,
                related_id: u64,
                _request: V3Message,
            ) -> V3Message {
                let first = {
                    let mut seen = self.seen.lock().unwrap();
                    seen.push(related_id);
                    seen.len() == 1
                };
                if first {
                    thread::sleep(std::time::Duration::from_millis(250));
                }
                V3Message::Done { related_id }
            }
        }

        let probe = Arc::new(SlowFirst {
            seen: Mutex::new(Vec::new()),
        });
        // Write-class, so the second genuinely waits behind the first and is
        // still queued when the cancel arrives.
        let flush = || V3Message::Flush { handle: 1 };
        let replies = fs_run_with_handler(
            Arc::clone(&probe) as Arc<dyn FsHandler>,
            (8, 8),
            &[
                (3, flush()),
                (4, flush()),
                (5, V3Message::Cancel { related_id: 4 }),
            ],
        );

        assert_eq!(*probe.seen.lock().unwrap(), vec![3], "cancelled work ran");
        let cancelled = replies.iter().find(|frame| {
            matches!(
                &frame.message,
                V3Message::Error {
                    related_id: 4,
                    code: FsErrorCode::Cancelled,
                    ..
                }
            )
        });
        assert!(cancelled.is_some(), "no cancellation response: {replies:?}");
        assert!(replies
            .iter()
            .any(|frame| frame.message == V3Message::Done { related_id: 3 }));
    }

    #[test]
    fn fs_session_is_fail_closed_on_frame_order_and_grammar() {
        let root = tempdir().unwrap();
        let run = |frames: &[(u64, V3Message)]| {
            Server::new(root.path())
                .run(
                    Cursor::new(fs_client_input(FS_V3_CLIENT, frames)),
                    &mut Vec::new(),
                )
                .unwrap_err()
        };

        // A feature-gated request cannot precede the exchange that gates it.
        let error = run(&[(2, V3Message::Keepalive { nonce: 1 })]);
        assert!(error.to_string().contains("Features first"), "{error}");

        // The exchange happens exactly once.
        let error = run(&[
            (2, V3Message::Features { features: 0 }),
            (3, V3Message::Features { features: 0 }),
        ]);
        assert!(error.to_string().contains("twice"), "{error}");

        // Duplicate message IDs are a session error, as in v2.
        let error = run(&[
            (2, V3Message::Features { features: 0 }),
            (2, V3Message::Keepalive { nonce: 1 }),
        ]);
        assert!(error.to_string().contains("duplicate"), "{error}");

        // A v2 frame after a v3 selection is refused by the envelope check;
        // the session never retries it as the older grammar.
        let mut input = encode_frame(1, &fs_session_handshake(FS_V3_CLIENT)).unwrap();
        input.extend_from_slice(
            &protocol_v3::encode_frame(2, &V3Message::Features { features: 0 }).unwrap(),
        );
        input.extend_from_slice(
            &protocol_v2::encode_frame(3, &V2Message::Keepalive { nonce: 1 }).unwrap(),
        );
        let error = Server::new(root.path())
            .run(Cursor::new(input), &mut Vec::new())
            .unwrap_err();
        assert!(matches!(error, ServerError::FsSession(_)), "{error}");
        assert!(error.to_string().contains("wrong version"), "{error}");
        assert_eq!(error.kind(), "protocol");
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
    fn browse_meta_verbs_chmod_mtime_and_read_link() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("file"), b"data").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("file", temp.path().join("link")).unwrap();
        let server = Server::new(temp.path());

        assert_eq!(
            server.browse_set_permissions_response(b"file", 0o600, 1),
            V2Message::SetPermissionsResponse {
                related_id: 1,
                status: MutationStatus::Ok,
                error: Vec::new(),
            }
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(temp.path().join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(mode, 0o600);
        }

        let mtime_ns = 1_700_000_000i64 * 1_000_000_000;
        assert_eq!(
            server.browse_set_mtime_response(b"file", mtime_ns, 2),
            V2Message::SetMtimeResponse {
                related_id: 2,
                status: MutationStatus::Ok,
                error: Vec::new(),
            }
        );
        let modified = fs::metadata(temp.path().join("file"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(modified, 1_700_000_000);

        assert!(matches!(
            server.browse_set_permissions_response(b"missing", 0o644, 3),
            V2Message::SetPermissionsResponse {
                related_id: 3,
                status: MutationStatus::ParentMissing,
                error,
            } if !error.is_empty()
        ));
        assert!(matches!(
            server.browse_set_permissions_response(b"../escape", 0o644, 4),
            V2Message::SetPermissionsResponse {
                status: MutationStatus::Error,
                ..
            }
        ));

        #[cfg(unix)]
        {
            assert_eq!(
                server.browse_read_link_response(b"link", 5),
                V2Message::ReadLinkResponse {
                    related_id: 5,
                    status: StatStatus::Ok,
                    target: b"file".to_vec(),
                    error: Vec::new(),
                }
            );
            assert!(matches!(
                server.browse_read_link_response(b"file", 6),
                V2Message::ReadLinkResponse {
                    related_id: 6,
                    status: StatStatus::Error,
                    target,
                    error,
                } if target.is_empty() && !error.is_empty()
            ));
            assert_eq!(
                server.browse_read_link_response(b"missing-link", 7),
                V2Message::ReadLinkResponse {
                    related_id: 7,
                    status: StatStatus::Missing,
                    target: Vec::new(),
                    error: Vec::new(),
                }
            );
        }
    }

    #[test]
    fn browse_meta_messages_are_rejected_without_the_capability_bit() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("file"), b"data").unwrap();
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [21; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let mut input = encode_frame(1, &handshake).unwrap();
        input.extend_from_slice(
            &protocol_v2::encode_frame(
                2,
                &V2Message::SetPermissionsRequest {
                    path: b"file".to_vec(),
                    mode: 0o600,
                },
            )
            .unwrap(),
        );
        let mut output = Vec::new();
        let error = Server::new_with_capabilities(temp.path(), capabilities)
            .run(Cursor::new(input), &mut output)
            .unwrap_err();
        assert!(
            error.to_string().contains("CAP_BROWSE_META"),
            "expected capability refusal, got {error}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(temp.path().join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777;
            assert_ne!(mode, 0o600, "chmod must not apply without the capability");
        }
    }

    #[test]
    fn browse_session_does_not_send_meta_verbs_to_an_older_peer() {
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION;
        let server_handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [22; 16],
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
        let mut session =
            BrowseSession::connect(Cursor::new(peer_bytes), Vec::new(), [7; 16]).unwrap();
        assert!(!session.supports_browse_meta());
        let error = session
            .set_permissions(b"file".to_vec(), 0o600)
            .unwrap_err();
        assert!(
            error.to_string().contains("CAP_BROWSE_META"),
            "expected local refusal, got {error}"
        );
        let (_reader, writer) = session.into_parts();
        let mut sent = Cursor::new(writer);
        let mut decoder = FrameDecoder::new();
        assert!(matches!(
            decoder.read(&mut sent).unwrap().message,
            Message::Handshake {
                role: Role::Session,
                ..
            }
        ));
        assert_eq!(
            sent.position() as usize,
            sent.get_ref().len(),
            "no v2 frame may be sent to an older peer"
        );
    }

    #[test]
    fn session_handshake_advertises_browse_meta_by_default() {
        let capabilities = CAP_BROWSE_V2 | CAP_VERSION_NEGOTIATION | CAP_BROWSE_META;
        let handshake = Message::Handshake {
            role: Role::Session,
            capabilities,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [23; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let input = encode_frame(1, &handshake).unwrap();
        let mut output = Vec::new();
        Server::new(tempdir().unwrap().path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let mut cursor = Cursor::new(output);
        match decoder.read(&mut cursor).unwrap().message {
            Message::Handshake {
                role: Role::Session,
                capabilities: advertised,
                ..
            } => assert_ne!(
                advertised & CAP_BROWSE_META,
                0,
                "default session server must advertise CAP_BROWSE_META"
            ),
            other => panic!("expected handshake, got {other:?}"),
        }
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
        let replacement = b"new content";
        let digest = *blake3::hash(replacement).as_bytes();
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
                    content_size: replacement.len() as u64,
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
                    data: replacement.to_vec(),
                },
            )
            .unwrap(),
        );
        let mut output = Vec::new();
        Server::new(temp.path())
            .run(Cursor::new(input), &mut output)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), replacement);
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
            &crate::protocol::encode_frame_with_version(
                2,
                &Message::Ack {
                    acknowledged_id: 1,
                    acknowledged_type: 1,
                },
                1,
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
                    filter_rules: Vec::new(),
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
                    digest: *blake3::hash(b"must not be silently dropped").as_bytes(),
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
                    filter_rules: Vec::new(),
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
                    digest: *blake3::hash(b"hello").as_bytes(),
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
        let (_, args) = remote_server_command_with_shell(
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
                "set \"PATH=%USERPROFILE%\\.local\\bin;%PATH%\" & \"xs\" --server \"C:/backup\""
            ]
        );
    }

    #[test]
    fn windows_remote_expands_home_with_userprofile() {
        let (_, args) =
            remote_server_command_with_shell("~", None, Some("winbox"), RemoteShell::Windows)
                .unwrap();
        assert!(
            args[1].ends_with("\"xs\" --server \"%USERPROFILE%\""),
            "{}",
            args[1]
        );

        let (_, args) = remote_server_command_with_shell(
            "~/backup",
            None,
            Some("winbox"),
            RemoteShell::Windows,
        )
        .unwrap();
        assert!(
            args[1].ends_with("\"xs\" --server \"%USERPROFILE%\\backup\""),
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
            filter_rules: Vec::new(),
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
            digest: *blake3::hash(b"stdout validity").as_bytes(),
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

            fs::write(outside.join("file"), b"outside").unwrap();
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
            &WirePath::from_wire(b"same/path".to_vec()).unwrap(),
            &mut seen,
        )
        .unwrap();
        assert!(matches!(
            validate_unique_destination_path(
                &root,
                &WirePath::from_wire(b"same/path".to_vec()).unwrap(),
                &mut seen,
            ),
            Err(ServerError::DuplicatePath(path)) if path == "same/path"
        ));
        assert_eq!(seen.len(), 1);
    }
}
