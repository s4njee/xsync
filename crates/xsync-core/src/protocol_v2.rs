//! Strict payload codec for the initial v2 browse message table.
//!
//! The v2 session envelope and payload layout are kept separate from the
//! fail-closed v1 sync decoder.

use std::io::Read;

use thiserror::Error;

use crate::protocol::FRAME_HEADER_LEN;

/// `FRAME_HEADER_LEN` as it appears in the 16-bit header-length field.
///
/// The static assertion turns a header that outgrows the field into a build
/// error rather than a silently truncated length on the wire.
const FRAME_HEADER_LEN_U16: u16 = {
    const _: () = assert!(FRAME_HEADER_LEN <= u16::MAX as usize);
    // Truncation is impossible: the assertion above is evaluated at compile
    // time, so a header that outgrew the field would fail the build.
    #[allow(clippy::cast_possible_truncation)]
    let value = FRAME_HEADER_LEN as u16;
    value
};
use crate::HANDSHAKE_MAGIC;

const MAX_PATH: usize = 1024 * 1024;
const MAX_ERROR: usize = 64 * 1024;
const MAX_COLLECTION: usize = 65_536;
const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_DELETE_FAILURES: usize = MAX_COLLECTION;
const MAX_COMPLETE_FETCH_CHUNK: usize = 1024 * 1024;

/// A metadata-only v2 browse entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseEntry {
    /// Raw name/path bytes.
    pub name: Vec<u8>,
    /// Frozen v1 kind value.
    pub kind: u8,
    /// Logical size.
    pub size: u64,
    /// Modification time in nanoseconds.
    pub mtime_ns: i64,
    /// Permission bits.
    pub mode: u32,
    /// Raw symlink target, empty for non-links.
    pub symlink_target: Vec<u8>,
}

/// v2 stat result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StatStatus {
    /// The path exists and the entry follows.
    Ok = 0,
    /// The path does not exist.
    Missing = 1,
    /// The path could not be inspected.
    Error = 2,
}

/// Outcome of a remote mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MutationStatus {
    /// The mutation completed.
    Ok = 0,
    /// The destination already exists.
    AlreadyExists = 1,
    /// The operation was denied by permissions.
    PermissionDenied = 2,
    /// A parent directory does not exist.
    ParentMissing = 3,
    /// Rename crossed filesystems.
    CrossDevice = 4,
    /// Another filesystem error occurred.
    Error = 5,
}

impl MutationStatus {
    fn decode(value: u8) -> Result<Self, V2CodecError> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::AlreadyExists),
            2 => Ok(Self::PermissionDenied),
            3 => Ok(Self::ParentMissing),
            4 => Ok(Self::CrossDevice),
            5 => Ok(Self::Error),
            _ => Err(V2CodecError::InvalidEnum {
                field: "mutation status",
                value,
            }),
        }
    }
}

/// Terminal outcome of a recursive delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeleteStatus {
    /// Every discovered item was removed.
    Complete = 0,
    /// The operation stopped with one or more failures.
    Partial = 1,
    /// The client cancelled the operation before completion.
    Cancelled = 2,
}

impl DeleteStatus {
    fn decode(value: u8) -> Result<Self, V2CodecError> {
        match value {
            0 => Ok(Self::Complete),
            1 => Ok(Self::Partial),
            2 => Ok(Self::Cancelled),
            _ => Err(V2CodecError::InvalidEnum {
                field: "delete status",
                value,
            }),
        }
    }
}

/// One path that could not be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteFailure {
    /// Raw relative path.
    pub path: Vec<u8>,
    /// Platform errno, or zero when unavailable.
    pub errno: i32,
}

/// Outcome of publishing one fetched file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PublishStatus {
    /// The file was atomically published.
    Ok = 0,
    /// The remote file changed since fetch.
    Changed = 1,
    /// The publish failed for another reason.
    Error = 2,
}

impl PublishStatus {
    fn decode(value: u8) -> Result<Self, V2CodecError> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Changed),
            2 => Ok(Self::Error),
            _ => Err(V2CodecError::InvalidEnum {
                field: "publish status",
                value,
            }),
        }
    }
}

impl StatStatus {
    fn decode(value: u8) -> Result<Self, V2CodecError> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Missing),
            2 => Ok(Self::Error),
            _ => Err(V2CodecError::InvalidEnum {
                field: "stat status",
                value,
            }),
        }
    }
}

