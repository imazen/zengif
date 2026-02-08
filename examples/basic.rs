//! Basic usage example for zengif.
//!
//! Run with: cargo run --example basic

use enough::Unstoppable;
use zengif::{decode_gif, encode_gif, Decoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba};

fn main() {
    // Example 1: Create and encode a simple animation
    println!("Creating a 3-frame animation...");
    let animation = create_animation();
    println!("Encoded GIF size: {} bytes", animation.len());

    // Example 2: Decode the animation
    println!("\nDecoding animation...");
    decode_animation(&animation);

    // Example 3: Streaming decode
    println!("\nStreaming decode...");
    streaming_decode(&animation);

    // Example 4: Using limits for security
    println!("\nTesting limits...");
    test_limits(&animation);

    // Example 5: Memory tracking
    println!("\nMemory tracking...");
    track_memory(&animation);
}

/// Create a simple 3-frame animation (red -> green -> blue).
fn create_animation() -> Vec<u8> {
    let width = 64;
    let height = 64;

    // Create frames with solid colors
    let frames = vec![
        create_solid_frame(width, height, Rgba::rgb(255, 0, 0), 50), // Red, 500ms
        create_solid_frame(width, height, Rgba::rgb(0, 255, 0), 50), // Green, 500ms
        create_solid_frame(width, height, Rgba::rgb(0, 0, 255), 50), // Blue, 500ms
    ];

    // Configure encoder
    let config = EncoderConfig::new()
        .repeat(Repeat::Infinite) // Loop forever
        .use_transparency(true); // Enable transparency optimization

    // Encode using convenience function
    encode_gif(
        frames,
        width,
        height,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .expect("Failed to encode GIF")
}

/// Create a solid color frame.
fn create_solid_frame(width: u16, height: u16, color: Rgba, delay_cs: u16) -> FrameInput {
    let pixels = vec![color; width as usize * height as usize];
    FrameInput::new(width, height, delay_cs, pixels)
}

/// Decode an animation using the convenience function.
fn decode_animation(data: &[u8]) {
    let (metadata, frames, stats) =
        decode_gif(data, Limits::default(), &Unstoppable).expect("Failed to decode GIF");

    println!("  Dimensions: {}x{}", metadata.width, metadata.height);
    println!("  Frame count: {}", frames.len());
    println!("  Loop: {:?}", metadata.repeat);

    for (i, frame) in frames.iter().enumerate() {
        println!(
            "  Frame {}: {}x{}, delay={}cs",
            i, frame.width, frame.height, frame.delay
        );
    }

    println!("  Peak memory: {} bytes", stats.peak());
}

/// Streaming decode - process frames one at a time.
fn streaming_decode(data: &[u8]) {
    let limits = Limits::default();
    let cursor = std::io::Cursor::new(data);

    let mut decoder = Decoder::new(cursor, limits, &Unstoppable).expect("Failed to create decoder");

    println!("  Canvas: {}x{}", decoder.width(), decoder.height());

    let mut frame_count = 0;
    while let Some(frame) = decoder.next_frame().expect("Failed to read frame") {
        println!(
            "  Streamed frame {}: {} pixels, delay={}cs",
            frame_count,
            frame.pixels.len(),
            frame.delay
        );
        frame_count += 1;
    }
}

/// Demonstrate using limits for security.
fn test_limits(data: &[u8]) {
    // Strict limits for untrusted input
    let strict_limits = Limits::default()
        .max_dimensions(1024, 1024) // Max 1024x1024
        .max_total_pixels(1_000_000) // Max 1M pixels per frame
        .max_frame_count(100) // Max 100 frames
        .max_memory(50 * 1024 * 1024); // Max 50MB total

    let result = decode_gif(data, strict_limits, &Unstoppable);

    match result {
        Ok((metadata, frames, _stats)) => {
            println!(
                "  Passed limits: {}x{}, {} frames",
                metadata.width,
                metadata.height,
                frames.len()
            );
        }
        Err(e) => {
            println!("  Rejected by limits: {}", e);
        }
    }

    // Very restrictive limits (will reject our animation)
    let tiny_limits = Limits::default().max_dimensions(10, 10);

    let result = decode_gif(data, tiny_limits, &Unstoppable);

    match result {
        Ok(_) => println!("  Unexpectedly passed tiny limits"),
        Err(e) => println!("  Correctly rejected by tiny limits: {}", e.error()),
    }
}

/// Demonstrate memory tracking.
fn track_memory(data: &[u8]) {
    let cursor = std::io::Cursor::new(data);
    let limits = Limits::default();

    let mut decoder = Decoder::new(cursor, limits, &Unstoppable).expect("Failed to create decoder");

    println!(
        "  After decoder creation: {} bytes",
        decoder.stats().current()
    );

    while let Some(_frame) = decoder.next_frame().expect("Failed to read frame") {
        println!(
            "  During decode: current={}, peak={}",
            decoder.stats().current(),
            decoder.stats().peak()
        );
    }

    println!("  Final peak memory: {} bytes", decoder.stats().peak());
    println!("  Total allocations: {}", decoder.stats().alloc_count());
}
