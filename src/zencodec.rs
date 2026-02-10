//! zencodec-types trait implementations for zengif.
//!
//! Provides [`GifEncoding`] and [`GifDecoding`] types that implement the
//! [`Encoding`] / [`Decoding`] traits from zencodec-types.
//!
//! GIF encoding produces single-frame GIFs via the trait interface.
//! For animation encoding, use the native zengif API directly.
//!
//! Requires `std` feature (GIF codec uses `std::io`).

extern crate alloc;
use alloc::vec::Vec;

use zencodec_types::{
    DecodeOutput, Decoding, DecodingJob, EncodeOutput, Encoding, EncodingJob, ImageFormat,
    ImageInfo, ImageMetadata, ImgRef, ImgVec, PixelData, Stop,
};

use crate::encode::{EncoderConfig, EncodeRequest};
use crate::types::{FrameInput, Repeat};
use crate::{Decoder, GifError, Limits};

// ── Encoding ────────────────────────────────────────────────────────────────

/// GIF encoder configuration implementing [`Encoding`].
///
/// Produces single-frame GIFs via the trait interface. For animation,
/// use the native [`EncodeRequest`] / [`Encoder`](crate::Encoder) API.
///
/// # Quantizer Required
///
/// GIF encoding requires a color quantizer feature to be enabled
/// (`quantizr`, `imagequant`, etc.). Without one, encoding will fail.
#[derive(Clone, Debug)]
pub struct GifEncoding {
    inner: EncoderConfig,
    quality: Option<f32>,
    limit_pixels: Option<u64>,
    limit_memory: Option<u64>,
    limit_output: Option<u64>,
}

impl GifEncoding {
    /// Create a new GIF encoder config with defaults.
    #[must_use]
    pub fn new() -> Self {
        let mut inner = EncoderConfig::new();
        inner.repeat = Repeat::Once;
        // Set quantizer to auto-detect best available backend
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        {
            inner.quantizer = Some(crate::Quantizer::auto());
        }
        Self {
            inner,
            quality: None,
            limit_pixels: None,
            limit_memory: None,
            limit_output: None,
        }
    }

    /// Set lossy frame differencing tolerance (0-255, 0=lossless).
    #[must_use]
    pub fn with_lossy_tolerance(mut self, tolerance: u8) -> Self {
        self.inner = self.inner.lossy_tolerance(tolerance);
        self
    }

    /// Set repeat behavior.
    #[must_use]
    pub fn with_repeat(mut self, repeat: Repeat) -> Self {
        self.inner.repeat = repeat;
        self
    }

    /// Access the underlying [`EncoderConfig`].
    #[must_use]
    pub fn inner(&self) -> &EncoderConfig {
        &self.inner
    }

    /// Mutably access the underlying [`EncoderConfig`].
    pub fn inner_mut(&mut self) -> &mut EncoderConfig {
        &mut self.inner
    }
}

impl Default for GifEncoding {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoding for GifEncoding {
    type Error = GifError;
    type Job<'a> = GifEncodeJob<'a>;

    fn with_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality.clamp(0.0, 100.0));
        // Map to inner quality if quantizer features are available
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        {
            self.inner.quality = quality.clamp(0.0, 100.0) as u8;
        }
        self
    }

    fn with_effort(self, _effort: u32) -> Self {
        // GIF doesn't have an effort/speed tradeoff
        self
    }

    fn with_lossless(mut self, lossless: bool) -> Self {
        if lossless {
            self.inner.lossy_tolerance = 0;
        }
        self
    }

    fn with_alpha_quality(self, _quality: f32) -> Self {
        // GIF alpha is 1-bit (transparent or not), no quality setting
        self
    }

    fn with_limit_pixels(mut self, max: u64) -> Self {
        self.limit_pixels = Some(max);
        self
    }

    fn with_limit_memory(mut self, bytes: u64) -> Self {
        self.limit_memory = Some(bytes);
        self
    }

    fn with_limit_output(mut self, bytes: u64) -> Self {
        self.limit_output = Some(bytes);
        self
    }

    fn job(&self) -> GifEncodeJob<'_> {
        GifEncodeJob {
            config: self,
            stop: None,
            limit_pixels: None,
            limit_memory: None,
        }
    }
}