/// Initial v2 browse payloads assigned by `protocol.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2Message {
    /// Request one directory page.
    ListRequest {
        path: Vec<u8>,
        page_token: u64,
        page_size: u32,
    },
    /// Return one directory page.
    ListPage {
        related_id: u64,
        page_token: u64,
        final_page: bool,
        entries: Vec<BrowseEntry>,
    },
    /// Request metadata for one path.
    StatRequest { path: Vec<u8>, include_digest: bool },
    /// Return metadata for one path.
    StatResponse {
        related_id: u64,
        status: StatStatus,
        entry: Option<BrowseEntry>,
        digest: Option<[u8; 32]>,
        error: Vec<u8>,
    },
    /// Cancel one request.
    CancelRequest { related_id: u64 },
    /// Keep a session alive.
    Keepalive { nonce: u64 },
    /// Acknowledge a keepalive.
    KeepaliveAck { nonce: u64 },
    /// Report a request-scoped browse failure.
    BrowseError {
        related_id: u64,
        code: u16,
        message: Vec<u8>,
    },
    /// Rename one path without replacing an existing destination.
    RenameRequest {
        source: Vec<u8>,
        destination: Vec<u8>,
    },
    /// Return the result of a rename.
    RenameResponse {
        related_id: u64,
        status: MutationStatus,
        error: Vec<u8>,
    },
    /// Create one directory, without creating missing parents.
    CreateDirectoryRequest { path: Vec<u8> },
    /// Return the result of creating a directory.
    CreateDirectoryResponse {
        related_id: u64,
        status: MutationStatus,
        error: Vec<u8>,
    },
    /// Begin an irreversible recursive delete.
    DeleteRequest { path: Vec<u8> },
    /// Report the result for one deleted or failed path.
    DeleteProgress {
        related_id: u64,
        path: Vec<u8>,
        removed: bool,
        error: Vec<u8>,
    },
    /// Finish a recursive delete with all failures collected.
    DeleteResponse {
        related_id: u64,
        status: DeleteStatus,
        removed_count: u64,
        failures: Vec<DeleteFailure>,
        irreversible: bool,
    },
    /// Fetch one regular file with its stable-read metadata.
    FetchRequest { path: Vec<u8> },
    /// Describe a fetch before its chunks are sent.
    FetchStart {
        related_id: u64,
        size: u64,
        mtime_ns: i64,
        device: u64,
        file: u64,
        digest: [u8; 32],
    },
    /// One bounded fetch payload chunk.
    FetchChunk {
        related_id: u64,
        offset: u64,
        data: Vec<u8>,
    },
    /// Publish a local file only if the fetched remote identity still matches.
    PublishRequest {
        path: Vec<u8>,
        size: u64,
        mtime_ns: i64,
        device: u64,
        file: u64,
        content_size: u64,
        digest: [u8; 32],
    },
    /// Authorize data upload after the identity preflight.
    PublishReady { related_id: u64 },
    /// One bounded publish payload chunk.
    PublishChunk {
        related_id: u64,
        offset: u64,
        data: Vec<u8>,
    },
    /// Return publish status and the current remote identity when available.
    PublishResponse {
        related_id: u64,
        status: PublishStatus,
        current_present: bool,
        size: u64,
        mtime_ns: i64,
        device: u64,
        file: u64,
        error: Vec<u8>,
    },
    /// Set Unix permission bits on one path (follows a final symlink).
    SetPermissionsRequest { path: Vec<u8>, mode: u32 },
    /// Return the result of setting permission bits.
    SetPermissionsResponse {
        related_id: u64,
        status: MutationStatus,
        error: Vec<u8>,
    },
    /// Set modification time on one path (follows a final symlink).
    SetMtimeRequest { path: Vec<u8>, mtime_ns: i64 },
    /// Return the result of setting modification time.
    SetMtimeResponse {
        related_id: u64,
        status: MutationStatus,
        error: Vec<u8>,
    },
    /// Read one symlink's target without following it.
    ReadLinkRequest { path: Vec<u8> },
    /// Return a symlink target, or missing/error.
    ReadLinkResponse {
        related_id: u64,
        status: StatStatus,
        target: Vec<u8>,
        error: Vec<u8>,
    },
}

/// A complete, uncompressed v2 browse frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Frame {
    /// Unique frame identifier.
    pub message_id: u64,
    /// Typed browse payload.
    pub message: V2Message,
}

