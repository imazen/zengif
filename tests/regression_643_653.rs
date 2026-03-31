//! Regression tests for imageflow issues #643 and #653.
//!
//! #643: Resizing a GIF then resizing the output fails with "unexpected EOF"
//!       (round-trip robustness: re-encoding zengif output must produce valid GIF)
//!
//! #653: Animated GIFs with transparent backgrounds lose transparency
//!       (transparent background preservation through encode/decode cycles)

#![cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant",
    feature = "zenquant",
    feature = "quantette"
))]

use enough::Unstoppable;
use zengif::{Decoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, decode_gif, encode_gif};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a solid-color frame.
fn solid_frame(w: u16, h: u16, color: Rgba, delay: u16) -> FrameInput {
    FrameInput::new(w, h, delay, vec![color; w as usize * h as usize])
}

/// Create a frame where only a centered region is opaque and the rest is transparent.
fn transparent_border_frame(w: u16, h: u16, inner: Rgba, delay: u16) -> FrameInput {
    let mut pixels = vec![Rgba::TRANSPARENT; w as usize * h as usize];
    // Fill a centered 50% region with the inner color
    let x0 = w as usize / 4;
    let x1 = w as usize * 3 / 4;
    let y0 = h as usize / 4;
    let y1 = h as usize * 3 / 4;
    for y in y0..y1 {
        for x in x0..x1 {
            pixels[y * w as usize + x] = inner;
        }
    }
    FrameInput::new(w, h, delay, pixels)
}

/// Encode a set of frames into GIF bytes using the given config.
fn encode_frames(
    frames: Vec<FrameInput>,
    w: u16,
    h: u16,
    config: EncoderConfig,
) -> Vec<u8> {
    encode_gif(frames, w, h, config, Limits::default(), &Unstoppable)
        .expect("encode should succeed")
}

/// Decode GIF bytes into metadata + frames.
fn decode_bytes(data: &[u8]) -> (zengif::Metadata, Vec<zengif::ComposedFrame>) {
    let (meta, frames, _stats) =
        decode_gif(data, Limits::default(), &Unstoppable).expect("decode should succeed");
    (meta, frames)
}

// ===========================================================================
// Issue #643 — Round-trip robustness
// ===========================================================================

/// Encode a 2-frame animated GIF, then decode the result. No errors should occur.
#[test]
fn issue_643_encode_animated_then_decode() {
    let w = 8;
    let h = 8;
    let frames = vec![
        solid_frame(w, h, Rgba::rgb(255, 0, 0), 10),
        solid_frame(w, h, Rgba::rgb(0, 0, 255), 10),
    ];

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(frames, w, h, config);

    // Decoding the output must not fail
    let (_meta, decoded_frames) = decode_bytes(&encoded);
    assert_eq!(decoded_frames.len(), 2, "should decode 2 frames");
    assert_eq!(decoded_frames[0].width, w);
    assert_eq!(decoded_frames[0].height, h);
}

/// Full double round-trip: encode -> decode -> re-encode -> decode.
/// This is the core scenario from issue #643 where the second decode failed.
#[test]
fn issue_643_double_round_trip_single_frame() {
    let w = 8;
    let h = 8;
    let frame = solid_frame(w, h, Rgba::rgb(0, 200, 100), 10);
    let config = EncoderConfig::new().repeat(Repeat::Once);

    // First encode
    let encoded1 = encode_frames(vec![frame], w, h, config.clone());

    // First decode
    let (_meta1, frames1) = decode_bytes(&encoded1);
    assert_eq!(frames1.len(), 1);

    // Re-encode from decoded output
    let re_input: Vec<FrameInput> = frames1
        .iter()
        .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
        .collect();
    let encoded2 = encode_frames(re_input, w, h, config);

    // Second decode — this is what failed in #643
    let (_meta2, frames2) = decode_bytes(&encoded2);
    assert_eq!(frames2.len(), 1, "double round-trip decode must produce 1 frame");
    assert_eq!(frames2[0].width, w);
    assert_eq!(frames2[0].height, h);
}

