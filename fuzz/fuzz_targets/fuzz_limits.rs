//! Fuzz target for limits enforcement.
//!
//! Tests that various limit configurations are properly enforced.
//! Uses arbitrary to generate both GIF data and limit configurations.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use zengif::{decode_gif, Limits, Stats, Unstoppable};

/// Fuzzed limit configuration
#[derive(Debug, Arbitrary)]
struct FuzzedLimits {
    max_width: u16,
    max_height: u16,
    max_frames: u16,
    max_file_size_kb: u16,
    max_memory_kb: u16,
    max_ratio: u8,
}

impl FuzzedLimits {
    fn to_limits(&self) -> Limits {
        // Ensure minimums to avoid divide-by-zero and similar issues
        let max_width = self.max_width.max(1);
        let max_height = self.max_height.max(1);
        let max_frames = (self.max_frames as usize).max(1);
        let max_file_size = ((self.max_file_size_kb as u64).max(1)) * 1024;
        let max_memory = ((self.max_memory_kb as usize).max(1)) * 1024;
        let max_ratio = (self.max_ratio as f64).max(1.0);

        Limits::default()
            .max_dimensions(max_width, max_height)
            .max_total_pixels((max_width as u64) * (max_height as u64))
            .max_frame_count(max_frames)
            .max_file_size(max_file_size)
            .max_memory(max_memory)
            .max_decompression_ratio(max_ratio)
    }
}

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    limits: FuzzedLimits,
    data: Vec<u8>,
}

fuzz_target!(|input: FuzzInput| {
    let limits = input.limits.to_limits();
    let stats = Stats::new();

    // Try to decode with the fuzzed limits
    let result = decode_gif(&input.data, limits, &stats, Unstoppable);

    // Verify that limits were enforced
    match result {
        Ok((_, frames)) => {
            // If decode succeeded, verify limits were respected
            let max_width = input.limits.max_width.max(1);
            let max_height = input.limits.max_height.max(1);
            let max_frames = (input.limits.max_frames as usize).max(1);

            debug_assert!(
                frames.len() <= max_frames,
                "Decoded {} frames but limit was {}",
                frames.len(),
                max_frames
            );

            for (i, frame) in frames.iter().enumerate() {
                debug_assert!(
                    frame.width <= max_width,
                    "Frame {} width {} exceeds limit {}",
                    i,
                    frame.width,
                    max_width
                );
                debug_assert!(
                    frame.height <= max_height,
                    "Frame {} height {} exceeds limit {}",
                    i,
                    frame.height,
                    max_height
                );
            }
        }
        Err(_) => {
            // Errors are expected - limits should reject oversized inputs
        }
    }

    // Memory usage should never exceed configured limit (with some slack for overhead)
    let max_memory = ((input.limits.max_memory_kb as usize).max(1)) * 1024;
    let peak = stats.peak();
    // Allow 2x overhead for internal bookkeeping
    debug_assert!(
        peak <= max_memory * 2,
        "Peak memory {} exceeded 2x limit {}",
        peak,
        max_memory
    );
});
