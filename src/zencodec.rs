//! zencodec-types trait implementations for zengif.
//!
//! Provides [`GifEncoderConfig`] and [`GifDecoderConfig`] types that implement the
//! [`EncoderConfig`] / [`DecoderConfig`] traits from zencodec-types.
//!
//! Supports both single-frame and animation encoding/decoding via the
//! 4-layer trait hierarchy.
//!
//! Requires `std` feature (GIF codec uses `std::io`).

extern crate alloc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use zencodec_types::{
    CodecCapabilities, DecodeFrame, DecodeOutput, EncodeOutput, ImageFormat, ImageInfo,
    ImageMetadata, OutputInfo, PixelData, PixelDescriptor, PixelSlice, PixelSliceMut,
    ResourceLimits, Stop,
};

use crate::encode::{EncodeRequest, EncoderConfig};
use crate::types::{FrameInput, Repeat};
use crate::{Decoder, GifError, Limits};

/// Build a zengif [`Limits`] from a [`ResourceLimits`], starting from zengif defaults.
fn limits_from_resource(rl: &ResourceLimits) -> Limits {
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
fn merge_resource_limits(base: &Limits, rl: &ResourceLimits) -> Limits {
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

// ── Capabilities ─────────────────────────────────────────────────────

static ENCODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_encode_cancel(true)
    .with_cheap_probe(true)
    .with_lossless(true)
    .with_quality_range(0.0, 100.0)
    .with_encode_animation(true);

static DECODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_decode_cancel(true)
    .with_decode_animation(true);

// ── Supported descriptors ────────────────────────────────────────────

static ENCODE_DESCRIPTORS: &[PixelDescriptor] = &[
    PixelDescriptor::RGBA8_SRGB,
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::GRAY8_SRGB,
    PixelDescriptor::BGRA8_SRGB,
];

static DECODE_DESCRIPTORS: &[PixelDescriptor] = &[
    PixelDescriptor::RGBA8_SRGB,
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::GRAY8_SRGB,
    PixelDescriptor::BGRA8_SRGB,
    PixelDescriptor::RGBF32_LINEAR,
    PixelDescriptor::RGBAF32_LINEAR,
    PixelDescriptor::GRAYF32_LINEAR,
];

// ── GifEncoderConfig ─────────────────────────────────────────────────

/// GIF encoder configuration implementing [`EncoderConfig`](zencodec_types::EncoderConfig).
///
/// Supports both single-frame and animation encoding. Quality maps to
/// the quantizer quality setting. Lossless mode sets lossy tolerance to 0.
///
/// # Quantizer Required
///
/// GIF encoding requires a color quantizer feature to be enabled
/// (`quantizr`, `imagequant`, etc.). Without one, encoding will fail.
#[derive(Clone, Debug)]
pub struct GifEncoderConfig {
    inner: EncoderConfig,
    limits: ResourceLimits,
    quality: Option<f32>,
    lossless: Option<bool>,
}

impl GifEncoderConfig {
    /// Create a new GIF encoder config with defaults.
    #[must_use]
    pub fn new() -> Self {
        let mut inner = EncoderConfig::new();
        inner.repeat = Repeat::Once;
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
            quality: None,
            lossless: None,
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

impl Default for GifEncoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl zencodec_types::EncoderConfig for GifEncoderConfig {
    type Error = GifError;
    type Job<'a> = GifEncodeJob<'a>;

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ENCODE_DESCRIPTORS
    }

    fn capabilities() -> &'static CodecCapabilities {
        &ENCODE_CAPS
    }

    fn with_calibrated_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality.clamp(0.0, 100.0));
        self = self.with_quality(quality);
        self
    }

    fn calibrated_quality(&self) -> Option<f32> {
        self.quality
    }

    fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = Some(lossless);
        if lossless {
            self.inner.lossy_tolerance = 0;
        }
        self
    }

    fn is_lossless(&self) -> Option<bool> {
        self.lossless
    }

    fn job(&self) -> GifEncodeJob<'_> {
        GifEncodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }
}

// ── GifEncodeJob ─────────────────────────────────────────────────────

/// Per-operation GIF encode job.
pub struct GifEncodeJob<'a> {
    config: &'a GifEncoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl<'a> zencodec_types::EncodeJob<'a> for GifEncodeJob<'a> {
    type Error = GifError;
    type Encoder = GifEncoder<'a>;
    type FrameEncoder = GifFrameEncoder<'a>;

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

    fn encoder(self) -> GifEncoder<'a> {
        GifEncoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
        }
    }

    fn frame_encoder(self) -> Result<GifFrameEncoder<'a>, GifError> {
        Ok(GifFrameEncoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
            frames: Vec::new(),
        })
    }
}

