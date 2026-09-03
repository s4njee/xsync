//! Versioned, bounded frames for remote synchronization.
//!
//! The wire representation in this module is deliberately written out field
//! by field. Rust enum discriminants and serializer layout are not part of the
//! protocol contract; see `protocol.md` for the frozen native sync layout.

use std::io::{self, Read, Write};

use thiserror::Error;

use crate::{HANDSHAKE_MAGIC, PROTOCOL_VERSION};

/// Fixed v1 envelope size in bytes.
pub const FRAME_HEADER_LEN: usize = 32;
/// Maximum encoded payload, including compressed payload bytes.
pub const MAX_COMPLETE_PAYLOAD: usize = 16 * 1024 * 1024;
/// Maximum logical data segment in one message.
pub const MAX_DATA_SEGMENT: usize = 8 * 1024 * 1024;
/// Maximum encoded path length.
pub const MAX_ENCODED_PATH: usize = 1024 * 1024;
/// Default amount of unacknowledged logical data.
pub const DEFAULT_UNACKNOWLEDGED_WINDOW: usize = 32 * 1024 * 1024;
/// Maximum declared decompressed payload length.
pub const MAX_DECOMPRESSED_PAYLOAD: usize = MAX_COMPLETE_PAYLOAD;
/// Maximum entries in a batch or scan message.
pub const MAX_COLLECTION_COUNT: usize = 65_536;
/// Maximum number of exclude patterns in one session.
pub const MAX_EXCLUDE_PATTERNS: usize = 256;
/// Maximum bytes in one exclude pattern.
pub const MAX_EXCLUDE_PATTERN_BYTES: usize = 4 * 1024;
/// Maximum ranges in one resume page.
pub const MAX_RESUME_RANGES: usize = 65_536;
/// Maximum error text length in bytes.
pub const MAX_ERROR_MESSAGE: usize = 64 * 1024;
/// Maximum disjoint message-ID ranges retained by a stateful decoder.
pub const MAX_TRACKED_MESSAGE_ID_RANGES: usize = 1_048_576;

/// Capability bit requesting a data-only receiver session (multi-stream,
/// Story 4.2): the session skips the destination scan and only accepts
/// file/range segment traffic, leaving metadata, prepare/finish steps, and
/// journal ownership to the control session. v1 already carries arbitrary,
/// un-masked capability bits, so using one is not a wire change.
pub const CAP_DATA_ONLY: u32 = 1 << 0;

/// Endpoint supports zstd-compressed data frames.
pub const CAP_ZSTD: u32 = 1 << 1;

/// Peer understands the v2 browse capability set.
pub const CAP_BROWSE_V2: u32 = 1 << 2;

/// Peer understands the version-negotiation handshake extension.
pub const CAP_VERSION_NEGOTIATION: u32 = 1 << 3;

/// Peer understands ordered include/exclude filter rules.
///
/// `exclude_patterns` carries a flat list whose entries are all exclusions, so
/// it cannot express an include rule, whose meaning is entirely its position
/// relative to the excludes. `filter_rules` carries the ordered set instead.
/// Sent only to a peer advertising this, and a peer without it is refused
/// rather than sent the excludes alone, which would transfer a wider set of
/// files than the user asked for.
pub const CAP_FILTER_RULES: u32 = 1 << 4;

/// Peer's permission bits are real Unix modes, not a portable projection.
///
/// `permission_mode` invents `0o755`/`0o644` on hosts without Unix permissions,
/// so comparing an invented mode against a real one would classify every file
/// as permanently drifted. Mode drift is only repaired when **both** ends
/// advertise this.
pub const CAP_UNIX_MODES: u32 = 1 << 5;

/// Peer understands browse v2 metadata verbs (types 36–41).
///
/// `SetPermissionsRequest`, `SetMtimeRequest`, and `ReadLinkRequest` are
/// fail-closed on a decoder that does not know them, so they are sent only
/// when this bit is advertised. An older `xs` without the bit never receives
/// those types; the client degrades to another backend instead of erroring
/// the session.
pub const CAP_BROWSE_META: u32 = 1 << 6;

/// Peer understands the protocol v3 filesystem message table.
///
/// v3 is selected only when both peers advertise this bit together with
/// `CAP_VERSION_NEGOTIATION`; see `v2handshake.md`. The v3 grammar is decoded
/// by `protocol_v3`, is fail-closed, and is never entered from a v2 decode
/// failure. An older peer ignores the bit, so the pair degrades to v2 or v1.
pub const CAP_FS_V3: u32 = 1 << 7;

/// Capability bits with a defined meaning in the current contract.
pub const KNOWN_CAPABILITIES: u32 = CAP_DATA_ONLY
    | CAP_ZSTD
    | CAP_BROWSE_V2
    | CAP_VERSION_NEGOTIATION
    | CAP_BROWSE_META
    | CAP_FS_V3;

/// Select the session grammar before the first non-handshake frame.
///
/// v3 is tried before v2 because it is the strictly richer grammar: a peer
/// advertising `CAP_FS_V3` is expected to advertise `CAP_BROWSE_V2` too, so a
/// v2-only partner still gets browse. Selection happens exactly once and is
/// never revisited; a decode failure in the selected grammar is a session
/// error, never a downgrade.
///
/// Only a `Role::Session` endpoint advertises `CAP_FS_V3`, so a push or pull
/// continues to select v1 exactly as before.
#[must_use]
pub const fn negotiate_protocol_version(local: u32, remote: u32) -> u32 {
    const V3: u32 = CAP_VERSION_NEGOTIATION | CAP_FS_V3;
    const V2: u32 = CAP_VERSION_NEGOTIATION | CAP_BROWSE_V2;
    if local & V3 == V3 && remote & V3 == V3 {
        3
    } else if local & V2 == V2 && remote & V2 == V2 {
        2
    } else {
        1
    }
}

/// Return the intersection of capabilities defined by this contract.
#[must_use]
pub const fn common_capabilities(local: u32, remote: u32) -> u32 {
    local & remote & KNOWN_CAPABILITIES
}

/// Envelope flag indicating that the payload uses zstd compression.
pub const FRAME_FLAG_ZSTD: u8 = 0x01;
const MAX_HANDSHAKE_PAYLOAD: usize = 128;
const MAX_SESSION_CONFIG_PAYLOAD: usize = 8 * 1024;

/// The role of an endpoint in a synchronization session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The endpoint sends source metadata and bytes.
    Source,
    /// The endpoint publishes received metadata and bytes.
    Sink,
    /// Long-lived request/response browse session.
    Session,
}

/// Compression selected for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMode {
    /// Payload bytes are not compressed.
    None,
    /// Payload bytes use zstd level 3.
    Zstd,
}

/// Entry kind carried by scan and batch records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
    /// An unsupported or otherwise special filesystem object.
    Other,
}

/// A metadata record with a raw, length-prefixed relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRecord {
    /// Relative path bytes. The empty path identifies the transfer root.
    pub path: Vec<u8>,
    /// Filesystem object kind.
    pub kind: EntryKind,
    /// Logical file size, zero for non-regular entries.
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
    /// Platform permission bits or a portable permission projection.
    pub mode: u32,
    /// Content or metadata fingerprint selected by the planner.
    pub fingerprint: [u8; 32],
}

/// A file or directory metadata operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataOperation {
    /// Create a directory if it does not exist.
    CreateDirectory,
    /// Apply regular-file metadata.
    SetFile,
    /// Apply directory metadata after child publication.
    SetDirectory,
    /// Create a symbolic link with the supplied target bytes.
    CreateSymlink,
    /// Remove a destination entry.
    Delete,
}

/// A bounded half-open byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Starting byte offset.
    pub offset: u64,
    /// Number of bytes in the range.
    pub length: u64,
}