/// Per-operation GIF encode job.
pub struct GifEncodeJob<'a> {
    config: &'a GifEncoding,
    stop: Option<&'a dyn Stop>,
    limit_pixels: Option<u64>,
    limit_memory: Option<u64>,
}

impl<'a> GifEncodeJob<'a> {
    fn build_limits(&self) -> Limits {
        let mut limits = Limits::default();
        if let Some(px) = self.limit_pixels.or(self.config.limit_pixels) {
            limits.max_total_pixels = Some(px);
        }
        if let Some(mem) = self.limit_memory.or(self.config.limit_memory) {
            limits.max_memory = Some(mem);
        }
        limits
    }

    fn do_encode(self, rgba_pixels: Vec<crate::Rgba>, w: u16, h: u16) -> Result<EncodeOutput, GifError> {
        let limits = self.build_limits();
        let stop: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);

        let frame = FrameInput::new(w, h, 0, rgba_pixels);

        let data = EncodeRequest::new(&self.config.inner, w, h)
            .limits(&limits)
            .stop(stop)
            .encode(alloc::vec![frame])
            .map_err(|e| e.into_inner())?;

        Ok(EncodeOutput::new(data, ImageFormat::Gif))
    }
}

impl<'a> EncodingJob<'a> for GifEncodeJob<'a> {
    type Error = GifError;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_metadata(self, _meta: &'a ImageMetadata<'a>) -> Self {
        // GIF doesn't support ICC/EXIF/XMP metadata
        self
    }

    fn with_icc(self, _icc: &'a [u8]) -> Self {
        self
    }

    fn with_exif(self, _exif: &'a [u8]) -> Self {
        self
    }

    fn with_xmp(self, _xmp: &'a [u8]) -> Self {
        self
    }

    fn with_limit_pixels(mut self, max: u64) -> Self {
        self.limit_pixels = Some(max);
        self
    }

    fn with_limit_memory(mut self, bytes: u64) -> Self {
        self.limit_memory = Some(bytes);
        self
    }

    fn encode_rgb8(self, img: ImgRef<'_, zencodec_types::Rgb<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = img.width() as u16;
        let h = img.height() as u16;
        // Expand RGB to zengif RGBA
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::rgb(p.r, p.g, p.b))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_rgba8(self, img: ImgRef<'_, zencodec_types::Rgba<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = img.width() as u16;
        let h = img.height() as u16;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_gray8(self, img: ImgRef<'_, zencodec_types::Gray<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = img.width() as u16;
        let h = img.height() as u16;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|g| {
                let v = g.value();
                crate::Rgba::rgb(v, v, v)
            })
            .collect();
        self.do_encode(rgba, w, h)
    }
}

// ── Decoding ────────────────────────────────────────────────────────────────

/// GIF decoder configuration implementing [`Decoding`].
///
/// Decodes the first frame of a GIF. For animation decoding,
/// use the native [`Decoder`] API directly.
#[derive(Clone, Debug)]
pub struct GifDecoding {
    limits: Limits,
    limit_file_size: Option<u64>,
}

impl GifDecoding {
    /// Create a new decoder config with default limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: Limits::default(),
            limit_file_size: None,
        }
    }

    /// Access the underlying [`Limits`].
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Mutably access the underlying [`Limits`].
    pub fn limits_mut(&mut self) -> &mut Limits {
        &mut self.limits
    }
}

impl Default for GifDecoding {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoding for GifDecoding {
    type Error = GifError;
    type Job<'a> = GifDecodeJob<'a>;

    fn with_limit_pixels(mut self, max: u64) -> Self {
        self.limits.max_total_pixels = Some(max);
        self
    }

    fn with_limit_memory(mut self, bytes: u64) -> Self {
        self.limits.max_memory = Some(bytes);
        self
    }

    fn with_limit_dimensions(mut self, width: u32, height: u32) -> Self {
        self.limits.max_width = Some(width.min(u16::MAX as u32) as u16);
        self.limits.max_height = Some(height.min(u16::MAX as u32) as u16);
        self
    }

    fn with_limit_file_size(mut self, bytes: u64) -> Self {
        self.limit_file_size = Some(bytes);
        self.limits.max_file_size = Some(bytes);
        self
    }

