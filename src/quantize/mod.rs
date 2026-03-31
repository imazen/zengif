//! Quantization abstractions for GIF encoding.
//!
//! This module provides a trait-based abstraction for color quantization,
//! with multiple backend options for different quality/speed/license tradeoffs.
//!
//! # Choosing a Quantizer
//!
//! Use [`Quantizer`] to select and configure your quantization backend:
//!
//!
//! # Frame-Aware Quantization
//!
//! For animations, quantizers can use the previous frame as a "background"
//! to optimize transparency. Pixels that match the background after
//! quantization are made transparent, improving compression.
//!
//! # Frame Sampling
//!
//! For large animations, building a palette from every frame can be slow.
//! The [`QuantizeConfig::max_palette_frames`] option limits how many frames
//! are sampled, using uniform distribution across the animation.

use crate::error::Result;
use crate::types::Rgba;
use enough::Stop;
#[allow(unused_imports)]
use whereat::at;

// Backend implementations
#[cfg(feature = "color_quant")]
mod color_quant_impl;
#[cfg(feature = "imagequant")]
mod imagequant_impl;
#[cfg(feature = "quantette")]
mod quantette_impl;
#[cfg(feature = "quantizr")]
mod quantizr_impl;
#[cfg(feature = "zenquant")]
mod zenquant_impl;

// Re-export backend quantizers
#[cfg(feature = "color_quant")]
pub use color_quant_impl::ColorQuantQuantizer;
#[cfg(feature = "imagequant")]
pub use imagequant_impl::ImagequantQuantizer;
#[cfg(feature = "quantette")]
pub use quantette_impl::QuantetteQuantizer;
#[cfg(feature = "quantizr")]
pub use quantizr_impl::QuantizrQuantizer;
#[cfg(feature = "zenquant")]
pub use zenquant_impl::ZenquantQuantizer;

/// Result of quantizing a single frame.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QuantizedFrame {
    /// The 256-color palette for this frame (RGB, no alpha in palette itself).
    pub palette: Vec<u8>,
    /// Indexed pixels (palette indices).
    pub pixels: Vec<u8>,
    /// Index of the transparent color in the palette, if any.
    pub transparent_index: Option<u8>,
}

/// Configuration for quantization.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QuantizeConfig {
    /// Quality level (1-100). Higher = better quality, slower.
    pub quality: u8,
    /// Dithering level (0.0-1.0). Higher = more dithering.
    pub dithering: f32,
    /// Whether to use the previous frame as background for transparency optimization.
    pub use_background: bool,
    /// Maximum frames to sample for shared palette building.
    /// If None, all frames are used. If Some(n), uniformly samples n frames.
    /// This limits CPU/memory usage for large animations.
    pub max_palette_frames: Option<usize>,
}

impl Default for QuantizeConfig {
    fn default() -> Self {
        Self {
            quality: 80,
            dithering: 0.5,
            use_background: true,
            max_palette_frames: None, // Use all frames by default
        }
    }
}

impl QuantizeConfig {
    /// Configuration optimized for round-trip encoding.
    pub fn for_round_trip() -> Self {
        Self {
            quality: 100,
            dithering: 0.0,
            use_background: true,
            max_palette_frames: None,
        }
    }

    /// Set maximum frames to sample for palette building.
    ///
    /// For large animations, sampling a subset of frames can significantly
    /// reduce palette building time while still producing good results.
    /// Frames are sampled uniformly across the animation.
    ///
    /// Recommended values:
    /// - 16-32 for most animations
    /// - 64+ for animations with many distinct scenes
    /// - None to use all frames (default)
    #[must_use]
    pub fn max_palette_frames(mut self, max: usize) -> Self {
        self.max_palette_frames = Some(max);
        self
    }
}

