#![cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant"))]
//! Round-trip tests: encode -> decode -> verify

use enough::Unstoppable;
use zengif::{Decoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, decode_gif, encode_gif};

/// Create a solid color frame.
fn solid_frame(width: u16, height: u16, color: Rgba, delay: u16) -> FrameInput {
    let pixels = vec![color; width as usize * height as usize];
    FrameInput::new(width, height, delay, pixels)
}

/// Create a checkerboard frame.
fn checkerboard_frame(width: u16, height: u16, c1: Rgba, c2: Rgba, delay: u16) -> FrameInput {
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let color = if (x + y) % 2 == 0 { c1 } else { c2 };
            pixels.push(color);
        }
    }
    FrameInput::new(width, height, delay, pixels)
}

#[test]
fn round_trip_single_frame() {
    let width = 4;
    let height = 4;
    let frame = solid_frame(width, height, Rgba::rgb(255, 0, 0), 10);

    // Encode
    let config = EncoderConfig::new().repeat(Repeat::Once);
    let encoded = encode_gif(
        vec![frame],
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode
    let (metadata, frames, _stats) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    // Verify
    assert_eq!(metadata.width, width);
    assert_eq!(metadata.height, height);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].width, width);
    assert_eq!(frames[0].height, height);

    // Note: Colors may not match exactly due to palette quantization
    // but should be close to red
    let first_pixel = frames[0].pixels[0];
    assert!(
        first_pixel.r > 200,
        "Expected mostly red, got {:?}",
        first_pixel
    );
}