    fn job(&self) -> GifDecodeJob<'_> {
        GifDecodeJob {
            config: self,
            stop: None,
            limit_pixels: None,
            limit_memory: None,
        }
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        if let Some(max) = self.limit_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let cursor = std::io::Cursor::new(data);
        let mut decoder = Decoder::new(cursor, self.limits.clone(), &enough::Unstoppable)
            .map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();
        // Count frames by decoding (GIF requires parsing to count)
        let mut frame_count = 0u32;
        while decoder.next_frame().map_err(|e| e.into_inner())?.is_some() {
            frame_count += 1;
        }

        Ok(ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true) // GIF always supports transparency
        .with_animation(frame_count > 1)
        .with_frame_count(frame_count))
    }
}

/// Per-operation GIF decode job.
pub struct GifDecodeJob<'a> {
    config: &'a GifDecoding,
    stop: Option<&'a dyn Stop>,
    limit_pixels: Option<u64>,
    limit_memory: Option<u64>,
}

impl<'a> GifDecodeJob<'a> {
    fn build_limits(&self) -> Limits {
        let mut limits = self.config.limits.clone();
        if let Some(px) = self.limit_pixels {
            limits.max_total_pixels = Some(px);
        }
        if let Some(mem) = self.limit_memory {
            limits.max_memory = Some(mem);
        }
        limits
    }
}

impl<'a> DecodingJob<'a> for GifDecodeJob<'a> {
    type Error = GifError;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limit_pixels(mut self, max: u64) -> Self {
        self.limit_pixels = Some(max);
        self
    }

    fn with_limit_memory(mut self, bytes: u64) -> Self {
        self.limit_memory = Some(bytes);
        self
    }

    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error> {
        // Check file size
        if let Some(max) = self.config.limit_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let limits = self.build_limits();
        let stop: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);
        let cursor = std::io::Cursor::new(data);
        let mut decoder = Decoder::new(cursor, limits, stop)
            .map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        let frame = decoder
            .next_frame()
            .map_err(|e| e.into_inner())?
            .ok_or(GifError::UnexpectedEof)?;

        let w = frame.width as usize;
        let h = frame.height as usize;

        // Convert zengif::Rgba to zencodec_types::Rgba
        let rgba: Vec<zencodec_types::Rgba<u8>> = frame
            .pixels
            .iter()
            .map(|p| zencodec_types::Rgba {
                r: p.r,
                g: p.g,
                b: p.b,
                a: p.a,
            })
            .collect();

        let pixel_data = PixelData::Rgba8(ImgVec::new(rgba, w, h));

        let info = ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true)
        .with_animation(metadata.frame_count > 1)
        .with_frame_count(metadata.frame_count as u32);

        Ok(DecodeOutput::new(pixel_data, info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec_types::{Decoding, Encoding, Rgb, Rgba};

    // Minimal valid GIF89a (1x1 red pixel)
    const MINIMAL_GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00, 0x00, // Color 1: Black
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x44, 0x01, // LZW data
        0x00, // Block terminator
        0x3B, // Trailer
    ];

    #[test]
    fn decode_minimal() {
        let dec = GifDecoding::new();
        let output = dec.decode(MINIMAL_GIF).unwrap();
        assert_eq!(output.width(), 1);
        assert_eq!(output.height(), 1);
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[test]
    fn probe_minimal() {
        let dec = GifDecoding::new();
        let info = dec.probe(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.format, ImageFormat::Gif);
        assert!(!info.has_animation);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn roundtrip_rgb8() {
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 255,
                g: 0,
                b: 0,
            };
            16 * 16
        ];
        let img = ImgVec::new(pixels, 16, 16);

        let enc = GifEncoding::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);

        let dec = GifDecoding::new();
        let decoded = dec.decode(output.bytes()).unwrap();
        assert_eq!(decoded.width(), 16);
        assert_eq!(decoded.height(), 16);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encode_rgba8() {
        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 0,
                g: 128,
                b: 255,
                a: 200,
            };
            8 * 8
        ];
        let img = ImgVec::new(pixels, 8, 8);

        let enc = GifEncoding::new().with_quality(80.0);
        let output = enc.encode_rgba8(img.as_ref()).unwrap();
        assert!(!output.is_empty());
    }
}