/// Quantizer selection with backend-specific configuration.
///
/// # Example
///
/// ```rust,ignore
/// use zengif::{EncoderConfig, Quantizer};
///
/// // Best perceptual quality (AGPL)
/// let config = EncoderConfig::new()
///     .quantizer(Quantizer::zenquant());
///
/// // Good quality, fast (MIT)
/// let config = EncoderConfig::new()
///     .quantizer(Quantizer::quantizr());
///
/// // Best compression (GPL)
/// let config = EncoderConfig::new()
///     .quantizer(Quantizer::imagequant());
///
/// // Fastest encoding (MIT)
/// let config = EncoderConfig::new()
///     .quantizer(Quantizer::color_quant());
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
pub enum Quantizer {
    /// zenquant: Best perceptual quality, AGPL-3.0-or-later licensed.
    ///
    /// Produces the best butteraugli and SSIMULACRA2 scores.
    /// AGPL-3.0-or-later licensed.
    #[cfg(feature = "zenquant")]
    Zenquant {
        /// Dithering level (0.0 = none, 1.0 = full).
        /// Default: 0.5
        dithering: f32,
    },

    /// quantette: Oklab k-means, high quality, fast, MIT/Apache-2.0 licensed.
    ///
    /// Uses the Oklab color space for perceptually accurate quantization.
    /// K-means clustering produces high-quality palettes with fast convergence.
    #[cfg(feature = "quantette")]
    Quantette {
        /// Dithering level (0.0 = none, 1.0 = full).
        /// Default: 0.5
        dithering: f32,
    },

    /// Quantizr: Good quality, fast, MIT licensed.
    ///
    /// Best MIT-licensed option with good quality/speed balance.
    #[cfg(feature = "quantizr")]
    Quantizr {
        /// Dithering level (0.0 = none, 1.0 = full).
        ///
        /// Lower values produce smaller files but may show color banding.
        /// Default: 0.5
        dithering: f32,
    },

    /// Imagequant (libimagequant): Best quality AND compression, GPL-3.0-or-later licensed.
    ///
    /// **Recommended** - produces the best quality and smallest files thanks to
    /// LZW-aware dithering that compresses exceptionally well.
    /// **Requires GPL-3.0-or-later compliance** (source disclosure).
    /// Commercial license available at <https://pngquant.org>.
    #[cfg(feature = "imagequant")]
    Imagequant {
        /// Quality level (1-100). Higher = better quality, slower.
        /// Default: 80
        quality: u8,
        /// Dithering level (0.0 = none, 1.0 = full).
        /// Default: 0.5
        dithering: f32,
    },

    /// ColorQuant: Fastest encoder, MIT licensed.
    ///
    /// Uses the NeuQuant neural network algorithm.
    /// Best for high-throughput scenarios where encoding speed matters most.
    #[cfg(feature = "color_quant")]
    ColorQuant {
        /// Sample factor (1 = best quality/slowest, 30 = fastest/lowest quality).
        ///
        /// Controls the fraction of pixels used for learning.
        /// Default: 10
        sample_factor: i32,
    },
}

#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
impl Quantizer {
    /// Create a zenquant quantizer with default settings.
    ///
    /// Best perceptual quality (butteraugli/SSIMULACRA2). AGPL-3.0-or-later.
    #[cfg(feature = "zenquant")]
    #[must_use]
    pub fn zenquant() -> Self {
        Self::Zenquant { dithering: 0.5 }
    }

    /// Create a zenquant quantizer with custom dithering.
    ///
    /// # Arguments
    /// * `dithering` - Dithering level (0.0 = none, 1.0 = full)
    #[cfg(feature = "zenquant")]
    #[must_use]
    pub fn zenquant_with_dithering(dithering: f32) -> Self {
        Self::Zenquant {
            dithering: dithering.clamp(0.0, 1.0),
        }
    }

    /// Create a quantette quantizer with default settings.
    ///
    /// Oklab k-means — high quality, fast, MIT/Apache-2.0 licensed.
    #[cfg(feature = "quantette")]
    #[must_use]
    pub fn quantette() -> Self {
        Self::Quantette { dithering: 0.5 }
    }

    /// Create a quantette quantizer with custom dithering.
    #[cfg(feature = "quantette")]
    #[must_use]
    pub fn quantette_with_dithering(dithering: f32) -> Self {
        Self::Quantette {
            dithering: dithering.clamp(0.0, 1.0),
        }
    }

    /// Create a Quantizr quantizer with default settings.
    ///
    /// Good quality, fast, MIT licensed.
    #[cfg(feature = "quantizr")]
    #[must_use]
    pub fn quantizr() -> Self {
        Self::Quantizr { dithering: 0.5 }
    }

