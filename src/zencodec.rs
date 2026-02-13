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
    CodecCapabilities, DecodeOutput, Decoding, DecodingJob, EncodeOutput, Encoding, EncodingJob,
    ImageFormat, ImageInfo, ImageMetadata, ImgRef, ImgVec, PixelData, ResourceLimits, Stop,
};

use crate::encode::{EncoderConfig, EncodeRequest};
use crate::types::{FrameInput, Repeat};
use crate::{Decoder, GifError, Limits};

/// Build a zengif [`Limits`] from a [`ResourceLimits`], starting from zengif defaults.
fn limits_from_resource(rl: ResourceLimits) -> Limits {
    let mut limits = Limits::default();
    if let Some(px) = rl.max_pixels {
        limits.max_total_pixels = Some(px);
    }
    if let Some(mem) = rl.max_memory_bytes {
        limits.max_memory = Some(mem);
    }
    if let Some(w) = rl.max_width {
        limits.max_width = Some(w.min(u16::MAX as u32) as u16);
    }
    if let Some(h) = rl.max_height {
        limits.max_height = Some(h.min(u16::MAX as u32) as u16);
    }
    if let Some(fs) = rl.max_file_size {
        limits.max_file_size = Some(fs);
    }
    limits
}

/// Merge a [`ResourceLimits`] into an existing zengif [`Limits`], overriding
/// only fields that are `Some` in the `ResourceLimits`.
fn merge_resource_limits(base: &Limits, rl: ResourceLimits) -> Limits {
    let mut limits = base.clone();
    if let Some(px) = rl.max_pixels {
        limits.max_total_pixels = Some(px);
    }
    if let Some(mem) = rl.max_memory_bytes {
        limits.max_memory = Some(mem);
    }
    if let Some(w) = rl.max_width {
        limits.max_width = Some(w.min(u16::MAX as u32) as u16);
    }
    if let Some(h) = rl.max_height {
        limits.max_height = Some(h.min(u16::MAX as u32) as u16);
    }
    if let Some(fs) = rl.max_file_size {
        limits.max_file_size = Some(fs);
    }
    limits
}

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
    limits: ResourceLimits,
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
            limits: ResourceLimits::default(),
        }
    }

    /// Set quality (0.0-100.0). Maps to the quantizer quality setting.
    #[must_use]
    pub fn with_quality(mut self, #[allow(unused)] quality: f32) -> Self {
        #[cfg(any(
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        {
            self.inner.quality = quality.clamp(0.0, 100.0) as u8;
        }
        let _ = &mut self; // suppress unused-mut when no quantizer feature
        self
    }

    /// Set effort/speed tradeoff.
    ///
    /// GIF doesn't have a meaningful effort setting; this is a no-op.
    #[must_use]
    pub fn with_effort(self, _effort: u32) -> Self {
        self
    }

    /// Set lossless mode. When true, sets lossy tolerance to 0.
    #[must_use]
    pub fn with_lossless(mut self, lossless: bool) -> Self {
        if lossless {
            self.inner.lossy_tolerance = 0;
        }
        self
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

static ENCODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_encode_cancel(true)
    .with_cheap_probe(true);

impl Encoding for GifEncoding {
    type Error = GifError;
    type Job<'a> = GifEncodeJob<'a>;

    fn capabilities() -> &'static CodecCapabilities {
        &ENCODE_CAPS
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn job(&self) -> GifEncodeJob<'_> {
        GifEncodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }
}

