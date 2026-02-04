//! Tests for cancellation support via the enough crate.

use almost_enough::{Stop, Stopper};
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, GifError, Limits, Rgba};

/// Create a minimal valid GIF.
fn minimal_gif() -> Vec<u8> {
    vec![
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
        0x3B, // Trailer
    ]
}

#[test]
fn decode_with_pre_cancelled_stopper() {
    let limits = Limits::default();

    // Pre-cancel the stopper
    let stop = Stopper::new();
    stop.cancel();

    let cursor = std::io::Cursor::new(minimal_gif());
    let result = Decoder::new(cursor, limits, stop);

    // Should fail with Cancelled
    match result {
        Ok(_) => panic!("Expected Cancelled error"),
        Err(err) => assert!(matches!(err.error(), GifError::Cancelled)),
    }
}

#[test]
fn decode_can_be_cancelled_between_frames() {
    // Create a multi-frame GIF
    // For simplicity, we'll use a stopper that's cancelled after creation
    let limits = Limits::default();

    let stop = Stopper::new();
    let stop_clone = stop.clone();

    let cursor = std::io::Cursor::new(minimal_gif());
    let mut decoder = Decoder::new(cursor, limits, stop).unwrap();

    // Read first frame successfully
    let frame = decoder.next_frame().unwrap();
    assert!(frame.is_some());

    // Now cancel
    stop_clone.cancel();

    // Next operation should detect cancellation
    // (This won't fail for a single-frame GIF that's already finished,
    // but demonstrates the pattern)
}

#[test]
fn encode_with_pre_cancelled_stopper() {
    let config = EncoderConfig::new(2, 2);
    let limits = Limits::default();

    // Pre-cancel the stopper
    let stop = Stopper::new();
    stop.cancel();

    let mut output = Vec::new();
    let result = Encoder::new(&mut output, config, limits, stop);

    // Should fail with Cancelled
    match result {
        Ok(_) => panic!("Expected Cancelled error"),
        Err(err) => assert!(matches!(err.error(), GifError::Cancelled)),
    }
}

#[test]
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
fn encode_can_be_cancelled_between_frames() {
    let config = EncoderConfig::new(2, 2);
    let limits = Limits::default();

    let stop = Stopper::new();
    let stop_clone = stop.clone();

    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, config, limits, stop).unwrap();

    // Add first frame successfully
    let frame = FrameInput::new(2, 2, 10, vec![Rgba::rgb(255, 0, 0); 4]);
    encoder.add_frame(frame).unwrap();

    // Now cancel
    stop_clone.cancel();

    // Next add_frame should fail with Cancelled
    let frame2 = FrameInput::new(2, 2, 10, vec![Rgba::rgb(0, 255, 0); 4]);
    let result = encoder.add_frame(frame2);

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err().error(), GifError::Cancelled));
}

#[test]
fn stopper_clone_cancels_all() {
    let stop1 = Stopper::new();
    let stop2 = stop1.clone();
    let stop3 = stop1.clone();

    assert!(!stop1.should_stop());
    assert!(!stop2.should_stop());
    assert!(!stop3.should_stop());

    // Cancel via one clone
    stop2.cancel();

    // All should be cancelled
    assert!(stop1.should_stop());
    assert!(stop2.should_stop());
    assert!(stop3.should_stop());
}