    /// Create a Quantizr quantizer with custom dithering.
    ///
    /// # Arguments
    /// * `dithering` - Dithering level (0.0 = none, 1.0 = full)
    #[cfg(feature = "quantizr")]
    #[must_use]
    pub fn quantizr_with_dithering(dithering: f32) -> Self {
        Self::Quantizr {
            dithering: dithering.clamp(0.0, 1.0),
        }
    }

    /// Create an Imagequant quantizer with default settings.
    ///
    /// **Note**: Imagequant is GPL-3.0-or-later licensed. See <https://pngquant.org> for commercial licensing.
    #[cfg(feature = "imagequant")]
    #[must_use]
    pub fn imagequant() -> Self {
        Self::Imagequant {
            quality: 80,
            dithering: 0.5,
        }
    }

    /// Create an Imagequant quantizer with custom settings.
    ///
    /// # Arguments
    /// * `quality` - Quality level (1-100, higher = better)
    /// * `dithering` - Dithering level (0.0 = none, 1.0 = full)
    #[cfg(feature = "imagequant")]
    #[must_use]
    pub fn imagequant_with_settings(quality: u8, dithering: f32) -> Self {
        Self::Imagequant {
            quality: quality.clamp(1, 100),
            dithering: dithering.clamp(0.0, 1.0),
        }
    }

    /// Create a ColorQuant quantizer with default settings.
    ///
    /// This is the **fastest** quantizer, best for high-throughput scenarios.
    #[cfg(feature = "color_quant")]
    #[must_use]
    pub fn color_quant() -> Self {
        Self::ColorQuant { sample_factor: 10 }
    }

    /// Create a ColorQuant quantizer with custom sample factor.
    ///
    /// # Arguments
    /// * `sample_factor` - Sample factor (1 = best quality, 30 = fastest)
    #[cfg(feature = "color_quant")]
    #[must_use]
    pub fn color_quant_with_sample_factor(sample_factor: i32) -> Self {
        Self::ColorQuant {
            sample_factor: sample_factor.clamp(1, 30),
        }
    }

    /// Create the default quantizer based on available features.
    ///
    /// Priority: zenquant > quantette > imagequant > quantizr > color_quant
    #[must_use]
    #[allow(clippy::needless_return)] // Returns needed for conditional compilation branches
    pub fn auto() -> Self {
        #[cfg(feature = "zenquant")]
        {
            return Self::zenquant();
        }
        #[cfg(all(feature = "quantette", not(feature = "zenquant")))]
        {
            return Self::quantette();
        }
        #[cfg(all(
            feature = "imagequant",
            not(feature = "zenquant"),
            not(feature = "quantette")
        ))]
        {
            return Self::imagequant();
        }
        #[cfg(all(
            feature = "quantizr",
            not(feature = "zenquant"),
            not(feature = "quantette"),
            not(feature = "imagequant")
        ))]
        {
            return Self::quantizr();
        }
        #[cfg(all(
            feature = "color_quant",
            not(feature = "zenquant"),
            not(feature = "quantette"),
            not(feature = "imagequant"),
            not(feature = "quantizr")
        ))]
        {
            return Self::color_quant();
        }
    }

    /// Create the backend quantizer instance.
    pub(crate) fn create_backend(&self) -> Box<dyn QuantizerTrait> {
        match self {
            #[cfg(feature = "zenquant")]
            Self::Zenquant { .. } => Box::new(ZenquantQuantizer::new()),
            #[cfg(feature = "quantette")]
            Self::Quantette { .. } => Box::new(QuantetteQuantizer::new()),
            #[cfg(feature = "quantizr")]
            Self::Quantizr { .. } => Box::new(QuantizrQuantizer::new()),
            #[cfg(feature = "imagequant")]
            Self::Imagequant { .. } => Box::new(ImagequantQuantizer::new()),
            #[cfg(feature = "color_quant")]
            Self::ColorQuant { .. } => Box::new(ColorQuantQuantizer::new()),
        }
    }
}

#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
impl Default for Quantizer {
    fn default() -> Self {
        Self::auto()
    }
}