// ── GifEncoder ───────────────────────────────────────────────────────

/// Single-image GIF encoder.
pub struct GifEncoder<'a> {
    config: &'a GifEncoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl GifEncoder<'_> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(&self.config.limits);
        match self.limits {
            Some(ref job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
    }

    fn do_encode(
        &self,
        rgba_pixels: Vec<crate::Rgba>,
        w: u16,
        h: u16,
    ) -> Result<EncodeOutput, GifError> {
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

/// Convert a PixelSlice to GIF RGBA pixels.
fn pixels_to_gif_rgba(pixels: &PixelSlice<'_>) -> Result<(Vec<crate::Rgba>, u16, u16), GifError> {
    let w = u16::try_from(pixels.width()).map_err(|_| GifError::DimensionsTooLarge {
        width: pixels.width().min(u16::MAX as u32) as u16,
        height: pixels.rows().min(u16::MAX as u32) as u16,
        max_width: u16::MAX,
        max_height: u16::MAX,
    })?;
    let h = u16::try_from(pixels.rows()).map_err(|_| GifError::DimensionsTooLarge {
        width: pixels.width().min(u16::MAX as u32) as u16,
        height: pixels.rows().min(u16::MAX as u32) as u16,
        max_width: u16::MAX,
        max_height: u16::MAX,
    })?;

    let desc = pixels.descriptor();
    let bytes = collect_contiguous_bytes(pixels);

    let rgba = match (desc.channel_type, desc.layout) {
        (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgb) => bytes
            .chunks_exact(3)
            .map(|c| crate::Rgba::rgb(c[0], c[1], c[2]))
            .collect(),
        (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgba) => bytes
            .chunks_exact(4)
            .map(|c| crate::Rgba::new(c[0], c[1], c[2], c[3]))
            .collect(),
        (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Gray) => {
            bytes.iter().map(|&v| crate::Rgba::rgb(v, v, v)).collect()
        }
        (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Bgra) => bytes
            .chunks_exact(4)
            .map(|c| crate::Rgba::new(c[2], c[1], c[0], c[3]))
            .collect(),
        (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgb) => {
            use linear_srgb::default::linear_to_srgb_u8;
            bytes
                .chunks_exact(12)
                .map(|c| {
                    let r = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                    let g = f32::from_ne_bytes([c[4], c[5], c[6], c[7]]);
                    let b = f32::from_ne_bytes([c[8], c[9], c[10], c[11]]);
                    crate::Rgba::rgb(
                        linear_to_srgb_u8(r.clamp(0.0, 1.0)),
                        linear_to_srgb_u8(g.clamp(0.0, 1.0)),
                        linear_to_srgb_u8(b.clamp(0.0, 1.0)),
                    )
                })
                .collect()
        }
        (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgba) => {
            use linear_srgb::default::linear_to_srgb_u8;
            bytes
                .chunks_exact(16)
                .map(|c| {
                    let r = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                    let g = f32::from_ne_bytes([c[4], c[5], c[6], c[7]]);
                    let b = f32::from_ne_bytes([c[8], c[9], c[10], c[11]]);
                    let a = f32::from_ne_bytes([c[12], c[13], c[14], c[15]]);
                    crate::Rgba::new(
                        linear_to_srgb_u8(r.clamp(0.0, 1.0)),
                        linear_to_srgb_u8(g.clamp(0.0, 1.0)),
                        linear_to_srgb_u8(b.clamp(0.0, 1.0)),
                        (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    )
                })
                .collect()
        }
        (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Gray) => {
            use linear_srgb::default::linear_to_srgb_u8;
            bytes
                .chunks_exact(4)
                .map(|c| {
                    let v = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                    let s = linear_to_srgb_u8(v.clamp(0.0, 1.0));
                    crate::Rgba::rgb(s, s, s)
                })
                .collect()
        }
        _ => {
            return Err(GifError::InvalidEncoderState {
                message: "unsupported pixel format for GIF encoding",
            });
        }
    };

    Ok((rgba, w, h))
}

impl zencodec_types::Encoder for GifEncoder<'_> {
    type Error = GifError;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, GifError> {
        let (rgba, w, h) = pixels_to_gif_rgba(&pixels)?;
        self.do_encode(rgba, w, h)
    }

    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support incremental encoding",
        })
    }

    fn finish(self) -> Result<EncodeOutput, GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support incremental encoding",
        })
    }

    fn encode_from(
        self,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support pull-based encoding",
        })
    }
}

