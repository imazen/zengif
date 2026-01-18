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

use std::io::Write;

use enough::Stop;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::stats::Stats;
use crate::types::{FrameInput, Metadata, Repeat, Rgba};

/// Strategy for palette selection during encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PaletteStrategy {
    /// Each frame gets its own optimal 256-color palette.
    ///
    /// Pros: Best color accuracy per frame.
    /// Cons: Can cause flickering between frames, larger file size.
    #[default]
    PerFrame,

    /// Compute a single shared palette from all frames.
    ///
    /// Pros: No flickering, better LZW compression.
    /// Cons: May lose some color accuracy, requires pre-collecting all frames.
    ///
    /// Use `encode_gif_shared_palette` for this strategy.
    Shared,

    /// Use the provided global palette without re-quantizing.
    ///
    /// Best for round-trip encoding when preserving the original palette.
    /// Falls back to PerFrame if no global palette is set.
    Global,
}

/// Result of frame differencing analysis.
#[derive(Debug, Clone)]
struct DiffResult {
    /// Left offset of the changed region.
    left: u16,
    /// Top offset of the changed region.
    top: u16,
    /// Width of the changed region.
    width: u16,
    /// Height of the changed region.
    height: u16,
    /// Pixels for the changed region with unchanged pixels marked transparent.
    pixels: Vec<Rgba>,
}

/// Compare current frame to previous and find the minimal changed region.
///
/// Returns None if the entire frame has changed (no optimization possible).
fn compute_frame_diff(
    current: &[Rgba],
    previous: &[Rgba],
    width: u16,
    height: u16,
) -> Option<DiffResult> {
    let w = width as usize;
    let h = height as usize;

    // Find bounding box of changed pixels
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if current[idx] != previous[idx] {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    // No changes at all - shouldn't happen in practice but handle gracefully
    if min_x > max_x || min_y > max_y {
        // Emit a 1x1 transparent frame at origin
        return Some(DiffResult {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            pixels: vec![Rgba::TRANSPARENT],
        });
    }

    let diff_width = (max_x - min_x + 1) as u16;
    let diff_height = (max_y - min_y + 1) as u16;

    // If the changed region is the entire frame, no optimization benefit
    if diff_width == width && diff_height == height {
        return None;
    }

    // Extract the changed region, marking unchanged pixels as transparent
    let mut pixels = Vec::with_capacity(diff_width as usize * diff_height as usize);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let idx = y * w + x;
            if current[idx] == previous[idx] {
                // Unchanged pixel - mark transparent
                pixels.push(Rgba::TRANSPARENT);
            } else {
                // Changed pixel - keep as is
                pixels.push(current[idx]);
            }
        }
    }

    Some(DiffResult {
        left: min_x as u16,
        top: min_y as u16,
        width: diff_width,
        height: diff_height,
        pixels,
    })
}

/// Encoder configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub quality: u8,

    /// Dithering level (0.0-1.0). Lower values = less noise = better compression.
    /// Default is 0.5. Use 0.0 for re-encoding already-dithered content.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub dithering: f32,

    /// If true, buffer frames and compute a shared palette before encoding.
    /// This improves compression and reduces flickering in animations.
    ///
    /// When using the streaming `Encoder::add_frame()` API with this enabled,
    /// frames are buffered until `max_buffer_frames` or `max_buffer_bytes` is
    /// reached, at which point the palette is computed and all buffered frames
    /// are encoded. Subsequent frames use the shared palette immediately.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub shared_palette: bool,

    /// Maximum frames to buffer before building shared palette.
    /// Default is 64. Only used when `shared_palette` is true.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub max_buffer_frames: usize,

    /// Maximum bytes to buffer before building shared palette.
    /// Default is 64MB. Only used when `shared_palette` is true.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub max_buffer_bytes: usize,

    /// Quantizer backend to use. Doc-hidden for internal/testing use.
    /// Default is Imagequant when available, falls back to the first available backend.
    ///
    /// **Deprecated**: Use `quantizer` field instead.
    #[doc(hidden)]
    #[deprecated(since = "0.2.0", note = "Use the `quantizer` field instead")]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub quantizer_backend: crate::quantize::QuantizerBackend,

    /// Quantizer selection and configuration.
    ///
    /// Use this to select which quantization backend to use and configure it.
    /// If not set, defaults to the best available backend (quantizr > imagequant > color_quant).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use zengif::{EncoderConfig, Quantizer};
    ///
    /// let config = EncoderConfig::new(100, 100)
    ///     .quantizer(Quantizer::quantizr());
    /// ```
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub quantizer: Option<crate::quantize::Quantizer>,
}

