//! Fuzz target for encode/decode round-trip.
//!
//! Tests that valid decoded frames can be re-encoded and decoded again.
//! This exercises:
//! - Encoder state machine
//! - Frame encoding
//! - Palette handling
//! - Round-trip consistency

#![no_main]

use libfuzzer_sys::fuzz_target;
use zengif::{
    decode_gif, encode_gif, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable,
};

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default()
        .max_dimensions(256, 256) // Very small for roundtrip fuzzing
        .max_total_pixels(65536)
        .max_frame_count(10) // Few frames for speed
        .max_file_size(512 * 1024)
        .max_memory(5 * 1024 * 1024)
        .max_decompression_ratio(100.0);

    // Step 1: Try to decode the input
    let (metadata, frames, _stats) = match decode_gif(data, limits.clone(), &Unstoppable) {
        Ok(result) if !result.1.is_empty() => result,
        _ => return, // Need at least one valid frame
    };

    // Get dimensions from metadata
    let width = metadata.width;
    let height = metadata.height;

    // Step 2: Convert to FrameInput for encoding
    let frame_inputs: Vec<FrameInput> = frames
        .iter()
        .map(|f| FrameInput::new(f.width, f.height, f.delay.max(1), f.pixels.clone()))
        .collect();

    // Step 3: Encode
    let config = EncoderConfig::new().repeat(Repeat::Once);

    let output = match encode_gif(
        frame_inputs.clone(),
        width,
        height,
        config,
        limits.clone(),
        &Unstoppable,
    ) {
        Ok(o) => o,
        Err(_) => return, // Encoding failure is acceptable
    };

    // Step 4: Decode the re-encoded output
    let (_, decoded, _stats2) = match decode_gif(&output, limits, &Unstoppable) {
        Ok(d) => d,
        Err(_) => {
            // If we successfully encoded but can't decode, that's a bug!
            // But only if the output is non-empty
            if !output.is_empty() {
                panic!("Successfully encoded GIF but failed to decode it");
            }
            return;
        }
    };

    // Step 5: Verify basic consistency
    debug_assert_eq!(
        decoded.len(),
        frame_inputs.len(),
        "Frame count mismatch after round-trip"
    );

    for (i, (orig, dec)) in frames.iter().zip(decoded.iter()).enumerate() {
        debug_assert_eq!(orig.width, dec.width, "Frame {} width mismatch", i);
        debug_assert_eq!(orig.height, dec.height, "Frame {} height mismatch", i);
    }
});
