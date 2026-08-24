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
