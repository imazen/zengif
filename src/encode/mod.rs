//! GIF streaming encoder.
//!
//! Provides a streaming encoder that accepts RGBA frames and produces
//! optimized GIF output with proper transparency handling.
//!
//! # Palette Strategies
//!
//! GIF encoding requires quantizing RGBA colors to a 256-color palette.
//! The choice of strategy affects quality, file size, and flickering:
//!
//! - [`PaletteStrategy::PerFrame`]: Each frame gets its own optimal palette.
//!   Best color accuracy per frame, but can cause flickering and larger files.
//!
//! - [`PaletteStrategy::Shared`]: A single palette computed from all frames.
//!   Eliminates flickering, better compression, slight color quality loss.
//!   Requires pre-collecting all frames (use `encode_gif_shared_palette`).
//!
//! - [`PaletteStrategy::Global`]: Use the provided global palette (e.g. from
//!   a decoded GIF). Best for round-tripping when the original palette should
//!   be preserved.
//!
//! # Dithering Options
//!
//! Dithering adds noise to simulate colors not in the palette:
//!
//! - `dithering: 0.0` - No dithering. Best compression, may show banding.
//! - `dithering: 0.5` - Moderate dithering (default). Good balance.
//! - `dithering: 1.0` - Full dithering. Best appearance, worst compression.
//!
//! For round-trip encoding (decode -> encode), use `dithering: 0.0` since
//! the content is already dithered.
//!
//! **Note**: Temporal dithering (spreading error across frames) is not yet
//! implemented. This is an advanced feature that would require explicit opt-in.

#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
use std::borrow::Cow;

use enough::Stop;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::types::FrameInput;
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
use crate::types::{Repeat, Rgba};

// Module declarations
mod config;
mod encoder;
/// Cross-codec uniformity bundle (`__expert`-gated). Mirrors `zenjpeg`'s
/// `InternalParams` so external pipelines (calibration sweeps, picker
/// training) can drive every codec the same way. See
/// [`internal_params::InternalParams`] and
/// [`EncoderConfig::with_internal_params`].
#[cfg(feature = "__expert")]
pub mod internal_params;
mod palette;
mod request;

// Re-exports
pub use config::EncoderConfig;
pub use encoder::Encoder;
#[cfg(feature = "__expert")]
pub use internal_params::InternalParams;
pub use palette::PaletteStrategy;
pub use request::EncodeRequest;

/// Convenience function to encode frames to a GIF byte vector.
///
/// For more control over encoding options, use [`EncodeRequest`] and [`Encoder`].
///
/// # Example
///
/// ```no_run
/// use zengif::{encode_gif, EncoderConfig, FrameInput, Limits, Repeat, Rgba};
/// use enough::Unstoppable;
///
/// let frames = vec![
///     FrameInput::new(100, 100, 50, vec![Rgba::rgb(255, 0, 0); 10000]),
/// ];
/// let output = encode_gif(frames, 100, 100, EncoderConfig::new(), Limits::default(), &Unstoppable)?;
/// # Ok::<(), whereat::At<zengif::GifError>>(())
/// ```
pub fn encode_gif(
    frames: Vec<FrameInput>,
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: &dyn Stop,
) -> Result<Vec<u8>> {
    // Estimate initial output size (header + per-frame overhead)
    // GIF header ~13 bytes, each frame has overhead of ~100-500 bytes + compressed data
    // This is a conservative estimate to reduce reallocations
    let estimated_size = 1024 + frames.len() * 512;

    let mut output: Vec<u8> = Vec::new();
    output.try_reserve(estimated_size).map_err(|_| {
        at!(GifError::AllocationFailed {
            requested: estimated_size as u64
        })
    })?;
    let req = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(stop);
    let mut encoder = Encoder::from_request(req)?;

    for frame in frames {
        encoder.add_frame(frame)?;
    }

    encoder.finish()
}