/// A typed v1 protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Initial role, capabilities, and safety-limit negotiation.
    Handshake {
        /// Endpoint role.
        role: Role,
        /// Capability bitset assigned by the protocol specification.
        capabilities: u32,
        /// Maximum complete payload accepted by this endpoint.
        max_payload: u32,
        /// Maximum data segment accepted by this endpoint.
        max_segment: u32,
        /// Maximum unacknowledged logical data accepted by this endpoint.
        window: u32,
        /// Stable job identifier.
        job_id: [u8; 16],
        /// Compression selected for payloads.
        compression: CompressionMode,
        /// zstd level selected for payloads.
        compression_level: i32,
    },
    /// Per-session strategy and behavior configuration.
    SessionConfig {
        /// Requested parallel remote streams.
        streams: u8,
        /// Logical small-file batch size.
        batch_bytes: u32,
        /// Logical large-file chunk size.
        chunk_bytes: u32,
        /// Maximum unacknowledged data for this session.
        window: u32,
        /// Whether destination-only entries are removed after success.
        delete: bool,
        /// Whether content checksums classify unchanged files.
        checksum: bool,
        /// Whether published output receives readback verification.
        paranoid: bool,
        /// Do discovery and classification without destination mutation.
        dry_run: bool,
        /// Relative-path glob patterns applied by both endpoints.
        ///
        /// Every entry is an exclusion. Empty when `filter_rules` is used.
        exclude_patterns: Vec<Vec<u8>>,
        /// Ordered include/exclude rules, `"+ pattern"` or `"- pattern"`.
        ///
        /// Populated only when the peer advertises [`CAP_FILTER_RULES`], and
        /// mutually exclusive with `exclude_patterns` so a receiver never has
        /// to guess which of the two describes the transfer.
        filter_rules: Vec<Vec<u8>>,
    },
    /// A bounded collection of metadata records.
    FileBatch {
        /// Logical batch identifier.
        batch_id: u64,
        /// Records in this frame.
        entries: Vec<EntryRecord>,
    },
    /// One bounded regular-file data segment.
    FileSegment {
        /// Stable file identity within the session.
        file_id: u64,
        /// Starting byte offset.
        offset: u64,
        /// BLAKE3 digest of this segment's logical bytes.
        digest: [u8; 32],
        /// Segment bytes.
        data: Vec<u8>,
    },
    /// Announces a large file before its ranges are sent.
    LargeFilePrepare {
        /// Stable file identity within the session.
        file_id: u64,
        /// Relative path bytes.
        path: Vec<u8>,
        /// Complete logical file size.
        size: u64,
        /// Nanoseconds since the Unix epoch.
        mtime_ns: i64,
        /// Platform permission bits.
        mode: u32,
        /// Source fingerprint.
        fingerprint: [u8; 32],
    },
    /// Requests or describes one large-file range.
    LargeFileRange {
        /// Stable file identity within the session.
        file_id: u64,
        /// Range being transferred or requested.
        range: ByteRange,
    },
    /// Completes a large-file publication after all ranges are verified.
    LargeFileFinish {
        /// Stable file identity within the session.
        file_id: u64,
        /// Complete logical content digest.
        digest: [u8; 32],
    },
    /// Applies one metadata operation.
    Metadata {
        /// Operation to perform.
        operation: MetadataOperation,
        /// Relative destination path bytes.
        path: Vec<u8>,
        /// Symlink target bytes, only for `CreateSymlink`.
        target: Vec<u8>,
        /// Platform permission bits.
        mode: u32,
        /// Nanoseconds since the Unix epoch.
        mtime_ns: i64,
    },
    /// A bounded scan page.
    Scan {
        /// Logical scan identifier.
        scan_id: u64,
        /// Whether this is the final page for the scan.
        final_page: bool,
        /// Scan records in this frame.
        entries: Vec<EntryRecord>,
    },
    /// Aggregate transfer statistics.
    Stats {
        /// Number of regular files considered.
        files: u64,
        /// Logical bytes considered.
        bytes: u64,
        /// Number of unchanged files.
        skipped: u64,
        /// Number of warnings.
        warnings: u64,
        /// Number of failed entries.
        failed: u64,
    },
    /// Acknowledges a frame or logical operation.
    Ack {
        /// ID being acknowledged.
        acknowledged_id: u64,
        /// Message type or operation class acknowledged.
        acknowledged_type: u8,
    },
    /// A bounded protocol error.
    Error {
        /// Stable machine-readable error code.
        code: u16,
        /// Related message ID, or zero when not applicable.
        related_id: u64,
        /// UTF-8 diagnostic text.
        message: String,
    },
    /// A page of verified ranges used by durable resume.
    ResumePage {
        /// Stable file identity within the session.
        file_id: u64,
        /// Zero-based page number.
        page: u32,
        /// Whether no more pages follow.
        final_page: bool,
        /// Sorted, non-overlapping verified ranges.
        ranges: Vec<ByteRange>,
    },
}

/// A decoded frame and its envelope identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Unique frame ID within the session.
    pub message_id: u64,
    /// Typed frame payload.
    pub message: Message,
}

/// Options controlling frame encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Compress the payload with zstd level 3 when it reduces wire size.
    pub compression: CompressionMode,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            compression: CompressionMode::None,
        }
    }
}

/// Stateful bounded frame decoder.
#[derive(Debug)]
pub struct FrameDecoder {
    seen_id_ranges: Vec<(u64, u64)>,
    last_wire_bytes: usize,
    expected_version: u32,
}

impl FrameDecoder {
    /// Create an empty decoder with no previously observed frame IDs.
    #[must_use]
    pub fn new() -> Self {
        Self {
            seen_id_ranges: Vec::new(),
            last_wire_bytes: 0,
            expected_version: PROTOCOL_VERSION,
        }
    }

    /// Create a decoder committed to one envelope version.
    ///
    /// The version is intentionally fixed for the decoder's lifetime. A
    /// session must select its grammar during the handshake rather than
    /// downgrading after a frame fails to decode.
    #[must_use]
    pub fn for_version(version: u32) -> Self {
        Self {
            seen_id_ranges: Vec::new(),
            last_wire_bytes: 0,
            expected_version: version,
        }
    }

    /// Return the encoded size of the most recently read frame.
    #[must_use]
    pub fn last_wire_bytes(&self) -> usize {
        self.last_wire_bytes
    }

    /// Decode one complete in-memory frame and reject a duplicate message ID.
    ///
    /// # Errors
    /// Returns [`ProtocolError`] for malformed envelopes, payloads, limits, or
    /// duplicate IDs.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Frame, ProtocolError> {
        let frame = decode_frame_for_version(bytes, self.expected_version)?;
        self.remember(frame.message_id)?;
        Ok(frame)
    }

    /// Read and decode one frame from a byte stream.
    ///
    /// The fixed header is validated before the bounded body allocation and
    /// read. A stream may contain subsequent frames; only one is consumed.
    ///
    /// # Errors
    /// Returns [`ProtocolError`] for malformed envelopes, payloads, limits,
    /// duplicate IDs, or I/O failures.
    pub fn read<R: Read>(&mut self, reader: &mut R) -> Result<Frame, ProtocolError> {
        let mut header = [0_u8; FRAME_HEADER_LEN];
        reader
            .read_exact(&mut header)
            .map_err(ProtocolError::Read)?;
        let parsed = parse_header(&header, self.expected_version)?;
        let payload_len =
            usize::try_from(parsed.payload_len).map_err(|_| ProtocolError::LengthOverflow)?;
        let mut payload = vec![0_u8; payload_len];
        reader
            .read_exact(&mut payload)
            .map_err(ProtocolError::Read)?;
        let frame = decode_parts(parsed, &payload)?;
        self.last_wire_bytes = FRAME_HEADER_LEN.saturating_add(payload.len());
        self.remember(frame.message_id)?;
        Ok(frame)
    }

    fn remember(&mut self, message_id: u64) -> Result<(), ProtocolError> {
        let insertion = self.seen_id_ranges.binary_search_by(|(start, end)| {
            if message_id < *start {
                std::cmp::Ordering::Greater
            } else if message_id > *end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        if insertion.is_ok() {
            return Err(ProtocolError::DuplicateId { message_id });
        }
        let index = insertion.unwrap_err();
        let joins_previous = index > 0
            && self.seen_id_ranges[index - 1]
                .1
                .checked_add(1)
                .is_some_and(|end| end == message_id);
        let joins_next = index < self.seen_id_ranges.len()
            && self.seen_id_ranges[index].0 == message_id.saturating_add(1);
        match (joins_previous, joins_next) {
            (true, true) => {
                let next_end = self.seen_id_ranges[index].1;
                self.seen_id_ranges[index - 1].1 = next_end;
                self.seen_id_ranges.remove(index);
            }
            (true, false) => self.seen_id_ranges[index - 1].1 = message_id,
            (false, true) => self.seen_id_ranges[index].0 = message_id,
            (false, false) => {
                if self.seen_id_ranges.len() >= MAX_TRACKED_MESSAGE_ID_RANGES {
                    return Err(ProtocolError::SessionBudget {
                        detail: "message ID tracking range limit exceeded",
                    });
                }
                self.seen_id_ranges.insert(index, (message_id, message_id));
            }
        }
        Ok(())
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode one uncompressed v1 frame.
///
/// # Errors
/// Returns [`ProtocolError`] when the message exceeds a protocol limit or
/// cannot be represented by the checked field layout.
pub fn encode_frame(message_id: u64, message: &Message) -> Result<Vec<u8>, ProtocolError> {
    encode_frame_with_version(message_id, message, PROTOCOL_VERSION)
}

/// Encode one frame using an explicitly selected envelope version.
///
/// The caller must select the version during the handshake. This function does
/// not perform fallback or reinterpret v2 payloads as v1 messages.
///
/// # Errors
/// Returns [`ProtocolError`] when the message exceeds a protocol limit or the
/// version is invalid.
pub fn encode_frame_with_version(
    message_id: u64,
    message: &Message,
    version: u32,
) -> Result<Vec<u8>, ProtocolError> {
    encode_frame_with_options_and_version(message_id, message, EncodeOptions::default(), version)
}

/// Encode one v1 frame with an optional zstd payload.
///
/// # Errors
/// Returns [`ProtocolError`] when the message exceeds a protocol limit,
/// compression fails, or cannot be represented by the checked field layout.
pub fn encode_frame_with_options(
    message_id: u64,
    message: &Message,
    options: EncodeOptions,
) -> Result<Vec<u8>, ProtocolError> {
    let payload_capacity = validate_message(message)?;
    let mut payload = Vec::with_capacity(payload_capacity);
    encode_message_payload(message, &mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: payload.len(),
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }

    encode_payload_frame(
        message_id,
        message,
        &payload,
        options.compression,
        3,
        PROTOCOL_VERSION,
    )
}

fn encode_frame_with_options_and_version(
    message_id: u64,
    message: &Message,
    options: EncodeOptions,
    version: u32,
) -> Result<Vec<u8>, ProtocolError> {
    if !matches!(version, 1 | 2) {
        return Err(ProtocolError::VersionMismatch {
            local: PROTOCOL_VERSION,
            remote: version,
        });
    }
    let payload_capacity = validate_message(message)?;
    let mut payload = Vec::with_capacity(payload_capacity);
    encode_message_payload(message, &mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: payload.len(),
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    encode_payload_frame(
        message_id,
        message,
        &payload,
        options.compression,
        3,
        version,
    )
}

/// Encode a frame using a caller-selected zstd level.
///
/// # Errors
/// Returns [`ProtocolError`] when the message exceeds a protocol limit or
/// compression fails.
pub fn encode_frame_with_compression(
    message_id: u64,
    message: &Message,
    compression: CompressionMode,
    level: i32,
) -> Result<Vec<u8>, ProtocolError> {
    let payload_capacity = validate_message(message)?;
    let mut payload = Vec::with_capacity(payload_capacity);
    encode_message_payload(message, &mut payload)?;
    if payload.len() > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: payload.len(),
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    encode_payload_frame(
        message_id,
        message,
        &payload,
        compression,
        level,
        PROTOCOL_VERSION,
    )
}

fn encode_payload_frame(
    message_id: u64,
    message: &Message,
    payload: &[u8],
    compression: CompressionMode,
    level: i32,
    version: u32,
) -> Result<Vec<u8>, ProtocolError> {
    let (flags, wire_payload) = if compression == CompressionMode::Zstd {
        let decision =
            crate::compression::decide([payload], level).map_err(ProtocolError::Compression)?;
        if decision.use_compression {
            let compressed = compress_zstd(payload, level)?;
            (FRAME_FLAG_ZSTD, compressed)
        } else {
            (0, payload.to_owned())
        }
    } else {
        (0, payload.to_owned())
    };
    if wire_payload.len() > MAX_COMPLETE_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: wire_payload.len(),
            maximum: MAX_COMPLETE_PAYLOAD,
        });
    }

    let payload_len =
        u32::try_from(wire_payload.len()).map_err(|_| ProtocolError::LengthOverflow)?;
    let decoded_len = u32::try_from(payload.len()).map_err(|_| ProtocolError::LengthOverflow)?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + wire_payload.len());
    frame.extend_from_slice(HANDSHAKE_MAGIC);
    let header_len = u16::try_from(FRAME_HEADER_LEN).map_err(|_| ProtocolError::LengthOverflow)?;
    frame.extend_from_slice(&header_len.to_le_bytes());
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&version.to_le_bytes());
    frame.push(message_type(message).as_u8());
    frame.push(flags);
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&decoded_len.to_le_bytes());
    frame.extend_from_slice(&message_id.to_le_bytes());
    debug_assert_eq!(frame.len(), FRAME_HEADER_LEN);
    frame.extend_from_slice(&wire_payload);
    Ok(frame)
}