/// Compute indices of frames to sample for palette building.
///
/// Returns indices uniformly distributed across the frame range.
/// Always includes first and last frame if max_samples >= 2.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
pub(crate) fn compute_sample_indices(
    total_frames: usize,
    max_samples: Option<usize>,
) -> Vec<usize> {
    let max = match max_samples {
        None => return (0..total_frames).collect(),
        Some(m) if m >= total_frames => return (0..total_frames).collect(),
        Some(0) => return vec![],
        Some(m) => m,
    };

    if max == 1 {
        return vec![0];
    }

    // Uniform sampling including first and last
    let mut indices = Vec::with_capacity(max);
    for i in 0..max {
        let idx = if max == 1 {
            0
        } else {
            i * (total_frames - 1) / (max - 1)
        };
        indices.push(idx);
    }

    // Remove duplicates (can happen with very few frames)
    indices.dedup();
    indices
}

/// Trait for color quantization implementations.
///
/// Implementations can be stateful to support features like:
/// - Shared palettes across frames
/// - Frame-to-frame background optimization
/// - Histogram accumulation
///
/// # Implementing
///
/// Only [`quantize_frame`](QuantizerTrait::quantize_frame) is required.
/// The remaining methods have default implementations that either return
/// an error (shared palette methods) or do nothing (`reset`). Override
/// them to support shared-palette animation workflows.
///
/// New methods may be added to this trait in minor versions — they will
/// always have default implementations, so existing implementations
/// will continue to compile.
pub trait QuantizerTrait: Send {
    /// Quantize a single frame.
    ///
    /// # Arguments
    /// * `pixels` - RGBA pixels to quantize
    /// * `width` - Frame width
    /// * `height` - Frame height
    /// * `background` - Optional previous frame for transparency optimization
    /// * `config` - Quantization settings
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame>;

    /// Build a shared palette from multiple frames.
    ///
    /// Call this before `quantize_frame_with_palette` to compute
    /// an optimal palette across all frames.
    ///
    /// # Arguments
    /// * `frames` - All frames to consider for palette building
    /// * `width` - Frame width
    /// * `height` - Frame height
    /// * `config` - Quantization settings (including max_palette_frames for sampling)
    /// * `stop` - Cancellation token
    ///
    /// If `config.max_palette_frames` is set, only a sample of frames
    /// will be used for histogram building (uniformly distributed).
    ///
    /// The default implementation returns an error. Override this to
    /// support shared-palette workflows.
    fn build_shared_palette(
        &mut self,
        _frames: &[&[Rgba]],
        _width: u16,
        _height: u16,
        _config: &QuantizeConfig,
        _stop: &dyn Stop,
    ) -> Result<Vec<u8>> {
        Err(at!(crate::error::GifError::QuantizationFailed {
            message: "shared palettes not supported by this quantizer",
        }))
    }

    /// Quantize a frame using a pre-computed shared palette.
    ///
    /// # Arguments
    /// * `pixels` - RGBA pixels to quantize
    /// * `width` - Frame width
    /// * `height` - Frame height
    /// * `background` - Optional previous frame for transparency optimization
    /// * `config` - Quantization settings
    ///
    /// The default implementation ignores the shared palette and falls
    /// back to [`quantize_frame`](QuantizerTrait::quantize_frame).
    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        self.quantize_frame(pixels, width, height, background, config)
    }

    /// Reset any accumulated state (e.g., shared palette).
    ///
    /// The default implementation does nothing.
    fn reset(&mut self) {}
}

/// Available quantizer backends.
///
/// Used to select which quantizer implementation to use at runtime.
///
/// The `Default` impl picks the best *available* backend at compile time:
/// zenquant > imagequant > quantizr > color_quant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuantizerBackend {
    /// Use zenquant (best perceptual quality, AGPL-3.0-or-later).
    /// Requires `zenquant` feature.
    Zenquant,
    /// Use quantette (Oklab k-means, high quality, MIT/Apache-2.0).
    /// Requires `quantette` feature.
    Quantette,
    /// Use imagequant (good quality, best compression, GPL licensed).
    /// Requires `imagequant` feature.
    Imagequant,
    /// Use quantizr (fast, MIT licensed).
    /// Requires `quantizr` feature.
    Quantizr,
    /// Use color_quant (NEUQUANT algorithm, MIT licensed).
    /// Requires `color_quant` feature.
    ColorQuant,
}

