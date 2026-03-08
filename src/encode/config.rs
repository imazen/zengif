//! Encoder configuration.

use crate::types::{Repeat, Rgba};

/// Encoder configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EncoderConfig {
    /// Loop behavior.
    pub repeat: Repeat,

    /// Global palette (if any).
    pub global_palette: Option<Vec<Rgba>>,

    /// Whether to use transparency for unchanged pixels.
    pub use_transparency: bool,

    /// Quality setting for quantization (1-100, higher = better quality).
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub quality: u8,

    /// Dithering level (0.0-1.0). Lower values = less noise = better compression.
    /// Default is 0.5. Use 0.0 for re-encoding already-dithered content.
    #[cfg(any(
        feature = "zenquant",
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
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub shared_palette: bool,

    /// Maximum frames to buffer before building shared palette.
    /// Default is 64. Only used when `shared_palette` is true.
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub max_buffer_frames: usize,

    /// Maximum bytes to buffer before building shared palette.
    /// Default is 64MB. Only used when `shared_palette` is true.
    #[cfg(any(
        feature = "zenquant",
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
        feature = "zenquant",
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
    /// let config = EncoderConfig::new()
    ///     .quantizer(Quantizer::quantizr());
    /// ```
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub quantizer: Option<crate::quantize::Quantizer>,

    /// Per-frame palette error threshold for hybrid palette mode.
    ///
    /// When `shared_palette` is true and this is `Some(threshold)`, frames
    /// whose RMSE (RGB, 0-255 scale) exceeds the threshold get their own
    /// local color table instead of inheriting the global one.
    ///
    /// This gives the best of both worlds: most frames use the global
    /// palette (no flicker, saves 768 bytes each), but outlier frames
    /// with very different colors get accurate per-frame palettes.
    ///
    /// RMSE guidelines (0-255 RGB scale):
    /// - < 2: imperceptible difference
    /// - 2-5: slight difference, visible on close inspection
    /// - 5-10: noticeable difference in color accuracy
    /// - > 10: obvious color distortion
    ///
    /// - `None`: always use the shared palette (no fallback)
    /// - `Some(5.0)`: catches most problematic frames (default)
    /// - `Some(2.0)`: strict — more frames get per-frame palettes
    /// - `Some(15.0)`: permissive — only severe outliers get per-frame palettes
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    pub palette_error_threshold: Option<f32>,

    /// Lossy frame differencing tolerance per color channel (0-255).
    ///
    /// When set, pixels whose RGBA channels are all within `tolerance` of
    /// the previous frame are considered unchanged. This can significantly
    /// reduce the dirty region size and improve compression.
    ///
    /// - `0`: exact matching only (default, lossless)
    /// - `2-4`: imperceptible differences, good compression boost
    /// - `8-16`: visible on close inspection, major compression gains
    ///
    /// **Note**: This is lossy - the output won't be pixel-perfect.
    /// Use 0 for round-trip encoding or when exact reproduction is needed.
    pub lossy_tolerance: u8,
}

/// Compute default buffer frame count based on image dimensions.
///
/// Targets ~2 megapixels worth of frames for palette building.
/// Smaller images get more frames (better palette coverage),
/// larger images get fewer frames (faster palette refresh for scene changes).
///
/// | Dimensions  | Pixels | Buffer Frames |
/// |-------------|--------|---------------|
/// | 100×100     | 10K    | 32            |
/// | 256×256     | 65K    | 30            |
/// | 512×512     | 262K   | 8             |
/// | 1920×1080   | 2M     | 4             |
#[cfg(any(
    feature = "zenquant",
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub(super) fn default_buffer_frames(width: u16, height: u16) -> usize {
    const TARGET_PIXELS: u64 = 2_000_000; // ~2 megapixels worth of frames
    const MIN_FRAMES: u64 = 4;
    const MAX_FRAMES: u64 = 32;

    let pixels_per_frame = width as u64 * height as u64;
    if pixels_per_frame == 0 {
        return MAX_FRAMES as usize;
    }

    let frames = TARGET_PIXELS / pixels_per_frame;
    frames.clamp(MIN_FRAMES, MAX_FRAMES) as usize
}

impl EncoderConfig {
    /// Create a new encoder configuration.
    #[allow(deprecated)] // quantizer_backend is deprecated
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            repeat: Repeat::Infinite,
            global_palette: None,
            use_transparency: true,
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quality: 80,
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            dithering: 0.5, // Lower default for better compression
            // Default to shared palette for new encodes:
            // - Multi-frame: eliminates palette flicker, better LZW compression
            // - Single-frame: negligible overhead (histogram from 1 image)
            // Round-trip encoding (from_metadata) overrides this to false.
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            shared_palette: true,
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_frames: 32, // Will be updated when encoder is created
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_bytes: 64 * 1024 * 1024, // 64 MB
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quantizer_backend: crate::quantize::QuantizerBackend::default(),
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            quantizer: None, // Use default auto-selection
            #[cfg(any(
                feature = "zenquant",
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            palette_error_threshold: Some(5.0), // Hybrid: per-frame fallback when RMSE > 5
            lossy_tolerance: 0, // Lossless by default
        }
    }

    /// Set lossy frame differencing tolerance.
    ///
    /// Pixels within `tolerance` of the previous frame are considered unchanged.
    /// This reduces dirty region size and improves compression at the cost of
    /// some visual fidelity.
    ///
    /// - `0`: exact matching only (default, lossless)
    /// - `2-4`: imperceptible, good compression boost
    /// - `8-16`: visible on close inspection, major compression gains
    #[must_use]
    pub fn lossy_tolerance(mut self, tolerance: u8) -> Self {
        self.lossy_tolerance = tolerance;
        self
    }

    /// Set the quantizer backend to use.
    ///
    /// **Deprecated**: Use [`quantizer`](Self::quantizer) instead.
    #[doc(hidden)]
    #[deprecated(since = "0.2.0", note = "Use the `quantizer` method instead")]
    #[cfg(any(
        feature = "zenquant",
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
    /// let config = EncoderConfig::new()
    ///     .quantizer(Quantizer::quantizr());
    ///
    /// // Use quantizr with custom dithering
    /// let config = EncoderConfig::new()
    ///     .quantizer(Quantizer::quantizr_with_dithering(0.3));
    ///
    /// // Use imagequant (GPL) for smallest files
    /// let config = EncoderConfig::new()
    ///     .quantizer(Quantizer::imagequant());
    /// ```
    #[cfg(any(
        feature = "zenquant",
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
        feature = "zenquant",
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
        feature = "zenquant",
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
        feature = "zenquant",
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
        feature = "zenquant",
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
        feature = "zenquant",
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

    /// Set the per-frame palette error threshold for hybrid palette mode.
    ///
    /// When `shared_palette` is enabled and a threshold is set, frames
    /// whose RGB RMSE exceeds this value get their own local color table.
    /// Set to `None` to always use the shared palette.
    ///
    /// Default: `Some(15.0)` — barely visible errors use the shared palette,
    /// frames with very different colors get per-frame palettes automatically.
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn palette_error_threshold(mut self, threshold: Option<f32>) -> Self {
        self.palette_error_threshold = threshold;
        self
    }

    /// Configure for optimal round-trip encoding.
    ///
    /// This sets parameters that minimize bloat when re-encoding a decoded GIF:
    /// - Zero dithering (content is already dithered)
    /// - Shared palette (consistent colors across frames)
    #[cfg(any(
        feature = "zenquant",
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
