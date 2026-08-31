//! Native whole-file sender for the rsync receiver wire protocol.
//!
//! This is intentionally a bounded GNU protocol-32 sender. No local rsync
//! executable is launched or required.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use globset::{Glob, GlobSetBuilder};
use md5::{Digest as _, Md5};

use crate::local::{LocalEvent, LocalSyncOptions, LocalSyncReport, TransferMethod};
use crate::transport::{TransportCapabilities, TransportKind, TransportSelection};

/// Selected GNU receiver dialect.
pub const RSYNC_WIRE_VERSION: i32 = 32;
const MAX_FILE_LIST_ENTRIES: usize = 1_048_576;
const MAX_PATH_BYTES: usize = 1024 * 1024;
const MAX_SYMLINK_TARGET_BYTES: usize = 1024 * 1024;
const MAX_MULTIPLEX_PAYLOAD: usize = 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
const DATA_CHUNK_BYTES: usize = 32 * 1024;
const XMIT_TOP_DIR: u32 = 1 << 0;
const XMIT_SAME_UID: u32 = 1 << 3;
const XMIT_SAME_GID: u32 = 1 << 4;
const XMIT_LONG_NAME: u32 = 1 << 6;
const XMIT_MOD_NSEC: u32 = 1 << 13;
const XMIT_NO_CONTENT_DIR: u32 = 1 << 8;
const CF_INC_RECURSE: u32 = 1 << 0;
const CF_CHKSUM_SEED_FIX: u32 = 1 << 5;
const CF_VARINT_FLIST_FLAGS: u32 = 1 << 7;
const ITEM_BASIS_TYPE_FOLLOWS: u16 = 1 << 11;
const ITEM_XNAME_FOLLOWS: u16 = 1 << 12;
const ITEM_TRANSFER: u16 = 1 << 15;
const NDX_DONE: i32 = -1;
const NDX_DEL_STATS: i32 = -3;

/// A positively identified remote rsync implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsyncPeer {
    /// Implementation family reported by `rsync --version`.
    pub implementation: String,
    /// Program version text.
    pub version: String,
    /// Maximum protocol advertised by the version probe.
    pub max_protocol: i32,
}

impl RsyncPeer {
    /// Convert the peer into the engine's transport selection event model.
    #[must_use]
    pub fn selection(&self, reason: impl Into<String>) -> TransportSelection {
        TransportSelection {
            transport: TransportKind::Rsync,
            remote_implementation: self.implementation.clone(),
            remote_version: Some(self.version.clone()),
            wire_version: RSYNC_WIRE_VERSION as u32,
            capabilities: TransportCapabilities {
                multi_stream: false,
                durable_resume: false,
                blake3_frames: false,
                paranoid_readback: false,
                whole_file: true,
            },
            mapped_options: vec![
                "recursive",
                "symlinks",
                "permissions",
                "mtimes",
                "whole-file",
                "force-type-replacement",
            ],
            checksum_algorithm: Some("md5"),
            compression_algorithm: None,
            unavailable_guarantees: vec![
                "multi-stream",
                "durable-resume",
                "blake3-frames",
                "paranoid-readback",
                "compression",
            ],
            reason: reason.into(),
        }
    }
}