/// Write one frame without buffering another copy of its wire payload.
///
/// The frame is bounded to [`MAX_COMPLETE_PAYLOAD`]. This helper is intended
/// for transports that already have a writer and should not be interpreted as
/// support for a frame larger than the protocol limit.
///
/// # Errors
/// Returns [`ProtocolError`] for encoding or I/O failures.
pub fn write_frame<W: Write>(
    writer: &mut W,
    message_id: u64,
    message: &Message,
    options: EncodeOptions,
) -> Result<(), ProtocolError> {
    let frame = encode_frame_with_options(message_id, message, options)?;
    writer.write_all(&frame).map_err(ProtocolError::Write)
}

/// Decode one complete frame from memory.
///
/// # Errors
/// Returns [`ProtocolError`] for malformed envelopes, payloads, limits, or
/// trailing bytes.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame, ProtocolError> {
    decode_frame_for_version(bytes, PROTOCOL_VERSION)
}

/// Decode one complete frame while requiring the selected envelope version.
///
/// # Errors
/// Returns [`ProtocolError::VersionMismatch`] when the frame belongs to a
/// different session grammar. The caller must not retry it with another
/// version.
pub fn decode_frame_for_version(bytes: &[u8], version: u32) -> Result<Frame, ProtocolError> {
    if bytes.len() < FRAME_HEADER_LEN {
        return Err(ProtocolError::Truncated {
            expected: FRAME_HEADER_LEN,
            actual: bytes.len(),
        });
    }
    let parsed = parse_header(&bytes[..FRAME_HEADER_LEN], version)?;
    let payload_len =
        usize::try_from(parsed.payload_len).map_err(|_| ProtocolError::LengthOverflow)?;
    let total = FRAME_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(ProtocolError::LengthOverflow)?;
    if bytes.len() < total {
        return Err(ProtocolError::Truncated {
            expected: total,
            actual: bytes.len(),
        });
    }
    if bytes.len() > total {
        return Err(ProtocolError::TrailingBytes {
            count: bytes.len() - total,
        });
    }
    decode_parts(parsed, &bytes[FRAME_HEADER_LEN..total])
}

#[derive(Debug, Clone, Copy)]
struct ParsedHeader {
    message_type: u8,
    flags: u8,
    payload_len: u32,
    decoded_len: u32,
    message_id: u64,
}

fn parse_header(header: &[u8], expected_version: u32) -> Result<ParsedHeader, ProtocolError> {
    if header.len() != FRAME_HEADER_LEN {
        return Err(ProtocolError::InvalidHeaderLength {
            declared: header.len(),
        });
    }
    if &header[..4] != HANDSHAKE_MAGIC {
        return Err(ProtocolError::InvalidMagic {
            actual: header[..4].try_into().unwrap_or([0; 4]),
        });
    }
    let header_len = u16::from_le_bytes([header[4], header[5]]) as usize;
    if header_len != FRAME_HEADER_LEN {
        return Err(ProtocolError::InvalidHeaderLength {
            declared: header_len,
        });
    }
    if u16::from_le_bytes([header[6], header[7]]) != 0 {
        return Err(ProtocolError::InvalidFlags {
            flags: 0,
            detail: "reserved header bits are non-zero",
        });
    }
    let version = u32::from_le_bytes(header[8..12].try_into().unwrap_or([0; 4]));
    if version != expected_version {
        return Err(ProtocolError::VersionMismatch {
            local: expected_version,
            remote: version,
        });
    }
    let message_type = header[12];
    if !MessageType::is_known(message_type) {
        return Err(ProtocolError::UnknownType { message_type });
    }
    let flags = header[13];
    if flags & !FRAME_FLAG_ZSTD != 0 {
        return Err(ProtocolError::InvalidFlags {
            flags,
            detail: "unknown frame flags",
        });
    }
    if u16::from_le_bytes([header[14], header[15]]) != 0 {
        return Err(ProtocolError::InvalidFlags {
            flags,
            detail: "reserved frame bits are non-zero",
        });
    }
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap_or([0; 4]));
    let decoded_len = u32::from_le_bytes(header[20..24].try_into().unwrap_or([0; 4]));
    let payload_limit = u32::try_from(MAX_COMPLETE_PAYLOAD).unwrap_or(u32::MAX);
    if payload_len > payload_limit {
        return Err(ProtocolError::PayloadTooLarge {
            length: payload_len as usize,
            maximum: MAX_COMPLETE_PAYLOAD,
        });
    }
    if decoded_len > u32::try_from(MAX_DECOMPRESSED_PAYLOAD).unwrap_or(u32::MAX) {
        return Err(ProtocolError::DecompressedTooLarge {
            length: decoded_len as usize,
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    if flags & FRAME_FLAG_ZSTD == 0 && payload_len != decoded_len {
        return Err(ProtocolError::LengthMismatch {
            encoded: payload_len,
            decoded: decoded_len,
        });
    }
    let message_id = u64::from_le_bytes(header[24..32].try_into().unwrap_or([0; 8]));
    Ok(ParsedHeader {
        message_type,
        flags,
        payload_len,
        decoded_len,
        message_id,
    })
}

fn compress_zstd(payload: &[u8], level: i32) -> Result<Vec<u8>, ProtocolError> {
    zstd::bulk::compress(payload, level).map_err(ProtocolError::Compression)
}

/// Select a compression mode supported by both endpoints.
///
/// The requested mode is only a preference; capabilities are authoritative so
/// an older or feature-reduced peer safely falls back to uncompressed frames.
#[must_use]
pub const fn negotiate_compression(
    requested: CompressionMode,
    local_capabilities: u32,
    remote_capabilities: u32,
) -> CompressionMode {
    match requested {
        CompressionMode::Zstd
            if local_capabilities & CAP_ZSTD != 0 && remote_capabilities & CAP_ZSTD != 0 =>
        {
            CompressionMode::Zstd
        }
        CompressionMode::Zstd | CompressionMode::None => CompressionMode::None,
    }
}

fn decompress_zstd(payload: &[u8], decoded_len: usize) -> Result<Vec<u8>, ProtocolError> {
    zstd::bulk::decompress(payload, decoded_len).map_err(ProtocolError::Decompression)
}

fn decode_parts(header: ParsedHeader, wire_payload: &[u8]) -> Result<Frame, ProtocolError> {
    if wire_payload.len()
        != usize::try_from(header.payload_len).map_err(|_| ProtocolError::LengthOverflow)?
    {
        return Err(ProtocolError::LengthMismatch {
            encoded: header.payload_len,
            decoded: u32::try_from(wire_payload.len()).unwrap_or(u32::MAX),
        });
    }
    let payload = if header.flags & FRAME_FLAG_ZSTD != 0 {
        decompress_zstd(
            wire_payload,
            usize::try_from(header.decoded_len).map_err(|_| ProtocolError::LengthOverflow)?,
        )?
    } else {
        wire_payload.to_vec()
    };
    if payload.len()
        != usize::try_from(header.decoded_len).map_err(|_| ProtocolError::LengthOverflow)?
    {
        return Err(ProtocolError::LengthMismatch {
            encoded: header.payload_len,
            decoded: u32::try_from(payload.len()).unwrap_or(u32::MAX),
        });
    }
    let message = decode_message_payload(header.message_type, &payload)?;
    Ok(Frame {
        message_id: header.message_id,
        message,
    })
}

/// Frozen v1 message type assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// Handshake message.
    Handshake = 1,
    /// Session configuration message.
    SessionConfig = 2,
    /// Small-file batch message.
    FileBatch = 3,
    /// Regular-file data segment message.
    FileSegment = 4,
    /// Large-file preparation message.
    LargeFilePrepare = 5,
    /// Large-file range message.
    LargeFileRange = 6,
    /// Large-file completion message.
    LargeFileFinish = 7,
    /// Metadata operation message.
    Metadata = 8,
    /// Scan page message.
    Scan = 9,
    /// Statistics message.
    Stats = 10,
    /// Acknowledgement message.
    Ack = 11,
    /// Error message.
    Error = 12,
    /// Resume page message.
    ResumePage = 13,
}

impl MessageType {
    /// Return whether a wire type is assigned by v1.
    #[must_use]
    pub fn is_known(value: u8) -> bool {
        (1..=13).contains(&value)
    }

    /// Decode a frozen wire type assignment.
    ///
    /// # Errors
    /// Returns [`ProtocolError::UnknownType`] for an unassigned value.
    pub fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::SessionConfig),
            3 => Ok(Self::FileBatch),
            4 => Ok(Self::FileSegment),
            5 => Ok(Self::LargeFilePrepare),
            6 => Ok(Self::LargeFileRange),
            7 => Ok(Self::LargeFileFinish),
            8 => Ok(Self::Metadata),
            9 => Ok(Self::Scan),
            10 => Ok(Self::Stats),
            11 => Ok(Self::Ack),
            12 => Ok(Self::Error),
            13 => Ok(Self::ResumePage),
            _ => Err(ProtocolError::UnknownType {
                message_type: value,
            }),
        }
    }

    /// Return the assigned wire byte.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

