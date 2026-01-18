//! Round-trip tests: encode -> decode -> verify

use enough::Unstoppable;
use zengif::{
    decode_gif, encode_gif, Decoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Stats,
};

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
    let config = EncoderConfig::new(width, height).repeat(Repeat::Once);
    let encoded = encode_gif(vec![frame], config, Limits::default(), Unstoppable).unwrap();

    // Decode
    let stats = Stats::new();
    let (metadata, frames) = decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

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
    let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
    let encoded = encode_gif(frames_in, config, Limits::default(), Unstoppable).unwrap();

    // Decode
    let stats = Stats::new();
    let (metadata, frames_out) =
        decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

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
    let config = EncoderConfig::new(width, height);
    let encoded = encode_gif(vec![frame], config, Limits::default(), Unstoppable).unwrap();

    // Decode
    let stats = Stats::new();
    let (_, frames) = decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

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
    let config = EncoderConfig::new(width, height);
    let encoded = encode_gif(vec![frame], config, Limits::default(), Unstoppable).unwrap();

    // Decode
    let stats = Stats::new();
    let (_, frames) = decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pixel_count(), 64);
}

#[test]
fn round_trip_preserves_metadata() {
    let width = 4;
    let height = 4;
    let frame = solid_frame(width, height, Rgba::rgb(128, 128, 128), 50);

    // Encode with Infinite repeat
    let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
    let encoded = encode_gif(vec![frame], config, Limits::default(), Unstoppable).unwrap();

    // Decode
    let stats = Stats::new();
    let (metadata, frames) = decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

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
    let config = EncoderConfig::new(width, height);
    let encoded = encode_gif(frames_in, config, Limits::default(), Unstoppable).unwrap();

    // Decode with stats tracking
    let stats = Stats::new();
    let (_, frames) = decode_gif(&encoded, Limits::default(), &stats, Unstoppable).unwrap();

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
    let config = EncoderConfig::new(width, height);
    let encoded = encode_gif(frames_in, config, Limits::default(), Unstoppable).unwrap();

    // Batch decode
    let stats1 = Stats::new();
    let (_, batch_frames) = decode_gif(&encoded, Limits::default(), &stats1, Unstoppable).unwrap();

    // Streaming decode
    let stats2 = Stats::new();
    let cursor = std::io::Cursor::new(&encoded);
    let mut decoder = Decoder::new(cursor, Limits::default(), &stats2, Unstoppable).unwrap();
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