/// Payload codec failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum V2CodecError {
    /// A length or count exceeds the frozen bound.
    #[error("v2 {field} exceeds its bound: {value}")]
    Bound { field: &'static str, value: usize },
    /// The payload ended before a field was complete.
    #[error("truncated v2 payload")]
    Truncated,
    /// A boolean or enum value is invalid.
    #[error("invalid v2 {field} value: {value}")]
    InvalidEnum { field: &'static str, value: u8 },
    /// A required UTF-8 field is not UTF-8.
    #[error("v2 {field} is not UTF-8")]
    InvalidUtf8 { field: &'static str },
    /// A conditional field is inconsistent with its status.
    #[error("invalid v2 stat response fields")]
    InvalidStatFields,
    /// A mutation response contains fields inconsistent with its status.
    #[error("invalid v2 mutation response fields")]
    InvalidMutationFields,
    /// A delete payload contains inconsistent fields.
    #[error("invalid v2 delete fields")]
    InvalidDeleteFields,
    /// An entry kind or symlink target is inconsistent with the entry.
    #[error("invalid v2 entry fields")]
    InvalidEntryFields,
    /// Bytes remain after the typed payload.
    #[error("trailing v2 payload bytes: {0}")]
    Trailing(usize),
    /// The v2 envelope is malformed.
    #[error("malformed v2 envelope: {0}")]
    Envelope(&'static str),
    /// Reading a v2 frame failed.
    #[error("read v2 frame: {0}")]
    Io(String),
}

/// Encode a complete uncompressed v2 browse frame.
///
/// # Errors
///
/// Returns [`V2CodecError::Bound`] when the encoded payload is longer than the
/// `u32` length field the frame header carries, and propagates any error from
/// encoding the message body itself.
pub fn encode_frame(message_id: u64, message: &V2Message) -> Result<Vec<u8>, V2CodecError> {
    let message_type = message_type(message);
    let payload = encode(message)?;
    let payload_len = u32::try_from(payload.len()).map_err(|_| V2CodecError::Bound {
        field: "payload",
        value: payload.len(),
    })?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    frame.extend_from_slice(HANDSHAKE_MAGIC);
    frame.extend_from_slice(&FRAME_HEADER_LEN_U16.to_le_bytes());
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&2_u32.to_le_bytes());
    frame.push(message_type);
    frame.push(0);
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&message_id.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode one complete v2 browse frame, rejecting trailing bytes.
///
/// # Errors
///
/// Returns [`V2CodecError::Envelope`] for a header that is truncated, declares
/// a header length or protocol version this build does not speak, sets a
/// reserved field, or is followed by a payload whose length disagrees with the
/// header. Body decode errors are propagated unchanged.
///
/// # Panics
///
/// Does not panic in practice. The fixed-width `try_into` conversions below are
/// infallible because the length check at the top of this function has already
/// established that `header` is exactly `FRAME_HEADER_LEN` bytes.
pub fn decode_frame(bytes: &[u8]) -> Result<V2Frame, V2CodecError> {
    if bytes.len() < FRAME_HEADER_LEN {
        return Err(V2CodecError::Envelope("truncated header"));
    }
    let header = &bytes[..FRAME_HEADER_LEN];
    if &header[..4] != HANDSHAKE_MAGIC {
        return Err(V2CodecError::Envelope("invalid magic"));
    }
    if u16::from_le_bytes([header[4], header[5]]) as usize != FRAME_HEADER_LEN {
        return Err(V2CodecError::Envelope("invalid header length"));
    }
    if u16::from_le_bytes([header[6], header[7]]) != 0
        || header[13] != 0
        || u16::from_le_bytes([header[14], header[15]]) != 0
    {
        return Err(V2CodecError::Envelope("non-zero reserved field"));
    }
    if u32::from_le_bytes(header[8..12].try_into().unwrap()) != 2 {
        return Err(V2CodecError::Envelope("wrong version"));
    }
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    let decoded_len = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    if payload_len != decoded_len {
        return Err(V2CodecError::Envelope("compressed payload is unsupported"));
    }
    if payload_len > MAX_PAYLOAD {
        return Err(V2CodecError::Bound {
            field: "payload",
            value: payload_len,
        });
    }
    let total = FRAME_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(V2CodecError::Envelope("frame length overflow"))?;
    if bytes.len() < total {
        return Err(V2CodecError::Truncated);
    }
    if bytes.len() > total {
        return Err(V2CodecError::Trailing(bytes.len() - total));
    }
    Ok(V2Frame {
        message_id: u64::from_le_bytes(header[24..32].try_into().unwrap()),
        message: decode(header[12], &bytes[FRAME_HEADER_LEN..])?,
    })
}

/// Read one v2 frame from a persistent stream. `None` means clean EOF before
/// the next frame, which is the normal session shutdown path.
///
/// # Errors
///
/// Returns [`V2CodecError::Io`] if the stream fails, and
/// [`V2CodecError::Envelope`] if it ends part-way through a frame — a partial
/// frame is a protocol violation, distinct from the clean EOF reported as
/// `Ok(None)`. Frame validation errors are propagated from [`decode_frame`].
///
/// # Panics
///
/// Does not panic in practice: the fixed-width conversions read from `header`
/// only after the full `FRAME_HEADER_LEN` bytes have been read into it.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<V2Frame>, V2CodecError> {
    let mut header = [0_u8; FRAME_HEADER_LEN];
    let first = reader
        .read(&mut header[..1])
        .map_err(|error| V2CodecError::Io(error.to_string()))?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut header[1..])
        .map_err(|error| V2CodecError::Io(error.to_string()))?;
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    if payload_len > MAX_PAYLOAD {
        return Err(V2CodecError::Bound {
            field: "payload",
            value: payload_len,
        });
    }
    let mut bytes = header.to_vec();
    let old_len = bytes.len();
    bytes.resize(old_len + payload_len, 0);
    reader
        .read_exact(&mut bytes[old_len..])
        .map_err(|error| V2CodecError::Io(error.to_string()))?;
    decode_frame(&bytes).map(Some)
}

fn message_type(message: &V2Message) -> u8 {
    match message {
        V2Message::ListRequest { .. } => 14,
        V2Message::ListPage { .. } => 15,
        V2Message::StatRequest { .. } => 16,
        V2Message::StatResponse { .. } => 17,
        V2Message::CancelRequest { .. } => 18,
        V2Message::Keepalive { .. } => 19,
        V2Message::KeepaliveAck { .. } => 20,
        V2Message::BrowseError { .. } => 21,
        V2Message::RenameRequest { .. } => 22,
        V2Message::RenameResponse { .. } => 23,
        V2Message::CreateDirectoryRequest { .. } => 24,
        V2Message::CreateDirectoryResponse { .. } => 25,
        V2Message::DeleteRequest { .. } => 26,
        V2Message::DeleteProgress { .. } => 27,
        V2Message::DeleteResponse { .. } => 28,
        V2Message::FetchRequest { .. } => 29,
        V2Message::FetchStart { .. } => 30,
        V2Message::FetchChunk { .. } => 31,
        V2Message::PublishRequest { .. } => 32,
        V2Message::PublishReady { .. } => 33,
        V2Message::PublishChunk { .. } => 34,
        V2Message::PublishResponse { .. } => 35,
        V2Message::SetPermissionsRequest { .. } => 36,
        V2Message::SetPermissionsResponse { .. } => 37,
        V2Message::SetMtimeRequest { .. } => 38,
        V2Message::SetMtimeResponse { .. } => 39,
        V2Message::ReadLinkRequest { .. } => 40,
        V2Message::ReadLinkResponse { .. } => 41,
    }
}

/// Encode one v2 payload without an envelope.
///
/// # Errors
/// Returns [`V2CodecError`] when a field is malformed or exceeds its bound.
// One arm per protocol message, deliberately kept in a single function so the
// wire format can be read top to bottom against protocol.md. Splitting it would
// scatter the encoding across helpers, and merging arms that coincidentally
// share a payload shape -- `CancelRequest` and `PublishReady` are both a bare
// u64 -- would destroy the one-to-one message-to-wire correspondence that makes
// this auditable.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn encode(message: &V2Message) -> Result<Vec<u8>, V2CodecError> {
    let mut writer = Writer::default();
    match message {
        V2Message::ListRequest {
            path,
            page_token,
            page_size,
        } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.u64(*page_token);
            if *page_size == 0 || (*page_size as usize) > MAX_COLLECTION {
                return Err(V2CodecError::Bound {
                    field: "page size",
                    value: *page_size as usize,
                });
            }
            writer.u32(*page_size);
        }
        V2Message::ListPage {
            related_id,
            page_token,
            final_page,
            entries,
        } => {
            writer.u64(*related_id);
            writer.u64(*page_token);
            writer.bool(*final_page);
            writer.entries(entries)?;
        }
        V2Message::StatRequest {
            path,
            include_digest,
        } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.bool(*include_digest);
        }
        V2Message::StatResponse {
            related_id,
            status,
            entry,
            digest,
            error,
        } => {
            validate_stat_fields(*status, entry.as_ref(), digest.as_ref(), error)?;
            writer.u64(*related_id);
            writer.u8(*status as u8);
            if *status == StatStatus::Ok {
                let entry = entry.as_ref().ok_or(V2CodecError::InvalidStatFields)?;
                encode_entry(&mut writer, entry)?;
                writer.bool(digest.is_some());
                if let Some(digest) = digest {
                    writer.bytes(digest);
                }
            } else if *status == StatStatus::Missing {
                writer.bool(false);
            } else if *status == StatStatus::Error {
                writer.utf8_blob(error, MAX_ERROR, "error message")?;
            }
        }
        V2Message::CancelRequest { related_id } => writer.u64(*related_id),
        V2Message::Keepalive { nonce } | V2Message::KeepaliveAck { nonce } => writer.u64(*nonce),
        V2Message::BrowseError {
            related_id,
            code,
            message,
        } => {
            writer.u64(*related_id);
            writer.u16(*code);
            writer.utf8_blob(message, MAX_ERROR, "error message")?;
        }
        V2Message::RenameRequest {
            source,
            destination,
        } => {
            writer.blob(source, MAX_PATH, "source path")?;
            writer.blob(destination, MAX_PATH, "destination path")?;
        }
        V2Message::RenameResponse {
            related_id,
            status,
            error,
        }
        | V2Message::CreateDirectoryResponse {
            related_id,
            status,
            error,
        }
        | V2Message::SetPermissionsResponse {
            related_id,
            status,
            error,
        }
        | V2Message::SetMtimeResponse {
            related_id,
            status,
            error,
        } => {
            writer.u64(*related_id);
            writer.u8(*status as u8);
            if *status == MutationStatus::Ok {
                if !error.is_empty() {
                    return Err(V2CodecError::InvalidMutationFields);
                }
            } else if error.is_empty() {
                return Err(V2CodecError::InvalidMutationFields);
            } else {
                writer.utf8_blob(error, MAX_ERROR, "mutation error")?;
            }
        }
        V2Message::CreateDirectoryRequest { path } => writer.blob(path, MAX_PATH, "path")?,
        V2Message::DeleteRequest { path } => writer.blob(path, MAX_PATH, "path")?,
        V2Message::DeleteProgress {
            related_id,
            path,
            removed,
            error,
        } => {
            writer.u64(*related_id);
            writer.blob(path, MAX_PATH, "delete path")?;
            writer.bool(*removed);
            if *removed {
                if !error.is_empty() {
                    return Err(V2CodecError::InvalidDeleteFields);
                }
            } else if error.is_empty() {
                return Err(V2CodecError::InvalidDeleteFields);
            } else {
                writer.utf8_blob(error, MAX_ERROR, "delete error")?;
            }
        }
        V2Message::DeleteResponse {
            related_id,
            status,
            removed_count,
            failures,
            irreversible,
        } => {
            if !*irreversible || failures.len() > MAX_DELETE_FAILURES {
                return Err(V2CodecError::InvalidDeleteFields);
            }
            writer.u64(*related_id);
            writer.u8(*status as u8);
            writer.u64(*removed_count);
            writer.bool(*irreversible);
            writer.failures(failures)?;
        }
        V2Message::FetchRequest { path } => writer.blob(path, MAX_PATH, "path")?,
        V2Message::FetchStart {
            related_id,
            size,
            mtime_ns,
            device,
            file,
            digest,
        } => {
            writer.u64(*related_id);
            writer.u64(*size);
            writer.i64(*mtime_ns);
            writer.u64(*device);
            writer.u64(*file);
            writer.bytes(digest);
        }
        V2Message::FetchChunk {
            related_id,
            offset,
            data,
        } => {
            if data.len() > MAX_COMPLETE_FETCH_CHUNK {
                return Err(V2CodecError::Bound {
                    field: "fetch chunk",
                    value: data.len(),
                });
            }
            writer.u64(*related_id);
            writer.u64(*offset);
            writer.blob(data, MAX_COMPLETE_FETCH_CHUNK, "fetch chunk")?;
        }
        V2Message::PublishRequest {
            path,
            size,
            mtime_ns,
            device,
            file,
            content_size,
            digest,
        } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.u64(*size);
            writer.i64(*mtime_ns);
            writer.u64(*device);
            writer.u64(*file);
            writer.u64(*content_size);
            writer.bytes(digest);
        }
        V2Message::PublishReady { related_id } => writer.u64(*related_id),
        V2Message::PublishChunk {
            related_id,
            offset,
            data,
        } => {
            if data.len() > MAX_COMPLETE_FETCH_CHUNK {
                return Err(V2CodecError::Bound {
                    field: "publish chunk",
                    value: data.len(),
                });
            }
            writer.u64(*related_id);
            writer.u64(*offset);
            writer.blob(data, MAX_COMPLETE_FETCH_CHUNK, "publish chunk")?;
        }
        V2Message::PublishResponse {
            related_id,
            status,
            current_present,
            size,
            mtime_ns,
            device,
            file,
            error,
        } => {
            if *status == PublishStatus::Ok && !error.is_empty()
                || *status != PublishStatus::Ok && error.is_empty()
            {
                return Err(V2CodecError::InvalidMutationFields);
            }
            writer.u64(*related_id);
            writer.u8(*status as u8);
            writer.bool(*current_present);
            if *current_present {
                writer.u64(*size);
                writer.i64(*mtime_ns);
                writer.u64(*device);
                writer.u64(*file);
            }
            writer.utf8_blob(error, MAX_ERROR, "publish error")?;
        }
        V2Message::SetPermissionsRequest { path, mode } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.u32(*mode);
        }
        V2Message::SetMtimeRequest { path, mtime_ns } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.i64(*mtime_ns);
        }
        V2Message::ReadLinkRequest { path } => writer.blob(path, MAX_PATH, "path")?,
        V2Message::ReadLinkResponse {
            related_id,
            status,
            target,
            error,
        } => {
            validate_read_link_fields(*status, target, error)?;
            writer.u64(*related_id);
            writer.u8(*status as u8);
            if *status == StatStatus::Ok {
                writer.blob(target, MAX_PATH, "symlink target")?;
            } else if *status == StatStatus::Error {
                writer.utf8_blob(error, MAX_ERROR, "error message")?;
            }
        }
    }
    if writer.bytes.len() > MAX_PAYLOAD {
        return Err(V2CodecError::Bound {
            field: "payload",
            value: writer.bytes.len(),
        });
    }
    Ok(writer.bytes)
}

