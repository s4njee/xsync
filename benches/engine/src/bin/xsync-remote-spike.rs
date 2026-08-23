use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand, ValueEnum};
use filetime::{set_file_times, FileTime};
use serde::{Deserialize, Serialize};
use xsync_bench::manifest::build_manifest;

const FRAME_MAGIC: &[u8; 4] = b"xrb1";
const FRAME_HEADER_LEN: usize = 14;
const FRAME_HEADER_BYTES: u64 = 14;
const FRAME_HANDSHAKE: u8 = 1;
const FRAME_FILE_START: u8 = 2;
const FRAME_DATA: u8 = 3;
const FRAME_FILE_END: u8 = 4;
const FRAME_SESSION_END: u8 = 5;
const FLAG_ZSTD: u8 = 1;
const MAX_FRAME_PAYLOAD: usize = 2 * 1024 * 1024;
const DATA_CHUNK_BYTES: usize = 1024 * 1024;
const ACK: &[u8; 4] = b"ack1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompressionMode {
    None,
    Adaptive,
}

impl CompressionMode {
    const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Adaptive => "adaptive-zstd-3",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "xsync-remote-spike",
    about = "Story 0.5 bounded framed-transfer and compression spike"
)]
struct Cli {
    #[command(subcommand)]
    command: SpikeCommand,
}