/// Rsync fallback failure.
#[derive(Debug, thiserror::Error)]
pub enum RsyncError {
    /// An option cannot preserve its requested guarantee on this backend.
    #[error("rsync transport does not support {0}")]
    UnsupportedOption(&'static str),
    /// The remote rsync probe failed or identified an unsupported peer.
    #[error("unsupported remote rsync: {0}")]
    UnsupportedPeer(String),
    /// A source filesystem operation failed.
    #[error("cannot inspect rsync source '{}': {source}", path.display())]
    Source {
        /// Source path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A source entry changed while it was being sent.
    #[error("source changed while reading '{}'", path.display())]
    SourceChanged {
        /// Path that changed.
        path: PathBuf,
    },
    /// A source object is outside the v1 fallback subset.
    #[error("unsupported source object '{}'", path.display())]
    UnsupportedSource {
        /// Unsupported source path.
        path: PathBuf,
    },
    /// A path exceeds a bounded wire limit or is unsafe.
    #[error("invalid rsync wire path: {0}")]
    InvalidPath(String),
    /// A protocol sequence, field, or multiplex frame was invalid.
    #[error("rsync protocol error: {0}")]
    Protocol(String),
    /// Transport I/O failed.
    #[error("rsync transport I/O failed: {0}")]
    Io(#[from] io::Error),
    /// The remote receiver exited unsuccessfully.
    #[error("remote rsync exited unsuccessfully{status}: {message}")]
    RemoteExit {
        /// Formatted status suffix.
        status: String,
        /// Remote stderr or multiplex diagnostic.
        message: String,
    },
}

#[derive(Debug)]
struct WireEntry {
    source: PathBuf,
    path: Vec<u8>,
    kind: WireKind,
    size: u64,
    mtime: i64,
    mtime_nsec: u32,
    mode: u32,
    link_target: Vec<u8>,
    top_level: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug)]
struct RequestAttributes {
    flags: u16,
    basis_type: Option<u8>,
    comparison_name: Option<Vec<u8>>,
}

impl RequestAttributes {
    fn read<R: Read>(reader: &mut MultiplexReader<'_, R>) -> Result<Self, RsyncError> {
        let flags = reader.read_u16()?;
        let basis_type = if flags & ITEM_BASIS_TYPE_FOLLOWS != 0 {
            Some(reader.read_u8()?)
        } else {
            None
        };
        let comparison_name = if flags & ITEM_XNAME_FOLLOWS != 0 {
            Some(reader.read_vstring(MAX_PATH_BYTES)?)
        } else {
            None
        };
        Ok(Self {
            flags,
            basis_type,
            comparison_name,
        })
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<(), RsyncError> {
        writer.write_all(&self.flags.to_le_bytes())?;
        if let Some(basis_type) = self.basis_type {
            writer.write_all(&[basis_type])?;
        }
        if let Some(name) = &self.comparison_name {
            write_vstring(writer, name)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct IndexDecoder {
    previous_positive: i32,
    previous_negative: i32,
}

impl Default for IndexDecoder {
    fn default() -> Self {
        Self {
            previous_positive: -1,
            previous_negative: 1,
        }
    }
}

impl IndexDecoder {
    fn read<R: Read>(&mut self, reader: &mut MultiplexReader<'_, R>) -> Result<i32, RsyncError> {
        let mut first = reader.read_u8()?;
        let negative = first == 0xff;
        if negative {
            first = reader.read_u8()?;
        } else if first == 0 {
            return Ok(NDX_DONE);
        }
        let previous = if negative {
            &mut self.previous_negative
        } else {
            &mut self.previous_positive
        };
        let number = if first == 0xfe {
            let high = reader.read_u8()?;
            let low = reader.read_u8()?;
            if high & 0x80 != 0 {
                let middle_low = reader.read_u8()?;
                let middle_high = reader.read_u8()?;
                i32::from_le_bytes([low, middle_low, middle_high, high & 0x7f])
            } else {
                previous
                    .checked_add(i32::from(u16::from_be_bytes([high, low])))
                    .ok_or_else(|| RsyncError::Protocol("file index overflow".to_owned()))?
            }
        } else {
            previous
                .checked_add(i32::from(first))
                .ok_or_else(|| RsyncError::Protocol("file index overflow".to_owned()))?
        };
        *previous = number;
        Ok(if negative { -number } else { number })
    }
}

#[derive(Debug)]
struct IndexEncoder {
    previous_positive: i32,
    previous_negative: i32,
}

impl Default for IndexEncoder {
    fn default() -> Self {
        Self {
            previous_positive: -1,
            previous_negative: 1,
        }
    }
}

impl IndexEncoder {
    fn write<W: Write>(&mut self, writer: &mut W, mut index: i32) -> Result<(), RsyncError> {
        if index == NDX_DONE {
            writer.write_all(&[0])?;
            return Ok(());
        }
        let negative = index < 0;
        let previous = if negative {
            writer.write_all(&[0xff])?;
            index = index
                .checked_neg()
                .ok_or_else(|| RsyncError::Protocol("file index overflow".to_owned()))?;
            &mut self.previous_negative
        } else {
            &mut self.previous_positive
        };
        let difference = index - *previous;
        *previous = index;
        if (1..0xfe).contains(&difference) {
            writer.write_all(&[u8::try_from(difference).expect("difference is 1..=253")])?;
        } else if difference < 0 || difference > i32::from(i16::MAX) {
            let bytes = index.to_le_bytes();
            writer.write_all(&[0xfe, bytes[3] | 0x80, bytes[0], bytes[1], bytes[2]])?;
        } else {
            let difference = u16::try_from(difference).expect("difference is 254..=32767");
            writer.write_all(&[
                0xfe,
                u8::try_from(difference >> 8).expect("high byte fits"),
                u8::try_from(difference & 0xff).expect("low byte fits"),
            ])?;
        }
        Ok(())
    }
}

/// Verify options before opening a receiver or mutating a destination.
///
/// # Errors
/// Returns a precise incompatibility for unsupported guarantees.
pub fn validate_options(options: &LocalSyncOptions) -> Result<(), RsyncError> {
    if options.streams > 1 {
        return Err(RsyncError::UnsupportedOption("--streams greater than one"));
    }
    if options.delete {
        return Err(RsyncError::UnsupportedOption("--delete"));
    }
    if options.paranoid {
        return Err(RsyncError::UnsupportedOption("--paranoid"));
    }
    // This path applies `exclude_patterns` only. Include rules mean nothing
    // without their position among the excludes, so honouring the excludes
    // alone would silently transfer a wider set of files than was asked for.
    if options.filter.as_ref().is_some_and(crate::filter::FilterSet::has_includes) {
        return Err(RsyncError::UnsupportedOption(
            "--include (the rsync fallback carries exclude patterns only)",
        ));
    }
    Ok(())
}

/// Validate a probed peer before source scanning or receiver launch.
///
/// # Errors
/// Returns an unsupported-peer error for non-GNU peers older than protocol 32.
pub fn validate_peer(peer: &RsyncPeer) -> Result<(), RsyncError> {
    // The sender speaks protocol 32 explicitly.  A newer GNU rsync remains
    // backwards compatible and negotiates down to 32; requiring equality
    // would unnecessarily reject future (and locally configured) receivers.
    if peer.implementation != "GNU rsync" || peer.max_protocol < RSYNC_WIRE_VERSION {
        return Err(RsyncError::UnsupportedPeer(format!(
            "{} {} advertises protocol {}; v1 requires GNU rsync protocol {RSYNC_WIRE_VERSION}",
            peer.implementation, peer.version, peer.max_protocol
        )));
    }
    Ok(())
}

/// Probe `rsync --version` through the configured remote shell.
///
/// # Errors
/// Returns an error unless a supported GNU/openrsync family and protocol is
/// positively identified.
pub fn probe_remote(rsh: Option<&str>, host: &str) -> Result<RsyncPeer, RsyncError> {
    let output = run_command_capture(remote_command(rsh, host, &[b"rsync", b"--version"])?)?;
    if !output.status.success() {
        let status = output
            .status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string());
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if detail.is_empty() {
            format!("remote shell exited with status {status}")
        } else {
            format!("remote shell exited with status {status}: {detail}")
        };
        return Err(RsyncError::UnsupportedPeer(detail));
    }
    parse_version_probe(&output.stdout)
}

/// Parse `rsync --version` stdout into a peer identity.
///
/// Embeddings that already hold a remote exec channel can probe with that
/// instead of spawning `ssh`.
///
/// # Errors
/// Returns [`RsyncError::UnsupportedPeer`] when the banner is not GNU rsync
/// or openrsync, or when it does not advertise a protocol version.
pub fn parse_version_output(stdout: &[u8]) -> Result<RsyncPeer, RsyncError> {
    parse_version_probe(stdout)
}

/// `rsync --server` argv for a receiver rooted at `destination`.
///
/// Arguments are unquoted; the caller quotes them for the remote shell.
///
/// # Errors
/// Returns [`RsyncError::UnsupportedOption`] when `options` cannot be
/// expressed on this backend.
pub fn server_argv(
    destination: &str,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
) -> Result<Vec<Vec<u8>>, RsyncError> {
    validate_options(options)?;
    let mut command_args = vec![
        b"rsync".to_vec(),
        b"--server".to_vec(),
        b"-lptrW".to_vec(),
        b"-e.Cv".to_vec(),
        b"--dirs".to_vec(),
        b"--force".to_vec(),
        b"--no-inc-recursive".to_vec(),
    ];
    command_args.extend(
        options
            .exclude_patterns
            .iter()
            .map(|pattern| format!("--exclude={pattern}").into_bytes()),
    );
    if options.dry_run {
        command_args.push(b"--dry-run".to_vec());
    }
    command_args.push(b".".to_vec());
    let mut destination_arg = destination.as_bytes().to_vec();
    if destination_trailing_slash && !destination_arg.ends_with(b"/") {
        destination_arg.push(b'/');
    }
    command_args.push(destination_arg);
    Ok(command_args)
}

/// Drive the native rsync-wire sender over caller-provided I/O.
///
/// Same codec as [`sync_push`], but does not spawn `ssh` or `rsync`. Used by
/// embeddings that already hold a transport (a russh exec channel, a test
/// pipe).
///
/// # Errors
/// Returns on source, protocol, or transport failure.
pub fn sync_push_io<R: Read, W: Write, F: FnMut(LocalEvent)>(
    source: &Path,
    source_trailing_slash: bool,
    options: &LocalSyncOptions,
    peer: &RsyncPeer,
    reader: R,
    writer: W,
    mut emit: F,
) -> Result<LocalSyncReport, RsyncError> {
    validate_options(options)?;
    validate_peer(peer)?;
    let entries = apply_excludes(
        scan_source(source, source_trailing_slash)?,
        &options.exclude_patterns,
    )?;
    let planned_files = entries.iter().filter(|e| e.kind == WireKind::File).count();
    let planned_bytes = entries
        .iter()
        .filter(|e| e.kind == WireKind::File)
        .map(|e| e.size)
        .sum();

    emit(LocalEvent::Started {
        local_workers: 1,
        streams: 1,
    });
    emit(LocalEvent::Planned {
        files: planned_files,
        bytes: planned_bytes,
    });
    if options.dry_run {
        for entry in &entries {
            emit(LocalEvent::Action {
                path: display_wire_path(&entry.path),
                action: "create",
            });
        }
    }

    let mut reader = BufReader::new(reader);
    let mut writer = CountingWriter::new(BufWriter::new(writer));
    let mut result = run_session(
        &mut reader,
        &mut writer,
        &entries,
        &mut emit,
        options.dry_run,
    );
    let wire_bytes = writer.bytes;
    drop(reader);
    drop(writer);
    if let Ok(report) = &mut result {
        report.wire_bytes = wire_bytes;
    }
    let report = result?;
    emit_finished(&report, &mut emit);
    Ok(report)
}

/// Push a local source through the native rsync-wire sender.
///
/// # Errors
/// Returns on source, protocol, transport, or remote receiver failure.
#[allow(clippy::too_many_arguments)]
pub fn sync_push<F: FnMut(LocalEvent)>(
    source: &Path,
    source_trailing_slash: bool,
    destination: &str,
    destination_trailing_slash: bool,
    options: &LocalSyncOptions,
    rsh: Option<&str>,
    host: &str,
    peer: &RsyncPeer,
    mut emit: F,
) -> Result<LocalSyncReport, RsyncError> {
    validate_options(options)?;
    validate_peer(peer)?;
    let entries = apply_excludes(
        scan_source(source, source_trailing_slash)?,
        &options.exclude_patterns,
    )?;
    let planned_files = entries.iter().filter(|e| e.kind == WireKind::File).count();
    let planned_bytes = entries
        .iter()
        .filter(|e| e.kind == WireKind::File)
        .map(|e| e.size)
        .sum();

    emit(LocalEvent::Started {
        local_workers: 1,
        streams: 1,
    });
    emit(LocalEvent::Planned {
        files: planned_files,
        bytes: planned_bytes,
    });
    if options.dry_run {
        for entry in &entries {
            emit(LocalEvent::Action {
                path: display_wire_path(&entry.path),
                action: "create",
            });
        }
    }

    let mut command_args = vec![
        b"rsync".to_vec(),
        b"--server".to_vec(),
        b"-lptrW".to_vec(),
        b"-e.Cv".to_vec(),
        b"--dirs".to_vec(),
        b"--force".to_vec(),
        b"--no-inc-recursive".to_vec(),
    ];
    command_args.extend(
        options
            .exclude_patterns
            .iter()
            .map(|pattern| format!("--exclude={pattern}").into_bytes()),
    );
    if options.dry_run {
        command_args.push(b"--dry-run".to_vec());
    }
    command_args.push(b".".to_vec());
    let mut destination_arg = destination.as_bytes().to_vec();
    // Path parsing keeps the slash separately, but rsync uses it to decide
    // whether a non-existent destination should be created as a directory.
    if destination_trailing_slash && !destination_arg.ends_with(b"/") {
        destination_arg.push(b'/');
    }
    command_args.push(destination_arg);
    let command_refs: Vec<&[u8]> = command_args.iter().map(Vec::as_slice).collect();
    let mut child = remote_command(rsh, host, &command_refs)?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RsyncError::Io)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| RsyncError::Protocol("remote rsync stdin was not piped".to_owned()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RsyncError::Protocol("remote rsync stdout was not piped".to_owned()))?;
    let stderr = child.stderr.take();
    let stderr_thread = stderr.map(|pipe| std::thread::spawn(move || read_bounded(pipe)));

    let mut reader = BufReader::new(stdout);
    let mut writer = CountingWriter::new(BufWriter::new(stdin));
    let mut result = run_session(
        &mut reader,
        &mut writer,
        &entries,
        &mut emit,
        options.dry_run,
    );
    let wire_bytes = writer.bytes;
    drop(reader);
    drop(writer);
    if let Ok(report) = &mut result {
        report.wire_bytes = wire_bytes;
    }
    let status = finish_child(&mut child, stderr_thread);

    match (result, status) {
        (Err(session), Err(remote)) => Err(RsyncError::Protocol(format!("{session}; {remote}"))),
        (Ok(_), Err(err)) | (Err(err), Ok(())) => Err(err),
        (Ok(report), Ok(())) => {
            emit_finished(&report, &mut emit);
            Ok(report)
        }
    }
}

fn parse_version_probe(stdout: &[u8]) -> Result<RsyncPeer, RsyncError> {
    let text = String::from_utf8_lossy(stdout);
    let first = text.lines().next().unwrap_or_default();
    let lower = first.to_ascii_lowercase();
    let implementation = if lower.starts_with("rsync  version") {
        "GNU rsync"
    } else if lower.starts_with("openrsync:") {
        "openrsync"
    } else {
        return Err(RsyncError::UnsupportedPeer(first.to_owned()));
    };
    let protocol = text
        .lines()
        .find_map(|line| line.split("protocol version").nth(1))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|n| n.parse::<i32>().ok())
        .ok_or_else(|| RsyncError::UnsupportedPeer("missing protocol version".to_owned()))?;
    let version = if implementation == "GNU rsync" {
        first
            .split("version")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("unknown")
            .to_owned()
    } else {
        text.lines()
            .find_map(|line| line.strip_prefix("rsync version "))
            .unwrap_or(first)
            .to_owned()
    };
    Ok(RsyncPeer {
        implementation: implementation.to_owned(),
        version,
        max_protocol: protocol,
    })
}

#[allow(clippy::too_many_lines)]
fn run_session<R: Read, W: Write, F: FnMut(LocalEvent)>(
    reader: &mut R,
    writer: &mut W,
    entries: &[WireEntry],
    emit: &mut F,
    dry_run: bool,
) -> Result<LocalSyncReport, RsyncError> {
    write_i32(writer, RSYNC_WIRE_VERSION)?;
    writer.flush()?;
    let remote_version = read_raw_i32(reader)?;
    if remote_version != RSYNC_WIRE_VERSION {
        return Err(RsyncError::UnsupportedPeer(format!(
            "receiver negotiated wire protocol {remote_version}; expected {RSYNC_WIRE_VERSION}"
        )));
    }
    let compat_flags = read_varint_raw(reader)?;
    let required = CF_CHKSUM_SEED_FIX | CF_VARINT_FLIST_FLAGS;
    if compat_flags & CF_INC_RECURSE != 0 || compat_flags & required != required {
        return Err(RsyncError::UnsupportedPeer(format!(
            "incompatible GNU capability flags 0x{compat_flags:x}"
        )));
    }
    write_vstring(writer, b"md5")?;
    writer.flush()?;
    let checksum_choices = read_vstring_raw(reader)?;
    if !checksum_choices
        .split(|byte| *byte == b' ')
        .any(|name| name == b"md5")
    {
        return Err(RsyncError::UnsupportedPeer(format!(
            "receiver checksum list '{}' does not include md5",
            String::from_utf8_lossy(&checksum_choices)
        )));
    }
    let _checksum_seed = read_raw_i32(reader)?;
    let mut input = MultiplexReader::new(reader);
    let mut output = MultiplexWriter::new(writer);

    write_file_list(&mut output, entries)?;
    output.flush()?;

    let mut report = LocalSyncReport {
        local_workers: 1,
        streams: 1,
        ..LocalSyncReport::default()
    };
    let mut requested = vec![false; entries.len()];
    let mut incoming_indexes = IndexDecoder::default();
    let mut outgoing_indexes = IndexEncoder::default();

    for _phase in 0..3 {
        loop {
            let index = incoming_indexes.read(&mut input)?;
            if index == NDX_DONE {
                break;
            }
            if index == NDX_DEL_STATS {
                for _ in 0..5 {
                    let _ = input.read_varint()?;
                }
                continue;
            }
            let index = usize::try_from(index)
                .map_err(|_| RsyncError::Protocol(format!("unexpected control index {index}")))?;
            let entry = entries.get(index).ok_or_else(|| {
                RsyncError::Protocol(format!("file index {index} is outside the file list"))
            })?;
            let attributes = RequestAttributes::read(&mut input)?;

            outgoing_indexes.write(
                &mut output,
                i32::try_from(index)
                    .map_err(|_| RsyncError::Protocol("file index exceeds i32".to_owned()))?,
            )?;
            attributes.write(&mut output)?;
            if attributes.flags & ITEM_TRANSFER != 0 {
                if entry.kind != WireKind::File {
                    return Err(RsyncError::Protocol(format!(
                        "receiver requested data for non-file index {index} flags 0x{:x} ({:?} {})",
                        attributes.flags,
                        entry.kind,
                        display_wire_path(&entry.path)
                    )));
                }
                let sums = [
                    input.read_i32()?,
                    input.read_i32()?,
                    input.read_i32()?,
                    input.read_i32()?,
                ];
                if sums != [0; 4] {
                    return Err(RsyncError::Protocol(
                        "receiver requested delta tokens despite --whole-file".to_owned(),
                    ));
                }
                send_file_data(&mut output, entry, sums)?;
                if !dry_run {
                    report.physical_bytes = report.physical_bytes.saturating_add(entry.size);
                }
                if !requested[index] && !dry_run {
                    requested[index] = true;
                    report.transferred_files = report.transferred_files.saturating_add(1);
                    report.transferred_bytes = report.transferred_bytes.saturating_add(entry.size);
                    report.byte_copies = report.byte_copies.saturating_add(1);
                    emit(LocalEvent::Transferred {
                        path: display_wire_path(&entry.path),
                        bytes: entry.size,
                        physical_bytes: entry.size,
                        method: TransferMethod::ByteCopy,
                    });
                }
            }
            output.flush()?;
        }
        outgoing_indexes.write(&mut output, NDX_DONE)?;
        output.flush()?;
    }

    expect_done(&mut incoming_indexes, &mut input, "early goodbye")?;
    outgoing_indexes.write(&mut output, NDX_DONE)?;
    output.flush()?;
    expect_done(&mut incoming_indexes, &mut input, "final goodbye")?;

    for (index, entry) in entries.iter().enumerate() {
        if entry.kind == WireKind::File && !requested[index] && !dry_run {
            report.skipped_files = report.skipped_files.saturating_add(1);
            emit(LocalEvent::Skipped {
                path: display_wire_path(&entry.path),
                bytes: entry.size,
            });
        }
    }
    Ok(report)
}

fn expect_done<R: Read>(
    indexes: &mut IndexDecoder,
    reader: &mut MultiplexReader<'_, R>,
    stage: &str,
) -> Result<(), RsyncError> {
    loop {
        let index = indexes.read(reader)?;
        if index == NDX_DONE {
            return Ok(());
        }
        if index == NDX_DEL_STATS {
            for _ in 0..5 {
                let _ = reader.read_varint()?;
            }
            continue;
        }
        return Err(RsyncError::Protocol(format!(
            "expected {stage} marker, got index {index}"
        )));
    }
}

fn emit_finished<F: FnMut(LocalEvent)>(report: &LocalSyncReport, emit: &mut F) {
    emit(LocalEvent::Finished {
        dropped_metadata: crate::sparse::DroppedMetadata::default(),
        transport: None,
        transferred_files: report.transferred_files,
        transferred_bytes: report.transferred_bytes,
        physical_bytes: report.physical_bytes,
        wire_bytes: report.wire_bytes,
        skipped_files: report.skipped_files,
        metadata_repaired: report.metadata_repaired,
        failed_entries: 0,
        deleted_entries: 0,
        warnings: 0,
        local_workers: 1,
        streams: 1,
        partial_failure: false,
        directory_clones: 0,
        file_clones: 0,
        byte_copies: report.byte_copies,
        restarted_files: 0,
        resumed_bytes: 0,
        retransmitted_bytes: report.physical_bytes,
        checkpoint_bytes: 0,
        checksum_cache_hits: 0,
        checksum_cache_misses: 0,
    });
}

fn write_file_list<W: Write>(writer: &mut W, entries: &[WireEntry]) -> Result<(), RsyncError> {
    for entry in entries {
        let mut flags = XMIT_SAME_UID | XMIT_SAME_GID;
        if entry.top_level {
            flags |= XMIT_TOP_DIR;
        } else if entry.kind == WireKind::Directory {
            flags |= XMIT_NO_CONTENT_DIR;
        }
        if entry.path.len() > usize::from(u8::MAX) {
            flags |= XMIT_LONG_NAME;
        }
        if entry.mtime_nsec != 0 {
            flags |= XMIT_MOD_NSEC;
        }
        write_varint(writer, flags)?;
        if flags & XMIT_LONG_NAME != 0 {
            write_varint(
                writer,
                u32::try_from(entry.path.len())
                    .map_err(|_| RsyncError::InvalidPath("path length exceeds u32".to_owned()))?,
            )?;
        } else {
            writer.write_all(&[u8::try_from(entry.path.len()).expect("short path fits")])?;
        }
        writer.write_all(&entry.path)?;
        write_varlong(writer, entry.size, 3)?;
        write_varlong(
            writer,
            u64::try_from(entry.mtime).map_err(|_| {
                RsyncError::Protocol("negative mtimes are unsupported by GNU v1".to_owned())
            })?,
            4,
        )?;
        if flags & XMIT_MOD_NSEC != 0 {
            write_varint(writer, entry.mtime_nsec)?;
        }
        writer.write_all(&entry.mode.to_le_bytes())?;
        if entry.kind == WireKind::Symlink {
            write_varint(
                writer,
                u32::try_from(entry.link_target.len()).map_err(|_| {
                    RsyncError::InvalidPath("symlink target length exceeds u32".to_owned())
                })?,
            )?;
            writer.write_all(&entry.link_target)?;
        }
    }
    write_varint(writer, 0)?; // End of file list.
    write_varint(writer, 0)?; // Sender-side IO error accumulator.
    Ok(())
}

fn send_file_data<W: Write>(
    writer: &mut W,
    entry: &WireEntry,
    sums: [i32; 4],
) -> Result<(), RsyncError> {
    let before = fs::symlink_metadata(&entry.source).map_err(|source| RsyncError::Source {
        path: entry.source.clone(),
        source,
    })?;
    for value in sums {
        write_i32(writer, value)?;
    }
    let mut digest = Md5::new();
    let mut file = open_source_file(&entry.source)?;
    let mut buf = vec![0u8; DATA_CHUNK_BYTES];
    loop {
        let count = file.read(&mut buf).map_err(|source| RsyncError::Source {
            path: entry.source.clone(),
            source,
        })?;
        if count == 0 {
            break;
        }
        write_i32(
            writer,
            i32::try_from(count).expect("32 KiB chunk always fits i32"),
        )?;
        writer.write_all(&buf[..count])?;
        digest.update(&buf[..count]);
    }
    write_i32(writer, 0)?;
    writer.write_all(&digest.finalize())?;

    let after = file.metadata().map_err(|source| RsyncError::Source {
        path: entry.source.clone(),
        source,
    })?;
    if !metadata_stable(&before, &after) {
        return Err(RsyncError::SourceChanged {
            path: entry.source.clone(),
        });
    }
    Ok(())
}

fn apply_excludes(
    entries: Vec<WireEntry>,
    patterns: &[String],
) -> Result<Vec<WireEntry>, RsyncError> {
    if patterns.is_empty() {
        return Ok(entries);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|error| {
            RsyncError::InvalidPath(format!("invalid exclude pattern: {error}"))
        })?;
        builder.add(glob);
    }
    let matcher = builder
        .build()
        .map_err(|error| RsyncError::InvalidPath(format!("invalid exclude patterns: {error}")))?;
    Ok(entries
        .into_iter()
        .filter(|entry| {
            let display = String::from_utf8_lossy(&entry.path);
            if matcher.is_match(display.as_ref()) {
                return false;
            }
            let mut prefix = display.as_ref();
            while let Some((ancestor, _)) = prefix.rsplit_once('/') {
                if matcher.is_match(ancestor) {
                    return false;
                }
                prefix = ancestor;
            }
            true
        })
        .collect())
}

fn scan_source(source: &Path, trailing_slash: bool) -> Result<Vec<WireEntry>, RsyncError> {
    let metadata = fs::symlink_metadata(source).map_err(|source_error| RsyncError::Source {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut entries = Vec::new();
    if metadata.is_dir() {
        let root_name = if trailing_slash {
            b".".to_vec()
        } else {
            os_bytes(source.file_name().unwrap_or_else(|| OsStr::new(".")))
        };
        push_entry(
            &mut entries,
            source.to_path_buf(),
            root_name.clone(),
            &metadata,
            true,
        )?;
        scan_directory(source, &root_name, trailing_slash, &mut entries)?;
    } else {
        let name = os_bytes(
            source
                .file_name()
                .ok_or_else(|| RsyncError::InvalidPath("source has no file name".to_owned()))?,
        );
        push_entry(&mut entries, source.to_path_buf(), name, &metadata, true)?;
    }
    if entries.len() > MAX_FILE_LIST_ENTRIES {
        return Err(RsyncError::InvalidPath(format!(
            "file list exceeds {MAX_FILE_LIST_ENTRIES} entries"
        )));
    }
    entries.sort_by(rsync_name_cmp);
    Ok(entries)
}

/// Order entries exactly the way GNU rsync's `f_name_cmp` does.
///
/// The receiver re-sorts the file list it is handed, so an ordering that
/// merely looks reasonable is not enough: any divergence renumbers the shared
/// index space and the receiver ends up asking for the wrong entry. Names
/// compare as `dirname`, `/`, then `basename`; a directory compares as though
/// its name carried a trailing `/`; and entries that share a parent directory
/// put non-directories ahead of directories.
fn rsync_name_cmp(left: &WireEntry, right: &WireEntry) -> std::cmp::Ordering {
    let (left_parent, left_name) = split_wire_name(&left.path);
    let (right_parent, right_name) = split_wire_name(&right.path);
    // rsync interns directory names and takes a shortcut when two entries
    // share one, which is what makes the file-before-directory rule apply
    // within a directory but not across directories.
    let shared_parent = left_parent == right_parent;
    let mut left = NameCursor::new(
        left_parent,
        left_name,
        left.kind == WireKind::Directory,
        shared_parent,
    );
    let mut right = NameCursor::new(
        right_parent,
        right_name,
        right.kind == WireKind::Directory,
        shared_parent,
    );
    loop {
        if left.remaining.is_empty() && left.state != NameState::Done {
            left.advance();
            continue;
        }
        if right.remaining.is_empty() && right.state != NameState::Done {
            right.advance();
            continue;
        }
        if left.kind != right.kind {
            return if left.kind == NameKind::Path {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        let left_byte = left.remaining.first().copied().unwrap_or(0);
        let right_byte = right.remaining.first().copied().unwrap_or(0);
        if left_byte != right_byte {
            return left_byte.cmp(&right_byte);
        }
        if left_byte == 0 {
            return std::cmp::Ordering::Equal;
        }
        left.remaining = &left.remaining[1..];
        right.remaining = &right.remaining[1..];
    }
}

/// Split a wire path into its parent directory and its final component.
fn split_wire_name(path: &[u8]) -> (Option<&[u8]>, &[u8]) {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or((None, path), |index| {
            (Some(&path[..index]), &path[index + 1..])
        })
}

/// Which of rsync's two name classes the cursor is currently emitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameKind {
    Path,
    Item,
}

/// Position of a name cursor within `dirname` `/` `basename` `/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameState {
    Dir,
    Slash,
    Base,
    Trailing,
    Done,
}

/// Streams one entry's name the way rsync's comparison walks it.
#[derive(Debug)]
struct NameCursor<'a> {
    basename: &'a [u8],
    directory: bool,
    remaining: &'a [u8],
    state: NameState,
    kind: NameKind,
}

impl<'a> NameCursor<'a> {
    fn new(
        parent: Option<&'a [u8]>,
        basename: &'a [u8],
        directory: bool,
        shared_parent: bool,
    ) -> Self {
        let mut cursor = Self {
            basename,
            directory,
            remaining: b"",
            state: NameState::Base,
            kind: NameKind::Item,
        };
        match parent {
            Some(parent) if !shared_parent => {
                cursor.kind = NameKind::Path;
                cursor.state = NameState::Dir;
                cursor.remaining = parent;
            }
            _ => cursor.enter_basename(),
        }
        cursor
    }

    fn enter_basename(&mut self) {
        self.kind = if self.directory {
            NameKind::Path
        } else {
            NameKind::Item
        };
        self.remaining = self.basename;
        if self.kind == NameKind::Path && self.basename == b"." {
            // The transfer root sorts ahead of everything it contains.
            self.kind = NameKind::Item;
            self.state = NameState::Trailing;
            self.remaining = b"";
        } else {
            self.state = NameState::Base;
        }
    }

    fn advance(&mut self) {
        match self.state {
            NameState::Dir => {
                self.state = NameState::Slash;
                self.remaining = b"/";
            }
            NameState::Slash => self.enter_basename(),
            NameState::Base => {
                self.state = NameState::Trailing;
                if self.kind == NameKind::Path {
                    self.remaining = b"/";
                } else {
                    self.remaining = b"";
                }
            }
            NameState::Trailing | NameState::Done => {
                self.state = NameState::Done;
                self.kind = NameKind::Item;
                self.remaining = b"";
            }
        }
    }
}

fn scan_directory(
    directory: &Path,
    wire_root: &[u8],
    root_is_dot: bool,
    entries: &mut Vec<WireEntry>,
) -> Result<(), RsyncError> {
    let iter = fs::read_dir(directory).map_err(|source| RsyncError::Source {
        path: directory.to_path_buf(),
        source,
    })?;
    for item in iter {
        let item = item.map_err(|source| RsyncError::Source {
            path: directory.to_path_buf(),
            source,
        })?;
        let source_path = item.path();
        let name = os_bytes(&item.file_name());
        let path = if root_is_dot && wire_root == b"." {
            name
        } else {
            let mut path = Vec::with_capacity(wire_root.len() + 1 + name.len());
            path.extend_from_slice(wire_root);
            path.push(b'/');
            path.extend_from_slice(&name);
            path
        };
        let metadata = fs::symlink_metadata(&source_path).map_err(|source| RsyncError::Source {
            path: source_path.clone(),
            source,
        })?;
        let is_dir = metadata.is_dir();
        push_entry(entries, source_path.clone(), path.clone(), &metadata, false)?;
        if is_dir {
            scan_directory(&source_path, &path, false, entries)?;
        }
    }
    Ok(())
}

fn push_entry(
    entries: &mut Vec<WireEntry>,
    source: PathBuf,
    path: Vec<u8>,
    metadata: &Metadata,
    top_level: bool,
) -> Result<(), RsyncError> {
    if entries.len() >= MAX_FILE_LIST_ENTRIES {
        return Err(RsyncError::InvalidPath(format!(
            "file list exceeds {MAX_FILE_LIST_ENTRIES} entries"
        )));
    }
    validate_wire_path(&path)?;
    let file_type = metadata.file_type();
    let (kind, target) = if file_type.is_file() {
        (WireKind::File, Vec::new())
    } else if file_type.is_dir() {
        (WireKind::Directory, Vec::new())
    } else if file_type.is_symlink() {
        let target = fs::read_link(&source).map_err(|source_error| RsyncError::Source {
            path: source.clone(),
            source: source_error,
        })?;
        let target = os_bytes(target.as_os_str());
        if target.len() > MAX_SYMLINK_TARGET_BYTES || target.contains(&0) {
            return Err(RsyncError::InvalidPath(format!(
                "symlink target for '{}' is oversized or contains NUL",
                source.display()
            )));
        }
        (WireKind::Symlink, target)
    } else {
        return Err(RsyncError::UnsupportedSource { path: source });
    };
    entries.push(WireEntry {
        source,
        path,
        kind,
        size: metadata.len(),
        mtime: metadata_mtime(metadata),
        mtime_nsec: metadata_mtime_nsec(metadata),
        mode: metadata_mode(metadata),
        link_target: target,
        top_level,
    });
    Ok(())
}

fn validate_wire_path(path: &[u8]) -> Result<(), RsyncError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains(&0) {
        return Err(RsyncError::InvalidPath(
            "empty, oversized, or NUL path".to_owned(),
        ));
    }
    if path.first() == Some(&b'/')
        || path
            .split(|b| *b == b'/')
            .any(|part| part == b".." || part.is_empty())
    {
        return Err(RsyncError::InvalidPath(display_wire_path(path)));
    }
    Ok(())
}

struct MultiplexReader<'a, R> {
    inner: &'a mut R,
    data_remaining: usize,
}

struct MultiplexWriter<'a, W> {
    inner: &'a mut W,
}

