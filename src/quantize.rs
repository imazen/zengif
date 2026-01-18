//! Quantization abstractions for GIF encoding.
//!
//! This module provides a trait-based abstraction for color quantization,
//! allowing different quantization backends (imagequant, custom, etc.).
//!
//! # Frame-Aware Quantization
//!
//! For animations, quantizers can use the previous frame as a "background"
//! to optimize transparency. Pixels that match the background after
//! quantization are made transparent, improving compression.

use crate::error::{GifError, Result};
use crate::types::Rgba;
use whereat::at;

/// Result of quantizing a single frame.
#[derive(Debug, Clone)]
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
pub struct QuantizeConfig {
    /// Quality level (1-100). Higher = better quality, slower.
    pub quality: u8,
    /// Dithering level (0.0-1.0). Higher = more dithering.
    pub dithering: f32,
    /// Whether to use the previous frame as background for transparency optimization.
    pub use_background: bool,
}

impl Default for QuantizeConfig {
    fn default() -> Self {
        Self {
            quality: 80,
            dithering: 0.5,
            use_background: true,
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
        }
    }
}

/// Trait for color quantization implementations.
///
/// Implementations can be stateful to support features like:
/// - Shared palettes across frames
/// - Frame-to-frame background optimization
/// - Histogram accumulation
pub trait Quantizer: Send {
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
    fn build_shared_palette(
        &mut self,
        frames: &[&[Rgba]],
        width: u16,
        height: u16,
        config: &QuantizeConfig,
    ) -> Result<Vec<u8>>;

    /// Quantize a frame using a pre-computed shared palette.
    ///
    /// # Arguments
    /// * `pixels` - RGBA pixels to quantize
    /// * `width` - Frame width
    /// * `height` - Frame height
    /// * `background` - Optional previous frame for transparency optimization
    /// * `config` - Quantization settings
    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame>;

    /// Reset any accumulated state (e.g., shared palette).
    fn reset(&mut self);
}

/// Imagequant-based quantizer.
///
/// Uses the imagequant library (pngquant's engine) for high-quality
/// color quantization with support for:
/// - Adaptive palette generation
/// - Floyd-Steinberg dithering
/// - Frame-aware transparency via `set_background()`
#[cfg(feature = "quantize")]
pub struct ImagequantQuantizer {
    /// Cached quantization result for shared palette mode.
    shared_result: Option<imagequant::QuantizationResult>,
    /// Cached attributes.
    attr: imagequant::Attributes,
}

#[cfg(feature = "quantize")]
impl ImagequantQuantizer {
    /// Create a new imagequant-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_result: None,
            attr: imagequant::Attributes::new(),
        }
    }

    /// Convert Rgba slice to imagequant RGBA slice (zero-copy).
    fn as_imagequant_rgba(pixels: &[Rgba]) -> &[imagequant::RGBA] {
        // SAFETY: Rgba and imagequant::RGBA have identical memory layout (4 bytes RGBA)
        unsafe {
            std::slice::from_raw_parts(pixels.as_ptr() as *const imagequant::RGBA, pixels.len())
        }
    }

    /// Find the most transparent color index in a palette.
    fn find_transparent_index(palette: &[imagequant::RGBA]) -> Option<u8> {
        palette
            .iter()
            .enumerate()
            .filter(|(_, c)| c.a < 128)
            .max_by_key(|(_, c)| 255 - c.a)
            .map(|(i, _)| i as u8)
    }

    /// Convert imagequant palette to GIF palette bytes (RGB only).
    fn palette_to_bytes(palette: &[imagequant::RGBA]) -> Vec<u8> {
        palette.iter().flat_map(|c| [c.r, c.g, c.b]).collect()
    }
}

