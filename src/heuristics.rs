//! Resource estimation heuristics for GIF encoding and decoding operations.
//!
//! These heuristics provide approximate estimates for memory consumption and
//! relative time costs of encoding/decoding operations. Use them for:
//!
//! - Pre-allocating buffers
//! - Sizing thread pools
//! - Memory budgeting
//! - Progress estimation
//!
//! # Accuracy
//!
//! Estimates are based on empirical measurements. Memory estimates include only
//! zengif's tracked allocations (canvas, pixel buffers), not the gif crate's
//! internal allocations or quantizer library allocations.
//!
//! # Content Type Impact
//!
//! Image content significantly affects both memory and time:
//!
//! | Content | Decode | Encode Memory | Encode Time |
//! |---------|--------|---------------|-------------|
//! | Solid   | Min    | Min           | **Fastest** |
//! | Gradient| Typical| Typical       | Fast        |
//! | Photo   | Typical| Typical       | **Slowest** |
//! | Noise   | Max    | Max           | Slow        |
//!
//! For photos (typical web content), quantization is the bottleneck.
//! For noise/high-entropy content, LZW compression is slower.
//!
//! # Quantizer Impact
//!
//! Different quantizers have dramatically different resource profiles:
//!
//! | Quantizer | Memory | Speed | Quality |
//! |-----------|--------|-------|---------|
//! | imagequant| ~3x frame | Slow | **Best** |
//! | quantizr  | ~1.5x frame | Fast | Good |
//! | color_quant| ~1.5x frame | **Fastest** | Good |
//!
//! # Example
//!
//! ```rust
//! use zengif::heuristics::{estimate_decode, estimate_encode, QuantizerType};
//!
//! // Estimate decode resources for a 640x480 animation with 30 frames
//! let decode_est = estimate_decode(640, 480, 30);
//! println!("Decode peak memory: {:.1} MB", decode_est.peak_memory_bytes as f64 / 1_000_000.0);
//! println!("Decode time: {:.0}ms (typical)", decode_est.time_ms);
//!
//! // Estimate encode resources
//! let encode_est = estimate_encode(640, 480, 30, QuantizerType::Imagequant);
//! println!("Encode peak memory: {:.1} MB", encode_est.peak_memory_bytes as f64 / 1_000_000.0);
//! println!("Encode time: {:.0}ms (typical)", encode_est.time_ms);
//! ```

// =============================================================================
// Constants derived from profiling (see examples/memory_profile.rs)
// Measured on 2026-01-21 using tracking allocator, zengif 0.2.1, release build
// =============================================================================

// --- Decode constants ---
// Measured decode_all(): ~10 B/pixel for single frame, ~5 B/pixel/frame for multi-frame
// Memory = canvas (4B) + indexed (1B) + output frames (4B × frame_count)

/// Fixed overhead for decode (gif crate structures, screen, buffers).
/// Measured: minimal, most memory is per-pixel.
const DECODE_FIXED_OVERHEAD: u64 = 50_000;

/// Decode throughput in Mpix/s for simple content (solid colors).
/// Measured: 300-950 Mpix/s for solid, using conservative 300.
const DECODE_THROUGHPUT_MAX_MPIXELS: f64 = 300.0;

/// Decode throughput in Mpix/s for typical content (photos, gradients).
/// Measured: 200-265 Mpix/s for photos with tracking allocator.
const DECODE_THROUGHPUT_TYP_MPIXELS: f64 = 230.0;

/// Decode throughput in Mpix/s for complex content (noise, many colors).
/// Measured: 74-170 Mpix/s for noise.
const DECODE_THROUGHPUT_MIN_MPIXELS: f64 = 75.0;

// --- Encode constants (no quantizer) ---

/// Bytes per pixel for encode base (previous frame buffer + output buffer).
const ENCODE_BASE_BYTES_PER_PIXEL: f64 = 5.0;

/// Fixed overhead for encode (gif crate structures, LZW tables).
const ENCODE_FIXED_OVERHEAD: u64 = 100_000;

// --- Quantizer-specific constants ---
// Measured using tracking allocator (examples/memory_profile.rs)
// These are TOTAL heap allocations including quantizer library internals.