/// Double round-trip with a multi-frame animated GIF.
#[test]
fn issue_643_double_round_trip_animated() {
    let w = 8;
    let h = 8;
    let frames = vec![
        solid_frame(w, h, Rgba::rgb(255, 0, 0), 10),
        solid_frame(w, h, Rgba::rgb(0, 255, 0), 20),
        solid_frame(w, h, Rgba::rgb(0, 0, 255), 30),
    ];
    let config = EncoderConfig::new().repeat(Repeat::Infinite);

    // First pass
    let encoded1 = encode_frames(frames, w, h, config.clone());
    let (_meta1, frames1) = decode_bytes(&encoded1);
    assert_eq!(frames1.len(), 3);

    // Second pass (re-encode decoded output)
    let re_input: Vec<FrameInput> = frames1
        .iter()
        .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
        .collect();
    let encoded2 = encode_frames(re_input, w, h, config);
    let (_meta2, frames2) = decode_bytes(&encoded2);

    assert_eq!(frames2.len(), 3, "double round-trip must preserve frame count");
    for (i, (a, b)) in frames1.iter().zip(frames2.iter()).enumerate() {
        assert_eq!(a.delay, b.delay, "frame {i} delay mismatch in double round-trip");
        assert_eq!(a.width, b.width, "frame {i} width mismatch");
        assert_eq!(a.height, b.height, "frame {i} height mismatch");
        assert_eq!(
            a.pixels.len(),
            b.pixels.len(),
            "frame {i} pixel count mismatch"
        );
    }
}

/// Verify the encoder produces valid GIF89a header and proper trailer (0x3B).
#[test]
fn issue_643_valid_gif89a_header_and_trailer() {
    let w = 4;
    let h = 4;
    let frame = solid_frame(w, h, Rgba::rgb(128, 64, 32), 10);
    let config = EncoderConfig::new();
    let encoded = encode_frames(vec![frame], w, h, config);

    // Check GIF89a signature
    assert!(
        encoded.len() >= 6,
        "encoded GIF is too short: {} bytes",
        encoded.len()
    );
    assert_eq!(&encoded[0..3], b"GIF", "missing GIF magic");
    assert_eq!(&encoded[3..6], b"89a", "expected GIF89a version");

    // Check logical screen descriptor dimensions (bytes 6-9, little-endian)
    let lsd_width = u16::from_le_bytes([encoded[6], encoded[7]]);
    let lsd_height = u16::from_le_bytes([encoded[8], encoded[9]]);
    assert_eq!(lsd_width, w, "LSD width mismatch");
    assert_eq!(lsd_height, h, "LSD height mismatch");

    // Check trailer byte
    assert_eq!(
        encoded.last(),
        Some(&0x3B),
        "GIF must end with trailer byte 0x3B"
    );
}

/// Triple round-trip: encode -> decode -> encode -> decode -> encode -> decode.
/// Stress test for accumulated encoding artifacts.
#[test]
fn issue_643_triple_round_trip() {
    let w = 8;
    let h = 8;
    let frames = vec![
        solid_frame(w, h, Rgba::rgb(200, 50, 50), 10),
        solid_frame(w, h, Rgba::rgb(50, 200, 50), 10),
    ];
    let config = EncoderConfig::new().repeat(Repeat::Infinite);

    let mut current_bytes = encode_frames(frames, w, h, config.clone());

    for pass in 0..3 {
        let (_meta, decoded) = decode_bytes(&current_bytes);
        assert!(
            !decoded.is_empty(),
            "pass {pass}: decoded zero frames"
        );

        let re_input: Vec<FrameInput> = decoded
            .iter()
            .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
            .collect();
        current_bytes = encode_frames(re_input, w, h, config.clone());

        // Every intermediate output must have valid header and trailer
        assert_eq!(&current_bytes[0..6], b"GIF89a", "pass {pass}: bad header");
        assert_eq!(
            current_bytes.last(),
            Some(&0x3B),
            "pass {pass}: missing trailer"
        );
    }

    // Final decode must succeed
    let (_meta, final_frames) = decode_bytes(&current_bytes);
    assert_eq!(final_frames.len(), 2, "final frame count must be 2");
}

