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

use std::borrow::Cow;
use std::io::Write;

use enough::{Stop, Unstoppable};
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

/// Reusable scratch buffer for frame operations.
/// This avoids repeated allocations during encoding.
#[derive(Debug, Default)]
struct ScratchBuffer {
    /// Buffer for diff pixels - reused across frames
    diff_pixels: Vec<Rgba>,
    /// Buffer for frame pixels when cloning is needed
    frame_pixels: Vec<Rgba>,
}

/// Check if two pixels are similar within a tolerance.
/// Returns true if all RGBA channels differ by at most `tolerance`.
#[inline(always)]
fn pixels_similar(a: Rgba, b: Rgba, tolerance: u8) -> bool {
    if tolerance == 0 {
        return a == b;
    }
    let dr = (a.r as i16 - b.r as i16).unsigned_abs() as u8;
    let dg = (a.g as i16 - b.g as i16).unsigned_abs() as u8;
    let db = (a.b as i16 - b.b as i16).unsigned_abs() as u8;
    let da = (a.a as i16 - b.a as i16).unsigned_abs() as u8;
    dr <= tolerance && dg <= tolerance && db <= tolerance && da <= tolerance
}

/// Compare current frame to previous and find the minimal changed region.
///
/// Returns None if the entire frame has changed (no optimization possible).
#[cfg_attr(not(test), allow(dead_code))]
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

/// Compare current frame to previous and find the minimal changed region.
/// Uses a scratch buffer to avoid allocations.
///
/// Returns None if the entire frame has changed (no optimization possible).
/// Compute RGB RMSE between original RGBA pixels and palette-mapped output.
///
/// Skips fully transparent pixels (alpha == 0) since they're invisible.
/// Returns RMSE in 0-255 RGB space (0 = perfect, ~5 = invisible, ~20 = visible).
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
fn compute_remap_rmse(original: &[Rgba], indices: &[u8], palette_rgb: &[u8]) -> f32 {
    let mut total = 0u64;
    let mut count = 0u64;
    for (orig, &idx) in original.iter().zip(indices.iter()) {
        // Skip transparent pixels — they're invisible
        if orig.a == 0 {
            continue;
        }
        let base = idx as usize * 3;
        if base + 2 >= palette_rgb.len() {
            continue;
        }
        let dr = orig.r as i64 - palette_rgb[base] as i64;
        let dg = orig.g as i64 - palette_rgb[base + 1] as i64;
        let db = orig.b as i64 - palette_rgb[base + 2] as i64;
        total += (dr * dr + dg * dg + db * db) as u64;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((total as f64) / (count as f64)).sqrt() as f32
}

fn compute_frame_diff_pooled(
    current: &[Rgba],
    previous: &[Rgba],
    width: u16,
    height: u16,
    tolerance: u8,
    scratch: &mut ScratchBuffer,
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
            if !pixels_similar(current[idx], previous[idx], tolerance) {
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
        scratch.diff_pixels.clear();
        scratch.diff_pixels.push(Rgba::TRANSPARENT);
        return Some(DiffResult {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            pixels: core::mem::take(&mut scratch.diff_pixels),
        });
    }

    let diff_width = (max_x - min_x + 1) as u16;
    let diff_height = (max_y - min_y + 1) as u16;

    // If the changed region is the entire frame, no optimization benefit
    if diff_width == width && diff_height == height {
        return None;
    }

    // Extract the changed region, marking unchanged pixels as transparent
    // Reuse the scratch buffer
    scratch.diff_pixels.clear();
    let region_size = diff_width as usize * diff_height as usize;
    if scratch.diff_pixels.capacity() < region_size {
        scratch
            .diff_pixels
            .reserve(region_size - scratch.diff_pixels.capacity());
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let idx = y * w + x;
            if pixels_similar(current[idx], previous[idx], tolerance) {
                // Unchanged pixel (within tolerance) - mark transparent
                scratch.diff_pixels.push(Rgba::TRANSPARENT);
            } else {
                // Changed pixel - keep as is
                scratch.diff_pixels.push(current[idx]);
            }
        }
    }

    // Take ownership of the buffer (will be returned to scratch on next call)
    Some(DiffResult {
        left: min_x as u16,
        top: min_y as u16,
        width: diff_width,
        height: diff_height,
        pixels: core::mem::take(&mut scratch.diff_pixels),
    })
}

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
    /// let config = EncoderConfig::new()
    ///     .quantizer(Quantizer::quantizr());
    /// ```
    #[cfg(any(
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
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
fn default_buffer_frames(width: u16, height: u16) -> usize {
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
            // Default to shared palette for new encodes:
            // - Multi-frame: eliminates palette flicker, better LZW compression
            // - Single-frame: negligible overhead (histogram from 1 image)
            // Round-trip encoding (from_metadata) overrides this to false.
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            shared_palette: true,
            #[cfg(any(
                feature = "imagequant",
                feature = "quantizr",
                feature = "exoquant-deprecated",
                feature = "color_quant"
            ))]
            max_buffer_frames: 32, // Will be updated when encoder is created
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
            #[cfg(any(
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
    /// // Use imagequant (AGPL) for smallest files
    /// let config = EncoderConfig::new()
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

    /// Set the per-frame palette error threshold for hybrid palette mode.
    ///
    /// When `shared_palette` is enabled and a threshold is set, frames
    /// whose RGB RMSE exceeds this value get their own local color table.
    /// Set to `None` to always use the shared palette.
    ///
    /// Default: `Some(15.0)` — barely visible errors use the shared palette,
    /// frames with very different colors get per-frame palettes automatically.
    #[cfg(any(
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

