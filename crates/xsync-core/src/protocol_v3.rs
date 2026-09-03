//! Strict payload codec for the protocol v3 filesystem message table, Phase 1.
//!
//! v3 is the random-access filesystem surface specified in `xsyncv3.md` and
//! frozen in `protocol.md` ("v3 message table"). It shares the 32-byte `xsn1`
//! envelope with v1 and v2, carries envelope version `3`, and is selected only
//! when both peers advertise `CAP_VERSION_NEGOTIATION` and `CAP_FS_V3`.
//!
//! Like the v2 codec this module is fail-closed: an unknown type, an unknown
//! flag or presence bit, an out-of-range length, an inconsistent field pair, or
//! a trailing byte is an error, and no decode failure is ever taken as a reason
//! to fall back to an older grammar. The Rust types here are a convenience; the
//! wire contract is the field-by-field layout in `protocol.md`.

use std::io::Read;

use thiserror::Error;

use crate::protocol::{
    FRAME_HEADER_LEN, MAX_COLLECTION_COUNT, MAX_COMPLETE_PAYLOAD, MAX_DATA_SEGMENT,
    MAX_ENCODED_PATH, MAX_ERROR_MESSAGE,
};
use crate::HANDSHAKE_MAGIC;

/// `FRAME_HEADER_LEN` as it appears in the 16-bit header-length field.
const FRAME_HEADER_LEN_U16: u16 = {
    const _: () = assert!(FRAME_HEADER_LEN <= u16::MAX as usize);
    // Truncation is impossible: the assertion above is evaluated at compile
    // time, so a header that outgrew the field would fail the build.
    #[allow(clippy::cast_possible_truncation)]
    let value = FRAME_HEADER_LEN as u16;
    value
};

/// Envelope version carried by every v3 frame after selection.
pub const V3_ENVELOPE_VERSION: u32 = 3;

const MAX_PATH: usize = MAX_ENCODED_PATH;
const MAX_TEXT: usize = MAX_ERROR_MESSAGE;
const MAX_DATA: usize = MAX_DATA_SEGMENT;
const MAX_COLLECTION: usize = MAX_COLLECTION_COUNT;
const MAX_PAYLOAD: usize = MAX_COMPLETE_PAYLOAD;
/// Maximum bytes in an owner name, group name, or filesystem type string.
pub const MAX_NAME: usize = 256;
/// Maximum Unix permission bits carried in `Attrs.mode` or `Open.mode`.
pub const MAX_MODE: u32 = 0o7777;

/// Frozen v3 message type bytes. Types 18–20 are shared with v2 unchanged.
pub mod types {
    /// Shared with v2: abandon one in-flight request.
    pub const CANCEL: u8 = 18;
    /// Shared with v2: liveness probe.
    pub const KEEPALIVE: u8 = 19;
    /// Shared with v2: liveness reply.
    pub const KEEPALIVE_ACK: u8 = 20;
    /// Client's v3 feature bitmap.
    pub const FEATURES: u8 = 42;
    /// Server's v3 feature bitmap.
    pub const FEATURES_ACK: u8 = 43;
    /// Attach to an export.
    pub const MOUNT: u8 = 50;
    /// Facts about the attached export, including writability.
    pub const MOUNT_INFO: u8 = 51;
    /// Open a file or directory handle.
    pub const OPEN: u8 = 56;
    /// Handle and attributes for an `Open`.
    pub const OPENED: u8 = 57;
    /// Release a handle.
    pub const CLOSE: u8 = 58;
    /// Positional read.
    pub const READ: u8 = 59;
    /// Bytes for a `Read`.
    pub const READ_DATA: u8 = 60;
    /// Positional write.
    pub const WRITE: u8 = 61;
    /// Outcome of a `Write`.
    pub const WRITE_ACK: u8 = 62;
    /// Make a handle's writes durable.
    pub const FLUSH: u8 = 63;
    /// Begin or resume a staged upload (`xsyncv3.md` E4-S6).
    pub const STAGE_OPEN: u8 = 70;
    /// Stage identity and what is already on disk.
    pub const STAGE_OPENED: u8 = 71;
    /// One range into a stage.
    pub const STAGE_WRITE: u8 = 72;
    /// Outcome of a `StageWrite`.
    pub const STAGE_ACK: u8 = 73;
    /// Ask which ranges a stage already holds.
    pub const STAGE_STATUS: u8 = 74;
    /// The committed range set, paged.
    pub const STAGE_RANGES: u8 = 75;
    /// Verify and publish a stage.
    pub const STAGE_COMMIT: u8 = 76;
    /// Outcome of a `StageCommit`.
    pub const STAGE_RESULT: u8 = 77;
    /// Discard a stage.
    pub const STAGE_ABORT: u8 = 78;
    /// Positional write guarded by the destination's change cookie (E4-S7).
    pub const WRITE_CAS: u8 = 79;
    /// Attributes of a path or handle.
    pub const STAT: u8 = 80;
    /// Attributes for a `Stat`.
    pub const ATTRS: u8 = 81;
    /// One page of a directory handle.
    pub const READ_DIR: u8 = 82;
    /// Entries for a `ReadDir`.
    pub const DIR_PAGE: u8 = 83;
    /// Capacity and filesystem facts for the mount.
    pub const STAT_FS: u8 = 84;
    /// Reply to `StatFs`.
    pub const FS_INFO: u8 = 85;
    /// Rename or exchange a path (`xsyncv3.md` E5-S4).
    pub const RENAME: u8 = 86;
    /// Remove exactly one non-directory.
    pub const UNLINK: u8 = 87;
    /// Remove exactly one empty directory.
    pub const RMDIR: u8 = 88;
    /// Create one directory level.
    pub const MKDIR: u8 = 89;
    /// Create a symbolic link.
    pub const SYMLINK: u8 = 90;
    /// Create a hard link.
    pub const LINK: u8 = 91;
    /// Change owner and/or group.
    pub const CHOWN: u8 = 92;
    /// Change access and/or modification times.
    pub const SET_TIMES: u8 = 93;
    /// Change permission bits.
    pub const SET_PERMISSIONS: u8 = 94;
    /// Shared reply for the mutations that leave something to describe.
    pub const MUTATED: u8 = 95;
    /// Request-scoped failure.
    pub const ERROR: u8 = 121;
    /// Request-scoped success with no payload.
    pub const DONE: u8 = 122;
}

/// `Open.flags` bits. Any other bit is a protocol error.
pub mod open_flags {
    /// Read access.
    pub const READ: u32 = 1 << 0;
    /// Write access; required by every mutating flag below.
    pub const WRITE: u32 = 1 << 1;
    /// Create when missing.
    pub const CREATE: u32 = 1 << 2;
    /// Fail when present; requires `CREATE`.
    pub const EXCL: u32 = 1 << 3;
    /// Truncate on open.
    pub const TRUNC: u32 = 1 << 4;
    /// Every write lands at the current end of file.
    pub const APPEND: u32 = 1 << 5;
    /// Refuse to follow a final symlink.
    pub const NOFOLLOW: u32 = 1 << 6;
    /// Open a directory for `ReadDir`; excludes every write flag.
    pub const DIRECTORY: u32 = 1 << 7;
    /// Every bit with a defined meaning.
    pub const KNOWN: u32 = 0xff;
    const WRITE_CLASS: u32 = CREATE | EXCL | TRUNC | APPEND;

    /// Reject flag combinations the contract forbids.
    pub(super) fn validate(flags: u32) -> Result<(), super::V3CodecError> {
        use super::V3CodecError;
        if flags & !KNOWN != 0 {
            return Err(V3CodecError::UnknownFlags {
                field: "open flags",
                value: u64::from(flags),
            });
        }
        if flags & (READ | WRITE | DIRECTORY) == 0 {
            return Err(V3CodecError::Inconsistent(
                "open flags need READ, WRITE or DIRECTORY",
            ));
        }
        if flags & WRITE_CLASS != 0 && flags & WRITE == 0 {
            return Err(V3CodecError::Inconsistent(
                "open flags require WRITE for CREATE, EXCL, TRUNC or APPEND",
            ));
        }
        if flags & EXCL != 0 && flags & CREATE == 0 {
            return Err(V3CodecError::Inconsistent(
                "open flags require CREATE for EXCL",
            ));
        }
        if flags & DIRECTORY != 0 && flags & (WRITE | WRITE_CLASS) != 0 {
            return Err(V3CodecError::Inconsistent(
                "open flags: DIRECTORY excludes every write flag",
            ));
        }
        Ok(())
    }
}

/// `Attrs` presence bitmap. Optional fields follow the fixed part in bit order.
pub mod attr_presence {
    /// `uid u32, gid u32`.
    pub const OWNER: u32 = 1 << 0;
    /// `nlink u32`.
    pub const NLINK: u32 = 1 << 1;
    /// `atime_ns i64`.
    pub const ATIME: u32 = 1 << 2;
    /// `ctime_ns i64`.
    pub const CTIME: u32 = 1 << 3;
    /// `btime_ns i64`.
    pub const BTIME: u32 = 1 << 4;
    /// `dev u64, ino u64`.
    pub const IDENTITY: u32 = 1 << 5;
    /// `rdev u64`.
    pub const RDEV: u32 = 1 << 6;
    /// Symlink target blob; valid only for kind `3`.
    pub const SYMLINK_TARGET: u32 = 1 << 7;
    /// `allocated_size u64`.
    pub const ALLOCATED_SIZE: u32 = 1 << 8;
    /// `owner_name` and `group_name` UTF-8 blobs.
    pub const NAMES: u32 = 1 << 9;
    /// `flags u32` (see [`attr_flags`](super::attr_flags)).
    pub const FLAGS: u32 = 1 << 10;
    /// Every bit with a defined meaning.
    pub const KNOWN: u32 = 0x7ff;

    /// A request's `attr_mask` names the optional blocks the client wants,
    /// using this same numbering. Unknown *mask* bits are ignored, like
    /// capability bits, so a newer client degrades against an older server;
    /// unknown *presence* bits are rejected, because a decoder cannot skip a
    /// block whose length it does not know. A response's presence bitmap is a
    /// subset of the mask and may be a strict subset.
    pub const MASK_FIXED_PART_ONLY: u32 = 0;
}

/// `Attrs.flags` bits. Unknown bits are preserved, not rejected: they describe
/// the file, not the grammar.
pub mod attr_flags {
    /// Immutable file.
    pub const IMMUTABLE: u32 = 1 << 0;
    /// Append-only file.
    pub const APPEND_ONLY: u32 = 1 << 1;
    /// Hidden by platform convention (Windows/macOS hidden bit).
    pub const HIDDEN: u32 = 1 << 2;
}

/// `MountInfo.supports` bits: what the export's filesystem can do. Unknown bits
/// are preserved, not rejected, so a newer server may advertise more.
pub mod supports {
    /// Extended attributes.
    pub const XATTRS: u64 = 1 << 0;
    /// Symbolic links.
    pub const SYMLINKS: u64 = 1 << 1;
    /// Hard links.
    pub const HARDLINKS: u64 = 1 << 2;
    /// Byte-range locks.
    pub const LOCKS: u64 = 1 << 3;
    /// Leases.
    pub const LEASES: u64 = 1 << 4;
    /// Change notification.
    pub const NOTIFY: u64 = 1 << 5;
    /// Change notification by polling.
    pub const NOTIFY_POLLING: u64 = 1 << 6;
    /// Sparse-aware allocation and seek.
    pub const SPARSE: u64 = 1 << 7;
    /// Names differing only by case are the same file (APFS, NTFS).
    ///
    /// Distinct from `MountInfo.case_sensitive`, which is the same fact; this
    /// bit exists so the pair below can be reported together.
    pub const CASE_INSENSITIVE: u64 = 1 << 8;
    /// Canonically-equivalent Unicode forms are the same file.
    ///
    /// `MountInfo.normalization` says which form the filesystem *applies*;
    /// this says whether it can tell two forms apart, which is the question a
    /// client writing names actually has. A filesystem may preserve what it is
    /// given (applying nothing) and still fold the two for comparison, so
    /// neither field implies the other.
    pub const NORMALIZATION_INSENSITIVE: u64 = 1 << 9;
}

/// `Features` bitmap: optional v3 message groups. The negotiated set is the
/// intersection; unknown bits are ignored so peers of different ages agree.
pub mod features {
    /// Byte-range locks (types 100–104).
    pub const LOCKS: u64 = 1 << 0;
    /// Leases and lease breaks.
    pub const LEASES: u64 = 1 << 1;
    /// Share/deny modes on `Open`.
    pub const SHARE_MODES: u64 = 1 << 2;
    /// Directory watches.
    pub const NOTIFY: u64 = 1 << 3;
    /// Directory watches by polling.
    pub const NOTIFY_POLLING: u64 = 1 << 4;
    /// Extended attribute verbs.
    pub const XATTR: u64 = 1 << 5;
    /// Sparse verbs.
    pub const SPARSE: u64 = 1 << 6;
    /// Compound requests.
    pub const COMPOUND: u64 = 1 << 7;
    /// Staged resumable uploads.
    pub const STAGE_RESUME: u64 = 1 << 8;
    /// `Access` queries.
    pub const ACCESS: u64 = 1 << 9;
    /// Server resolves owner/group names.
    pub const OWNER_NAMES: u64 = 1 << 10;
}

