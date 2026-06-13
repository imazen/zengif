#![cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
//! Native grayscale fast path (issue #4).
//!
//! When every pixel is gray (`r == g == b`), the encoder builds an exact 8-bit
//! gray palette instead of running the general RGBA quantizer. These tests
//! exercise that path through the public API and assert the property that only
//! the exact path can satisfy: a grayscale image round-trips **losslessly**
//! (the default quantizer, at quality 80 + dithering 0.5, would not).

use enough::Unstoppable;
use zengif::{
    EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Rgba, decode_gif, encode_gif,
};

fn gray(v: u8) -> Rgba {
    Rgba::rgb(v, v, v)
}

/// A 16×16 frame walking through all 256 gray levels exactly once.
fn full_range_gray_frame(delay: u16) -> FrameInput {
    let pixels: Vec<Rgba> = (0..256u16).map(|v| gray(v as u8)).collect();
    FrameInput::new(16, 16, delay, pixels)
}

#[test]
fn single_frame_grayscale_is_lossless() {
    let frame = full_range_gray_frame(0);
    let original = frame.pixels.clone();

    // Default config: shared_palette = true, quality 80, dithering 0.5.
    // If the RGBA quantizer ran, dithering alone would perturb these pixels.
    let encoded = encode_gif(
        vec![frame],
        16,
        16,
        EncoderConfig::new().repeat(Repeat::Once),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 1);

    // Every pixel reproduced exactly — proof the exact gray path was taken.
    assert_eq!(
        frames[0].pixels, original,
        "grayscale single frame must round-trip losslessly"
    );
}