/// Decode one v2 payload for a type byte from `protocol.md`.
///
/// # Errors
/// Returns [`V2CodecError`] when the type, payload, field value, or bounds are
/// invalid.
// One arm per protocol message, deliberately kept in a single function so the
// wire format can be read top to bottom against protocol.md. Splitting it would
// scatter the encoding across helpers, and merging arms that coincidentally
// share a payload shape -- `CancelRequest` and `PublishReady` are both a bare
// u64 -- would destroy the one-to-one message-to-wire correspondence that makes
// this auditable.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn decode(message_type: u8, payload: &[u8]) -> Result<V2Message, V2CodecError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(V2CodecError::Bound {
            field: "payload",
            value: payload.len(),
        });
    }
    let mut reader = Reader {
        bytes: payload,
        offset: 0,
    };
    let message = match message_type {
        14 => {
            let path = reader.blob(MAX_PATH, "path")?;
            let page_token = reader.u64()?;
            let page_size = reader.u32()?;
            validate_page_size(page_size)?;
            V2Message::ListRequest {
                path,
                page_token,
                page_size,
            }
        }
        15 => V2Message::ListPage {
            related_id: reader.u64()?,
            page_token: reader.u64()?,
            final_page: reader.bool()?,
            entries: reader.entries()?,
        },
        16 => V2Message::StatRequest {
            path: reader.blob(MAX_PATH, "path")?,
            include_digest: reader.bool()?,
        },
        17 => {
            let related_id = reader.u64()?;
            let status = StatStatus::decode(reader.u8()?)?;
            let (entry, digest, error) = match status {
                StatStatus::Ok => {
                    let entry = decode_entry(&mut reader)?;
                    let has_digest = reader.bool()?;
                    let digest = has_digest.then(|| reader.array()).transpose()?;
                    (Some(entry), digest, Vec::new())
                }
                StatStatus::Missing => {
                    if reader.bool()? {
                        return Err(V2CodecError::InvalidStatFields);
                    }
                    (None, None, Vec::new())
                }
                StatStatus::Error => (None, None, reader.utf8_blob(MAX_ERROR, "error message")?),
            };
            V2Message::StatResponse {
                related_id,
                status,
                entry,
                digest,
                error,
            }
        }
        18 => V2Message::CancelRequest {
            related_id: reader.u64()?,
        },
        19 => V2Message::Keepalive {
            nonce: reader.u64()?,
        },
        20 => V2Message::KeepaliveAck {
            nonce: reader.u64()?,
        },
        21 => V2Message::BrowseError {
            related_id: reader.u64()?,
            code: reader.u16()?,
            message: reader.utf8_blob(MAX_ERROR, "error message")?,
        },
        22 => V2Message::RenameRequest {
            source: reader.blob(MAX_PATH, "source path")?,
            destination: reader.blob(MAX_PATH, "destination path")?,
        },
        23 | 25 | 37 | 39 => {
            let related_id = reader.u64()?;
            let status = MutationStatus::decode(reader.u8()?)?;
            let error = if status == MutationStatus::Ok {
                Vec::new()
            } else {
                reader.utf8_blob(MAX_ERROR, "mutation error")?
            };
            if (status == MutationStatus::Ok && reader.offset != payload.len())
                || (status != MutationStatus::Ok && error.is_empty())
            {
                return Err(V2CodecError::InvalidMutationFields);
            }
            match message_type {
                23 => V2Message::RenameResponse {
                    related_id,
                    status,
                    error,
                },
                25 => V2Message::CreateDirectoryResponse {
                    related_id,
                    status,
                    error,
                },
                37 => V2Message::SetPermissionsResponse {
                    related_id,
                    status,
                    error,
                },
                _ => V2Message::SetMtimeResponse {
                    related_id,
                    status,
                    error,
                },
            }
        }
        24 => V2Message::CreateDirectoryRequest {
            path: reader.blob(MAX_PATH, "path")?,
        },
        26 => V2Message::DeleteRequest {
            path: reader.blob(MAX_PATH, "path")?,
        },
        27 => {
            let related_id = reader.u64()?;
            let path = reader.blob(MAX_PATH, "delete path")?;
            let removed = reader.bool()?;
            let error = if removed {
                Vec::new()
            } else {
                reader.utf8_blob(MAX_ERROR, "delete error")?
            };
            if !removed && error.is_empty() {
                return Err(V2CodecError::InvalidDeleteFields);
            }
            V2Message::DeleteProgress {
                related_id,
                path,
                removed,
                error,
            }
        }
        28 => {
            let related_id = reader.u64()?;
            let status = DeleteStatus::decode(reader.u8()?)?;
            let removed_count = reader.u64()?;
            if !reader.bool()? {
                return Err(V2CodecError::InvalidDeleteFields);
            }
            let failures = reader.failures()?;
            V2Message::DeleteResponse {
                related_id,
                status,
                removed_count,
                failures,
                irreversible: true,
            }
        }
        29 => V2Message::FetchRequest {
            path: reader.blob(MAX_PATH, "path")?,
        },
        30 => V2Message::FetchStart {
            related_id: reader.u64()?,
            size: reader.u64()?,
            mtime_ns: reader.i64()?,
            device: reader.u64()?,
            file: reader.u64()?,
            digest: reader.array()?,
        },
        31 => V2Message::FetchChunk {
            related_id: reader.u64()?,
            offset: reader.u64()?,
            data: reader.blob(MAX_COMPLETE_FETCH_CHUNK, "fetch chunk")?,
        },
        32 => V2Message::PublishRequest {
            path: reader.blob(MAX_PATH, "path")?,
            size: reader.u64()?,
            mtime_ns: reader.i64()?,
            device: reader.u64()?,
            file: reader.u64()?,
            content_size: reader.u64()?,
            digest: reader.array()?,
        },
        33 => V2Message::PublishReady {
            related_id: reader.u64()?,
        },
        34 => V2Message::PublishChunk {
            related_id: reader.u64()?,
            offset: reader.u64()?,
            data: reader.blob(MAX_COMPLETE_FETCH_CHUNK, "publish chunk")?,
        },
        35 => {
            let related_id = reader.u64()?;
            let status = PublishStatus::decode(reader.u8()?)?;
            let current_present = reader.bool()?;
            let (size, mtime_ns, device, file) = if current_present {
                (reader.u64()?, reader.i64()?, reader.u64()?, reader.u64()?)
            } else {
                (0, 0, 0, 0)
            };
            let error = reader.utf8_blob(MAX_ERROR, "publish error")?;
            if (status == PublishStatus::Ok && !error.is_empty())
                || (status != PublishStatus::Ok && error.is_empty())
            {
                return Err(V2CodecError::InvalidMutationFields);
            }
            V2Message::PublishResponse {
                related_id,
                status,
                current_present,
                size,
                mtime_ns,
                device,
                file,
                error,
            }
        }
        36 => V2Message::SetPermissionsRequest {
            path: reader.blob(MAX_PATH, "path")?,
            mode: reader.u32()?,
        },
        38 => V2Message::SetMtimeRequest {
            path: reader.blob(MAX_PATH, "path")?,
            mtime_ns: reader.i64()?,
        },
        40 => V2Message::ReadLinkRequest {
            path: reader.blob(MAX_PATH, "path")?,
        },
        41 => {
            let related_id = reader.u64()?;
            let status = StatStatus::decode(reader.u8()?)?;
            let (target, error) = match status {
                StatStatus::Ok => (reader.blob(MAX_PATH, "symlink target")?, Vec::new()),
                StatStatus::Missing => (Vec::new(), Vec::new()),
                StatStatus::Error => (Vec::new(), reader.utf8_blob(MAX_ERROR, "error message")?),
            };
            validate_read_link_fields(status, &target, &error)?;
            V2Message::ReadLinkResponse {
                related_id,
                status,
                target,
                error,
            }
        }
        value => {
            return Err(V2CodecError::InvalidEnum {
                field: "message type",
                value,
            })
        }
    };
    if reader.offset != payload.len() {
        return Err(V2CodecError::Trailing(payload.len() - reader.offset));
    }
    Ok(message)
}