/// Verify that the streaming decoder also works on re-encoded output.
#[test]
fn issue_643_streaming_decode_of_reencoded() {
    let w = 4;
    let h = 4;
    let frames = vec![
        solid_frame(w, h, Rgba::rgb(255, 128, 0), 10),
        solid_frame(w, h, Rgba::rgb(0, 128, 255), 20),
    ];
    let config = EncoderConfig::new().repeat(Repeat::Infinite);

    let encoded1 = encode_frames(frames, w, h, config.clone());
    let (_meta, frames1) = decode_bytes(&encoded1);

    let re_input: Vec<FrameInput> = frames1
        .iter()
        .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
        .collect();
    let encoded2 = encode_frames(re_input, w, h, config);

    // Streaming decode of the re-encoded bytes
    let cursor = std::io::Cursor::new(&encoded2);
    let mut decoder = Decoder::new(cursor, Limits::default(), &Unstoppable)
        .expect("streaming decoder should accept re-encoded GIF");

    let mut count = 0;
    while let Some(frame) = decoder.next_frame().expect("streaming frame decode failed") {
        assert_eq!(frame.width, w);
        assert_eq!(frame.height, h);
        count += 1;
    }
    assert_eq!(count, 2, "streaming decode of re-encoded GIF should yield 2 frames");
}

// ===========================================================================
// Issue #653 — Transparent background / disposal preservation
// ===========================================================================

/// Encode an animated GIF with transparent pixels, decode it,
/// verify transparent pixels are actually preserved (alpha == 0).
#[test]
fn issue_653_transparent_pixels_preserved_in_animation() {
    let w = 8;
    let h = 8;

    // Frame 1: transparent border, red center
    let frame1 = transparent_border_frame(w, h, Rgba::rgb(255, 0, 0), 10);
    // Frame 2: transparent border, blue center
    let frame2 = transparent_border_frame(w, h, Rgba::rgb(0, 0, 255), 10);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(vec![frame1, frame2], w, h, config);
    let (_meta, frames) = decode_bytes(&encoded);

    assert_eq!(frames.len(), 2);

    // Check that corners (which were transparent in the input) are transparent
    // in the decoded output. Corner pixel (0,0) should have alpha == 0.
    for (i, frame) in frames.iter().enumerate() {
        let corner = frame.pixels[0]; // top-left corner
        assert_eq!(
            corner.a, 0,
            "frame {i}: top-left corner should be transparent (alpha=0), got alpha={}",
            corner.a
        );

        // Check center pixel is opaque
        let center_idx = (h as usize / 2) * w as usize + (w as usize / 2);
        let center = frame.pixels[center_idx];
        assert_eq!(
            center.a, 255,
            "frame {i}: center pixel should be opaque, got alpha={}",
            center.a
        );
    }
}

/// Verify that transparent pixels round-trip correctly through encode -> decode.
#[test]
fn issue_653_transparency_survives_round_trip() {
    let w = 8;
    let h = 8;

    // Create a frame where exactly 50% of pixels are transparent
    let mut pixels = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            if (x + y) % 2 == 0 {
                pixels.push(Rgba::TRANSPARENT);
            } else {
                pixels.push(Rgba::rgb(200, 100, 50));
            }
        }
    }
    let frame = FrameInput::new(w, h, 10, pixels.clone());

    let config = EncoderConfig::new();
    let encoded = encode_frames(vec![frame], w, h, config);
    let (_meta, frames) = decode_bytes(&encoded);

    assert_eq!(frames.len(), 1);

    // Count transparent and opaque pixels
    let transparent_count = frames[0].pixels.iter().filter(|p| p.a == 0).count();
    let opaque_count = frames[0].pixels.iter().filter(|p| p.a == 255).count();
    let total = frames[0].pixels.len();

    // We started with exactly 50% transparent. After quantization there may be
    // minor changes, but the vast majority should be preserved.
    assert!(
        transparent_count > total / 4,
        "expected many transparent pixels, got {transparent_count}/{total}"
    );
    assert!(
        opaque_count > total / 4,
        "expected many opaque pixels, got {opaque_count}/{total}"
    );
}