fn message_type(message: &Message) -> MessageType {
    match message {
        Message::Handshake { .. } => MessageType::Handshake,
        Message::SessionConfig { .. } => MessageType::SessionConfig,
        Message::FileBatch { .. } => MessageType::FileBatch,
        Message::FileSegment { .. } => MessageType::FileSegment,
        Message::LargeFilePrepare { .. } => MessageType::LargeFilePrepare,
        Message::LargeFileRange { .. } => MessageType::LargeFileRange,
        Message::LargeFileFinish { .. } => MessageType::LargeFileFinish,
        Message::Metadata { .. } => MessageType::Metadata,
        Message::Scan { .. } => MessageType::Scan,
        Message::Stats { .. } => MessageType::Stats,
        Message::Ack { .. } => MessageType::Ack,
        Message::Error { .. } => MessageType::Error,
        Message::ResumePage { .. } => MessageType::ResumePage,
    }
}

#[allow(clippy::match_same_arms, clippy::too_many_lines)]
fn validate_message(message: &Message) -> Result<usize, ProtocolError> {
    let mut size;
    match message {
        Message::Handshake {
            max_payload,
            max_segment,
            window,
            compression,
            compression_level,
            ..
        } => {
            validate_handshake(*max_payload, *max_segment, *window)?;
            validate_compression(*compression, *compression_level)?;
            size = 38;
        }
        Message::SessionConfig {
            streams,
            window,
            exclude_patterns,
            filter_rules,
            ..
        } => {
            if *streams == 0 || *streams > 16 {
                return Err(ProtocolError::InvalidValue { field: "streams" });
            }
            validate_window(*window)?;
            size = 19;
            if exclude_patterns.len() > MAX_EXCLUDE_PATTERNS {
                return Err(ProtocolError::CountTooLarge {
                    count: exclude_patterns.len(),
                    maximum: MAX_EXCLUDE_PATTERNS,
                });
            }
            for pattern in exclude_patterns {
                if pattern.len() > MAX_EXCLUDE_PATTERN_BYTES {
                    return Err(ProtocolError::PayloadTooLarge {
                        length: pattern.len(),
                        maximum: MAX_EXCLUDE_PATTERN_BYTES,
                    });
                }
                size = add_payload_size(size, prefixed_len(pattern.len())?)?;
            }
            if filter_rules.len() > MAX_EXCLUDE_PATTERNS {
                return Err(ProtocolError::CountTooLarge {
                    count: filter_rules.len(),
                    maximum: MAX_EXCLUDE_PATTERNS,
                });
            }
            size = add_payload_size(size, 2)?;
            for rule in filter_rules {
                if rule.len() > MAX_EXCLUDE_PATTERN_BYTES {
                    return Err(ProtocolError::PayloadTooLarge {
                        length: rule.len(),
                        maximum: MAX_EXCLUDE_PATTERN_BYTES,
                    });
                }
                size = add_payload_size(size, prefixed_len(rule.len())?)?;
            }
        }
        Message::FileBatch { entries, .. } => {
            if entries.len() > MAX_COLLECTION_COUNT {
                return Err(ProtocolError::CountTooLarge {
                    count: entries.len(),
                    maximum: MAX_COLLECTION_COUNT,
                });
            }
            size = 12;
            for entry in entries {
                size = add_payload_size(size, entry_wire_len(entry)?)?;
            }
        }
        Message::FileSegment { data, .. } => {
            if data.len() > MAX_DATA_SEGMENT {
                return Err(ProtocolError::DataSegmentTooLarge { length: data.len() });
            }
            size = add_payload_size(20 + 32, data.len())?;
        }
        Message::LargeFilePrepare { path, .. } => {
            validate_path_length(path)?;
            size = add_payload_size(8, prefixed_len(path.len())?)?;
            size = add_payload_size(size, 8 + 8 + 4 + 32)?;
        }
        Message::LargeFileRange { range, .. } => {
            validate_range(*range)?;
            size = 24;
        }
        Message::LargeFileFinish { .. } => {
            size = 40;
        }
        Message::Metadata {
            operation,
            path,
            target,
            ..
        } => {
            validate_path_length(path)?;
            validate_path_length(target)?;
            if *operation != MetadataOperation::CreateSymlink && !target.is_empty() {
                return Err(ProtocolError::InvalidValue {
                    field: "metadata target",
                });
            }
            size = add_payload_size(1, prefixed_len(path.len())?)?;
            size = add_payload_size(size, prefixed_len(target.len())? + 4 + 8)?;
        }
        Message::Scan { entries, .. } => {
            if entries.len() > MAX_COLLECTION_COUNT {
                return Err(ProtocolError::CountTooLarge {
                    count: entries.len(),
                    maximum: MAX_COLLECTION_COUNT,
                });
            }
            size = 13;
            for entry in entries {
                size = add_payload_size(size, entry_wire_len(entry)?)?;
            }
        }
        Message::Stats { .. } => {
            size = 40;
        }
        Message::Ack {
            acknowledged_type, ..
        } => {
            if !MessageType::is_known(*acknowledged_type) {
                return Err(ProtocolError::UnknownType {
                    message_type: *acknowledged_type,
                });
            }
            size = 9;
        }
        Message::Error { message, .. } => {
            if message.len() > MAX_ERROR_MESSAGE {
                return Err(ProtocolError::PayloadTooLarge {
                    length: message.len(),
                    maximum: MAX_ERROR_MESSAGE,
                });
            }
            size = add_payload_size(10, prefixed_len(message.len())?)?;
        }
        Message::ResumePage { ranges, .. } => {
            validate_ranges(ranges)?;
            size = 17;
            size = add_payload_size(
                size,
                ranges
                    .len()
                    .checked_mul(16)
                    .ok_or(ProtocolError::LengthOverflow)?,
            )?;
        }
    }
    if size > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: size,
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    Ok(size)
}

fn entry_wire_len(entry: &EntryRecord) -> Result<usize, ProtocolError> {
    validate_path_length(&entry.path)?;
    let size = prefixed_len(entry.path.len())?;
    add_payload_size(size, 1 + 8 + 8 + 4 + 32)
}

fn validate_path_length(path: &[u8]) -> Result<(), ProtocolError> {
    if path.len() > MAX_ENCODED_PATH {
        return Err(ProtocolError::PayloadTooLarge {
            length: path.len(),
            maximum: MAX_ENCODED_PATH,
        });
    }
    Ok(())
}

fn prefixed_len(length: usize) -> Result<usize, ProtocolError> {
    4_usize
        .checked_add(length)
        .ok_or(ProtocolError::LengthOverflow)
}

fn add_payload_size(current: usize, addition: usize) -> Result<usize, ProtocolError> {
    let size = current
        .checked_add(addition)
        .ok_or(ProtocolError::LengthOverflow)?;
    if size > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: size,
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    Ok(size)
}

