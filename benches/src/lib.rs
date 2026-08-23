//! Independent benchmark reporting and correctness infrastructure for xsync.
//!
//! This crate deliberately does not depend on `xsync-core`: a destination
//! oracle must not reuse the scanner or metadata interpretation it verifies.

pub mod corpus;
pub mod gate;
pub mod manifest;
pub mod report;
pub mod scratch;

pub use corpus::{create_corpus, CorpusClass, CorpusRequest, GeneratedCorpus, Tier, Workload};
pub use gate::{evaluate_gate, GateOutcome};
pub use manifest::{build_manifest, verify_manifest, Manifest, Verification};
pub use report::{Report, ReportInput};
pub use scratch::{OwnedScratch, ScratchError};

#[cfg(test)]
mod test_support;