#[test]
fn decoded_palette_is_grayscale() {
    let encoded = encode_gif(
        vec![full_range_gray_frame(0)],
        16,
        16,
        EncoderConfig::new().repeat(Repeat::Once),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    let palette = frames[0]
        .palette
        .as_ref()
        .expect("decoded frame should expose its palette");

    for c in palette.colors() {
        assert!(
            c.r == c.g && c.g == c.b,
            "palette entry {c:?} is not a true gray"
        );
    }
}

#[test]
fn grayscale_animation_round_trips_losslessly() {
    // Two frames that differ in a sub-region, so frame differencing (and thus
    // the reserved transparent slot in the shared gray palette) is exercised.
    let a = vec![gray(40); 8 * 8];
    let mut b = vec![gray(40); 8 * 8];
    for y in 2..6 {
        for x in 2..6 {
            b[y * 8 + x] = gray(200);
        }
    }
    let frame_a = FrameInput::new(8, 8, 10, a.clone());
    let frame_b = FrameInput::new(8, 8, 10, b.clone());

    let encoded = encode_gif(
        vec![frame_a, frame_b],
        8,
        8,
        EncoderConfig::new(),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (meta, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(meta.width, 8);

    // Composited frames reconstruct the originals exactly (disposal=Keep +
    // 1-bit transparency restore the unchanged pixels).
    assert_eq!(frames[0].pixels, a, "frame 0 must match exactly");
    assert_eq!(frames[1].pixels, b, "frame 1 must match exactly");

    // The shared palette across both gray frames must itself be grayscale.
    let palette = frames[1].palette.as_ref().unwrap();
    for c in palette.colors() {
        assert!(c.r == c.g && c.g == c.b, "shared palette must be grayscale");
    }
}

#[test]
fn grayscale_lossless_with_per_frame_palette() {
    // shared_palette = false routes each frame through the per-frame fast path
    // (a different code branch than the shared/flush path above).
    let frame = full_range_gray_frame(0);
    let original = frame.pixels.clone();

    let encoded = encode_gif(
        vec![frame],
        16,
        16,
        EncoderConfig::new()
            .shared_palette(false)
            .repeat(Repeat::Once),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(
        frames[0].pixels, original,
        "per-frame grayscale path must also be lossless"
    );
}

#[test]
fn full_range_gray_survives_midstream_byte_cap_flush() {
    // Regression for the logan-voss bug (issue #4 corpus run): a single
    // 256-level grayscale frame that exceeds `max_buffer_bytes` flushes
    // mid-stream. The fast path must still engage losslessly — not bail to the
    // lossy quantizer because a *speculative* transparent slot pushed the
    // palette to 257 entries. Forcing a tiny buffer reproduces the >64 MB
    // single-frame flush on a 16×16 image.
    let frame = full_range_gray_frame(0); // all 256 gray levels
    let original = frame.pixels.clone();

    let encoded = encode_gif(
        vec![frame],
        16,
        16,
        EncoderConfig::new().max_buffer_bytes(1),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(
        frames[0].pixels, original,
        "256-level grayscale must stay lossless even when it flushes mid-stream"
    );

    // Discriminator: the exact gray path emits all 256 levels as an ascending
    // [i,i,i] palette. The quantizer fallback (the bug) would select/order
    // colors differently, so this proves the fast path actually engaged rather
    // than relying on the quantizer happening to be lossless on tiny input.
    let palette = frames[0].palette.as_ref().expect("frame palette");
    let expected: Vec<Rgba> = (0..256u16).map(|i| gray(i as u8)).collect();
    assert_eq!(
        palette.colors(),
        expected.as_slice(),
        "expected the exact ascending 256-gray fast-path palette"
    );
}

#[test]
fn gray_then_color_frame_keeps_its_color() {
    // Force gray mode to engage on frame 0 (buffer of 1 → it flushes before the
    // color frame is seen), then feed a color frame. The hybrid fallback must
    // give that frame a real per-frame color palette — never silently
    // desaturate it through the committed gray palette.
    let gray_frame = FrameInput::new(8, 8, 10, vec![gray(100); 64]);
    let red_frame = FrameInput::new(8, 8, 10, vec![Rgba::rgb(220, 10, 10); 64]);

    let encoded = encode_gif(
        vec![gray_frame, red_frame],
        8,
        8,
        EncoderConfig::new().max_buffer_frames(1),
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 2);

    let p0 = frames[0].pixels[0];
    assert!(
        p0.r == p0.g && p0.g == p0.b,
        "frame 0 should stay gray, got {p0:?}"
    );

    let p1 = frames[1].pixels[0];
    assert!(
        p1.r > 150 && p1.g < 80 && p1.b < 80,
        "color frame must keep its color through the hybrid fallback, got {p1:?}"
    );
}

/// The gray fast path must never be *larger* than the optimal lossless
/// quantizer (quantizr at q100, dithering 0). Proven byte-identical across the
/// imazen-26 grayscale subset (28/28); this guards that 0%-compression-loss
/// property — at matched (lossless) fidelity — against regressions.
#[cfg(feature = "quantizr")]
#[test]
fn gray_path_never_larger_than_lossless_quantizer() {
    use zengif::{QuantizrQuantizer, encode_gif_with_quantizer};

    // Diverse grayscale content: smooth gradient, sparse "document", and a
    // deterministic high-entropy field — the three regimes that stress LZW.
    type Pat = fn(usize) -> u8;
    let cases: [(&str, Pat); 3] = [
        ("gradient", |i| (i % 256) as u8),
        ("document", |i| if i % 37 < 3 { 0 } else { 255 }),
        ("hi_entropy", |i| (i.wrapping_mul(2654435761) >> 13) as u8),
    ];
    let (w, h) = (128u16, 128u16);
    for (name, f) in cases {
        let px: Vec<Rgba> = (0..w as usize * h as usize).map(|i| gray(f(i))).collect();

        let g = encode_gif(
            vec![FrameInput::new(w, h, 0, px.clone())],
            w,
            h,
            EncoderConfig::new().repeat(Repeat::Once),
            Limits::none(),
            &Unstoppable,
        )
        .unwrap();

        let qz = encode_gif_with_quantizer(
            vec![FrameInput::new(w, h, 0, px.clone())],
            w,
            h,
            EncoderConfig::new()
                .repeat(Repeat::Once)
                .quality(100)
                .dithering(0.0),
            Limits::none(),
            &Unstoppable,
            QuantizrQuantizer::new(),
        )
        .unwrap();

        assert!(
            g.len() <= qz.len(),
            "{name}: gray path ({} bytes) must not exceed lossless quantizr ({} bytes)",
            g.len(),
            qz.len()
        );
        let (_, frames, _) = decode_gif(&g, Limits::none(), &Unstoppable).unwrap();
        assert_eq!(frames[0].pixels, px, "{name}: gray path must be lossless");
    }
}

#[test]
fn streaming_grayscale_round_trips_losslessly() {
    // Drive the streaming Encoder directly (the path codec callers use).
    let frame = full_range_gray_frame(0);
    let original = frame.pixels.clone();

    let config = EncoderConfig::new().repeat(Repeat::Once);
    let limits = Limits::default();
    let mut encoder = EncodeRequest::new(&config, 16, 16)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .unwrap();
    encoder.add_frame(frame).unwrap();
    let encoded = encoder.finish().unwrap();

    let (_, frames, _) = decode_gif(&encoded, Limits::default(), &Unstoppable).unwrap();
    assert_eq!(frames[0].pixels, original);
}
