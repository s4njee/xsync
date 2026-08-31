//! Environment-overridable tuning knobs, for measurement rather than for users.
//!
//! Every constant reachable from here was derived on one link: ~1 `GbE` reached
//! through a USB adapter on the Mac, at a measured 5.3 ms in-session round
//! trip. A knee found at that latency is not obviously the knee at another, and
//! re-deriving one currently means rebuilding and redeploying both ends per
//! data point. These accessors exist so a sweep is a loop over an environment
//! variable instead.
//!
//! Three rules hold for everything in this module:
//!
//! 1. **Local effect only.** A knob here changes how one process paces itself.
//!    Nothing here is negotiated, and nothing here changes what the peer will
//!    accept, so the two ends may legitimately run different values.
//!    `MAX_DATA_SEGMENT` is deliberately *absent* for exactly this reason: the
//!    receiver validates incoming frames against it, so overriding it on one
//!    end alone would produce rejected frames rather than a faster transfer.
//! 2. **Bounded.** Each value is clamped to a range that keeps the invariant
//!    its default was chosen to protect. An out-of-range or unparseable value
//!    is clamped or ignored, never fatal: a benchmark harness that fat-fingers
//!    an export should produce a slow run, not a failed one.
//! 3. **Read once.** Values are cached on first use. These sit in per-frame
//!    loops, and reading the environment there would be both slow and a data
//!    race against any concurrent `setenv`.
//!
//! # The one invariant that spans both ends
//!
//! **The sender's pipelining window must exceed the receiver's apply-pool
//! capacity.** The receiver acknowledges a file only once it is durably
//! renamed, so it can hold up to `capacity` jobs un-acked; if the sender's
//! window is no larger, the sender blocks waiting for acks that the receiver is
//! waiting for more work to produce. Both stop.
//!
//! Measured directly: with a capacity of 64, a window of 32 lands 31 files and
//! then hangs indefinitely, while a window of 64 completes.
//!
//! Neither side can check this at run time — they are different processes, with
//! different core counts and different environments. It is instead held
//! structurally, by a floor on the window and a ceiling on the worker count
//! that cannot overlap, asserted at compile time below. That is why
//! [`apply_worker_count`] lives here rather than beside its use: the two bounds
//! are one decision and must be read together.
//!
//! Use [`snapshot`] to record what was actually in effect for a run. A
//! benchmark number without that is not attributable.

use std::sync::OnceLock;

use crate::protocol::DEFAULT_UNACKNOWLEDGED_WINDOW;
use crate::server::{LARGE_FILE_CHUNK, MAX_PIPELINED_FRAMES};
use crate::strategy::{BATCH_TARGET_SIZE, MAX_BATCH_FILES};

/// Lower bound on the pipelining window.
///
/// Must stay above the largest reachable apply-pool capacity — see the
/// module-level invariant. A window below this deadlocks rather than running
/// slowly, so it is clamped up instead of being honoured.
const MIN_PIPELINE_FRAMES: usize = 512;

/// Upper bound on receiver apply workers.
///
/// The pool's capacity is `workers * 8`, and that product is what the window
/// floor above has to clear.
const MAX_APPLY_WORKERS: usize = 32;

/// Jobs the apply pool may hold per worker before it blocks.
///
/// The pool multiplies its worker count by this to size the un-acked backlog
/// the window floor above must clear, so the two must not drift apart.
pub const APPLY_JOBS_PER_WORKER: usize = 8;

/// Upper bound on the pipelining window.
///
/// An acknowledgement frame is 41 bytes, so this caps the peer's pending
/// replies at ~1.3 MiB, inside OpenSSH's 2 MiB default channel window. The
/// window exists to stop a client that writes without ever reading from
/// deadlocking against a receiver blocked writing acks; a value large enough to
/// fill the channel would reintroduce exactly that deadlock.
const MAX_PIPELINE_FRAMES_CEILING: usize = 32_768;

/// Upper bound on one small-file batch, which is held in memory on both ends.
///
/// The smallest supported receiver is a 3 GB Raspberry Pi 5.
const MAX_BATCH_BYTES_CEILING: u64 = 512 * 1024 * 1024;

/// Upper bound on files per batch, bounding per-entry metadata.
const MAX_BATCH_FILES_CEILING: usize = 262_144;

