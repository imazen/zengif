//! Fuzz target for full GIF decode path.
//!
//! Tests the `decode_gif` convenience function which decodes all frames at once.
//! This exercises:
//! - Header validation
//! - LZW decompression
//! - Frame compositing
//! - Disposal method handling
//! - Transparency handling
//! - Memory tracking

#![no_main]

use libfuzzer_sys::fuzz_target;
use zengif::{decode_gif, Limits, Unstoppable};

fuzz_target!(|data: &[u8]| {
    // Use restrictive limits for fuzzing to catch issues quickly
    let limits = Limits::default()
        .max_dimensions(1024, 1024) // Smaller for faster fuzzing
        .max_total_pixels(1_000_000) // 1 megapixel max
        .max_frame_count(100)
        .max_file_size(1024 * 1024) // 1 MB
        .max_memory(10 * 1024 * 1024) // 10 MB
        .max_decompression_ratio(100.0); // Tighter ratio for fuzzing

    // Try to decode - we expect most inputs to fail gracefully
    let _ = decode_gif(data, limits, &Unstoppable);

    // Note: Memory leak detection is tricky because:
    // - Stats tracks allocations but ComposedFrame doesn't dealloc via Stats
    // - The gif crate may have internal caches
    // For proper leak detection, run with --sanitizer address
});
