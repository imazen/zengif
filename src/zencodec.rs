//! zencodec-types trait implementations for zengif.
//!
//! Provides [`GifEncoderConfig`] and [`GifDecoderConfig`] types that implement the
//! [`EncoderConfig`](zc::encode::EncoderConfig) / [`DecoderConfig`](zc::decode::DecoderConfig)
//! traits from zencodec-types.
//!
//! Supports both single-frame and animation encoding/decoding via the
//! type-erased [`Encoder`](zc::encode::Encoder) and [`FrameEncoder`](zc::encode::FrameEncoder)
//! traits.
//!
//! Requires `zencodec` feature (GIF codec uses `std::io`).

extern crate alloc;
use alloc::sync::Arc;
use alloc::vec::Vec;

use zc::decode::{DecodeFrame, DecodeOutput, OutputInfo};
use zc::encode::EncodeOutput;
use zc::{ImageFormat, ImageInfo, MetadataView, ResourceLimits};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSlice};

// Import trait for inherent method forwarding
use zc::decode::DecoderConfig as _;

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
    if let Some(fs) = rl.max_input_bytes {
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
    if let Some(fs) = rl.max_input_bytes {
        limits.max_file_size = Some(fs);
    }
    limits
}

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

// ── Capabilities ─────────────────────────────────────────────────────

static GIF_ENCODE_CAPS: zc::encode::EncodeCapabilities =
    zc::encode::EncodeCapabilities::new()
        .with_cancel(true)
        .with_animation(true)
        .with_lossless(true)
        .with_lossy(true)
        .with_native_alpha(true)
        .with_native_gray(true)
        .with_enforces_max_pixels(true)
        .with_enforces_max_memory(true)
        .with_quality_range(0.0, 100.0);

static GIF_DECODE_CAPS: zc::decode::DecodeCapabilities =
    zc::decode::DecodeCapabilities::new()
        .with_cancel(true)
        .with_animation(true)
        .with_cheap_probe(true)
        .with_native_alpha(true)
        .with_enforces_max_pixels(true)
        .with_enforces_max_memory(true)
        .with_enforces_max_input_bytes(true);

// ── GifEncoderConfig ─────────────────────────────────────────────────

/// GIF encoder configuration implementing [`EncoderConfig`](zc::encode::EncoderConfig).
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

    /// Set dithering level (0.0 = none, 1.0 = full).
    ///
    /// Requires a quantizer feature (`imagequant`, `quantizr`, etc.).
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn with_dithering(mut self, dithering: f32) -> Self {
        self.inner = self.inner.dithering(dithering);
        self
    }

    /// Use a shared palette across all frames (faster, less optimal per-frame).
    ///
    /// Requires a quantizer feature (`imagequant`, `quantizr`, etc.).
    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[must_use]
    pub fn with_shared_palette(mut self, shared: bool) -> Self {
        self.inner = self.inner.shared_palette(shared);
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

impl zc::encode::EncoderConfig for GifEncoderConfig {
    type Error = GifError;
    type Job<'a> = GifEncodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Gif
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ENCODE_DESCRIPTORS
    }

    fn capabilities() -> &'static zc::encode::EncodeCapabilities {
        &GIF_ENCODE_CAPS
    }

    fn with_generic_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality.clamp(0.0, 100.0));
        self = self.with_quality(quality);
        self
    }

    fn generic_quality(&self) -> Option<f32> {
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
            canvas_size: None,
            loop_count: None,
        }
    }
}

// ── GifEncodeJob ─────────────────────────────────────────────────────

/// Per-operation GIF encode job.
pub struct GifEncodeJob<'a> {
    config: &'a GifEncoderConfig,
    stop: Option<&'a dyn zc::enough::Stop>,
    limits: Option<ResourceLimits>,
    canvas_size: Option<(u32, u32)>,
    loop_count: Option<Option<u32>>,
}