// ── GifFrameEncoder ──────────────────────────────────────────────────

/// Animation GIF encoder — collects frames, then encodes on finish.
pub struct GifFrameEncoder<'a> {
    config: &'a GifEncoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
    frames: Vec<FrameInput>,
}

impl GifFrameEncoder<'_> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(&self.config.limits);
        match self.limits {
            Some(ref job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
    }
}

impl zencodec_types::FrameEncoder for GifFrameEncoder<'_> {
    type Error = GifError;

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), GifError> {
        let (rgba, w, h) = pixels_to_gif_rgba(&pixels)?;
        // GIF uses centiseconds
        let delay_cs = (duration_ms / 10).max(1) as u16;
        let frame = FrameInput::new(w, h, delay_cs, rgba);
        self.frames.push(frame);
        Ok(())
    }

    fn begin_frame(&mut self, _duration_ms: u32) -> Result<(), GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support row-level frame building",
        })
    }

    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support row-level frame building",
        })
    }

    fn end_frame(&mut self) -> Result<(), GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support row-level frame building",
        })
    }

    fn pull_frame(
        &mut self,
        _duration_ms: u32,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support pull-based frame encoding",
        })
    }

    fn finish(self) -> Result<EncodeOutput, GifError> {
        if self.frames.is_empty() {
            return Err(GifError::InvalidEncoderState {
                message: "no frames to encode",
            });
        }

        let limits = self.build_limits();
        let stop: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);

        // Use the first frame's dimensions as the canvas size
        let w = self.frames[0].width;
        let h = self.frames[0].height;

        let data = EncodeRequest::new(&self.config.inner, w, h)
            .limits(&limits)
            .stop(stop)
            .encode(self.frames)
            .map_err(|e| e.into_inner())?;

        Ok(EncodeOutput::new(data, ImageFormat::Gif))
    }
}

// ── GifDecoderConfig ─────────────────────────────────────────────────

/// GIF decoder configuration implementing [`DecoderConfig`](zencodec_types::DecoderConfig).
///
/// Supports both single-frame and animation decoding.
#[derive(Clone, Debug)]
pub struct GifDecoderConfig {
    limits: ResourceLimits,
}

impl GifDecoderConfig {
    /// Create a new decoder config with default limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: ResourceLimits::default(),
        }
    }
}

impl Default for GifDecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl zencodec_types::DecoderConfig for GifDecoderConfig {
    type Error = GifError;
    type Job<'a> = GifDecodeJob<'a>;

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        DECODE_DESCRIPTORS
    }

    fn capabilities() -> &'static CodecCapabilities {
        &DECODE_CAPS
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

        let gif_limits = limits_from_resource(&self.limits);
        let cursor = std::io::Cursor::new(data);
        let decoder =
            Decoder::new(cursor, gif_limits, &enough::Unstoppable).map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        Ok(ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true))
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

        let gif_limits = limits_from_resource(&self.limits);
        let cursor = std::io::Cursor::new(data);
        let mut decoder =
            Decoder::new(cursor, gif_limits, &enough::Unstoppable).map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        let mut frame_count = 0u32;
        while decoder.next_frame().map_err(|e| e.into_inner())?.is_some() {
            frame_count += 1;
        }

        Ok(ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true)
        .with_animation(frame_count > 1)
        .with_frame_count(frame_count))
    }
}

// ── GifDecodeJob ─────────────────────────────────────────────────────

/// Per-operation GIF decode job.
pub struct GifDecodeJob<'a> {
    config: &'a GifDecoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl<'a> GifDecodeJob<'a> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(&self.config.limits);
        match self.limits {
            Some(ref job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
    }

    fn check_file_size(&self, data: &[u8]) -> Result<(), GifError> {
        if let Some(max) = self.config.limits.max_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }
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
        Ok(())
    }
}

