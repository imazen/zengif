#![cfg(feature = "std")]
//! Tests for handling malformed GIF inputs.

use enough::Unstoppable;
use zengif::{Decoder, GifError, Limits};

#[test]
fn empty_input() {
    let limits = Limits::default();

    let cursor = std::io::Cursor::new(Vec::<u8>::new());
    let result = Decoder::new(cursor, limits, &Unstoppable);

    assert!(result.is_err());
}

#[test]
fn truncated_header() {
    let limits = Limits::default();

    // Only "GIF" - truncated header
    let data = b"GIF";
    let cursor = std::io::Cursor::new(data.as_slice());
    let result = Decoder::new(cursor, limits, &Unstoppable);

    assert!(result.is_err());
}

#[test]
fn invalid_magic() {
    let limits = Limits::default();

    // Not a GIF
    let data = b"PNG\x89\x50\x4E\x47";
    let cursor = std::io::Cursor::new(data.as_slice());
    let result = Decoder::new(cursor, limits, &Unstoppable);

    assert!(result.is_err());
}

#[test]
fn header_only_no_frames() {
    let limits = Limits::default();

    // Header followed by trailer but no image data
    // Per GIF spec, a valid GIF should have at least one Image block.
    // The gif crate treats this as invalid (UnexpectedEof), which is correct.
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x00, // No global color table
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0x3B, // Trailer immediately
    ];

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &Unstoppable);

    // A GIF with no image data is malformed - failing is correct
    assert!(result.is_err());
}

#[test]
fn dimensions_exceed_limits() {
    let limits = Limits::default().max_dimensions(100, 100);

    // 200x200 image (exceeds 100x100 limit)
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0xC8, 0x00, 0xC8, 0x00, // 200x200
        0x00, // No global color table
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0x3B, // Trailer
    ];

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &Unstoppable);

    match result {
        Ok(_) => panic!("Expected DimensionsTooLarge error"),
        Err(err) => assert!(matches!(err.error(), GifError::DimensionsTooLarge { .. })),
    }
}

#[test]
fn total_pixels_exceed_limits() {
    // Allow up to 100 pixels total
    let limits = Limits::default()
        .max_dimensions(u16::MAX, u16::MAX)
        .max_total_pixels(100);

    // 20x20 = 400 pixels (exceeds 100)
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x14, 0x00, 0x14, 0x00, // 20x20
        0x00, // No global color table
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0x3B, // Trailer
    ];

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &Unstoppable);

    match result {
        Ok(_) => panic!("Expected TotalPixelsTooLarge error"),
        Err(err) => assert!(matches!(err.error(), GifError::TotalPixelsTooLarge { .. })),
    }
}

#[test]
fn zero_dimensions() {
    let limits = Limits::none(); // No limits, so 0x0 should still be checked

    // 0x0 image - technically valid according to limits but may cause issues
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x00, 0x00, 0x00, 0x00, // 0x0
        0x00, // No global color table
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0x3B, // Trailer
    ];

    let cursor = std::io::Cursor::new(data);
    // This should either succeed with an empty canvas or fail gracefully
    let result = Decoder::new(cursor, limits, &Unstoppable);

    // We don't care if it succeeds or fails, just that it doesn't panic
    match result {
        Ok(mut decoder) => {
            // If it succeeds, reading frames shouldn't panic
            let _ = decoder.next_frame();
        }
        Err(_) => {
            // Failing is also acceptable for 0x0
        }
    }
}

#[test]
fn truncated_lzw_data() {
    let limits = Limits::default();

    // Valid header, starts a frame, but LZW data is truncated
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x02, 0x00, 0x02, 0x00, // 2x2
        0x80, // Global color table, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00, 0x00, // Color 1: Black
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x02, 0x00, 0x02, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
              // LZW data truncated here - no block size or data
    ];

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &Unstoppable);

    // Should fail during decoding
    match result {
        Ok(mut decoder) => {
            // If decoder creation succeeds, reading should fail
            let frame_result = decoder.next_frame();
            assert!(frame_result.is_err());
        }
        Err(_) => {
            // Failing at creation is also acceptable
        }
    }
}

#[test]
fn random_garbage() {
    let limits = Limits::default();

    // Random bytes
    let data: Vec<u8> = (0..100).map(|i| (i * 17) as u8).collect();

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &Unstoppable);

    // Should fail, not panic
    assert!(result.is_err());
}

