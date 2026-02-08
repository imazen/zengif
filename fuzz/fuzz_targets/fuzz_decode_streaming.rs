//! Fuzz target for streaming GIF decode.
//!
//! Tests the `Decoder` streaming API with frame-by-frame iteration.
//! This exercises:
//! - Streaming decoder state machine
//! - Frame iteration consistency
//! - Partial decode handling
//! - Memory tracking during iteration

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use zengif::{Decoder, Limits, Unstoppable};

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default()
        .max_dimensions(1024, 1024)
        .max_total_pixels(1_000_000)
        .max_frame_count(100)
        .max_file_size(1024 * 1024)
        .max_memory(10 * 1024 * 1024)
        .max_decompression_ratio(100.0);

    let reader = Cursor::new(data);

    // Try to create decoder
    let mut decoder = match Decoder::new(reader, limits, &Unstoppable) {
        Ok(d) => d,
        Err(_) => return, // Invalid header is fine
    };

    // Iterate through frames using next_frame()
    let mut frame_count = 0;
    loop {
        match decoder.next_frame() {
            Ok(Some(frame)) => {
                frame_count += 1;
                // Basic sanity checks on frame data
                debug_assert!(frame.width > 0 && frame.height > 0);
                debug_assert_eq!(
                    frame.pixels.len(),
                    frame.width as usize * frame.height as usize
                );
                // Bail if we've seen enough frames
                if frame_count >= 100 {
                    break;
                }
            }
            Ok(None) => break, // No more frames
            Err(_) => break,   // Errors during iteration are expected
        }
    }

    // Frame count should be bounded by limits
    debug_assert!(frame_count <= 100);
});