impl EncoderConfig {
    /// Create a new encoder configuration.
    #[allow(deprecated)] // quantizer_backend is deprecated
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            repeat: Repeat::Infinite,
            global_palette: None,
            use_transparency: true,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quality: 80,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            dithering: 0.5, // Lower default for better compression
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            shared_palette: false,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_frames: 64,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_bytes: 64 * 1024 * 1024, // 64 MB
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
            quantizer: None, // Use default auto-selection
        }
    }

    /// Set the quantizer backend to use.
    ///
    /// **Deprecated**: Use [`quantizer`](Self::quantizer) instead.
    #[doc(hidden)]
    #[deprecated(since = "0.2.0", note = "Use the `quantizer` method instead")]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    #[allow(deprecated)]
    pub fn quantizer_backend(mut self, backend: crate::quantize::QuantizerBackend) -> Self {
        self.quantizer_backend = backend;
        self
    }

    /// Set the quantizer to use.
    ///
    /// Use this to select which quantization backend to use and configure it.
    /// See [`Quantizer`](crate::quantize::Quantizer) for available options.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use zengif::{EncoderConfig, Quantizer};
    ///
    /// // Use quantizr with default settings
    /// let config = EncoderConfig::new(100, 100)
    ///     .quantizer(Quantizer::quantizr());
    ///
    /// // Use quantizr with custom dithering
    /// let config = EncoderConfig::new(100, 100)
    ///     .quantizer(Quantizer::quantizr_with_dithering(0.3));
    ///
    /// // Use imagequant (AGPL) for smallest files
    /// let config = EncoderConfig::new(100, 100)
    ///     .quantizer(Quantizer::imagequant());
    /// ```
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn quantizer(mut self, quantizer: crate::quantize::Quantizer) -> Self {
        self.quantizer = Some(quantizer);
        self
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
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = quality.clamp(1, 100);
        self
    }

    /// Set dithering level (0.0-1.0).
    ///
    /// Lower values produce less noise and better LZW compression.
    /// Use 0.0 when re-encoding already-dithered content (round-trip).
    /// Default is 0.5.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn dithering(mut self, level: f32) -> Self {
        self.dithering = level.clamp(0.0, 1.0);
        self
    }

    /// Enable shared palette mode.
    ///
    /// When true, frames are buffered until buffer limits are reached,
    /// then a single palette is computed and used for all frames.
    /// This improves compression and reduces flickering.
    ///
    /// For streaming encoding, frames are buffered up to `max_buffer_frames`
    /// or `max_buffer_bytes`, then the palette is built and buffered frames
    /// are encoded. Subsequent frames use the shared palette immediately.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn shared_palette(mut self, shared: bool) -> Self {
        self.shared_palette = shared;
        self
    }

    /// Set maximum frames to buffer for shared palette building.
    ///
    /// When `shared_palette` is enabled, frames are buffered until this
    /// limit is reached, then the palette is computed and encoding begins.
    /// Default is 64 frames.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn max_buffer_frames(mut self, max: usize) -> Self {
        self.max_buffer_frames = max;
        self
    }

    /// Set maximum bytes to buffer for shared palette building.
    ///
    /// When `shared_palette` is enabled, frames are buffered until this
    /// memory limit is reached, then the palette is computed and encoding begins.
    /// Default is 64 MB.
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn max_buffer_bytes(mut self, max: usize) -> Self {
        self.max_buffer_bytes = max;
        self
    }

    /// Configure for optimal round-trip encoding.
    ///
    /// This sets parameters that minimize bloat when re-encoding a decoded GIF:
    /// - Zero dithering (content is already dithered)
    /// - Shared palette (consistent colors across frames)
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn for_round_trip(self) -> Self {
        self.dithering(0.0).shared_palette(true)
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
}