/// Encode frames using a shared palette computed from all frames.
///
/// This produces better compression and eliminates palette flicker in animations
/// by using a single global palette derived from all frames' colors.
///
/// Uses imagequant's `set_background()` for frame-aware transparency optimization:
/// pixels that match the previous frame after quantization are made transparent.
///
/// For round-trip encoding (decode -> encode), this combined with zero dithering
/// significantly reduces output bloat.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
pub fn encode_gif_shared_palette(
    frames: Vec<FrameInput>,
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: &dyn Stop,
) -> Result<Vec<u8>> {
    // Select quantizer based on available features (priority: zenquant > quantette > imagequant > quantizr > color_quant)
    #[cfg(feature = "zenquant")]
    let quantizer = crate::quantize::ZenquantQuantizer::new();

    #[cfg(all(feature = "quantette", not(feature = "zenquant")))]
    let quantizer = crate::quantize::QuantetteQuantizer::new();

    #[cfg(all(
        feature = "imagequant",
        not(feature = "zenquant"),
        not(feature = "quantette")
    ))]
    let quantizer = crate::quantize::ImagequantQuantizer::new();

    #[cfg(all(
        feature = "quantizr",
        not(feature = "zenquant"),
        not(feature = "quantette"),
        not(feature = "imagequant")
    ))]
    let quantizer = crate::quantize::QuantizrQuantizer::new();

    #[cfg(all(
        feature = "color_quant",
        not(feature = "zenquant"),
        not(feature = "quantette"),
        not(feature = "imagequant"),
        not(feature = "quantizr")
    ))]
    let quantizer = crate::quantize::ColorQuantQuantizer::new();

    encode_gif_with_quantizer(frames, width, height, config, limits, stop, quantizer)
}

/// Encode frames using a custom quantizer.
///
/// This is the generic version that accepts any [`Quantizer`](crate::Quantizer)
/// implementation, allowing for custom quantization algorithms.
///
/// See [`encode_gif_shared_palette`] for the default imagequant-based version.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
pub fn encode_gif_with_quantizer<Q: crate::quantize::QuantizerTrait>(
    frames: Vec<FrameInput>,
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: &dyn Stop,
    mut quantizer: Q,
) -> Result<Vec<u8>> {
    use crate::quantize::QuantizeConfig;

    if frames.is_empty() {
        return encode_gif(frames, width, height, config, limits, stop);
    }

    stop.check().map_err(|_| at!(GifError::Cancelled))?;

    // Build quantize config from encoder config
    let quant_config = QuantizeConfig {
        quality: config.quality,
        dithering: config.dithering,
        use_background: config.use_transparency,
        max_palette_frames: None, // Sample all frames for shared palette
    };

    // Collect frame references for shared palette building
    let frame_refs: Vec<&[Rgba]> = frames.iter().map(|f| f.pixels.as_slice()).collect();

    // Build shared palette from all frames (with cancellation support)
    let palette_bytes =
        quantizer.build_shared_palette(&frame_refs, width, height, &quant_config, stop)?;

    // Estimate output size
    let estimated_size = 1024 + frames.len() * 512;
    let mut output = Vec::new();
    output.try_reserve(estimated_size).map_err(|_| {
        at!(GifError::AllocationFailed {
            requested: estimated_size as u64
        })
    })?;

    // Create encoder with global palette
    let mut gif_encoder = gif::Encoder::new(output, width, height, &palette_bytes)
        .map_err(|e| at!(GifError::from(e)))?;

    // Write repeat extension
    let repeat = match config.repeat {
        Repeat::Once => None,
        Repeat::Infinite => Some(gif::Repeat::Infinite),
        Repeat::Count(n) => Some(gif::Repeat::Finite(n)),
    };
    if let Some(r) = repeat {
        gif_encoder
            .write_extension(gif::ExtensionData::Repetitions(r))
            .map_err(|e| at!(GifError::from(e)))?;
    }

    // Encode each frame using the shared palette with set_background()
    let mut previous_frame: Option<Vec<Rgba>> = None;

    for (frame_index, frame) in frames.into_iter().enumerate() {
        stop.check().map_err(|_| at!(GifError::Cancelled))?;
        limits.check_frame_count(frame_index as u64)?;

        // Quantize frame with previous frame as background
        // imagequant's set_background() will make matching pixels transparent
        let quantized = quantizer.quantize_frame_with_palette(
            &frame.pixels,
            frame.width,
            frame.height,
            previous_frame.as_deref(),
            &quant_config,
        )?;

        // Build gif frame (no local palette - uses global)
        let gif_frame = gif::Frame {
            left: 0,
            top: 0,
            width: frame.width,
            height: frame.height,
            delay: frame.delay,
            dispose: gif::DisposalMethod::Keep,
            transparent: quantized.transparent_index,
            palette: None, // Use global palette
            buffer: Cow::Owned(quantized.pixels),
            ..Default::default()
        };

        gif_encoder
            .write_frame(&gif_frame)
            .map_err(|e| at!(GifError::from(e)))?;

        // Save for next frame's background
        if config.use_transparency {
            previous_frame = Some(frame.pixels);
        }
    }

    gif_encoder.into_inner().map_err(|e| at!(GifError::from(e)))
}