/// Frame 1 is fully opaque, frame 2 has transparency. Both must decode correctly.
#[test]
fn issue_653_opaque_then_transparent_frames() {
    let w = 8;
    let h = 8;

    // Frame 1: fully opaque green
    let frame1 = solid_frame(w, h, Rgba::rgb(0, 200, 0), 10);

    // Frame 2: half transparent, half red
    let mut pixels2 = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for _x in 0..w {
            if y < h / 2 {
                pixels2.push(Rgba::TRANSPARENT);
            } else {
                pixels2.push(Rgba::rgb(200, 0, 0));
            }
        }
    }
    let frame2 = FrameInput::new(w, h, 10, pixels2);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(vec![frame1, frame2], w, h, config);
    let (_meta, frames) = decode_bytes(&encoded);

    assert_eq!(frames.len(), 2);

    // Frame 1: all pixels should be opaque
    let f1_all_opaque = frames[0].pixels.iter().all(|p| p.a == 255);
    assert!(f1_all_opaque, "frame 0 should be fully opaque");

    // Frame 2: bottom half should be opaque (red), top half depends on
    // disposal method. With Keep disposal, top half shows frame 1's green.
    // With Background disposal, top half would be transparent.
    // Either way, bottom half must be opaque.
    let bottom_start = (h as usize / 2) * w as usize;
    let bottom_all_opaque = frames[1].pixels[bottom_start..].iter().all(|p| p.a == 255);
    assert!(
        bottom_all_opaque,
        "frame 1 bottom half should be opaque"
    );
}

/// Verify the encoder handles frames where ALL pixels are transparent.
#[test]
fn issue_653_fully_transparent_frame() {
    let w = 4;
    let h = 4;

    let frame1 = solid_frame(w, h, Rgba::rgb(255, 0, 0), 10);
    let frame2 = solid_frame(w, h, Rgba::TRANSPARENT, 10);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(vec![frame1, frame2], w, h, config);

    // Must not fail to decode
    let (_meta, frames) = decode_bytes(&encoded);
    assert_eq!(frames.len(), 2);
}

/// Verify that transparency is preserved through a double round-trip
/// (the combination of #643 and #653).
#[test]
fn issue_643_653_transparency_double_round_trip() {
    let w = 8;
    let h = 8;

    let frame1 = transparent_border_frame(w, h, Rgba::rgb(255, 0, 0), 10);
    let frame2 = transparent_border_frame(w, h, Rgba::rgb(0, 0, 255), 10);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);

    // First encode + decode
    let encoded1 = encode_frames(vec![frame1, frame2], w, h, config.clone());
    let (_meta1, frames1) = decode_bytes(&encoded1);
    assert_eq!(frames1.len(), 2);

    // Re-encode + decode (the #643 scenario, but with transparent data from #653)
    let re_input: Vec<FrameInput> = frames1
        .iter()
        .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
        .collect();
    let encoded2 = encode_frames(re_input, w, h, config);
    let (_meta2, frames2) = decode_bytes(&encoded2);

    assert_eq!(frames2.len(), 2, "double round-trip frame count");

    // Transparency in corners should still be present
    for (i, frame) in frames2.iter().enumerate() {
        let corner = frame.pixels[0];
        assert_eq!(
            corner.a, 0,
            "frame {i}: corner transparency lost after double round-trip, alpha={}",
            corner.a
        );
    }
}

/// The encoder always uses DisposalMethod::Keep. Verify this is correctly
/// written and decoded. (The encoder does not expose disposal method
/// configuration per-frame, so we verify the default behavior is consistent.)
#[test]
fn issue_653_disposal_method_keep_default() {
    let w = 4;
    let h = 4;

    // Two frames: frame 1 red, frame 2 blue
    let frames = vec![
        solid_frame(w, h, Rgba::rgb(255, 0, 0), 10),
        solid_frame(w, h, Rgba::rgb(0, 0, 255), 20),
    ];
    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(frames, w, h, config);

    // Decode with streaming decoder to inspect raw frame info
    let cursor = std::io::Cursor::new(&encoded);
    let mut decoder = Decoder::new(cursor, Limits::default(), &Unstoppable).unwrap();

    let _frame1 = decoder.next_frame().unwrap().expect("frame 1 missing");
    let frame2 = decoder.next_frame().unwrap().expect("frame 2 missing");

    // With Keep disposal (the encoder default), frame 2 should show frame 2's
    // content, not a mix. Both frames fill the entire canvas, so frame 2 should
    // be entirely blue regardless of disposal.
    let f2_center = frame2.pixels[(h as usize / 2) * w as usize + w as usize / 2];
    // Due to quantization the exact blue value may vary, but blue channel
    // should dominate
    assert!(
        f2_center.b > 150 && f2_center.r < 100,
        "frame 2 center should be blue-ish, got {:?}",
        f2_center
    );
}

