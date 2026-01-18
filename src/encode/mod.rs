//! GIF streaming encoder.
//!
//! Provides a streaming encoder that accepts RGBA frames and produces
//! optimized GIF output with proper transparency handling.

use std::io::Write;

use enough::Stop;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::stats::Stats;
use crate::types::{FrameInput, Metadata, Repeat, Rgba};

/// Encoder configuration.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// Canvas width.
    pub width: u16,

    /// Canvas height.
    pub height: u16,

    /// Loop behavior.
    pub repeat: Repeat,

    /// Global palette (if any).
    pub global_palette: Option<Vec<Rgba>>,

    /// Whether to use transparency for unchanged pixels.
    pub use_transparency: bool,

    /// Quality setting for quantization (1-100, higher = better quality).
    #[cfg(feature = "quantize")]
    pub quality: u8,
}

impl EncoderConfig {
    /// Create a new encoder configuration.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            repeat: Repeat::Infinite,
            global_palette: None,
            use_transparency: true,
            #[cfg(feature = "quantize")]
            quality: 80,
        }
    }

    /// Set the loop behavior.
    #[must_use]
    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// Set a global palette.
    #[must_use]
    pub fn global_palette(mut self, palette: Vec<Rgba>) -> Self {
        self.global_palette = Some(palette);
        self
    }

    /// Enable or disable transparency optimization.
    #[must_use]
    pub fn use_transparency(mut self, use_it: bool) -> Self {
        self.use_transparency = use_it;
        self
    }

    /// Set quality for quantization (1-100).
    #[cfg(feature = "quantize")]
    #[must_use]
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = quality.clamp(1, 100);
        self
    }
}

/// Streaming GIF encoder.
///
/// Encodes RGBA frames into a GIF animation with proper
/// transparency and optimization.
pub struct Encoder<W: Write, S: Stop> {
    /// Underlying gif encoder.
    encoder: gif::Encoder<W>,

    /// Configuration.
    config: EncoderConfig,

    /// Previous frame for transparency optimization.
    previous_frame: Option<Vec<Rgba>>,

    /// Frame index.
    frame_index: usize,

    /// Limits configuration.
    limits: Limits,

    /// Stats tracker.
    stats: Stats,

    /// Cancellation checker.
    stop: S,

    /// Whether the repeat extension has been written.
    repeat_written: bool,
}

impl<W: Write, S: Stop> Encoder<W, S> {
    /// Create a new encoder.
    pub fn new(
        writer: W,
        config: EncoderConfig,
        limits: Limits,
        stop: S,
    ) -> Result<Self> {
        // Check cancellation
        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Validate dimensions
        limits.check_dimensions(config.width, config.height)?;

        let stats = Stats::new();

        // Create gif encoder
        let global_palette_bytes: Vec<u8> = config
            .global_palette
            .as_ref()
            .map(|p| p.iter().flat_map(|c| [c.r, c.g, c.b]).collect())
            .unwrap_or_default();

        let encoder = gif::Encoder::new(
            writer,
            config.width,
            config.height,
            &global_palette_bytes,
        )
        .map_err(|e| at!(GifError::from(e)))?;

        Ok(Self {
            encoder,
            config,
            previous_frame: None,
            frame_index: 0,
            limits,
            stats,
            stop,
            repeat_written: false,
        })
    }

    /// Create an encoder from metadata.
    pub fn from_metadata(
        writer: W,
        metadata: &Metadata,
        limits: Limits,
        stop: S,
    ) -> Result<Self> {
        let config = EncoderConfig {
            width: metadata.width,
            height: metadata.height,
            repeat: metadata.repeat,
            global_palette: metadata.global_palette.as_ref().map(|p| p.colors().to_vec()),
            use_transparency: true,
            #[cfg(feature = "quantize")]
            quality: 80,
        };

        Self::new(writer, config, limits, stop)
    }

    /// Get the encoder configuration.
    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// Get the stats.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Get the current frame index.
    pub fn frame_index(&self) -> usize {
        self.frame_index
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

        self.encoder
            .write_extension(gif::ExtensionData::Repetitions(repeat))
            .map_err(|e| at!(GifError::from(e)))?;

        self.repeat_written = true;
        Ok(())
    }

