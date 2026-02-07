//! Streaming GIF encoder.

use crate::{
    types::{FrameInput, Metadata, Repeat, Rgba},
    GifError, Limits, Result, Stats,
};
use super::{EncodeRequest, EncoderConfig};
use super::palette::{compute_frame_diff_pooled, ScratchBuffer};
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
use super::config::default_buffer_frames;
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
use super::palette::compute_remap_rmse;
use enough::Stop;
use std::borrow::Cow;
use whereat::at;

/// Streaming GIF encoder.
///
/// Created via [`EncodeRequest::build()`]. Add frames progressively with [`add_frame()`](Self::add_frame),
/// then call [`finish()`](Self::finish) to get the encoded GIF bytes.
///
/// The encoder handles:
/// - Progressive frame encoding (no need to buffer all frames)
/// - Memory tracking and limits
/// - Cancellation support
/// - Frame differencing for smaller output
/// - Optional shared palette computation
///
/// # Example
///
/// ```no_run
/// use zengif::{EncodeRequest, EncoderConfig, FrameInput, Limits, Rgba};
/// use enough::Unstoppable;
///
/// let config = EncoderConfig::new();
/// let limits = Limits::default();
///
/// let mut encoder = EncodeRequest::new(&config, 100, 100)
///     .limits(&limits)
///     .stop(&Unstoppable)
///     .build()?;
///
/// // Add frames one at a time
/// for i in 0..10 {
///     let pixels = vec![Rgba::rgb((i * 25) as u8, 0, 0); 10000];
///     encoder.add_frame(FrameInput::new(100, 100, 10, pixels))?;
/// }
///
/// let output = encoder.finish()?;
/// # Ok::<(), whereat::At<zengif::GifError>>(())
/// ```
pub struct Encoder<'a> {
    /// Underlying gif encoder writing to internal buffer.
    /// Created lazily when shared_palette is true.
    encoder: Option<gif::Encoder<Vec<u8>>>,

    /// Internal buffer for GIF output.
    /// The gif::Encoder writes to this, or we hold it until encoder is created.
    buffer: Vec<u8>,

    /// Canvas width.
    width: u16,

    /// Canvas height.
    height: u16,

    /// Whether the encoder was created with a non-empty global color table.
    /// When true, frames with the same palette can use `palette: None`.
    has_global_palette: bool,

    /// Configuration (borrowed from request).
    config: &'a EncoderConfig,

    /// Previous frame for transparency optimization.
    previous_frame: Option<Vec<Rgba>>,

    /// Frame index.
    frame_index: usize,

    /// Limits configuration (borrowed from request).
    limits: &'a Limits,

    /// Stats tracker.
    stats: Stats,

    /// Cancellation checker (borrowed from request).
    stop: &'a dyn Stop,

    /// Whether the repeat extension has been written.
    repeat_written: bool,

    /// Buffered frames for shared palette mode.
    /// Frames are buffered until limits are reached, then palette is built.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    buffered_frames: Vec<FrameInput>,

    /// Current buffered memory in bytes.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    buffered_bytes: usize,

    /// Shared palette (computed once buffer limits are reached).
    /// Once set, all subsequent frames use this palette.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    computed_palette: Option<Vec<u8>>,

    /// Quantizer instance for shared palette mode.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    quantizer: Box<dyn crate::quantize::QuantizerTrait>,

    /// Reusable scratch buffer to avoid per-frame allocations.
    scratch: ScratchBuffer,
}