fn validate_page_size(page_size: u32) -> Result<(), V2CodecError> {
    if page_size == 0 || page_size as usize > MAX_COLLECTION {
        return Err(V2CodecError::Bound {
            field: "page size",
            value: page_size as usize,
        });
    }
    Ok(())
}

fn validate_read_link_fields(
    status: StatStatus,
    target: &[u8],
    error: &[u8],
) -> Result<(), V2CodecError> {
    match status {
        StatStatus::Ok if error.is_empty() => Ok(()),
        StatStatus::Missing if target.is_empty() && error.is_empty() => Ok(()),
        StatStatus::Error if target.is_empty() && !error.is_empty() => Ok(()),
        _ => Err(V2CodecError::InvalidStatFields),
    }
}

fn validate_stat_fields(
    status: StatStatus,
    entry: Option<&BrowseEntry>,
    digest: Option<&[u8; 32]>,
    error: &[u8],
) -> Result<(), V2CodecError> {
    match status {
        StatStatus::Ok if entry.is_some() && error.is_empty() => Ok(()),
        StatStatus::Missing if entry.is_none() && digest.is_none() && error.is_empty() => Ok(()),
        StatStatus::Error if entry.is_none() && digest.is_none() && !error.is_empty() => Ok(()),
        _ => Err(V2CodecError::InvalidStatFields),
    }
}