impl<'a, W> MultiplexWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for MultiplexWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = bytes.len().min(MAX_MULTIPLEX_PAYLOAD);
        let header = (7u32 << 24) | u32::try_from(count).expect("multiplex bound fits u32");
        self.inner.write_all(&header.to_le_bytes())?;
        self.inner.write_all(&bytes[..count])?;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.bytes = self.bytes.saturating_add(count as u64);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<'a, R: Read> MultiplexReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            data_remaining: 0,
        }
    }

    fn read_i32(&mut self) -> Result<i32, RsyncError> {
        let mut bytes = [0u8; 4];
        self.read_data_exact(&mut bytes)?;
        Ok(i32::from_le_bytes(bytes))
    }

    fn read_u8(&mut self) -> Result<u8, RsyncError> {
        let mut byte = [0u8; 1];
        self.read_data_exact(&mut byte)?;
        Ok(byte[0])
    }

    fn read_u16(&mut self) -> Result<u16, RsyncError> {
        let mut bytes = [0u8; 2];
        self.read_data_exact(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_varint(&mut self) -> Result<u32, RsyncError> {
        let first = self.read_u8()?;
        decode_varint(first, |bytes| self.read_data_exact(bytes))
    }

    fn read_vstring(&mut self, maximum: usize) -> Result<Vec<u8>, RsyncError> {
        let first = self.read_u8()?;
        let length = if first & 0x80 == 0 {
            usize::from(first)
        } else {
            (usize::from(first & 0x7f) << 8) | usize::from(self.read_u8()?)
        };
        if length > maximum {
            return Err(RsyncError::Protocol(format!(
                "vstring length {length} exceeds {maximum}"
            )));
        }
        let mut value = vec![0u8; length];
        self.read_data_exact(&mut value)?;
        Ok(value)
    }

    fn read_data_exact(&mut self, mut output: &mut [u8]) -> Result<(), RsyncError> {
        while !output.is_empty() {
            if self.data_remaining == 0 {
                self.read_header()?;
                continue;
            }
            let count = output.len().min(self.data_remaining);
            self.inner.read_exact(&mut output[..count])?;
            self.data_remaining -= count;
            output = &mut output[count..];
        }
        Ok(())
    }

    fn read_header(&mut self) -> Result<(), RsyncError> {
        loop {
            let header = read_raw_i32(self.inner)?.cast_unsigned();
            let length = (header & 0x00ff_ffff) as usize;
            let tag = (header >> 24).cast_signed() - 7;
            if length > MAX_MULTIPLEX_PAYLOAD {
                return Err(RsyncError::Protocol(format!(
                    "multiplex payload {length} exceeds {MAX_MULTIPLEX_PAYLOAD}"
                )));
            }
            if tag == 0 {
                self.data_remaining = length;
                if length != 0 {
                    return Ok(());
                }
                continue;
            }
            let mut message = vec![0u8; length];
            self.inner.read_exact(&mut message)?;
            let text = String::from_utf8_lossy(&message).trim().to_owned();
            if matches!(tag, 1 | 3 | 5) {
                return Err(RsyncError::RemoteExit {
                    status: String::new(),
                    message: text,
                });
            }
        }
    }
}

fn remote_command(
    rsh: Option<&str>,
    host: &str,
    remote_args: &[&[u8]],
) -> Result<Command, RsyncError> {
    let host_name = host.rsplit_once('@').map_or(host, |(_, host)| host);
    if host_name.starts_with('-')
        || host.is_empty()
        || host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(RsyncError::InvalidPath("unsafe remote host".to_owned()));
    }
    let parts = shlex::split(rsh.unwrap_or("ssh"))
        .ok_or_else(|| RsyncError::InvalidPath("invalid remote-shell command".to_owned()))?;
    let program = parts
        .first()
        .ok_or_else(|| RsyncError::InvalidPath("empty remote-shell command".to_owned()))?;
    let mut command = Command::new(program);
    command.args(&parts[1..]);
    command.arg(host);
    command.arg(shell_command(remote_args)?);
    Ok(command)
}