/// Requested or granted access to an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Access {
    /// Read and write.
    ReadWrite = 0,
    /// Read only.
    ReadOnly = 1,
}

impl Access {
    fn decode(value: u8) -> Result<Self, V3CodecError> {
        match value {
            0 => Ok(Self::ReadWrite),
            1 => Ok(Self::ReadOnly),
            _ => Err(V3CodecError::InvalidEnum {
                field: "access",
                value,
            }),
        }
    }
}

/// How a `StageCommit` ended.
///
/// An enum rather than a boolean, and carried in the *reply* rather than
/// signalled by an `Error`, for two reasons that pull the same way. A refused
/// commit has to hand back the destination's current attributes — that is the
/// whole point of compare-and-swap, and an `Error` has nowhere to put them. And
/// a client must not be able to read the refusal as a success by ignoring a
/// field: matching on an enum has no default arm that means "committed", where
/// `if !refused` has exactly that shape when someone forgets to check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommitOutcome {
    /// The stage was verified and published. `attrs` describes the new file.
    Committed = 0,
    /// `expect_cookie` did not match. Nothing was published, and `attrs`
    /// describes the destination as it now stands.
    Changed = 1,
}

impl CommitOutcome {
    fn decode(value: u8) -> Result<Self, V3CodecError> {
        match value {
            0 => Ok(Self::Committed),
            1 => Ok(Self::Changed),
            _ => Err(V3CodecError::InvalidEnum {
                field: "commit_outcome",
                value,
            }),
        }
    }
}

/// What `Rename` should do when the destination already exists.
///
/// An enum rather than the bitmask the story called "flags": the three
/// behaviours are mutually exclusive, and a bitmask can encode a contradiction
/// (`NOREPLACE | EXCHANGE`) that then has to be rejected at every call site. A
/// value the decoder does not know is refused, as everywhere else in v3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RenameMode {
    /// Fail with `EEXIST` when the destination exists. The v2 behaviour.
    NoReplace = 0,
    /// Replace the destination atomically. Both must be on one filesystem.
    Replace = 1,
    /// Swap the two paths atomically; both must exist.
    Exchange = 2,
}

impl RenameMode {
    fn decode(value: u8) -> Result<Self, V3CodecError> {
        match value {
            0 => Ok(Self::NoReplace),
            1 => Ok(Self::Replace),
            2 => Ok(Self::Exchange),
            _ => Err(V3CodecError::InvalidEnum {
                field: "rename_mode",
                value,
            }),
        }
    }
}

/// One timestamp in a `SetTimes`, with `utimensat`'s three cases.
///
/// `Omit` and `Now` are distinct from any value the client could send: `Now`
/// must be resolved by the *server*, because the client's clock is not the one
/// the filesystem stamps with, and `Omit` has to leave the existing value
/// alone rather than rewrite it with what a stat happened to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeChange {
    /// Leave this timestamp as it is (`UTIME_OMIT`).
    Omit,
    /// Set it to the server's current time (`UTIME_NOW`).
    Now,
    /// Set it to an explicit time.
    Set {
        /// Seconds since the Unix epoch; negative is before it.
        seconds: i64,
        /// Nanoseconds within the second, `0..1_000_000_000`.
        nanos: u32,
    },
}

impl TimeChange {
    const OMIT: u8 = 0;
    const NOW: u8 = 1;
    const SET: u8 = 2;

    fn encode(self, writer: &mut Writer) {
        match self {
            Self::Omit => {
                writer.u8(Self::OMIT);
                writer.i64(0);
                writer.u32(0);
            }
            Self::Now => {
                writer.u8(Self::NOW);
                writer.i64(0);
                writer.u32(0);
            }
            Self::Set { seconds, nanos } => {
                writer.u8(Self::SET);
                writer.i64(seconds);
                writer.u32(nanos);
            }
        }
    }

    fn decode(reader: &mut Reader) -> Result<Self, V3CodecError> {
        // Fixed width whatever the tag, so the frame layout does not depend on
        // a value the peer chose — a decoder that had to branch on the tag to
        // know how far to advance is one bad byte away from desynchronising.
        let tag = reader.u8()?;
        let seconds = reader.i64()?;
        let nanos = reader.u32()?;
        match tag {
            Self::OMIT => Ok(Self::Omit),
            Self::NOW => Ok(Self::Now),
            Self::SET => {
                if nanos >= 1_000_000_000 {
                    return Err(V3CodecError::Bound {
                        field: "nanos",
                        value: nanos as usize,
                    });
                }
                Ok(Self::Set { seconds, nanos })
            }
            value => Err(V3CodecError::InvalidEnum {
                field: "time_change",
                value,
            }),
        }
    }
}

/// Unicode normalization applied by the export's filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Normalization {
    /// Names are stored as given.
    None = 0,
    /// Names are normalized to NFC.
    Nfc = 1,
    /// Names are normalized to NFD.
    Nfd = 2,
}

impl Normalization {
    fn decode(value: u8) -> Result<Self, V3CodecError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Nfc),
            2 => Ok(Self::Nfd),
            _ => Err(V3CodecError::InvalidEnum {
                field: "normalization",
                value,
            }),
        }
    }
}

/// What a `Stat` request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatTarget {
    /// A path relative to the export root (`stat`/`lstat`).
    Path(Vec<u8>),
    /// An open handle (`fstat`).
    Handle(u64),
}

/// Frozen v3 error codes. Values map 1:1 to POSIX errno names where one
/// exists; the last four are xsync-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    /// No such file or directory.
    NoEntry = 1,
    /// Permission denied.
    Access = 2,
    /// Read-only mount or filesystem.
    ReadOnly = 3,
    /// Already exists.
    Exists = 4,
    /// Directory not empty.
    NotEmpty = 5,
    /// Is a directory.
    IsDirectory = 6,
    /// Not a directory.
    NotDirectory = 7,
    /// Cross-device operation.
    CrossDevice = 8,
    /// Stale handle or session.
    Stale = 9,
    /// No space left.
    NoSpace = 10,
    /// Quota exceeded.
    Quota = 11,
    /// Name too long.
    NameTooLong = 12,
    /// Too many symbolic links.
    Loop = 13,
    /// Resource busy.
    Busy = 14,
    /// Would block.
    WouldBlock = 15,
    /// Timed out.
    TimedOut = 16,
    /// Cancelled by the client.
    Cancelled = 17,
    /// Unknown handle.
    BadHandle = 18,
    /// Invalid argument.
    Invalid = 19,
    /// I/O error.
    Io = 20,
    /// Operation not supported by the export's filesystem.
    NotSupported = 21,
    /// Name not representable on this server.
    IllegalSequence = 22,
    /// A payload digest did not match its data.
    Integrity = 23,
    /// A per-session limit was exceeded.
    Limit = 24,
    /// A compare-and-swap precondition failed.
    Changed = 25,
    /// A lease was broken before the operation.
    LeaseBroken = 26,
}

impl ErrorCode {
    /// Every code, in wire order.
    pub const ALL: [Self; 26] = [
        Self::NoEntry,
        Self::Access,
        Self::ReadOnly,
        Self::Exists,
        Self::NotEmpty,
        Self::IsDirectory,
        Self::NotDirectory,
        Self::CrossDevice,
        Self::Stale,
        Self::NoSpace,
        Self::Quota,
        Self::NameTooLong,
        Self::Loop,
        Self::Busy,
        Self::WouldBlock,
        Self::TimedOut,
        Self::Cancelled,
        Self::BadHandle,
        Self::Invalid,
        Self::Io,
        Self::NotSupported,
        Self::IllegalSequence,
        Self::Integrity,
        Self::Limit,
        Self::Changed,
        Self::LeaseBroken,
    ];

    /// The frozen errno-style name, for logs and generated docs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NoEntry => "ENOENT",
            Self::Access => "EACCES",
            Self::ReadOnly => "EROFS",
            Self::Exists => "EEXIST",
            Self::NotEmpty => "ENOTEMPTY",
            Self::IsDirectory => "EISDIR",
            Self::NotDirectory => "ENOTDIR",
            Self::CrossDevice => "EXDEV",
            Self::Stale => "ESTALE",
            Self::NoSpace => "ENOSPC",
            Self::Quota => "EDQUOT",
            Self::NameTooLong => "ENAMETOOLONG",
            Self::Loop => "ELOOP",
            Self::Busy => "EBUSY",
            Self::WouldBlock => "EWOULDBLOCK",
            Self::TimedOut => "ETIMEDOUT",
            Self::Cancelled => "ECANCELED",
            Self::BadHandle => "EBADF",
            Self::Invalid => "EINVAL",
            Self::Io => "EIO",
            Self::NotSupported => "EOPNOTSUPP",
            Self::IllegalSequence => "EILSEQ",
            Self::Integrity => "EINTEGRITY",
            Self::Limit => "ELIMIT",
            Self::Changed => "ECHANGED",
            Self::LeaseBroken => "ELEASEBROKEN",
        }
    }

    fn decode(value: u16) -> Result<Self, V3CodecError> {
        Self::ALL
            .iter()
            .copied()
            .find(|code| *code as u16 == value)
            .ok_or(V3CodecError::InvalidErrorCode(value))
    }
}

/// Attributes of one filesystem entry.
///
/// The fixed part is always present; every `Option` is an optional block whose
/// presence is recorded in the leading bitmap, so absent fields cost nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attrs {
    /// Frozen v1 kind value: `1` file, `2` directory, `3` symlink, `4` other.
    pub kind: u8,
    /// Unix permission bits, at most `0o7777`.
    pub mode: u32,
    /// Logical size in bytes.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
    /// Opaque change cookie; equality means "unchanged".
    pub change_cookie: [u8; 16],
    /// `(uid, gid)`.
    pub owner: Option<(u32, u32)>,
    /// Hard link count.
    pub nlink: Option<u32>,
    /// Access time.
    pub atime_ns: Option<i64>,
    /// Inode change time.
    pub ctime_ns: Option<i64>,
    /// Birth time, where the filesystem records one.
    pub btime_ns: Option<i64>,
    /// `(dev, ino)`.
    pub identity: Option<(u64, u64)>,
    /// Device number for device entries.
    pub rdev: Option<u64>,
    /// Raw symlink target; only for kind `3`.
    pub symlink_target: Option<Vec<u8>>,
    /// Allocated size in bytes (sparse files are smaller than `size`).
    pub allocated_size: Option<u64>,
    /// `(owner_name, group_name)` resolved by the server.
    pub names: Option<(Vec<u8>, Vec<u8>)>,
    /// Platform flags, see [`attr_flags`].
    pub flags: Option<u32>,
}

impl Attrs {
    /// The fixed part only, with every optional block absent.
    #[must_use]
    pub const fn minimal(
        kind: u8,
        mode: u32,
        size: u64,
        mtime_ns: i64,
        change_cookie: [u8; 16],
    ) -> Self {
        Self {
            kind,
            mode,
            size,
            mtime_ns,
            change_cookie,
            owner: None,
            nlink: None,
            atime_ns: None,
            ctime_ns: None,
            btime_ns: None,
            identity: None,
            rdev: None,
            symlink_target: None,
            allocated_size: None,
            names: None,
            flags: None,
        }
    }

    fn presence(&self) -> u32 {
        let mut bits = 0;
        if self.owner.is_some() {
            bits |= attr_presence::OWNER;
        }
        if self.nlink.is_some() {
            bits |= attr_presence::NLINK;
        }
        if self.atime_ns.is_some() {
            bits |= attr_presence::ATIME;
        }
        if self.ctime_ns.is_some() {
            bits |= attr_presence::CTIME;
        }
        if self.btime_ns.is_some() {
            bits |= attr_presence::BTIME;
        }
        if self.identity.is_some() {
            bits |= attr_presence::IDENTITY;
        }
        if self.rdev.is_some() {
            bits |= attr_presence::RDEV;
        }
        if self.symlink_target.is_some() {
            bits |= attr_presence::SYMLINK_TARGET;
        }
        if self.allocated_size.is_some() {
            bits |= attr_presence::ALLOCATED_SIZE;
        }
        if self.names.is_some() {
            bits |= attr_presence::NAMES;
        }
        if self.flags.is_some() {
            bits |= attr_presence::FLAGS;
        }
        bits
    }