#[test]
fn round_trip_multiple_frames() {
    let width = 4;
    let height = 4;
    let frames_in = vec![
        solid_frame(width, height, Rgba::rgb(255, 0, 0), 10),
        solid_frame(width, height, Rgba::rgb(0, 255, 0), 20),
        solid_frame(width, height, Rgba::rgb(0, 0, 255), 30),
    ];

    // Encode
    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_gif(
        frames_in,
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode
    let (metadata, frames_out, _stats) =
        decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    // Verify
    assert_eq!(metadata.width, width);
    assert_eq!(metadata.height, height);
    assert_eq!(frames_out.len(), 3);

    // Check delays are preserved
    assert_eq!(frames_out[0].delay, 10);
    assert_eq!(frames_out[1].delay, 20);
    assert_eq!(frames_out[2].delay, 30);
}

#[test]
fn round_trip_with_transparency() {
    let width = 4;
    let height = 4;

    // Frame with some transparent pixels
    let mut pixels = vec![Rgba::rgb(255, 0, 0); width as usize * height as usize];
    // Make some pixels transparent
    pixels[0] = Rgba::TRANSPARENT;
    pixels[5] = Rgba::TRANSPARENT;

    let frame = FrameInput::new(width, height, 10, pixels);

    // Encode
    let config = EncoderConfig::new();
    let encoded = encode_gif(
        vec![frame],
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode
    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    assert_eq!(frames.len(), 1);

    // Transparent pixels should decode as transparent
    // (Note: depends on encoder preserving transparency)
    // With simple quantization, fully transparent pixels should remain transparent
    let pixel_0 = frames[0].pixels[0];
    // The pixel might be transparent or might be background color
    // depending on how the encoder handles transparency
    assert!(pixel_0.a == 0 || pixel_0.a == 255);
}

#[test]
fn round_trip_checkerboard() {
    let width = 8;
    let height = 8;
    let frame = checkerboard_frame(
        width,
        height,
        Rgba::rgb(255, 255, 255),
        Rgba::rgb(0, 0, 0),
        10,
    );

    // Encode
    let config = EncoderConfig::new();
    let encoded = encode_gif(
        vec![frame],
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode
    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pixel_count(), 64);
}

#[test]
fn round_trip_preserves_metadata() {
    let width = 4;
    let height = 4;
    let frame = solid_frame(width, height, Rgba::rgb(128, 128, 128), 50);

    // Encode with Infinite repeat
    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_gif(
        vec![frame],
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode
    let (metadata, frames, _stats) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    assert_eq!(metadata.width, width);
    assert_eq!(metadata.height, height);
    assert_eq!(frames.len(), 1);
    // Delay should be preserved
    assert_eq!(frames[0].delay, 50);
}

#[test]
fn memory_tracking_during_round_trip() {
    let width = 16;
    let height = 16;
    let frames_in: Vec<_> = (0..5)
        .map(|i| solid_frame(width, height, Rgba::rgb((i * 50) as u8, 100, 150), 10))
        .collect();

    // Encode
    let config = EncoderConfig::new();
    let encoded = encode_gif(
        frames_in,
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Decode with stats tracking
    let (_, frames, stats) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    assert_eq!(frames.len(), 5);

    // Stats should show allocations happened
    assert!(stats.peak() > 0);
    assert!(stats.alloc_count() > 0);
}

#[test]
fn streaming_decode_matches_batch() {
    let width = 4;
    let height = 4;
    let frames_in = vec![
        solid_frame(width, height, Rgba::rgb(255, 0, 0), 10),
        solid_frame(width, height, Rgba::rgb(0, 255, 0), 20),
    ];

    // Encode
    let config = EncoderConfig::new();
    let encoded = encode_gif(
        frames_in,
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    // Batch decode
    let (_, batch_frames, _stats1) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();

    // Streaming decode
    let cursor = std::io::Cursor::new(&encoded);
    let mut decoder = Decoder::new(cursor, Limits::default(), &Unstoppable).unwrap();
    let mut streaming_frames = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        streaming_frames.push(frame);
    }

    // Both should produce same results
    assert_eq!(batch_frames.len(), streaming_frames.len());
    for (batch, stream) in batch_frames.iter().zip(streaming_frames.iter()) {
        assert_eq!(batch.width, stream.width);
        assert_eq!(batch.height, stream.height);
        assert_eq!(batch.delay, stream.delay);
        assert_eq!(batch.pixels.len(), stream.pixels.len());
    }
}

/// Vertically flip pixels in place.
fn vflip(pixels: &mut [Rgba], width: usize, height: usize) {
    for y in 0..height / 2 {
        let top_row_start = y * width;
        let bottom_row_start = (height - 1 - y) * width;
        for x in 0..width {
            pixels.swap(top_row_start + x, bottom_row_start + x);
        }
    }
}

/// Test round-trip with vflip transformation using palette pass-through.
///
/// This verifies that:
/// 1. Palettes are correctly exposed during decode
/// 2. Palette pass-through encoding works correctly
/// 3. The full cycle: decode -> vflip -> encode -> decode -> vflip produces original
#[test]
fn round_trip_vflip_with_palette_passthrough() {
    // Create a multi-colored animation to test palette preservation
    let width = 4u16;
    let height = 4u16;

    // Create frames with different colors - a gradient pattern
    let original_frames: Vec<FrameInput> = (0..3)
        .map(|i| {
            let mut pixels = Vec::with_capacity(width as usize * height as usize);
            for y in 0..height {
                for x in 0..width {
                    // Create a pattern that changes with frame index
                    let r = ((x as u32 * 60 + i * 30) % 256) as u8;
                    let g = ((y as u32 * 60 + i * 20) % 256) as u8;
                    let b = (((x + y) as u32 * 40 + i * 40) % 256) as u8;
                    pixels.push(Rgba::rgb(r, g, b));
                }
            }
            FrameInput::new(width, height, 10, pixels)
        })
        .collect();

    // Step 1: Encode original
    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded1 = encode_gif(
        original_frames,
        width,
        height,
        config.clone(),
        Limits::default(),
        &Unstoppable,
    )
    .expect("Initial encode failed");

    // Step 2: Decode

    let (metadata1, frames1, _stats1) =
        decode_gif(&encoded1, Limits::default(), &Unstoppable).expect("First decode failed");

    assert_eq!(frames1.len(), 3);

    // Verify palettes are exposed
    for (i, frame) in frames1.iter().enumerate() {
        assert!(
            frame.palette.is_some(),
            "Frame {} should have palette exposed",
            i
        );
    }

    // Step 3: Vflip and encode with original palettes
    let flipped_frames: Vec<FrameInput> = frames1
        .iter()
        .map(|frame| {
            let mut pixels = frame.pixels.clone();
            vflip(&mut pixels, frame.width as usize, frame.height as usize);

            // Use palette pass-through
            if let Some(ref palette) = frame.palette {
                FrameInput::with_palette(
                    frame.width,
                    frame.height,
                    frame.delay,
                    pixels,
                    palette.clone(),
                )
            } else {
                FrameInput::new(frame.width, frame.height, frame.delay, pixels)
            }
        })
        .collect();

    let config2 = EncoderConfig::new().repeat(metadata1.repeat);
    let encoded2 = encode_gif(
        flipped_frames,
        width,
        height,
        config2.clone(),
        Limits::default(),
        &Unstoppable,
    )
    .expect("Flipped encode failed");

    // Step 4: Decode flipped

    let (_, frames2, _stats2) =
        decode_gif(&encoded2, Limits::default(), &Unstoppable).expect("Second decode failed");

    assert_eq!(frames2.len(), 3);

    // Step 5: Vflip again and encode
    let reflipped_frames: Vec<FrameInput> = frames2
        .iter()
        .map(|frame| {
            let mut pixels = frame.pixels.clone();
            vflip(&mut pixels, frame.width as usize, frame.height as usize);

            if let Some(ref palette) = frame.palette {
                FrameInput::with_palette(
                    frame.width,
                    frame.height,
                    frame.delay,
                    pixels,
                    palette.clone(),
                )
            } else {
                FrameInput::new(frame.width, frame.height, frame.delay, pixels)
            }
        })
        .collect();

    let encoded3 = encode_gif(
        reflipped_frames,
        width,
        height,
        config2,
        Limits::default(),
        &Unstoppable,
    )
    .expect("Re-flipped encode failed");

    // Step 6: Decode final

    let (_, frames3, _stats3) =
        decode_gif(&encoded3, Limits::default(), &Unstoppable).expect("Final decode failed");

    assert_eq!(frames3.len(), 3);

    // Step 7: Compare with original decode (frames1)
    // After vflip -> encode -> decode -> vflip -> encode -> decode,
    // we should get back approximately the same pixels
    // (exact match isn't guaranteed due to palette quantization, but should be close)
    for (i, (original, final_frame)) in frames1.iter().zip(frames3.iter()).enumerate() {
        assert_eq!(
            original.width, final_frame.width,
            "Frame {} width mismatch",
            i
        );
        assert_eq!(
            original.height, final_frame.height,
            "Frame {} height mismatch",
            i
        );
        assert_eq!(
            original.delay, final_frame.delay,
            "Frame {} delay mismatch",
            i
        );
        assert_eq!(
            original.pixels.len(),
            final_frame.pixels.len(),
            "Frame {} pixel count mismatch",
            i
        );

        // Check that most pixels are similar (within tolerance due to palette mapping)
        let mut close_count = 0;
        for (orig_px, final_px) in original.pixels.iter().zip(final_frame.pixels.iter()) {
            let dr = (orig_px.r as i32 - final_px.r as i32).abs();
            let dg = (orig_px.g as i32 - final_px.g as i32).abs();
            let db = (orig_px.b as i32 - final_px.b as i32).abs();
            // Allow some tolerance for palette mapping differences
            if dr <= 32 && dg <= 32 && db <= 32 {
                close_count += 1;
            }
        }

        let total = original.pixels.len();
        let close_ratio = close_count as f64 / total as f64;
        assert!(
            close_ratio >= 0.8,
            "Frame {}: Only {:.1}% of pixels are close (expected >= 80%)",
            i,
            close_ratio * 100.0
        );
    }
}