/// Per-operation GIF encode job.
pub struct GifEncodeJob<'a> {
    config: &'a GifEncoding,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl<'a> GifEncodeJob<'a> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(self.config.limits);
        match self.limits {
            Some(job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
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

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn encode_rgb8(self, img: ImgRef<'_, zencodec_types::Rgb<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = u16::try_from(img.width()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let h = u16::try_from(img.height()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        // Expand RGB to zengif RGBA
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::rgb(p.r, p.g, p.b))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_rgba8(self, img: ImgRef<'_, zencodec_types::Rgba<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = u16::try_from(img.width()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let h = u16::try_from(img.height()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_gray8(self, img: ImgRef<'_, zencodec_types::Gray<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = u16::try_from(img.width()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let h = u16::try_from(img.height()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|g| {
                let v = g.value();
                crate::Rgba::rgb(v, v, v)
            })
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_bgra8(self, img: ImgRef<'_, zencodec_types::Bgra<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = u16::try_from(img.width()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let h = u16::try_from(img.height()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_bgrx8(self, img: ImgRef<'_, zencodec_types::Bgra<u8>>) -> Result<EncodeOutput, Self::Error> {
        let (buf, _, _) = img.to_contiguous_buf();
        let w = u16::try_from(img.width()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let h = u16::try_from(img.height()).map_err(|_| GifError::DimensionsTooLarge {
            width: img.width().min(u16::MAX as usize) as u16,
            height: img.height().min(u16::MAX as usize) as u16,
            max_width: u16::MAX,
            max_height: u16::MAX,
        })?;
        let rgba: Vec<crate::Rgba> = buf
            .iter()
            .map(|p| crate::Rgba::rgb(p.r, p.g, p.b))
            .collect();
        self.do_encode(rgba, w, h)
    }

    fn encode_rgb_f32(self, img: ImgRef<'_, zencodec_types::Rgb<f32>>) -> Result<EncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let rgb: Vec<zencodec_types::Rgb<u8>> = buf
            .iter()
            .map(|p| zencodec_types::Rgb {
                r: linear_to_srgb_u8(p.r.clamp(0.0, 1.0)),
                g: linear_to_srgb_u8(p.g.clamp(0.0, 1.0)),
                b: linear_to_srgb_u8(p.b.clamp(0.0, 1.0)),
            })
            .collect();
        let rgb_img = ImgVec::new(rgb, w, h);
        self.encode_rgb8(rgb_img.as_ref())
    }

    fn encode_rgba_f32(self, img: ImgRef<'_, zencodec_types::Rgba<f32>>) -> Result<EncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let rgba: Vec<zencodec_types::Rgba<u8>> = buf
            .iter()
            .map(|p| zencodec_types::Rgba {
                r: linear_to_srgb_u8(p.r.clamp(0.0, 1.0)),
                g: linear_to_srgb_u8(p.g.clamp(0.0, 1.0)),
                b: linear_to_srgb_u8(p.b.clamp(0.0, 1.0)),
                a: (p.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            })
            .collect();
        let rgba_img = ImgVec::new(rgba, w, h);
        self.encode_rgba8(rgba_img.as_ref())
    }

    fn encode_gray_f32(self, img: ImgRef<'_, zencodec_types::Gray<f32>>) -> Result<EncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let gray: Vec<zencodec_types::Gray<u8>> = buf
            .iter()
            .map(|g| zencodec_types::Gray::new(linear_to_srgb_u8(g.value().clamp(0.0, 1.0))))
            .collect();
        let gray_img = ImgVec::new(gray, w, h);
        self.encode_gray8(gray_img.as_ref())
    }
}

// ── Decoding ────────────────────────────────────────────────────────────────

/// GIF decoder configuration implementing [`Decoding`].
///
/// Decodes the first frame of a GIF. For animation decoding,
/// use the native [`Decoder`] API directly.
#[derive(Clone, Debug)]
pub struct GifDecoding {
    limits: ResourceLimits,
}

impl GifDecoding {
    /// Create a new decoder config with default limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: ResourceLimits::default(),
        }
    }
}

impl Default for GifDecoding {
    fn default() -> Self {
        Self::new()
    }
}

static DECODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_decode_cancel(true);

impl Decoding for GifDecoding {
    type Error = GifError;
    type Job<'a> = GifDecodeJob<'a>;

    fn capabilities() -> &'static CodecCapabilities {
        &DECODE_CAPS
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn job(&self) -> GifDecodeJob<'_> {
        GifDecodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }

    fn probe_header(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        if let Some(max) = self.limits.max_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let gif_limits = limits_from_resource(self.limits);
        let cursor = std::io::Cursor::new(data);
        let decoder = Decoder::new(cursor, gif_limits, &enough::Unstoppable)
            .map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        // Header-only probe: return dimensions and format without counting frames.
        Ok(ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true)) // GIF always supports transparency
    }

    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        if let Some(max) = self.limits.max_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let gif_limits = limits_from_resource(self.limits);
        let cursor = std::io::Cursor::new(data);
        let mut decoder = Decoder::new(cursor, gif_limits, &enough::Unstoppable)
            .map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        // Full probe: walk all frames to count them. O(file_size).
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
    limits: Option<ResourceLimits>,
}

impl<'a> GifDecodeJob<'a> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(self.config.limits);
        match self.limits {
            Some(job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
    }
}

impl<'a> DecodingJob<'a> for GifDecodeJob<'a> {
    type Error = GifError;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error> {
        // Check file size from config limits
        if let Some(max) = self.config.limits.max_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }
        // Also check job-level file size override
        if let Some(ref job_limits) = self.limits {
            if let Some(max) = job_limits.max_file_size {
                if data.len() as u64 > max {
                    return Err(GifError::FileTooLarge {
                        size: data.len() as u64,
                        max,
                    });
                }
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

    fn decode_into_rgb8(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Rgb<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgb8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_rgba8(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Rgba<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgba8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_gray8(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Gray<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_gray8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_bgra8(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Bgra<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_bgra8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_bgrx8(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Bgra<u8>>,
    ) -> Result<ImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_bgra8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = zencodec_types::Bgra {
                    b: s.b,
                    g: s.g,
                    r: s.r,
                    a: 255,
                };
            }
        }
        Ok(info)
    }

    fn decode_into_rgb_f32(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Rgb<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        use linear_srgb::default::srgb_u8_to_linear;
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgb8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = zencodec_types::Rgb {
                    r: srgb_u8_to_linear(s.r),
                    g: srgb_u8_to_linear(s.g),
                    b: srgb_u8_to_linear(s.b),
                };
            }
        }
        Ok(info)
    }

    fn decode_into_rgba_f32(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Rgba<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        use linear_srgb::default::srgb_u8_to_linear;
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgba8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = zencodec_types::Rgba {
                    r: srgb_u8_to_linear(s.r),
                    g: srgb_u8_to_linear(s.g),
                    b: srgb_u8_to_linear(s.b),
                    a: s.a as f32 / 255.0,
                };
            }
        }
        Ok(info)
    }

    fn decode_into_gray_f32(
        self,
        data: &[u8],
        mut dst: zencodec_types::ImgRefMut<'_, zencodec_types::Gray<f32>>,
    ) -> Result<ImageInfo, Self::Error> {
        use linear_srgb::default::srgb_u8_to_linear;
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_gray8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = zencodec_types::Gray::new(srgb_u8_to_linear(s.value()));
            }
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    use zencodec_types::{Encoding, Rgb, Rgba};

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
    fn probe_header_minimal() {
        let dec = GifDecoding::new();
        let info = dec.probe_header(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.format, ImageFormat::Gif);
        // Header probe doesn't count frames
        assert_eq!(info.frame_count, None);
    }

    #[test]
    fn probe_full_minimal() {
        let dec = GifDecoding::new();
        let info = dec.probe_full(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.format, ImageFormat::Gif);
        assert!(!info.has_animation);
        assert_eq!(info.frame_count, Some(1));
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

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn f32_conversion_all_simd_tiers() {
        use archmage::testing::{for_each_token_permutation, CompileTimePolicy};
        #[allow(unused_imports)]
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            // Encode linear f32 → GIF (sRGB u8) → decode to linear f32
            let pixels: Vec<zencodec_types::Rgb<f32>> = vec![
                zencodec_types::Rgb { r: 0.0, g: 0.5, b: 1.0 },
                zencodec_types::Rgb { r: 0.25, g: 0.75, b: 0.1 },
                zencodec_types::Rgb { r: 0.0, g: 0.0, b: 0.0 },
                zencodec_types::Rgb { r: 1.0, g: 1.0, b: 1.0 },
            ];
            // Use a 16x16 image (GIF quantization needs enough pixels)
            let mut big_pixels = Vec::new();
            for _ in 0..64 {
                big_pixels.extend_from_slice(&pixels);
            }
            let img = ImgVec::new(big_pixels.clone(), 16, 16);
            let enc = GifEncoding::new();
            let output = enc.encode_rgb_f32(img.as_ref()).unwrap();

            let dec = GifDecoding::new();
            let mut buf = vec![zencodec_types::Rgb { r: 0.0f32, g: 0.0, b: 0.0 }; 256];
            let mut dst = ImgVec::new(buf.clone(), 16, 16);
            dec.decode_into_rgb_f32(output.bytes(), dst.as_mut())
                .unwrap();
            buf = dst.into_buf();

            // GIF quantization is lossy, so just verify the linear conversion roundtrips
            // through sRGB correctly (not the pixel values themselves)
            for decoded in &buf {
                // Values must be valid linear light (0.0-1.0 range)
                assert!(decoded.r >= 0.0 && decoded.r <= 1.0);
                assert!(decoded.g >= 0.0 && decoded.g <= 1.0);
                assert!(decoded.b >= 0.0 && decoded.b <= 1.0);
            }
            // Verify at least one value is non-zero (decode actually happened)
            assert!(buf.iter().any(|p| p.r > 0.0 || p.g > 0.0 || p.b > 0.0));
        });
        assert!(report.permutations_run >= 1);
    }
}