#[derive(Debug, Subcommand)]
enum SpikeCommand {
    /// Send a flat regular-file corpus over persistent SSH data sessions.
    Send {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        remote_binary: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=16))]
        streams: u16,
        #[arg(long, value_enum, default_value_t = CompressionMode::None)]
        compression: CompressionMode,
        #[arg(long, default_value_t = xsync_core::DEFAULT_COMPRESSION_SAMPLE_BYTES)]
        sample_bytes: usize,
        /// Trusted benchmark-only receiver prefix, for example `taskset -c 0`.
        #[arg(long)]
        receiver_prefix: Option<String>,
    },
    /// Hidden receiver for one persistent framed stream.
    #[command(hide = true)]
    Receive {
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        stream: u16,
        #[arg(long)]
        streams: u16,
        #[arg(long, value_enum)]
        compression: CompressionMode,
        #[arg(long)]
        sample_bytes: usize,
    },
    /// Emit the independent manifest as one JSON document.
    Manifest {
        #[arg(long)]
        root: PathBuf,
    },
    /// Create an exact benchmark destination parent.
    Prepare {
        #[arg(long)]
        root: PathBuf,
    },
    /// Print a setup marker then replace this process with reference rsync.
    #[command(hide = true, trailing_var_arg = true)]
    RsyncWrapper {
        #[arg(allow_hyphen_values = true)]
        arguments: Vec<OsString>,
    },
    /// Measure adaptive compression sampling without transport effects.
    CompressionProbe {
        /// Repeated `LABEL=PATH` corpus roots.
        #[arg(long = "corpus", value_parser = parse_labeled_path)]
        corpora: Vec<LabeledPath>,
        #[arg(long = "sample-bytes", value_delimiter = ',', default_values_t = [64 * 1024, 256 * 1024, 1024 * 1024])]
        sample_bytes: Vec<usize>,
        #[arg(long)]
        json: PathBuf,
        #[arg(long)]
        markdown: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct LabeledPath {
    label: String,
    path: PathBuf,
}

fn parse_labeled_path(value: &str) -> Result<LabeledPath, String> {
    let (label, path) = value
        .split_once('=')
        .ok_or_else(|| "corpus must be LABEL=PATH".to_owned())?;
    if label.is_empty() || path.is_empty() {
        return Err("corpus label and path must be non-empty".to_owned());
    }
    Ok(LabeledPath {
        label: label.to_owned(),
        path: PathBuf::from(path),
    })
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xsync-remote-spike: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), SpikeError> {
    match cli.command {
        SpikeCommand::Send {
            source,
            host,
            remote_binary,
            destination,
            streams,
            compression,
            sample_bytes,
            receiver_prefix,
        } => {
            let result = send(&SendOptions {
                source,
                host,
                remote_binary,
                destination,
                streams,
                compression,
                sample_bytes,
                receiver_prefix,
            })?;
            serde_json::to_writer(std::io::stdout().lock(), &result)?;
            println!();
        }
        SpikeCommand::Receive {
            destination,
            stream,
            streams,
            compression,
            sample_bytes,
        } => receive(&destination, stream, streams, compression, sample_bytes)?,
        SpikeCommand::Manifest { root } => {
            serde_json::to_writer(std::io::stdout().lock(), &build_manifest(root)?)?;
            println!();
        }
        SpikeCommand::Prepare { root } => {
            fs::create_dir_all(&root)
                .map_err(|error| path_io("create benchmark destination parent", &root, error))?;
        }
        SpikeCommand::RsyncWrapper { arguments } => rsync_wrapper(&arguments)?,
        SpikeCommand::CompressionProbe {
            corpora,
            sample_bytes,
            json,
            markdown,
        } => compression_probe(&corpora, &sample_bytes, &json, &markdown)?,
    }
    Ok(())
}

#[derive(Debug)]
struct SendOptions {
    source: PathBuf,
    host: String,
    remote_binary: PathBuf,
    destination: PathBuf,
    streams: u16,
    compression: CompressionMode,
    sample_bytes: usize,
    receiver_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SendResult {
    schema: String,
    framing: String,
    streams: u16,
    compression: String,
    sample_bytes: usize,
    item_count: u64,
    logical_bytes: u64,
    wire_bytes: u64,
    setup_seconds: f64,
    transfer_seconds: f64,
    teardown_seconds: f64,
    wall_seconds: f64,
    source_manifest_digest: String,
    receiver_summaries: Vec<ReceiveSummary>,
}

#[derive(Debug, Clone)]
struct FileSpec {
    source: PathBuf,
    relative: String,
    length: u64,
    mode: u32,
    mtime_seconds: i64,
    mtime_nanoseconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RootInfo {
    mode: u32,
    mtime_seconds: i64,
    mtime_nanoseconds: u32,
    stream: u16,
    streams: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileInfo {
    relative: String,
    length: u64,
    mode: u32,
    mtime_seconds: i64,
    mtime_nanoseconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReceiveSummary {
    stream: u16,
    files: u64,
    logical_bytes: u64,
    wire_bytes: u64,
    compressed_frames: u64,
    raw_frames: u64,
}

struct SenderSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    initial_wire_bytes: u64,
}

fn send(options: &SendOptions) -> Result<SendResult, SpikeError> {
    if options.sample_bytes == 0 || options.sample_bytes > DATA_CHUNK_BYTES {
        return Err(SpikeError::InvalidSampleSize(options.sample_bytes));
    }
    let source = fs::canonicalize(&options.source)
        .map_err(|error| path_io("canonicalize source", &options.source, error))?;
    let (files, root_info) = scan_flat_corpus(&source, options.streams)?;
    let manifest = build_manifest(&source)?;
    let logical_bytes = files.iter().map(|file| file.length).sum();
    let wall_start = Instant::now();
    let setup_start = Instant::now();
    let mut sessions = (0..options.streams)
        .map(|stream| spawn_receiver(options, stream))
        .collect::<Result<Vec<_>, _>>()?;
    for (stream, session) in sessions.iter_mut().enumerate() {
        let mut info = root_info.clone();
        info.stream = u16::try_from(stream).expect("stream index fits u16");
        let payload = serde_json::to_vec(&info)?;
        session.initial_wire_bytes = write_frame(
            session
                .stdin
                .as_mut()
                .ok_or(SpikeError::MissingPipe("stdin"))?,
            FRAME_HANDSHAKE,
            0,
            u32_len(&payload)?,
            &payload,
        )?;
        session
            .stdin
            .as_mut()
            .ok_or(SpikeError::MissingPipe("stdin"))?
            .flush()?;
    }
    for session in &mut sessions {
        let mut ack = [0_u8; 4];
        session.stdout.read_exact(&mut ack)?;
        if ack != *ACK {
            return Err(SpikeError::BadAck);
        }
    }
    let setup_seconds = setup_start.elapsed().as_secs_f64();
    let transfer_start = Instant::now();
    let mut assignments = vec![Vec::new(); usize::from(options.streams)];
    let assignment_count = assignments.len();
    for (index, file) in files.iter().cloned().enumerate() {
        assignments[index % assignment_count].push(file);
    }
    let mode = options.compression;
    let sample_bytes = options.sample_bytes;
    let writers = sessions
        .iter_mut()
        .zip(assignments)
        .map(|(session, files)| {
            let stdin = session
                .stdin
                .take()
                .expect("receiver stdin exists until its sender starts");
            let initial = session.initial_wire_bytes;
            thread::spawn(move || send_files(stdin, &files, mode, sample_bytes, initial))
        })
        .collect::<Vec<_>>();
    let mut wire_bytes = 0_u64;
    for writer in writers {
        wire_bytes = wire_bytes
            .checked_add(writer.join().map_err(|_| SpikeError::WorkerPanic)??)
            .ok_or(SpikeError::WireOverflow)?;
    }
    let transfer_seconds = transfer_start.elapsed().as_secs_f64();
    let teardown_start = Instant::now();
    let mut receiver_summaries = Vec::with_capacity(sessions.len());
    for mut session in sessions {
        let status = session.child.wait()?;
        let mut output = String::new();
        session.stdout.read_to_string(&mut output)?;
        if !status.success() {
            return Err(SpikeError::ReceiverFailed {
                status: status.code(),
                output,
            });
        }
        receiver_summaries.push(serde_json::from_str(output.trim())?);
    }
    let teardown_seconds = teardown_start.elapsed().as_secs_f64();
    Ok(SendResult {
        schema: "xsync.remote-spike.send.v1".to_owned(),
        framing: "xsync-story-0.5-spike-v1; 14-byte bounded frame header; 1 MiB data payload"
            .to_owned(),
        streams: options.streams,
        compression: options.compression.label().to_owned(),
        sample_bytes: options.sample_bytes,
        item_count: manifest.item_count,
        logical_bytes,
        wire_bytes,
        setup_seconds,
        transfer_seconds,
        teardown_seconds,
        wall_seconds: wall_start.elapsed().as_secs_f64(),
        source_manifest_digest: manifest.manifest_digest,
        receiver_summaries,
    })
}

fn spawn_receiver(options: &SendOptions, stream: u16) -> Result<SenderSession, SpikeError> {
    let mut command = Command::new("ssh");
    command.args(["-o", "BatchMode=yes", &options.host]);
    if let Some(prefix) = &options.receiver_prefix {
        command.args(prefix.split_whitespace());
    }
    command
        .arg(&options.remote_binary)
        .arg("receive")
        .arg("--destination")
        .arg(&options.destination)
        .arg("--stream")
        .arg(stream.to_string())
        .arg("--streams")
        .arg(options.streams.to_string())
        .arg("--compression")
        .arg(
            options
                .compression
                .label()
                .split('-')
                .next()
                .unwrap_or("none"),
        )
        .arg("--sample-bytes")
        .arg(options.sample_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or(SpikeError::MissingPipe("stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(SpikeError::MissingPipe("stdout"))?;
    Ok(SenderSession {
        child,
        stdin: Some(stdin),
        stdout,
        initial_wire_bytes: 0,
    })
}

fn send_files(
    stdin: ChildStdin,
    files: &[FileSpec],
    mode: CompressionMode,
    sample_bytes: usize,
    initial_wire_bytes: u64,
) -> Result<u64, SpikeError> {
    let mut writer = BufWriter::with_capacity(DATA_CHUNK_BYTES, stdin);
    let mut wire_bytes = initial_wire_bytes;
    for file in files {
        let info = FileInfo {
            relative: file.relative.clone(),
            length: file.length,
            mode: file.mode,
            mtime_seconds: file.mtime_seconds,
            mtime_nanoseconds: file.mtime_nanoseconds,
        };
        let metadata = serde_json::to_vec(&info)?;
        wire_bytes += write_frame(
            &mut writer,
            FRAME_FILE_START,
            0,
            u32_len(&metadata)?,
            &metadata,
        )?;
        let mut input = File::open(&file.source)
            .map_err(|error| path_io("open framed source", &file.source, error))?;
        let compress = should_compress(&mut input, sample_bytes, mode)?;
        input.seek(SeekFrom::Start(0))?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0_u8; DATA_CHUNK_BYTES];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            let (flags, payload) = encode_data(&buffer[..count], compress)?;
            wire_bytes += write_frame(
                &mut writer,
                FRAME_DATA,
                flags,
                u32::try_from(count).map_err(|_| SpikeError::FrameTooLarge(count))?,
                &payload,
            )?;
        }
        wire_bytes += write_frame(
            &mut writer,
            FRAME_FILE_END,
            0,
            32,
            hasher.finalize().as_bytes(),
        )?;
    }
    wire_bytes += write_frame(&mut writer, FRAME_SESSION_END, 0, 0, &[])?;
    writer.flush()?;
    Ok(wire_bytes)
}

fn receive(
    destination: &Path,
    stream: u16,
    streams: u16,
    compression: CompressionMode,
    sample_bytes: usize,
) -> Result<(), SpikeError> {
    if streams == 0 || stream >= streams || sample_bytes == 0 {
        return Err(SpikeError::InvalidReceiverConfig);
    }
    let input = std::io::stdin().lock();
    let mut reader = BufReader::with_capacity(DATA_CHUNK_BYTES, input);
    let handshake = read_frame(&mut reader)?;
    if handshake.kind != FRAME_HANDSHAKE {
        return Err(SpikeError::UnexpectedFrame(handshake.kind));
    }
    let root: RootInfo = serde_json::from_slice(&handshake.payload)?;
    if root.stream != stream || root.streams != streams {
        return Err(SpikeError::InvalidReceiverConfig);
    }
    fs::create_dir_all(destination)
        .map_err(|error| path_io("create receiver destination", destination, error))?;
    std::io::stdout().write_all(ACK)?;
    std::io::stdout().flush()?;
    let mut summary = ReceiveSummary {
        stream,
        files: 0,
        logical_bytes: 0,
        wire_bytes: handshake.wire_bytes + u64::try_from(ACK.len()).unwrap_or(0),
        compressed_frames: 0,
        raw_frames: 0,
    };
    let mut current: Option<ReceivingFile> = None;
    loop {
        let frame = read_frame(&mut reader)?;
        summary.wire_bytes = summary
            .wire_bytes
            .checked_add(frame.wire_bytes)
            .ok_or(SpikeError::WireOverflow)?;
        match frame.kind {
            FRAME_FILE_START if current.is_none() => {
                let info: FileInfo = serde_json::from_slice(&frame.payload)?;
                current = Some(ReceivingFile::create(destination, info)?);
            }
            FRAME_DATA => {
                let receiving = current.as_mut().ok_or(SpikeError::DataWithoutFile)?;
                let decoded = if frame.flags & FLAG_ZSTD == FLAG_ZSTD {
                    summary.compressed_frames += 1;
                    zstd::bulk::decompress(&frame.payload, frame.raw_len as usize)?
                } else {
                    summary.raw_frames += 1;
                    if frame.payload.len() != frame.raw_len as usize {
                        return Err(SpikeError::DeclaredLengthMismatch);
                    }
                    frame.payload
                };
                if decoded.len() != frame.raw_len as usize {
                    return Err(SpikeError::DeclaredLengthMismatch);
                }
                receiving.write(&decoded)?;
            }
            FRAME_FILE_END => {
                let receiving = current.take().ok_or(SpikeError::DataWithoutFile)?;
                if frame.payload.len() != 32 {
                    return Err(SpikeError::DeclaredLengthMismatch);
                }
                let length = receiving.finish(&frame.payload)?;
                summary.files += 1;
                summary.logical_bytes += length;
            }
            FRAME_SESSION_END if current.is_none() => break,
            kind => return Err(SpikeError::UnexpectedFrame(kind)),
        }
    }
    apply_mode_time(
        destination,
        root.mode,
        root.mtime_seconds,
        root.mtime_nanoseconds,
    )?;
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &summary)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    let _ = compression;
    Ok(())
}

struct ReceivingFile {
    info: FileInfo,
    stage: PathBuf,
    target: PathBuf,
    output: BufWriter<File>,
    hasher: blake3::Hasher,
    written: u64,
}

impl ReceivingFile {
    fn create(destination: &Path, info: FileInfo) -> Result<Self, SpikeError> {
        let target = safe_target(destination, &info.relative)?;
        let parent = target.parent().ok_or(SpikeError::UnsafePath)?;
        fs::create_dir_all(parent)
            .map_err(|error| path_io("create receiver parent", parent, error))?;
        let stage = parent.join(format!(
            ".xsync.remote-spike.{}",
            blake3::hash(info.relative.as_bytes()).to_hex()
        ));
        match fs::remove_file(&stage) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(path_io("remove stale receiver stage", &stage, error)),
        }
        let output = BufWriter::with_capacity(
            DATA_CHUNK_BYTES,
            File::create(&stage)
                .map_err(|error| path_io("create receiver stage", &stage, error))?,
        );
        Ok(Self {
            info,
            stage,
            target,
            output,
            hasher: blake3::Hasher::new(),
            written: 0,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SpikeError> {
        let count = u64::try_from(bytes.len()).map_err(|_| SpikeError::WireOverflow)?;
        self.written = self
            .written
            .checked_add(count)
            .ok_or(SpikeError::WireOverflow)?;
        if self.written > self.info.length {
            return Err(SpikeError::DeclaredLengthMismatch);
        }
        self.hasher.update(bytes);
        self.output.write_all(bytes)?;
        Ok(())
    }

    fn finish(mut self, expected_hash: &[u8]) -> Result<u64, SpikeError> {
        self.output.flush()?;
        drop(self.output);
        if self.written != self.info.length || self.hasher.finalize().as_bytes() != expected_hash {
            let _ = fs::remove_file(&self.stage);
            return Err(SpikeError::ContentMismatch(self.info.relative));
        }
        apply_mode_time(
            &self.stage,
            self.info.mode,
            self.info.mtime_seconds,
            self.info.mtime_nanoseconds,
        )?;
        fs::rename(&self.stage, &self.target)
            .map_err(|error| path_io("publish receiver stage", &self.target, error))?;
        Ok(self.written)
    }
}

fn scan_flat_corpus(root: &Path, streams: u16) -> Result<(Vec<FileSpec>, RootInfo), SpikeError> {
    let root_metadata =
        fs::metadata(root).map_err(|error| path_io("inspect source root", root, error))?;
    if !root_metadata.is_dir() {
        return Err(SpikeError::UnsupportedCorpus(
            "source root must be a directory".to_owned(),
        ));
    }
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut children = fs::read_dir(&directory)
            .map_err(|error| path_io("read source corpus", &directory, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| path_io("read source entry", &directory, error))?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| path_io("inspect source entry", &path, error))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                if path != *root {
                    return Err(SpikeError::UnsupportedCorpus(
                        "framing spike intentionally accepts flat regular-file corpora only"
                            .to_owned(),
                    ));
                }
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| SpikeError::UnsafePath)?
                    .to_str()
                    .ok_or_else(|| {
                        SpikeError::UnsupportedCorpus(
                            "Story 2.1b raw path codec is not implemented yet".to_owned(),
                        )
                    })?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                files.push(FileSpec {
                    source: path,
                    relative,
                    length: metadata.len(),
                    mode: metadata.mode() & 0o7777,
                    mtime_seconds: metadata.mtime(),
                    mtime_nanoseconds: u32::try_from(metadata.mtime_nsec()).unwrap_or(0),
                });
            } else {
                return Err(SpikeError::UnsupportedCorpus(
                    "framing spike intentionally accepts regular files only".to_owned(),
                ));
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok((
        files,
        RootInfo {
            mode: root_metadata.mode() & 0o7777,
            mtime_seconds: root_metadata.mtime(),
            mtime_nanoseconds: u32::try_from(root_metadata.mtime_nsec()).unwrap_or(0),
            stream: 0,
            streams,
        },
    ))
}

fn should_compress(
    input: &mut File,
    sample_bytes: usize,
    mode: CompressionMode,
) -> Result<bool, SpikeError> {
    if mode == CompressionMode::None {
        return Ok(false);
    }
    let mut sample = vec![0_u8; sample_bytes];
    let count = input.read(&mut sample)?;
    if count == 0 {
        return Ok(false);
    }
    let compressed = zstd::bulk::compress(&sample[..count], 3)?;
    Ok(compressed.len().saturating_mul(100)
        <= count.saturating_mul(usize::from(
            xsync_core::DEFAULT_COMPRESSION_THRESHOLD_PERCENT,
        )))
}

fn encode_data(bytes: &[u8], compress: bool) -> Result<(u8, Vec<u8>), SpikeError> {
    if !compress {
        return Ok((0, bytes.to_vec()));
    }
    let compressed = zstd::bulk::compress(bytes, 3)?;
    if compressed.len() < bytes.len() {
        Ok((FLAG_ZSTD, compressed))
    } else {
        Ok((0, bytes.to_vec()))
    }
}

struct Frame {
    kind: u8,
    flags: u8,
    raw_len: u32,
    payload: Vec<u8>,
    wire_bytes: u64,
}

fn write_frame(
    writer: &mut impl Write,
    kind: u8,
    flags: u8,
    raw_len: u32,
    payload: &[u8],
) -> Result<u64, SpikeError> {
    if payload.len() > MAX_FRAME_PAYLOAD {
        return Err(SpikeError::FrameTooLarge(payload.len()));
    }
    let payload_len = u32_len(payload)?;
    writer.write_all(FRAME_MAGIC)?;
    writer.write_all(&[kind, flags])?;
    writer.write_all(&payload_len.to_be_bytes())?;
    writer.write_all(&raw_len.to_be_bytes())?;
    writer.write_all(payload)?;
    Ok(FRAME_HEADER_BYTES + u64::from(payload_len))
}

fn read_frame(reader: &mut impl Read) -> Result<Frame, SpikeError> {
    let mut header = [0_u8; FRAME_HEADER_LEN];
    reader.read_exact(&mut header)?;
    if &header[..4] != FRAME_MAGIC {
        return Err(SpikeError::BadMagic);
    }
    let kind = header[4];
    let flags = header[5];
    let payload_len = u32::from_be_bytes(header[6..10].try_into().expect("four bytes"));
    let raw_len = u32::from_be_bytes(header[10..14].try_into().expect("four bytes"));
    if payload_len as usize > MAX_FRAME_PAYLOAD || raw_len as usize > DATA_CHUNK_BYTES {
        return Err(SpikeError::FrameTooLarge(payload_len as usize));
    }
    let mut payload = vec![0_u8; payload_len as usize];
    reader.read_exact(&mut payload)?;
    Ok(Frame {
        kind,
        flags,
        raw_len,
        payload,
        wire_bytes: FRAME_HEADER_BYTES + u64::from(payload_len),
    })
}

fn safe_target(root: &Path, relative: &str) -> Result<PathBuf, SpikeError> {
    let path = Path::new(relative);
    if path.is_absolute() || relative.is_empty() {
        return Err(SpikeError::UnsafePath);
    }
    let mut target = root.to_path_buf();
    for component in path.components() {
        match component {
            Component::Normal(value) => target.push(value),
            _ => return Err(SpikeError::UnsafePath),
        }
    }
    Ok(target)
}

fn apply_mode_time(
    path: &Path,
    mode: u32,
    seconds: i64,
    nanoseconds: u32,
) -> Result<(), SpikeError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| path_io("set receiver permissions", path, error))?;
    let time = FileTime::from_unix_time(seconds, nanoseconds);
    set_file_times(path, time, time).map_err(|error| path_io("set receiver times", path, error))
}

fn rsync_wrapper(arguments: &[OsString]) -> Result<(), SpikeError> {
    eprintln!("XSYNC_RSYNC_SERVER_READY");
    std::io::stderr().flush()?;
    let error = Command::new("rsync").args(arguments).exec();
    Err(SpikeError::Io(error))
}

#[derive(Debug, Clone, Serialize)]
struct CompressionProbeReport {
    schema: String,
    generated_unix_nanos: u128,
    zstd_version: String,
    threshold: String,
    observations: Vec<CompressionObservation>,
}

#[derive(Debug, Clone, Serialize)]
struct CompressionObservation {
    corpus: String,
    manifest_digest: String,
    sample_bytes: usize,
    item_count: u64,
    logical_bytes: u64,
    selected_files: u64,
    raw_wire_bytes: u64,
    adaptive_wire_bytes: u64,
    adaptive_overhead_ratio: f64,
}

fn compression_probe(
    corpora: &[LabeledPath],
    sample_sizes: &[usize],
    json: &Path,
    markdown: &Path,
) -> Result<(), SpikeError> {
    if corpora.is_empty() || sample_sizes.is_empty() {
        return Err(SpikeError::EmptyProbe);
    }
    let mut observations = Vec::new();
    for corpus in corpora {
        let root = fs::canonicalize(&corpus.path)
            .map_err(|error| path_io("canonicalize compression corpus", &corpus.path, error))?;
        let files = scan_compression_files(&root)?;
        let manifest = build_manifest(&root)?;
        for &sample_bytes in sample_sizes {
            if sample_bytes == 0 || sample_bytes > DATA_CHUNK_BYTES {
                return Err(SpikeError::InvalidSampleSize(sample_bytes));
            }
            let mut raw_wire_bytes = 0_u64;
            let mut adaptive_wire_bytes = 0_u64;
            let mut selected_files = 0_u64;
            for file in &files {
                let mut input = File::open(&file.source)?;
                let selected =
                    should_compress(&mut input, sample_bytes, CompressionMode::Adaptive)?;
                selected_files += u64::from(selected);
                input.seek(SeekFrom::Start(0))?;
                let mut buffer = vec![0_u8; DATA_CHUNK_BYTES];
                loop {
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    raw_wire_bytes += FRAME_HEADER_BYTES + u64::try_from(count).unwrap_or(u64::MAX);
                    let (_, payload) = encode_data(&buffer[..count], selected)?;
                    adaptive_wire_bytes +=
                        FRAME_HEADER_BYTES + u64::try_from(payload.len()).unwrap_or(u64::MAX);
                }
            }
            observations.push(CompressionObservation {
                corpus: corpus.label.clone(),
                manifest_digest: manifest.manifest_digest.clone(),
                sample_bytes,
                item_count: manifest.item_count,
                logical_bytes: manifest.logical_bytes,
                selected_files,
                raw_wire_bytes,
                adaptive_wire_bytes,
                adaptive_overhead_ratio: ratio_u64(adaptive_wire_bytes, raw_wire_bytes),
            });
        }
    }
    let report = CompressionProbeReport {
        schema: "xsync.compression-probe.report.v1".to_owned(),
        generated_unix_nanos: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SpikeError::Clock)?
            .as_nanos(),
        zstd_version: zstd::zstd_safe::version_string().to_owned(),
        threshold: "select zstd level 3 when bounded sample output is <= 95% of input".to_owned(),
        observations,
    };
    write_json(json, &report)?;
    write_compression_markdown(markdown, &report)
}

fn scan_compression_files(root: &Path) -> Result<Vec<FileSpec>, SpikeError> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for child in fs::read_dir(&directory)
            .map_err(|error| path_io("read compression corpus", &directory, error))?
        {
            let child =
                child.map_err(|error| path_io("read compression entry", &directory, error))?;
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| path_io("inspect compression entry", &path, error))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push(FileSpec {
                    relative: path
                        .strip_prefix(root)
                        .map_err(|_| SpikeError::UnsafePath)?
                        .to_string_lossy()
                        .into_owned(),
                    source: path,
                    length: metadata.len(),
                    mode: metadata.mode() & 0o7777,
                    mtime_seconds: metadata.mtime(),
                    mtime_nanoseconds: u32::try_from(metadata.mtime_nsec()).unwrap_or(0),
                });
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn write_compression_markdown(
    path: &Path,
    report: &CompressionProbeReport,
) -> Result<(), SpikeError> {
    let mut output = format!(
        "# xsync compression sampling spike\n\n- Schema: `{}`\n- zstd: `{}`\n- Rule: {}\n\n| Corpus | Sample | Selected files | Logical | Raw wire | Adaptive wire | Ratio |\n|---|---:|---:|---:|---:|---:|---:|\n",
        report.schema, report.zstd_version, report.threshold
    );
    for observation in &report.observations {
        use std::fmt::Write as _;
        writeln!(
            &mut output,
            "| {} | {} | {} | {} | {} | {} | {:.5}x |",
            observation.corpus,
            observation.sample_bytes,
            observation.selected_files,
            observation.logical_bytes,
            observation.raw_wire_bytes,
            observation.adaptive_wire_bytes,
            observation.adaptive_overhead_ratio
        )
        .expect("writing to String cannot fail");
    }
    atomic_write(path, output.as_bytes())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), SpikeError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SpikeError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| path_io("create report directory", parent, error))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("xsync-remote-spike"),
        std::process::id()
    ));
    fs::write(&temporary, bytes)
        .map_err(|error| path_io("write temporary report", &temporary, error))?;
    fs::rename(&temporary, path).map_err(|error| path_io("publish report", path, error))
}