/// Imagequant bytes per pixel (per-pixel working set above the fixed overhead).
/// Heaptrack/VmHWM-recalibrated 2026-06-23 (benchmarks/zengif_encode_mem_2026-06-23.tsv,
/// bike.png 256²..4096² single-frame, R²=1.000): the measured est slope is 41.5 B/px
/// total touched RSS = (base 5 + 30) × typ_mult 1.2. The old 24 (→ 28.8 est) under-
/// predicted ~16% — under-prediction is the unsafe direction for a memory budget.
const IMAGEQUANT_BYTES_PER_PIXEL: f64 = 30.0;

/// Imagequant fixed overhead.
/// Measured: ~1.7 MB fixed overhead (kmeans, histogram structures).
const IMAGEQUANT_FIXED_OVERHEAD: u64 = 1_700_000;

/// Imagequant throughput in Mpix/s for simple content.
/// Measured: 7-21 Mpix/s for solid colors.
const IMAGEQUANT_THROUGHPUT_MAX: f64 = 20.0;

/// Imagequant throughput in Mpix/s for typical photos.
/// Measured: 1.7-9.3 Mpix/s for photos (varies with size).
const IMAGEQUANT_THROUGHPUT_TYP: f64 = 5.0;

/// Imagequant throughput in Mpix/s for complex content.
/// Measured: 0.5-6 Mpix/s for noise.
const IMAGEQUANT_THROUGHPUT_MIN: f64 = 1.5;

/// Quantizr bytes per pixel.
/// Measured: 10-35 B/pixel depending on image size.
/// Using ~8 B/pixel (derived from linear regression).
const QUANTIZR_BYTES_PER_PIXEL: f64 = 8.0;

/// Quantizr fixed overhead.
/// Measured: ~1.7 MB fixed overhead (histogram hashmap).
const QUANTIZR_FIXED_OVERHEAD: u64 = 1_700_000;

/// Quantizr throughput in Mpix/s for simple content.
/// Measured: 55-65 Mpix/s for solid colors.
const QUANTIZR_THROUGHPUT_MAX: f64 = 60.0;

/// Quantizr throughput in Mpix/s for typical photos.
/// Measured: 2.7-10 Mpix/s for photos.
const QUANTIZR_THROUGHPUT_TYP: f64 = 5.0;

/// Quantizr throughput in Mpix/s for complex content.
/// Measured: 0.7-1.5 Mpix/s for noise (highly variable).
const QUANTIZR_THROUGHPUT_MIN: f64 = 1.0;

/// color_quant bytes per pixel.
/// Measured: consistently ~5 B/pixel across all sizes.
const COLOR_QUANT_BYTES_PER_PIXEL: f64 = 5.0;

/// color_quant fixed overhead (neural network weights).
/// Measured: ~4 KB fixed overhead (very efficient).
const COLOR_QUANT_FIXED_OVERHEAD: u64 = 4_000;

/// color_quant throughput in Mpix/s for simple content.
/// Measured: consistently 1.9-2.1 Mpix/s regardless of content.
const COLOR_QUANT_THROUGHPUT_MAX: f64 = 2.1;

/// color_quant throughput in Mpix/s for typical photos.
/// Measured: consistently 1.9-2.1 Mpix/s.
const COLOR_QUANT_THROUGHPUT_TYP: f64 = 2.0;

/// color_quant throughput in Mpix/s for complex content.
/// Measured: consistently 1.9-2.0 Mpix/s.
const COLOR_QUANT_THROUGHPUT_MIN: f64 = 1.9;

/// No quantizer (passthrough mode) - minimal overhead.
const NO_QUANT_BYTES_PER_PIXEL: f64 = 5.0;

/// No quantizer fixed overhead.
const NO_QUANT_FIXED_OVERHEAD: u64 = 50_000;

/// Passthrough throughput (limited by LZW encoding).
/// Based on decode measurements since passthrough skips quantization.
const NO_QUANT_THROUGHPUT_MAX: f64 = 200.0;
const NO_QUANT_THROUGHPUT_TYP: f64 = 100.0;
const NO_QUANT_THROUGHPUT_MIN: f64 = 50.0;

// =============================================================================
// Public types
// =============================================================================

