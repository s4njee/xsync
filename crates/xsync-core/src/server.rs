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

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use filetime::{set_file_mtime, FileTime};
use thiserror::Error;

use crate::local::{
    LocalEvent, LocalSyncOptions, LocalSyncReport, TransferMethod,
};
use crate::planner::{
    try_plan, DestinationIndex, IndexConfig, PlannerError,
};
use crate::protocol::{
    encode_frame, ByteRange, CompressionMode,
    EntryRecord, FrameDecoder, Message, MetadataOperation,
    ProtocolError, Role, MAX_COLLECTION_COUNT, MAX_COMPLETE_PAYLOAD,
    MAX_DATA_SEGMENT, DEFAULT_UNACKNOWLEDGED_WINDOW,
};
use crate::scanner::{
    permission_mode, scan, EntryKind as ScanEntryKind, FileEntry, FileIdentity, ScanError,
    SourceFingerprint,
};
use crate::sink::{Sink, SinkError, SymlinkTargetKind};
use crate::source::{SourceReadError, SourceReader};

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
    /// A transport error occurred on the named stream.
    #[error("transport error on stream {stream}: {message}")]
    Transport {
        /// Stream identifier.
        stream: usize,
        /// Diagnostic message.
        message: String,
    },
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

/// Convert an [`EntryRecord`] received from the wire into a [`FileEntry`].
///
/// # Errors
/// Returns [`ServerError::InvalidPath`] if the path bytes are not valid UTF-8.
pub fn file_entry_from_entry_record(record: &EntryRecord) -> Result<FileEntry, ServerError> {
    let path = String::from_utf8(record.path.clone())
        .map_err(|_| ServerError::InvalidPath(format!("{:?}", record.path)))?;
    let mtime = nanos_to_system_time(record.mtime_ns);
    let device = u64::from_le_bytes(
        record.fingerprint[0..8]
            .try_into()
            .unwrap_or([0; 8]),
    );
    let file = u64::from_le_bytes(
        record.fingerprint[8..16]
            .try_into()
            .unwrap_or([0; 8]),
    );
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

/// Validate that a relative path does not escape the root via traversal or symlinks.
///
/// # Errors
/// Returns [`ServerError::InvalidPath`] or [`ServerError::SymlinkEscape`].
pub fn validate_destination_path(root: &Path, relative_path: &str) -> Result<PathBuf, ServerError> {
    if relative_path.is_empty() {
        return Err(ServerError::InvalidPath("empty path".to_owned()));
    }
    let mut current = root.to_path_buf();
    for part in relative_path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(ServerError::InvalidPath(relative_path.to_owned()));
        }
        let mut components = Path::new(part).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(ServerError::InvalidPath(relative_path.to_owned()));
        }
        current.push(part);
        // Check if an intermediate ancestor is a symlink.
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() && current.is_dir() {
                return Err(ServerError::SymlinkEscape(relative_path.to_owned()));
            }
        }
    }
    Ok(current)
}