impl Default for QuantizerBackend {
    #[allow(unreachable_code, clippy::needless_return)]
    fn default() -> Self {
        #[cfg(feature = "zenquant")]
        {
            return Self::Zenquant;
        }
        #[cfg(feature = "quantette")]
        {
            return Self::Quantette;
        }
        #[cfg(feature = "imagequant")]
        {
            return Self::Imagequant;
        }
        #[cfg(feature = "quantizr")]
        {
            return Self::Quantizr;
        }
        #[cfg(feature = "color_quant")]
        {
            return Self::ColorQuant;
        }
        #[cfg(not(any(
            feature = "zenquant",
            feature = "quantette",
            feature = "imagequant",
            feature = "quantizr",
            feature = "color_quant"
        )))]
        {
            // No quantizer backend enabled — pick an arbitrary variant.
            // `create_quantizer()` will return None at runtime.
            Self::Quantizr
        }
    }
}

impl QuantizerBackend {
    /// Create a boxed quantizer for this backend.
    ///
    /// Returns `None` if the required feature is not enabled.
    #[must_use]
    pub fn create_quantizer(&self) -> Option<Box<dyn QuantizerTrait>> {
        match self {
            #[cfg(feature = "zenquant")]
            QuantizerBackend::Zenquant => Some(Box::new(ZenquantQuantizer::new())),
            #[cfg(not(feature = "zenquant"))]
            QuantizerBackend::Zenquant => None,

            #[cfg(feature = "quantette")]
            QuantizerBackend::Quantette => Some(Box::new(QuantetteQuantizer::new())),
            #[cfg(not(feature = "quantette"))]
            QuantizerBackend::Quantette => None,

            #[cfg(feature = "imagequant")]
            QuantizerBackend::Imagequant => Some(Box::new(ImagequantQuantizer::new())),
            #[cfg(not(feature = "imagequant"))]
            QuantizerBackend::Imagequant => None,

            #[cfg(feature = "quantizr")]
            QuantizerBackend::Quantizr => Some(Box::new(QuantizrQuantizer::new())),
            #[cfg(not(feature = "quantizr"))]
            QuantizerBackend::Quantizr => None,

            #[cfg(feature = "color_quant")]
            QuantizerBackend::ColorQuant => Some(Box::new(ColorQuantQuantizer::new())),
            #[cfg(not(feature = "color_quant"))]
            QuantizerBackend::ColorQuant => None,
        }
    }

    /// Check if the required feature for this backend is enabled.
    #[must_use]
    pub fn is_available(&self) -> bool {
        match self {
            QuantizerBackend::Zenquant => cfg!(feature = "zenquant"),
            QuantizerBackend::Quantette => cfg!(feature = "quantette"),
            QuantizerBackend::Imagequant => cfg!(feature = "imagequant"),
            QuantizerBackend::Quantizr => cfg!(feature = "quantizr"),
            QuantizerBackend::ColorQuant => cfg!(feature = "color_quant"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_sample_indices_all_frames() {
        // None means use all frames
        assert_eq!(
            compute_sample_indices(10, None),
            (0..10).collect::<Vec<_>>()
        );
        assert_eq!(compute_sample_indices(3, None), vec![0, 1, 2]);
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_sample_indices_more_than_total() {
        // max_samples >= total returns all frames
        assert_eq!(compute_sample_indices(5, Some(5)), vec![0, 1, 2, 3, 4]);
        assert_eq!(compute_sample_indices(5, Some(10)), vec![0, 1, 2, 3, 4]);
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_sample_indices_uniform_distribution() {
        // Sample 3 from 10: should be 0, 4 or 5, 9
        let indices = compute_sample_indices(10, Some(3));
        assert_eq!(indices.len(), 3);
        assert_eq!(indices[0], 0); // Always includes first
        assert_eq!(indices[2], 9); // Always includes last
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn compute_sample_indices_edge_cases() {
        // Zero samples
        assert_eq!(compute_sample_indices(10, Some(0)), Vec::<usize>::new());

        // One sample
        assert_eq!(compute_sample_indices(10, Some(1)), vec![0]);

        // Two samples: first and last
        assert_eq!(compute_sample_indices(10, Some(2)), vec![0, 9]);

        // Single frame animation
        assert_eq!(compute_sample_indices(1, Some(5)), vec![0]);
        assert_eq!(compute_sample_indices(1, None), vec![0]);
    }
}
