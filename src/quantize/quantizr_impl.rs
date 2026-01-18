//! Quantizr quantizer backend.

use super::{compute_sample_indices, QuantizeConfig, QuantizedFrame, Quantizer};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// Quantizr-based quantizer.
///
/// Uses the quantizr library for fast color quantization.
/// MIT licensed, good balance of speed and quality.
pub struct QuantizrQuantizer {
    /// Cached palette for shared palette mode.
    shared_palette: Option<Vec<u8>>,
}

impl QuantizrQuantizer {
    /// Create a new quantizr-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_palette: None,
        }
    }

    /// Find the most transparent color index in a palette.
    fn find_transparent_index(palette: &quantizr::Palette) -> Option<u8> {
        let colors = &palette.entries[..palette.count as usize];
        colors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.a < 128)
            .max_by_key(|(_, c)| 255 - c.a)
            .map(|(i, _)| i as u8)
    }

    /// Convert quantizr palette to GIF palette bytes (RGB only).
    fn palette_to_bytes(palette: &quantizr::Palette) -> Vec<u8> {
        let colors = &palette.entries[..palette.count as usize];
        colors.iter().flat_map(|c| [c.r, c.g, c.b]).collect()
    }
}

impl Default for QuantizrQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Quantizer for QuantizrQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        use quantizr::{Image, Options, QuantizeResult};

        // Convert pixels to bytes
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        let image = Image::new(&pixel_bytes, width as usize, height as usize).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to create quantizr image"
            })
        })?;

        let mut options = Options::default();
        options.set_max_colors(256).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid max colors"
            })
        })?;

        let mut result = QuantizeResult::quantize(&image, &options);

        // Apply dithering level from config (0.0 = none, 1.0 = full)
        // Lower dithering = smaller files but potentially more banding
        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid dithering level"
            })
        })?;

        let mut indexed = vec![0u8; width as usize * height as usize];
        result.remap_image(&image, &mut indexed).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "remapping failed"
            })
        })?;

        let palette = result.get_palette();

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(palette),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(palette),
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
        use quantizr::{Histogram, Image, Options, QuantizeResult};

        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        let mut histogram = Histogram::new();
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);

        for &idx in &sample_indices {
            stop.check().map_err(|_| at!(GifError::Cancelled))?;

            let frame_pixels = frames[idx];
            let pixel_bytes: Vec<u8> = frame_pixels
                .iter()
                .flat_map(|p| [p.r, p.g, p.b, p.a])
                .collect();
            let image =
                Image::new(&pixel_bytes, width as usize, height as usize).map_err(|_| {
                    at!(GifError::QuantizationFailed {
                        message: "failed to create quantizr image for histogram"
                    })
                })?;

            histogram.add_image(&image);
        }

        let mut options = Options::default();
        options.set_max_colors(256).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid max colors"
            })
        })?;

        let result = QuantizeResult::quantize_histogram(&histogram, &options);

        let palette = result.get_palette();
        let palette_bytes = Self::palette_to_bytes(palette);

        self.shared_palette = Some(palette_bytes.clone());

        Ok(palette_bytes)
    }

    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        use quantizr::{Image, Options, QuantizeResult};

        let palette_bytes = self.shared_palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;

        // Convert pixels to bytes
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        let image = Image::new(&pixel_bytes, width as usize, height as usize).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to create quantizr image"
            })
        })?;

        let mut options = Options::default();
        options.set_max_colors(256).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid max colors"
            })
        })?;

        // Re-quantize but the palette should be similar due to histogram
        let mut result = QuantizeResult::quantize(&image, &options);

        // Apply dithering level from config
        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid dithering level"
            })
        })?;

        let mut indexed = vec![0u8; width as usize * height as usize];
        result.remap_image(&image, &mut indexed).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "remapping failed"
            })
        })?;

        let palette = result.get_palette();

        Ok(QuantizedFrame {
            palette: palette_bytes.clone(),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(palette),
        })
    }

    fn reset(&mut self) {
        self.shared_palette = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizr_quantize_single_frame() {
        let mut quantizer = QuantizrQuantizer::new();
        let config = QuantizeConfig::default();

        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
    }
}