fn shell_command(args: &[&[u8]]) -> Result<OsString, RsyncError> {
    let mut command = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            command.push(b' ');
        }
        if index == args.len() - 1 {
            command.extend_from_slice(&shell_quote_remote_path(arg)?);
        } else {
            command.extend_from_slice(&shell_quote(arg)?);
        }
    }
    os_string(command)
}

fn shell_quote_remote_path(arg: &[u8]) -> Result<Vec<u8>, RsyncError> {
    if arg == b"~" {
        return Ok(b"\"$HOME\"".to_vec());
    }
    if let Some(relative) = arg.strip_prefix(b"~/") {
        let mut quoted = b"\"$HOME\"/".to_vec();
        quoted.extend_from_slice(&shell_quote(relative)?);
        return Ok(quoted);
    }
    shell_quote(arg)
}

fn shell_quote(arg: &[u8]) -> Result<Vec<u8>, RsyncError> {
    if arg.contains(&0) {
        return Err(RsyncError::InvalidPath("NUL in remote argument".to_owned()));
    }
    let mut out = Vec::with_capacity(arg.len() + 2);
    out.push(b'\'');
    for &byte in arg {
        if byte == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(byte);
        }
    }
    out.push(b'\'');
    Ok(out)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn os_string(bytes: Vec<u8>) -> Result<OsString, RsyncError> {
    Ok(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn os_string(bytes: Vec<u8>) -> Result<OsString, RsyncError> {
    String::from_utf8(bytes)
        .map(OsString::from)
        .map_err(|_| RsyncError::InvalidPath("non-UTF-8 remote argument".to_owned()))
}

fn finish_child(
    child: &mut Child,
    stderr_thread: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> Result<(), RsyncError> {
    let status = child.wait()?;
    let stderr = stderr_thread
        .map(|thread| {
            thread
                .join()
                .map_err(|_| io::Error::other("remote stderr reader panicked"))?
        })
        .transpose()?
        .unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(RsyncError::RemoteExit {
        status: status
            .code()
            .map_or_else(String::new, |code| format!(" (status {code})")),
        message: String::from_utf8_lossy(&stderr).trim().to_owned(),
    })
}

struct CapturedOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command_capture(mut command: Command) -> Result<CapturedOutput, RsyncError> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("probe stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("probe stderr was not piped"))?;
    let stdout_thread = std::thread::spawn(move || read_bounded(stdout));
    let stderr_thread = std::thread::spawn(move || read_bounded(stderr));
    let status = child.wait()?;
    let stdout = stdout_thread
        .join()
        .map_err(|_| io::Error::other("probe stdout reader panicked"))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| io::Error::other("probe stderr reader panicked"))??;
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded<R: Read>(reader: R) -> io::Result<Vec<u8>> {
    let limit = u64::try_from(MAX_DIAGNOSTIC_BYTES).expect("diagnostic bound fits u64") + 1;
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_DIAGNOSTIC_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("remote diagnostic exceeds {MAX_DIAGNOSTIC_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

fn write_i32<W: Write>(writer: &mut W, value: i32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_varint<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    write_varlong(writer, u64::from(value), 1)
}

fn write_varlong<W: Write>(writer: &mut W, value: u64, minimum: usize) -> io::Result<()> {
    debug_assert!((1..=8).contains(&minimum));
    let bytes = value.to_le_bytes();
    let mut count = 8;
    while count > minimum && bytes[count - 1] == 0 {
        count -= 1;
    }
    let bit = 1u8 << (7 - count + minimum);
    let prefix;
    if bytes[count - 1] >= bit {
        count += 1;
        prefix = !(bit - 1);
    } else if count > minimum {
        prefix = bytes[count - 1] | !(bit * 2 - 1);
    } else {
        prefix = bytes[count - 1];
    }
    writer.write_all(&[prefix])?;
    writer.write_all(&bytes[..count - 1])
}

fn write_vstring<W: Write>(writer: &mut W, value: &[u8]) -> Result<(), RsyncError> {
    if value.len() > 0x7fff {
        return Err(RsyncError::Protocol(format!(
            "vstring length {} exceeds 32767",
            value.len()
        )));
    }
    if value.len() > 0x7f {
        writer.write_all(&[
            u8::try_from(value.len() / 0x100).expect("vstring high byte fits") + 0x80,
        ])?;
    }
    writer.write_all(&[u8::try_from(value.len() & 0xff).expect("vstring low byte fits")])?;
    writer.write_all(value)?;
    Ok(())
}

fn read_raw_i32<R: Read>(reader: &mut R) -> io::Result<i32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_varint_raw<R: Read>(reader: &mut R) -> Result<u32, RsyncError> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first)?;
    decode_varint(first[0], |bytes| {
        reader.read_exact(bytes).map_err(RsyncError::Io)
    })
}

fn decode_varint<F>(first: u8, mut read_exact: F) -> Result<u32, RsyncError>
where
    F: FnMut(&mut [u8]) -> Result<(), RsyncError>,
{
    let extra = match first {
        0x00..=0x7f => 0,
        0x80..=0xbf => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => {
            return Err(RsyncError::Protocol(
                "varint exceeds 32-bit range".to_owned(),
            ));
        }
    };
    if extra == 0 {
        return Ok(u32::from(first));
    }
    let mut bytes = [0u8; 4];
    read_exact(&mut bytes[..extra])?;
    let prefix = first & ((1u8 << (8 - extra)) - 1);
    if extra == 4 {
        if prefix != 0 {
            return Err(RsyncError::Protocol("varint overflow".to_owned()));
        }
    } else {
        bytes[extra] = prefix;
    }
    Ok(u32::from_le_bytes(bytes))
}

fn read_vstring_raw<R: Read>(reader: &mut R) -> Result<Vec<u8>, RsyncError> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first)?;
    let length = if first[0] & 0x80 == 0 {
        usize::from(first[0])
    } else {
        let mut low = [0u8; 1];
        reader.read_exact(&mut low)?;
        (usize::from(first[0] & 0x7f) << 8) | usize::from(low[0])
    };
    if length > MAX_PATH_BYTES {
        return Err(RsyncError::Protocol(format!(
            "negotiation string length {length} exceeds {MAX_PATH_BYTES}"
        )));
    }
    let mut value = vec![0u8; length];
    reader.read_exact(&mut value)?;
    Ok(value)
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn metadata_mode(metadata: &Metadata) -> u32 {
    metadata.mode()
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &Metadata) -> u32 {
    let permission = if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    };
    if metadata.is_dir() {
        0o040_000 | permission
    } else {
        0o100_000 | permission
    }
}

#[cfg(unix)]
fn metadata_mtime(metadata: &Metadata) -> i64 {
    metadata.mtime()
}

#[cfg(unix)]
fn metadata_mtime_nsec(metadata: &Metadata) -> u32 {
    u32::try_from(metadata.mtime_nsec()).unwrap_or(0)
}

#[cfg(not(unix))]
fn metadata_mtime(metadata: &Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn metadata_mtime_nsec(metadata: &Metadata) -> u32 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.subsec_nanos())
}