impl<W: Write, S: Stop> Encoder<W, S> {
    /// Create a new encoder.
    pub fn new(writer: W, config: EncoderConfig, limits: Limits, stop: S) -> Result<Self> {
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

        let encoder = gif::Encoder::new(writer, config.width, config.height, &global_palette_bytes)
            .map_err(|e| at!(GifError::from(e)))?;

        // Create quantizer from config.quantizer or fall back to quantizer_backend
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        #[allow(deprecated)] // quantizer_backend is deprecated
        let quantizer: Box<dyn crate::quantize::QuantizerTrait> =
            if let Some(ref q) = config.quantizer {
                q.create_backend()
            } else {
                config.quantizer_backend.create_quantizer().ok_or_else(|| {
                    at!(GifError::QuantizationFailed {
                        message: "selected quantizer backend is not available"
                    })
                })?
            };

        Ok(Self {
            encoder,
            config,
            previous_frame: None,
            frame_index: 0,
            limits,
            stats,
            stop,
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
        })
    }

    /// Create an encoder from metadata.
    ///
    /// This preserves the original global palette if available, and uses
    /// round-trip optimized settings (zero dithering) to minimize bloat.
    #[allow(deprecated)] // quantizer_backend is deprecated
    pub fn from_metadata(writer: W, metadata: &Metadata, limits: Limits, stop: S) -> Result<Self> {
        let config = EncoderConfig {
            width: metadata.width,
            height: metadata.height,
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
            max_buffer_frames: 64,
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
    ///
    /// When `shared_palette` is enabled, frames are buffered until buffer
    /// limits are reached, then the palette is computed and all frames are
    /// encoded. Subsequent frames are encoded immediately with the shared palette.
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

        // Check frame count (including buffered frames)
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        let total_frames = self.frame_index + self.buffered_frames.len();
        #[cfg(not(feature = "imagequant"))]
        let total_frames = self.frame_index;
        self.limits.check_frame_count(total_frames)?;

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
            self.config.width,
            self.config.height,
            &quant_config,
            &self.stop,
        )?;

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
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        {
            self.prepare_frame_quantized(input)
        }

        #[cfg(not(feature = "imagequant"))]
        {
            self.prepare_frame_simple(input)
        }
    }

    /// Simple frame preparation without quantization.
    #[cfg(not(feature = "imagequant"))]
    fn prepare_frame_simple(&mut self, input: &FrameInput) -> Result<gif::Frame<'static>> {
        // Check if we can optimize using frame differencing
        let (frame_pixels, frame_left, frame_top, frame_width, frame_height) =
            if self.config.use_transparency {
                if let Some(ref prev) = self.previous_frame {
                    if let Some(diff) =
                        compute_frame_diff(&input.pixels, prev, input.width, input.height)
                    {
                        // Use the optimized diff region
                        (diff.pixels, diff.left, diff.top, diff.width, diff.height)
                    } else {
                        // No optimization possible, use full frame
                        (input.pixels.clone(), 0, 0, input.width, input.height)
                    }
                } else {
                    // First frame, no diff possible
                    (input.pixels.clone(), 0, 0, input.width, input.height)
                }
            } else {
                // Transparency optimization disabled
                (input.pixels.clone(), 0, 0, input.width, input.height)
            };

        // If frame has a palette, use it directly (pass-through mode)
        if let Some(ref palette) = input.palette {
            let (pixels, transparent_index) = palette.map_pixels(&frame_pixels);
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
                buffer: std::borrow::Cow::Owned(pixels),
                ..Default::default()
            };

            return Ok(frame);
        }

        // Convert RGBA to the gif crate's expected format
        let mut rgba_bytes: Vec<u8> = frame_pixels
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect();

        let mut frame = gif::Frame::from_rgba_speed(frame_width, frame_height, &mut rgba_bytes, 10);

        frame.left = frame_left;
        frame.top = frame_top;
        frame.delay = input.delay;

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
                    if let Some(diff) =
                        compute_frame_diff(&input.pixels, prev, input.width, input.height)
                    {
                        // Use the optimized diff region
                        (diff.pixels, diff.left, diff.top, diff.width, diff.height)
                    } else {
                        // No optimization possible, use full frame
                        (input.pixels.clone(), 0, 0, input.width, input.height)
                    }
                } else {
                    // First frame, no diff possible
                    (input.pixels.clone(), 0, 0, input.width, input.height)
                }
            } else {
                // Transparency optimization disabled
                (input.pixels.clone(), 0, 0, input.width, input.height)
            };

        // If frame has a palette, use it directly (pass-through mode)
        if let Some(ref palette) = input.palette {
            let (pixels, transparent_index) = palette.map_pixels(&frame_pixels);
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
                buffer: std::borrow::Cow::Owned(pixels),
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

        // Use shared palette if available, otherwise per-frame quantization
        let (palette_bytes, pixels, transparent_index) = if self.computed_palette.is_some() {
            // Shared palette mode: use pre-computed palette
            let background = self.previous_frame.as_deref();
            let quantized = self.quantizer.quantize_frame_with_palette(
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
            )
        } else {
            // Per-frame quantization
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
            )
        };

        let frame = gif::Frame {
            left: frame_left,
            top: frame_top,
            width: frame_width,
            height: frame_height,
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
    ///
    /// If there are buffered frames (from shared palette mode), they are
    /// encoded before finishing.
    #[allow(unused_mut)]
    pub fn finish(mut self) -> Result<W> {
        // Flush any remaining buffered frames
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        self.flush_buffer()?;

        let writer = self
            .encoder
            .into_inner()
            .map_err(|e| at!(GifError::from(e)))?;
        Ok(writer)
    }
}