/// Quantizer type for resource estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum QuantizerType {
    /// No quantizer (passthrough mode with pre-computed palettes).
    None,
    /// imagequant (libimagequant) - highest quality, slowest.
    #[default]
    Imagequant,
    /// quantizr - good quality, fast.
    Quantizr,
    /// color_quant (NeuQuant) - good quality, fastest.
    ColorQuant,
}

impl QuantizerType {
    /// Map a [`QuantizerBackend`](crate::quantize::QuantizerBackend) to the
    /// nearest resource profile understood by these heuristics.
    ///
    /// The estimator only models four resource profiles (passthrough,
    /// imagequant, quantizr, color_quant). `zenquant` and `quantette` are
    /// k-means perceptual backends with imagequant-class cost, so they map to
    /// [`Self::Imagequant`] for estimation purposes.
    #[cfg(feature = "zencodec")]
    pub(crate) fn from_backend(backend: crate::quantize::QuantizerBackend) -> Self {
        use crate::quantize::QuantizerBackend;
        match backend {
            QuantizerBackend::Imagequant
            | QuantizerBackend::Zenquant
            | QuantizerBackend::Quantette => Self::Imagequant,
            QuantizerBackend::Quantizr => Self::Quantizr,
            QuantizerBackend::ColorQuant => Self::ColorQuant,
        }
    }

    /// Resolve the resource profile for an [`EncoderConfig`](crate::EncoderConfig),
    /// matching the encoder's own backend-resolution precedence
    /// (explicit `quantizer` → `quantizer_preference` → build default)
    /// without touching the deprecated `quantizer_backend` field.
    #[cfg(feature = "zencodec")]
    pub(crate) fn from_encoder_config(config: &crate::encode::EncoderConfig) -> Self {
        // 1. An explicit cfg-gated `Quantizer` choice wins.
        #[cfg(any(
            feature = "zenquant",
            feature = "quantette",
            feature = "imagequant",
            feature = "quantizr",
            feature = "color_quant"
        ))]
        if let Some(quantizer) = config.quantizer.as_ref() {
            use crate::quantize::Quantizer;
            return match quantizer {
                #[cfg(feature = "zenquant")]
                Quantizer::Zenquant { .. } => Self::Imagequant,
                #[cfg(feature = "quantette")]
                Quantizer::Quantette { .. } => Self::Imagequant,
                #[cfg(feature = "imagequant")]
                Quantizer::Imagequant { .. } => Self::Imagequant,
                #[cfg(feature = "quantizr")]
                Quantizer::Quantizr { .. } => Self::Quantizr,
                #[cfg(feature = "color_quant")]
                Quantizer::ColorQuant { .. } => Self::ColorQuant,
            };
        }

        // 2. A preference series: first backend this build compiled in.
        if let Some(series) = config.quantizer_preference.as_deref()
            && let Some(backend) = crate::quantize::QuantizerBackend::first_available(series)
        {
            return Self::from_backend(backend);
        }

        // 3. Otherwise the build default (auto-selected best available backend).
        Self::from_backend(crate::quantize::QuantizerBackend::default())
    }

    /// Get the quantizer memory bytes per pixel.
    fn bytes_per_pixel(self) -> f64 {
        match self {
            Self::None => NO_QUANT_BYTES_PER_PIXEL,
            Self::Imagequant => IMAGEQUANT_BYTES_PER_PIXEL,
            Self::Quantizr => QUANTIZR_BYTES_PER_PIXEL,
            Self::ColorQuant => COLOR_QUANT_BYTES_PER_PIXEL,
        }
    }

    /// Get the quantizer fixed overhead.
    fn fixed_overhead(self) -> u64 {
        match self {
            Self::None => NO_QUANT_FIXED_OVERHEAD,
            Self::Imagequant => IMAGEQUANT_FIXED_OVERHEAD,
            Self::Quantizr => QUANTIZR_FIXED_OVERHEAD,
            Self::ColorQuant => COLOR_QUANT_FIXED_OVERHEAD,
        }
    }

    /// Get throughput values (max, typical, min) in Mpix/s.
    fn throughputs(self) -> (f64, f64, f64) {
        match self {
            Self::None => (
                NO_QUANT_THROUGHPUT_MAX,
                NO_QUANT_THROUGHPUT_TYP,
                NO_QUANT_THROUGHPUT_MIN,
            ),
            Self::Imagequant => (
                IMAGEQUANT_THROUGHPUT_MAX,
                IMAGEQUANT_THROUGHPUT_TYP,
                IMAGEQUANT_THROUGHPUT_MIN,
            ),
            Self::Quantizr => (
                QUANTIZR_THROUGHPUT_MAX,
                QUANTIZR_THROUGHPUT_TYP,
                QUANTIZR_THROUGHPUT_MIN,
            ),
            Self::ColorQuant => (
                COLOR_QUANT_THROUGHPUT_MAX,
                COLOR_QUANT_THROUGHPUT_TYP,
                COLOR_QUANT_THROUGHPUT_MIN,
            ),
        }
    }
}