fn encode_entry(writer: &mut Writer, entry: &BrowseEntry) -> Result<(), V2CodecError> {
    if !(1..=4).contains(&entry.kind) || (entry.kind != 3 && !entry.symlink_target.is_empty()) {
        return Err(if (1..=4).contains(&entry.kind) {
            V2CodecError::InvalidEntryFields
        } else {
            V2CodecError::InvalidEnum {
                field: "entry kind",
                value: entry.kind,
            }
        });
    }
    writer.blob(&entry.name, MAX_PATH, "entry name")?;
    writer.u8(entry.kind);
    writer.u64(entry.size);
    writer.i64(entry.mtime_ns);
    writer.u32(entry.mode);
    writer.blob(&entry.symlink_target, MAX_PATH, "symlink target")
}

fn decode_entry(reader: &mut Reader<'_>) -> Result<BrowseEntry, V2CodecError> {
    let entry = BrowseEntry {
        name: reader.blob(MAX_PATH, "entry name")?,
        kind: reader.u8()?,
        size: reader.u64()?,
        mtime_ns: reader.i64()?,
        mode: reader.u32()?,
        symlink_target: reader.blob(MAX_PATH, "symlink target")?,
    };
    if !(1..=4).contains(&entry.kind) {
        return Err(V2CodecError::InvalidEnum {
            field: "entry kind",
            value: entry.kind,
        });
    }
    if entry.kind != 3 && !entry.symlink_target.is_empty() {
        return Err(V2CodecError::InvalidEntryFields);
    }
    Ok(entry)
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }
    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
    fn blob(
        &mut self,
        value: &[u8],
        maximum: usize,
        field: &'static str,
    ) -> Result<(), V2CodecError> {
        if value.len() > maximum {
            return Err(V2CodecError::Bound {
                field,
                value: value.len(),
            });
        }
        self.u32(u32::try_from(value.len()).map_err(|_| V2CodecError::Bound {
            field,
            value: value.len(),
        })?);
        self.bytes(value);
        Ok(())
    }
    fn utf8_blob(
        &mut self,
        value: &[u8],
        maximum: usize,
        field: &'static str,
    ) -> Result<(), V2CodecError> {
        if std::str::from_utf8(value).is_err() {
            return Err(V2CodecError::InvalidUtf8 { field });
        }
        self.blob(value, maximum, field)
    }
    fn entries(&mut self, entries: &[BrowseEntry]) -> Result<(), V2CodecError> {
        if entries.len() > MAX_COLLECTION {
            return Err(V2CodecError::Bound {
                field: "entry count",
                value: entries.len(),
            });
        }
        self.u32(
            u32::try_from(entries.len()).map_err(|_| V2CodecError::Bound {
                field: "entry count",
                value: entries.len(),
            })?,
        );
        for entry in entries {
            encode_entry(self, entry)?;
        }
        Ok(())
    }
    fn failures(&mut self, failures: &[DeleteFailure]) -> Result<(), V2CodecError> {
        if failures.len() > MAX_DELETE_FAILURES {
            return Err(V2CodecError::Bound {
                field: "delete failure count",
                value: failures.len(),
            });
        }
        self.u32(
            u32::try_from(failures.len()).map_err(|_| V2CodecError::Bound {
                field: "delete failure count",
                value: failures.len(),
            })?,
        );
        for failure in failures {
            self.blob(&failure.path, MAX_PATH, "delete failure path")?;
            self.i32(failure.errno);
        }
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], V2CodecError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(V2CodecError::Truncated)?;
        if end > self.bytes.len() {
            return Err(V2CodecError::Truncated);
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, V2CodecError> {
        Ok(*self.take(1)?.first().ok_or(V2CodecError::Truncated)?)
    }
    fn u16(&mut self) -> Result<u16, V2CodecError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| V2CodecError::Truncated)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, V2CodecError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| V2CodecError::Truncated)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, V2CodecError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| V2CodecError::Truncated)?,
        ))
    }
    fn i64(&mut self) -> Result<i64, V2CodecError> {
        Ok(i64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| V2CodecError::Truncated)?,
        ))
    }
    fn i32(&mut self) -> Result<i32, V2CodecError> {
        Ok(i32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| V2CodecError::Truncated)?,
        ))
    }
    fn bool(&mut self) -> Result<bool, V2CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(V2CodecError::InvalidEnum {
                field: "boolean",
                value,
            }),
        }
    }
    fn array(&mut self) -> Result<[u8; 32], V2CodecError> {
        self.take(32)?
            .try_into()
            .map_err(|_| V2CodecError::Truncated)
    }
    fn blob(&mut self, maximum: usize, field: &'static str) -> Result<Vec<u8>, V2CodecError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(V2CodecError::Bound {
                field,
                value: length,
            });
        }
        Ok(self.take(length)?.to_vec())
    }
    fn utf8_blob(&mut self, maximum: usize, field: &'static str) -> Result<Vec<u8>, V2CodecError> {
        let value = self.blob(maximum, field)?;
        if std::str::from_utf8(&value).is_err() {
            return Err(V2CodecError::InvalidUtf8 { field });
        }
        Ok(value)
    }
    fn entries(&mut self) -> Result<Vec<BrowseEntry>, V2CodecError> {
        let count = self.u32()? as usize;
        if count > MAX_COLLECTION {
            return Err(V2CodecError::Bound {
                field: "entry count",
                value: count,
            });
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(decode_entry(self)?);
        }
        Ok(entries)
    }
    fn failures(&mut self) -> Result<Vec<DeleteFailure>, V2CodecError> {
        let count = self.u32()? as usize;
        if count > MAX_DELETE_FAILURES {
            return Err(V2CodecError::Bound {
                field: "delete failure count",
                value: count,
            });
        }
        let mut failures = Vec::with_capacity(count);
        for _ in 0..count {
            failures.push(DeleteFailure {
                path: self.blob(MAX_PATH, "delete failure path")?,
                errno: self.i32()?,
            });
        }
        Ok(failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_messages_round_trip_raw_names() {
        let message = V2Message::ListPage {
            related_id: 7,
            page_token: 9,
            final_page: true,
            entries: vec![BrowseEntry {
                name: vec![0xff, b'x'],
                kind: 1,
                size: 4,
                mtime_ns: -2,
                mode: 0o644,
                symlink_target: Vec::new(),
            }],
        };
        assert_eq!(decode(15, &encode(&message).unwrap()).unwrap(), message);
    }

    #[test]
    fn rejects_invalid_boolean_and_trailing_bytes() {
        let mut payload = encode(&V2Message::StatRequest {
            path: Vec::new(),
            include_digest: false,
        })
        .unwrap();
        *payload.last_mut().unwrap() = 2;
        assert!(matches!(
            decode(16, &payload),
            Err(V2CodecError::InvalidEnum {
                field: "boolean",
                ..
            })
        ));
        let mut payload = encode(&V2Message::Keepalive { nonce: 1 }).unwrap();
        payload.push(0);
        assert!(matches!(
            decode(19, &payload),
            Err(V2CodecError::Trailing(1))
        ));
    }

    #[test]
    fn mutation_messages_round_trip_and_reject_empty_errors() {
        for message in [
            V2Message::RenameRequest {
                source: b"old".to_vec(),
                destination: b"new".to_vec(),
            },
            V2Message::RenameResponse {
                related_id: 4,
                status: MutationStatus::CrossDevice,
                error: b"cross-device".to_vec(),
            },
            V2Message::CreateDirectoryRequest {
                path: b"new-dir".to_vec(),
            },
            V2Message::CreateDirectoryResponse {
                related_id: 5,
                status: MutationStatus::Ok,
                error: Vec::new(),
            },
        ] {
            assert_eq!(
                decode(message_type(&message), &encode(&message).unwrap()).unwrap(),
                message
            );
        }
        assert!(matches!(
            encode(&V2Message::RenameResponse {
                related_id: 1,
                status: MutationStatus::AlreadyExists,
                error: Vec::new(),
            }),
            Err(V2CodecError::InvalidMutationFields)
        ));
    }

    #[test]
    fn delete_messages_round_trip_and_mark_irreversible() {
        let messages = [
            V2Message::DeleteRequest {
                path: b"tree".to_vec(),
            },
            V2Message::DeleteProgress {
                related_id: 9,
                path: b"tree/file".to_vec(),
                removed: true,
                error: Vec::new(),
            },
            V2Message::DeleteProgress {
                related_id: 9,
                path: b"tree/locked".to_vec(),
                removed: false,
                error: b"permission denied".to_vec(),
            },
            V2Message::DeleteResponse {
                related_id: 9,
                status: DeleteStatus::Partial,
                removed_count: 1,
                failures: vec![DeleteFailure {
                    path: b"tree/locked".to_vec(),
                    errno: 13,
                }],
                irreversible: true,
            },
        ];
        for message in messages {
            assert_eq!(
                decode(message_type(&message), &encode(&message).unwrap()).unwrap(),
                message
            );
        }
        assert!(matches!(
            encode(&V2Message::DeleteResponse {
                related_id: 1,
                status: DeleteStatus::Complete,
                removed_count: 0,
                failures: Vec::new(),
                irreversible: false,
            }),
            Err(V2CodecError::InvalidDeleteFields)
        ));
    }

    #[test]
    fn fetch_messages_round_trip_with_bounded_chunks() {
        let messages = [
            V2Message::FetchRequest {
                path: b"edit.txt".to_vec(),
            },
            V2Message::FetchStart {
                related_id: 7,
                size: 3,
                mtime_ns: -1,
                device: 11,
                file: 12,
                digest: [4; 32],
            },
            V2Message::FetchChunk {
                related_id: 7,
                offset: 0,
                data: b"abc".to_vec(),
            },
        ];
        for message in messages {
            assert_eq!(
                decode(message_type(&message), &encode(&message).unwrap()).unwrap(),
                message
            );
        }
    }

    #[test]
    fn publish_messages_round_trip_changed_identity() {
        let messages = [
            V2Message::PublishRequest {
                path: b"edit.txt".to_vec(),
                size: 3,
                mtime_ns: 4,
                device: 5,
                file: 6,
                content_size: 9,
                digest: [7; 32],
            },
            V2Message::PublishReady { related_id: 8 },
            V2Message::PublishChunk {
                related_id: 8,
                offset: 0,
                data: b"new".to_vec(),
            },
            V2Message::PublishResponse {
                related_id: 8,
                status: PublishStatus::Changed,
                current_present: true,
                size: 4,
                mtime_ns: 9,
                device: 10,
                file: 11,
                error: b"changed".to_vec(),
            },
        ];
        for message in messages {
            assert_eq!(
                decode(message_type(&message), &encode(&message).unwrap()).unwrap(),
                message
            );
        }
    }

    #[test]
    fn browse_meta_messages_round_trip() {
        let messages = [
            V2Message::SetPermissionsRequest {
                path: b"a".to_vec(),
                mode: 0o644,
            },
            V2Message::SetPermissionsResponse {
                related_id: 4,
                status: MutationStatus::Ok,
                error: Vec::new(),
            },
            V2Message::SetMtimeRequest {
                path: b"a".to_vec(),
                mtime_ns: 1_000_000_000,
            },
            V2Message::SetMtimeResponse {
                related_id: 5,
                status: MutationStatus::PermissionDenied,
                error: b"permission denied".to_vec(),
            },
            V2Message::ReadLinkRequest {
                path: b"a".to_vec(),
            },
            V2Message::ReadLinkResponse {
                related_id: 6,
                status: StatStatus::Ok,
                target: b"b".to_vec(),
                error: Vec::new(),
            },
            V2Message::ReadLinkResponse {
                related_id: 7,
                status: StatStatus::Missing,
                target: Vec::new(),
                error: Vec::new(),
            },
            V2Message::ReadLinkResponse {
                related_id: 8,
                status: StatStatus::Error,
                target: Vec::new(),
                error: b"not a symlink".to_vec(),
            },
        ];
        for message in messages {
            assert_eq!(
                decode(message_type(&message), &encode(&message).unwrap()).unwrap(),
                message
            );
        }
        assert!(matches!(
            encode(&V2Message::SetPermissionsResponse {
                related_id: 1,
                status: MutationStatus::Error,
                error: Vec::new(),
            }),
            Err(V2CodecError::InvalidMutationFields)
        ));
        assert!(matches!(
            encode(&V2Message::ReadLinkResponse {
                related_id: 1,
                status: StatStatus::Error,
                target: Vec::new(),
                error: Vec::new(),
            }),
            Err(V2CodecError::InvalidStatFields)
        ));
    }
}