#[allow(clippy::too_many_lines)]
fn encode_message_payload(message: &Message, output: &mut Vec<u8>) -> Result<(), ProtocolError> {
    let mut writer = Writer::new(output);
    match message {
        Message::Handshake {
            role,
            capabilities,
            max_payload,
            max_segment,
            window,
            job_id,
            compression,
            compression_level,
        } => {
            writer.u8(role_to_wire(*role));
            writer.u32(*capabilities);
            writer.u32(*max_payload);
            writer.u32(*max_segment);
            writer.u32(*window);
            writer.array(job_id);
            writer.u8(compression_to_wire(*compression));
            writer.i32(*compression_level);
            validate_handshake(*max_payload, *max_segment, *window)?;
            validate_compression(*compression, *compression_level)?;
        }
        Message::SessionConfig {
            streams,
            batch_bytes,
            chunk_bytes,
            window,
            delete,
            checksum,
            paranoid,
            dry_run,
            exclude_patterns,
            filter_rules,
        } => {
            if *streams == 0 || *streams > 16 {
                return Err(ProtocolError::InvalidValue { field: "streams" });
            }
            validate_window(*window)?;
            writer.u8(*streams);
            writer.u32(*batch_bytes);
            writer.u32(*chunk_bytes);
            writer.u32(*window);
            writer.bool(*delete);
            writer.bool(*checksum);
            writer.bool(*paranoid);
            writer.bool(*dry_run);
            writer.count(exclude_patterns.len(), MAX_EXCLUDE_PATTERNS)?;
            for pattern in exclude_patterns {
                writer.blob(pattern, MAX_EXCLUDE_PATTERN_BYTES)?;
            }
            writer.count(filter_rules.len(), MAX_EXCLUDE_PATTERNS)?;
            for rule in filter_rules {
                writer.blob(rule, MAX_EXCLUDE_PATTERN_BYTES)?;
            }
        }
        Message::FileBatch { batch_id, entries } => {
            writer.u64(*batch_id);
            writer.count(entries.len(), MAX_COLLECTION_COUNT)?;
            for entry in entries {
                encode_entry(&mut writer, entry)?;
            }
        }
        Message::FileSegment {
            file_id,
            offset,
            digest,
            data,
        } => {
            if data.len() > MAX_DATA_SEGMENT {
                return Err(ProtocolError::DataSegmentTooLarge { length: data.len() });
            }
            writer.u64(*file_id);
            writer.u64(*offset);
            writer.array(digest);
            writer.blob(data, MAX_DATA_SEGMENT)?;
        }
        Message::LargeFilePrepare {
            file_id,
            path,
            size,
            mtime_ns,
            mode,
            fingerprint,
        } => {
            writer.u64(*file_id);
            writer.path(path)?;
            writer.u64(*size);
            writer.i64(*mtime_ns);
            writer.u32(*mode);
            writer.array(fingerprint);
        }
        Message::LargeFileRange { file_id, range } => {
            validate_range(*range)?;
            writer.u64(*file_id);
            encode_range(&mut writer, *range);
        }
        Message::LargeFileFinish { file_id, digest } => {
            writer.u64(*file_id);
            writer.array(digest);
        }
        Message::Metadata {
            operation,
            path,
            target,
            mode,
            mtime_ns,
        } => {
            if *operation != MetadataOperation::CreateSymlink && !target.is_empty() {
                return Err(ProtocolError::InvalidValue {
                    field: "metadata target",
                });
            }
            writer.u8(metadata_operation_to_wire(*operation));
            writer.path(path)?;
            writer.path(target)?;
            writer.u32(*mode);
            writer.i64(*mtime_ns);
        }
        Message::Scan {
            scan_id,
            final_page,
            entries,
        } => {
            writer.u64(*scan_id);
            writer.bool(*final_page);
            writer.count(entries.len(), MAX_COLLECTION_COUNT)?;
            for entry in entries {
                encode_entry(&mut writer, entry)?;
            }
        }
        Message::Stats {
            files,
            bytes,
            skipped,
            warnings,
            failed,
        } => {
            writer.u64(*files);
            writer.u64(*bytes);
            writer.u64(*skipped);
            writer.u64(*warnings);
            writer.u64(*failed);
        }
        Message::Ack {
            acknowledged_id,
            acknowledged_type,
        } => {
            if !MessageType::is_known(*acknowledged_type) {
                return Err(ProtocolError::UnknownType {
                    message_type: *acknowledged_type,
                });
            }
            writer.u64(*acknowledged_id);
            writer.u8(*acknowledged_type);
        }
        Message::Error {
            code,
            related_id,
            message,
        } => {
            writer.u16(*code);
            writer.u64(*related_id);
            writer.string(message, MAX_ERROR_MESSAGE)?;
        }
        Message::ResumePage {
            file_id,
            page,
            final_page,
            ranges,
        } => {
            validate_ranges(ranges)?;
            writer.u64(*file_id);
            writer.u32(*page);
            writer.bool(*final_page);
            writer.count(ranges.len(), MAX_RESUME_RANGES)?;
            for range in ranges {
                encode_range(&mut writer, *range);
            }
        }
    }
    if output.len() > MAX_DECOMPRESSED_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            length: output.len(),
            maximum: MAX_DECOMPRESSED_PAYLOAD,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn decode_message_payload(message_type: u8, payload: &[u8]) -> Result<Message, ProtocolError> {
    let kind = MessageType::from_wire(message_type)?;
    let mut reader = Reader::new(payload);
    let message = match kind {
        MessageType::Handshake => {
            let role = role_from_wire(reader.u8()?)?;
            let capabilities = reader.u32()?;
            let max_payload = reader.u32()?;
            let max_segment = reader.u32()?;
            let window = reader.u32()?;
            let job_id = reader.array::<16>()?;
            let compression = compression_from_wire(reader.u8()?)?;
            let compression_level = reader.i32()?;
            validate_handshake(max_payload, max_segment, window)?;
            if payload.len() > MAX_HANDSHAKE_PAYLOAD {
                return Err(ProtocolError::PayloadTooLarge {
                    length: payload.len(),
                    maximum: MAX_HANDSHAKE_PAYLOAD,
                });
            }
            Message::Handshake {
                role,
                capabilities,
                max_payload,
                max_segment,
                window,
                job_id,
                compression,
                compression_level,
            }
        }
        MessageType::SessionConfig => {
            let streams = reader.u8()?;
            let batch_bytes = reader.u32()?;
            let chunk_bytes = reader.u32()?;
            let window = reader.u32()?;
            let delete = reader.bool()?;
            let checksum = reader.bool()?;
            let paranoid = reader.bool()?;
            let dry_run = reader.bool()?;
            let count = reader.count(MAX_EXCLUDE_PATTERNS)?;
            let mut exclude_patterns = Vec::with_capacity(count);
            for _ in 0..count {
                exclude_patterns.push(reader.blob(MAX_EXCLUDE_PATTERN_BYTES)?);
            }
            let rule_count = reader.count(MAX_EXCLUDE_PATTERNS)?;
            let mut filter_rules = Vec::with_capacity(rule_count);
            for _ in 0..rule_count {
                filter_rules.push(reader.blob(MAX_EXCLUDE_PATTERN_BYTES)?);
            }
            if !exclude_patterns.is_empty() && !filter_rules.is_empty() {
                // The two describe the same thing in different ways. A receiver
                // that had to choose could silently apply the wrong one.
                return Err(ProtocolError::InvalidValue {
                    field: "filter_rules",
                });
            }
            if payload.len() > MAX_SESSION_CONFIG_PAYLOAD {
                return Err(ProtocolError::PayloadTooLarge {
                    length: payload.len(),
                    maximum: MAX_SESSION_CONFIG_PAYLOAD,
                });
            }
            if streams == 0 || streams > 16 {
                return Err(ProtocolError::InvalidValue { field: "streams" });
            }
            validate_window(window)?;
            Message::SessionConfig {
                streams,
                batch_bytes,
                chunk_bytes,
                window,
                delete,
                checksum,
                paranoid,
                dry_run,
                exclude_patterns,
                filter_rules,
            }
        }
        MessageType::FileBatch => {
            let batch_id = reader.u64()?;
            let count = reader.count(MAX_COLLECTION_COUNT)?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(decode_entry(&mut reader)?);
            }
            Message::FileBatch { batch_id, entries }
        }
        MessageType::FileSegment => {
            let file_id = reader.u64()?;
            let offset = reader.u64()?;
            let digest = reader.array::<32>()?;
            let data = reader.blob(MAX_DATA_SEGMENT)?;
            Message::FileSegment {
                file_id,
                offset,
                digest,
                data,
            }
        }
        MessageType::LargeFilePrepare => Message::LargeFilePrepare {
            file_id: reader.u64()?,
            path: reader.path()?,
            size: reader.u64()?,
            mtime_ns: reader.i64()?,
            mode: reader.u32()?,
            fingerprint: reader.array::<32>()?,
        },
        MessageType::LargeFileRange => {
            let file_id = reader.u64()?;
            let range = decode_range(&mut reader)?;
            validate_range(range)?;
            Message::LargeFileRange { file_id, range }
        }
        MessageType::LargeFileFinish => Message::LargeFileFinish {
            file_id: reader.u64()?,
            digest: reader.array::<32>()?,
        },
        MessageType::Metadata => {
            let operation = metadata_operation_from_wire(reader.u8()?)?;
            let path = reader.path()?;
            let target = reader.path()?;
            if operation != MetadataOperation::CreateSymlink && !target.is_empty() {
                return Err(ProtocolError::InvalidValue {
                    field: "metadata target",
                });
            }
            Message::Metadata {
                operation,
                path,
                target,
                mode: reader.u32()?,
                mtime_ns: reader.i64()?,
            }
        }
        MessageType::Scan => {
            let scan_id = reader.u64()?;
            let final_page = reader.bool()?;
            let count = reader.count(MAX_COLLECTION_COUNT)?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(decode_entry(&mut reader)?);
            }
            Message::Scan {
                scan_id,
                final_page,
                entries,
            }
        }
        MessageType::Stats => Message::Stats {
            files: reader.u64()?,
            bytes: reader.u64()?,
            skipped: reader.u64()?,
            warnings: reader.u64()?,
            failed: reader.u64()?,
        },
        MessageType::Ack => {
            let acknowledged_id = reader.u64()?;
            let acknowledged_type = reader.u8()?;
            if !MessageType::is_known(acknowledged_type) {
                return Err(ProtocolError::UnknownType {
                    message_type: acknowledged_type,
                });
            }
            Message::Ack {
                acknowledged_id,
                acknowledged_type,
            }
        }
        MessageType::Error => Message::Error {
            code: reader.u16()?,
            related_id: reader.u64()?,
            message: reader.string(MAX_ERROR_MESSAGE)?,
        },
        MessageType::ResumePage => {
            let file_id = reader.u64()?;
            let page = reader.u32()?;
            let final_page = reader.bool()?;
            let count = reader.count(MAX_RESUME_RANGES)?;
            let mut ranges = Vec::with_capacity(count);
            for _ in 0..count {
                ranges.push(decode_range(&mut reader)?);
            }
            validate_ranges(&ranges)?;
            Message::ResumePage {
                file_id,
                page,
                final_page,
                ranges,
            }
        }
    };
    reader.finish()?;
    Ok(message)
}

fn encode_entry(writer: &mut Writer<'_>, entry: &EntryRecord) -> Result<(), ProtocolError> {
    writer.path(&entry.path)?;
    writer.u8(entry_kind_to_wire(entry.kind));
    writer.u64(entry.size);
    writer.i64(entry.mtime_ns);
    writer.u32(entry.mode);
    writer.array(&entry.fingerprint);
    Ok(())
}

fn decode_entry(reader: &mut Reader<'_>) -> Result<EntryRecord, ProtocolError> {
    Ok(EntryRecord {
        path: reader.path()?,
        kind: entry_kind_from_wire(reader.u8()?)?,
        size: reader.u64()?,
        mtime_ns: reader.i64()?,
        mode: reader.u32()?,
        fingerprint: reader.array::<32>()?,
    })
}

fn encode_range(writer: &mut Writer<'_>, range: ByteRange) {
    writer.u64(range.offset);
    writer.u64(range.length);
}

fn decode_range(reader: &mut Reader<'_>) -> Result<ByteRange, ProtocolError> {
    Ok(ByteRange {
        offset: reader.u64()?,
        length: reader.u64()?,
    })
}