/// Resource estimation for decode operations.
///
/// Based on profiling of zengif decode with various content types.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct DecodeEstimate {
    /// Minimum expected peak memory (best case: simple images).
    pub peak_memory_bytes_min: u64,

    /// Typical peak memory in bytes during decoding (natural photos).
    pub peak_memory_bytes: u64,

    /// Maximum expected peak memory (worst case: complex images).
    pub peak_memory_bytes_max: u64,

    /// Estimated heap allocations during decoding.
    /// Fewer allocations = better latency.
    pub allocations: u32,

    /// Decode time in milliseconds (best case: solid color frames).
    pub time_ms_min: f32,

    /// Decode time in milliseconds (typical: real photographs).
    pub time_ms: f32,

    /// Decode time in milliseconds (worst case: noise/high-entropy).
    pub time_ms_max: f32,

    /// Output buffer size in bytes (all frames uncompressed RGBA).
    pub output_bytes: u64,

    /// Canvas size in bytes (width × height × 4).
    pub canvas_bytes: u64,
}

/// Resource estimation for encode operations.
///
/// Based on profiling of zengif encode with various quantizers and content types.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EncodeEstimate {
    /// Minimum expected peak memory (best case: solid color, simple content).
    pub peak_memory_bytes_min: u64,

    /// Typical peak memory in bytes during encoding (natural photos).
    pub peak_memory_bytes: u64,

    /// Maximum expected peak memory (worst case: noise, high-entropy).
    pub peak_memory_bytes_max: u64,

    /// Estimated heap allocations. Fewer allocations = better latency.
    pub allocations: u32,

    /// Encode time in milliseconds (best case: simple content).
    pub time_ms_min: f32,

    /// Encode time in milliseconds (typical: real photographs).
    pub time_ms: f32,

    /// Encode time in milliseconds (worst case: noise/high-entropy).
    pub time_ms_max: f32,

    /// Estimated output size in bytes.
    /// GIF compression varies widely; this is a rough estimate.
    pub output_bytes: u64,

    /// Input size in bytes (all frames uncompressed RGBA).
    pub input_bytes: u64,
}

// =============================================================================
// Estimation functions
// =============================================================================