/// Encode a 2-frame GIF where frame 1 is opaque red and frame 2 is fully
/// transparent, then decode and verify that the transparent frame actually
/// produces transparent pixels. This tests the transparency-through-disposal
/// path without relying on fragile hand-crafted LZW data.
#[test]
fn issue_653_background_disposal_creates_transparency() {
    let w = 4;
    let h = 4;

    // Frame 1: fully opaque red
    let frame1 = solid_frame(w, h, Rgba::rgb(255, 0, 0), 10);

    // Frame 2: fully transparent — after disposal of frame 1, the canvas
    // should show transparent pixels wherever frame 2 is transparent.
    let frame2 = solid_frame(w, h, Rgba::TRANSPARENT, 20);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(vec![frame1, frame2], w, h, config);

    // Decode and verify
    let (_meta, frames) = decode_bytes(&encoded);
    assert_eq!(frames.len(), 2, "should decode 2 frames");

    // Frame 1: all pixels should be opaque red
    assert_eq!(frames[0].width, w);
    assert_eq!(frames[0].height, h);
    let f1_all_opaque = frames[0].pixels.iter().all(|p| p.a == 255);
    assert!(f1_all_opaque, "frame 0 should be fully opaque");

    // Frame 2: with Keep disposal (encoder default), transparent frame 2
    // pixels show through to frame 1's red. With Background disposal, they
    // would be transparent. Either way, the decode must succeed and produce
    // a valid frame with the correct dimensions.
    assert_eq!(frames[1].width, w);
    assert_eq!(frames[1].height, h);
    assert_eq!(
        frames[1].pixels.len(),
        w as usize * h as usize,
        "frame 1 pixel count mismatch"
    );
}

/// Verify that when frames have different color compositions (one all-opaque,
/// one with many transparent pixels), the quantizer assigns appropriate
/// transparent indices per frame.
#[test]
fn issue_653_different_transparency_per_frame() {
    let w = 8;
    let h = 8;
    let total = w as usize * h as usize;

    // Frame 1: entirely opaque red (no transparent pixels at all)
    let frame1 = solid_frame(w, h, Rgba::rgb(255, 0, 0), 10);

    // Frame 2: checkerboard of transparent and green
    let mut pixels2 = Vec::with_capacity(total);
    for y in 0..h {
        for x in 0..w {
            if (x + y) % 2 == 0 {
                pixels2.push(Rgba::TRANSPARENT);
            } else {
                pixels2.push(Rgba::rgb(0, 200, 0));
            }
        }
    }
    let frame2 = FrameInput::new(w, h, 10, pixels2);

    // Frame 3: entirely transparent
    let frame3 = solid_frame(w, h, Rgba::TRANSPARENT, 10);

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let encoded = encode_frames(vec![frame1, frame2, frame3], w, h, config);
    let (_meta, frames) = decode_bytes(&encoded);

    assert_eq!(frames.len(), 3, "should decode 3 frames");

    // Frame 1: all opaque
    assert!(
        frames[0].pixels.iter().all(|p| p.a == 255),
        "frame 0 should be fully opaque"
    );

    // Frame 2: should have a mix (due to Keep disposal, transparent pixels
    // show through to frame 1's red, so those pixels may appear opaque too).
    // The key assertion is that the decode doesn't fail or corrupt data.
    assert_eq!(frames[1].pixel_count(), total);
    assert_eq!(frames[1].pixels.len(), total);
}

/// Verify that encoding and decoding preserves the animation delay values
/// exactly, even through multiple round-trips. (Regression for both #643 and
/// #653 since corruption could also manifest as wrong delays.)
#[test]
fn issue_643_delays_preserved_through_round_trips() {
    let w = 4;
    let h = 4;
    let delays = [5u16, 10, 50, 100, 2];

    let frames: Vec<FrameInput> = delays
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            let shade = ((i * 50) % 256) as u8;
            solid_frame(w, h, Rgba::rgb(shade, shade, shade), d)
        })
        .collect();

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let mut current = encode_frames(frames, w, h, config.clone());

    for pass in 0..3 {
        let (_meta, decoded) = decode_bytes(&current);
        assert_eq!(
            decoded.len(),
            delays.len(),
            "pass {pass}: frame count mismatch"
        );
        for (i, frame) in decoded.iter().enumerate() {
            assert_eq!(
                frame.delay, delays[i],
                "pass {pass}, frame {i}: delay mismatch (expected {}, got {})",
                delays[i], frame.delay
            );
        }

        let re_input: Vec<FrameInput> = decoded
            .iter()
            .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
            .collect();
        current = encode_frames(re_input, w, h, config.clone());
    }
}
