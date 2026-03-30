//! zenquant quantizer backend.

use super::{QuantizeConfig, QuantizedFrame, QuantizerTrait, compute_sample_indices};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// zenquant-based quantizer.
///
/// Uses the zenquant library for perceptual-quality color quantization.
/// Produces the best butteraugli/SSIMULACRA2 scores among available backends.
/// AGPL-3.0-or-later licensed.
pub struct ZenquantQuantizer {
    /// Cached QuantizeResult for shared palette mode.
    cached_result: Option<zenquant::QuantizeResult>,
}

impl ZenquantQuantizer {
    /// Create a new zenquant-based quantizer.
    pub fn new() -> Self {
        Self {
            cached_result: None,
        }
    }

    /// Build a zenquant config from zengif's QuantizeConfig.
    fn make_config(config: &QuantizeConfig) -> zenquant::QuantizeConfig {
        let mut zq = zenquant::QuantizeConfig::new(zenquant::OutputFormat::Gif);
        // Map quality ranges to zenquant Quality enum
        zq = if config.quality >= 75 {
            zq.with_quality(zenquant::Quality::Best)
        } else if config.quality >= 40 {
            zq.with_quality(zenquant::Quality::Balanced)
        } else {
            zq.with_quality(zenquant::Quality::Fast)
        };
        // Map dithering level
        zq = zq._with_dither_strength(config.dithering);
        zq
    }

    /// Convert zengif Rgba pixels to zenquant RGBA pixels.
    ///
    /// Both types have identical layout (r, g, b, a as u8), but are
    /// distinct types. We use bytemuck for zero-copy conversion.
    fn convert_pixels(pixels: &[Rgba]) -> &[zenquant::RGBA<u8>] {
        // Safety: Rgba and rgb::RGBA<u8> have identical repr(C) layout.
        // Both are { r: u8, g: u8, b: u8, a: u8 } with no padding.
        // We use bytemuck to verify this at compile time.
        bytemuck::cast_slice(pixels)
    }

    /// Extract palette bytes (RGB) from a zenquant result.
    fn palette_to_bytes(result: &zenquant::QuantizeResult) -> Vec<u8> {
        result
            .palette()
            .iter()
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect()
    }
}

impl Default for ZenquantQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantizerTrait for ZenquantQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        let zq_config = Self::make_config(config);
        let zq_pixels = Self::convert_pixels(pixels);

        let result =
            zenquant::quantize_rgba(zq_pixels, width as usize, height as usize, &zq_config)
                .map_err(|_| {
                    at!(GifError::QuantizationFailed {
                        message: "zenquant quantization failed"
                    })
                })?;

        let transparent_index = result.transparent_index();

        // Post-process: ensure alpha==0 pixels map to transparent index
        let mut indices = result.indices().to_vec();
        let actual_transparent_index = transparent_index.unwrap_or(255);
        let mut has_transparent_pixels = false;

        for (i, p) in pixels.iter().enumerate() {
            if p.a == 0 {
                indices[i] = actual_transparent_index;
                has_transparent_pixels = true;
            }
        }

        let final_transparent_index = if has_transparent_pixels {
            Some(actual_transparent_index)
        } else {
            transparent_index
        };

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(&result),
            pixels: indices,
            transparent_index: final_transparent_index,
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
        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        let zq_config = Self::make_config(config);
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);

        // Build ImgRef slices for sampled frames
        let sampled_pixels: Vec<&[zenquant::RGBA<u8>]> = sample_indices
            .iter()
            .map(|&idx| Self::convert_pixels(frames[idx]))
            .collect();

        let img_refs: Vec<zenquant::ImgRef<'_, zenquant::RGBA<u8>>> = sampled_pixels
            .iter()
            .map(|pixels| zenquant::ImgRef::new(pixels, width as usize, height as usize))
            .collect();

        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        let result = zenquant::build_palette_rgba(&img_refs, &zq_config).map_err(|_| {
            at!(GifError::QuantizationFailed {
                message: "zenquant palette building failed"
            })
        })?;

        let palette_bytes = Self::palette_to_bytes(&result);
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
        let cached = self.cached_result.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;

        let zq_config = Self::make_config(config);
        let zq_pixels = Self::convert_pixels(pixels);

        let result = cached
            .remap_rgba(zq_pixels, width as usize, height as usize, &zq_config)
            .map_err(|_| {
                at!(GifError::QuantizationFailed {
                    message: "zenquant remapping failed"
                })
            })?;

        let transparent_index = result.transparent_index();
        let palette_bytes = Self::palette_to_bytes(&result);

        // Post-process: ensure alpha==0 pixels map to transparent index
        let mut indices = result.indices().to_vec();
        let actual_transparent_index = transparent_index.unwrap_or(255);
        let mut has_transparent_pixels = false;

        for (i, p) in pixels.iter().enumerate() {
            if p.a == 0 {
                indices[i] = actual_transparent_index;
                has_transparent_pixels = true;
            }
        }

        let final_transparent_index = if has_transparent_pixels {
            Some(actual_transparent_index)
        } else {
            transparent_index
        };

        Ok(QuantizedFrame {
            palette: palette_bytes,
            pixels: indices,
            transparent_index: final_transparent_index,
        })
    }

    fn reset(&mut self) {
        self.cached_result = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zenquant_quantize_single_frame() {
        let mut quantizer = ZenquantQuantizer::new();
        let config = QuantizeConfig::default();

        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
    }

    #[test]
    fn zenquant_shared_palette_reuses_result() {
        let mut quantizer = ZenquantQuantizer::new();
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
        assert!(
            blue_result
                .pixels
                .iter()
                .all(|&p| p == blue_result.pixels[0])
        );

        // Red and blue should map to different palette entries
        assert_ne!(red_result.pixels[0], blue_result.pixels[0]);

        // Verify the indices point to correct colors in the palette
        let red_idx = red_result.pixels[0] as usize;
        let blue_idx = blue_result.pixels[0] as usize;
        let red_color = &palette_bytes[red_idx * 3..red_idx * 3 + 3];
        let blue_color = &palette_bytes[blue_idx * 3..blue_idx * 3 + 3];

        // Red entry should be mostly red
        assert!(
            red_color[0] > 200 && red_color[1] < 50 && red_color[2] < 50,
            "expected red, got RGB({}, {}, {})",
            red_color[0],
            red_color[1],
            red_color[2]
        );
        // Blue entry should be mostly blue
        assert!(
            blue_color[0] < 50 && blue_color[1] < 50 && blue_color[2] > 200,
            "expected blue, got RGB({}, {}, {})",
            blue_color[0],
            blue_color[1],
            blue_color[2]
        );
    }

    #[test]
    fn zenquant_reset_clears_cached_result() {
        let mut quantizer = ZenquantQuantizer::new();
        let config = QuantizeConfig::default();
        let stop = enough::Unstoppable;

        let pixels = vec![Rgba::rgb(128, 128, 128); 16];
        let frames: Vec<&[Rgba]> = vec![&pixels];

        quantizer
            .build_shared_palette(&frames, 4, 4, &config, &stop)
            .expect("palette should build");
        assert!(quantizer.cached_result.is_some());

        quantizer.reset();
        assert!(quantizer.cached_result.is_none());
    }
}