impl<'a> zc::encode::EncodeJob<'a> for GifEncodeJob<'a> {
    type Error = GifError;
    type Enc = GifEncoder<'a>;
    type FrameEnc = GifFrameEncoder<'a>;

    fn with_stop(mut self, stop: &'a dyn zc::enough::Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_metadata(self, _meta: &'a MetadataView<'a>) -> Self {
        // GIF doesn't support ICC/EXIF/XMP metadata
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn with_canvas_size(mut self, width: u32, height: u32) -> Self {
        self.canvas_size = Some((width, height));
        self
    }

    fn with_loop_count(mut self, count: Option<u32>) -> Self {
        self.loop_count = Some(count);
        self
    }

    fn encoder(self) -> Result<GifEncoder<'a>, GifError> {
        Ok(GifEncoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
        })
    }

    fn frame_encoder(self) -> Result<GifFrameEncoder<'a>, GifError> {
        // Map loop_count to GIF repeat
        let mut inner_config = self.config.inner.clone();
        if let Some(count) = self.loop_count {
            inner_config.repeat = match count {
                Some(0) => Repeat::Infinite,
                Some(n) => Repeat::Count(n as u16),
                None => Repeat::Once,
            };
        }
        Ok(GifFrameEncoder {
            config: self.config,
            inner_config,
            stop: self.stop,
            limits: self.limits,
            canvas_size: self.canvas_size,
            frames: Vec::new(),
        })
    }
}

// ── GifEncoder ───────────────────────────────────────────────────────

/// Single-image GIF encoder.
pub struct GifEncoder<'a> {
    config: &'a GifEncoderConfig,
    stop: Option<&'a dyn zc::enough::Stop>,
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
        // Pre-flight memory check: 4 bytes/pixel for RGBA
        let effective_limits = match &self.limits {
            Some(job_limits) => job_limits,
            None => &self.config.limits,
        };
        let estimated_mem = w as u64 * h as u64 * 4;
        if let Some(max_mem) = effective_limits.max_memory_bytes {
            if estimated_mem > max_mem {
                return Err(GifError::MemoryLimitExceeded {
                    current: estimated_mem,
                    limit: max_mem,
                });
            }
        }

        let limits = self.build_limits();
        let stop: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);

        let frame = FrameInput::new(w, h, 0, rgba_pixels);

        let data = EncodeRequest::new(&self.config.inner, w, h)
            .limits(&limits)
            .stop(stop)
            .encode(alloc::vec![frame])
            .map_err(|e| e.into_inner())?;

        Ok(EncodeOutput::new(data, ImageFormat::Gif))
    }
}

impl zc::encode::Encoder for GifEncoder<'_> {
    type Error = GifError;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, GifError> {
        let (rgba, w, h) = pixels_to_gif_rgba(&pixels)?;
        self.do_encode(rgba, w, h)
    }
}

/// Convert a type-erased PixelSlice to GIF RGBA pixels.
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
    let bytes = pixels.contiguous_bytes();

    let rgba = match (desc.channel_type(), desc.layout()) {
        (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Rgb) => bytes
            .chunks_exact(3)
            .map(|c| crate::Rgba::rgb(c[0], c[1], c[2]))
            .collect(),
        (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Rgba) => bytes
            .chunks_exact(4)
            .map(|c| crate::Rgba::new(c[0], c[1], c[2], c[3]))
            .collect(),
        (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Gray) => {
            bytes.iter().map(|&v| crate::Rgba::rgb(v, v, v)).collect()
        }
        (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Bgra) => bytes
            .chunks_exact(4)
            .map(|c| crate::Rgba::new(c[2], c[1], c[0], c[3]))
            .collect(),
        (zenpixels::ChannelType::F32, zenpixels::ChannelLayout::Rgb) => {
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
        (zenpixels::ChannelType::F32, zenpixels::ChannelLayout::Rgba) => {
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
        (zenpixels::ChannelType::F32, zenpixels::ChannelLayout::Gray) => {
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

// ── GifFrameEncoder ──────────────────────────────────────────────────

/// Animation GIF encoder — collects frames, then encodes on finish.
pub struct GifFrameEncoder<'a> {
    config: &'a GifEncoderConfig,
    inner_config: EncoderConfig,
    stop: Option<&'a dyn zc::enough::Stop>,
    limits: Option<ResourceLimits>,
    canvas_size: Option<(u32, u32)>,
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

    fn push_frame_erased(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
    ) -> Result<(), GifError> {
        let (rgba, w, h) = pixels_to_gif_rgba(&pixels)?;
        // GIF uses centiseconds
        let delay_cs = (duration_ms / 10).max(1) as u16;
        let frame = FrameInput::new(w, h, delay_cs, rgba);
        self.frames.push(frame);
        Ok(())
    }

    fn do_finish(self) -> Result<EncodeOutput, GifError> {
        if self.frames.is_empty() {
            return Err(GifError::InvalidEncoderState {
                message: "no frames to encode",
            });
        }

        let limits = self.build_limits();
        let stop: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);

        // Use explicit canvas size if provided, otherwise first frame's dimensions
        let (w, h) = self.canvas_size.map_or_else(
            || (self.frames[0].width, self.frames[0].height),
            |(cw, ch)| (cw.min(u16::MAX as u32) as u16, ch.min(u16::MAX as u32) as u16),
        );

        let data = EncodeRequest::new(&self.inner_config, w, h)
            .limits(&limits)
            .stop(stop)
            .encode(self.frames)
            .map_err(|e| e.into_inner())?;

        Ok(EncodeOutput::new(data, ImageFormat::Gif))
    }
}

impl zc::encode::FrameEncoder for GifFrameEncoder<'_> {
    type Error = GifError;

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), GifError> {
        self.push_frame_erased(pixels, duration_ms)
    }

    fn with_loop_count(&mut self, count: Option<u32>) {
        self.inner_config.repeat = match count {
            Some(0) => Repeat::Infinite,
            Some(n) => Repeat::Count(n as u16),
            None => Repeat::Once,
        };
    }

    fn finish(self) -> Result<EncodeOutput, GifError> {
        self.do_finish()
    }
}