fn validate_range(range: ByteRange) -> Result<(), ProtocolError> {
    if range.length == 0 || range.length > MAX_DATA_SEGMENT as u64 {
        return Err(ProtocolError::InvalidRange(range));
    }
    range
        .offset
        .checked_add(range.length)
        .ok_or(ProtocolError::InvalidRange(range))?;
    Ok(())
}

fn validate_ranges(ranges: &[ByteRange]) -> Result<(), ProtocolError> {
    if ranges.len() > MAX_RESUME_RANGES {
        return Err(ProtocolError::CountTooLarge {
            count: ranges.len(),
            maximum: MAX_RESUME_RANGES,
        });
    }
    let mut previous_end = 0_u64;
    for (index, range) in ranges.iter().copied().enumerate() {
        validate_range(range)?;
        if index > 0 && range.offset < previous_end {
            return Err(ProtocolError::OverlappingRanges);
        }
        previous_end = range
            .offset
            .checked_add(range.length)
            .ok_or(ProtocolError::InvalidRange(range))?;
    }
    Ok(())
}

fn validate_handshake(
    max_payload: u32,
    max_segment: u32,
    window: u32,
) -> Result<(), ProtocolError> {
    if max_payload == 0 || max_payload as usize > MAX_COMPLETE_PAYLOAD {
        return Err(ProtocolError::InvalidValue {
            field: "handshake max_payload",
        });
    }
    if max_segment == 0 || max_segment as usize > MAX_DATA_SEGMENT {
        return Err(ProtocolError::InvalidValue {
            field: "handshake max_segment",
        });
    }
    validate_window(window)
}

fn validate_compression(compression: CompressionMode, level: i32) -> Result<(), ProtocolError> {
    if compression == CompressionMode::Zstd && !(1..=22).contains(&level) {
        return Err(ProtocolError::InvalidValue {
            field: "compression level",
        });
    }
    Ok(())
}

fn validate_window(window: u32) -> Result<(), ProtocolError> {
    if window == 0 || window as usize > DEFAULT_UNACKNOWLEDGED_WINDOW {
        return Err(ProtocolError::InvalidValue { field: "window" });
    }
    Ok(())
}

fn role_to_wire(role: Role) -> u8 {
    match role {
        Role::Source => 1,
        Role::Sink => 2,
        Role::Session => 3,
    }
}

fn role_from_wire(value: u8) -> Result<Role, ProtocolError> {
    match value {
        1 => Ok(Role::Source),
        2 => Ok(Role::Sink),
        3 => Ok(Role::Session),
        _ => Err(ProtocolError::UnknownEnum {
            field: "role",
            value,
        }),
    }
}

fn compression_to_wire(compression: CompressionMode) -> u8 {
    match compression {
        CompressionMode::None => 0,
        CompressionMode::Zstd => 1,
    }
}

fn compression_from_wire(value: u8) -> Result<CompressionMode, ProtocolError> {
    match value {
        0 => Ok(CompressionMode::None),
        1 => Ok(CompressionMode::Zstd),
        _ => Err(ProtocolError::UnknownEnum {
            field: "compression",
            value,
        }),
    }
}

fn entry_kind_to_wire(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::File => 1,
        EntryKind::Directory => 2,
        EntryKind::Symlink => 3,
        EntryKind::Other => 4,
    }
}

fn entry_kind_from_wire(value: u8) -> Result<EntryKind, ProtocolError> {
    match value {
        1 => Ok(EntryKind::File),
        2 => Ok(EntryKind::Directory),
        3 => Ok(EntryKind::Symlink),
        4 => Ok(EntryKind::Other),
        _ => Err(ProtocolError::UnknownEnum {
            field: "entry kind",
            value,
        }),
    }
}

fn metadata_operation_to_wire(operation: MetadataOperation) -> u8 {
    match operation {
        MetadataOperation::CreateDirectory => 1,
        MetadataOperation::SetFile => 2,
        MetadataOperation::SetDirectory => 3,
        MetadataOperation::CreateSymlink => 4,
        MetadataOperation::Delete => 5,
    }
}

fn metadata_operation_from_wire(value: u8) -> Result<MetadataOperation, ProtocolError> {
    match value {
        1 => Ok(MetadataOperation::CreateDirectory),
        2 => Ok(MetadataOperation::SetFile),
        3 => Ok(MetadataOperation::SetDirectory),
        4 => Ok(MetadataOperation::CreateSymlink),
        5 => Ok(MetadataOperation::Delete),
        _ => Err(ProtocolError::UnknownEnum {
            field: "metadata operation",
            value,
        }),
    }
}

struct Writer<'a> {
    output: &'a mut Vec<u8>,
}

impl Writer<'_> {
    fn new(output: &mut Vec<u8>) -> Writer<'_> {
        Writer { output }
    }

    fn u8(&mut self, value: u8) {
        self.output.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn array<const N: usize>(&mut self, value: &[u8; N]) {
        self.output.extend_from_slice(value);
    }

    fn count(&mut self, count: usize, maximum: usize) -> Result<(), ProtocolError> {
        if count > maximum {
            return Err(ProtocolError::CountTooLarge { count, maximum });
        }
        self.u32(u32::try_from(count).map_err(|_| ProtocolError::LengthOverflow)?);
        Ok(())
    }

    fn blob(&mut self, value: &[u8], maximum: usize) -> Result<(), ProtocolError> {
        if value.len() > maximum {
            return Err(ProtocolError::PayloadTooLarge {
                length: value.len(),
                maximum,
            });
        }
        self.u32(u32::try_from(value.len()).map_err(|_| ProtocolError::LengthOverflow)?);
        self.output.extend_from_slice(value);
        Ok(())
    }

    fn path(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.blob(value, MAX_ENCODED_PATH)
    }

    fn string(&mut self, value: &str, maximum: usize) -> Result<(), ProtocolError> {
        self.blob(value.as_bytes(), maximum)
    }
}

struct Reader<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::LengthOverflow)?;
        if end > self.input.len() {
            return Err(ProtocolError::Truncated {
                expected: end,
                actual: self.input.len(),
            });
        }
        let value = &self.input[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::InvalidBoolean),
        }
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().unwrap_or([0; 2]),
        ))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or([0; 4]),
        ))
    }

    fn i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or([0; 4]),
        ))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().unwrap_or([0; 8]),
        ))
    }

    fn i64(&mut self) -> Result<i64, ProtocolError> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().unwrap_or([0; 8]),
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        Ok(self.take(N)?.try_into().unwrap_or([0; N]))
    }

    fn count(&mut self, maximum: usize) -> Result<usize, ProtocolError> {
        let count = self.u32()? as usize;
        if count > maximum {
            return Err(ProtocolError::CountTooLarge { count, maximum });
        }
        Ok(count)
    }

    fn blob(&mut self, maximum: usize) -> Result<Vec<u8>, ProtocolError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(ProtocolError::PayloadTooLarge { length, maximum });
        }
        Ok(self.take(length)?.to_vec())
    }

    fn path(&mut self) -> Result<Vec<u8>, ProtocolError> {
        self.blob(MAX_ENCODED_PATH)
    }

    fn string(&mut self, maximum: usize) -> Result<String, ProtocolError> {
        let bytes = self.blob(maximum)?;
        String::from_utf8(bytes).map_err(|_| ProtocolError::InvalidUtf8)
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.position != self.input.len() {
            return Err(ProtocolError::TrailingBytes {
                count: self.input.len() - self.position,
            });
        }
        Ok(())
    }
}