// Default instances for EncodeRequest::new()
static DEFAULT_LIMITS: Limits = Limits {
    max_width: Some(16384),
    max_height: Some(16384),
    max_total_pixels: Some(100_000_000),
    max_frame_count: Some(10_000),
    max_file_size: Some(100 * 1024 * 1024),
    max_memory: Some(1024 * 1024 * 1024),
    max_decompression_ratio: Some(1000.0),
};

static UNSTOPPABLE: Unstoppable = Unstoppable;

/// Request to encode a GIF animation.
///
/// Intermediate layer between `EncoderConfig` and `Encoder`. Binds configuration,
/// limits, and control parameters before encoding.
pub struct EncodeRequest<'a> {
    config: &'a EncoderConfig,
    width: u16,
    height: u16,
    limits: &'a Limits,
    stop: &'a dyn Stop,
}

impl<'a> EncodeRequest<'a> {
    /// Create a new encode request.
    pub fn new(config: &'a EncoderConfig, width: u16, height: u16) -> Self {
        Self {
            config,
            width,
            height,
            limits: &DEFAULT_LIMITS,
            stop: &UNSTOPPABLE,
        }
    }

    /// Set limits for this encode operation.
    #[must_use]
    pub fn limits(mut self, limits: &'a Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Set stop token for cooperative cancellation.
    #[must_use]
    pub fn stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = stop;
        self
    }

    /// One-shot encode: encode all frames and return bytes.
    pub fn encode(self, frames: Vec<FrameInput>) -> Result<Vec<u8>> {
        let mut encoder = self.build()?;
        for frame in frames {
            encoder.add_frame(frame)?;
        }
        encoder.finish()
    }

    /// One-shot encode: encode all frames into provided buffer.
    pub fn encode_into(self, frames: Vec<FrameInput>, out: &mut Vec<u8>) -> Result<()> {
        let bytes = self.encode(frames)?;
        out.extend_from_slice(&bytes);
        Ok(())
    }

    /// One-shot encode: encode all frames to a writer.
    #[cfg(feature = "std")]
    pub fn encode_to<W: Write>(self, frames: Vec<FrameInput>, mut dest: W) -> Result<()> {
        let bytes = self.encode(frames)?;
        dest.write_all(&bytes)
            .map_err(|e| at!(GifError::from(e)))?;
        Ok(())
    }

    /// Create a streaming encoder for frame-by-frame encoding.
    pub fn build(self) -> Result<Encoder<'a>> {
        Encoder::from_request(self)
    }
}

/// Streaming GIF encoder (no generics!).
///
/// Created via `EncodeRequest::build()`. Add frames with `add_frame()`,
/// then call `finish()` to get the encoded bytes.
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
                .map(|p| p.iter().flat_map(|c| [c.r, c.g, c.b]).collect())
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