    fn validate(&self) -> Result<(), V3CodecError> {
        if !(1..=4).contains(&self.kind) {
            return Err(V3CodecError::InvalidEnum {
                field: "entry kind",
                value: self.kind,
            });
        }
        if self.mode > MAX_MODE {
            return Err(V3CodecError::Bound {
                field: "attrs mode",
                value: self.mode as usize,
            });
        }
        if self.symlink_target.is_some() && self.kind != 3 {
            return Err(V3CodecError::Inconsistent(
                "symlink target on a non-symlink entry",
            ));
        }
        Ok(())
    }
}

/// One entry of a `DirPage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Single relative name component, raw bytes.
    pub name: Vec<u8>,
    /// The entry's attributes, carrying the optional blocks the request's
    /// `attr_mask` asked for; a mask of `0` yields the fixed part only.
    pub attrs: Attrs,
}

/// Phase 1 v3 payloads assigned by `protocol.md`.
// `Opened`, `AttrsResponse` and `DirPage` carry an `Attrs`, which is wide because
// of its optional blocks; boxing it would put an allocation on the hottest
// response path for no wire benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V3Message {
    /// Shared with v2: abandon one in-flight request.
    Cancel {
        /// The request to abandon.
        related_id: u64,
    },
    /// Shared with v2: liveness probe.
    Keepalive {
        /// Echoed by the acknowledgement.
        nonce: u64,
    },
    /// Shared with v2: liveness reply.
    KeepaliveAck {
        /// The probe's nonce.
        nonce: u64,
    },
    /// Client's optional-feature bitmap.
    Features {
        /// See [`features`].
        features: u64,
    },
    /// Server's optional-feature bitmap; the session uses the intersection.
    FeaturesAck {
        /// The `Features` request.
        related_id: u64,
        /// See [`features`].
        features: u64,
    },
    /// Attach to an export.
    Mount {
        /// Export name; empty names the server's `--server` root.
        export: Vec<u8>,
        /// The client's preference. The server answers with what it granted.
        requested_access: Access,
    },
    /// Facts about the attached export. `effective_writable` is the single
    /// source of truth for every write affordance in a client.
    MountInfo {
        /// The `Mount` request.
        related_id: u64,
        /// Export name as the server knows it.
        export: Vec<u8>,
        /// Configured access.
        access: Access,
        /// Whether this session may write, after every rule is applied.
        effective_writable: bool,
        /// Why not, when `effective_writable` is false; empty otherwise.
        reason: Vec<u8>,
        /// Operator-supplied option string, shown verbatim by clients.
        options: Vec<u8>,
        /// Whether names differing only by case are distinct.
        case_sensitive: bool,
        /// Normalization the filesystem applies to names.
        normalization: Normalization,
        /// Longest single name component.
        max_name_len: u32,
        /// Longest path.
        max_path_len: u32,
        /// See [`supports`].
        supports: u64,
        /// Largest `Read.length`, `1..=8 MiB`.
        max_read: u32,
        /// Largest `Write.data`, `1..=8 MiB`.
        max_write: u32,
        /// Suggested attribute cache lifetime; `0` is no hint.
        attr_cache_ms: u32,
        /// Suggested directory cache lifetime; `0` is no hint.
        dir_cache_ms: u32,
    },
    /// Open a file or directory.
    Open {
        /// Path relative to the export root.
        path: Vec<u8>,
        /// See [`open_flags`].
        flags: u32,
        /// Creation mode; `0` unless `CREATE` is set.
        mode: u32,
        /// Optional `Attrs` blocks wanted in the reply; see [`attr_presence`].
        attr_mask: u32,
    },
    /// Handle for an `Open`, with the attributes at open time.
    Opened {
        /// The `Open` request.
        related_id: u64,
        /// Session-scoped handle.
        handle: u64,
        /// Attributes at open.
        attrs: Attrs,
    },
    /// Release a handle.
    Close {
        /// The handle to close.
        handle: u64,
    },
    /// Positional read.
    Read {
        /// An open handle.
        handle: u64,
        /// Byte offset.
        offset: u64,
        /// Bytes wanted, `1..=max_read`.
        length: u32,
        /// Whether the reply should carry a BLAKE3 digest of its data.
        want_digest: bool,
    },
    /// Bytes for a `Read`. A short read is legal only when `eof` is set.
    ReadData {
        /// The `Read` request.
        related_id: u64,
        /// Offset of the first byte.
        offset: u64,
        /// The data ends at end of file.
        eof: bool,
        /// BLAKE3 of `data`, when requested.
        digest: Option<[u8; 32]>,
        /// The bytes.
        data: Vec<u8>,
    },
    /// Positional write.
    Write {
        /// An open handle.
        handle: u64,
        /// Byte offset; ignored on an `APPEND` handle.
        offset: u64,
        /// BLAKE3 of `data`, verified before anything is written.
        digest: Option<[u8; 32]>,
        /// The bytes, `1..=max_write`.
        data: Vec<u8>,
    },
    /// Outcome of a `Write`.
    WriteAck {
        /// The `Write` request.
        related_id: u64,
        /// Bytes written.
        bytes_written: u32,
        /// File size after the write.
        new_size: u64,
        /// The bytes are already durable (export configured `sync`).
        stable: bool,
        /// Change cookie after the write.
        change_cookie: [u8; 16],
    },
    /// Make a handle's writes durable.
    Flush {
        /// The handle to flush.
        handle: u64,
    },
    /// Attributes of a path or handle.
    Stat {
        /// What to inspect.
        target: StatTarget,
        /// Follow a final symlink (ignored for a handle target).
        follow: bool,
        /// Optional `Attrs` blocks wanted in the reply; see [`attr_presence`].
        attr_mask: u32,
    },
    /// Attributes for a `Stat`.
    AttrsResponse {
        /// The `Stat` request.
        related_id: u64,
        /// The attributes.
        attrs: Attrs,
    },
    /// One page of a directory handle.
    ReadDir {
        /// A handle opened with `DIRECTORY`.
        handle: u64,
        /// Opaque position; `0` starts a fresh snapshot.
        cursor: u64,
        /// Entries wanted, `1..=65,536`.
        max_entries: u32,
        /// Optional `Attrs` blocks wanted per entry; see [`attr_presence`].
        attr_mask: u32,
    },
    /// Entries for a `ReadDir`.
    DirPage {
        /// The `ReadDir` request.
        related_id: u64,
        /// Position for the next page.
        cursor: u64,
        /// No more entries follow.
        final_page: bool,
        /// The entries.
        entries: Vec<DirEntry>,
    },
    /// Capacity and filesystem facts for the mount.
    StatFs,
    /// Reply to `StatFs`.
    FsInfo {
        /// The `StatFs` request.
        related_id: u64,
        /// Filesystem block size.
        block_size: u32,
        /// Total bytes.
        total_bytes: u64,
        /// Free bytes.
        free_bytes: u64,
        /// Bytes available to this identity.
        available_bytes: u64,
        /// Total inodes, `0` when unknown.
        total_inodes: u64,
        /// Free inodes, `0` when unknown.
        free_inodes: u64,
        /// Filesystem type name, UTF-8.
        fs_type: Vec<u8>,
        /// Longest single name component.
        max_name_len: u32,
        /// Whether names differing only by case are distinct.
        case_sensitive: bool,
        /// Normalization the filesystem applies to names.
        normalization: Normalization,
        /// The filesystem itself is read-only (distinct from the export).
        read_only: bool,
    },
    /// Begin or resume a staged upload.
    ///
    /// The `resume_token` is what makes a stage outlive the connection that
    /// created it: it is written beside the temporary file, so a client that
    /// reconnects — or comes back after the server restarted — hands the same
    /// token to `StageOpen` and continues. Session resume (E3-S2) is a
    /// different mechanism for a different problem and this deliberately does
    /// not depend on it.
    StageOpen {
        /// Where the finished file should end up, relative to the export root.
        destination: Vec<u8>,
        /// Expected final size, or `0` when the client does not know it yet.
        size: u64,
        /// BLAKE3 of the whole file, when known in advance.
        digest: Option<[u8; 32]>,
        /// Permission bits for the published file, before umask.
        mode: u32,
        /// Empty to start a stage; a token from a previous `StageOpened` to
        /// resume one.
        resume_token: Vec<u8>,
    },
    /// A stage's identity, and how much of it already exists.
    StageOpened {
        /// The `StageOpen` request.
        related_id: u64,
        /// Session-scoped handle for the stage.
        stage_id: u64,
        /// Hand this back to `StageOpen` to resume after a disconnect.
        resume_token: Vec<u8>,
        /// Bytes already staged, so a resuming client knows where to look.
        staged_bytes: u64,
    },
    /// Write one range into a stage.
    StageWrite {
        /// From `StageOpened`.
        stage_id: u64,
        /// Byte offset in the finished file.
        offset: u64,
        /// BLAKE3 of `data`, verified before anything is written.
        digest: Option<[u8; 32]>,
        /// The bytes, `1..=max_write`.
        data: Vec<u8>,
    },
    /// Outcome of a `StageWrite`.
    StageAck {
        /// The `StageWrite` request.
        related_id: u64,
        /// Bytes accepted.
        bytes_written: u32,
        /// Total bytes the stage now holds.
        staged_bytes: u64,
    },
    /// Ask which ranges a stage already holds.
    StageStatus {
        /// From `StageOpened`.
        stage_id: u64,
        /// `0` to start; a cursor from a previous `StageRanges` to continue.
        cursor: u64,
    },
    /// One page of a stage's committed range set.
    StageRanges {
        /// The `StageStatus` request.
        related_id: u64,
        /// Where to resume paging; meaningless when `final_page` is set.
        cursor: u64,
        /// No further pages follow.
        final_page: bool,
        /// Half-open `[start, end)` byte ranges, ascending and disjoint.
        ranges: Vec<(u64, u64)>,
    },
    /// Verify a stage and publish it.
    StageCommit {
        /// From `StageOpened`.
        stage_id: u64,
        /// BLAKE3 of the finished file, checked before anything is published.
        digest: [u8; 32],
        /// The destination's expected `change_cookie` (E4-S7).
        ///
        /// `None` publishes unconditionally. `Some` publishes only if the
        /// destination still has that cookie — or does not exist, when the
        /// cookie is all zeroes, which is how "create, and only create" is
        /// expressed.
        expect_cookie: Option<[u8; 16]>,
        /// Modification time to stamp, or `None` to leave the server's.
        mtime_ns: Option<i64>,
    },
    /// Outcome of a `StageCommit`.
    StageResult {
        /// The `StageCommit` request.
        related_id: u64,
        /// Whether it published, or was refused by the cookie check.
        outcome: CommitOutcome,
        /// The destination afterwards: the new file, or the one in the way.
        attrs: Attrs,
    },
    /// Discard a stage and its temporary file.
    StageAbort {
        /// From `StageOpened`.
        stage_id: u64,
    },
    /// Positional write guarded by the destination's change cookie.
    ///
    /// E4-S7 asked for an optional `expect_cookie` on `Write`. `Write` is in
    /// the frozen Phase 1 table, so a field cannot be added to it without
    /// breaking every peer that already speaks v3; this is the same request
    /// with the cookie required.
    ///
    /// The check happens immediately before the write, inside the handle's
    /// exclusive ordering domain. That closes the case this exists for — an
    /// editor saving over a file someone else edited minutes ago — and it is
    /// **not** a lock: another handle can still write between the check and
    /// the write. Locking is E6-S1.
    WriteCas {
        /// An open handle.
        handle: u64,
        /// Byte offset; ignored on an `APPEND` handle.
        offset: u64,
        /// The cookie the file must still have.
        expect_cookie: [u8; 16],
        /// BLAKE3 of `data`, verified before anything is written.
        digest: Option<[u8; 32]>,
        /// The bytes, `1..=max_write`.
        data: Vec<u8>,
    },
    /// Rename or exchange a path.
    Rename {
        /// Source path, relative to the export root.
        source: Vec<u8>,
        /// Destination path, relative to the export root.
        destination: Vec<u8>,
        /// What to do about an existing destination.
        mode: RenameMode,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Remove exactly one non-directory.
    Unlink {
        /// Path to remove.
        path: Vec<u8>,
    },
    /// Remove exactly one empty directory.
    Rmdir {
        /// Directory to remove.
        path: Vec<u8>,
    },
    /// Create one directory level.
    Mkdir {
        /// Directory to create; its parent must exist.
        path: Vec<u8>,
        /// Permission bits before umask.
        mode: u32,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Create a symbolic link.
    Symlink {
        /// What the link should point at. Stored verbatim, never resolved.
        target: Vec<u8>,
        /// Where to create the link, relative to the export root.
        path: Vec<u8>,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Create a hard link.
    Link {
        /// The existing path.
        existing: Vec<u8>,
        /// The new name for it.
        path: Vec<u8>,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Change owner and/or group.
    Chown {
        /// Path or open handle.
        target: StatTarget,
        /// New owner, or `None` to leave it.
        uid: Option<u32>,
        /// New group, or `None` to leave it.
        gid: Option<u32>,
        /// Whether to follow a final symlink.
        follow: bool,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Change access and/or modification times.
    SetTimes {
        /// Path or open handle.
        target: StatTarget,
        /// What to do with the access time.
        atime: TimeChange,
        /// What to do with the modification time.
        mtime: TimeChange,
        /// Whether to follow a final symlink.
        follow: bool,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// Change permission bits.
    SetPermissions {
        /// Path or open handle.
        target: StatTarget,
        /// New mode bits.
        mode: u32,
        /// Whether to follow a final symlink.
        follow: bool,
        /// Optional `Attrs` blocks wanted for the result.
        attr_mask: u32,
    },
    /// What a mutation left behind.
    ///
    /// One reply for every mutation that produces or changes something, so the
    /// client gets the new `change_cookie` without a follow-up `Stat`. The
    /// removals answer `Done` instead: there is nothing left to describe.
    Mutated {
        /// The request.
        related_id: u64,
        /// Attributes of the affected path after the change.
        attrs: Attrs,
    },
    /// Request-scoped failure.
    Error {
        /// The failed request.
        related_id: u64,
        /// Frozen code.
        code: ErrorCode,
        /// Platform errno, `0` when unavailable.
        platform_errno: i32,
        /// UTF-8 detail.
        message: Vec<u8>,
    },
    /// Request-scoped success with nothing to return (`Close`, `Flush`).
    Done {
        /// The completed request.
        related_id: u64,
    },
}

/// One decoded v3 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V3Frame {
    /// Unique frame identifier.
    pub message_id: u64,
    /// Typed payload.
    pub message: V3Message,
}

/// Payload codec failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum V3CodecError {
    /// A length or count exceeds the frozen bound.
    #[error("v3 {field} exceeds its bound: {value}")]
    Bound { field: &'static str, value: usize },
    /// The payload ended before a field was complete.
    #[error("truncated v3 payload")]
    Truncated,
    /// A boolean or enum value is invalid.
    #[error("invalid v3 {field} value: {value}")]
    InvalidEnum { field: &'static str, value: u8 },
    /// An error code is outside the frozen table.
    #[error("invalid v3 error code: {0}")]
    InvalidErrorCode(u16),
    /// A flag or presence bitmap carries a bit without a defined meaning.
    #[error("unknown v3 {field} bits: {value:#x}")]
    UnknownFlags { field: &'static str, value: u64 },
    /// Two fields that must agree do not.
    #[error("inconsistent v3 fields: {0}")]
    Inconsistent(&'static str),
    /// A required UTF-8 field is not UTF-8.
    #[error("v3 {field} is not UTF-8")]
    InvalidUtf8 { field: &'static str },
    /// Bytes remain after the typed payload.
    #[error("trailing v3 payload bytes: {0}")]
    Trailing(usize),
    /// The v3 envelope is malformed.
    #[error("malformed v3 envelope: {0}")]
    Envelope(&'static str),
    /// Reading a v3 frame failed.
    #[error("read v3 frame: {0}")]
    Io(String),
}

/// The frozen type byte for a message.
#[must_use]
pub const fn message_type(message: &V3Message) -> u8 {
    match message {
        V3Message::Cancel { .. } => types::CANCEL,
        V3Message::Keepalive { .. } => types::KEEPALIVE,
        V3Message::KeepaliveAck { .. } => types::KEEPALIVE_ACK,
        V3Message::Features { .. } => types::FEATURES,
        V3Message::FeaturesAck { .. } => types::FEATURES_ACK,
        V3Message::Mount { .. } => types::MOUNT,
        V3Message::MountInfo { .. } => types::MOUNT_INFO,
        V3Message::Open { .. } => types::OPEN,
        V3Message::Opened { .. } => types::OPENED,
        V3Message::Close { .. } => types::CLOSE,
        V3Message::Read { .. } => types::READ,
        V3Message::ReadData { .. } => types::READ_DATA,
        V3Message::Write { .. } => types::WRITE,
        V3Message::WriteAck { .. } => types::WRITE_ACK,
        V3Message::Flush { .. } => types::FLUSH,
        V3Message::Stat { .. } => types::STAT,
        V3Message::AttrsResponse { .. } => types::ATTRS,
        V3Message::ReadDir { .. } => types::READ_DIR,
        V3Message::DirPage { .. } => types::DIR_PAGE,
        V3Message::StatFs => types::STAT_FS,
        V3Message::FsInfo { .. } => types::FS_INFO,
        V3Message::StageOpen { .. } => types::STAGE_OPEN,
        V3Message::StageOpened { .. } => types::STAGE_OPENED,
        V3Message::StageWrite { .. } => types::STAGE_WRITE,
        V3Message::StageAck { .. } => types::STAGE_ACK,
        V3Message::StageStatus { .. } => types::STAGE_STATUS,
        V3Message::StageRanges { .. } => types::STAGE_RANGES,
        V3Message::StageCommit { .. } => types::STAGE_COMMIT,
        V3Message::StageResult { .. } => types::STAGE_RESULT,
        V3Message::StageAbort { .. } => types::STAGE_ABORT,
        V3Message::WriteCas { .. } => types::WRITE_CAS,
        V3Message::Rename { .. } => types::RENAME,
        V3Message::Unlink { .. } => types::UNLINK,
        V3Message::Rmdir { .. } => types::RMDIR,
        V3Message::Mkdir { .. } => types::MKDIR,
        V3Message::Symlink { .. } => types::SYMLINK,
        V3Message::Link { .. } => types::LINK,
        V3Message::Chown { .. } => types::CHOWN,
        V3Message::SetTimes { .. } => types::SET_TIMES,
        V3Message::SetPermissions { .. } => types::SET_PERMISSIONS,
        V3Message::Mutated { .. } => types::MUTATED,
        V3Message::Error { .. } => types::ERROR,
        V3Message::Done { .. } => types::DONE,
    }
}

/// Encode a complete uncompressed v3 frame.
///
/// # Errors
///
/// Returns [`V3CodecError::Bound`] when the encoded payload is longer than the
/// `u32` length field the frame header carries, and propagates any error from
/// encoding the message body itself.
pub fn encode_frame(message_id: u64, message: &V3Message) -> Result<Vec<u8>, V3CodecError> {
    let message_type = message_type(message);
    let payload = encode(message)?;
    let payload_len = u32::try_from(payload.len()).map_err(|_| V3CodecError::Bound {
        field: "payload",
        value: payload.len(),
    })?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    frame.extend_from_slice(HANDSHAKE_MAGIC);
    frame.extend_from_slice(&FRAME_HEADER_LEN_U16.to_le_bytes());
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&V3_ENVELOPE_VERSION.to_le_bytes());
    frame.push(message_type);
    frame.push(0);
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&message_id.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode one complete v3 frame, rejecting trailing bytes.
///
/// # Errors
///
/// Returns [`V3CodecError::Envelope`] for a header that is truncated, declares
/// a header length or protocol version this grammar does not speak, sets a
/// reserved field, or is followed by a payload whose length disagrees with the
/// header. Body decode errors are propagated unchanged.
///
/// # Panics
///
/// Does not panic in practice: the fixed-width conversions below are
/// infallible once the length check has established that `header` is exactly
/// `FRAME_HEADER_LEN` bytes.
pub fn decode_frame(bytes: &[u8]) -> Result<V3Frame, V3CodecError> {
    if bytes.len() < FRAME_HEADER_LEN {
        return Err(V3CodecError::Envelope("truncated header"));
    }
    let header = &bytes[..FRAME_HEADER_LEN];
    if &header[..4] != HANDSHAKE_MAGIC {
        return Err(V3CodecError::Envelope("invalid magic"));
    }
    if u16::from_le_bytes([header[4], header[5]]) as usize != FRAME_HEADER_LEN {
        return Err(V3CodecError::Envelope("invalid header length"));
    }
    if u16::from_le_bytes([header[6], header[7]]) != 0
        || header[13] != 0
        || u16::from_le_bytes([header[14], header[15]]) != 0
    {
        return Err(V3CodecError::Envelope("non-zero reserved field"));
    }
    if u32::from_le_bytes(header[8..12].try_into().unwrap()) != V3_ENVELOPE_VERSION {
        return Err(V3CodecError::Envelope("wrong version"));
    }
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    let decoded_len = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    if payload_len != decoded_len {
        return Err(V3CodecError::Envelope("compressed payload is unsupported"));
    }
    if payload_len > MAX_PAYLOAD {
        return Err(V3CodecError::Bound {
            field: "payload",
            value: payload_len,
        });
    }
    let total = FRAME_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(V3CodecError::Envelope("frame length overflow"))?;
    if bytes.len() < total {
        return Err(V3CodecError::Truncated);
    }
    if bytes.len() > total {
        return Err(V3CodecError::Trailing(bytes.len() - total));
    }
    Ok(V3Frame {
        message_id: u64::from_le_bytes(header[24..32].try_into().unwrap()),
        message: decode(header[12], &bytes[FRAME_HEADER_LEN..])?,
    })
}

/// Read one v3 frame from a persistent stream. `None` means clean EOF before
/// the next frame, which is the normal session shutdown path.
///
/// # Errors
///
/// Returns [`V3CodecError::Io`] if the stream fails, and
/// [`V3CodecError::Envelope`] if it ends part-way through a frame. Frame
/// validation errors are propagated from [`decode_frame`].
///
/// # Panics
///
/// Does not panic in practice: the fixed-width conversion reads from `header`
/// only after the full `FRAME_HEADER_LEN` bytes have been read into it.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<V3Frame>, V3CodecError> {
    let mut header = [0_u8; FRAME_HEADER_LEN];
    let first = reader
        .read(&mut header[..1])
        .map_err(|error| V3CodecError::Io(error.to_string()))?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut header[1..])
        .map_err(|error| V3CodecError::Io(error.to_string()))?;
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    if payload_len > MAX_PAYLOAD {
        return Err(V3CodecError::Bound {
            field: "payload",
            value: payload_len,
        });
    }
    let mut bytes = header.to_vec();
    let old_len = bytes.len();
    bytes.resize(old_len + payload_len, 0);
    reader
        .read_exact(&mut bytes[old_len..])
        .map_err(|error| V3CodecError::Io(error.to_string()))?;
    decode_frame(&bytes).map(Some)
}

/// Encode one v3 payload without an envelope.
///
/// # Errors
/// Returns [`V3CodecError`] when a field is malformed or exceeds its bound.
// One arm per protocol message, kept in a single function so the wire format
// can be read top to bottom against protocol.md, as in the v2 codec.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn encode(message: &V3Message) -> Result<Vec<u8>, V3CodecError> {
    let mut writer = Writer::default();
    match message {
        V3Message::Cancel { related_id } => writer.u64(*related_id),
        V3Message::Keepalive { nonce } | V3Message::KeepaliveAck { nonce } => writer.u64(*nonce),
        V3Message::Features { features } => writer.u64(*features),
        V3Message::FeaturesAck {
            related_id,
            features,
        } => {
            writer.u64(*related_id);
            writer.u64(*features);
        }
        V3Message::Mount {
            export,
            requested_access,
        } => {
            writer.blob(export, MAX_PATH, "export")?;
            writer.u8(*requested_access as u8);
        }
        V3Message::MountInfo {
            related_id,
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
            attr_cache_ms,
            dir_cache_ms,
        } => {
            validate_mount_info(*access, *effective_writable, reason, *max_read, *max_write)?;
            writer.u64(*related_id);
            writer.blob(export, MAX_PATH, "export")?;
            writer.u8(*access as u8);
            writer.bool(*effective_writable);
            writer.utf8_blob(reason, MAX_TEXT, "reason")?;
            writer.utf8_blob(options, MAX_TEXT, "options")?;
            writer.bool(*case_sensitive);
            writer.u8(*normalization as u8);
            writer.u32(*max_name_len);
            writer.u32(*max_path_len);
            writer.u64(*supports);
            writer.u32(*max_read);
            writer.u32(*max_write);
            writer.u32(*attr_cache_ms);
            writer.u32(*dir_cache_ms);
        }
        V3Message::Open {
            path,
            flags,
            mode,
            attr_mask,
        } => {
            validate_open(*flags, *mode)?;
            writer.blob(path, MAX_PATH, "path")?;
            writer.u32(*flags);
            writer.u32(*mode);
            writer.u32(*attr_mask);
        }
        V3Message::Opened {
            related_id,
            handle,
            attrs,
        } => {
            writer.u64(*related_id);
            writer.u64(*handle);
            encode_attrs(&mut writer, attrs)?;
        }
        V3Message::Close { handle } | V3Message::Flush { handle } => writer.u64(*handle),
        V3Message::Read {
            handle,
            offset,
            length,
            want_digest,
        } => {
            validate_read_length(*length)?;
            writer.u64(*handle);
            writer.u64(*offset);
            writer.u32(*length);
            writer.bool(*want_digest);
        }
        V3Message::ReadData {
            related_id,
            offset,
            eof,
            digest,
            data,
        } => {
            writer.u64(*related_id);
            writer.u64(*offset);
            writer.bool(*eof);
            writer.bool(digest.is_some());
            if let Some(digest) = digest {
                writer.bytes(digest);
            }
            writer.blob(data, MAX_DATA, "read data")?;
        }
        V3Message::Write {
            handle,
            offset,
            digest,
            data,
        } => {
            if data.is_empty() {
                return Err(V3CodecError::Bound {
                    field: "write data",
                    value: 0,
                });
            }
            writer.u64(*handle);
            writer.u64(*offset);
            writer.bool(digest.is_some());
            if let Some(digest) = digest {
                writer.bytes(digest);
            }
            writer.blob(data, MAX_DATA, "write data")?;
        }
        V3Message::WriteAck {
            related_id,
            bytes_written,
            new_size,
            stable,
            change_cookie,
        } => {
            writer.u64(*related_id);
            writer.u32(*bytes_written);
            writer.u64(*new_size);
            writer.bool(*stable);
            writer.bytes(change_cookie);
        }
        V3Message::Stat {
            target,
            follow,
            attr_mask,
        } => {
            encode_stat_target(&mut writer, target)?;
            writer.bool(*follow);
            writer.u32(*attr_mask);
        }
        V3Message::AttrsResponse { related_id, attrs } => {
            writer.u64(*related_id);
            encode_attrs(&mut writer, attrs)?;
        }
        V3Message::ReadDir {
            handle,
            cursor,
            max_entries,
            attr_mask,
        } => {
            validate_max_entries(*max_entries)?;
            writer.u64(*handle);
            writer.u64(*cursor);
            writer.u32(*max_entries);
            writer.u32(*attr_mask);
        }
        V3Message::DirPage {
            related_id,
            cursor,
            final_page,
            entries,
        } => {
            writer.u64(*related_id);
            writer.u64(*cursor);
            writer.bool(*final_page);
            writer.entries(entries)?;
        }
        V3Message::StatFs => {}
        V3Message::FsInfo {
            related_id,
            block_size,
            total_bytes,
            free_bytes,
            available_bytes,
            total_inodes,
            free_inodes,
            fs_type,
            max_name_len,
            case_sensitive,
            normalization,
            read_only,
        } => {
            writer.u64(*related_id);
            writer.u32(*block_size);
            writer.u64(*total_bytes);
            writer.u64(*free_bytes);
            writer.u64(*available_bytes);
            writer.u64(*total_inodes);
            writer.u64(*free_inodes);
            writer.utf8_blob(fs_type, MAX_NAME, "fs type")?;
            writer.u32(*max_name_len);
            writer.bool(*case_sensitive);
            writer.u8(*normalization as u8);
            writer.bool(*read_only);
        }
        V3Message::StageOpen {
            destination,
            size,
            digest,
            mode,
            resume_token,
        } => {
            writer.blob(destination, MAX_PATH, "destination")?;
            writer.u64(*size);
            writer.bool(digest.is_some());
            if let Some(digest) = digest {
                writer.bytes(digest);
            }
            writer.u32(*mode);
            writer.blob(resume_token, MAX_NAME, "resume token")?;
        }
        V3Message::StageOpened {
            related_id,
            stage_id,
            resume_token,
            staged_bytes,
        } => {
            writer.u64(*related_id);
            writer.u64(*stage_id);
            writer.blob(resume_token, MAX_NAME, "resume token")?;
            writer.u64(*staged_bytes);
        }
        V3Message::StageWrite {
            stage_id,
            offset,
            digest,
            data,
        } => {
            writer.u64(*stage_id);
            writer.u64(*offset);
            writer.bool(digest.is_some());
            if let Some(digest) = digest {
                writer.bytes(digest);
            }
            writer.blob(data, MAX_DATA, "stage data")?;
        }
        V3Message::StageAck {
            related_id,
            bytes_written,
            staged_bytes,
        } => {
            writer.u64(*related_id);
            writer.u32(*bytes_written);
            writer.u64(*staged_bytes);
        }
        V3Message::StageStatus { stage_id, cursor } => {
            writer.u64(*stage_id);
            writer.u64(*cursor);
        }
        V3Message::StageRanges {
            related_id,
            cursor,
            final_page,
            ranges,
        } => {
            if ranges.len() > MAX_COLLECTION {
                return Err(V3CodecError::Bound {
                    field: "stage ranges",
                    value: ranges.len(),
                });
            }
            writer.u64(*related_id);
            writer.u64(*cursor);
            writer.bool(*final_page);
            writer.u32(u32::try_from(ranges.len()).unwrap_or(u32::MAX));
            for (start, end) in ranges {
                writer.u64(*start);
                writer.u64(*end);
            }
        }
        V3Message::StageCommit {
            stage_id,
            digest,
            expect_cookie,
            mtime_ns,
        } => {
            writer.u64(*stage_id);
            writer.bytes(digest);
            writer.bool(expect_cookie.is_some());
            if let Some(cookie) = expect_cookie {
                writer.bytes(cookie);
            }
            writer.bool(mtime_ns.is_some());
            if let Some(mtime) = mtime_ns {
                writer.i64(*mtime);
            }
        }
        V3Message::StageResult {
            related_id,
            outcome,
            attrs,
        } => {
            writer.u64(*related_id);
            writer.u8(*outcome as u8);
            encode_attrs(&mut writer, attrs)?;
        }
        V3Message::StageAbort { stage_id } => writer.u64(*stage_id),
        V3Message::WriteCas {
            handle,
            offset,
            expect_cookie,
            digest,
            data,
        } => {
            writer.u64(*handle);
            writer.u64(*offset);
            writer.bytes(expect_cookie);
            writer.bool(digest.is_some());
            if let Some(digest) = digest {
                writer.bytes(digest);
            }
            writer.blob(data, MAX_DATA, "write data")?;
        }
        V3Message::Rename {
            source,
            destination,
            mode,
            attr_mask,
        } => {
            writer.blob(source, MAX_PATH, "source")?;
            writer.blob(destination, MAX_PATH, "destination")?;
            writer.u8(*mode as u8);
            writer.u32(*attr_mask);
        }
        V3Message::Unlink { path } | V3Message::Rmdir { path } => {
            writer.blob(path, MAX_PATH, "path")?;
        }
        V3Message::Mkdir {
            path,
            mode,
            attr_mask,
        } => {
            writer.blob(path, MAX_PATH, "path")?;
            writer.u32(*mode);
            writer.u32(*attr_mask);
        }
        V3Message::Symlink {
            target,
            path,
            attr_mask,
        } => {
            // The target is not a path in the export: it is opaque text that
            // may point anywhere, including outside. It is bounded like a path
            // and confined nowhere, because storing it is not following it.
            writer.blob(target, MAX_PATH, "target")?;
            writer.blob(path, MAX_PATH, "path")?;
            writer.u32(*attr_mask);
        }
        V3Message::Link {
            existing,
            path,
            attr_mask,
        } => {
            writer.blob(existing, MAX_PATH, "existing")?;
            writer.blob(path, MAX_PATH, "path")?;
            writer.u32(*attr_mask);
        }
        V3Message::Chown {
            target,
            uid,
            gid,
            follow,
            attr_mask,
        } => {
            encode_stat_target(&mut writer, target)?;
            writer.bool(uid.is_some());
            writer.u32(uid.unwrap_or(0));
            writer.bool(gid.is_some());
            writer.u32(gid.unwrap_or(0));
            writer.bool(*follow);
            writer.u32(*attr_mask);
        }
        V3Message::SetTimes {
            target,
            atime,
            mtime,
            follow,
            attr_mask,
        } => {
            encode_stat_target(&mut writer, target)?;
            atime.encode(&mut writer);
            mtime.encode(&mut writer);
            writer.bool(*follow);
            writer.u32(*attr_mask);
        }
        V3Message::SetPermissions {
            target,
            mode,
            follow,
            attr_mask,
        } => {
            encode_stat_target(&mut writer, target)?;
            writer.u32(*mode);
            writer.bool(*follow);
            writer.u32(*attr_mask);
        }
        V3Message::Mutated { related_id, attrs } => {
            writer.u64(*related_id);
            encode_attrs(&mut writer, attrs)?;
        }
        V3Message::Error {
            related_id,
            code,
            platform_errno,
            message,
        } => {
            writer.u64(*related_id);
            writer.u16(*code as u16);
            writer.i32(*platform_errno);
            writer.utf8_blob(message, MAX_TEXT, "error message")?;
        }
        V3Message::Done { related_id } => writer.u64(*related_id),
    }
    Ok(writer.bytes)
}

/// Decode one v3 payload of the given type without an envelope.
///
/// # Errors
/// Returns [`V3CodecError`] for an unknown type, any bound violation, an
/// inconsistent field pair, or trailing bytes.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn decode(message_type: u8, payload: &[u8]) -> Result<V3Message, V3CodecError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(V3CodecError::Bound {
            field: "payload",
            value: payload.len(),
        });
    }
    let mut reader = Reader {
        bytes: payload,
        offset: 0,
    };
    let message = match message_type {
        types::CANCEL => V3Message::Cancel {
            related_id: reader.u64()?,
        },
        types::KEEPALIVE => V3Message::Keepalive {
            nonce: reader.u64()?,
        },
        types::KEEPALIVE_ACK => V3Message::KeepaliveAck {
            nonce: reader.u64()?,
        },
        types::FEATURES => V3Message::Features {
            features: reader.u64()?,
        },
        types::FEATURES_ACK => V3Message::FeaturesAck {
            related_id: reader.u64()?,
            features: reader.u64()?,
        },
        types::MOUNT => V3Message::Mount {
            export: reader.blob(MAX_PATH, "export")?,
            requested_access: Access::decode(reader.u8()?)?,
        },
        types::MOUNT_INFO => {
            let related_id = reader.u64()?;
            let export = reader.blob(MAX_PATH, "export")?;
            let access = Access::decode(reader.u8()?)?;
            let effective_writable = reader.bool()?;
            let reason = reader.utf8_blob(MAX_TEXT, "reason")?;
            let options = reader.utf8_blob(MAX_TEXT, "options")?;
            let case_sensitive = reader.bool()?;
            let normalization = Normalization::decode(reader.u8()?)?;
            let max_name_len = reader.u32()?;
            let max_path_len = reader.u32()?;
            let supports = reader.u64()?;
            let max_read = reader.u32()?;
            let max_write = reader.u32()?;
            let attr_cache_ms = reader.u32()?;
            let dir_cache_ms = reader.u32()?;
            validate_mount_info(access, effective_writable, &reason, max_read, max_write)?;
            V3Message::MountInfo {
                related_id,
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
                attr_cache_ms,
                dir_cache_ms,
            }
        }
        types::OPEN => {
            let path = reader.blob(MAX_PATH, "path")?;
            let flags = reader.u32()?;
            let mode = reader.u32()?;
            validate_open(flags, mode)?;
            V3Message::Open {
                path,
                flags,
                mode,
                attr_mask: reader.u32()?,
            }
        }
        types::OPENED => V3Message::Opened {
            related_id: reader.u64()?,
            handle: reader.u64()?,
            attrs: decode_attrs(&mut reader)?,
        },
        types::CLOSE => V3Message::Close {
            handle: reader.u64()?,
        },
        types::READ => {
            let handle = reader.u64()?;
            let offset = reader.u64()?;
            let length = reader.u32()?;
            validate_read_length(length)?;
            let want_digest = reader.bool()?;
            V3Message::Read {
                handle,
                offset,
                length,
                want_digest,
            }
        }
        types::READ_DATA => {
            let related_id = reader.u64()?;
            let offset = reader.u64()?;
            let eof = reader.bool()?;
            let digest = reader.bool()?.then(|| reader.array32()).transpose()?;
            let data = reader.blob(MAX_DATA, "read data")?;
            V3Message::ReadData {
                related_id,
                offset,
                eof,
                digest,
                data,
            }
        }
        types::WRITE => {
            let handle = reader.u64()?;
            let offset = reader.u64()?;
            let digest = reader.bool()?.then(|| reader.array32()).transpose()?;
            let data = reader.blob(MAX_DATA, "write data")?;
            if data.is_empty() {
                return Err(V3CodecError::Bound {
                    field: "write data",
                    value: 0,
                });
            }
            V3Message::Write {
                handle,
                offset,
                digest,
                data,
            }
        }
        types::WRITE_ACK => V3Message::WriteAck {
            related_id: reader.u64()?,
            bytes_written: reader.u32()?,
            new_size: reader.u64()?,
            stable: reader.bool()?,
            change_cookie: reader.array16()?,
        },
        types::FLUSH => V3Message::Flush {
            handle: reader.u64()?,
        },
        types::STAT => V3Message::Stat {
            target: decode_stat_target(&mut reader)?,
            follow: reader.bool()?,
            attr_mask: reader.u32()?,
        },
        types::ATTRS => V3Message::AttrsResponse {
            related_id: reader.u64()?,
            attrs: decode_attrs(&mut reader)?,
        },
        types::READ_DIR => {
            let handle = reader.u64()?;
            let cursor = reader.u64()?;
            let max_entries = reader.u32()?;
            validate_max_entries(max_entries)?;
            let attr_mask = reader.u32()?;
            V3Message::ReadDir {
                handle,
                cursor,
                max_entries,
                attr_mask,
            }
        }
        types::DIR_PAGE => V3Message::DirPage {
            related_id: reader.u64()?,
            cursor: reader.u64()?,
            final_page: reader.bool()?,
            entries: reader.entries()?,
        },
        types::STAT_FS => V3Message::StatFs,
        types::FS_INFO => V3Message::FsInfo {
            related_id: reader.u64()?,
            block_size: reader.u32()?,
            total_bytes: reader.u64()?,
            free_bytes: reader.u64()?,
            available_bytes: reader.u64()?,
            total_inodes: reader.u64()?,
            free_inodes: reader.u64()?,
            fs_type: reader.utf8_blob(MAX_NAME, "fs type")?,
            max_name_len: reader.u32()?,
            case_sensitive: reader.bool()?,
            normalization: Normalization::decode(reader.u8()?)?,
            read_only: reader.bool()?,
        },
        types::STAGE_OPEN => {
            let destination = reader.blob(MAX_PATH, "destination")?;
            let size = reader.u64()?;
            let digest = reader.bool()?.then(|| reader.array32()).transpose()?;
            V3Message::StageOpen {
                destination,
                size,
                digest,
                mode: reader.u32()?,
                resume_token: reader.blob(MAX_NAME, "resume token")?,
            }
        }
        types::STAGE_OPENED => V3Message::StageOpened {
            related_id: reader.u64()?,
            stage_id: reader.u64()?,
            resume_token: reader.blob(MAX_NAME, "resume token")?,
            staged_bytes: reader.u64()?,
        },
        types::STAGE_WRITE => {
            let stage_id = reader.u64()?;
            let offset = reader.u64()?;
            let digest = reader.bool()?.then(|| reader.array32()).transpose()?;
            V3Message::StageWrite {
                stage_id,
                offset,
                digest,
                data: reader.blob(MAX_DATA, "stage data")?,
            }
        }
        types::STAGE_ACK => V3Message::StageAck {
            related_id: reader.u64()?,
            bytes_written: reader.u32()?,
            staged_bytes: reader.u64()?,
        },
        types::STAGE_STATUS => V3Message::StageStatus {
            stage_id: reader.u64()?,
            cursor: reader.u64()?,
        },
        types::STAGE_RANGES => {
            let related_id = reader.u64()?;
            let cursor = reader.u64()?;
            let final_page = reader.bool()?;
            let count = reader.u32()? as usize;
            if count > MAX_COLLECTION {
                return Err(V3CodecError::Bound {
                    field: "stage ranges",
                    value: count,
                });
            }
            let mut ranges = Vec::with_capacity(count);
            let mut previous_end = 0_u64;
            for _ in 0..count {
                let start = reader.u64()?;
                let end = reader.u64()?;
                // Ascending, disjoint and non-empty. A range set that overlaps
                // or runs backwards describes no file, and accepting one would
                // let a peer make a resuming client skip bytes it never sent.
                if end <= start || start < previous_end {
                    return Err(V3CodecError::Inconsistent(
                        "stage ranges must be ascending, disjoint and non-empty",
                    ));
                }
                previous_end = end;
                ranges.push((start, end));
            }
            V3Message::StageRanges {
                related_id,
                cursor,
                final_page,
                ranges,
            }
        }
        types::STAGE_COMMIT => {
            let stage_id = reader.u64()?;
            let digest = reader.array32()?;
            let expect_cookie = reader.bool()?.then(|| reader.array16()).transpose()?;
            let mtime_ns = reader.bool()?.then(|| reader.i64()).transpose()?;
            V3Message::StageCommit {
                stage_id,
                digest,
                expect_cookie,
                mtime_ns,
            }
        }
        types::STAGE_RESULT => V3Message::StageResult {
            related_id: reader.u64()?,
            outcome: CommitOutcome::decode(reader.u8()?)?,
            attrs: decode_attrs(&mut reader)?,
        },
        types::STAGE_ABORT => V3Message::StageAbort {
            stage_id: reader.u64()?,
        },
        types::WRITE_CAS => {
            let handle = reader.u64()?;
            let offset = reader.u64()?;
            let expect_cookie = reader.array16()?;
            let digest = reader.bool()?.then(|| reader.array32()).transpose()?;
            V3Message::WriteCas {
                handle,
                offset,
                expect_cookie,
                digest,
                data: reader.blob(MAX_DATA, "write data")?,
            }
        }
        types::RENAME => V3Message::Rename {
            source: reader.blob(MAX_PATH, "source")?,
            destination: reader.blob(MAX_PATH, "destination")?,
            mode: RenameMode::decode(reader.u8()?)?,
            attr_mask: reader.u32()?,
        },
        types::UNLINK => V3Message::Unlink {
            path: reader.blob(MAX_PATH, "path")?,
        },
        types::RMDIR => V3Message::Rmdir {
            path: reader.blob(MAX_PATH, "path")?,
        },
        types::MKDIR => V3Message::Mkdir {
            path: reader.blob(MAX_PATH, "path")?,
            mode: reader.u32()?,
            attr_mask: reader.u32()?,
        },
        types::SYMLINK => V3Message::Symlink {
            target: reader.blob(MAX_PATH, "target")?,
            path: reader.blob(MAX_PATH, "path")?,
            attr_mask: reader.u32()?,
        },
        types::LINK => V3Message::Link {
            existing: reader.blob(MAX_PATH, "existing")?,
            path: reader.blob(MAX_PATH, "path")?,
            attr_mask: reader.u32()?,
        },
        types::CHOWN => {
            let target = decode_stat_target(&mut reader)?;
            let owner_present = reader.bool()?;
            let owner = reader.u32()?;
            let group_present = reader.bool()?;
            let group = reader.u32()?;
            V3Message::Chown {
                target,
                uid: owner_present.then_some(owner),
                gid: group_present.then_some(group),
                follow: reader.bool()?,
                attr_mask: reader.u32()?,
            }
        }
        types::SET_TIMES => {
            let target = decode_stat_target(&mut reader)?;
            V3Message::SetTimes {
                target,
                atime: TimeChange::decode(&mut reader)?,
                mtime: TimeChange::decode(&mut reader)?,
                follow: reader.bool()?,
                attr_mask: reader.u32()?,
            }
        }
        types::SET_PERMISSIONS => {
            let target = decode_stat_target(&mut reader)?;
            V3Message::SetPermissions {
                target,
                mode: reader.u32()?,
                follow: reader.bool()?,
                attr_mask: reader.u32()?,
            }
        }
        types::MUTATED => V3Message::Mutated {
            related_id: reader.u64()?,
            attrs: decode_attrs(&mut reader)?,
        },
        types::ERROR => V3Message::Error {
            related_id: reader.u64()?,
            code: ErrorCode::decode(reader.u16()?)?,
            platform_errno: reader.i32()?,
            message: reader.utf8_blob(MAX_TEXT, "error message")?,
        },
        types::DONE => V3Message::Done {
            related_id: reader.u64()?,
        },
        value => {
            return Err(V3CodecError::InvalidEnum {
                field: "message type",
                value,
            })
        }
    };
    if reader.offset != payload.len() {
        return Err(V3CodecError::Trailing(payload.len() - reader.offset));
    }
    Ok(message)
}

fn validate_open(flags: u32, mode: u32) -> Result<(), V3CodecError> {
    open_flags::validate(flags)?;
    if mode > MAX_MODE {
        return Err(V3CodecError::Bound {
            field: "open mode",
            value: mode as usize,
        });
    }
    if mode != 0 && flags & open_flags::CREATE == 0 {
        return Err(V3CodecError::Inconsistent("open mode without CREATE"));
    }
    Ok(())
}

fn validate_read_length(length: u32) -> Result<(), V3CodecError> {
    if length == 0 || length as usize > MAX_DATA {
        return Err(V3CodecError::Bound {
            field: "read length",
            value: length as usize,
        });
    }
    Ok(())
}

fn validate_max_entries(max_entries: u32) -> Result<(), V3CodecError> {
    if max_entries == 0 || max_entries as usize > MAX_COLLECTION {
        return Err(V3CodecError::Bound {
            field: "max entries",
            value: max_entries as usize,
        });
    }
    Ok(())
}

fn validate_mount_info(
    access: Access,
    effective_writable: bool,
    reason: &[u8],
    max_read: u32,
    max_write: u32,
) -> Result<(), V3CodecError> {
    if effective_writable && !reason.is_empty() {
        return Err(V3CodecError::Inconsistent(
            "mount info reason must be empty when writable",
        ));
    }
    if !effective_writable && reason.is_empty() {
        return Err(V3CodecError::Inconsistent(
            "mount info reason is required when not writable",
        ));
    }
    if access == Access::ReadOnly && effective_writable {
        return Err(V3CodecError::Inconsistent(
            "mount info cannot be writable on a read-only export",
        ));
    }
    for (field, value) in [("max read", max_read), ("max write", max_write)] {
        if value == 0 || value as usize > MAX_DATA {
            return Err(V3CodecError::Bound {
                field,
                value: value as usize,
            });
        }
    }
    Ok(())
}

/// A `StatTarget` on the wire: a tag, then both fields, always.
///
/// Fixed width whichever arm it is, so the frame layout never depends on a
/// value the peer chose. Shared by `Stat` and by the mutations that can name
/// either a path or an open handle, which is the only reason it is a function.
fn encode_stat_target(writer: &mut Writer, target: &StatTarget) -> Result<(), V3CodecError> {
    match target {
        StatTarget::Path(path) => {
            writer.u8(0);
            writer.blob(path, MAX_PATH, "path")?;
            writer.u64(0);
        }
        StatTarget::Handle(handle) => {
            writer.u8(1);
            writer.blob(&[], MAX_PATH, "path")?;
            writer.u64(*handle);
        }
    }
    Ok(())
}

fn decode_stat_target(reader: &mut Reader<'_>) -> Result<StatTarget, V3CodecError> {
    let tag = reader.u8()?;
    let path = reader.blob(MAX_PATH, "path")?;
    let handle = reader.u64()?;
    // The unused half of the pair must be zero. A sender that filled both said
    // two things, and guessing which one it meant is how a mutation lands on
    // the wrong file.
    match tag {
        0 if handle == 0 => Ok(StatTarget::Path(path)),
        0 => Err(V3CodecError::Inconsistent(
            "stat target handle must be zero for a path target",
        )),
        1 if path.is_empty() => Ok(StatTarget::Handle(handle)),
        1 => Err(V3CodecError::Inconsistent(
            "stat target path must be empty for a handle target",
        )),
        value => Err(V3CodecError::InvalidEnum {
            field: "stat target",
            value,
        }),
    }
}

fn encode_attrs(writer: &mut Writer, attrs: &Attrs) -> Result<(), V3CodecError> {
    attrs.validate()?;
    writer.u32(attrs.presence());
    writer.u8(attrs.kind);
    writer.u32(attrs.mode);
    writer.u64(attrs.size);
    writer.i64(attrs.mtime_ns);
    writer.bytes(&attrs.change_cookie);
    if let Some((uid, gid)) = attrs.owner {
        writer.u32(uid);
        writer.u32(gid);
    }
    if let Some(nlink) = attrs.nlink {
        writer.u32(nlink);
    }
    if let Some(atime_ns) = attrs.atime_ns {
        writer.i64(atime_ns);
    }
    if let Some(ctime_ns) = attrs.ctime_ns {
        writer.i64(ctime_ns);
    }
    if let Some(btime_ns) = attrs.btime_ns {
        writer.i64(btime_ns);
    }
    if let Some((dev, ino)) = attrs.identity {
        writer.u64(dev);
        writer.u64(ino);
    }
    if let Some(rdev) = attrs.rdev {
        writer.u64(rdev);
    }
    if let Some(target) = &attrs.symlink_target {
        writer.blob(target, MAX_PATH, "symlink target")?;
    }
    if let Some(allocated_size) = attrs.allocated_size {
        writer.u64(allocated_size);
    }
    if let Some((owner_name, group_name)) = &attrs.names {
        writer.utf8_blob(owner_name, MAX_NAME, "owner name")?;
        writer.utf8_blob(group_name, MAX_NAME, "group name")?;
    }
    if let Some(flags) = attrs.flags {
        writer.u32(flags);
    }
    Ok(())
}

fn decode_attrs(reader: &mut Reader<'_>) -> Result<Attrs, V3CodecError> {
    let presence = reader.u32()?;
    if presence & !attr_presence::KNOWN != 0 {
        return Err(V3CodecError::UnknownFlags {
            field: "attrs presence",
            value: u64::from(presence),
        });
    }
    let has = |bit: u32| presence & bit != 0;
    let mut attrs = Attrs::minimal(
        reader.u8()?,
        reader.u32()?,
        reader.u64()?,
        reader.i64()?,
        reader.array16()?,
    );
    if has(attr_presence::OWNER) {
        attrs.owner = Some((reader.u32()?, reader.u32()?));
    }
    if has(attr_presence::NLINK) {
        attrs.nlink = Some(reader.u32()?);
    }
    if has(attr_presence::ATIME) {
        attrs.atime_ns = Some(reader.i64()?);
    }
    if has(attr_presence::CTIME) {
        attrs.ctime_ns = Some(reader.i64()?);
    }
    if has(attr_presence::BTIME) {
        attrs.btime_ns = Some(reader.i64()?);
    }
    if has(attr_presence::IDENTITY) {
        attrs.identity = Some((reader.u64()?, reader.u64()?));
    }
    if has(attr_presence::RDEV) {
        attrs.rdev = Some(reader.u64()?);
    }
    if has(attr_presence::SYMLINK_TARGET) {
        attrs.symlink_target = Some(reader.blob(MAX_PATH, "symlink target")?);
    }
    if has(attr_presence::ALLOCATED_SIZE) {
        attrs.allocated_size = Some(reader.u64()?);
    }
    if has(attr_presence::NAMES) {
        attrs.names = Some((
            reader.utf8_blob(MAX_NAME, "owner name")?,
            reader.utf8_blob(MAX_NAME, "group name")?,
        ));
    }
    if has(attr_presence::FLAGS) {
        attrs.flags = Some(reader.u32()?);
    }
    attrs.validate()?;
    Ok(attrs)
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
    ) -> Result<(), V3CodecError> {
        if value.len() > maximum {
            return Err(V3CodecError::Bound {
                field,
                value: value.len(),
            });
        }
        self.u32(u32::try_from(value.len()).map_err(|_| V3CodecError::Bound {
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
    ) -> Result<(), V3CodecError> {
        if std::str::from_utf8(value).is_err() {
            return Err(V3CodecError::InvalidUtf8 { field });
        }
        self.blob(value, maximum, field)
    }
    fn entries(&mut self, entries: &[DirEntry]) -> Result<(), V3CodecError> {
        if entries.len() > MAX_COLLECTION {
            return Err(V3CodecError::Bound {
                field: "entry count",
                value: entries.len(),
            });
        }
        self.u32(
            u32::try_from(entries.len()).map_err(|_| V3CodecError::Bound {
                field: "entry count",
                value: entries.len(),
            })?,
        );
        for entry in entries {
            self.blob(&entry.name, MAX_PATH, "entry name")?;
            encode_attrs(self, &entry.attrs)?;
        }
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], V3CodecError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(V3CodecError::Truncated)?;
        if end > self.bytes.len() {
            return Err(V3CodecError::Truncated);
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, V3CodecError> {
        Ok(*self.take(1)?.first().ok_or(V3CodecError::Truncated)?)
    }
    fn u16(&mut self) -> Result<u16, V3CodecError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| V3CodecError::Truncated)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, V3CodecError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| V3CodecError::Truncated)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, V3CodecError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| V3CodecError::Truncated)?,
        ))
    }
    fn i64(&mut self) -> Result<i64, V3CodecError> {
        Ok(i64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| V3CodecError::Truncated)?,
        ))
    }
    fn i32(&mut self) -> Result<i32, V3CodecError> {
        Ok(i32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| V3CodecError::Truncated)?,
        ))
    }
    fn bool(&mut self) -> Result<bool, V3CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(V3CodecError::InvalidEnum {
                field: "boolean",
                value,
            }),
        }
    }
    fn array16(&mut self) -> Result<[u8; 16], V3CodecError> {
        self.take(16)?
            .try_into()
            .map_err(|_| V3CodecError::Truncated)
    }
    fn array32(&mut self) -> Result<[u8; 32], V3CodecError> {
        self.take(32)?
            .try_into()
            .map_err(|_| V3CodecError::Truncated)
    }
    fn blob(&mut self, maximum: usize, field: &'static str) -> Result<Vec<u8>, V3CodecError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(V3CodecError::Bound {
                field,
                value: length,
            });
        }
        Ok(self.take(length)?.to_vec())
    }
    fn utf8_blob(&mut self, maximum: usize, field: &'static str) -> Result<Vec<u8>, V3CodecError> {
        let value = self.blob(maximum, field)?;
        if std::str::from_utf8(&value).is_err() {
            return Err(V3CodecError::InvalidUtf8 { field });
        }
        Ok(value)
    }
    fn entries(&mut self) -> Result<Vec<DirEntry>, V3CodecError> {
        let count = self.u32()? as usize;
        if count > MAX_COLLECTION {
            return Err(V3CodecError::Bound {
                field: "entry count",
                value: count,
            });
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(DirEntry {
                name: self.blob(MAX_PATH, "entry name")?,
                attrs: decode_attrs(self)?,
            });
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_attrs() -> Attrs {
        Attrs {
            kind: 3,
            mode: 0o777,
            size: 1,
            mtime_ns: -1,
            change_cookie: [0x33; 16],
            owner: Some((1000, 1000)),
            nlink: Some(1),
            atime_ns: Some(2),
            ctime_ns: Some(3),
            btime_ns: Some(4),
            identity: Some((5, 6)),
            rdev: Some(0),
            symlink_target: Some(b"target".to_vec()),
            allocated_size: Some(4096),
            names: Some((b"dorian".to_vec(), b"staff".to_vec())),
            flags: Some(attr_flags::HIDDEN),
        }
    }

    // One sample of every Phase 1 message; a data table, not logic.
    #[allow(clippy::too_many_lines)]
    fn every_message() -> Vec<V3Message> {
        vec![
            V3Message::Cancel { related_id: 7 },
            V3Message::Keepalive { nonce: 42 },
            V3Message::KeepaliveAck { nonce: 42 },
            V3Message::Features {
                features: features::LOCKS | features::NOTIFY,
            },
            V3Message::FeaturesAck {
                related_id: 1,
                features: features::LOCKS,
            },
            V3Message::Mount {
                export: b"media".to_vec(),
                requested_access: Access::ReadWrite,
            },
            V3Message::MountInfo {
                related_id: 2,
                export: b"media".to_vec(),
                access: Access::ReadOnly,
                effective_writable: false,
                reason: b"export is ro".to_vec(),
                options: b"ro,root_squash".to_vec(),
                case_sensitive: true,
                normalization: Normalization::None,
                max_name_len: 255,
                max_path_len: 4096,
                supports: supports::SYMLINKS,
                max_read: 1 << 20,
                max_write: 1 << 20,
                attr_cache_ms: 0,
                dir_cache_ms: 0,
            },
            V3Message::Open {
                path: b"a".to_vec(),
                flags: open_flags::WRITE | open_flags::CREATE | open_flags::EXCL,
                mode: 0o644,
                attr_mask: attr_presence::IDENTITY,
            },
            V3Message::Opened {
                related_id: 3,
                handle: 10,
                attrs: Attrs::minimal(1, 0o644, 4, -2, [0x11; 16]),
            },
            V3Message::Close { handle: 10 },
            V3Message::Read {
                handle: 10,
                offset: 4096,
                length: 65_536,
                want_digest: true,
            },
            V3Message::ReadData {
                related_id: 4,
                offset: 4096,
                eof: true,
                digest: Some([0xab; 32]),
                data: b"hi".to_vec(),
            },
            V3Message::Write {
                handle: 10,
                offset: 0,
                digest: None,
                data: b"hi".to_vec(),
            },
            V3Message::WriteAck {
                related_id: 5,
                bytes_written: 2,
                new_size: 2,
                stable: false,
                change_cookie: [0x22; 16],
            },
            V3Message::Flush { handle: 10 },
            V3Message::Stat {
                target: StatTarget::Path(b"a".to_vec()),
                follow: true,
                attr_mask: attr_presence::MASK_FIXED_PART_ONLY,
            },
            V3Message::Stat {
                target: StatTarget::Handle(10),
                follow: false,
                attr_mask: attr_presence::OWNER | attr_presence::NAMES,
            },
            V3Message::AttrsResponse {
                related_id: 6,
                attrs: full_attrs(),
            },
            V3Message::ReadDir {
                handle: 11,
                cursor: 0,
                max_entries: 256,
                attr_mask: attr_presence::KNOWN,
            },
            V3Message::DirPage {
                related_id: 7,
                cursor: 0,
                final_page: true,
                entries: vec![DirEntry {
                    name: vec![0xff, b'x'],
                    attrs: Attrs::minimal(2, 0o755, 0, 0, [0x44; 16]),
                }],
            },
            V3Message::StatFs,
            V3Message::FsInfo {
                related_id: 8,
                block_size: 4096,
                total_bytes: 1000,
                free_bytes: 500,
                available_bytes: 400,
                total_inodes: 10,
                free_inodes: 5,
                fs_type: b"apfs".to_vec(),
                max_name_len: 255,
                case_sensitive: false,
                normalization: Normalization::Nfc,
                read_only: false,
            },
            V3Message::StageOpen {
                destination: b"big.mkv".to_vec(),
                size: 1 << 30,
                digest: Some([0x5a; 32]),
                mode: 0o644,
                resume_token: Vec::new(),
            },
            V3Message::StageOpened {
                related_id: 20,
                stage_id: 3,
                resume_token: b"tok".to_vec(),
                staged_bytes: 4096,
            },
            V3Message::StageWrite {
                stage_id: 3,
                offset: 4096,
                digest: None,
                data: b"chunk".to_vec(),
            },
            V3Message::StageAck {
                related_id: 21,
                bytes_written: 5,
                staged_bytes: 4101,
            },
            V3Message::StageStatus {
                stage_id: 3,
                cursor: 0,
            },
            V3Message::StageRanges {
                related_id: 22,
                cursor: 0,
                final_page: true,
                ranges: vec![(0, 4096), (8192, 12288)],
            },
            V3Message::StageCommit {
                stage_id: 3,
                digest: [0x5a; 32],
                expect_cookie: Some([0x11; 16]),
                mtime_ns: Some(-1),
            },
            V3Message::StageResult {
                related_id: 23,
                outcome: CommitOutcome::Changed,
                attrs: full_attrs(),
            },
            V3Message::StageAbort { stage_id: 3 },
            V3Message::WriteCas {
                handle: 10,
                offset: 0,
                expect_cookie: [0x22; 16],
                digest: Some([0x33; 32]),
                data: b"edit".to_vec(),
            },
            V3Message::Rename {
                source: b"a.txt".to_vec(),
                destination: b"b.txt".to_vec(),
                mode: RenameMode::Exchange,
                attr_mask: attr_presence::OWNER,
            },
            V3Message::Unlink {
                path: b"a.txt".to_vec(),
            },
            V3Message::Rmdir {
                path: b"empty".to_vec(),
            },
            V3Message::Mkdir {
                path: b"new".to_vec(),
                mode: 0o755,
                attr_mask: 0,
            },
            V3Message::Symlink {
                target: b"../outside".to_vec(),
                path: b"link".to_vec(),
                attr_mask: attr_presence::SYMLINK_TARGET,
            },
            V3Message::Link {
                existing: b"a.txt".to_vec(),
                path: b"b.txt".to_vec(),
                attr_mask: 0,
            },
            V3Message::Chown {
                target: StatTarget::Path(b"a.txt".to_vec()),
                uid: Some(1000),
                gid: None,
                follow: false,
                attr_mask: attr_presence::OWNER,
            },
            V3Message::SetTimes {
                target: StatTarget::Handle(4),
                atime: TimeChange::Omit,
                mtime: TimeChange::Set {
                    seconds: -1,
                    nanos: 999_999_999,
                },
                follow: true,
                attr_mask: 0,
            },
            V3Message::SetPermissions {
                target: StatTarget::Path(b"a.txt".to_vec()),
                mode: 0o640,
                follow: true,
                attr_mask: 0,
            },
            V3Message::Mutated {
                related_id: 11,
                attrs: full_attrs(),
            },
            V3Message::Error {
                related_id: 9,
                code: ErrorCode::ReadOnly,
                platform_errno: 30,
                message: b"read-only".to_vec(),
            },
            V3Message::Done { related_id: 9 },
        ]
    }

    #[test]
    fn every_mutation_type_is_in_the_round_trip_table() {
        // The table is hand-written, so a message added to the enum but not to
        // it would be encoded by nothing and decoded by no test.
        let covered: std::collections::BTreeSet<u8> =
            every_message().iter().map(message_type).collect();
        for (name, value) in [
            ("Rename", types::RENAME),
            ("Unlink", types::UNLINK),
            ("Rmdir", types::RMDIR),
            ("Mkdir", types::MKDIR),
            ("Symlink", types::SYMLINK),
            ("Link", types::LINK),
            ("Chown", types::CHOWN),
            ("SetTimes", types::SET_TIMES),
            ("SetPermissions", types::SET_PERMISSIONS),
            ("Mutated", types::MUTATED),
        ] {
            assert!(covered.contains(&value), "{name} is not round-tripped");
        }
    }

    #[test]
    fn stage_ranges_must_be_ascending_and_disjoint() {
        // A range set that overlaps or runs backwards describes no file, and
        // accepting one would let a peer make a resuming client skip bytes it
        // never sent.
        for ranges in [
            vec![(0_u64, 10_u64), (5, 20)], // overlapping
            vec![(10_u64, 20_u64), (0, 5)], // descending
            vec![(10_u64, 10_u64)],         // empty
            vec![(20_u64, 10_u64)],         // inverted
        ] {
            let payload = encode(&V3Message::StageRanges {
                related_id: 1,
                cursor: 0,
                final_page: true,
                ranges: ranges.clone(),
            })
            .unwrap();
            assert!(
                decode(types::STAGE_RANGES, &payload).is_err(),
                "accepted {ranges:?}"
            );
        }
    }

    #[test]
    fn a_commit_outcome_cannot_be_defaulted_into_success() {
        // The refusal is carried in the reply, not as an Error, so the client
        // gets the destination's current attributes. An unknown value must be
        // refused rather than read as `Committed`.
        let mut payload = encode(&V3Message::StageResult {
            related_id: 1,
            outcome: CommitOutcome::Committed,
            attrs: full_attrs(),
        })
        .unwrap();
        payload[8] = 7;
        assert!(decode(types::STAGE_RESULT, &payload).is_err());
    }

    #[test]
    fn an_unknown_rename_mode_is_refused_rather_than_defaulted() {
        // Defaulting would turn a mode this server does not implement into a
        // rename it does — quietly replacing a file the client asked never to
        // replace.
        let mut payload = encode(&V3Message::Rename {
            source: b"a".to_vec(),
            destination: b"b".to_vec(),
            mode: RenameMode::Replace,
            attr_mask: 0,
        })
        .unwrap();
        let mode_at = payload.len() - 5;
        payload[mode_at] = 9;
        assert!(decode(types::RENAME, &payload).is_err());
    }

    #[test]
    fn a_time_change_keeps_its_width_whatever_the_tag() {
        // The frame layout must not depend on a value the peer chose.
        let widths: std::collections::BTreeSet<usize> = [
            TimeChange::Omit,
            TimeChange::Now,
            TimeChange::Set {
                seconds: 1,
                nanos: 2,
            },
        ]
        .into_iter()
        .map(|change| {
            let mut writer = Writer::default();
            change.encode(&mut writer);
            writer.bytes.len()
        })
        .collect();
        assert_eq!(
            widths.len(),
            1,
            "tags encode to different widths: {widths:?}"
        );
    }

    #[test]
    fn an_out_of_range_nanosecond_is_refused() {
        let mut writer = Writer::default();
        writer.u8(2); // SET
        writer.i64(0);
        writer.u32(1_000_000_000);
        let mut reader = Reader {
            bytes: &writer.bytes,
            offset: 0,
        };
        assert!(TimeChange::decode(&mut reader).is_err());
    }

    #[test]
    fn a_stat_target_may_not_fill_both_halves() {
        // Shared by Stat and by every mutation that names a path or a handle,
        // so this one check covers all of them.
        let mut writer = Writer::default();
        writer.u8(0); // path target...
        writer.blob(b"a.txt", MAX_PATH, "path").unwrap();
        writer.u64(7); // ...but a handle as well.
        let mut reader = Reader {
            bytes: &writer.bytes,
            offset: 0,
        };
        assert!(decode_stat_target(&mut reader).is_err());
    }

    #[test]
    fn every_message_round_trips_through_payload_and_frame() {
        for message in every_message() {
            let payload = encode(&message).unwrap();
            assert_eq!(
                decode(message_type(&message), &payload).unwrap(),
                message,
                "{message:?}"
            );
            let frame = encode_frame(99, &message).unwrap();
            assert_eq!(
                decode_frame(&frame).unwrap(),
                V3Frame {
                    message_id: 99,
                    message: message.clone(),
                }
            );
            let mut cursor = std::io::Cursor::new(frame);
            assert_eq!(read_frame(&mut cursor).unwrap().unwrap().message, message);
            assert_eq!(read_frame(&mut cursor).unwrap(), None);
        }
    }

    #[test]
    fn every_error_code_round_trips_and_has_a_unique_name() {
        let mut names = std::collections::BTreeSet::new();
        for (index, code) in ErrorCode::ALL.iter().enumerate() {
            assert_eq!(*code as u16 as usize, index + 1);
            assert_eq!(ErrorCode::decode(*code as u16).unwrap(), *code);
            assert!(names.insert(code.name()));
        }
        assert_eq!(ErrorCode::decode(0), Err(V3CodecError::InvalidErrorCode(0)));
        assert_eq!(
            ErrorCode::decode(27),
            Err(V3CodecError::InvalidErrorCode(27))
        );
    }

    #[test]
    fn open_flags_are_fail_closed() {
        let cases: [(u32, u32, &str); 7] = [
            (1 << 8, 0, "unknown"),
            (0, 0, "need READ"),
            (open_flags::READ | open_flags::TRUNC, 0, "require WRITE"),
            (open_flags::WRITE | open_flags::EXCL, 0, "require CREATE"),
            (open_flags::DIRECTORY | open_flags::WRITE, 0, "DIRECTORY"),
            (open_flags::READ, 0o644, "without CREATE"),
            (open_flags::WRITE | open_flags::CREATE, 0o10000, "bound"),
        ];
        for (flags, mode, expected) in cases {
            let error = encode(&V3Message::Open {
                path: b"a".to_vec(),
                flags,
                mode,
                attr_mask: 0,
            })
            .expect_err("accepted invalid open");
            assert!(
                error.to_string().contains(expected),
                "{flags:#x}/{mode:#o}: {error}"
            );
        }
        assert!(encode(&V3Message::Open {
            path: b"d".to_vec(),
            flags: open_flags::DIRECTORY | open_flags::NOFOLLOW,
            mode: 0,
            attr_mask: 0,
        })
        .is_ok());
    }

    #[test]
    fn attrs_reject_unknown_presence_and_misplaced_targets() {
        let mut payload = encode(&V3Message::AttrsResponse {
            related_id: 1,
            attrs: Attrs::minimal(1, 0, 0, 0, [0; 16]),
        })
        .unwrap();
        payload[8..12].copy_from_slice(&(1_u32 << 11).to_le_bytes());
        assert!(matches!(
            decode(types::ATTRS, &payload),
            Err(V3CodecError::UnknownFlags {
                field: "attrs presence",
                ..
            })
        ));

        let mut attrs = Attrs::minimal(1, 0, 0, 0, [0; 16]);
        attrs.symlink_target = Some(b"t".to_vec());
        assert_eq!(
            encode(&V3Message::AttrsResponse {
                related_id: 1,
                attrs,
            }),
            Err(V3CodecError::Inconsistent(
                "symlink target on a non-symlink entry"
            ))
        );
        let mut attrs = Attrs::minimal(1, 0o10000, 0, 0, [0; 16]);
        attrs.kind = 1;
        assert!(matches!(
            encode(&V3Message::AttrsResponse {
                related_id: 1,
                attrs,
            }),
            Err(V3CodecError::Bound {
                field: "attrs mode",
                ..
            })
        ));
    }

    #[test]
    fn mount_info_ties_writability_to_its_reason() {
        let base = |writable: bool, reason: &[u8], access: Access| V3Message::MountInfo {
            related_id: 1,
            export: Vec::new(),
            access,
            effective_writable: writable,
            reason: reason.to_vec(),
            options: Vec::new(),
            case_sensitive: true,
            normalization: Normalization::None,
            max_name_len: 255,
            max_path_len: 4096,
            supports: 0,
            max_read: 1 << 20,
            max_write: 1 << 20,
            attr_cache_ms: 0,
            dir_cache_ms: 0,
        };
        assert!(encode(&base(true, b"", Access::ReadWrite)).is_ok());
        assert!(encode(&base(false, b"ro", Access::ReadOnly)).is_ok());
        assert!(encode(&base(true, b"x", Access::ReadWrite)).is_err());
        assert!(encode(&base(false, b"", Access::ReadWrite)).is_err());
        assert!(encode(&base(true, b"", Access::ReadOnly)).is_err());
    }

    #[test]
    fn stat_target_fields_must_agree() {
        // tag=1 (handle) with a non-empty path.
        let payload = [
            &[1_u8][..],
            &1_u32.to_le_bytes(),
            b"a",
            &10_u64.to_le_bytes(),
            &[0][..],
            &0_u32.to_le_bytes(),
        ]
        .concat();
        assert!(matches!(
            decode(types::STAT, &payload),
            Err(V3CodecError::Inconsistent(_))
        ));
        // tag=0 (path) with a non-zero handle.
        let payload = [
            &[0_u8][..],
            &1_u32.to_le_bytes(),
            b"a",
            &10_u64.to_le_bytes(),
            &[0][..],
            &0_u32.to_le_bytes(),
        ]
        .concat();
        assert!(matches!(
            decode(types::STAT, &payload),
            Err(V3CodecError::Inconsistent(_))
        ));
    }

    #[test]
    fn bounds_and_trailing_bytes_are_rejected() {
        assert!(matches!(
            encode(&V3Message::Read {
                handle: 1,
                offset: 0,
                length: 0,
                want_digest: false,
            }),
            Err(V3CodecError::Bound {
                field: "read length",
                ..
            })
        ));
        assert!(matches!(
            encode(&V3Message::Write {
                handle: 1,
                offset: 0,
                digest: None,
                data: Vec::new(),
            }),
            Err(V3CodecError::Bound {
                field: "write data",
                ..
            })
        ));
        assert!(matches!(
            encode(&V3Message::ReadDir {
                handle: 1,
                cursor: 0,
                max_entries: 65_537,
                attr_mask: 0,
            }),
            Err(V3CodecError::Bound {
                field: "max entries",
                ..
            })
        ));
        let mut payload = encode(&V3Message::Done { related_id: 1 }).unwrap();
        payload.push(0);
        assert_eq!(
            decode(types::DONE, &payload),
            Err(V3CodecError::Trailing(1))
        );
        assert_eq!(decode(types::STAT_FS, &[0]), Err(V3CodecError::Trailing(1)));
    }

    #[test]
    fn unknown_mask_bits_are_ignored_but_unknown_presence_bits_are_not() {
        // A newer client may ask for a block this build does not know: the
        // request decodes, and the server simply does not set that bit in its
        // reply. This is the capability-bit rule, not the enum rule.
        let future = attr_presence::KNOWN | (1 << 11) | (1 << 31);
        for message in [
            V3Message::Stat {
                target: StatTarget::Path(b"a".to_vec()),
                follow: true,
                attr_mask: future,
            },
            V3Message::ReadDir {
                handle: 1,
                cursor: 0,
                max_entries: 1,
                attr_mask: future,
            },
            V3Message::Open {
                path: b"a".to_vec(),
                flags: open_flags::READ,
                mode: 0,
                attr_mask: future,
            },
        ] {
            let payload = encode(&message).unwrap();
            assert_eq!(decode(message_type(&message), &payload).unwrap(), message);
        }
        // The same bit in a *response* presence bitmap is fail-closed, because
        // a decoder cannot skip a block whose length it does not know.
        let mut payload = encode(&V3Message::AttrsResponse {
            related_id: 1,
            attrs: Attrs::minimal(1, 0, 0, 0, [0; 16]),
        })
        .unwrap();
        payload[8..12].copy_from_slice(&future.to_le_bytes());
        assert!(matches!(
            decode(types::ATTRS, &payload),
            Err(V3CodecError::UnknownFlags { .. })
        ));
    }

    #[test]
    fn v2_and_v3_envelopes_reject_each_other() {
        let v3 = encode_frame(1, &V3Message::Done { related_id: 1 }).unwrap();
        assert!(matches!(
            crate::protocol_v2::decode_frame(&v3),
            Err(crate::protocol_v2::V2CodecError::Envelope("wrong version"))
        ));
        let v2 = crate::protocol_v2::encode_frame(
            1,
            &crate::protocol_v2::V2Message::Keepalive { nonce: 1 },
        )
        .unwrap();
        assert_eq!(
            decode_frame(&v2),
            Err(V3CodecError::Envelope("wrong version"))
        );
    }
}