fn metadata_stable(before: &Metadata, after: &Metadata) -> bool {
    metadata_identity_matches(before, after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

fn open_source_file(path: &Path) -> Result<File, RsyncError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(path).map_err(|source| RsyncError::Source {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn metadata_identity_matches(before: &Metadata, after: &Metadata) -> bool {
    before.dev() == after.dev() && before.ino() == after.ino()
}

#[cfg(not(unix))]
fn metadata_identity_matches(_before: &Metadata, _after: &Metadata) -> bool {
    true
}

fn display_wire_path(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_probe_accepts_gnu_and_openrsync() {
        let gnu = parse_version_probe(b"rsync  version 3.4.4  protocol version 32\n").unwrap();
        assert_eq!(gnu.implementation, "GNU rsync");
        assert_eq!(gnu.version, "3.4.4");
        assert_eq!(gnu.max_protocol, 32);

        let open = parse_version_probe(
            b"openrsync: protocol version 29\nrsync version 2.6.9 compatible\n",
        )
        .unwrap();
        assert_eq!(open.implementation, "openrsync");
        assert_eq!(open.max_protocol, 29);
    }

    #[test]
    fn newer_gnu_protocol_is_accepted_for_protocol_32_sender() {
        let peer = RsyncPeer {
            implementation: "GNU rsync".to_owned(),
            version: "future".to_owned(),
            max_protocol: RSYNC_WIRE_VERSION + 1,
        };
        assert!(validate_peer(&peer).is_ok());
        let old = RsyncPeer {
            max_protocol: RSYNC_WIRE_VERSION - 1,
            ..peer
        };
        assert!(validate_peer(&old).is_err());
    }

    #[test]
    fn shell_quoting_blocks_command_injection() {
        assert_eq!(
            shell_quote(b"a b'; touch /tmp/pwn").unwrap(),
            b"'a b'\\''; touch /tmp/pwn'"
        );
        assert!(shell_quote(b"bad\0arg").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn shell_command_preserves_non_utf8_bytes() {
        assert_eq!(
            shell_command(&[b"rsync", b"name\xff"]).unwrap().as_bytes(),
            b"'rsync' 'name\xff'"
        );
    }

    #[test]
    fn shell_command_expands_remote_home_path() {
        assert_eq!(
            shell_command(&[b"rsync", b"--server", b"~"])
                .unwrap()
                .as_encoded_bytes(),
            b"'rsync' '--server' \"$HOME\""
        );
        assert_eq!(
            shell_command(&[b"rsync", b"--server", b"~/nested"])
                .unwrap()
                .as_encoded_bytes(),
            b"'rsync' '--server' \"$HOME\"/'nested'"
        );
    }

    #[test]
    fn excludes_filter_files_and_descendants_for_rsync_fallback() {
        let entry = |path: &[u8], kind: WireKind| WireEntry {
            source: PathBuf::from("source"),
            path: path.to_vec(),
            kind,
            size: 0,
            mtime: 0,
            mtime_nsec: 0,
            mode: 0o644,
            link_target: Vec::new(),
            top_level: false,
        };
        let entries = apply_excludes(
            vec![
                entry(b"keep.txt", WireKind::File),
                entry(b"skip", WireKind::Directory),
                entry(b"skip/child.txt", WireKind::File),
                entry(b"nested/debug.log", WireKind::File),
            ],
            &["skip".to_owned(), "*.log".to_owned()],
        )
        .unwrap();

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_slice())
                .collect::<Vec<_>>(),
            vec![b"keep.txt".as_slice()]
        );
    }

    #[test]
    fn rsync_fallback_accepts_dry_run() {
        let options = LocalSyncOptions {
            dry_run: true,
            ..LocalSyncOptions::default()
        };
        assert!(validate_options(&options).is_ok());
    }

    #[test]
    fn multiplex_reader_skips_info_and_reads_data() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(((7 + 2) << 24 | 4i32).to_le_bytes()));
        bytes.extend_from_slice(b"info");
        bytes.extend_from_slice(&((7 << 24 | 4i32).to_le_bytes()));
        bytes.extend_from_slice(&42i32.to_le_bytes());
        let mut cursor = io::Cursor::new(bytes);
        assert_eq!(MultiplexReader::new(&mut cursor).read_i32().unwrap(), 42);
    }

    #[test]
    fn multiplex_reader_rejects_error_and_oversized_frames() {
        let mut error = Vec::new();
        error.extend_from_slice(&(((7 + 1) << 24 | 4i32).to_le_bytes()));
        error.extend_from_slice(b"fail");
        let mut cursor = io::Cursor::new(error);
        assert!(matches!(
            MultiplexReader::new(&mut cursor).read_i32(),
            Err(RsyncError::RemoteExit { .. })
        ));

        let oversized = ((7u32 << 24)
            | (u32::try_from(MAX_MULTIPLEX_PAYLOAD).expect("test bound fits") + 1))
            .to_le_bytes();
        let mut cursor = io::Cursor::new(oversized);
        assert!(matches!(
            MultiplexReader::new(&mut cursor).read_i32(),
            Err(RsyncError::Protocol(_))
        ));
    }

    #[test]
    fn bounded_diagnostics_reject_excess() {
        let bytes = vec![0u8; MAX_DIAGNOSTIC_BYTES + 1];
        assert_eq!(
            read_bounded(io::Cursor::new(bytes)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_scan_preserves_non_utf8_names() {
        use std::os::unix::ffi::OsStringExt as _;

        let root = tempfile::tempdir().unwrap();
        let name = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
        fs::write(root.path().join(name), b"x").unwrap();
        let entries = scan_source(root.path(), true).unwrap();
        assert!(entries.iter().any(|entry| entry.path == [b'f', 0xff]));
    }

    /// The order below is what GNU rsync 3.4.4 itself produces for this tree
    /// (`rsync -rlptW --no-inc-recursive --protocol=32 -n --out-format=%n`).
    /// The receiver re-sorts whatever file list it is sent, so a sender that
    /// orders entries differently hands the receiver a different index space:
    /// the receiver then asks for data at an index the sender believes is a
    /// directory, and the transfer dies with "receiver requested data for
    /// non-file index". The deep tree matters -- shallow corpora happen to
    /// sort the same way under a naive comparator.
    #[test]
    fn deep_tree_file_list_matches_gnu_rsync_ordering() {
        let root = tempfile::tempdir().unwrap();
        for directory in [
            "bills/hr/1",
            "bills/hr/2",
            "bills/hr-extra",
            "bills/hjres/1",
            "bills/s/1",
            "committees",
            "votes/2023",
        ] {
            fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        for file in [
            "README.md",
            "zzz-top.txt",
            "bills/index.json",
            "bills/hr/index.json",
            "bills/hr/1/data.json",
            "bills/hr/1/text.txt",
            "bills/hr/2/data.json",
            "bills/hr-extra/note.txt",
            "bills/hjres/1/data.json",
            "bills/s/1/data.json",
            "committees/list.json",
            "votes/2023/v1.json",
        ] {
            fs::write(root.path().join(file), b"x").unwrap();
        }

        let entries = scan_source(root.path(), true).unwrap();
        let ordered: Vec<String> = entries
            .iter()
            .map(|entry| display_wire_path(&entry.path))
            .collect();
        assert_eq!(
            ordered,
            vec![
                // The transfer root leads the list.
                ".",
                // Every non-directory in a directory precedes its
                // subdirectories, however the names themselves compare.
                "README.md",
                "zzz-top.txt",
                "bills",
                "bills/index.json",
                "bills/hjres",
                "bills/hjres/1",
                "bills/hjres/1/data.json",
                // A directory compares as though its name ended in '/', so
                // "hr-extra" sorts ahead of "hr" ('-' is below '/').
                "bills/hr-extra",
                "bills/hr-extra/note.txt",
                "bills/hr",
                "bills/hr/index.json",
                "bills/hr/1",
                "bills/hr/1/data.json",
                "bills/hr/1/text.txt",
                "bills/hr/2",
                "bills/hr/2/data.json",
                "bills/s",
                "bills/s/1",
                "bills/s/1/data.json",
                "committees",
                "committees/list.json",
                "votes",
                "votes/2023",
                "votes/2023/v1.json",
            ]
        );
    }

    /// A directory entry is never a data-transfer candidate, so the indexes the
    /// receiver may legitimately request are exactly the regular-file indexes.
    #[test]
    fn file_list_indexes_of_regular_files_are_stable_under_the_wire_ordering() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("a/b/c")).unwrap();
        fs::create_dir_all(root.path().join("a/b-sibling")).unwrap();
        fs::write(root.path().join("a/b/c/deep.txt"), b"deep").unwrap();
        fs::write(root.path().join("a/b-sibling/near.txt"), b"near").unwrap();
        fs::write(root.path().join("top.txt"), b"top").unwrap();

        let entries = scan_source(root.path(), true).unwrap();
        let files: Vec<(usize, String)> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind == WireKind::File)
            .map(|(index, entry)| (index, display_wire_path(&entry.path)))
            .collect();
        assert_eq!(
            files,
            vec![
                (1, "top.txt".to_owned()),
                (4, "a/b-sibling/near.txt".to_owned()),
                (7, "a/b/c/deep.txt".to_owned()),
            ]
        );
    }
}