/// The ceilings must hold the invariants the defaults were chosen to protect.
/// These are compile-time facts, so they are checked at compile time: an ack
/// frame is 41 bytes, and the whole window must stay inside OpenSSH's 2 MiB
/// channel window or the deadlock the window guards against returns.
const _: () = assert!(MAX_PIPELINE_FRAMES_CEILING * 41 < 2 * 1024 * 1024);
// The window floor must clear the largest apply-pool capacity a receiver can
// be configured to build, or a tuned run can deadlock instead of finishing.
const _: () = assert!(MIN_PIPELINE_FRAMES > MAX_APPLY_WORKERS * APPLY_JOBS_PER_WORKER);
const _: () = assert!(MIN_PIPELINE_FRAMES <= MAX_PIPELINE_FRAMES_CEILING);
const _: () = assert!(MAX_PIPELINED_FRAMES <= MAX_PIPELINE_FRAMES_CEILING);
const _: () = assert!(BATCH_TARGET_SIZE <= MAX_BATCH_BYTES_CEILING);
const _: () = assert!(MAX_BATCH_FILES <= MAX_BATCH_FILES_CEILING);

static PIPELINED_FRAMES: OnceLock<usize> = OnceLock::new();
static BATCH_BYTES: OnceLock<u64> = OnceLock::new();
static BATCH_FILES: OnceLock<usize> = OnceLock::new();
static APPLY_WORKERS: OnceLock<usize> = OnceLock::new();
static LARGE_CHUNKS: OnceLock<usize> = OnceLock::new();
static CHECKPOINT_CHUNKS: OnceLock<usize> = OnceLock::new();

/// Read an environment variable as `T`, clamped to `min..=max`.
///
/// An absent, empty, or unparseable value yields `default`, as does a value of
/// zero: `XSYNC_PIPELINE_FRAMES=0` is far more likely to be an unset shell
/// variable expanding to nothing than a deliberate request to stall.
fn bounded<T>(name: &str, default: T, min: T, max: T) -> T
where
    T: std::str::FromStr + PartialOrd + Copy,
{
    clamp_parsed(std::env::var(name).ok().as_deref(), default, min, max)
}

/// The parse-and-clamp half of [`bounded`], split out so it can be tested.
///
/// Setting an environment variable from a test is unsound in a threaded
/// process and the workspace denies `unsafe`, so the only honest way to cover
/// the clamping is to hand it the string directly.
fn clamp_parsed<T>(raw: Option<&str>, default: T, min: T, max: T) -> T
where
    T: std::str::FromStr + PartialOrd + Copy,
{
    let Some(raw) = raw else {
        return default;
    };
    let Ok(parsed) = raw.trim().parse::<T>() else {
        return default;
    };
    if parsed < min {
        min
    } else if parsed > max {
        max
    } else {
        parsed
    }
}

/// Frames the client may leave unacknowledged before it drains replies.
///
/// Overridden by `XSYNC_PIPELINE_FRAMES`. This is the knob most likely to want
/// re-deriving on a different link: the default was chosen at 5.3 ms round
/// trip, and the window that keeps a pipe full scales with the
/// bandwidth-delay product.
#[must_use]
pub fn max_pipelined_frames() -> usize {
    *PIPELINED_FRAMES.get_or_init(|| {
        bounded(
            "XSYNC_PIPELINE_FRAMES",
            MAX_PIPELINED_FRAMES,
            MIN_PIPELINE_FRAMES,
            MAX_PIPELINE_FRAMES_CEILING,
        )
    })
}

/// Target data size for one small-file batch.
///
/// Overridden by `XSYNC_BATCH_BYTES`.
#[must_use]
pub fn batch_target_size() -> u64 {
    *BATCH_BYTES.get_or_init(|| {
        bounded(
            "XSYNC_BATCH_BYTES",
            BATCH_TARGET_SIZE,
            64 * 1024,
            MAX_BATCH_BYTES_CEILING,
        )
    })
}

/// Maximum files in one small-file batch.
///
/// Overridden by `XSYNC_BATCH_FILES`.
#[must_use]
pub fn max_batch_files() -> usize {
    *BATCH_FILES.get_or_init(|| {
        bounded(
            "XSYNC_BATCH_FILES",
            MAX_BATCH_FILES,
            1,
            MAX_BATCH_FILES_CEILING,
        )
    })
}

/// Receiver threads publishing received files.
///
/// The receive loop must stay single-threaded — it decodes an ordered stream —
/// but publishing a file (write temp, verify, set metadata, rename) is
/// independent per file and was the serialized half of the transfer. Before
/// the pool, a Raspberry Pi 5 received within 7% of a 7950X, which is what a
/// one-thread apply path looks like.
///
/// Overridden by `XSYNC_APPLY_WORKERS`, and **capped**: the pool holds
/// `workers * 8` un-acked jobs, and that must stay under the sender's window
/// floor. Before this cap, `XSYNC_APPLY_WORKERS=1000` built a pool deep enough
/// to deadlock against even the stock 2048-frame window.
#[must_use]
pub fn apply_worker_count() -> usize {
    *APPLY_WORKERS.get_or_init(|| {
        let default = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(8);
        bounded("XSYNC_APPLY_WORKERS", default, 1, MAX_APPLY_WORKERS)
    })
}

