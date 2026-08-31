//! Transport selection and capability reporting shared by the engine and CLI.

/// User-selected remote transport policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    /// Prefer native xsync and use rsync only when xsync is unavailable.
    Auto,
    /// Require the native xsync receiver.
    Xsync,
    /// Require the native local rsync-wire codec and remote rsync receiver.
    Rsync,
}

/// Concrete transport selected for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// In-process local synchronization.
    Local,
    /// Native xsync framed protocol.
    Xsync,
    /// Native sender speaking the rsync wire protocol.
    Rsync,
}

impl TransportKind {
    /// Stable event/report spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Xsync => "xsync",
            Self::Rsync => "rsync",
        }
    }
}

/// Capabilities and guarantees of the selected transport.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportCapabilities {
    /// Parallel network streams are available.
    pub multi_stream: bool,
    /// Durable xsync range checkpoints are available.
    pub durable_resume: bool,
    /// BLAKE3 frames verify incoming payloads.
    pub blake3_frames: bool,
    /// Destination readback is available.
    pub paranoid_readback: bool,
    /// Whole-file transfer is available.
    pub whole_file: bool,
}

/// Observable result of selecting a remote transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportSelection {
    /// Selected backend.
    pub transport: TransportKind,
    /// Remote implementation family.
    pub remote_implementation: String,
    /// Remote program version, when the probe exposes it.
    pub remote_version: Option<String>,
    /// Negotiated wire protocol version.
    pub wire_version: u32,
    /// Backend capability matrix.
    pub capabilities: TransportCapabilities,
    /// User-visible options mapped by this backend.
    pub mapped_options: Vec<&'static str>,
    /// Negotiated whole-file verification checksum, when applicable.
    pub checksum_algorithm: Option<&'static str>,
    /// Negotiated compression algorithm, when applicable.
    pub compression_algorithm: Option<&'static str>,
    /// Guarantees unavailable on this backend.
    pub unavailable_guarantees: Vec<&'static str>,
    /// Why this backend was selected.
    pub reason: String,
}

/// Counts bytes crossing a transport, so a report can state what was actually
/// sent rather than what the frame accounting remembered to add up.
///
/// The frame-level counters this replaces were maintained by hand at each
/// write site, and every site that was missed -- all four metadata paths --
/// silently understated the total. Counting at the boundary cannot drift as
/// new message types are added, because it does not know about message types.
pub struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W> CountingWriter<W> {
    /// Wrap a writer, counting from zero.
    pub const fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }

    /// Bytes written so far.
    pub const fn byte_count(&self) -> u64 {
        self.bytes
    }
}

impl<W: std::io::Write> std::io::Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.bytes = self.bytes.saturating_add(count as u64);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Counts bytes read from a transport. The pull direction's payload arrives
/// inbound, so that is where its wire total has to be measured.
pub struct CountingReader<R> {
    inner: R,
    bytes: u64,
}

impl<R> CountingReader<R> {
    /// Wrap a reader, counting from zero.
    pub const fn new(inner: R) -> Self {
        Self { inner, bytes: 0 }
    }

    /// Bytes read so far.
    ///
    /// Named `byte_count` rather than `bytes` because `Read::bytes` already
    /// exists and would shadow it at every call site.
    pub const fn byte_count(&self) -> u64 {
        self.bytes
    }
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.bytes = self.bytes.saturating_add(count as u64);
        Ok(count)
    }
}
