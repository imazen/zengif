//! ColorQuant quantizer backend.

use super::{QuantizeConfig, QuantizedFrame, QuantizerTrait, compute_sample_indices};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// ColorQuant-based quantizer.
///
/// Uses the color_quant library with the NEUQUANT algorithm.
/// MIT licensed, fast neural network-based quantization.
pub struct ColorQuantQuantizer {
    /// Cached palette for shared palette mode.
    shared_palette: Option<Vec<u8>>,
}

impl ColorQuantQuantizer {
    /// Create a new color_quant-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_palette: None,
        }
    }

    /// Find the most transparent color index in a palette (RGBA format).
    fn find_transparent_index(palette_rgba: &[u8]) -> Option<u8> {
        palette_rgba
            .chunks(4)
            .enumerate()
            .filter(|(_, c)| c.len() == 4 && c[3] < 128)
            .max_by_key(|(_, c)| 255 - c[3])
            .map(|(i, _)| i as u8)
    }

    /// Convert RGBA palette to RGB palette bytes.
    fn rgba_to_rgb(palette_rgba: &[u8]) -> Vec<u8> {
        palette_rgba
            .chunks(4)
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect()
    }
}

impl Default for ColorQuantQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantizerTrait for ColorQuantQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        _width: u16,
        _height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        use color_quant::NeuQuant;

        // Convert pixels to RGBA bytes
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();

        // sampling_factor: 1 = best quality (slowest), 30 = fastest
        // Map quality 1-100 to sampling_factor 30-1
        let sampling_factor = (100 - config.quality.clamp(1, 100)) as i32 * 29 / 99 + 1;

        let nq = NeuQuant::new(sampling_factor, 256, &pixel_bytes);
        let palette_rgba = nq.color_map_rgba();

        let indexed: Vec<u8> = pixel_bytes
            .chunks(4)
            .map(|pix| nq.index_of(pix) as u8)
            .collect();

        Ok(QuantizedFrame {
            palette: Self::rgba_to_rgb(&palette_rgba),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(&palette_rgba),
        })
    }

    fn build_shared_palette(
        &mut self,
        frames: &[&[Rgba]],
        _width: u16,
        _height: u16,
        config: &QuantizeConfig,
        stop: &dyn Stop,
    ) -> Result<Vec<u8>> {
        use color_quant::NeuQuant;

        stop.check().map_err(|r| at!(GifError::Cancelled(r)))?;

        // Collect pixels from sampled frames
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);
        let mut all_pixels: Vec<u8> = Vec::new();

        for &idx in &sample_indices {
            stop.check().map_err(|r| at!(GifError::Cancelled(r)))?;

            let frame_pixels = frames[idx];
            for p in frame_pixels {
                all_pixels.extend_from_slice(&[p.r, p.g, p.b, p.a]);
            }
        }

        let sampling_factor = (100 - config.quality.clamp(1, 100)) as i32 * 29 / 99 + 1;

        let nq = NeuQuant::new(sampling_factor, 256, &all_pixels);
        let palette_rgba = nq.color_map_rgba();
        let palette_rgb = Self::rgba_to_rgb(&palette_rgba);

        self.shared_palette = Some(palette_rgba);

        Ok(palette_rgb)
    }

    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        _width: u16,
        _height: u16,
        _background: Option<&[Rgba]>,
        _config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        let palette_rgba = self.shared_palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;

        // NeuQuant doesn't support remapping with a pre-built palette,
        // so we do manual nearest-color lookup
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();

        // Use the shared palette's RGBA data to create a lookup
        // For each pixel, find the closest color in the shared palette
        let indexed: Vec<u8> = pixel_bytes
            .chunks(4)
            .map(|pix| {
                // Find closest color in palette
                let mut best_idx = 0u8;
                let mut best_dist = u32::MAX;
                for (i, pc) in palette_rgba.chunks(4).enumerate() {
                    let dr = (pix[0] as i32 - pc[0] as i32).unsigned_abs();
                    let dg = (pix[1] as i32 - pc[1] as i32).unsigned_abs();
                    let db = (pix[2] as i32 - pc[2] as i32).unsigned_abs();
                    let da = (pix[3] as i32 - pc[3] as i32).unsigned_abs();
                    let dist = dr * dr + dg * dg + db * db + da * da;
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = i as u8;
                    }
                }
                best_idx
            })
            .collect();

        Ok(QuantizedFrame {
            palette: Self::rgba_to_rgb(palette_rgba),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(palette_rgba),
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
    fn color_quant_quantize_single_frame() {
        let mut quantizer = ColorQuantQuantizer::new();
        let config = QuantizeConfig::default();

        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
    }
}