#[test]
fn very_large_declared_dimensions() {
    // Declare max dimensions but don't provide data
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0xFF, 0xFF, 0xFF, 0xFF, // 65535x65535
        0x00, // No global color table
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0x3B, // Trailer
    ];

    let cursor = std::io::Cursor::new(data);
    // This should fail with default limits (16384x16384)
    let result = Decoder::new(cursor, Limits::default(), &Unstoppable);

    // With default limits (16384x16384), this should fail
    assert!(result.is_err());
}

#[test]
fn missing_trailer_single_frame() {
    // Valid 1x1 GIF with frame data but no trailing 0x3B byte.
    // Browsers display these fine. We should too.
    // See: https://github.com/image-rs/image-gif/issues/138
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00, 0x00, // Color 1: Black
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x44, 0x01, // LZW data
        0x00, // Block terminator
              // NOTE: no 0x3B trailer
    ];

    let limits = Limits::default();
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

    // First frame should decode successfully
    let frame = decoder.next_frame().unwrap().expect("should get one frame");
    assert_eq!(frame.width, 1);
    assert_eq!(frame.height, 1);
    // Red pixel
    assert_eq!(frame.pixels[0].r, 255);
    assert_eq!(frame.pixels[0].g, 0);
    assert_eq!(frame.pixels[0].b, 0);

    // Should signal end-of-stream, not error
    assert!(decoder.next_frame().unwrap().is_none());
    assert!(decoder.is_finished());
}

#[test]
fn missing_trailer_multi_frame() {
    // Two-frame 1x1 GIF without trailer byte.
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0xFF, 0x00, // Color 1: Green
        // Frame 1 (color index 0 = red)
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x44, 0x01, // LZW data (index 0)
        0x00, // Block terminator
        // Frame 2 (color index 1 = green)
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x4C, 0x01, // LZW data (index 1)
        0x00, // Block terminator
              // NOTE: no 0x3B trailer
    ];

    let limits = Limits::default();
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

    // Both frames should decode
    let frame1 = decoder.next_frame().unwrap().expect("should get frame 1");
    assert_eq!(frame1.pixels[0].r, 255);

    let frame2 = decoder.next_frame().unwrap().expect("should get frame 2");
    assert_eq!(frame2.pixels[0].g, 255);

    // Should signal end-of-stream, not error
    assert!(decoder.next_frame().unwrap().is_none());
}

#[test]
fn missing_trailer_zero_frames_still_errors() {
    // A GIF header with no frame data and no trailer.
    // With zero frames decoded, UnexpectedEof should still be an error
    // (we can't silently succeed with nothing).
    let data = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00,
        0x00, // Color 1: Black
              // No image descriptor, no trailer — just ends
    ];

    let limits = Limits::default();
    let cursor = std::io::Cursor::new(data);
    // This can fail at Decoder::new or at next_frame — either is fine
    match Decoder::new(cursor, limits, &Unstoppable) {
        Ok(mut decoder) => {
            let result = decoder.next_frame();
            assert!(
                result.is_err(),
                "should error with zero frames and no trailer"
            );
        }
        Err(_) => {
            // Also acceptable — failing early is fine
        }
    }
}

// --- Reduced fuzz artifacts (crash-repro corpus) ---
//
// Each test loads a minimal malformed GIF from tests/corpus/crash-repro/
// and verifies that decoding handles it gracefully (error or safe result,
// never a panic).

/// Helper: attempt full decode of a crash-repro GIF file.
/// Returns Ok(()) whether decoding succeeds or returns an error —
/// the only failure mode is a panic.
fn decode_crash_repro(filename: &str) {
    let path = format!(
        "{}/tests/corpus/crash-repro/{}",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    let limits = Limits::default();
    let cursor = std::io::Cursor::new(data);
    match Decoder::new(cursor, limits, &Unstoppable) {
        Ok(mut decoder) => {
            // Drain all frames — errors are fine, panics are not
            loop {
                match decoder.next_frame() {
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
        Err(_) => {
            // Failing at construction is acceptable
        }
    }
}

#[test]
fn crash_repro_oob_palette_index() {
    decode_crash_repro("gif_oob_palette_index.gif");
}

#[test]
fn crash_repro_frame_exceeds_canvas() {
    decode_crash_repro("gif_frame_exceeds_canvas.gif");
}

#[test]
fn crash_repro_bad_bg_index() {
    decode_crash_repro("gif_bad_bg_index.gif");
}

#[test]
fn crash_repro_zero_size_frame() {
    decode_crash_repro("gif_zero_size_frame.gif");
}

#[test]
fn crash_repro_overflow_frame_pos() {
    decode_crash_repro("gif_overflow_frame_pos.gif");
}

#[test]
fn crash_repro_frame_buffer_oob() {
    decode_crash_repro("gif_frame_buffer_oob.gif");
}
