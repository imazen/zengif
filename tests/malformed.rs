//! Tests for handling malformed GIF inputs.

use enough::Unstoppable;
use zengif::{Decoder, GifError, Limits, Stats};

#[test]
fn empty_input() {
    let stats = Stats::new();
    let limits = Limits::default();

    let cursor = std::io::Cursor::new(Vec::<u8>::new());
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    assert!(result.is_err());
}

#[test]
fn truncated_header() {
    let stats = Stats::new();
    let limits = Limits::default();

    // Only "GIF" - truncated header
    let data = b"GIF";
    let cursor = std::io::Cursor::new(data.as_slice());
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    assert!(result.is_err());
}

#[test]
fn invalid_magic() {
    let stats = Stats::new();
    let limits = Limits::default();

    // Not a GIF
    let data = b"PNG\x89\x50\x4E\x47";
    let cursor = std::io::Cursor::new(data.as_slice());
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    assert!(result.is_err());
}

#[test]
fn header_only_no_frames() {
    let stats = Stats::new();
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
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    // A GIF with no image data is malformed - failing is correct
    assert!(result.is_err());
}

#[test]
fn dimensions_exceed_limits() {
    let stats = Stats::new();
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
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    match result {
        Ok(_) => panic!("Expected DimensionsTooLarge error"),
        Err(err) => assert!(matches!(err.error(), GifError::DimensionsTooLarge { .. })),
    }
}

#[test]
fn total_pixels_exceed_limits() {
    let stats = Stats::new();
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
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    match result {
        Ok(_) => panic!("Expected TotalPixelsTooLarge error"),
        Err(err) => assert!(matches!(err.error(), GifError::TotalPixelsTooLarge { .. })),
    }
}

#[test]
fn zero_dimensions() {
    let stats = Stats::new();
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
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

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
    let stats = Stats::new();
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
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

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
    let stats = Stats::new();
    let limits = Limits::default();

    // Random bytes
    let data: Vec<u8> = (0..100).map(|i| (i * 17) as u8).collect();

    let cursor = std::io::Cursor::new(data);
    let result = Decoder::new(cursor, limits, &stats, Unstoppable);

    // Should fail, not panic
    assert!(result.is_err());
}

#[test]
fn very_large_declared_dimensions() {
    let stats = Stats::new();

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
    let result = Decoder::new(cursor, Limits::default(), &stats, Unstoppable);

    // With default limits (16384x16384), this should fail
    assert!(result.is_err());
}