impl<'a> Encoder<'a> {
    /// Create encoder from request (internal constructor).
    pub(crate) fn from_request(req: EncodeRequest<'a>) -> Result<Self> {
        // Check cancellation
        req.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Validate dimensions
        req.limits.check_dimensions(req.width, req.height)?;

        let stats = Stats::new();

        // Determine if we should defer encoder creation.
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        let defer_encoder = req.config.shared_palette && req.config.global_palette.is_none();
        #[cfg(not(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        )))]
        let defer_encoder = false;

        let (encoder, buffer, has_global_palette) = if defer_encoder {
            // Defer encoder creation until palette is computed
            (None, Vec::new(), false)
        } else {
            // Create encoder immediately
            let global_pal_bytes = req
                .config
                .global_palette
                .as_ref()
                .map(|p| p.iter().flat_map(|c| [c.r, c.g, c.b]).collect::<Vec<u8>>())
                .unwrap_or_default();

            let has_global = !global_pal_bytes.is_empty();

            let mut enc = gif::Encoder::new(Vec::new(), req.width, req.height, &global_pal_bytes)
                .map_err(|e| at!(GifError::from(e)))?;

            enc.set_repeat(match req.config.repeat { Repeat::Once => gif::Repeat::Finite(0), Repeat::Infinite => gif::Repeat::Infinite, Repeat::Count(n) => gif::Repeat::Finite(n) })
                .map_err(|e| at!(GifError::from(e)))?;

            (Some(enc), Vec::new(), has_global)
        };

        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        #[allow(deprecated)] // quantizer_backend fallback for backward compat
        let quantizer = req.config.quantizer.as_ref().map(|q| q.create_backend()).unwrap_or_else(|| req.config.quantizer_backend.create_quantizer().expect("quantizer feature should be enabled"));

        Ok(Self {
            encoder,
            buffer,
            width: req.width,
            height: req.height,
            has_global_palette,
            config: req.config,
            previous_frame: None,
            frame_index: 0,
            limits: req.limits,
            stats,
            stop: req.stop,
            repeat_written: false,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            buffered_frames: Vec::new(),
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            buffered_bytes: 0,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            computed_palette: None,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quantizer,
            scratch: ScratchBuffer::default(),
        })
    }

        // Check cancellation

    /// Create an encoder from metadata.
    ///
    /// This preserves the original global palette if available, and uses
    /// round-trip optimized settings (zero dithering) to minimize bloat.
    #[allow(deprecated)] // quantizer_backend is deprecated
    pub fn from_metadata(metadata: &Metadata, limits: &'a Limits, stop: &'a dyn Stop) -> Result<Self> {
        let config = EncoderConfig {
            repeat: metadata.repeat,
            global_palette: metadata
                .global_palette
                .as_ref()
                .map(|p| p.colors().to_vec()),
            use_transparency: true,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quality: 100, // Max quality for round-trip
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            dithering: 0.0, // No dithering for round-trip (already dithered)
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            shared_palette: false, // Will use global if available
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_frames: default_buffer_frames(metadata.width, metadata.height),
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_bytes: 64 * 1024 * 1024,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quantizer_backend: crate::quantize::QuantizerBackend::default(),
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quantizer: None,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            palette_error_threshold: None, // Round-trip: always use global palette

            lossy_tolerance: 0, // Lossless for round-trip

        };

        // Box and leak the config to satisfy the 'a lifetime requirement.
        // This is acceptable for from_metadata as it's used for round-tripping,
        // which is typically done once per GIF, not in a loop.
        let config: &'a EncoderConfig = Box::leak(Box::new(config));

        let req = EncodeRequest::new(config, metadata.width, metadata.height)
            .limits(limits)
            .stop(stop);
        Self::from_request(req)
    }






    /// Get the encoder configuration.
    pub fn config(&self) -> &EncoderConfig {
        self.config
    }

    /// Get the stats.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Get the current frame index.
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// Ensure the gif encoder is created, using the given palette as global color table.
    /// If the encoder already exists, this is a no-op.
    fn ensure_encoder_created(&mut self, global_palette: &[u8]) -> Result<()> {
        if self.encoder.is_some() {
            return Ok(());
        }
        let buffer = core::mem::take(&mut self.buffer);

        self.has_global_palette = !global_palette.is_empty();
        let enc = gif::Encoder::new(buffer, self.width, self.height, global_palette)
            .map_err(|e| at!(GifError::from(e)))?;
        self.encoder = Some(enc);
        Ok(())
    }

    /// Get a mutable reference to the gif encoder, creating it if needed.
    fn encoder_mut(&mut self) -> Result<&mut gif::Encoder<Vec<u8>>> {
        if self.encoder.is_none() {
            // Non-deferred path: create with config's global palette or empty
            let global_palette_bytes: Vec<u8> = self
                .config
                .global_palette
                .as_ref()
                .map(|p| p.iter().flat_map(|c| [c.r, c.g, c.b]).collect::<Vec<u8>>())
                .unwrap_or_default();
            self.ensure_encoder_created(&global_palette_bytes)?;
        }
        Ok(self.encoder.as_mut().unwrap())
    }