impl<'a> zencodec_types::DecodeJob<'a> for GifDecodeJob<'a> {
    type Error = GifError;
    type Decoder = GifDecoder<'a>;
    type FrameDecoder = GifFrameDecoder;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, GifError> {
        self.check_file_size(data)?;
        let gif_limits = limits_from_resource(&self.config.limits);
        let cursor = std::io::Cursor::new(data);
        let decoder =
            Decoder::new(cursor, gif_limits, &enough::Unstoppable).map_err(|e| e.into_inner())?;
        let metadata = decoder.metadata().clone();
        Ok(OutputInfo::full_decode(
            metadata.width as u32,
            metadata.height as u32,
            PixelDescriptor::RGBA8_SRGB,
        )
        .with_alpha(true))
    }

    fn decoder(self) -> GifDecoder<'a> {
        GifDecoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
        }
    }

    fn frame_decoder(self, data: &[u8]) -> Result<GifFrameDecoder, GifError> {
        self.check_file_size(data)?;
        let limits = self.build_limits();
        // Use 'static Unstoppable — cancellation not yet supported for frame decoding
        let cursor = std::io::Cursor::new(data.to_vec());
        let decoder =
            Decoder::new(cursor, limits, &enough::Unstoppable).map_err(|e| e.into_inner())?;
        let metadata = decoder.metadata().clone();
        let shared_info = Arc::new(
            ImageInfo::new(
                metadata.width as u32,
                metadata.height as u32,
                ImageFormat::Gif,
            )
            .with_alpha(true)
            .with_animation(metadata.frame_count > 1)
            .with_frame_count(metadata.frame_count as u32),
        );
        Ok(GifFrameDecoder {
            decoder,
            shared_info,
            frame_index: 0,
        })
    }
}

// ── GifDecoder ───────────────────────────────────────────────────────

/// Single-image GIF decoder (decodes first frame).
pub struct GifDecoder<'a> {
    config: &'a GifDecoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl GifDecoder<'_> {
    fn build_limits(&self) -> Limits {
        let base = limits_from_resource(&self.config.limits);
        match self.limits {
            Some(ref job_limits) => merge_resource_limits(&base, job_limits),
            None => base,
        }
    }
}

impl zencodec_types::Decoder for GifDecoder<'_> {
    type Error = GifError;

    fn decode(self, data: &[u8]) -> Result<DecodeOutput, GifError> {
        // Check file size limits
        if let Some(max) = self.config.limits.max_file_size {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }
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
        let mut decoder = Decoder::new(cursor, limits, stop).map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        let frame = decoder
            .next_frame()
            .map_err(|e| e.into_inner())?
            .ok_or(GifError::UnexpectedEof)?;

        let w = frame.width as usize;
        let h = frame.height as usize;

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

        let pixel_data = PixelData::Rgba8(zencodec_types::ImgVec::new(rgba, w, h));

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

    fn decode_into(self, data: &[u8], mut dst: PixelSliceMut<'_>) -> Result<ImageInfo, GifError> {
        let desc = dst.descriptor();
        let output = self.decode(data)?;
        let info = output.info().clone();

        match (desc.channel_type, desc.layout) {
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgb) => {
                let src = output.into_rgb8();
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgba) => {
                let src = output.into_rgba8();
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Gray) => {
                let src = output.into_gray8();
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Bgra) => {
                let src = output.into_bgra8();
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgb) => {
                use linear_srgb::default::srgb_u8_to_linear;
                let src = output.into_rgb8();
                for y in 0..src.height().min(dst.rows() as usize) {
                    let src_row = &src.buf()[y * src.stride()..][..src.width()];
                    let dst_row = dst.row_mut(y as u32);
                    for (i, s) in src_row.iter().enumerate() {
                        let offset = i * 12;
                        if offset + 12 > dst_row.len() {
                            break;
                        }
                        dst_row[offset..offset + 4]
                            .copy_from_slice(&srgb_u8_to_linear(s.r).to_ne_bytes());
                        dst_row[offset + 4..offset + 8]
                            .copy_from_slice(&srgb_u8_to_linear(s.g).to_ne_bytes());
                        dst_row[offset + 8..offset + 12]
                            .copy_from_slice(&srgb_u8_to_linear(s.b).to_ne_bytes());
                    }
                }
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgba) => {
                use linear_srgb::default::srgb_u8_to_linear;
                let src = output.into_rgba8();
                for y in 0..src.height().min(dst.rows() as usize) {
                    let src_row = &src.buf()[y * src.stride()..][..src.width()];
                    let dst_row = dst.row_mut(y as u32);
                    for (i, s) in src_row.iter().enumerate() {
                        let offset = i * 16;
                        if offset + 16 > dst_row.len() {
                            break;
                        }
                        dst_row[offset..offset + 4]
                            .copy_from_slice(&srgb_u8_to_linear(s.r).to_ne_bytes());
                        dst_row[offset + 4..offset + 8]
                            .copy_from_slice(&srgb_u8_to_linear(s.g).to_ne_bytes());
                        dst_row[offset + 8..offset + 12]
                            .copy_from_slice(&srgb_u8_to_linear(s.b).to_ne_bytes());
                        dst_row[offset + 12..offset + 16]
                            .copy_from_slice(&(s.a as f32 / 255.0).to_ne_bytes());
                    }
                }
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Gray) => {
                use linear_srgb::default::srgb_u8_to_linear;
                let src = output.into_gray8();
                for y in 0..src.height().min(dst.rows() as usize) {
                    let src_row = &src.buf()[y * src.stride()..][..src.width()];
                    let dst_row = dst.row_mut(y as u32);
                    for (i, s) in src_row.iter().enumerate() {
                        let offset = i * 4;
                        if offset + 4 > dst_row.len() {
                            break;
                        }
                        dst_row[offset..offset + 4]
                            .copy_from_slice(&srgb_u8_to_linear(s.value()).to_ne_bytes());
                    }
                }
            }
            _ => {
                return Err(GifError::InvalidEncoderState {
                    message: "unsupported decode_into format",
                });
            }
        }

        Ok(info)
    }

    fn decode_rows(
        self,
        _data: &[u8],
        _sink: &mut dyn FnMut(u32, PixelSlice<'_>),
    ) -> Result<ImageInfo, GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support row-level decode callback",
        })
    }
}