#[cfg(feature = "quantize")]
impl Default for ImagequantQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "quantize")]
impl Quantizer for ImagequantQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        self.attr.set_quality(0, config.quality).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to set quality"
            })
        })?;

        let rgba_slice = Self::as_imagequant_rgba(pixels);

        let mut img = self
            .attr
            .new_image(rgba_slice, width as usize, height as usize, 0.0)
            .map_err(|_| {
                at!(GifError::QuantizationFailed {
                    message: "failed to create image"
                })
            })?;

        // Set background for frame-aware transparency optimization
        if config.use_background {
            if let Some(bg_pixels) = background {
                if bg_pixels.len() == pixels.len() {
                    let bg_rgba = Self::as_imagequant_rgba(bg_pixels);
                    let bg_img = self
                        .attr
                        .new_image(bg_rgba, width as usize, height as usize, 0.0)
                        .map_err(|_| {
                            at!(GifError::QuantizationFailed {
                                message: "failed to create background image"
                            })
                        })?;

                    // set_background tells imagequant to make matching pixels transparent
                    img.set_background(bg_img).map_err(|_| {
                        at!(GifError::QuantizationFailed {
                            message: "failed to set background"
                        })
                    })?;
                }
            }
        }

        // Quantize
        let mut result = self.attr.quantize(&mut img).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "quantization failed"
            })
        })?;

        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to set dithering"
            })
        })?;

        // Remap
        let (palette, indexed) = result.remapped(&mut img).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "remapping failed"
            })
        })?;

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(&palette),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(&palette),
        })
    }

    fn build_shared_palette(
        &mut self,
        frames: &[&[Rgba]],
        width: u16,
        height: u16,
        config: &QuantizeConfig,
    ) -> Result<Vec<u8>> {
        use imagequant::Histogram;

        self.attr.set_quality(0, config.quality).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to set quality"
            })
        })?;

        let mut histogram = Histogram::new(&self.attr);

        // Add all frames to histogram
        for frame_pixels in frames {
            let rgba_slice = Self::as_imagequant_rgba(frame_pixels);
            let mut img = self
                .attr
                .new_image(rgba_slice, width as usize, height as usize, 0.0)
                .map_err(|_| {
                    at!(GifError::QuantizationFailed {
                        message: "failed to create image for histogram"
                    })
                })?;

            histogram.add_image(&self.attr, &mut img).map_err(|_| {
                at!(GifError::QuantizationFailed {
                    message: "failed to add image to histogram"
                })
            })?;
        }

        // Quantize histogram to get shared palette
        let mut result = histogram.quantize(&self.attr).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "histogram quantization failed"
            })
        })?;

        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to set dithering"
            })
        })?;

        // Cache the result for subsequent frame remapping
        let palette = result.palette();
        let palette_bytes = Self::palette_to_bytes(palette);

        self.shared_result = Some(result);

        Ok(palette_bytes)
    }

    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        let result = self.shared_result.as_mut().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;

        let rgba_slice = Self::as_imagequant_rgba(pixels);

        let mut img = self
            .attr
            .new_image(rgba_slice, width as usize, height as usize, 0.0)
            .map_err(|_| {
                at!(GifError::QuantizationFailed {
                    message: "failed to create image"
                })
            })?;

        // Set background for frame-aware transparency
        if config.use_background {
            if let Some(bg_pixels) = background {
                if bg_pixels.len() == pixels.len() {
                    let bg_rgba = Self::as_imagequant_rgba(bg_pixels);
                    let bg_img = self
                        .attr
                        .new_image(bg_rgba, width as usize, height as usize, 0.0)
                        .map_err(|_| {
                            at!(GifError::QuantizationFailed {
                                message: "failed to create background image"
                            })
                        })?;

                    img.set_background(bg_img).map_err(|_| {
                        at!(GifError::QuantizationFailed {
                            message: "failed to set background"
                        })
                    })?;
                }
            }
        }

        // Remap using shared palette
        let (palette, indexed) = result.remapped(&mut img).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "remapping failed"
            })
        })?;

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(&palette),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(&palette),
        })
    }

    fn reset(&mut self) {
        self.shared_result = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "quantize")]
    #[test]
    fn imagequant_quantize_single_frame() {
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig::default();

        // Create a simple red frame
        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .unwrap();

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
    }

    #[cfg(feature = "quantize")]
    #[test]
    fn imagequant_shared_palette() {
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig::default();

        // Create frames with different colors
        let frame1 = vec![Rgba::rgb(255, 0, 0); 16];
        let frame2 = vec![Rgba::rgb(0, 255, 0); 16];
        let frame3 = vec![Rgba::rgb(0, 0, 255); 16];

        let frames: Vec<&[Rgba]> = vec![&frame1, &frame2, &frame3];
        let palette = quantizer
            .build_shared_palette(&frames, 4, 4, &config)
            .unwrap();

        assert!(!palette.is_empty());

        // Quantize each frame with the shared palette
        let result1 = quantizer
            .quantize_frame_with_palette(&frame1, 4, 4, None, &config)
            .unwrap();
        let result2 = quantizer
            .quantize_frame_with_palette(&frame2, 4, 4, Some(&frame1), &config)
            .unwrap();
        let result3 = quantizer
            .quantize_frame_with_palette(&frame3, 4, 4, Some(&frame2), &config)
            .unwrap();

        // All should use the same palette (global palette mode)
        assert_eq!(result1.palette, result2.palette);
        assert_eq!(result2.palette, result3.palette);
    }

    #[cfg(feature = "quantize")]
    #[test]
    fn imagequant_background_optimization() {
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig {
            dithering: 0.0, // No dithering for predictable results
            ..Default::default()
        };

        // Create frames where second has some transparent pixels
        // (imagequant needs transparent pixels in input to have transparent in palette)
        let frame1 = vec![Rgba::rgb(255, 0, 0); 16];

        // Second frame: mostly same, but some pixels are transparent
        let mut frame2 = vec![Rgba::rgb(255, 0, 0); 16];
        frame2[0] = Rgba::TRANSPARENT;
        frame2[5] = Rgba::TRANSPARENT;

        // Quantize first frame
        let _result1 = quantizer
            .quantize_frame(&frame1, 4, 4, None, &config)
            .unwrap();

        // Quantize second frame with first as background
        let result2 = quantizer
            .quantize_frame(&frame2, 4, 4, Some(&frame1), &config)
            .unwrap();

        // Frame with transparent pixels should have a transparent index
        assert!(
            result2.transparent_index.is_some(),
            "Frame with transparent pixels should have transparent index"
        );
    }

    #[cfg(feature = "quantize")]
    #[test]
    fn imagequant_set_background_reduces_output() {
        // Test that set_background produces more transparent pixels for identical regions
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig {
            dithering: 0.0,
            use_background: true,
            ..Default::default()
        };
        let config_no_bg = QuantizeConfig {
            dithering: 0.0,
            use_background: false,
            ..Default::default()
        };

        // Create frames with partially identical content
        let frame1: Vec<Rgba> = (0..64)
            .map(|i| Rgba::rgb(i as u8 * 4, 0, 0))
            .collect();
        let mut frame2 = frame1.clone();
        // Change only a few pixels
        frame2[0] = Rgba::rgb(0, 255, 0);
        frame2[1] = Rgba::TRANSPARENT;

        // Quantize with background (uses set_background)
        let result_with_bg = quantizer
            .quantize_frame(&frame2, 8, 8, Some(&frame1), &config)
            .unwrap();

        // Quantize without background
        quantizer.reset();
        let result_no_bg = quantizer
            .quantize_frame(&frame2, 8, 8, Some(&frame1), &config_no_bg)
            .unwrap();

        // With set_background, more pixels should be transparent (matching background)
        let transparent_with_bg = result_with_bg
            .transparent_index
            .map(|ti| result_with_bg.pixels.iter().filter(|&&p| p == ti).count())
            .unwrap_or(0);
        let transparent_no_bg = result_no_bg
            .transparent_index
            .map(|ti| result_no_bg.pixels.iter().filter(|&&p| p == ti).count())
            .unwrap_or(0);

        // set_background should produce at least as many transparent pixels
        assert!(
            transparent_with_bg >= transparent_no_bg,
            "set_background should make matching pixels transparent: {} vs {}",
            transparent_with_bg,
            transparent_no_bg
        );
    }
}
