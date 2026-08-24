//! Bounded compression sampling for adaptive per-payload encoding.

use std::io;

/// Sample sizes used by the Story 0.5 compression matrix.
pub const SAMPLE_SIZES: [usize; 3] = [64 * 1024, 256 * 1024, 1024 * 1024];
/// Compress only when the sample is smaller than 95% of its input.
pub const COMPRESSION_THRESHOLD: f32 = 0.95;

/// Result of evaluating one or more logical payloads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressionDecision {
    /// Matrix sample size selected for this input.
    pub sample_size: usize,
    /// Bytes represented by the sample.
    pub sampled_bytes: usize,
    /// Compressed sample length at the requested level.
    pub compressed_bytes: usize,
    /// Whether the complete payload should use compression.
    pub use_compression: bool,
}

/// Choose a matrix sample size from the actual logical input size.
#[must_use]
pub const fn sample_size(total_bytes: usize) -> usize {
    if total_bytes <= SAMPLE_SIZES[0] {
        total_bytes
    } else if total_bytes <= SAMPLE_SIZES[1] {
        SAMPLE_SIZES[1]
    } else {
        SAMPLE_SIZES[2]
    }
}

/// Evaluate a heterogeneous batch without allocating a full concatenated copy.
///
/// # Errors
/// Returns an I/O error if the zstd encoder cannot evaluate the bounded sample.
pub fn decide<'a, I>(payloads: I, level: i32) -> io::Result<CompressionDecision>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let payloads: Vec<&[u8]> = payloads.into_iter().filter(|p| !p.is_empty()).collect();
    let total = payloads.iter().map(|p| p.len()).sum();
    let selected = sample_size(total);
    let mut sample = Vec::with_capacity(selected.min(total));
    for payload in &payloads {
        if sample.len() == selected {
            break;
        }
        sample.extend_from_slice(&payload[..(selected - sample.len()).min(payload.len())]);
    }
    let compressed = zstd::bulk::compress(&sample, level)?;
    let use_compression = !sample.is_empty()
        && compressed.len().saturating_mul(100) < sample.len().saturating_mul(95);
    Ok(CompressionDecision {
        sample_size: selected,
        sampled_bytes: sample.len(),
        compressed_bytes: compressed.len(),
        use_compression,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_actual_matrix_bucket_and_short_input() {
        assert_eq!(sample_size(12), 12);
        assert_eq!(sample_size(64 * 1024), 64 * 1024);
        assert_eq!(sample_size(64 * 1024 + 1), 256 * 1024);
        assert_eq!(sample_size(256 * 1024 + 1), 1024 * 1024);
    }

    #[test]
    fn detects_compressible_and_incompressible_batches() {
        let text = vec![b'a'; 300 * 1024];
        let random: Vec<u8> = (0..300 * 1024)
            .map(|n| {
                u8::try_from(n % 256)
                    .expect("modulo keeps test byte in range")
                    .wrapping_mul(53)
            })
            .collect();
        let text_decision = decide([text.as_slice()], 3).unwrap();
        let mixed_decision = decide([text.as_slice(), random.as_slice()], 3).unwrap();
        assert!(text_decision.use_compression);
        assert_eq!(mixed_decision.sample_size, 1024 * 1024);
        assert!(mixed_decision.sampled_bytes > text.len());
    }

    #[test]
    fn large_concurrent_style_batch_keeps_sampling_bounded() {
        let payloads: Vec<Vec<u8>> = (0..32)
            .map(|index| vec![u8::try_from(index).expect("test byte fits"); 4 * 1024 * 1024])
            .collect();
        let slices: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
        let decision = decide(slices, 3).unwrap();
        assert_eq!(decision.sample_size, 1024 * 1024);
        assert_eq!(decision.sampled_bytes, 1024 * 1024);
    }
}