/// Convenience function to encode frames to a byte vector.
///
/// Takes ownership of the frames to avoid cloning pixel buffers.
pub fn encode_gif<S: Stop>(
    frames: Vec<FrameInput>,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
) -> Result<Vec<u8>> {
    // Estimate initial output size (header + per-frame overhead)
    // GIF header ~13 bytes, each frame has overhead of ~100-500 bytes + compressed data
    // This is a conservative estimate to reduce reallocations
    let estimated_size = 1024 + frames.len() * 512;

    let mut output = Vec::new();
    output.try_reserve(estimated_size).map_err(|_| {
        at!(GifError::AllocationFailed {
            requested: estimated_size
        })
    })?;

    let mut encoder = Encoder::new(&mut output, config, limits, stop)?;

    for frame in frames {
        encoder.add_frame(frame)?;
    }

    encoder.finish()?;
    Ok(output)
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
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub fn encode_gif_shared_palette<S: Stop + Clone>(
    frames: Vec<FrameInput>,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
) -> Result<Vec<u8>> {
    encode_gif_with_quantizer(
        frames,
        config,
        limits,
        stop,
        crate::quantize::ImagequantQuantizer::new(),
    )
}

/// Encode frames using a custom quantizer.
///
/// This is the generic version that accepts any [`Quantizer`](crate::Quantizer)
/// implementation, allowing for custom quantization algorithms.
///
/// See [`encode_gif_shared_palette`] for the default imagequant-based version.
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub fn encode_gif_with_quantizer<S: Stop + Clone, Q: crate::quantize::QuantizerTrait>(
    frames: Vec<FrameInput>,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
    mut quantizer: Q,
) -> Result<Vec<u8>> {
    use crate::quantize::QuantizeConfig;

    if frames.is_empty() {
        return encode_gif(frames, config, limits, stop);
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
    let palette_bytes = quantizer.build_shared_palette(
        &frame_refs,
        config.width,
        config.height,
        &quant_config,
        &stop,
    )?;

    // Estimate output size
    let estimated_size = 1024 + frames.len() * 512;
    let mut output = Vec::new();
    output.try_reserve(estimated_size).map_err(|_| {
        at!(GifError::AllocationFailed {
            requested: estimated_size
        })
    })?;

    // Create encoder with global palette
    let mut gif_encoder =
        gif::Encoder::new(&mut output, config.width, config.height, &palette_bytes)
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
        limits.check_frame_count(frame_index)?;

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
            buffer: std::borrow::Cow::Owned(quantized.pixels),
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

    drop(gif_encoder);
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

        let frames = vec![make_red_frame(2, 2, 10), make_red_frame(2, 2, 10)];

        let output = encode_gif(frames, config, limits, Unstoppable).unwrap();

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
    fn frame_diff_produces_smaller_output() {
        // Encode two identical red frames - second should be tiny due to diff
        let config = EncoderConfig::new(100, 100)
            .repeat(Repeat::Once)
            .use_transparency(true);
        let limits = Limits::default();

        // Create two identical frames
        let frame1 = make_red_frame(100, 100, 10);
        let frame2 = make_red_frame(100, 100, 10);

        let output_with_diff = {
            let mut output = Vec::new();
            let mut encoder =
                Encoder::new(&mut output, config.clone(), limits.clone(), Unstoppable).unwrap();
            encoder.add_frame(frame1.clone()).unwrap();
            encoder.add_frame(frame2.clone()).unwrap();
            encoder.finish().unwrap();
            output
        };

        // Encode without transparency optimization
        let config_no_opt = config.use_transparency(false);
        let output_without_diff = {
            let mut output = Vec::new();
            let mut encoder =
                Encoder::new(&mut output, config_no_opt, limits, Unstoppable).unwrap();
            encoder.add_frame(frame1).unwrap();
            encoder.add_frame(frame2).unwrap();
            encoder.finish().unwrap();
            output
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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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

        let config = EncoderConfig::new(width, height)
            .repeat(Repeat::Infinite)
            .dithering(0.0); // No dithering for deterministic test
        let limits = Limits::default();

        let output =
            encode_gif_shared_palette(vec![frame1, frame2, frame3], config, limits, Unstoppable)
                .unwrap();

        // Should produce valid GIF
        assert!(output.len() > 100);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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

        let config_shared = EncoderConfig::new(width, height)
            .repeat(Repeat::Once)
            .dithering(0.0);
        let config_perframe = EncoderConfig::new(width, height)
            .repeat(Repeat::Once)
            .dithering(0.0);

        let limits = Limits::default();

        // Encode with shared palette
        let output_shared =
            encode_gif_shared_palette(frames.clone(), config_shared, limits.clone(), Unstoppable)
                .unwrap();

        // Encode with per-frame palettes (normal encode_gif)
        let output_perframe = encode_gif(frames, config_perframe, limits, Unstoppable).unwrap();

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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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

        let config_low = EncoderConfig::new(width, height)
            .repeat(Repeat::Once)
            .dithering(0.0);
        let config_high = EncoderConfig::new(width, height)
            .repeat(Repeat::Once)
            .dithering(1.0);

        let limits = Limits::default();

        let output_low =
            encode_gif(vec![frame.clone()], config_low, limits.clone(), Unstoppable).unwrap();
        let output_high = encode_gif(vec![frame], config_high, limits, Unstoppable).unwrap();

        // Low dithering should produce smaller output (less noise = better LZW)
        assert!(
            output_low.len() < output_high.len(),
            "Low dithering ({} bytes) should be smaller than high dithering ({} bytes)",
            output_low.len(),
            output_high.len()
        );
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn for_round_trip_config() {
        let config = EncoderConfig::new(100, 100).for_round_trip();

        // Should have zero dithering and shared palette enabled
        assert_eq!(config.dithering, 0.0);
        assert!(config.shared_palette);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_shared_palette() {
        // Test that streaming encoder buffers frames and builds shared palette
        let config = EncoderConfig::new(4, 4)
            .repeat(Repeat::Infinite)
            .shared_palette(true)
            .max_buffer_frames(3); // Buffer up to 3 frames

        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        // Add 5 frames - should buffer first 3, then flush and encode
        for i in 0..5 {
            let color = ((i * 50) % 256) as u8;
            let pixels = vec![Rgba::rgb(color, color, color); 16];
            let frame = FrameInput::new(4, 4, 10, pixels);
            encoder.add_frame(frame).unwrap();
        }

        encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 50, "Should produce valid GIF output");
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_flushes_on_finish() {
        // Test that finish() flushes remaining buffered frames
        let config = EncoderConfig::new(4, 4)
            .repeat(Repeat::Once)
            .shared_palette(true)
            .max_buffer_frames(10); // Large buffer - won't hit limit

        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        // Add only 2 frames - less than buffer limit
        for _ in 0..2 {
            let frame = make_red_frame(4, 4, 10);
            encoder.add_frame(frame).unwrap();
        }

        // finish() should flush the buffer
        encoder.finish().unwrap();

        // Should have produced valid GIF with content
        assert!(output.len() > 50, "Should produce valid GIF output");
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn buffered_streaming_memory_limit() {
        // Test that buffer flushes when memory limit is reached
        let config = EncoderConfig::new(4, 4)
            .repeat(Repeat::Once)
            .shared_palette(true)
            .max_buffer_frames(1000) // High frame limit
            .max_buffer_bytes(100); // Low memory limit (~1 frame = 64 bytes RGBA)

        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();

        // Add 5 frames - should trigger memory limit flush
        for _ in 0..5 {
            let frame = make_red_frame(4, 4, 10);
            encoder.add_frame(frame).unwrap();
        }

        encoder.finish().unwrap();

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

        let config = EncoderConfig::new(2, 2).repeat(Repeat::Once);
        let limits = Limits::default();

        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable).unwrap();
        encoder.add_frame(frame).unwrap();
        encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
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
