#![no_main]

use libfuzzer_sys::fuzz_target;
use xsync_core::protocol::{decode_frame, FrameDecoder};

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_frame(bytes);
    let mut decoder = FrameDecoder::new();
    let _ = decoder.decode(bytes);
});
