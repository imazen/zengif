//! Regressions for sweep issue #14 (2026-08-26 ultracode sweep): frame-diff
//! transparency markers encoded against palettes with no transparent slot
//! silently repainted unchanged regions, and the quantette backend declared
//! an opaque palette entry as the GIF transparent index. Every test here
//! DECODES the encoder output — the pre-existing tests on these paths only
//! checked magic bytes and sizes, which is exactly how the corruption
//! stayed green.
//!
//! Every entry point exercised here (`encode_gif`, `decode_gif`, `EncoderConfig`)
//! is gated behind the `std` feature, so this whole test compiles to nothing
//! without it — same as `cancellation.rs`, `corpus.rs`, `malformed.rs` and the
//! rest of the std-only suites. Without the gate `cargo test
//! --no-default-features --lib --tests` fails to resolve the imports, which is
//! what broke the `Feature permutations` job.
#![cfg(feature = "std")]

use enough::Unstoppable;
use zengif::{EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, decode_gif, encode_gif};

/// Two frames differing at exactly two far-apart pixels, so the frame-diff
/// bounding rectangle spans nearly the whole canvas and is FULL of unchanged
/// pixels — the pixels the differ encodes as transparency markers. (A solid
/// changed region would leave no markers inside the rect and let composition
/// hide the bug.)
const CHANGED: [(u16, u16); 2] = [(2, 2), (61, 61)];

fn two_frames(w: u16, h: u16) -> Vec<FrameInput> {
    let mut f1 = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            f1.push(Rgba::rgb(
                (x * 3 % 200) as u8 + 20,
                (y * 5 % 200) as u8 + 20,
                ((x + y) * 7 % 200) as u8 + 20,
            ));
        }
    }
    let mut f2 = f1.clone();
    for &(x, y) in &CHANGED {
        f2[y as usize * w as usize + x as usize] = Rgba::rgb(250, 250, 250);
    }
    vec![FrameInput::new(w, h, 10, f1), FrameInput::new(w, h, 10, f2)]
}

/// The core frame-differencing invariant, independent of quantizer loss:
/// whatever the decoder shows for frame 1's static region must be EXACTLY
/// what it shows for frame 2's static region (an unchanged pixel is either
/// re-emitted identically or diffed out as transparent — never repainted).
fn assert_static_region_survives(gif: &[u8], w: u16, h: u16, ctx: &str) {
    let (_, frames, _) = decode_gif(gif, Limits::default(), &Unstoppable)
        .unwrap_or_else(|e| panic!("{ctx}: decode failed: {e}"));
    assert_eq!(frames.len(), 2, "{ctx}: frame count");
    let (a, b) = (&frames[0], &frames[1]);
    let mut wrong = 0usize;
    for y in 0..h {
        for x in 0..w {
            if CHANGED.contains(&(x, y)) {
                continue;
            }
            let i = y as usize * w as usize + x as usize;
            if a.pixels[i] != b.pixels[i] {
                wrong += 1;
                if wrong <= 3 {
                    eprintln!(
                        "{ctx}: unchanged pixel ({x},{y}) repainted: {:?} -> {:?}",
                        a.pixels[i], b.pixels[i]
                    );
                }
            }
        }
    }
    assert_eq!(
        wrong, 0,
        "{ctx}: {wrong} unchanged pixels repainted — diff markers were \
         encoded onto opaque palette entries (issue #14)"
    );
    // Sanity: the two changed pixels did change.
    for &(x, y) in &CHANGED {
        let i = y as usize * w as usize + x as usize;
        assert_ne!(a.pixels[i], b.pixels[i], "{ctx}: change at ({x},{y}) lost");
    }
}

/// Supplies no per-frame palette, so the encoder must quantize — which needs a
/// quantizer backend. The condition below is the exact inverse of the
/// `prepare_frame_passthrough` gate in `src/encode/encoder.rs`, whose
/// no-quantizer arm returns `QuantizationFailed { "no quantizer feature enabled
/// and frame has no palette" }`. `default` carries `zenquant`, so this runs in
/// every normal build and in each per-quantizer CI job; only the deliberately
/// quantizer-less `--no-default-features --features std` permutation skips it.
/// The sibling test below covers the same regression with a caller-supplied
/// palette and therefore runs in every permutation.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
#[test]
fn diff_static_region_survives_shared_palette() {
    let (w, h) = (64u16, 64u16);
    let config = EncoderConfig::new()
        .repeat(Repeat::Once)
        .use_transparency(true);
    let gif = encode_gif(
        two_frames(w, h),
        w,
        h,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();
    assert_static_region_survives(&gif, w, h, "shared-palette");
}

#[test]
fn diff_static_region_survives_passthrough_opaque_palette() {
    let (w, h) = (64u16, 64u16);
    // Small fully-opaque caller palette (as a decoder would supply).
    let palette = Palette::from_rgb_bytes(&[
        0, 0, 0, 255, 255, 255, 200, 40, 40, 40, 200, 40, 40, 40, 200, 200, 200, 40,
    ]);
    let mut frames = two_frames(w, h);
    for f in &mut frames {
        f.palette = Some(palette.clone());
    }
    let config = EncoderConfig::new()
        .repeat(Repeat::Once)
        .use_transparency(true);
    let gif = encode_gif(frames, w, h, config, Limits::default(), &Unstoppable).unwrap();
    assert_static_region_survives(&gif, w, h, "passthrough-opaque-palette");
}

/// quantette strips alpha before clustering; pre-fix it declared the palette
/// entry nearest to (0,0,0) as the GIF transparent index, making every
/// legitimately dark pixel see-through.
#[cfg(feature = "quantette")]
#[test]
fn quantette_dark_pixels_stay_opaque() {
    let (w, h) = (32u16, 32u16);
    let mut px = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            if x < w / 2 {
                px.push(Rgba::new(8, 8, 8, 255)); // near-black, OPAQUE
            } else if y < h / 2 {
                px.push(Rgba::new(0, 0, 0, 0)); // transparent hole
            } else {
                px.push(Rgba::rgb(220, 40, 40));
            }
        }
    }
    let config = EncoderConfig::new()
        .repeat(Repeat::Once)
        .use_transparency(true)
        .quantizer_preference(vec![zengif::QuantizerBackend::Quantette]);
    let gif = encode_gif(
        vec![FrameInput::new(w, h, 10, px)],
        w,
        h,
        config,
        Limits::default(),
        &Unstoppable,
    )
    .unwrap();
    let (_, frames, _) = decode_gif(&gif, Limits::default(), &Unstoppable).unwrap();
    let f = &frames[0];
    for y in 0..h as usize {
        for x in 0..(w / 2) as usize {
            let p = f.pixels[y * w as usize + x];
            assert_eq!(
                p.a, 255,
                "dark opaque pixel ({x},{y}) became transparent (issue #14): {p:?}"
            );
        }
    }
    // The genuine hole survives as transparent.
    let hole = f.pixels[(w as usize) - 1];
    assert_eq!(hole.a, 0, "transparent hole was flattened: {hole:?}");
}