/// Estimate resources for decoding a GIF animation using `decode_all()`.
///
/// This estimates memory for buffering ALL frames in memory. For streaming
/// decode (one frame at a time), use [`estimate_decode_streaming`].
///
/// # Arguments
///
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `frame_count` - Number of frames (use 1 for static images)
///
/// # Memory Note
///
/// The estimate includes zengif's tracked allocations:
/// - Canvas buffer (width × height × 4)
/// - Indexed pixel buffer (width × height)
/// - Previous frame buffer for disposal (width × height × 4)
/// - Output frames (width × height × 4 × frame_count)
///
/// Does NOT include gif crate's internal buffers.
///
/// # Example
///
/// ```rust
/// use zengif::heuristics::estimate_decode;
///
/// let est = estimate_decode(640, 480, 30);
/// println!("Peak memory: {:.1} MB", est.peak_memory_bytes as f64 / 1_000_000.0);
/// println!("Time: {:.0}ms (typical)", est.time_ms);
/// ```
#[must_use]
pub fn estimate_decode(width: u32, height: u32, frame_count: u32) -> DecodeEstimate {
    let pixels = (width as u64) * (height as u64);
    let total_pixels = pixels * (frame_count as u64);

    // Canvas memory: RGBA (4 bytes) + indexed buffer (1 byte)
    let canvas_bytes = pixels * 4;
    let index_buffer_bytes = pixels;

    // Previous disposal may need a backup buffer (+4 bytes worst case)
    let disposal_buffer = canvas_bytes;

    // decode_all() stores all frames in output
    let output_frames_bytes = total_pixels * 4;

    // Total memory for decode_all:
    // canvas + indexed + disposal backup + all output frames
    let base_memory = DECODE_FIXED_OVERHEAD + canvas_bytes + index_buffer_bytes;
    let peak_with_disposal = base_memory + disposal_buffer;
    let peak_with_all_frames = peak_with_disposal + output_frames_bytes;

    // Content multipliers: GIF decode is relatively stable
    let peak_memory_bytes_min = peak_with_all_frames;
    let peak_memory_bytes = peak_with_all_frames;
    let peak_memory_bytes_max = (peak_with_all_frames as f64 * 1.1) as u64;

    // Time calculation from throughput
    let total_pixels_f = total_pixels as f64;
    let time_ms_min = (total_pixels_f / (DECODE_THROUGHPUT_MAX_MPIXELS * 1000.0)) as f32;
    let time_ms = (total_pixels_f / (DECODE_THROUGHPUT_TYP_MPIXELS * 1000.0)) as f32;
    let time_ms_max = (total_pixels_f / (DECODE_THROUGHPUT_MIN_MPIXELS * 1000.0)) as f32;

    // Output: all frames as RGBA
    let output_bytes = total_pixels * 4;

    // Allocations: initial setup + per-frame
    let allocations = 5 + frame_count * 2;

    DecodeEstimate {
        peak_memory_bytes_min,
        peak_memory_bytes,
        peak_memory_bytes_max,
        allocations,
        time_ms_min,
        time_ms,
        time_ms_max,
        output_bytes,
        canvas_bytes,
    }
}

/// Estimate resources for encoding a GIF animation.
///
/// # Arguments
///
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `frame_count` - Number of frames (use 1 for static images)
/// * `quantizer` - Quantizer type being used
///
/// # Example
///
/// ```rust
/// use zengif::heuristics::{estimate_encode, QuantizerType};
///
/// let est = estimate_encode(640, 480, 30, QuantizerType::Imagequant);
/// println!("Peak memory: {:.1} MB", est.peak_memory_bytes as f64 / 1_000_000.0);
/// println!("Time: {:.0}ms (typical)", est.time_ms);
/// ```
#[must_use]
pub fn estimate_encode(
    width: u32,
    height: u32,
    frame_count: u32,
    quantizer: QuantizerType,
) -> EncodeEstimate {
    let pixels = (width as u64) * (height as u64);
    let total_pixels = pixels * (frame_count as u64);
    let frame_bytes = pixels * 4; // RGBA

    // Base encode memory: previous frame buffer + output buffer
    let base_memory = ENCODE_FIXED_OVERHEAD + (pixels as f64 * ENCODE_BASE_BYTES_PER_PIXEL) as u64;

    // Quantizer memory (per-frame working memory, not cumulative)
    let quant_memory =
        quantizer.fixed_overhead() + (pixels as f64 * quantizer.bytes_per_pixel()) as u64;

    let peak_memory = base_memory + quant_memory;

    // Content multipliers vary by quantizer
    let (min_mult, typ_mult, max_mult) = match quantizer {
        QuantizerType::None => (0.8, 1.0, 1.2),
        QuantizerType::Imagequant => (0.8, 1.3, 1.4), // heaptrack 2026-06-23: typ (est, the gating value) raised to ~1.3× so it clears the measured cell with ~10% margin (was 1.2 = +1.3% only — too tight for content variance); max headroom 1.4
        QuantizerType::Quantizr => (0.9, 1.1, 1.4),
        QuantizerType::ColorQuant => (0.9, 1.1, 1.3),
    };

    let peak_memory_bytes_min = (peak_memory as f64 * min_mult) as u64;
    let peak_memory_bytes = (peak_memory as f64 * typ_mult) as u64;
    let peak_memory_bytes_max = (peak_memory as f64 * max_mult) as u64;

    // Time calculation from throughput
    let total_pixels_f = total_pixels as f64;
    let (throughput_max, throughput_typ, throughput_min) = quantizer.throughputs();

    let time_ms_min = (total_pixels_f / (throughput_max * 1000.0)) as f32;
    let time_ms = (total_pixels_f / (throughput_typ * 1000.0)) as f32;
    let time_ms_max = (total_pixels_f / (throughput_min * 1000.0)) as f32;

    // Output estimate: GIF typically 10-30% of RGBA size for photos
    // Solid colors compress much better, noise worse
    let compression_ratio = 0.15; // 15% of uncompressed
    let output_bytes = (total_pixels as f64 * 4.0 * compression_ratio) as u64;

    // Input: all frames as RGBA
    let input_bytes = frame_bytes * (frame_count as u64);

    // Allocations: setup + per-frame quantization
    let allocations = 10 + frame_count * 5;

    EncodeEstimate {
        peak_memory_bytes_min,
        peak_memory_bytes,
        peak_memory_bytes_max,
        allocations,
        time_ms_min,
        time_ms,
        time_ms_max,
        output_bytes,
        input_bytes,
    }
}