/// Errors produced while encoding, reading, or validating protocol frames.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// The frame magic does not identify xsync v1.
    #[error("invalid protocol magic: {actual:02x?}")]
    InvalidMagic { actual: [u8; 4] },
    /// The fixed envelope length was not exactly 32 bytes.
    #[error("invalid protocol header length: {declared}")]
    InvalidHeaderLength { declared: usize },
    /// The remote endpoint uses a different protocol version.
    #[error("xsync version mismatch: local v{local} / remote v{remote}")]
    VersionMismatch { local: u32, remote: u32 },
    /// A message type is not assigned by v1.
    #[error("unknown protocol message type: {message_type}")]
    UnknownType { message_type: u8 },
    /// A frame flag or reserved bit is unsupported.
    #[error("invalid protocol flags 0x{flags:02x}: {detail}")]
    InvalidFlags { flags: u8, detail: &'static str },
    /// A bounded length arithmetic operation overflowed.
    #[error("protocol length arithmetic overflow")]
    LengthOverflow,
    /// A frame ended before the declared bytes were available.
    #[error("truncated protocol data: expected {expected} bytes, received {actual}")]
    Truncated { expected: usize, actual: usize },
    /// Bytes after the complete frame or typed payload are forbidden.
    #[error("trailing protocol bytes: {count}")]
    TrailingBytes { count: usize },
    /// The encoded and declared decoded lengths disagree.
    #[error("protocol length mismatch: encoded {encoded}, decoded {decoded}")]
    LengthMismatch { encoded: u32, decoded: u32 },
    /// A payload exceeds a fixed protocol limit.
    #[error("protocol payload length {length} exceeds maximum {maximum}")]
    PayloadTooLarge { length: usize, maximum: usize },
    /// A declared decompressed payload exceeds the session budget.
    #[error("declared decompressed length {length} exceeds maximum {maximum}")]
    DecompressedTooLarge { length: usize, maximum: usize },
    /// A data segment exceeds the segment budget.
    #[error("data segment length {length} exceeds the allowed segment bounds")]
    DataSegmentTooLarge { length: usize },
    /// A collection count exceeds its bounded page size.
    #[error("protocol collection count {count} exceeds maximum {maximum}")]
    CountTooLarge { count: usize, maximum: usize },
    /// An enum value is not assigned by v1.
    #[error("unknown {field} value: {value}")]
    UnknownEnum { field: &'static str, value: u8 },
    /// A field value is outside the negotiated protocol range.
    #[error("invalid protocol value for {field}")]
    InvalidValue { field: &'static str },
    /// A boolean field was neither zero nor one.
    #[error("invalid protocol boolean")]
    InvalidBoolean,
    /// A path or length-prefixed text field is not valid UTF-8 where text is required.
    #[error("invalid UTF-8 in protocol text")]
    InvalidUtf8,
    /// A range is empty, overflows, or exceeds one data segment.
    #[error("invalid byte range: {0:?}")]
    InvalidRange(ByteRange),
    /// Resume ranges overlap or are not sorted.
    #[error("overlapping or unsorted resume ranges")]
    OverlappingRanges,
    /// A frame ID was already received in this session.
    #[error("duplicate protocol message ID: {message_id}")]
    DuplicateId { message_id: u64 },
    /// The decoder's bounded session bookkeeping budget is exhausted.
    #[error("protocol session budget exceeded: {detail}")]
    SessionBudget { detail: &'static str },
    /// Reading a frame failed.
    #[error("read protocol frame: {0}")]
    Read(#[source] io::Error),
    /// Writing a frame failed.
    #[error("write protocol frame: {0}")]
    Write(#[source] io::Error),
    /// zstd compression failed.
    #[error("compress protocol payload: {0}")]
    Compression(#[source] io::Error),
    /// zstd decompression failed or exceeded its declared output size.
    #[error("decompress protocol payload: {0}")]
    Decompression(#[source] io::Error),
}

/// Tracks received large-file ranges without permitting duplicates or overlap.
#[derive(Debug, Clone)]
pub struct RangeTracker {
    file_id: u64,
    file_size: u64,
    ranges: Vec<ByteRange>,
}

impl RangeTracker {
    /// Create a tracker for one prepared file.
    #[must_use]
    pub fn new(file_id: u64, file_size: u64) -> Self {
        Self {
            file_id,
            file_size,
            ranges: Vec::new(),
        }
    }

    /// Add one range, rejecting duplicate or overlapping coverage.
    ///
    /// # Errors
    /// Returns a protocol range or session-budget error.
    pub fn add(&mut self, file_id: u64, range: ByteRange) -> Result<(), ProtocolError> {
        if self.file_id != file_id {
            return Err(ProtocolError::InvalidValue {
                field: "range file_id",
            });
        }
        validate_range(range)?;
        let end = range
            .offset
            .checked_add(range.length)
            .ok_or(ProtocolError::InvalidRange(range))?;
        if end > self.file_size {
            return Err(ProtocolError::InvalidRange(range));
        }
        if self.ranges.len() >= MAX_RESUME_RANGES {
            return Err(ProtocolError::CountTooLarge {
                count: self.ranges.len() + 1,
                maximum: MAX_RESUME_RANGES,
            });
        }
        if self.ranges.iter().any(|existing| {
            let existing_end = existing.offset.saturating_add(existing.length);
            range.offset < existing_end && existing.offset < end
        }) {
            return Err(ProtocolError::OverlappingRanges);
        }
        self.ranges.push(range);
        Ok(())
    }

    /// Return the accepted ranges in offset order.
    #[must_use]
    pub fn ranges(&self) -> Vec<ByteRange> {
        let mut ranges = self.ranges.clone();
        ranges.sort_by_key(|range| range.offset);
        ranges
    }
}

#[cfg(test)]
mod tests {
    /// The two representations describe the same thing differently, so a
    /// message carrying both is rejected rather than one being picked.
    #[test]
    fn session_config_rejects_carrying_both_filter_representations() {
        let message = Message::SessionConfig {
            streams: 1,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: u32::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW).unwrap(),
            delete: false,
            checksum: false,
            paranoid: false,
            dry_run: false,
            exclude_patterns: vec![b"*.tmp".to_vec()],
            filter_rules: vec![b"- *.tmp".to_vec()],
        };
        let encoded = encode_frame(1, &message).unwrap();
        let mut decoder = FrameDecoder::new();
        let error = decoder.read(&mut encoded.as_slice()).unwrap_err();
        assert!(
            matches!(error, ProtocolError::InvalidValue { field } if field == "filter_rules"),
            "{error:?}"
        );
    }

    /// Ordered rules must survive the wire intact: their meaning is their order.
    #[test]
    fn session_config_round_trips_ordered_filter_rules() {
        let rules = vec![
            b"+ keep/**".to_vec(),
            b"- *.tmp".to_vec(),
            b"+ keep/important.tmp".to_vec(),
        ];
        let message = Message::SessionConfig {
            streams: 1,
            batch_bytes: 32 * 1024 * 1024,
            chunk_bytes: 16 * 1024 * 1024,
            window: u32::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW).unwrap(),
            delete: false,
            checksum: false,
            paranoid: false,
            dry_run: false,
            exclude_patterns: Vec::new(),
            filter_rules: rules.clone(),
        };
        let encoded = encode_frame(7, &message).unwrap();
        let mut decoder = FrameDecoder::new();
        let frame = decoder.read(&mut encoded.as_slice()).unwrap();
        match frame.message {
            Message::SessionConfig {
                filter_rules,
                exclude_patterns,
                ..
            } => {
                assert_eq!(filter_rules, rules);
                assert!(exclude_patterns.is_empty());
            }
            other => panic!("expected SessionConfig, got {other:?}"),
        }
    }

    use std::io::Cursor;

    use super::*;

    #[test]
    fn version_negotiation_is_determined_before_session_data() {
        let v2 = CAP_VERSION_NEGOTIATION | CAP_BROWSE_V2;
        assert_eq!(negotiate_protocol_version(v2, v2), 2);
        assert_eq!(negotiate_protocol_version(v2, CAP_VERSION_NEGOTIATION), 1);
        assert_eq!(negotiate_protocol_version(v2, 0), 1);

        // Every client/server pair in the compatibility matrix. v3 needs both
        // bits on both sides; anything less falls to the best shared grammar,
        // and a peer advertising only CAP_FS_V3 without negotiation is v1.
        let v3 = v2 | CAP_FS_V3;
        assert_eq!(negotiate_protocol_version(v3, v3), 3);
        assert_eq!(negotiate_protocol_version(v3, v2), 2);
        assert_eq!(negotiate_protocol_version(v2, v3), 2);
        assert_eq!(negotiate_protocol_version(v3, CAP_VERSION_NEGOTIATION), 1);
        assert_eq!(negotiate_protocol_version(v3, 0), 1);
        assert_eq!(negotiate_protocol_version(v3, CAP_FS_V3), 1);
        // A sync endpoint never advertises CAP_FS_V3, so a push or pull is
        // unaffected by v3 existing at all.
        let sync = CAP_ZSTD | CAP_VERSION_NEGOTIATION | CAP_FILTER_RULES;
        assert_eq!(negotiate_protocol_version(sync, v3), 1);
        assert_eq!(negotiate_protocol_version(v3, sync), 1);
        assert_eq!(
            common_capabilities(v2 | CAP_ZSTD, v2 | CAP_ZSTD),
            v2 | CAP_ZSTD
        );
        assert_eq!(
            common_capabilities(v2 | CAP_BROWSE_META, v2 | CAP_BROWSE_META),
            v2 | CAP_BROWSE_META
        );
        assert_eq!(common_capabilities(v2 | CAP_BROWSE_META, v2), v2);
        assert_eq!(common_capabilities(v2, u32::MAX) & !KNOWN_CAPABILITIES, 0);
    }

    #[test]
    fn selected_version_is_committed_at_the_frame_boundary() {
        let message = Message::Ack {
            acknowledged_id: 42,
            acknowledged_type: 1,
        };
        let v2_frame = encode_frame_with_version(1, &message, 2).unwrap();
        assert_eq!(u32::from_le_bytes(v2_frame[8..12].try_into().unwrap()), 2);
        assert_eq!(
            decode_frame_for_version(&v2_frame, 2).unwrap().message,
            message
        );

        let mut v1_decoder = FrameDecoder::for_version(1);
        assert_eq!(
            v1_decoder.decode(&v2_frame).unwrap_err().to_string(),
            "xsync version mismatch: local v1 / remote v2"
        );

        let v1_frame = encode_frame_with_version(2, &message, 1).unwrap();
        let mut v2_decoder = FrameDecoder::for_version(2);
        assert_eq!(
            v2_decoder.decode(&v1_frame).unwrap_err().to_string(),
            "xsync version mismatch: local v2 / remote v1"
        );
    }

    fn entry(path: &[u8], kind: EntryKind) -> EntryRecord {
        EntryRecord {
            path: path.to_vec(),
            kind,
            size: 123,
            mtime_ns: -456,
            mode: 0o644,
            fingerprint: [7; 32],
        }
    }

    fn all_messages() -> Vec<Message> {
        vec![
            Message::Handshake {
                role: Role::Sink,
                capabilities: 3,
                max_payload: u32::try_from(MAX_COMPLETE_PAYLOAD).unwrap(),
                max_segment: u32::try_from(MAX_DATA_SEGMENT).unwrap(),
                window: u32::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW).unwrap(),
                job_id: [1; 16],
                compression: CompressionMode::Zstd,
                compression_level: 3,
            },
            Message::SessionConfig {
                streams: 4,
                batch_bytes: 32 * 1024 * 1024,
                chunk_bytes: 16 * 1024 * 1024,
                window: u32::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW).unwrap(),
                delete: true,
                checksum: true,
                paranoid: false,
                dry_run: true,
                exclude_patterns: vec![b"*.log".to_vec(), b"cache".to_vec()],
                filter_rules: Vec::new(),
            },
            Message::FileBatch {
                batch_id: 8,
                entries: vec![
                    entry(b"a/b", EntryKind::File),
                    entry(b"empty", EntryKind::Directory),
                ],
            },
            Message::FileSegment {
                file_id: 9,
                offset: 16,
                digest: *blake3::hash(b"hello").as_bytes(),
                data: b"hello".to_vec(),
            },
            Message::LargeFilePrepare {
                file_id: 10,
                path: b"large.bin".to_vec(),
                size: 99,
                mtime_ns: 100,
                mode: 0o600,
                fingerprint: [2; 32],
            },
            Message::LargeFileRange {
                file_id: 10,
                range: ByteRange {
                    offset: 0,
                    length: 4096,
                },
            },
            Message::LargeFileFinish {
                file_id: 10,
                digest: [3; 32],
            },
            Message::Metadata {
                operation: MetadataOperation::CreateSymlink,
                path: b"link".to_vec(),
                target: b"target".to_vec(),
                mode: 0,
                mtime_ns: 0,
            },
            Message::Scan {
                scan_id: 11,
                final_page: true,
                entries: vec![entry(b"scan", EntryKind::Symlink)],
            },
            Message::Stats {
                files: 1,
                bytes: 2,
                skipped: 3,
                warnings: 4,
                failed: 5,
            },
            Message::Ack {
                acknowledged_id: 12,
                acknowledged_type: 4,
            },
            Message::Error {
                code: 23,
                related_id: 12,
                message: "partial failure".to_owned(),
            },
            Message::ResumePage {
                file_id: 10,
                page: 0,
                final_page: true,
                ranges: vec![ByteRange {
                    offset: 0,
                    length: 4096,
                }],
            },
        ]
    }

    #[test]
    fn every_message_round_trips_without_serializer_layout() {
        for (id, message) in all_messages().iter().enumerate() {
            let frame = encode_frame(id as u64 + 1, message).unwrap();
            let decoded = decode_frame(&frame).unwrap();
            assert_eq!(decoded.message_id, id as u64 + 1);
            assert_eq!(decoded.message, *message);
        }
    }

    #[test]
    fn handshake_golden_bytes_are_stable() {
        let message = Message::Handshake {
            role: Role::Source,
            capabilities: 0x1122_3344,
            max_payload: 0x0002_0304,
            max_segment: 0x0006_0708,
            window: 0x000a_0b0c,
            job_id: [0x10; 16],
            compression: CompressionMode::None,
            compression_level: 3,
        };
        let frame = encode_frame(0x0102_0304_0506_0708, &message).unwrap();
        let expected = [
            0x78, 0x73, 0x6e, 0x31, 0x20, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x26, 0x00, 0x00, 0x00, 0x26, 0x00, 0x00, 0x00, 0x08, 0x07, 0x06, 0x05,
            0x04, 0x03, 0x02, 0x01, 0x01, 0x44, 0x33, 0x22, 0x11, 0x04, 0x03, 0x02, 0x00, 0x08,
            0x07, 0x06, 0x00, 0x0c, 0x0b, 0x0a, 0x00, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
            0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x03, 0x00, 0x00, 0x00,
        ];
        assert_eq!(frame, expected);
    }

    #[test]
    fn streaming_decoder_validates_header_before_body_and_rejects_duplicate_ids() {
        let frame = encode_frame(
            7,
            &Message::Stats {
                files: 1,
                bytes: 2,
                skipped: 0,
                warnings: 0,
                failed: 0,
            },
        )
        .unwrap();
        let mut decoder = FrameDecoder::new();
        let mut bytes = frame.clone();
        bytes.extend_from_slice(&frame);
        let mut cursor = Cursor::new(bytes);
        assert!(decoder.read(&mut cursor).is_ok());
        assert!(matches!(
            decoder.read(&mut cursor),
            Err(ProtocolError::DuplicateId { message_id: 7 })
        ));
    }

    #[test]
    fn sequential_message_ids_do_not_exhaust_tracking_budget() {
        let mut decoder = FrameDecoder::new();
        for id in 1..=1_100_000 {
            decoder.remember(id).unwrap();
        }
        assert!(matches!(
            decoder.remember(1_000_000),
            Err(ProtocolError::DuplicateId {
                message_id: 1_000_000
            })
        ));
    }

    #[test]
    fn malformed_envelopes_are_rejected() {
        let frame = encode_frame(
            1,
            &Message::Stats {
                files: 1,
                bytes: 2,
                skipped: 3,
                warnings: 4,
                failed: 5,
            },
        )
        .unwrap();
        for length in 0..frame.len() {
            assert!(matches!(
                decode_frame(&frame[..length]),
                Err(ProtocolError::Truncated { .. })
            ));
        }
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(matches!(
            decode_frame(&trailing),
            Err(ProtocolError::TrailingBytes { .. })
        ));

        let mut unknown_magic = frame.clone();
        unknown_magic[0] = b'!';
        assert!(matches!(
            decode_frame(&unknown_magic),
            Err(ProtocolError::InvalidMagic { .. })
        ));

        let mut unknown_type = frame.clone();
        unknown_type[12] = 99;
        assert!(matches!(
            decode_frame(&unknown_type),
            Err(ProtocolError::UnknownType { .. })
        ));

        let mut unknown_flags = frame;
        unknown_flags[13] = 0x80;
        assert!(matches!(
            decode_frame(&unknown_flags),
            Err(ProtocolError::InvalidFlags { .. })
        ));

        let mut wrong_version = encode_frame(
            2,
            &Message::Stats {
                files: 0,
                bytes: 0,
                skipped: 0,
                warnings: 0,
                failed: 0,
            },
        )
        .unwrap();
        wrong_version[8..12].copy_from_slice(&1_u32.to_le_bytes());
        let error = decode_frame(&wrong_version).unwrap_err();
        assert_eq!(
            error.to_string(),
            "xsync version mismatch: local v2 / remote v1"
        );

        let mut oversized_count = encode_frame(
            3,
            &Message::FileBatch {
                batch_id: 1,
                entries: Vec::new(),
            },
        )
        .unwrap();
        oversized_count[40..44]
            .copy_from_slice(&(u32::try_from(MAX_COLLECTION_COUNT + 1).unwrap()).to_le_bytes());
        assert!(matches!(
            decode_frame(&oversized_count),
            Err(ProtocolError::CountTooLarge { .. })
        ));
    }

    #[test]
    fn compressed_payload_is_bounded_and_round_trips() {
        let message = Message::FileSegment {
            file_id: 4,
            offset: 0,
            digest: *blake3::hash(&vec![b'x'; 128 * 1024]).as_bytes(),
            data: vec![b'x'; 128 * 1024],
        };
        let frame = encode_frame_with_options(
            1,
            &message,
            EncodeOptions {
                compression: CompressionMode::Zstd,
            },
        )
        .unwrap();
        assert_ne!(frame[13] & FRAME_FLAG_ZSTD, 0);
        assert_eq!(decode_frame(&frame).unwrap().message, message);

        let mut bomb = frame;
        bomb[20..24]
            .copy_from_slice(&(u32::try_from(MAX_DECOMPRESSED_PAYLOAD + 1).unwrap()).to_le_bytes());
        assert!(matches!(
            decode_frame(&bomb),
            Err(ProtocolError::DecompressedTooLarge { .. })
        ));
    }

    #[test]
    fn compression_negotiation_intersects_endpoint_capabilities() {
        assert_eq!(
            negotiate_compression(CompressionMode::Zstd, CAP_ZSTD, CAP_ZSTD),
            CompressionMode::Zstd
        );
        assert_eq!(
            negotiate_compression(CompressionMode::Zstd, CAP_ZSTD, 0),
            CompressionMode::None
        );
        assert_eq!(
            negotiate_compression(CompressionMode::None, CAP_ZSTD, CAP_ZSTD),
            CompressionMode::None
        );
    }

    #[test]
    fn compression_is_selected_per_frame_and_level_is_negotiated() {
        let text = Message::FileSegment {
            file_id: 1,
            offset: 0,
            digest: *blake3::hash(&vec![b't'; 256 * 1024]).as_bytes(),
            data: vec![b't'; 256 * 1024],
        };
        let mut random = Vec::with_capacity(256 * 1024);
        let mut state = 0x1234_5678_u32;
        for _ in 0..256 * 1024 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            random.push(u8::try_from(state & 0xff).expect("test byte fits"));
        }
        let incompressible = Message::FileSegment {
            file_id: 2,
            offset: 0,
            digest: *blake3::hash(&random).as_bytes(),
            data: random,
        };
        let compressed = encode_frame_with_compression(1, &text, CompressionMode::Zstd, 9).unwrap();
        let raw =
            encode_frame_with_compression(2, &incompressible, CompressionMode::Zstd, 9).unwrap();
        assert_ne!(compressed[13] & FRAME_FLAG_ZSTD, 0);
        assert_eq!(raw[13] & FRAME_FLAG_ZSTD, 0);
        assert!(compressed.len() < text_data_len(&text));
        assert_eq!(decode_frame(&compressed).unwrap().message, text);
        assert_eq!(decode_frame(&raw).unwrap().message, incompressible);
    }

    fn text_data_len(message: &Message) -> usize {
        match message {
            Message::FileSegment { data, .. } => FRAME_HEADER_LEN + 20 + 32 + data.len(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn single_bit_corruption_never_panics_or_returns_the_original_frame() {
        let frame = encode_frame(
            1,
            &Message::Stats {
                files: 1,
                bytes: 2,
                skipped: 3,
                warnings: 4,
                failed: 5,
            },
        )
        .unwrap();
        let original = decode_frame(&frame).unwrap();
        for index in 0..frame.len() {
            for bit in 0..8 {
                let mut corrupted = frame.clone();
                corrupted[index] ^= 1 << bit;
                if let Ok(decoded) = decode_frame(&corrupted) {
                    assert_ne!(decoded, original);
                }
            }
        }
    }

    #[test]
    fn paths_are_raw_bytes_and_resume_ranges_are_checked() {
        let message = Message::LargeFilePrepare {
            file_id: 1,
            path: vec![0xff, 0x00, b'x'],
            size: 1,
            mtime_ns: 0,
            mode: 0,
            fingerprint: [0; 32],
        };
        assert_eq!(
            decode_frame(&encode_frame(1, &message).unwrap())
                .unwrap()
                .message,
            message
        );

        let mut tracker = RangeTracker::new(1, 100);
        tracker
            .add(
                1,
                ByteRange {
                    offset: 0,
                    length: 10,
                },
            )
            .unwrap();
        assert!(matches!(
            tracker.add(
                1,
                ByteRange {
                    offset: 5,
                    length: 10,
                }
            ),
            Err(ProtocolError::OverlappingRanges)
        ));
    }
}