/// A server instance executing either Source or Sink roles over framed streams.
#[derive(Debug)]
pub struct Server {
    root: PathBuf,
    next_message_id: u64,
    decoder: FrameDecoder,
    seen_destinations: HashSet<String>,
    journal: Option<crate::journal::ResumeJournal>,
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
        }
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
    pub fn run<R: Read, W: Write>(
        &mut self,
        mut reader: R,
        mut writer: W,
    ) -> Result<(), ServerError> {
        // 1. Receive Handshake from client.
        let frame = self.decoder.read(&mut reader)?;
        let (client_role, job_id, compression) = match frame.message {
            Message::Handshake {
                role,
                job_id,
                compression,
                ..
            } => (role, job_id, compression),
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Handshake, got {other:?}"
                )))
            }
        };

        // Establish the durable resume journal for this session's job ID.
        self.journal = Some(crate::journal::ResumeJournal::new(&job_id)?);

        // Determine server's role.
        let server_role = match client_role {
            Role::Source => Role::Sink,
            Role::Sink => Role::Source,
        };

        // Send Server Handshake and Ack.
        let server_handshake = Message::Handshake {
            role: server_role,
            capabilities: 0,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id,
            compression,
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

        // 2. Receive SessionConfig from client.
        let frame = self.decoder.read(&mut reader)?;
        let (paranoid, delete, checksum) = match frame.message {
            Message::SessionConfig {
                paranoid,
                delete,
                checksum,
                ..
            } => (paranoid, delete, checksum),
            other => {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected SessionConfig, got {other:?}"
                )))
            }
        };

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
            Role::Sink => self.run_sink(&mut reader, &mut writer, paranoid, delete, checksum),
            Role::Source => self.run_source(&mut reader, &mut writer),
        }
    }

    fn run_sink<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        paranoid: bool,
        delete: bool,
        _checksum: bool,
    ) -> Result<(), ServerError> {
        // Destination scan phase: if destination exists, scan and send Scan frames.
        let mut entries = Vec::new();
        if self.root.exists() {
            if let Ok(scan_result) = scan(&self.root) {
                for item in scan_result.entries() {
                    if let Ok(entry) = item {
                        entries.push(entry_record_from_file_entry(&entry));
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

            match frame.message {
                Message::Metadata {
                    operation,
                    path,
                    target,
                    mode,
                    mtime_ns,
                } => {
                    let rel_path = String::from_utf8(path)
                        .map_err(|_| ServerError::InvalidPath("invalid UTF-8 path".to_owned()))?;
                    if !rel_path.is_empty() && !self.seen_destinations.insert(rel_path.clone()) {
                        // Check if duplicate for same operation; allow set directory after creation
                        if operation != MetadataOperation::SetDirectory {
                            return Err(ServerError::DuplicatePath(rel_path));
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
                            let target_str = String::from_utf8(target).map_err(|_| {
                                ServerError::InvalidPath("invalid symlink target".to_owned())
                            })?;
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
                                Path::new(&target_str),
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
                                    path: ".".to_owned(),
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
                                sink.delete_entry(&entry)?;
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
                        let file_entry = file_entry_from_entry_record(&record)?;
                        validate_destination_path(&self.root, &file_entry.path)?;
                        if !self.seen_destinations.insert(file_entry.path.clone()) {
                            return Err(ServerError::DuplicatePath(file_entry.path.clone()));
                        }
                        let hash = blake3::hash(&data);
                        sink.write_file_with_retry(&file_entry, &hash, |_attempt| {
                            Ok(data.clone())
                        })?;

                        if paranoid {
                            let committed_path = sink.path_for(&file_entry.path)?;
                            let readback = fs::read(&committed_path)?;
                            if blake3::hash(&readback) != hash {
                                return Err(ServerError::Sink(SinkError::VerificationFailed {
                                    path: file_entry.path,
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
                    let rel_path = String::from_utf8(path)
                        .map_err(|_| ServerError::InvalidPath("invalid UTF-8 path".to_owned()))?;
                    validate_destination_path(&self.root, &rel_path)?;
                    if !self.seen_destinations.insert(rel_path.clone()) {
                        return Err(ServerError::DuplicatePath(rel_path));
                    }
                    let device = u64::from_le_bytes(fingerprint[0..8].try_into().unwrap_or([0; 8]));
                    let file =
                        u64::from_le_bytes(fingerprint[8..16].try_into().unwrap_or([0; 8]));
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
                    if let Some(entry) = large_files.remove(&file_id) {
                        sink.finish_large(&entry)?;
                        // The file is committed; discard its resume record.
                        let journal = self
                            .journal
                            .as_ref()
                            .expect("journal is initialized during handshake");
                        let identity = crate::journal::ResumeIdentity {
                            path: entry.path.clone().into_bytes(),
                            fingerprint: entry.fingerprint,
                        };
                        journal.clear(&identity)?;
                        large_ranges.remove(&file_id);
                        if paranoid {
                            let committed_path = sink.path_for(&entry.path)?;
                            let readback = fs::read(&committed_path)?;
                            if *blake3::hash(&readback).as_bytes() != digest {
                                return Err(ServerError::Sink(SinkError::VerificationFailed {
                                    path: entry.path,
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
                        let bytes = encode_frame(msg_id, &seg)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;

                        // Wait for client Ack for segment.
                        let ack_frame = self.decoder.read(reader)?;
                        if !matches!(ack_frame.message, Message::Ack { .. }) {
                            return Err(ServerError::UnexpectedMessage(format!(
                                "expected Ack for segment, got {:?}",
                                ack_frame.message
                            )));
                        }
                    }

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
                    let rel_path = String::from_utf8(path)
                        .map_err(|_| ServerError::InvalidPath("invalid UTF-8 path".to_owned()))?;
                    let device = u64::from_le_bytes(fingerprint[0..8].try_into().unwrap_or([0; 8]));
                    let file =
                        u64::from_le_bytes(fingerprint[8..16].try_into().unwrap_or([0; 8]));
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
                Message::LargeFileRange {
                    file_id,
                    range,
                } => {
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
                        let bytes = encode_frame(msg_id, &seg)?;
                        writer.write_all(&bytes)?;
                        writer.flush()?;

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
                Message::LargeFileFinish {
                    file_id,
                    digest: _,
                } => {
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
                        let rel_path = String::from_utf8(path).map_err(|_| {
                            ServerError::InvalidPath("invalid UTF-8 path".to_owned())
                        })?;
                        let symlink_path = self.root.join(&rel_path);
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
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut server = Server::new(root);
    let stdin_lock = stdin.lock();
    let stdout_lock = stdout.lock();
    let mut reader = BufReader::new(stdin_lock);
    let mut writer = BufWriter::new(stdout_lock);
    server.run(&mut reader, &mut writer)
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
    dest_trailing_slash: bool,
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

    // 1. Send Handshake (Client is Source).
    let handshake = Message::Handshake {
        role: Role::Source,
        capabilities: 0,
        max_payload: MAX_COMPLETE_PAYLOAD as u32,
        max_segment: MAX_DATA_SEGMENT as u32,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        job_id: session_job_id(
            source_path.to_string_lossy().as_ref(),
            dest_path,
        ),
        compression: CompressionMode::None,
    };
    let hs_id = alloc_id();
    let bytes = encode_frame(hs_id, &handshake)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Server Handshake and Ack.
    let frame1 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame1.message, Message::Handshake { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Server Handshake, got {:?}",
            frame1.message
        )));
    }
    let frame2 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame2.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Handshake, got {:?}",
            frame2.message
        )));
    }

    // 2. Send SessionConfig.
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: false,
        paranoid: options.paranoid,
    };
    let sc_id = alloc_id();
    let bytes = encode_frame(sc_id, &session_config)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Ack for SessionConfig.
    let frame3 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame3.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for SessionConfig, got {:?}",
            frame3.message
        )));
    }

    // 3. Receive Scan pages from server.
    let mut dest_entries = Vec::new();
    loop {
        let frame = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
        match frame.message {
            Message::Scan {
                scan_id: _,
                final_page,
                entries,
            } => {
                for rec in entries {
                    dest_entries.push(file_entry_from_entry_record(&rec)?);
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

    // Scan local source root.
    let source_scan = scan(source_path)?;
    let mut source_entries = Vec::new();
    for item in source_scan.entries() {
        let entry = item?;
        source_entries.push(entry);
    }
    source_scan.finish()?;

    // Map source entries relative to destination root according to trailing-slash rules.
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
        if dest_trailing_slash || dest_path.ends_with('/') {
            source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_owned()
        } else {
            String::new()
        }
    };

    let mut mapped_source = Vec::new();
    for mut entry in source_entries {
        if !prefix.is_empty() {
            entry.path = format!("{prefix}/{}", entry.path);
        }
        mapped_source.push(entry);
    }

    // Plan differences.
    let plan = try_plan(mapped_source, dest_index)?;

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
            path: entry.path.clone(),
        });
    }

    if !options.dry_run {
        // Create directories.
        let mut dirs_to_create = plan.directories.new.clone();
        dirs_to_create.sort_by_key(|d| d.path.len());
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
            writer.flush()?;

            let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
            if !matches!(ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for CreateDirectory, got {:?}",
                    ack.message
                )));
            }
        }

        // Create symlinks.
        for sym in plan.symlinks.new.iter().chain(&plan.symlinks.changed) {
            let local_sym_path = if prefix.is_empty() {
                source_path.join(&sym.path)
            } else {
                let stripped = sym.path.strip_prefix(&format!("{prefix}/")).unwrap_or(&sym.path);
                source_path.join(stripped)
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
            writer.flush()?;

            let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
            if !matches!(ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for CreateSymlink, got {:?}",
                    ack.message
                )));
            }
        }

        // Transfer files.
        let source_reader = SourceReader::new(source_path);
        for file in plan.files.new.iter().chain(&plan.files.changed) {
            let mut file_to_read = file.clone();
            if !prefix.is_empty() {
                file_to_read.path = file
                    .path
                    .strip_prefix(&format!("{prefix}/"))
                    .unwrap_or(&file.path)
                    .to_owned();
            }

            let stable = match source_reader.read(&file_to_read) {
                Ok(s) => s,
                Err(err) => {
                    emit(LocalEvent::Failed {
                        path: file.path.clone(),
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

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                let b = encode_frame(msg_id, &seg_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                    path: file.path.clone(),
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

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                        let frame =
                            decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                let missing = crate::journal::missing_chunks(
                    file.size,
                    8 * 1024 * 1024,
                    &verified_ranges,
                );
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
                    let b = encode_frame(msg_id, &seg_msg)?;
                    writer.write_all(&b)?;
                    writer.flush()?;

                    let ack1 =
                        decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
                    let ack2 =
                        decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
                    if !matches!(ack1.message, Message::Ack { .. })
                        || !matches!(ack2.message, Message::Ack { .. })
                    {
                        return Err(ServerError::UnexpectedMessage(
                            "expected Ack for LargeFileRange/Segment".to_owned(),
                        ));
                    }
                }
                retransmitted_bytes_total =
                    retransmitted_bytes_total.saturating_add(sent_bytes);
                checkpoint_bytes_total = checkpoint_bytes_total
                    .saturating_add(resumed_bytes.saturating_add(sent_bytes));

                let finish_msg = Message::LargeFileFinish {
                    file_id,
                    digest: *stable.blake3.as_bytes(),
                };
                let msg_id = alloc_id();
                let b = encode_frame(msg_id, &finish_msg)?;
                writer.write_all(&b)?;
                writer.flush()?;

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                    path: file.path.clone(),
                    bytes: file.size,
                    physical_bytes: sent_bytes,
                    method: TransferMethod::ByteCopy,
                });
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
            writer.flush()?;

            let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
            if !matches!(ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for SetDirectory, got {:?}",
                    ack.message
                )));
            }
        }

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

            let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
            if !matches!(ack.message, Message::Ack { .. }) {
                return Err(ServerError::UnexpectedMessage(format!(
                    "expected Ack for root SetDirectory, got {:?}",
                    ack.message
                )));
            }
        }

        // Delete extraneous entries if enabled.
        if options.delete && !report.partial_failure() {
            let mut to_delete = Vec::new();
            to_delete.extend(plan.files.extraneous.clone());
            to_delete.extend(plan.symlinks.extraneous.clone());
            to_delete.extend(plan.other.extraneous.clone());
            // Directories deleted deepest first
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

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
                if !matches!(ack.message, Message::Ack { .. }) {
                    return Err(ServerError::UnexpectedMessage(format!(
                        "expected Ack for Delete, got {:?}",
                        ack.message
                    )));
                }

                emit(LocalEvent::Deleted { path: entry.path });
            }
        }
    }

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

    let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
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

    // 1. Send Handshake (Client is Sink).
    let job_id =
        session_job_id(src_path, dest_path.to_string_lossy().as_ref());
    let resume_journal = crate::journal::ResumeJournal::new(&job_id)?;
    let handshake = Message::Handshake {
        role: Role::Sink,
        capabilities: 0,
        max_payload: MAX_COMPLETE_PAYLOAD as u32,
        max_segment: MAX_DATA_SEGMENT as u32,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        job_id,
        compression: CompressionMode::None,
    };
    let hs_id = alloc_id();
    let bytes = encode_frame(hs_id, &handshake)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Server Handshake and Ack.
    let frame1 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame1.message, Message::Handshake { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Server Handshake, got {:?}",
            frame1.message
        )));
    }
    let frame2 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame2.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Handshake, got {:?}",
            frame2.message
        )));
    }

    // 2. Send SessionConfig.
    let session_config = Message::SessionConfig {
        streams: u8::try_from(options.streams).unwrap_or(1),
        batch_bytes: 32 * 1024 * 1024,
        chunk_bytes: 16 * 1024 * 1024,
        window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
        delete: options.delete,
        checksum: false,
        paranoid: options.paranoid,
    };
    let sc_id = alloc_id();
    let bytes = encode_frame(sc_id, &session_config)?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    // Read Ack for SessionConfig.
    let frame3 = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(frame3.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for SessionConfig, got {:?}",
            frame3.message
        )));
    }

    // 3. Receive source Scan pages from server.
    let mut source_entries = Vec::new();
    let mut source_root_entry: Option<FileEntry> = None;
    loop {
        let frame = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                            path: ".".to_owned(),
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
                        source_entries.push(file_entry_from_entry_record(&rec)?);
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
                    dest_index.insert(entry)?;
                }
            }
            let _ = dest_scan.finish();
        }
    }

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
            entry.path = format!("{prefix}/{}", entry.path);
        }
        mapped_source.push(entry);
    }

    // Plan differences.
    let plan = try_plan(mapped_source, dest_index)?;

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
            path: entry.path.clone(),
        });
    }

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
                    .strip_prefix(&format!("{prefix}/"))
                    .unwrap_or(&sym.path)
                    .to_owned()
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

            let reply = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
            if let Message::Metadata { target, .. } = reply.message {
                let target_str = String::from_utf8(target).map_err(|_| {
                    ServerError::InvalidPath("invalid symlink target".to_owned())
                })?;
                sink.create_symlink(
                    sym,
                    Path::new(&target_str),
                    SymlinkTargetKind::File,
                )?;
            }
        }

        // Request files from server.
        for file in plan.files.new.iter().chain(&plan.files.changed) {
            let raw_path = if prefix.is_empty() {
                file.path.clone()
            } else {
                file.path
                    .strip_prefix(&format!("{prefix}/"))
                    .unwrap_or(&file.path)
                    .to_owned()
            };

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
                let seg_frame = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                            path: file.path.clone(),
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
                let batch_ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                    path: file.path.clone(),
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
                let resumed_bytes: u64 =
                    verified_ranges.iter().map(|r| r.length).sum();
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

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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

                    let seg_frame =
                        decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                    let range_ack =
                        decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
                    if !matches!(range_ack.message, Message::Ack { .. }) {
                        return Err(ServerError::UnexpectedMessage(format!(
                            "expected Ack for LargeFileRange, got {:?}",
                            range_ack.message
                        )));
                    }
                }
                retransmitted_bytes_total = retransmitted_bytes_total.saturating_add(sent_bytes);
                checkpoint_bytes_total = checkpoint_bytes_total
                    .saturating_add(resumed_bytes.saturating_add(sent_bytes));

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

                let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
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
                    path: file.path.clone(),
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
                sink.delete_entry(entry)?;
                emit(LocalEvent::Deleted {
                    path: entry.path.clone(),
                });
            }
            for entry in &plan.symlinks.extraneous {
                sink.delete_entry(entry)?;
                emit(LocalEvent::Deleted {
                    path: entry.path.clone(),
                });
            }
            let mut ext_dirs = plan.directories.extraneous.clone();
            ext_dirs.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
            for entry in &ext_dirs {
                sink.delete_entry(entry)?;
                emit(LocalEvent::Deleted {
                    path: entry.path.clone(),
                });
            }
        }
    }

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

    let ack = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;
    if !matches!(ack.message, Message::Ack { .. }) {
        return Err(ServerError::UnexpectedMessage(format!(
            "expected Ack for Stats, got {:?}",
            ack.message
        )));
    }
    let _server_stats = decoder.read(&mut reader).map_err(|e| map_transport_error(e, 0))?;

    report.resumed_bytes = resumed_bytes_total;
    report.restarted_files = restarted_files_total;
    report.retransmitted_bytes = retransmitted_bytes_total;
    report.checkpoint_bytes = checkpoint_bytes_total;

    emit(LocalEvent::Finished {
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        skipped_files: report.skipped_files,
        failed_entries: report.failed_entries,
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
    let child = spawn_server_child(dest_path, rsh, host)?;
    run_server_child_session(child, |reader, writer| {
        run_client_push(
            source_path,
            source_trailing_slash,
            dest_path,
            dest_trailing_slash,
            options,
            reader,
            writer,
            emit,
        )
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
    let child = spawn_server_child(src_path, rsh, host)?;
    run_server_child_session(child, |reader, writer| {
        run_client_pull(
            src_path,
            src_trailing_slash,
            dest_path,
            dest_trailing_slash,
            options,
            reader,
            writer,
            emit,
        )
    })
}

/// Message reported when the remote `xsync` binary cannot be located.
const MISSING_XSYNC_MSG: &str = "xsync not found on remote host — install it or check PATH";

fn parse_rsh_command(rsh: &str) -> Vec<String> {
    shlex::split(rsh).unwrap_or_else(|| vec![rsh.to_owned()])
}

/// Default remote shell; replaced only by an explicit `-e/--rsh`.
const DEFAULT_RSH: &str = "ssh";

fn is_missing_xsync_stderr(stderr: &str, exit_code: Option<i32>) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("xsync: command not found")
        || lower.contains("command not found")
        || lower.contains("no such file")
        || lower.contains("not found")
        || exit_code == Some(127)
}

/// Compute `(program, args)` used to launch the remote `xsync --server`.
///
/// - Explicit `-e CMD`: shell request parsed (shlex), then `{host}` and
///   `xsync --server {path}` are appended.
/// - No `-e` but a host: the default `ssh {host} xsync --server {path}`.
/// - No host and no `-e`: an in-process/local child server via `current_exe`.
#[must_use]
fn remote_server_command(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
) -> (String, Vec<String>) {
    if let Some(rsh_cmd) = rsh {
        let parts = parse_rsh_command(rsh_cmd);
        let program = parts.first().cloned().unwrap_or_else(|| rsh_cmd.to_owned());
        let mut args = if parts.is_empty() {
            Vec::new()
        } else {
            parts[1..].to_vec()
        };
        if let Some(h) = host {
            args.push(h.to_owned());
        }
        args.push("xsync".to_owned());
        args.push("--server".to_owned());
        args.push(remote_path.to_owned());
        (program, args)
    } else if let Some(h) = host {
        (
            DEFAULT_RSH.to_owned(),
            vec![
                h.to_owned(),
                "xsync".to_owned(),
                "--server".to_owned(),
                remote_path.to_owned(),
            ],
        )
    } else {
        let exe = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("xsync"))
            .to_string_lossy()
            .into_owned();
        (
            exe,
            vec!["--server".to_owned(), remote_path.to_owned()],
        )
    }
}

fn spawn_server_child(
    remote_path: &str,
    rsh: Option<&str>,
    host: Option<&str>,
) -> Result<Child, ServerError> {
    let (program, args) = remote_server_command(remote_path, rsh, host);
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
/// child's stderr so a missing remote binary is reported as
/// [`MISSING_XSYNC_MSG`] rather than as a raw broken-pipe error.
fn run_server_child_session<F>(
    child: Child,
    f: F,
) -> Result<LocalSyncReport, ServerError>
where
    F: FnOnce(
        &mut BufReader<std::process::ChildStdout>,
        &mut BufWriter<std::process::ChildStdin>,
    ) -> Result<LocalSyncReport, ServerError>,
{
    let mut child = child;
    let stdin = child.stdin.take().ok_or_else(|| ServerError::Transport {
        stream: 0,
        message: "failed to open child stdin".to_owned(),
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ServerError::Transport {
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

    let mut reader = BufReader::new(stdout);
    let mut writer = BufWriter::new(stdin);
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
        return Err(ServerError::Transport {
            stream: 0,
            message: MISSING_XSYNC_MSG.to_owned(),
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
    fn unrouted_segment_is_a_loud_error_not_a_silent_drop() {
        let dst = tempdir().unwrap();
        let mut input = Vec::new();

        // Handshake: client is the source (we are sending data at the sink).
        input.extend_from_slice(&encode_frame(1, &Message::Handshake {
            role: Role::Source,
            capabilities: 0,
            max_payload: MAX_COMPLETE_PAYLOAD as u32,
            max_segment: MAX_DATA_SEGMENT as u32,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            job_id: [9u8; 16],
            compression: CompressionMode::None,
        }).unwrap());

        // SessionConfig.
        input.extend_from_slice(&encode_frame(2, &Message::SessionConfig {
            streams: 1,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: DEFAULT_UNACKNOWLEDGED_WINDOW as u32,
            delete: false,
            checksum: false,
            paranoid: false,
        }).unwrap());

        // Ack the (empty) destination Scan page the server sends after config.
        input.extend_from_slice(&encode_frame(3, &Message::Ack {
            acknowledged_id: 1002,
            acknowledged_type: 9,
        }).unwrap());

        // A FileSegment whose file_id was never prepared/batched: this must fail
        // loudly rather than report success while dropping the bytes.
        input.extend_from_slice(&encode_frame(4, &Message::FileSegment {
            file_id: 9_999,
            offset: 0,
            data: b"must not be silently dropped".to_vec(),
        }).unwrap());

        let mut server = Server::new(dst.path());
        let mut output = Vec::new();
        let result = server.run(Cursor::new(&input), &mut output);
        assert!(
            matches!(
                &result,
                Err(ServerError::UnexpectedMessage(msg)) if msg.contains("unregistered file_id")
            ),
            "unregistered FileSegment must be rejected, got {result:?}"
        );
        // No file may be published for the dropped segment.
        let count = dst
            .path()
            .read_dir()
            .map_or(0, |iter| iter.count());
        assert_eq!(count, 0);
    }

    #[test]
    fn default_remote_shell_is_ssh_over_host() {
        let (program, args) = remote_server_command("/dest", None, Some("user@mars"));
        assert_eq!(program, "ssh");
        assert_eq!(
            args,
            ["user@mars", "xsync", "--server", "/dest"]
        );
    }

    #[test]
    fn explicit_rsh_replaces_the_shell_but_preserves_host_and_args() {
        let (program, args) = remote_server_command("/dest", Some("myrsh -oK=1"), Some("host"));
        assert_eq!(program, "myrsh");
        assert_eq!(args, ["-oK=1", "host", "xsync", "--server", "/dest"]);
    }

    #[test]
    fn no_host_runs_an_in_process_local_server() {
        let (program, args) = remote_server_command("/dest", None, None);
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
            let mut server = Server::new(dst_path);
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
            let mut server = Server::new(src_path);
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
            let frame = decoder.read(&mut cursor).expect("all bytes must form valid frames");
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
        }
    }
}