// ── GifDecoderConfig ─────────────────────────────────────────────────

/// GIF decoder configuration implementing [`DecoderConfig`](zc::decode::DecoderConfig).
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

    /// Convenience: probe image header without decoding pixels.
    pub fn probe_header(&self, data: &[u8]) -> Result<ImageInfo, GifError> {
        use zc::decode::DecodeJob as _;
        self.job().probe(data)
    }

    /// Convenience: probe with full parse (counts all frames).
    pub fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, GifError> {
        use zc::decode::DecodeJob as _;
        self.job().probe_full(data)
    }

    /// Convenience: decode with default job settings.
    pub fn decode(&self, data: &[u8]) -> Result<DecodeOutput, GifError> {
        use zc::decode::{Decode as _, DecodeJob as _};
        self.job().decoder(data, &[])?.decode()
    }
}

impl Default for GifDecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl zc::decode::DecoderConfig for GifDecoderConfig {
    type Error = GifError;
    type Job<'a> = GifDecodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Gif
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        DECODE_DESCRIPTORS
    }

    fn capabilities() -> &'static zc::decode::DecodeCapabilities {
        &GIF_DECODE_CAPS
    }

    fn job(&self) -> GifDecodeJob<'_> {
        GifDecodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }
}

// ── GifDecodeJob ─────────────────────────────────────────────────────

