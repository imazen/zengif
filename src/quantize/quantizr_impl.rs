//! Quantizr quantizer backend.

use super::{compute_sample_indices, QuantizeConfig, QuantizedFrame, QuantizerTrait};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// Quantizr-based quantizer.
///
/// Uses the quantizr library for fast color quantization.
/// MIT licensed, good balance of speed and quality.
pub struct QuantizrQuantizer {
    /// Cached palette bytes for shared palette mode.
    shared_palette: Option<Vec<u8>>,
    /// Cached QuantizeResult for reuse in shared palette mode.
    /// Contains the VP-tree for nearest-color search.
    /// Reusing this across frames avoids re-quantizing each frame.
    cached_result: Option<quantizr::QuantizeResult>,
}

impl QuantizrQuantizer {
    /// Create a new quantizr-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_palette: None,
            cached_result: None,
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

impl QuantizerTrait for QuantizrQuantizer {
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

        let mut result = QuantizeResult::quantize_histogram(&histogram, &options);

        // Set dithering level so cached result is ready for remap calls.
        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid dithering level"
            })
        })?;

        let palette = result.get_palette();
        let palette_bytes = Self::palette_to_bytes(palette);

        self.shared_palette = Some(palette_bytes.clone());
        self.cached_result = Some(result);

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
        use quantizr::Image;

        let palette_bytes = self.shared_palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;
        let palette_bytes = palette_bytes.clone();

        let result = self.cached_result.as_mut().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no cached quantize result - call build_shared_palette first"
            })
        })?;

        // Update dithering level if it changed since palette was built.
        result.set_dithering_level(config.dithering).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "invalid dithering level"
            })
        })?;

        // Convert pixels to bytes
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        let image = Image::new(&pixel_bytes, width as usize, height as usize).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "failed to create quantizr image"
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
            palette: palette_bytes,
            pixels: indexed,
            transparent_index: Self::find_transparent_index(palette),
        })
    }

    fn reset(&mut self) {
        self.shared_palette = None;
        self.cached_result = None;
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

    #[test]
    fn quantizr_shared_palette_reuses_result() {
        let mut quantizer = QuantizrQuantizer::new();
        let config = QuantizeConfig {
            dithering: 0.0,
            ..QuantizeConfig::default()
        };
        let stop = enough::Unstoppable;

        // Two distinct frames: red and blue
        let red_frame = vec![Rgba::rgb(255, 0, 0); 16];
        let blue_frame = vec![Rgba::rgb(0, 0, 255); 16];
        let frames: Vec<&[Rgba]> = vec![&red_frame, &blue_frame];

        let palette_bytes = quantizer
            .build_shared_palette(&frames, 4, 4, &config, &stop)
            .expect("palette should build");

        // Verify the cached result exists
        assert!(quantizer.cached_result.is_some());

        // Remap both frames using cached result
        let red_result = quantizer
            .quantize_frame_with_palette(&red_frame, 4, 4, None, &config)
            .expect("remap red should work");
        let blue_result = quantizer
            .quantize_frame_with_palette(&blue_frame, 4, 4, None, &config)
            .expect("remap blue should work");

        // Both frames should use the same shared palette
        assert_eq!(red_result.palette, palette_bytes);
        assert_eq!(blue_result.palette, palette_bytes);

        // All pixels in each frame should map to the same index (uniform color)
        assert!(red_result.pixels.iter().all(|&p| p == red_result.pixels[0]));
        assert!(blue_result.pixels.iter().all(|&p| p == blue_result.pixels[0]));

        // Red and blue should map to different palette entries
        assert_ne!(red_result.pixels[0], blue_result.pixels[0]);

        // Verify the indices point to correct colors in the palette
        let red_idx = red_result.pixels[0] as usize;
        let blue_idx = blue_result.pixels[0] as usize;
        let red_color = &palette_bytes[red_idx * 3..red_idx * 3 + 3];
        let blue_color = &palette_bytes[blue_idx * 3..blue_idx * 3 + 3];

        // Red entry should be mostly red
        assert!(red_color[0] > 200 && red_color[1] < 50 && red_color[2] < 50,
            "expected red, got RGB({}, {}, {})", red_color[0], red_color[1], red_color[2]);
        // Blue entry should be mostly blue
        assert!(blue_color[0] < 50 && blue_color[1] < 50 && blue_color[2] > 200,
            "expected blue, got RGB({}, {}, {})", blue_color[0], blue_color[1], blue_color[2]);
    }

    #[test]
    fn quantizr_reset_clears_cached_result() {
        let mut quantizer = QuantizrQuantizer::new();
        let config = QuantizeConfig::default();
        let stop = enough::Unstoppable;

        let pixels = vec![Rgba::rgb(128, 128, 128); 16];
        let frames: Vec<&[Rgba]> = vec![&pixels];

        quantizer
            .build_shared_palette(&frames, 4, 4, &config, &stop)
            .expect("palette should build");
        assert!(quantizer.cached_result.is_some());
        assert!(quantizer.shared_palette.is_some());

        quantizer.reset();
        assert!(quantizer.cached_result.is_none());
        assert!(quantizer.shared_palette.is_none());
    }
}