fn u32_len(bytes: &[u8]) -> Result<u32, SpikeError> {
    u32::try_from(bytes.len()).map_err(|_| SpikeError::FrameTooLarge(bytes.len()))
}

fn ratio_u64(mut numerator: u64, mut denominator: u64) -> f64 {
    while numerator > u64::from(u32::MAX) || denominator > u64::from(u32::MAX) {
        numerator /= 2;
        denominator /= 2;
    }
    f64::from(u32::try_from(numerator).unwrap_or(u32::MAX))
        / f64::from(u32::try_from(denominator).unwrap_or(u32::MAX))
}

fn path_io(operation: &'static str, path: &Path, source: io::Error) -> SpikeError {
    SpikeError::PathIo {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, thiserror::Error)]
enum SpikeError {
    #[error("cannot {operation} '{}': {source}", path.display())]
    PathIo {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manifest failure: {0}")]
    Manifest(#[from] xsync_bench::manifest::ManifestError),
    #[error("invalid compression sample size {0}")]
    InvalidSampleSize(usize),
    #[error("receiver configuration is invalid")]
    InvalidReceiverConfig,
    #[error("benchmark framing spike supports a narrower corpus: {0}")]
    UnsupportedCorpus(String),
    #[error("frame payload is too large: {0} bytes")]
    FrameTooLarge(usize),
    #[error("framing magic mismatch")]
    BadMagic,
    #[error("receiver acknowledgement mismatch")]
    BadAck,
    #[error("unexpected frame type {0}")]
    UnexpectedFrame(u8),
    #[error("data frame arrived without an open file")]
    DataWithoutFile,
    #[error("declared and decoded lengths differ")]
    DeclaredLengthMismatch,
    #[error("unsafe relative destination path")]
    UnsafePath,
    #[error("content verification failed for '{0}'")]
    ContentMismatch(String),
    #[error("receiver pipe '{0}' was not available")]
    MissingPipe(&'static str),
    #[error("receiver failed with status {status:?}: {output}")]
    ReceiverFailed { status: Option<i32>, output: String },
    #[error("sender worker panicked")]
    WorkerPanic,
    #[error("wire byte count overflow")]
    WireOverflow,
    #[error("compression probe needs at least one corpus and sample size")]
    EmptyProbe,
    #[error("system clock is before the Unix epoch")]
    Clock,
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn frame_round_trip_and_bounds() {
        let mut bytes = Vec::new();
        let written = write_frame(&mut bytes, FRAME_DATA, FLAG_ZSTD, 12, b"payload").unwrap();
        assert_eq!(written, FRAME_HEADER_BYTES + 7);
        let frame = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(frame.kind, FRAME_DATA);
        assert_eq!(frame.flags, FLAG_ZSTD);
        assert_eq!(frame.raw_len, 12);
        assert_eq!(frame.payload, b"payload");
    }

    #[test]
    fn path_containment_rejects_escape_and_absolute_forms() {
        let root = Path::new("/safe");
        assert_eq!(safe_target(root, "a/b").unwrap(), Path::new("/safe/a/b"));
        for invalid in ["", "../escape", "/absolute", "a/../../escape", "./dot"] {
            assert!(safe_target(root, invalid).is_err());
        }
    }

    #[test]
    fn adaptive_sampling_selects_repetition_and_rejects_random_data() {
        let temp = tempdir().unwrap();
        let compressible = temp.path().join("compressible");
        let random = temp.path().join("random");
        fs::write(&compressible, vec![b'x'; 128 * 1024]).unwrap();
        let mut random_bytes = vec![0_u8; 128 * 1024];
        for (index, byte) in random_bytes.iter_mut().enumerate() {
            *byte = blake3::hash(&index.to_le_bytes()).as_bytes()[0];
        }
        fs::write(&random, random_bytes).unwrap();
        assert!(should_compress(
            &mut File::open(compressible).unwrap(),
            64 * 1024,
            CompressionMode::Adaptive
        )
        .unwrap());
        assert!(!should_compress(
            &mut File::open(random).unwrap(),
            64 * 1024,
            CompressionMode::Adaptive
        )
        .unwrap());
    }
}
