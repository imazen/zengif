//! GIF animations streamed through `AnimationFrameEncoder::begin`: each
//! frame is written as it is pushed (lag 0), or, with a shared palette, once
//! the palette is built (lag `max_buffer_frames - 1`). The streamed file is
//! byte-identical to the same encode without `begin`.

// Shared palettes and RGBA input need a quantizer.
#![cfg(all(
    feature = "std",
    any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    )
))]

use zencodec::encode::{AnimationFrameEncoder, AnimationOutputTiming, EncodeJob, EncoderConfig};
use zengif::{GifDecoderConfig, GifEncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};

const RGBA8: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
const W: u32 = 24;
const H: u32 = 16;

/// A block moving across a two-colour background.
fn frames(n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|f| {
            (0..W * H)
                .flat_map(|i| {
                    let (x, y) = ((i % W) as usize, (i / W) as usize);
                    if x >= f % 20 && x < f % 20 + 4 && y < 8 {
                        [220, 30, 30, 255]
                    } else if y < H as usize / 2 {
                        [20, 40, 200, 255]
                    } else {
                        [240, 240, 240, 255]
                    }
                })
                .collect()
        })
        .collect()
}

/// Encode `frames`; with `declare`, call `begin` first (with that frame
/// count). Drains after every frame. Returns the timing, the file, and the
/// bytes available after `begin` and after each frame.
fn encode(
    cfg: &GifEncoderConfig,
    frames: &[Vec<u8>],
    declare: Option<Option<u32>>,
) -> (Option<AnimationOutputTiming>, Vec<u8>, Vec<usize>) {
    let mut a = cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .animation_frame_encoder()
        .unwrap();
    let timing = declare.map(|n| a.begin(W, H, RGBA8, n).unwrap());
    let mut out = Vec::new();
    let mut seen = vec![a.pending_output().len()];
    for (i, f) in frames.iter().enumerate() {
        let s = PixelSlice::new(f, W, H, W as usize * 4, RGBA8).unwrap();
        a.push_frame(s, 40 + i as u32 * 10, None).unwrap();
        seen.push(out.len() + a.pending_output().len());
        let mut chunk = Vec::new();
        a.take_output(&mut chunk);
        out.extend_from_slice(&chunk);
    }
    out.extend_from_slice(a.finish(None).unwrap().data());
    (timing, out, seen)
}

#[test]
fn per_frame_palettes_stream_each_frame() {
    let cfg = GifEncoderConfig::new().with_shared_palette(false);
    let fr = frames(6);
    for declare in [Some(6), None] {
        let (timing, streamed, seen) = encode(&cfg, &fr, Some(declare));
        assert_eq!(timing, Some(AnimationOutputTiming::streaming(0)));
        assert!(seen.windows(2).all(|p| p[1] > p[0]), "{seen:?}");
        let (_, whole, _) = encode(&cfg, &fr, None);
        assert_eq!(streamed, whole);
    }
}

#[test]
fn a_shared_palette_holds_frames_until_it_is_built() {
    let cfg = GifEncoderConfig::new()
        .with_shared_palette(true)
        .with_max_buffer_frames(3);
    let fr = frames(8);
    let (timing, streamed, seen) = encode(&cfg, &fr, Some(Some(8)));
    assert_eq!(timing, Some(AnimationOutputTiming::streaming(2)));
    // Nothing until the third frame builds the palette, then every frame.
    assert_eq!(&seen[..3], &[0, 0, 0]);
    assert!(seen[3..].windows(2).all(|p| p[1] > p[0]), "{seen:?}");
    let (_, whole, _) = encode(&cfg, &fr, None);
    assert_eq!(streamed, whole);
}

#[test]
fn an_animation_that_fits_the_palette_buffer_comes_at_finish() {
    let cfg = GifEncoderConfig::new();
    let begin = |n| {
        cfg.clone()
            .job()
            .animation_frame_encoder()
            .unwrap()
            .begin(W, H, RGBA8, n)
            .unwrap()
    };
    // The default shared palette buffers 32 frames.
    assert_eq!(begin(Some(5)), AnimationOutputTiming::AT_FINISH);
    assert_eq!(begin(None), AnimationOutputTiming::streaming(31));
}

#[test]
fn the_declaration_binds() {
    let cfg = GifEncoderConfig::new().with_shared_palette(false);
    let fr = frames(3);
    fn slice(f: &[u8]) -> PixelSlice<'_> {
        PixelSlice::new(f, W, H, W as usize * 4, RGBA8).unwrap()
    }
    let job = || cfg.clone().job().animation_frame_encoder().unwrap();
    let mut a = job();
    a.begin(W, H, RGBA8, Some(2)).unwrap();
    a.push_frame(slice(&fr[0]), 40, None).unwrap();
    a.push_frame(slice(&fr[1]), 40, None).unwrap();
    assert!(a.push_frame(slice(&fr[2]), 40, None).is_err());
    let mut a = job();
    a.begin(W, H, RGBA8, Some(3)).unwrap();
    a.push_frame(slice(&fr[0]), 40, None).unwrap();
    assert!(a.finish(None).is_err());
    let mut a = job();
    a.begin(W, H, RGBA8, None).unwrap();
    assert!(a.begin(W, H, RGBA8, None).is_err());
    let mut a = job();
    assert!(a.begin(0, H, RGBA8, None).is_err());
    assert!(a.begin(70_000, H, RGBA8, None).is_err());
    // A failed begin left the encoder usable.
    assert!(a.begin(W, H, RGBA8, None).is_ok());
}

#[test]
fn testkit_animation_streaming_output_honesty() {
    let images = |n: usize| -> Vec<zencodec_testkit::TestImage> {
        (0..n)
            .map(|i| zencodec_testkit::TestImage::rgb8_gradient_seeded(W, H, (i * 37 % 256) as u8))
            .collect()
    };
    zencodec_testkit::check_animation_streaming_output_honesty(
        GifEncoderConfig::new().with_shared_palette(false),
        GifDecoderConfig::new(),
        &images(5),
    )
    .unwrap();
    zencodec_testkit::check_animation_streaming_output_honesty(
        GifEncoderConfig::new()
            .with_shared_palette(true)
            .with_max_buffer_frames(3),
        GifDecoderConfig::new(),
        &images(6),
    )
    .unwrap();
    // The default palette buffer (32 frames): enough frames to verify it
    // without a frame count.
    zencodec_testkit::check_animation_streaming_output_honesty(
        GifEncoderConfig::new(),
        GifDecoderConfig::new(),
        &images(34),
    )
    .unwrap();
}
