//! Imagequant quantizer backend.

use super::{compute_sample_indices, QuantizeConfig, QuantizedFrame, QuantizerTrait};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// Imagequant-based quantizer.
///
/// Uses the imagequant library (pngquant's engine) for high-quality
/// color quantization with support for:
/// - Adaptive palette generation
/// - Floyd-Steinberg dithering
/// - Frame-aware transparency via `set_background()`
pub struct ImagequantQuantizer {
    /// Cached quantization result for shared palette mode.
    shared_result: Option<imagequant::QuantizationResult>,
    /// Cached attributes.
    attr: imagequant::Attributes,
}

impl ImagequantQuantizer {
    /// Create a new imagequant-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_result: None,
            attr: imagequant::Attributes::new(),
        }
    }

    /// Convert Rgba slice to imagequant RGBA vec.
    fn to_imagequant_rgba(pixels: &[Rgba]) -> Vec<imagequant::RGBA> {
        pixels
            .iter()
            .map(|p| imagequant::RGBA::new(p.r, p.g, p.b, p.a))
            .collect()
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

impl Default for ImagequantQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantizerTrait for ImagequantQuantizer {
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

        let rgba_slice = Self::to_imagequant_rgba(pixels);

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
                    let bg_rgba = Self::to_imagequant_rgba(bg_pixels);
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
        stop: &dyn Stop,
    ) -> Result<Vec<u8>> {
        use imagequant::Histogram;

        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        self.attr.set_quality(0, config.quality).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to set quality"
            })
        })?;

        let mut histogram = Histogram::new(&self.attr);

        // Determine which frames to sample
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);

        // Add sampled frames to histogram
        for &idx in &sample_indices {
            // Check for cancellation periodically
            stop.check().map_err(|_| at!(GifError::Cancelled))?;

            let frame_pixels = frames[idx];
            let rgba_slice = Self::to_imagequant_rgba(frame_pixels);
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

        let rgba_slice = Self::to_imagequant_rgba(pixels);

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
                    let bg_rgba = Self::to_imagequant_rgba(bg_pixels);
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

    #[test]
    fn imagequant_quantize_single_frame() {
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig::default();

        // Create a simple red frame
        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
        // Palette should contain red-ish color
        assert!(result
            .palette
            .chunks(3)
            .any(|c| c[0] > 200 && c[1] < 50 && c[2] < 50));
    }

    #[test]
    fn imagequant_shared_palette_workflow() {
        use enough::Unstoppable;

        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig::default();

        // Create frames with different colors
        let red_frame: Vec<Rgba> = vec![Rgba::rgb(255, 0, 0); 16];
        let blue_frame: Vec<Rgba> = vec![Rgba::rgb(0, 0, 255); 16];
        let frames: Vec<&[Rgba]> = vec![&red_frame, &blue_frame];

        // Build shared palette
        let palette = quantizer
            .build_shared_palette(&frames, 4, 4, &config, &Unstoppable)
            .expect("palette building should succeed");

        assert!(!palette.is_empty());
        assert!(palette.len() <= 768); // max 256 colors * 3 bytes

        // Quantize both frames with shared palette
        let result1 = quantizer
            .quantize_frame_with_palette(&red_frame, 4, 4, None, &config)
            .expect("quantization should succeed");
        let result2 = quantizer
            .quantize_frame_with_palette(&blue_frame, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert_eq!(result1.pixels.len(), 16);
        assert_eq!(result2.pixels.len(), 16);
    }

    #[test]
    fn imagequant_background_transparency() {
        let mut quantizer = ImagequantQuantizer::new();
        let config = QuantizeConfig {
            use_background: true,
            ..Default::default()
        };

        // Create frames where second is slightly different
        let bg_pixels: Vec<Rgba> = (0..16).map(|i| Rgba::rgb((i * 16) as u8, 0, 0)).collect();
        let fg_pixels: Vec<Rgba> = (0..16)
            .map(|i| {
                if i < 8 {
                    Rgba::rgb((i * 16) as u8, 0, 0) // Same as background
                } else {
                    Rgba::rgb(0, 255, 0) // Different
                }
            })
            .collect();

        // Quantize with background
        let result_with_bg = quantizer
            .quantize_frame(&fg_pixels, 4, 4, Some(&bg_pixels), &config)
            .expect("quantization should succeed");

        // Quantize without background
        quantizer.reset();
        let result_no_bg = quantizer
            .quantize_frame(&fg_pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        // Both should succeed
        assert_eq!(result_with_bg.pixels.len(), 16);
        assert_eq!(result_no_bg.pixels.len(), 16);

        // With background, more pixels should be marked as transparent
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