    /// Add a frame to the animation.
    ///
    /// The frame pixels must match the encoder dimensions.
    pub fn add_frame(&mut self, input: FrameInput) -> Result<()> {
        // Check cancellation
        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Validate dimensions
        if input.width != self.config.width || input.height != self.config.height {
            return Err(at!(GifError::FrameDimensionMismatch {
                expected_width: self.config.width,
                expected_height: self.config.height,
                actual_width: input.width,
                actual_height: input.height,
            }));
        }

        // Check frame count
        self.limits.check_frame_count(self.frame_index)?;

        // Ensure repeat is written before first frame
        self.ensure_repeat_written()?;

        // Quantize and encode the frame
        let frame = self.prepare_frame(&input)?;

        self.encoder
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
    fn prepare_frame(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        #[cfg(feature = "quantize")]
        {
            self.prepare_frame_quantized(input)
        }

        #[cfg(not(feature = "quantize"))]
        {
            self.prepare_frame_simple(input)
        }
    }

    /// Simple frame preparation without quantization.
    #[cfg(not(feature = "quantize"))]
    fn prepare_frame_simple(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        // Convert RGBA to the gif crate's expected format
        let mut rgba_bytes: Vec<u8> = input
            .pixels
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect();

        let mut frame =
            gif::Frame::from_rgba_speed(input.width, input.height, &mut rgba_bytes, 10);

        frame.delay = input.delay;

        Ok(frame)
    }

    /// Frame preparation with imagequant quantization.
    #[cfg(feature = "quantize")]
    fn prepare_frame_quantized(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        use imagequant::Attributes;

        // Set up quantizer
        let mut attr = Attributes::new();
        attr.set_quality(0, self.config.quality)
            .map_err(|_| at!(GifError::QuantizationFailed { message: "failed to set quality" }))?;

        // Prepare image data
        let rgba_slice: &[imagequant::RGBA] = unsafe {
            std::slice::from_raw_parts(
                input.pixels.as_ptr() as *const imagequant::RGBA,
                input.pixels.len(),
            )
        };

        let mut img = attr
            .new_image(rgba_slice, input.width as usize, input.height as usize, 0.0)
            .map_err(|_| at!(GifError::QuantizationFailed { message: "failed to create image" }))?;

        // Quantize
        let mut result = attr
            .quantize(&mut img)
            .map_err(|_| at!(GifError::QuantizationFailed { message: "quantization failed" }))?;

        // Set dithering
        result.set_dithering_level(1.0)
            .map_err(|_| at!(GifError::QuantizationFailed { message: "failed to set dithering" }))?;

        // Remap to palette
        let (palette, pixels) = result
            .remapped(&mut img)
            .map_err(|_| at!(GifError::QuantizationFailed { message: "remapping failed" }))?;

        // Find transparent index (most transparent color)
        let transparent_index = palette
            .iter()
            .enumerate()
            .filter(|(_, c)| c.a < 128)
            .max_by_key(|(_, c)| 255 - c.a)
            .map(|(i, _)| i as u8);

        // Build gif frame
        // Set palette
        let palette_bytes: Vec<u8> = palette
            .iter()
            .flat_map(|c| [c.r, c.g, c.b])
            .collect();

        let frame = gif::Frame {
            width: input.width,
            height: input.height,
            delay: input.delay,
            dispose: gif::DisposalMethod::Keep,
            transparent: transparent_index,
            palette: Some(palette_bytes),
            buffer: std::borrow::Cow::Owned(pixels),
            ..Default::default()
        };

        Ok(frame)
    }

    /// Finish encoding and return the writer.
    pub fn finish(self) -> Result<W> {
        let writer = self
            .encoder
            .into_inner()
            .map_err(|e| at!(GifError::from(e)))?;
        Ok(writer)
    }
}

/// Convenience function to encode frames to a byte vector.
pub fn encode_gif<S: Stop>(
    frames: &[FrameInput],
    config: EncoderConfig,
    limits: Limits,
    stop: S,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, config, limits, stop)?;

    for frame in frames {
        encoder.add_frame(frame.clone())?;
    }

    encoder.finish()?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;

    fn make_red_frame(width: u16, height: u16, delay: u16) -> FrameInput {
        let pixels = vec![Rgba::rgb(255, 0, 0); width as usize * height as usize];
        FrameInput::new(width, height, delay, pixels)
    }

    #[test]
    fn encode_single_frame() {
        let config = EncoderConfig::new(2, 2).repeat(Repeat::Once);
        let limits = Limits::default();

        let frame = make_red_frame(2, 2, 10);

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        encoder.add_frame(frame).unwrap();
        encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    fn encode_multiple_frames() {
        let config = EncoderConfig::new(2, 2).repeat(Repeat::Infinite);
        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        for _ in 0..3 {
            let frame = make_red_frame(2, 2, 10);
            encoder.add_frame(frame).unwrap();
        }

        encoder.finish().unwrap();

        assert!(output.len() > 50);
    }

    #[test]
    fn encode_dimension_mismatch() {
        let config = EncoderConfig::new(4, 4);
        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        // Wrong dimensions
        let frame = make_red_frame(2, 2, 10);
        let result = encoder.add_frame(frame);

        assert!(result.is_err());
    }

    #[test]
    fn encode_convenience_function() {
        let config = EncoderConfig::new(2, 2);
        let limits = Limits::default();

        let frames = vec![
            make_red_frame(2, 2, 10),
            make_red_frame(2, 2, 10),
        ];

        let output = encode_gif(&frames, config, limits, Unstoppable).unwrap();

        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    fn encode_with_limits() {
        let config = EncoderConfig::new(2, 2);
        let limits = Limits::default().max_frame_count(1);

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        // First frame OK
        encoder.add_frame(make_red_frame(2, 2, 10)).unwrap();

        // Second frame should fail
        let result = encoder.add_frame(make_red_frame(2, 2, 10));
        assert!(result.is_err());
    }
}
