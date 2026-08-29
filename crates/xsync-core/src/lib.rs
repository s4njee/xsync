//! `xsync-core` — the xsync engine library.
//!
//! This crate holds the engine independent of any CLI or transport:
//! the parallel scanner, planner/diff classifier, transfer strategies,
//! verified write path, the wire protocol, transports, and the event stream.
//!
//! The binary crate (`xsync`) and the v2 daemon are thin frontends over the
//! `Event` stream this library emits — see [`crate::events`].
//!
//! Wiring is scaffolded here in milestone 1.1; engine modules land as the
//! Epic 2 stories are implemented.

//! xsync-core's public modules.
pub mod bootstrap;
pub mod clone;
pub mod cloud;
pub mod compression;
pub mod faillog;
pub mod filter;
pub mod hash_cache;
pub mod journal;
pub mod local;
pub mod path;
pub mod pathsem;
pub mod planner;
pub mod protocol;
pub mod protocol_v2;
pub mod rsync;
pub mod scanner;
pub mod server;
pub mod sink;
pub mod source;
pub mod sparse;
pub mod strategy;
pub mod transport;

/// The version of the xsync wire protocol. Bumped on any incompatible change;
/// `--server` peers reject a mismatch (see plan.md Story 3.1).
pub const PROTOCOL_VERSION: u32 = 2;

/// Magic bytes prefixing every handshake frame (see plan.md Story 3.1).
pub const HANDSHAKE_MAGIC: &[u8; 4] = b"xsn1";

/// Story 0.5 remote-stream default. Explicit user values remain authoritative.
pub const DEFAULT_REMOTE_STREAMS: u8 = 1;

/// Story 0.5 bounded sample selected for adaptive compression.
pub const DEFAULT_COMPRESSION_SAMPLE_BYTES: usize = 64 * 1024;

/// Compress when the bounded sample is no larger than this percentage of input.
pub const DEFAULT_COMPRESSION_THRESHOLD_PERCENT: u8 = 95;

/// Return the current crate version (used for diagnostics).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_and_magic_are_stable() {
        assert_eq!(PROTOCOL_VERSION, 2);
        assert_eq!(HANDSHAKE_MAGIC, b"xsn1");
    }

    #[test]
    fn evidence_selected_defaults_are_stable() {
        assert_eq!(DEFAULT_REMOTE_STREAMS, 1);
        assert_eq!(DEFAULT_COMPRESSION_SAMPLE_BYTES, 65_536);
        assert_eq!(DEFAULT_COMPRESSION_THRESHOLD_PERCENT, 95);
    }

    #[test]
    fn version_string_is_parseable_semver() {
        let v = version();
        let mut parts = v.split('.');
        assert!(parts.next().unwrap().parse::<u32>().is_ok());
        assert!(parts.next().unwrap().parse::<u32>().is_ok());
        assert!(parts.next().unwrap().parse::<u32>().is_ok());
    }
}