    /// Write the repeat extension if needed.
    fn ensure_repeat_written(&mut self) -> Result<()> {
        if self.repeat_written {
            return Ok(());
        }

        let repeat = match self.config.repeat {
            Repeat::Once => return Ok(()), // No extension needed
            Repeat::Infinite => gif::Repeat::Infinite,
            Repeat::Count(n) => gif::Repeat::Finite(n),
        };

        self.encoder_mut()?
            .write_extension(gif::ExtensionData::Repetitions(repeat))
            .map_err(|e| at!(GifError::from(e)))?;

        self.repeat_written = true;
        Ok(())
    }

    /// Add a frame to the animation.
    ///
    /// The frame pixels must match the encoder dimensions.
    ///
    /// When `shared_palette` is enabled, frames are buffered until buffer
    /// limits are reached, then the palette is computed and all frames are
    /// encoded. Subsequent frames are encoded immediately with the shared palette.
    pub fn add_frame(&mut self, input: FrameInput) -> Result<()> {
        // Check cancellation
        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Validate dimensions
        if input.width != self.width || input.height != self.height {
            return Err(at!(GifError::FrameDimensionMismatch {
                expected_width: self.width,
                expected_height: self.height,
                actual_width: input.width,
                actual_height: input.height,
            }));
        }

        // Check frame count (including buffered frames)
        // Count total frames including buffered ones
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        let total_frames = self.frame_index + self.buffered_frames.len();
        #[cfg(not(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        )))]
        let total_frames = self.frame_index;
        self.limits.check_frame_count(total_frames as u64)?;

        // Handle shared palette buffering mode
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        if self.config.shared_palette && self.computed_palette.is_none() {
            return self.buffer_frame(input);
        }

        // Direct encode mode
        self.encode_frame_direct(input)
    }

    /// Buffer a frame for later encoding with shared palette.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn buffer_frame(&mut self, input: FrameInput) -> Result<()> {
        let frame_bytes = input.pixels.len() * 4; // RGBA = 4 bytes per pixel
        self.buffered_frames.push(input);
        self.buffered_bytes += frame_bytes;

        // Check if buffer limits reached
        let should_flush = self.buffered_frames.len() >= self.config.max_buffer_frames
            || self.buffered_bytes >= self.config.max_buffer_bytes;

        if should_flush {
            self.flush_buffer()?;
        }