/*
impl<S: Stop> Encoder<Vec<u8>, S> {
    /// Finish encoding and append the output to an existing buffer.
    ///
    /// This allows reusing a buffer across multiple encoding operations
    /// without intermediate allocations.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use zengif::*;
    /// # use enough::Unstoppable;
    /// let mut buf = Vec::new();
    /// let config = EncoderConfig::new();
    /// let req = EncodeRequest::new(&config, 100, 100).limits(&Limits::default()).stop(&Unstoppable);
    /// let encoder = req.build()?;
    /// // ... add frames ...
    /// encoder.finish_into(&mut buf)?;
    /// # Ok::<(), whereat::At<GifError>>(())
    /// ```
    pub fn finish_into(self, buf: &mut Vec<u8>) -> Result<()> {
        let encoded = self.finish()?;
        buf.extend_from_slice(&encoded);
        Ok(())
    }
}
*/

/// Convenience function to encode frames to a byte vector.
///
/// Takes ownership of the frames to avoid cloning pixel buffers.
pub fn encode_gif<S: Stop>(
    frames: Vec<FrameInput>,
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
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
        .stop(&stop);
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
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub fn encode_gif_shared_palette<S: Stop + Clone>(
    frames: Vec<FrameInput>,
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
) -> Result<Vec<u8>> {
    // Select quantizer based on available features (priority: imagequant > quantizr > color_quant > exoquant)
    #[cfg(feature = "imagequant")]
    let quantizer = crate::quantize::ImagequantQuantizer::new();

    #[cfg(all(feature = "quantizr", not(feature = "imagequant")))]
    let quantizer = crate::quantize::QuantizrQuantizer::new();

    #[cfg(all(
        feature = "color_quant",
        not(feature = "imagequant"),
        not(feature = "quantizr")
    ))]
    let quantizer = crate::quantize::ColorQuantQuantizer::new();

    #[cfg(all(
        feature = "exoquant-deprecated",
        not(feature = "imagequant"),
        not(feature = "quantizr"),
        not(feature = "color_quant")
    ))]
    let quantizer = crate::quantize::ExoquantQuantizer::new();

    encode_gif_with_quantizer(frames, width, height, config, limits, stop, quantizer)
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
    width: u16,
    height: u16,
    config: EncoderConfig,
    limits: Limits,
    stop: S,
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
        quantizer.build_shared_palette(&frame_refs, width, height, &quant_config, &stop)?;

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
    use super::*;
    use enough::Unstoppable;

    fn make_red_frame(width: u16, height: u16, delay: u16) -> FrameInput {
        let pixels = vec![Rgba::rgb(255, 0, 0); width as usize * height as usize];
        FrameInput::new(width, height, delay, pixels)
    }

    #[test]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn encode_single_frame() {
        let config = EncoderConfig::new().repeat(Repeat::Once);
        let limits = Limits::default();

        let frame = make_red_frame(2, 2, 10);

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2).limits(&limits).stop(&Unstoppable).build().unwrap();

        encoder.add_frame(frame).unwrap();
        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn encode_multiple_frames() {
        let config = EncoderConfig::new().repeat(Repeat::Infinite);
        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2).limits(&limits).stop(&Unstoppable).build().unwrap();

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
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

        // Wrong dimensions
        let frame = make_red_frame(2, 2, 10);
        let result = encoder.add_frame(frame);

        assert!(result.is_err());
    }

    #[test]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn encode_convenience_function() {
        let config = EncoderConfig::new();
        let limits = Limits::default();

        let frames = vec![make_red_frame(2, 2, 10), make_red_frame(2, 2, 10)];

        let output = encode_gif(frames, 2, 2, config, limits, Unstoppable).unwrap();

        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[test]
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn encode_with_limits() {
        let config = EncoderConfig::new();
        let limits = Limits::default().max_frame_count(1);

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 2, 2).limits(&limits).stop(&Unstoppable).build().unwrap();

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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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
            let output = encoder.finish().unwrap();
            output
        };

        // Encode without transparency optimization
        let config_no_opt = config.use_transparency(false);
        let output_without_diff = {
            // output will be returned from encoder.finish()
            let mut encoder =
                EncodeRequest::new(&config_no_opt, 100, 100).limits(&limits).stop(&Unstoppable).build().unwrap();
            encoder.add_frame(frame1).unwrap();
            encoder.add_frame(frame2).unwrap();
            let output = encoder.finish().unwrap();
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

        let config = EncoderConfig::new().repeat(Repeat::Infinite).dithering(0.0); // No dithering for deterministic test
        let limits = Limits::default();

        let output = encode_gif_shared_palette(
            vec![frame1, frame2, frame3],
            4,
            4,
            config,
            limits,
            Unstoppable,
        )
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
            Unstoppable,
        )
        .unwrap();

        // Encode with per-frame palettes (normal encode_gif)
        let output_perframe =
            encode_gif(frames, 64, 64, config_perframe, limits, Unstoppable).unwrap();

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

        let config_low = EncoderConfig::new().repeat(Repeat::Once).dithering(0.0);
        let config_high = EncoderConfig::new().repeat(Repeat::Once).dithering(1.0);

        let limits = Limits::default();

        let output_low = encode_gif(
            vec![frame.clone()],
            64,
            64,
            config_low,
            limits.clone(),
            Unstoppable,
        )
        .unwrap();
        let output_high =
            encode_gif(vec![frame], 64, 64, config_high, limits, Unstoppable).unwrap();

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
        let config = EncoderConfig::new().for_round_trip();

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
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(true)
            .max_buffer_frames(3); // Buffer up to 3 frames

        let limits = Limits::default();

        // output will be returned from encoder.finish()
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

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
        let mut encoder = EncodeRequest::new(&config, 2, 2).limits(&limits).stop(&Unstoppable).build().unwrap();
        encoder.add_frame(frame).unwrap();
        let output = encoder.finish().unwrap();

        // Should have produced valid GIF
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

        for frame in &frames {
            encoder.add_frame(frame.clone()).unwrap();
        }
        let output = encoder.finish().unwrap();

        // Decode and verify all frames came through
        let limits = crate::limits::Limits::none();
        let (meta, decoded_frames, _stats) =
            crate::decode::decode_gif(&output, limits, Unstoppable).unwrap();
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
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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
        let mut encoder = EncodeRequest::new(&config, 4, 4).limits(&limits).stop(&Unstoppable).build().unwrap();

        for frame in &frames {
            encoder.add_frame(frame.clone()).unwrap();
        }
        let output = encoder.finish().unwrap();

        // Should produce valid GIF (we're just testing it doesn't panic/error)
        assert!(output.len() > 10);
        assert_eq!(&output[0..6], b"GIF89a");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn default_buffer_frames_scales_with_dimensions() {
        // Small images get more frames for better palette coverage
        assert_eq!(super::default_buffer_frames(100, 100), 32); // 10K pixels → max
        assert_eq!(super::default_buffer_frames(256, 256), 30); // 65K pixels

        // Medium images get moderate buffering
        assert_eq!(super::default_buffer_frames(512, 512), 7); // 262K pixels

        // Large images get fewer frames for faster palette refresh
        assert_eq!(super::default_buffer_frames(1920, 1080), 4); // 2M pixels → min
        assert_eq!(super::default_buffer_frames(3840, 2160), 4); // 8M pixels → min

        // Edge case: zero dimensions
        assert_eq!(super::default_buffer_frames(0, 100), 32);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_remap_rmse_perfect_match() {
        let pixels = vec![Rgba::rgb(255, 0, 0), Rgba::rgb(0, 255, 0)];
        let indices = vec![0u8, 1u8];
        let palette = vec![255, 0, 0, 0, 255, 0]; // RGB entries

        let rmse = super::compute_remap_rmse(&pixels, &indices, &palette);
        assert!(rmse < 0.01, "perfect match should have ~0 RMSE, got {rmse}");
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
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

        let rmse = super::compute_remap_rmse(&pixels, &indices, &palette);
        assert!(
            rmse < 0.01,
            "transparent pixels should be skipped, got RMSE={rmse}"
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