// ── GifFrameDecoder ──────────────────────────────────────────────────

/// Animation GIF decoder — yields frames one at a time.
pub struct GifFrameDecoder {
    decoder: Decoder<'static, std::io::Cursor<Vec<u8>>>,
    shared_info: Arc<ImageInfo>,
    frame_index: u32,
}

impl zencodec_types::FrameDecoder for GifFrameDecoder {
    type Error = GifError;

    fn frame_count(&self) -> Option<u32> {
        let count = self.decoder.metadata().frame_count;
        if count > 0 {
            Some(count as u32)
        } else {
            None
        }
    }

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, GifError> {
        let frame = self.decoder.next_frame().map_err(|e| e.into_inner())?;

        let frame = match frame {
            Some(f) => f,
            None => return Ok(None),
        };

        let w = frame.width as usize;
        let h = frame.height as usize;

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

        let pixel_data = PixelData::Rgba8(zencodec_types::ImgVec::new(rgba, w, h));
        let duration_ms = frame.delay as u32 * 10;

        let index = self.frame_index;
        self.frame_index += 1;

        Ok(Some(DecodeFrame::new(
            pixel_data,
            self.shared_info.clone(),
            duration_ms,
            index,
        )))
    }

    fn next_frame_into(
        &mut self,
        _dst: PixelSliceMut<'_>,
        _prior_frame: Option<u32>,
    ) -> Result<Option<ImageInfo>, GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support next_frame_into yet",
        })
    }

    fn next_frame_rows(
        &mut self,
        _sink: &mut dyn FnMut(u32, PixelSlice<'_>),
    ) -> Result<Option<ImageInfo>, GifError> {
        Err(GifError::InvalidEncoderState {
            message: "GIF does not support row-level frame decode",
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Collect contiguous bytes from a PixelSlice (handles stride).
fn collect_contiguous_bytes(pixels: &PixelSlice<'_>) -> Vec<u8> {
    let h = pixels.rows();
    let w = pixels.width();
    let bpp = pixels.descriptor().bytes_per_pixel();
    let row_bytes = w as usize * bpp;

    let mut out = Vec::with_capacity(row_bytes * h as usize);
    for y in 0..h {
        out.extend_from_slice(&pixels.row(y)[..row_bytes]);
    }
    out
}

/// Copy rows from a typed ImgVec into a PixelSliceMut.
fn copy_rows_u8<P: Copy>(src: &zencodec_types::ImgVec<P>, dst: &mut PixelSliceMut<'_>)
where
    [P]: rgb::ComponentBytes<u8>,
{
    use rgb::ComponentBytes;
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let src_bytes = src_row.as_bytes();
        let dst_row = dst.row_mut(y as u32);
        let n = src_bytes.len().min(dst_row.len());
        dst_row[..n].copy_from_slice(&src_bytes[..n]);
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec_types::{DecodeJob, DecoderConfig, EncoderConfig};

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
        let dec = GifDecoderConfig::new();
        let output = dec.decode(MINIMAL_GIF).unwrap();
        assert_eq!(output.width(), 1);
        assert_eq!(output.height(), 1);
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[test]
    fn probe_header_minimal() {
        let dec = GifDecoderConfig::new();
        let info = dec.probe_header(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.format, ImageFormat::Gif);
        assert_eq!(info.frame_count, None);
    }

    #[test]
    fn probe_full_minimal() {
        let dec = GifDecoderConfig::new();
        let info = dec.probe_full(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.format, ImageFormat::Gif);
        assert!(!info.has_animation);
        assert_eq!(info.frame_count, Some(1));
    }

    #[test]
    fn output_info_minimal() {
        let dec = GifDecoderConfig::new();
        let info = dec.job().output_info(MINIMAL_GIF).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn roundtrip_rgb8() {
        use zencodec_types::Rgb;

        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 255, g: 0, b: 0 }; 16 * 16];
        let img = zencodec_types::ImgVec::new(pixels, 16, 16);

        let enc = GifEncoderConfig::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);

        let dec = GifDecoderConfig::new();
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
        use zencodec_types::Rgba;

        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 0,
                g: 128,
                b: 255,
                a: 200,
            };
            8 * 8
        ];
        let img = zencodec_types::ImgVec::new(pixels, 8, 8);

        let enc = GifEncoderConfig::new().with_quality(80.0);
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
    fn four_layer_encode_flow() {
        use zencodec_types::{EncodeJob, Encoder, Rgb};

        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 255, g: 0, b: 0 }; 16 * 16];
        let img = zencodec_types::ImgVec::new(pixels, 16, 16);
        let config = GifEncoderConfig::new();

        let slice = PixelSlice::from(img.as_ref());
        let output = config.job().encoder().encode(slice).unwrap();
        assert_eq!(output.format(), ImageFormat::Gif);
        assert!(!output.bytes().is_empty());
    }

    #[test]
    fn four_layer_decode_flow() {
        use zencodec_types::Decoder as _;
        let config = GifDecoderConfig::new();
        let decoded = config.job().decoder().decode(MINIMAL_GIF).unwrap();
        assert_eq!(decoded.width(), 1);
        assert_eq!(decoded.height(), 1);
    }

    #[test]
    fn capabilities_are_correct() {
        let caps = GifEncoderConfig::capabilities();
        assert!(caps.encode_cancel());
        assert!(caps.lossless());
        assert_eq!(caps.quality_range(), Some([0.0, 100.0]));
        assert!(caps.encode_animation());
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

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            let pixels: Vec<zencodec_types::Rgb<f32>> = vec![
                zencodec_types::Rgb {
                    r: 0.0,
                    g: 0.5,
                    b: 1.0,
                },
                zencodec_types::Rgb {
                    r: 0.25,
                    g: 0.75,
                    b: 0.1,
                },
                zencodec_types::Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                zencodec_types::Rgb {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                },
            ];
            let mut big_pixels = Vec::new();
            for _ in 0..64 {
                big_pixels.extend_from_slice(&pixels);
            }
            let img = zencodec_types::ImgVec::new(big_pixels, 16, 16);
            let enc = GifEncoderConfig::new();
            let output = enc.encode_rgb_f32(img.as_ref()).unwrap();

            let dec = GifDecoderConfig::new();
            let mut buf = vec![
                zencodec_types::Rgb {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0
                };
                256
            ];
            let mut dst = zencodec_types::ImgVec::new(buf.clone(), 16, 16);
            dec.decode_into_rgb_f32(output.bytes(), dst.as_mut())
                .unwrap();
            buf = dst.into_buf();

            for decoded in &buf {
                assert!(decoded.r >= 0.0 && decoded.r <= 1.0);
                assert!(decoded.g >= 0.0 && decoded.g <= 1.0);
                assert!(decoded.b >= 0.0 && decoded.b <= 1.0);
            }
            assert!(buf.iter().any(|p| p.r > 0.0 || p.g > 0.0 || p.b > 0.0));
        });
        assert!(report.permutations_run >= 1);
    }

    #[test]
    fn encoding_clone_send_sync() {
        fn assert_traits<T: Clone + Send + Sync>() {}
        assert_traits::<GifEncoderConfig>();
    }

    #[test]
    fn decoding_clone_send_sync() {
        fn assert_traits<T: Clone + Send + Sync>() {}
        assert_traits::<GifDecoderConfig>();
    }
}