#[cfg(test)]
mod tests {
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    use super::config::default_buffer_frames;
    use super::palette::compute_frame_diff;
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    use super::palette::compute_remap_rmse;
    use super::*;
    use crate::types::{Repeat, Rgba};
    use enough::Unstoppable;

    fn make_red_frame(width: u16, height: u16, delay: u16) -> FrameInput {
        let pixels = vec![Rgba::rgb(255, 0, 0); width as usize * height as usize];
        FrameInput::new(width, height, delay, pixels)
    }

    #[test]
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    fn encode_single_frame() {
        let config = EncoderConfig::new().repeat(Repeat::Once);
        let limits = Limits::default();

        let frame = make_red_frame(2, 2, 10);

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        encoder.add_frame(frame).unwrap();
        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    fn encode_multiple_frames() {
        let config = EncoderConfig::new().repeat(Repeat::Infinite);
        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        for _ in 0..3 {
            let frame = make_red_frame(2, 2, 10);
            encoder.add_frame(frame).unwrap();
        }

        let output = encoder.finish().unwrap();

        assert!(output.len() > 50);
    }

    #[test]
    fn encode_dimension_mismatch() {
        let config = EncoderConfig::new();
        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // Wrong dimensions
        let frame = make_red_frame(2, 2, 10);
        let result = encoder.add_frame(frame);

        assert!(result.is_err());
    }

    #[test]
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    fn encode_convenience_function() {
        let config = EncoderConfig::new();
        let limits = Limits::default();

        let frames = vec![make_red_frame(2, 2, 10), make_red_frame(2, 2, 10)];

        let output = encode_gif(frames, 2, 2, config, limits, &Unstoppable).unwrap();

        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    fn encode_with_limits() {
        let config = EncoderConfig::new();
        let limits = Limits::default().max_frame_count(1);

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // First frame OK
        encoder.add_frame(make_red_frame(2, 2, 10)).unwrap();

        // Second frame should fail
        let result = encoder.add_frame(make_red_frame(2, 2, 10));
        assert!(result.is_err());
    }

    #[test]
    fn frame_diff_finds_changed_region() {
        let width = 10u16;
        let height = 10u16;

        // Create two frames with only a small region changed
        let prev = vec![Rgba::rgb(0, 0, 0); 100];
        let mut curr = prev.clone();

        // Change only a 2x2 region at position (3, 4)
        curr[4 * 10 + 3] = Rgba::rgb(255, 0, 0);
        curr[4 * 10 + 4] = Rgba::rgb(255, 0, 0);
        curr[5 * 10 + 3] = Rgba::rgb(255, 0, 0);
        curr[5 * 10 + 4] = Rgba::rgb(255, 0, 0);

        let diff = compute_frame_diff(&curr, &prev, width, height).unwrap();

        // Should find a 2x2 region at (3, 4)
        assert_eq!(diff.left, 3);
        assert_eq!(diff.top, 4);
        assert_eq!(diff.width, 2);
        assert_eq!(diff.height, 2);
        assert_eq!(diff.pixels.len(), 4);

        // All pixels in the diff region should be the changed color
        for pixel in &diff.pixels {
            assert_eq!(*pixel, Rgba::rgb(255, 0, 0));
        }
    }

    #[test]
    fn frame_diff_marks_unchanged_as_transparent() {
        let width = 10u16;
        let height = 10u16;

        // Create frames where only some pixels in the changed region differ
        let prev = vec![Rgba::rgb(0, 0, 0); 100];
        let mut curr = prev.clone();

        // Change a 3x3 region but only corners actually differ
        // This creates a region where interior pixels should be marked transparent
        curr[0] = Rgba::rgb(255, 0, 0); // (0,0) top-left
        curr[2] = Rgba::rgb(255, 0, 0); // (2,0) top-right
        curr[20] = Rgba::rgb(255, 0, 0); // (0,2) bottom-left
        curr[22] = Rgba::rgb(255, 0, 0); // (2,2) bottom-right

        let diff = compute_frame_diff(&curr, &prev, width, height).unwrap();

        // Should find a 3x3 region at (0, 0)
        assert_eq!(diff.left, 0);
        assert_eq!(diff.top, 0);
        assert_eq!(diff.width, 3);
        assert_eq!(diff.height, 3);
        assert_eq!(diff.pixels.len(), 9);

        // Check that unchanged pixels are transparent
        // Row 0: changed, unchanged, changed
        assert_eq!(diff.pixels[0], Rgba::rgb(255, 0, 0));
        assert_eq!(diff.pixels[1], Rgba::TRANSPARENT);
        assert_eq!(diff.pixels[2], Rgba::rgb(255, 0, 0));
        // Row 1: unchanged, unchanged, unchanged
        assert_eq!(diff.pixels[3], Rgba::TRANSPARENT);
        assert_eq!(diff.pixels[4], Rgba::TRANSPARENT);
        assert_eq!(diff.pixels[5], Rgba::TRANSPARENT);
        // Row 2: changed, unchanged, changed
        assert_eq!(diff.pixels[6], Rgba::rgb(255, 0, 0));
        assert_eq!(diff.pixels[7], Rgba::TRANSPARENT);
        assert_eq!(diff.pixels[8], Rgba::rgb(255, 0, 0));
    }

    #[test]
    fn frame_diff_no_changes() {
        let width = 10u16;
        let height = 10u16;
        let frame = vec![Rgba::rgb(128, 128, 128); 100];

        // Identical frames should produce a minimal 1x1 transparent diff
        let diff = compute_frame_diff(&frame, &frame, width, height).unwrap();

        assert_eq!(diff.width, 1);
        assert_eq!(diff.height, 1);
        assert_eq!(diff.pixels[0], Rgba::TRANSPARENT);
    }

    #[test]
    fn frame_diff_full_change() {
        let width = 10u16;
        let height = 10u16;
        let prev = vec![Rgba::rgb(0, 0, 0); 100];
        let curr = vec![Rgba::rgb(255, 255, 255); 100];

        // Completely different frames should return None (no optimization)
        let diff = compute_frame_diff(&curr, &prev, width, height);

        assert!(diff.is_none());
    }

    #[test]
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    fn frame_diff_produces_smaller_output() {
        // Encode two identical red frames - second should be tiny due to diff
        let config = EncoderConfig::new()
            .repeat(Repeat::Once)
            .use_transparency(true);
        let limits = Limits::default();

        // Create two identical frames
        let frame1 = make_red_frame(100, 100, 10);
        let frame2 = make_red_frame(100, 100, 10);

        let output_with_diff = {
            // output will be returned from encoder.finish()
            let mut encoder = EncodeRequest::new(&config, 100, 100)
                .limits(&limits)
                .stop(&Unstoppable)
                .build()
                .unwrap();
            encoder.add_frame(frame1.clone()).unwrap();
            encoder.add_frame(frame2.clone()).unwrap();
            encoder.finish().unwrap()
        };

        // Encode without transparency optimization
        let config_no_opt = config.use_transparency(false);
        let output_without_diff = {
            // output will be returned from encoder.finish()
            let mut encoder = EncodeRequest::new(&config_no_opt, 100, 100)
                .limits(&limits)
                .stop(&Unstoppable)
                .build()
                .unwrap();
            encoder.add_frame(frame1).unwrap();
            encoder.add_frame(frame2).unwrap();
            encoder.finish().unwrap()
        };

        // With diff optimization, output should be smaller
        // (identical second frame becomes tiny 1x1 transparent)
        assert!(
            output_with_diff.len() < output_without_diff.len(),
            "Output with diff ({} bytes) should be smaller than without ({} bytes)",
            output_with_diff.len(),
            output_without_diff.len()
        );
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn shared_palette_encodes_animation() {
        // Create frames with different but similar colors
        let width = 32u16;
        let height = 32u16;
        let size = width as usize * height as usize;

        let frame1 = FrameInput::new(
            width,
            height,
            10,
            vec![Rgba::rgb(255, 0, 0); size], // Red
        );
        let frame2 = FrameInput::new(
            width,
            height,
            10,
            vec![Rgba::rgb(0, 255, 0); size], // Green
        );
        let frame3 = FrameInput::new(
            width,
            height,
            10,
            vec![Rgba::rgb(0, 0, 255); size], // Blue
        );

        let config = EncoderConfig::new().repeat(Repeat::Infinite).dithering(0.0); // No dithering for deterministic test
        let limits = Limits::default();

        let output = encode_gif_shared_palette(
            vec![frame1, frame2, frame3],
            4,
            4,
            config,
            limits,
            &Unstoppable,
        )
        .unwrap();

        // Should produce valid GIF
        assert!(output.len() > 100);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn shared_palette_smaller_than_per_frame() {
        // Create an animation with similar colors across frames
        // Shared palette should be more efficient than per-frame palettes
        let width = 64u16;
        let height = 64u16;
        let size = width as usize * height as usize;

        // Create frames with gradual color transitions (similar palettes)
        let frames: Vec<FrameInput> = (0..5)
            .map(|i| {
                let r = (i * 40) as u8;
                FrameInput::new(width, height, 10, vec![Rgba::rgb(r, 100, 100); size])
            })
            .collect();

        let config_shared = EncoderConfig::new().repeat(Repeat::Once).dithering(0.0);
        let config_perframe = EncoderConfig::new()
            .repeat(Repeat::Once)
            .dithering(0.0)
            .shared_palette(false); // Explicitly per-frame

        let limits = Limits::default();

        // Encode with shared palette
        let output_shared = encode_gif_shared_palette(
            frames.clone(),
            64,
            64,
            config_shared,
            limits.clone(),
            &Unstoppable,
        )
        .unwrap();

        // Encode with per-frame palettes (normal encode_gif)
        let output_perframe =
            encode_gif(frames, 64, 64, config_perframe, limits, &Unstoppable).unwrap();

        // Shared palette should produce smaller output due to:
        // 1. No per-frame palette storage (uses global)
        // 2. More consistent indices = better LZW compression
        assert!(
            output_shared.len() <= output_perframe.len(),
            "Shared palette ({} bytes) should be <= per-frame ({} bytes)",
            output_shared.len(),
            output_perframe.len()
        );
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn low_dithering_smaller_than_high_dithering() {
        let width = 64u16;
        let height = 64u16;
        let size = width as usize * height as usize;

        // Create a gradient that will need dithering
        let pixels: Vec<Rgba> = (0..size)
            .map(|i| {
                let x = (i % width as usize) as u8;
                let y = (i / width as usize) as u8;
                Rgba::rgb(x * 4, y * 4, 128)
            })
            .collect();

        let frame = FrameInput::new(width, height, 10, pixels);

        let config_low = EncoderConfig::new().repeat(Repeat::Once).dithering(0.0);
        let config_high = EncoderConfig::new().repeat(Repeat::Once).dithering(1.0);

        let limits = Limits::default();

        let output_low = encode_gif(
            vec![frame.clone()],
            64,
            64,
            config_low,
            limits.clone(),
            &Unstoppable,
        )
        .unwrap();
        let output_high =
            encode_gif(vec![frame], 64, 64, config_high, limits, &Unstoppable).unwrap();

        // Low dithering should produce smaller output (less noise = better LZW)
        assert!(
            output_low.len() < output_high.len(),
            "Low dithering ({} bytes) should be smaller than high dithering ({} bytes)",
            output_low.len(),
            output_high.len()
        );
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn for_round_trip_config() {
        let config = EncoderConfig::new().for_round_trip();

        // Should have zero dithering and shared palette enabled
        assert_eq!(config.dithering, 0.0);
        assert!(config.shared_palette);
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_shared_palette() {
        // Test that streaming encoder buffers frames and builds shared palette
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(true)
            .max_buffer_frames(3); // Buffer up to 3 frames

        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // Add 5 frames - should buffer first 3, then flush and encode
        for i in 0..5 {
            let color = ((i * 50) % 256) as u8;
            let pixels = vec![Rgba::rgb(color, color, color); 16];
            let frame = FrameInput::new(4, 4, 10, pixels);
            encoder.add_frame(frame).unwrap();
        }

        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 50, "Should produce valid GIF output");
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_flushes_on_finish() {
        // Test that finish() flushes remaining buffered frames
        let config = EncoderConfig::new()
            .repeat(Repeat::Once)
            .shared_palette(true)
            .max_buffer_frames(10); // Large buffer - won't hit limit

        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // Add only 2 frames - less than buffer limit
        for _ in 0..2 {
            let frame = make_red_frame(4, 4, 10);
            encoder.add_frame(frame).unwrap();
        }

        // finish() should flush the buffer
        let output = encoder.finish().unwrap();

        // Should have produced valid GIF with content
        assert!(output.len() > 50, "Should produce valid GIF output");
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_memory_limit() {
        // Test that buffer flushes when memory limit is reached
        let config = EncoderConfig::new()
            .repeat(Repeat::Once)
            .shared_palette(true)
            .max_buffer_frames(1000) // High frame limit
            .max_buffer_bytes(100); // Low memory limit (~1 frame = 64 bytes RGBA)

        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // Add 5 frames - should trigger memory limit flush
        for _ in 0..5 {
            let frame = make_red_frame(4, 4, 10);
            encoder.add_frame(frame).unwrap();
        }

        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 50);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    fn palette_passthrough_encoding() {
        use crate::types::Palette;

        // Create a simple 4-color palette
        let palette = Palette::from_rgba(vec![
            Rgba::rgb(255, 0, 0),  // 0: red
            Rgba::rgb(0, 255, 0),  // 1: green
            Rgba::rgb(0, 0, 255),  // 2: blue
            Rgba::new(0, 0, 0, 0), // 3: transparent
        ]);

        // Create pixels using palette colors
        let pixels = vec![
            Rgba::rgb(255, 0, 0),  // red
            Rgba::rgb(0, 255, 0),  // green
            Rgba::rgb(0, 0, 255),  // blue
            Rgba::new(0, 0, 0, 0), // transparent
        ];

        // Create frame with explicit palette (pass-through mode)
        let frame = FrameInput::with_palette(2, 2, 10, pixels, palette);

        let config = EncoderConfig::new().repeat(Repeat::Once);
        let limits = Limits::default();

        // output created by encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();
        encoder.add_frame(frame).unwrap();
        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn hybrid_palette_outlier_gets_local_table() {
        // Create animation: 3 red frames + 1 blue frame.
        // With hybrid mode, the blue frame should get a local color table
        // because RMSE vs the shared palette (built from mostly red) is high.
        let w = 4u16;
        let h = 4u16;
        let red_pixels: Vec<Rgba> = (0..16)
            .map(|i| {
                // Slight variation so quantizer has something to work with
                Rgba::rgb(200 + (i % 56) as u8, 10, 10)
            })
            .collect();
        let blue_pixels: Vec<Rgba> = (0..16)
            .map(|i| Rgba::rgb(10, 10, 200 + (i % 56) as u8))
            .collect();

        let frames = vec![
            FrameInput::new(w, h, 10, red_pixels.clone()),
            FrameInput::new(w, h, 10, red_pixels.clone()),
            FrameInput::new(w, h, 10, blue_pixels.clone()),
            FrameInput::new(w, h, 10, red_pixels),
        ];

        // Encode with hybrid mode (threshold = 5.0 to force fallback for blue)
        let config = EncoderConfig::new()
            .shared_palette(true)
            .palette_error_threshold(Some(5.0));
        // output will be returned from encoder.finish()
        let limits = crate::limits::Limits::none();
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        for frame in &frames {
            encoder.add_frame(frame.clone()).unwrap();
        }
        let output = encoder.finish().unwrap();

        // Decode and verify all frames came through
        let limits = crate::limits::Limits::none();
        let (meta, decoded_frames, _stats) =
            crate::decode::decode_gif(&output, limits, &Unstoppable).unwrap();
        assert_eq!(meta.frame_count, 4);
        assert_eq!(decoded_frames.len(), 4);

        // Verify the blue frame's pixels are actually blue-ish (not mapped to red)
        let blue_frame = &decoded_frames[2];
        let avg_b: u32 = blue_frame.pixels.iter().map(|p| p.b as u32).sum::<u32>()
            / blue_frame.pixels.len() as u32;
        let avg_r: u32 = blue_frame.pixels.iter().map(|p| p.r as u32).sum::<u32>()
            / blue_frame.pixels.len() as u32;
        assert!(
            avg_b > 150,
            "blue frame should be blue-ish, got avg B={avg_b}"
        );
        assert!(
            avg_r < 80,
            "blue frame should not be red-ish, got avg R={avg_r}"
        );
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn hybrid_palette_none_threshold_always_shared() {
        // With threshold = None, all frames use shared palette (no fallback)
        let w = 4u16;
        let h = 4u16;
        let red_pixels: Vec<Rgba> = vec![Rgba::rgb(255, 0, 0); 16];
        let blue_pixels: Vec<Rgba> = vec![Rgba::rgb(0, 0, 255); 16];

        let frames = vec![
            FrameInput::new(w, h, 10, red_pixels.clone()),
            FrameInput::new(w, h, 10, blue_pixels),
        ];

        // No threshold — always shared, even if inaccurate
        let config = EncoderConfig::new()
            .shared_palette(true)
            .palette_error_threshold(None);
        // output created by encoder.finish()
        let limits = crate::limits::Limits::none();
        let mut encoder = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        for frame in &frames {
            encoder.add_frame(frame.clone()).unwrap();
        }
        let output = encoder.finish().unwrap();

        // Should produce valid GIF (we're just testing it doesn't panic/error)
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn default_buffer_frames_scales_with_dimensions() {
        // Small images get more frames for better palette coverage
        assert_eq!(default_buffer_frames(100, 100), 32); // 10K pixels → max
        assert_eq!(default_buffer_frames(256, 256), 30); // 65K pixels

        // Medium images get moderate buffering
        assert_eq!(default_buffer_frames(512, 512), 7); // 262K pixels

        // Large images get fewer frames for faster palette refresh
        assert_eq!(default_buffer_frames(1920, 1080), 4); // 2M pixels → min
        assert_eq!(default_buffer_frames(3840, 2160), 4); // 8M pixels → min

        // Edge case: zero dimensions
        assert_eq!(default_buffer_frames(0, 100), 32);
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_remap_rmse_perfect_match() {
        let pixels = vec![Rgba::rgb(255, 0, 0), Rgba::rgb(0, 255, 0)];
        let indices = vec![0u8, 1u8];
        let palette = vec![255, 0, 0, 0, 255, 0]; // RGB entries

        let rmse = compute_remap_rmse(&pixels, &indices, &palette);
        assert!(rmse < 0.01, "perfect match should have ~0 RMSE, got {rmse}");
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_remap_rmse_skips_transparent() {
        let pixels = vec![
            Rgba::rgb(255, 0, 0),
            Rgba::new(0, 0, 0, 0), // transparent — should be skipped
        ];
        let indices = vec![0u8, 0u8];
        let palette = vec![255, 0, 0]; // One entry, perfect match for opaque pixel

        let rmse = compute_remap_rmse(&pixels, &indices, &palette);
        assert!(
            rmse < 0.01,
            "transparent pixels should be skipped, got RMSE={rmse}"
        );
    }

    /// Buffered shared-palette frames must be charged against
    /// `Limits::max_memory`. Without the H2 fix the encoder happily holds
    /// gigabytes of RGBA pixels until `max_buffer_bytes` (independent
    /// EncoderConfig field) trips, never honouring per-request memory caps.
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_frames_respect_max_memory_limit() {
        use crate::error::GifError;

        let width = 64u16;
        let height = 64u16;
        let pixels_per_frame = width as usize * height as usize; // 4096
        let bytes_per_frame = pixels_per_frame * core::mem::size_of::<Rgba>(); // 16 KB

        // Allow exactly one buffered frame's worth of memory.
        let limits = Limits::default().max_memory(bytes_per_frame as u64);

        // Force buffering by enabling shared_palette without a global palette,
        // and set buffer triggers high so flush isn't reached.
        let config = EncoderConfig::new()
            .repeat(Repeat::Once)
            .shared_palette(true);

        let mut encoder = EncodeRequest::new(&config, width, height)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();

        // First frame fits exactly within the budget.
        let frame1 = FrameInput::new(
            width,
            height,
            10,
            vec![Rgba::rgb(255, 0, 0); pixels_per_frame],
        );
        encoder.add_frame(frame1).expect("first frame fits");

        // Second frame would push retained memory past max_memory and must
        // be rejected.
        let frame2 = FrameInput::new(
            width,
            height,
            10,
            vec![Rgba::rgb(0, 255, 0); pixels_per_frame],
        );
        let err = encoder
            .add_frame(frame2)
            .expect_err("second frame should exceed max_memory");
        assert!(
            matches!(err.error(), GifError::MemoryLimitExceeded { .. }),
            "expected MemoryLimitExceeded, got {:?}",
            err.error()
        );
    }

    #[test]
    fn palette_nearest_color_mapping() {
        use crate::types::Palette;

        // Create a simple palette
        let palette = Palette::from_rgba(vec![
            Rgba::rgb(255, 0, 0),  // 0: red
            Rgba::rgb(0, 255, 0),  // 1: green
            Rgba::rgb(0, 0, 255),  // 2: blue
            Rgba::new(0, 0, 0, 0), // 3: transparent
        ]);

        // Test exact matches
        assert_eq!(palette.find_nearest(Rgba::rgb(255, 0, 0)), 0);
        assert_eq!(palette.find_nearest(Rgba::rgb(0, 255, 0)), 1);
        assert_eq!(palette.find_nearest(Rgba::rgb(0, 0, 255)), 2);

        // Test near matches (should find nearest)
        assert_eq!(palette.find_nearest(Rgba::rgb(250, 10, 10)), 0); // nearest to red
        assert_eq!(palette.find_nearest(Rgba::rgb(10, 250, 10)), 1); // nearest to green

        // Test transparent pixels
        assert_eq!(palette.find_nearest(Rgba::new(128, 128, 128, 0)), 3); // transparent
    }
}