/// Per-operation GIF decode job.
pub struct GifDecodeJob<'a> {
    config: &'a GifDecoderConfig,
    stop: Option<&'a dyn zc::enough::Stop>,
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
        if let Some(max) = self.config.limits.max_input_bytes {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }
        if let Some(ref job_limits) = self.limits {
            if let Some(max) = job_limits.max_input_bytes {
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

impl<'a> zc::decode::DecodeJob<'a> for GifDecodeJob<'a> {
    type Error = GifError;
    type Dec = GifDecoder<'a>;
    type StreamDec = GifStreamingDecoder;
    type FrameDec = GifFrameDecoder;

    fn with_stop(mut self, stop: &'a dyn zc::enough::Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, GifError> {
        if let Some(max) = self.config.limits.max_input_bytes {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let gif_limits = limits_from_resource(&self.config.limits);
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

    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, GifError> {
        if let Some(max) = self.config.limits.max_input_bytes {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let gif_limits = limits_from_resource(&self.config.limits);
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

    fn decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<GifDecoder<'a>, GifError> {
        Ok(GifDecoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
            data,
            _preferred: preferred.to_vec(),
        })
    }

    fn streaming_decoder(
        self,
        _data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<GifStreamingDecoder, GifError> {
        Err(zc::UnsupportedOperation::RowLevelDecode.into())
    }

    fn frame_decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<GifFrameDecoder, GifError> {
        self.check_file_size(data)?;
        let limits = self.build_limits();
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
            _preferred: preferred.to_vec(),
        })
    }
}

// ── GifDecoder ───────────────────────────────────────────────────────

/// Single-image GIF decoder (decodes first frame).
pub struct GifDecoder<'a> {
    config: &'a GifDecoderConfig,
    stop: Option<&'a dyn zc::enough::Stop>,
    limits: Option<ResourceLimits>,
    data: &'a [u8],
    _preferred: Vec<PixelDescriptor>,
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

/// Convert a composed GIF frame to a raw RGBA8 byte vector.
fn frame_to_rgba_bytes(frame: &crate::ComposedFrame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(frame.pixels.len() * 4);
    for p in &frame.pixels {
        bytes.extend_from_slice(&[p.r, p.g, p.b, p.a]);
    }
    bytes
}

impl zc::decode::Decode for GifDecoder<'_> {
    type Error = GifError;

    fn decode(self) -> Result<DecodeOutput, GifError> {
        let data = self.data;

        // Check file size limits
        if let Some(max) = self.config.limits.max_input_bytes {
            if data.len() as u64 > max {
                return Err(GifError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }
        if let Some(ref job_limits) = self.limits {
            if let Some(max) = job_limits.max_input_bytes {
                if data.len() as u64 > max {
                    return Err(GifError::FileTooLarge {
                        size: data.len() as u64,
                        max,
                    });
                }
            }
        }

        let limits = self.build_limits();
        let stop: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);
        let cursor = std::io::Cursor::new(data);
        let mut decoder = Decoder::new(cursor, limits, stop).map_err(|e| e.into_inner())?;

        let metadata = decoder.metadata().clone();

        let frame = decoder
            .next_frame()
            .map_err(|e| e.into_inner())?
            .ok_or(GifError::UnexpectedEof)?;

        let rgba_bytes = frame_to_rgba_bytes(&frame);
        let buf = PixelBuffer::from_vec(
            rgba_bytes,
            metadata.width as u32,
            metadata.height as u32,
            PixelDescriptor::RGBA8_SRGB,
        )
        .map_err(|_| GifError::InvalidEncoderState {
            message: "frame size mismatch",
        })?;

        let info = ImageInfo::new(
            metadata.width as u32,
            metadata.height as u32,
            ImageFormat::Gif,
        )
        .with_alpha(true)
        .with_animation(metadata.frame_count > 1)
        .with_frame_count(metadata.frame_count as u32);

        Ok(DecodeOutput::new(buf, info))
    }
}

// ── GifStreamingDecoder ──────────────────────────────────────────────

/// Stub streaming decoder — GIF does not support streaming decode.
///
/// [`DecodeJob::streaming_decoder()`](zc::decode::DecodeJob::streaming_decoder)
/// always returns an error before this type is constructed.
pub struct GifStreamingDecoder {
    _private: (),
}

impl zc::decode::StreamingDecode for GifStreamingDecoder {
    type Error = GifError;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, GifError> {
        Err(zc::UnsupportedOperation::RowLevelDecode.into())
    }

    fn info(&self) -> &ImageInfo {
        unreachable!("GifStreamingDecoder is never constructed")
    }
}

// ── GifFrameDecoder ──────────────────────────────────────────────────

/// Animation GIF decoder — yields frames one at a time.
pub struct GifFrameDecoder {
    decoder: Decoder<'static, std::io::Cursor<Vec<u8>>>,
    shared_info: Arc<ImageInfo>,
    frame_index: u32,
    _preferred: Vec<PixelDescriptor>,
}

impl zc::decode::FrameDecode for GifFrameDecoder {
    type Error = GifError;

    fn frame_count(&self) -> Option<u32> {
        let count = self.decoder.metadata().frame_count;
        if count > 0 { Some(count as u32) } else { None }
    }

    fn loop_count(&self) -> Option<u32> {
        match self.decoder.metadata().repeat {
            Repeat::Infinite => Some(0),
            Repeat::Count(n) => Some(n as u32),
            Repeat::Once => Some(1),
        }
    }

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, GifError> {
        // GIF FrameDecoder returns fully composited RGBA frames — the internal
        // compositor applies disposal before returning each frame. DecodeFrame
        // defaults to FrameDisposal::None + FrameBlend::Source, which is correct:
        // the caller gets ready-to-display frames with no further compositing needed.
        let frame = self.decoder.next_frame().map_err(|e| e.into_inner())?;

        let frame = match frame {
            Some(f) => f,
            None => return Ok(None),
        };

        let rgba_bytes = frame_to_rgba_bytes(&frame);
        let buf = PixelBuffer::from_vec(
            rgba_bytes,
            frame.width as u32,
            frame.height as u32,
            PixelDescriptor::RGBA8_SRGB,
        )
        .map_err(|_| GifError::InvalidEncoderState {
            message: "frame size mismatch",
        })?;
        let duration_ms = frame.delay as u32 * 10;

        let index = self.frame_index;
        self.frame_index += 1;

        Ok(Some(DecodeFrame::new(
            buf,
            self.shared_info.clone(),
            duration_ms,
            index,
        )))
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zc::decode::DecodeJob as _;
    use zc::encode::{EncodeJob as _, Encoder as _, EncoderConfig as _};

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
    fn roundtrip_via_pixel_buffer() {
        // Create a 16x16 red image as raw RGBA bytes
        let mut rgba_bytes = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..16 * 16 {
            rgba_bytes.extend_from_slice(&[255, 0, 0, 255]);
        }
        let buf = PixelBuffer::from_vec(rgba_bytes, 16, 16, PixelDescriptor::RGBA8_SRGB).unwrap();
        let pixels = buf.as_slice();

        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(pixels).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);

        let dec = GifDecoderConfig::new();
        let decoded = dec.decode(output.data()).unwrap();
        assert_eq!(decoded.width(), 16);
        assert_eq!(decoded.height(), 16);
    }

    #[test]
    fn four_layer_decode_flow() {
        use zc::decode::Decode as _;
        let config = GifDecoderConfig::new();
        let decoded = config
            .job()
            .decoder(MINIMAL_GIF, &[])
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.width(), 1);
        assert_eq!(decoded.height(), 1);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_rgba8() {
        use zc::encode::Encoder as _;

        let mut rgba_bytes = Vec::with_capacity(16 * 16 * 4);
        for i in 0u32..16 * 16 {
            rgba_bytes.extend_from_slice(&[(i % 256) as u8, 128, 64, 255]);
        }
        let buf = PixelBuffer::from_vec(rgba_bytes, 16, 16, PixelDescriptor::RGBA8_SRGB).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_rgb8() {
        use zc::encode::Encoder as _;

        let mut rgb_bytes = Vec::with_capacity(16 * 16 * 3);
        for i in 0u32..16 * 16 {
            rgb_bytes.extend_from_slice(&[(i % 256) as u8, 128, 64]);
        }
        let buf = PixelBuffer::from_vec(rgb_bytes, 16, 16, PixelDescriptor::RGB8_SRGB).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_gray8() {
        use zc::encode::Encoder as _;

        let gray_bytes: Vec<u8> = (0..16 * 16).map(|i| (i % 256) as u8).collect();
        let buf =
            PixelBuffer::from_vec(gray_bytes, 16, 16, PixelDescriptor::GRAY8_SRGB).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_rgb_f32() {
        use zc::encode::Encoder as _;

        let mut f32_bytes = Vec::with_capacity(16 * 16 * 12);
        for i in 0u32..16 * 16 {
            let r = (i % 256) as f32 / 255.0;
            f32_bytes.extend_from_slice(&r.to_ne_bytes());
            f32_bytes.extend_from_slice(&0.5f32.to_ne_bytes());
            f32_bytes.extend_from_slice(&0.25f32.to_ne_bytes());
        }
        let buf =
            PixelBuffer::from_vec(f32_bytes, 16, 16, PixelDescriptor::RGBF32_LINEAR).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_rgba_f32() {
        use zc::encode::Encoder as _;

        let mut f32_bytes = Vec::with_capacity(16 * 16 * 16);
        for i in 0u32..16 * 16 {
            let r = (i % 256) as f32 / 255.0;
            f32_bytes.extend_from_slice(&r.to_ne_bytes());
            f32_bytes.extend_from_slice(&0.5f32.to_ne_bytes());
            f32_bytes.extend_from_slice(&0.25f32.to_ne_bytes());
            f32_bytes.extend_from_slice(&1.0f32.to_ne_bytes());
        }
        let buf =
            PixelBuffer::from_vec(f32_bytes, 16, 16, PixelDescriptor::RGBAF32_LINEAR).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_gray_f32() {
        use zc::encode::Encoder as _;

        let mut f32_bytes = Vec::with_capacity(16 * 16 * 4);
        for i in 0u32..16 * 16 {
            let v = (i % 256) as f32 / 255.0;
            f32_bytes.extend_from_slice(&v.to_ne_bytes());
        }
        let buf =
            PixelBuffer::from_vec(f32_bytes, 16, 16, PixelDescriptor::GRAYF32_LINEAR).unwrap();
        let config = GifEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn encoder_trait_dyn_encoder() {
        use zc::encode::EncodeJob as _;

        let mut rgba_bytes = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..32 * 32 {
            rgba_bytes.extend_from_slice(&[100, 150, 200, 255]);
        }
        let buf = PixelBuffer::from_vec(rgba_bytes, 32, 32, PixelDescriptor::RGBA8_SRGB).unwrap();
        let config = GifEncoderConfig::new();
        let dyn_enc = config.job().dyn_encoder().unwrap();
        let output = dyn_enc.encode(buf.as_slice()).unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Gif);
    }

    #[cfg(any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    #[test]
    fn frame_encoder_roundtrip() {
        use zc::decode::FrameDecode as _;
        use zc::encode::{EncodeJob as _, FrameEncoder as _};

        // Create two 8x8 frames
        let mut frame1_bytes = Vec::with_capacity(8 * 8 * 4);
        for _ in 0..8 * 8 {
            frame1_bytes.extend_from_slice(&[255, 0, 0, 255]); // red
        }
        let buf1 = PixelBuffer::from_vec(frame1_bytes, 8, 8, PixelDescriptor::RGBA8_SRGB).unwrap();

        let mut frame2_bytes = Vec::with_capacity(8 * 8 * 4);
        for _ in 0..8 * 8 {
            frame2_bytes.extend_from_slice(&[0, 0, 255, 255]); // blue
        }
        let buf2 = PixelBuffer::from_vec(frame2_bytes, 8, 8, PixelDescriptor::RGBA8_SRGB).unwrap();

        let config = GifEncoderConfig::new();
        let mut enc = config
            .job()
            .with_loop_count(Some(0))
            .frame_encoder()
            .unwrap();

        enc.push_frame(buf1.as_slice(), 100).unwrap();
        enc.push_frame(buf2.as_slice(), 100).unwrap();
        let output = enc.finish().unwrap();
        assert!(!output.is_empty());

        // Decode and verify frame count
        let dec_config = GifDecoderConfig::new();
        let mut frame_dec = dec_config
            .job()
            .frame_decoder(output.data(), &[])
            .unwrap();
        let mut count = 0;
        while frame_dec.next_frame().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2);
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

    #[test]
    fn frame_decoder_loop_count() {
        use zc::decode::{DecodeJob as _, FrameDecode as _};

        let dec = GifDecoderConfig::new();
        let frame_dec = dec.job().frame_decoder(MINIMAL_GIF, &[]).unwrap();
        // Minimal GIF has no NETSCAPE extension, so loop count depends on decoder default
        let lc = frame_dec.loop_count();
        assert!(lc.is_some());
    }

    #[test]
    fn capabilities_reported() {
        use zc::decode::DecoderConfig as _;
        use zc::encode::EncoderConfig as _;

        let enc_caps = GifEncoderConfig::capabilities();
        assert!(enc_caps.animation());
        assert!(enc_caps.cancel());
        assert!(enc_caps.native_alpha());

        let dec_caps = GifDecoderConfig::capabilities();
        assert!(dec_caps.animation());
        assert!(dec_caps.cheap_probe());
        assert!(dec_caps.cancel());
    }
}