/// Estimate resources for encoding with shared palette mode.
///
/// Shared palette mode buffers all frames before building a global palette,
/// which increases peak memory but improves compression.
///
/// # Arguments
///
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `frame_count` - Number of frames
/// * `quantizer` - Quantizer type being used
///
/// # Example
///
/// ```rust
/// use zengif::heuristics::{estimate_encode_shared_palette, QuantizerType};
///
/// let est = estimate_encode_shared_palette(640, 480, 30, QuantizerType::Imagequant);
/// println!("Peak memory (shared palette): {:.1} MB",
///     est.peak_memory_bytes as f64 / 1_000_000.0);
/// ```
#[must_use]
pub fn estimate_encode_shared_palette(
    width: u32,
    height: u32,
    frame_count: u32,
    quantizer: QuantizerType,
) -> EncodeEstimate {
    let mut est = estimate_encode(width, height, frame_count, quantizer);

    // Shared palette mode buffers all frames in memory
    let frame_bytes = (width as u64) * (height as u64) * 4;
    let buffer_bytes = frame_bytes * (frame_count as u64);

    // Peak includes: buffered frames + quantizer working memory
    est.peak_memory_bytes_min += buffer_bytes;
    est.peak_memory_bytes += buffer_bytes;
    est.peak_memory_bytes_max += buffer_bytes;

    // Slightly more allocations for buffering
    est.allocations += frame_count;

    est
}

/// Estimate resources for a streaming decode (processing one frame at a time).
///
/// This is more memory-efficient than decoding all frames, as only one
/// canvas is kept in memory.
///
/// # Arguments
///
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
///
/// # Example
///
/// ```rust
/// use zengif::heuristics::estimate_decode_streaming;
///
/// let est = estimate_decode_streaming(640, 480);
/// println!("Streaming decode memory: {:.1} MB",
///     est.peak_memory_bytes as f64 / 1_000_000.0);
/// ```
#[must_use]
pub fn estimate_decode_streaming(width: u32, height: u32) -> DecodeEstimate {
    // Streaming decode is same as single-frame decode
    estimate_decode(width, height, 1)
}

