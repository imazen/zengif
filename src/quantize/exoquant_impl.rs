//! Exoquant quantizer backend.

use super::{compute_sample_indices, QuantizeConfig, QuantizedFrame, QuantizerTrait};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

/// Exoquant-based quantizer.
///
/// Uses the exoquant library for high-quality K-Means quantization.
/// MIT licensed, produces very good results.
pub struct ExoquantQuantizer {
    /// Cached palette for shared palette mode.
    shared_palette: Option<Vec<exoquant::Color>>,
}

impl ExoquantQuantizer {
    /// Create a new exoquant-based quantizer.
    pub fn new() -> Self {
        Self {
            shared_palette: None,
        }
    }

    /// Convert Rgba to exoquant Color.
    fn to_exoquant_color(p: &Rgba) -> exoquant::Color {
        exoquant::Color::new(p.r, p.g, p.b, p.a)
    }

    /// Convert exoquant palette to GIF palette bytes (RGB only).
    fn palette_to_bytes(palette: &[exoquant::Color]) -> Vec<u8> {
        palette.iter().flat_map(|c| [c.r, c.g, c.b]).collect()
    }

    /// Find the most transparent color index in a palette.
    fn find_transparent_index(palette: &[exoquant::Color]) -> Option<u8> {
        palette
            .iter()
            .enumerate()
            .filter(|(_, c)| c.a < 128)
            .max_by_key(|(_, c)| 255 - c.a)
            .map(|(i, _)| i as u8)
    }
}

impl Default for ExoquantQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantizerTrait for ExoquantQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        _height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        use exoquant::{convert_to_indexed, ditherer, optimizer};

        let exo_pixels: Vec<exoquant::Color> = pixels.iter().map(Self::to_exoquant_color).collect();

        // Choose ditherer based on config
        let (palette, indexed) = if config.dithering > 0.01 {
            convert_to_indexed(
                &exo_pixels,
                width as usize,
                256,
                &optimizer::KMeans,
                &ditherer::FloydSteinberg::new(),
            )
        } else {
            convert_to_indexed(
                &exo_pixels,
                width as usize,
                256,
                &optimizer::KMeans,
                &ditherer::None,
            )
        };

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(&palette),
            pixels: indexed,
            transparent_index: Self::find_transparent_index(&palette),
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
        use exoquant::{Histogram, Quantizer as ExoQuantizer, SimpleColorSpace};

        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        let colorspace = SimpleColorSpace::default();
        let mut histogram: Histogram = Histogram::new();
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);

        for &idx in &sample_indices {
            stop.check().map_err(|_| at!(GifError::Cancelled))?;

            let frame_pixels = frames[idx];
            histogram.extend(frame_pixels.iter().map(Self::to_exoquant_color));
        }

        let mut quantizer = ExoQuantizer::new(&histogram, &colorspace);
        // Step to get 256 colors
        for _ in 0..256 {
            quantizer.step();
        }
        let palette = quantizer.colors(&colorspace);

        self.shared_palette = Some(palette.clone());

        Ok(Self::palette_to_bytes(&palette))
    }

    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        _height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        use exoquant::{ditherer, Color, Remapper, SimpleColorSpace};

        let palette = self.shared_palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette - call build_shared_palette first"
            })
        })?;

        let colorspace = SimpleColorSpace::default();
        let exo_pixels: Vec<Color> = pixels.iter().map(Self::to_exoquant_color).collect();

        let indexed = if config.dithering > 0.01 {
            let dither = ditherer::FloydSteinberg::new();
            let remapper = Remapper::new(palette, &colorspace, &dither);
            remapper.remap(&exo_pixels, width as usize)
        } else {
            let dither = ditherer::None;
            let remapper = Remapper::new(palette, &colorspace, &dither);
            remapper.remap(&exo_pixels, width as usize)
        };

        Ok(QuantizedFrame {
            palette: Self::palette_to_bytes(palette),
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
    fn exoquant_quantize_single_frame() {
        let mut quantizer = ExoquantQuantizer::new();
        let config = QuantizeConfig::default();

        let pixels = vec![Rgba::rgb(255, 0, 0); 16];
        let result = quantizer
            .quantize_frame(&pixels, 4, 4, None, &config)
            .expect("quantization should succeed");

        assert!(!result.palette.is_empty());
        assert_eq!(result.pixels.len(), 16);
    }
}