        Ok(())
    }

    /// Build shared palette from buffered frames and encode them all.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn flush_buffer(&mut self) -> Result<()> {
        use crate::quantize::QuantizeConfig;

        if self.buffered_frames.is_empty() {
            return Ok(());
        }

        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Build quantize config
        let quant_config = QuantizeConfig {
            quality: self.config.quality,
            dithering: self.config.dithering,
            use_background: self.config.use_transparency,
            max_palette_frames: None, // Use all buffered frames for palette
        };

        // Collect frame pixel references
        let frame_refs: Vec<&[Rgba]> = self
            .buffered_frames
            .iter()
            .map(|f| f.pixels.as_slice())
            .collect();

        // Build shared palette
        let palette_bytes = self.quantizer.build_shared_palette(
            &frame_refs,
            self.width,
            self.height,
            &quant_config,
            &self.stop,
        )?;

        // Create the gif encoder with the shared palette as global color table.
        // This avoids writing redundant local color tables on every frame.
        self.ensure_encoder_created(&palette_bytes)?;

        self.computed_palette = Some(palette_bytes);

        // Take ownership of buffered frames
        let frames = core::mem::take(&mut self.buffered_frames);
        self.buffered_bytes = 0;

        // Encode all buffered frames with the shared palette
        for frame_input in frames {
            self.encode_frame_direct(frame_input)?;
        }

        Ok(())
    }

    /// Encode a frame directly (not buffered).
    fn encode_frame_direct(&mut self, input: FrameInput) -> Result<()> {
        // Ensure repeat is written before first frame
        self.ensure_repeat_written()?;

        // Quantize and encode the frame
        let frame = self.prepare_frame(&input)?;

        self.encoder_mut()?
            .write_frame(&frame)
            .map_err(|e| at!(GifError::from(e)))?;

        // Save for next frame's transparency optimization
        if self.config.use_transparency {
            self.previous_frame = Some(input.pixels);
        }

        self.frame_index += 1;
        Ok(())
    }

    /// Prepare a frame for encoding.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn prepare_frame(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        self.prepare_frame_quantized(input)
    }

    /// Prepare a frame for encoding (no quantizer available).
    ///
    /// This path requires frames to have pre-computed palettes.
    #[cfg(not(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    )))]
    fn prepare_frame(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        self.prepare_frame_passthrough(input)
    }

    /// Passthrough frame preparation - requires frames to have palettes already.
    ///
    /// Without a quantizer feature enabled, frames must have pre-computed palettes.
    /// This is typically used for round-trip encoding where the palette is preserved.
    #[cfg(not(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    )))]
    fn prepare_frame_passthrough(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        // Without a quantizer, frames MUST have a palette
        let palette = input.palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no quantizer feature enabled and frame has no palette"
            })
        })?;

        // Check if we can optimize using frame differencing
        let (frame_pixels, frame_left, frame_top, frame_width, frame_height) =
            if self.config.use_transparency {
                if let Some(ref prev) = self.previous_frame {
                    if let Some(diff) = compute_frame_diff_pooled(
                        &input.pixels,
                        prev,
                        input.width,
                        input.height,
                        self.config.lossy_tolerance,
                        &mut self.scratch,
                    ) {
                        (diff.pixels, diff.left, diff.top, diff.width, diff.height)
                    } else {
                        // No optimization - reuse frame_pixels buffer
                        self.scratch.frame_pixels.clear();
                        self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                        (
                            core::mem::take(&mut self.scratch.frame_pixels),
                            0,
                            0,
                            input.width,
                            input.height,
                        )
                    }
                } else {
                    // First frame - no diff possible
                    self.scratch.frame_pixels.clear();
                    self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                    (
                        core::mem::take(&mut self.scratch.frame_pixels),
                        0,
                        0,
                        input.width,
                        input.height,
                    )
                }
            } else {
                // Transparency disabled
                self.scratch.frame_pixels.clear();
                self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                (
                    core::mem::take(&mut self.scratch.frame_pixels),
                    0,
                    0,
                    input.width,
                    input.height,
                )
            };

        let (pixels, transparent_index) = palette.map_pixels(&frame_pixels);

        // Return the frame_pixels buffer to scratch for reuse
        self.scratch.frame_pixels = frame_pixels;

        let palette_bytes = palette.to_rgb_bytes();

        let frame = gif::Frame {
            left: frame_left,
            top: frame_top,
            width: frame_width,
            height: frame_height,
            delay: input.delay,
            dispose: gif::DisposalMethod::Keep,
            transparent: transparent_index,
            palette: Some(palette_bytes),
            buffer: Cow::Owned(pixels),
            ..Default::default()
        };

        Ok(frame)
    }

    /// Frame preparation with imagequant quantization.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn prepare_frame_quantized(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        use crate::quantize::QuantizeConfig;

        // Check if we can optimize using frame differencing
        let (frame_pixels, frame_left, frame_top, frame_width, frame_height) =
            if self.config.use_transparency {
                if let Some(ref prev) = self.previous_frame {
                    if let Some(diff) = compute_frame_diff_pooled(
                        &input.pixels,
                        prev,
                        input.width,
                        input.height,
                        self.config.lossy_tolerance,
                        &mut self.scratch,
                    ) {
                        // Use the optimized diff region
                        (diff.pixels, diff.left, diff.top, diff.width, diff.height)
                    } else {
                        // No optimization possible, use full frame with pooled buffer
                        self.scratch.frame_pixels.clear();
                        self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                        (
                            core::mem::take(&mut self.scratch.frame_pixels),
                            0,
                            0,
                            input.width,
                            input.height,
                        )
                    }
                } else {
                    // First frame, no diff possible - use pooled buffer
                    self.scratch.frame_pixels.clear();
                    self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                    (
                        core::mem::take(&mut self.scratch.frame_pixels),
                        0,
                        0,
                        input.width,
                        input.height,
                    )
                }
            } else {
                // Transparency optimization disabled - use pooled buffer
                self.scratch.frame_pixels.clear();
                self.scratch.frame_pixels.extend_from_slice(&input.pixels);
                (
                    core::mem::take(&mut self.scratch.frame_pixels),
                    0,
                    0,
                    input.width,
                    input.height,
                )
            };

        // If frame has a palette, use it directly (pass-through mode)
        if let Some(ref palette) = input.palette {
            let (pixels, transparent_index) = palette.map_pixels(&frame_pixels);

            // Return buffer to scratch for reuse
            self.scratch.frame_pixels = frame_pixels;

            let palette_bytes = palette.to_rgb_bytes();

            let frame = gif::Frame {
                left: frame_left,
                top: frame_top,
                width: frame_width,
                height: frame_height,
                delay: input.delay,
                dispose: gif::DisposalMethod::Keep,
                transparent: transparent_index,
                palette: Some(palette_bytes),
                buffer: Cow::Owned(pixels),
                ..Default::default()
            };

            return Ok(frame);
        }

        let quant_config = QuantizeConfig {
            quality: self.config.quality,
            dithering: self.config.dithering,
            use_background: self.config.use_transparency,
            max_palette_frames: None,
        };

        // Use shared palette if available, otherwise per-frame quantization.
        // With hybrid mode (palette_error_threshold is Some), frames that
        // don't fit the shared palette well get their own local color table.
        let (palette_bytes, pixels, transparent_index, use_local_palette) =
            if self.computed_palette.is_some() {
                // Shared palette mode: remap with pre-computed palette
                let background = self.previous_frame.as_deref();
                let quantized = self.quantizer.quantize_frame_with_palette(
                    &frame_pixels,
                    frame_width,
                    frame_height,
                    background,
                    &quant_config,
                )?;

                // Hybrid check: if RMSE exceeds threshold, fall back to per-frame palette
                if let Some(threshold) = self.config.palette_error_threshold {
                    let rmse =
                        compute_remap_rmse(&frame_pixels, &quantized.pixels, &quantized.palette);
                    if rmse > threshold {
                        // Shared palette too inaccurate — quantize this frame independently
                        let background = self.previous_frame.as_deref();
                        let per_frame = self.quantizer.quantize_frame(
                            &frame_pixels,
                            frame_width,
                            frame_height,
                            background,
                            &quant_config,
                        )?;
                        (
                            per_frame.palette,
                            per_frame.pixels,
                            per_frame.transparent_index,
                            true,
                        )
                    } else {
                        // Shared palette is good enough
                        (
                            quantized.palette,
                            quantized.pixels,
                            quantized.transparent_index,
                            false,
                        )
                    }
                } else {
                    // No threshold — always use shared palette
                    (
                        quantized.palette,
                        quantized.pixels,
                        quantized.transparent_index,
                        false,
                    )
                }
            } else {
                // Per-frame quantization (no shared palette)
                let background = self.previous_frame.as_deref();
                let quantized = self.quantizer.quantize_frame(
                    &frame_pixels,
                    frame_width,
                    frame_height,
                    background,
                    &quant_config,
                )?;
                (
                    quantized.palette,
                    quantized.pixels,
                    quantized.transparent_index,
                    true,
                )
            };

        // Return buffer to scratch for reuse
        self.scratch.frame_pixels = frame_pixels;

        // If we're using the global color table and the shared palette was
        // accurate enough, omit the local color table to save ~768 bytes.
        let frame_palette = if self.has_global_palette && !use_local_palette {
            None
        } else {
            Some(palette_bytes)
        };

        let frame = gif::Frame {
            left: frame_left,
            top: frame_top,
            width: frame_width,
            height: frame_height,
            delay: input.delay,
            dispose: gif::DisposalMethod::Keep,
            transparent: transparent_index,
            palette: frame_palette,
            buffer: Cow::Owned(pixels),
            ..Default::default()
        };

        Ok(frame)
    }

    /// Finish encoding and return the writer.
    ///
    /// If there are buffered frames (from shared palette mode), they are
    /// encoded before finishing.
    #[allow(unused_mut)]
    pub fn finish(mut self) -> Result<Vec<u8>> {
        // Flush any remaining buffered frames
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        self.flush_buffer()?;

        // If encoder was never created (0 frames with deferred creation),
        // return the pending writer directly.
        if !self.buffer.is_empty() {
            return Ok(self.buffer);
        }

        let writer = self
            .encoder
            .expect("encoder should exist after flush")
            .into_inner()
            .map_err(|e| at!(GifError::from(e)))?;
        Ok(writer)
    }
}

// OLD impl block removed - now part of main Encoder<'a> impl