/// 8 MB file chunks the sender may leave unacknowledged.
///
/// Overridden by `XSYNC_LARGE_CHUNKS_IN_FLIGHT`. The default is the negotiated
/// unacknowledged byte window divided by the chunk size -- four chunks, 32 MB.
///
/// This is a *byte* budget expressed in chunks, deliberately not the frame
/// window [`max_pipelined_frames`] uses. These frames are 8 MB each rather than
/// ack-sized, so 2048 of them in flight would be gigabytes.
///
/// **`1` reproduces the pre-4.60 lockstep exactly**, which makes it the control
/// arm for measuring what pipelining is worth on a given link.
#[must_use]
pub fn large_chunks_in_flight() -> usize {
    *LARGE_CHUNKS.get_or_init(|| {
        let default = usize::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW as u64 / LARGE_FILE_CHUNK)
            .unwrap_or(1)
            .max(1);
        bounded("XSYNC_LARGE_CHUNKS_IN_FLIGHT", default, 1, 32)
    })
}

/// Chunks a receiver may write before flushing them and checkpointing.
///
/// Overridden by `XSYNC_CHECKPOINT_CHUNKS`. **This is a durability/throughput
/// trade, not a tuning knob**: it is the amount of work an interrupted
/// transfer may have to redo. At the default of 8 that is 64 MB.
///
/// The invariant it must not break is ordering, not frequency -- staged chunks
/// are flushed *before* the checkpoint that records them, so a resume never
/// trusts a range that exists only in page cache. `1` restores the per-chunk
/// behaviour exactly.
///
/// Measured on a macOS receiver: the three barriers cost ~21 ms per 8 MB chunk
/// against ~71 ms of wire time, which was the whole of pull's remaining gap to
/// rsync (4.65).
#[must_use]
pub fn checkpoint_chunks() -> usize {
    *CHECKPOINT_CHUNKS.get_or_init(|| bounded("XSYNC_CHECKPOINT_CHUNKS", 8, 1, 64))
}

/// The values actually in effect, for recording beside a measurement.
///
/// Reports what the process resolved, not what the environment said, so a
/// clamped or rejected override is visible as the value that was really used.
#[must_use]
pub fn snapshot() -> Vec<(&'static str, String)> {
    vec![
        ("pipeline_frames", max_pipelined_frames().to_string()),
        ("batch_bytes", batch_target_size().to_string()),
        ("batch_files", max_batch_files().to_string()),
        ("apply_workers", apply_worker_count().to_string()),
        (
            "large_chunks_in_flight",
            large_chunks_in_flight().to_string(),
        ),
    ]
}

/// True when any knob differs from its compiled-in default.
///
/// A run with this set is not a stock run and should not be reported as one.
#[must_use]
pub fn is_tuned() -> bool {
    max_pipelined_frames() != MAX_PIPELINED_FRAMES
        || batch_target_size() != BATCH_TARGET_SIZE
        || max_batch_files() != MAX_BATCH_FILES
        || checkpoint_chunks() != 8
        || large_chunks_in_flight()
            != usize::try_from(DEFAULT_UNACKNOWLEDGED_WINDOW as u64 / LARGE_FILE_CHUNK)
                .unwrap_or(1)
                .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The public accessors cache in a `OnceLock`, so the first test to touch
    // one would fix its value for the whole process. The clamping logic is
    // tested directly instead.

    #[test]
    fn an_absent_variable_yields_the_default() {
        assert_eq!(clamp_parsed(None, 2048_usize, 512, 32_768), 2048);
    }

    #[test]
    fn an_unparseable_value_is_ignored_rather_than_fatal() {
        // A fat-fingered export should produce a stock run, not a failed one.
        assert_eq!(clamp_parsed(Some("garbage"), 2048_usize, 512, 32_768), 2048);
        assert_eq!(clamp_parsed(Some(""), 2048_usize, 512, 32_768), 2048);
    }

    #[test]
    fn a_value_below_the_floor_clamps_up_rather_than_deadlocking() {
        // Below the floor the sender's window can sit under the receiver's
        // apply-pool capacity, which hangs the transfer outright. Clamping is
        // what keeps a bad sweep slow instead of stuck.
        assert_eq!(clamp_parsed(Some("1"), 2048_usize, 512, 32_768), 512);
    }

    #[test]
    fn a_value_above_the_ceiling_clamps_down() {
        assert_eq!(
            clamp_parsed(Some("999999"), 2048_usize, 512, 32_768),
            32_768
        );
    }

    #[test]
    fn an_in_range_value_is_honoured_and_surrounding_whitespace_ignored() {
        assert_eq!(clamp_parsed(Some("4096"), 2048_usize, 512, 32_768), 4096);
        assert_eq!(clamp_parsed(Some(" 4096 "), 2048_usize, 512, 32_768), 4096);
    }
}