/// Estimate resources for a streaming encode (processing one frame at a time).
///
/// This uses less memory than shared palette mode but may produce larger
/// output due to per-frame palettes.
///
/// # Arguments
///
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `quantizer` - Quantizer type being used
///
/// # Example
///
/// ```rust
/// use zengif::heuristics::{estimate_encode_streaming, QuantizerType};
///
/// let est = estimate_encode_streaming(640, 480, QuantizerType::Quantizr);
/// println!("Streaming encode memory: {:.1} MB",
///     est.peak_memory_bytes as f64 / 1_000_000.0);
/// ```
#[must_use]
pub fn estimate_encode_streaming(
    width: u32,
    height: u32,
    quantizer: QuantizerType,
) -> EncodeEstimate {
    // Streaming encode processes one frame at a time
    estimate_encode(width, height, 1, quantizer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_estimate_scales_with_size() {
        let small = estimate_decode(256, 256, 1);
        let large = estimate_decode(512, 512, 1);

        // 4x pixels should give roughly 4x memory
        let ratio = large.peak_memory_bytes as f64 / small.peak_memory_bytes as f64;
        assert!(ratio > 3.0 && ratio < 5.0, "Ratio was {}", ratio);
    }

    #[test]
    fn decode_estimate_scales_with_frames() {
        let single = estimate_decode(256, 256, 1);
        let multi = estimate_decode(256, 256, 10);

        // decode_all() buffers all frames, so memory scales roughly with frame count
        // The output bytes component should dominate
        let ratio = multi.peak_memory_bytes as f64 / single.peak_memory_bytes as f64;

        // With 10 frames vs 1, output_bytes is 10x, but base memory is constant
        // So ratio should be > 3 (accounting for fixed overhead)
        assert!(
            ratio > 3.0,
            "Memory ratio for 10 frames vs 1 should be > 3, was {}. single={}, multi={}",
            ratio,
            single.peak_memory_bytes,
            multi.peak_memory_bytes
        );
        assert!(multi.time_ms > single.time_ms * 5.0);
    }

    #[test]
    fn encode_estimate_quantizer_impact() {
        let none = estimate_encode(512, 512, 1, QuantizerType::None);
        let iq = estimate_encode(512, 512, 1, QuantizerType::Imagequant);
        let qz = estimate_encode(512, 512, 1, QuantizerType::Quantizr);
        let cq = estimate_encode(512, 512, 1, QuantizerType::ColorQuant);

        // imagequant should use more memory than no quantizer
        assert!(iq.peak_memory_bytes > none.peak_memory_bytes);
        // imagequant uses more memory than quantizr
        assert!(iq.peak_memory_bytes > qz.peak_memory_bytes);

        // Throughput comparisons based on profiling measurements:
        // - quantizr: highly variable (1-65 Mpix/s), fastest for simple content
        // - imagequant: moderate (2-20 Mpix/s)
        // - color_quant: consistent but slow (1.5-2.2 Mpix/s)
        // For typical content, quantizr is faster than color_quant
        assert!(
            qz.time_ms < cq.time_ms,
            "quantizr should be faster than color_quant"
        );
    }

    #[test]
    fn shared_palette_uses_more_memory() {
        let streaming = estimate_encode(256, 256, 10, QuantizerType::Imagequant);
        let shared = estimate_encode_shared_palette(256, 256, 10, QuantizerType::Imagequant);

        // Shared palette buffers all frames
        assert!(shared.peak_memory_bytes > streaming.peak_memory_bytes);

        // Memory difference should be roughly the buffered frames
        let frame_bytes = 256 * 256 * 4 * 10; // 10 frames of RGBA
        let diff = shared.peak_memory_bytes - streaming.peak_memory_bytes;
        assert!((diff as i64 - frame_bytes as i64).unsigned_abs() < frame_bytes / 2);
    }

    #[test]
    fn time_ranges_are_ordered() {
        let est = estimate_decode(1024, 1024, 10);
        assert!(est.time_ms_min < est.time_ms);
        assert!(est.time_ms < est.time_ms_max);

        let enc = estimate_encode(1024, 1024, 10, QuantizerType::Imagequant);
        assert!(enc.time_ms_min < enc.time_ms);
        assert!(enc.time_ms < enc.time_ms_max);
    }

    #[test]
    fn memory_ranges_are_ordered() {
        let est = estimate_decode(1024, 1024, 10);
        assert!(est.peak_memory_bytes_min <= est.peak_memory_bytes);
        assert!(est.peak_memory_bytes <= est.peak_memory_bytes_max);

        let enc = estimate_encode(1024, 1024, 10, QuantizerType::Imagequant);
        assert!(enc.peak_memory_bytes_min <= enc.peak_memory_bytes);
        assert!(enc.peak_memory_bytes <= enc.peak_memory_bytes_max);
    }

    #[test]
    fn streaming_estimates() {
        let dec = estimate_decode_streaming(640, 480);
        let enc = estimate_encode_streaming(640, 480, QuantizerType::Quantizr);

        // Streaming should have reasonable estimates for single frame
        assert!(dec.canvas_bytes == 640 * 480 * 4);
        assert!(enc.input_bytes == 640 * 480 * 4);
    }
}
