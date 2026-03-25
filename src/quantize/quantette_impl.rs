//! quantette quantizer backend.
//!
//! Uses Oklab k-means clustering for high-quality perceptual palette generation.
//! MIT/Apache-2.0 licensed. Does NOT handle alpha — transparent pixels are
//! identified by checking the source alpha after palette assignment.

use super::{QuantizeConfig, QuantizedFrame, QuantizerTrait, compute_sample_indices};
use crate::error::{GifError, Result};
use crate::types::Rgba;
use enough::Stop;
use whereat::at;

use quantette::deps::palette::Srgb;
use quantette::{ImageBuf, Pipeline, QuantizeMethod};

/// quantette-based quantizer.
///
/// Uses Oklab k-means for perceptually accurate color quantization.
/// MIT/Apache-2.0 licensed — best permissively-licensed quality option.
///
/// Note: quantette operates on RGB only. Alpha transparency is handled
/// by checking source pixel alpha after palette assignment.
pub struct QuantetteQuantizer {
    /// Cached shared palette (GIF RGB bytes).
    shared_palette: Option<Vec<u8>>,
    /// Cached palette as Srgb for remapping.
    shared_srgb: Option<Vec<Srgb<u8>>>,
}

impl QuantetteQuantizer {
    pub fn new() -> Self {
        Self {
            shared_palette: None,
            shared_srgb: None,
        }
    }

    fn build_pipeline(dithering: f32) -> Pipeline {
        let method = {
            use quantette::kmeans::KmeansOptions;
            QuantizeMethod::Kmeans(KmeansOptions::new())
        };

        let mut pipe = Pipeline::new().quantize_method(method);

        if dithering > 0.001 {
            use quantette::dither::FloydSteinberg;
            pipe = pipe.ditherer(Some(FloydSteinberg::new()));
        }

        pipe
    }

    fn pixels_to_srgb(pixels: &[Rgba]) -> Vec<Srgb<u8>> {
        pixels.iter().map(|p| Srgb::new(p.r, p.g, p.b)).collect()
    }

    fn find_transparent_index(pixels: &[Rgba], indices: &[u8]) -> Option<u8> {
        // Find the most common index assigned to transparent source pixels
        pixels
            .iter()
            .zip(indices.iter())
            .find(|(px, _)| px.a < 128)
            .map(|(_, &idx)| idx)
    }
}

impl Default for QuantetteQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantizerTrait for QuantetteQuantizer {
    fn quantize_frame(
        &mut self,
        pixels: &[Rgba],
        width: u16,
        height: u16,
        _background: Option<&[Rgba]>,
        config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        let srgb_pixels = Self::pixels_to_srgb(pixels);
        let image = ImageBuf::new(width as u32, height as u32, srgb_pixels).map_err(|_e| {
            at!(GifError::QuantizationFailed {
                message: "quantette image creation failed"
            })
        })?;

        let pipeline = Self::build_pipeline(config.dithering);
        let indexed = pipeline
            .input_image(image.as_ref())
            .output_srgb8_indexed_image();

        let palette_bytes: Vec<u8> = indexed
            .palette()
            .iter()
            .flat_map(|c| [c.red, c.green, c.blue])
            .collect();
        let indices = indexed.indices().to_vec();
        let transparent_index = Self::find_transparent_index(pixels, &indices);

        Ok(QuantizedFrame {
            palette: palette_bytes,
            pixels: indices,
            transparent_index,
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
        let sample_indices = compute_sample_indices(frames.len(), config.max_palette_frames);

        let mut all_srgb: Vec<Srgb<u8>> = Vec::new();
        for &idx in &sample_indices {
            stop.check().map_err(|_| at!(GifError::Cancelled))?;
            for px in frames[idx] {
                all_srgb.push(Srgb::new(px.r, px.g, px.b));
            }
        }

        let w = all_srgb.len() as u32;
        let image = ImageBuf::new(w, 1, all_srgb).map_err(|_e| {
            at!(GifError::QuantizationFailed {
                message: "quantette shared palette failed"
            })
        })?;

        let pipeline = Self::build_pipeline(config.dithering);
        let palette = pipeline
            .input_image(image.as_ref())
            .output_srgb8_palette()
            .ok_or_else(|| {
                at!(GifError::QuantizationFailed {
                    message: "quantette produced no palette"
                })
            })?;

        let palette_bytes: Vec<u8> = palette
            .iter()
            .flat_map(|c| [c.red, c.green, c.blue])
            .collect();
        let srgb_vec: Vec<Srgb<u8>> = palette.iter().copied().collect();
        self.shared_palette = Some(palette_bytes.clone());
        self.shared_srgb = Some(srgb_vec);

        Ok(palette_bytes)
    }

    fn quantize_frame_with_palette(
        &mut self,
        pixels: &[Rgba],
        _width: u16,
        _height: u16,
        _background: Option<&[Rgba]>,
        _config: &QuantizeConfig,
    ) -> Result<QuantizedFrame> {
        let palette_bytes = self.shared_palette.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette built"
            })
        })?;
        let srgb_palette = self.shared_srgb.as_ref().ok_or_else(|| {
            at!(GifError::QuantizationFailed {
                message: "no shared palette built"
            })
        })?;

        // Nearest-neighbor remap: find closest palette entry for each pixel
        let indices: Vec<u8> = pixels
            .iter()
            .map(|px| {
                let mut best_idx = 0u8;
                let mut best_dist = u32::MAX;
                for (i, pal) in srgb_palette.iter().enumerate() {
                    let dr = px.r as i32 - pal.red as i32;
                    let dg = px.g as i32 - pal.green as i32;
                    let db = px.b as i32 - pal.blue as i32;
                    let dist = (dr * dr + dg * dg + db * db) as u32;
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = i as u8;
                    }
                }
                best_idx
            })
            .collect();

        let transparent_index = Self::find_transparent_index(pixels, &indices);

        Ok(QuantizedFrame {
            palette: palette_bytes.clone(),
            pixels: indices,
            transparent_index,
        })
    }

    fn reset(&mut self) {
        self.shared_palette = None;
        self.shared_srgb = None;
    }
}
