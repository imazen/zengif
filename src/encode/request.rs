//! Encode request builder.

use super::{Encoder, EncoderConfig};
use crate::{GifError, Limits, Result, types::FrameInput};
use enough::{Stop, Unstoppable};
use std::io::Write;
use whereat::at;

// Default instances for EncodeRequest::new()
static DEFAULT_LIMITS: Limits = Limits {
    max_width: Some(16384),
    max_height: Some(16384),
    max_total_pixels: Some(120_000_000),
    max_frame_count: Some(10_000),
    max_file_size: Some(100 * 1024 * 1024),
    max_memory: Some(1024 * 1024 * 1024),
    max_decompression_ratio: Some(1000.0),
    max_animation_ms: None,
    max_output_bytes: None,
};

static UNSTOPPABLE: Unstoppable = Unstoppable;

/// Request to encode a GIF animation.
///
/// Intermediate builder layer between [`EncoderConfig`] and [`Encoder`].
/// Binds configuration, dimensions, limits, and cancellation before building the encoder.
///
/// # Example
///
/// ```no_run
/// use zengif::{EncodeRequest, EncoderConfig, FrameInput, Limits, Rgba};
/// use enough::Unstoppable;
///
/// let config = EncoderConfig::new();
/// let limits = Limits::default();
///
/// let mut encoder = EncodeRequest::new(&config, 100, 100)
///     .limits(&limits)
///     .stop(&Unstoppable)
///     .build()?;
///
/// let frame = FrameInput::new(100, 100, 50, vec![Rgba::rgb(255, 0, 0); 10000]);
/// encoder.add_frame(frame)?;
/// let output = encoder.finish()?;
/// # Ok::<(), whereat::At<zengif::GifError>>(())
/// ```
pub struct EncodeRequest<'a> {
    pub(crate) config: &'a EncoderConfig,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) limits: &'a Limits,
    pub(crate) stop: &'a dyn Stop,
}

impl<'a> EncodeRequest<'a> {
    /// Create a new encode request.
    pub fn new(config: &'a EncoderConfig, width: u16, height: u16) -> Self {
        Self {
            config,
            width,
            height,
            limits: &DEFAULT_LIMITS,
            stop: &UNSTOPPABLE,
        }
    }

    /// Set limits for this encode operation.
    #[must_use]
    pub fn limits(mut self, limits: &'a Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Set stop token for cooperative cancellation.
    #[must_use]
    pub fn stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = stop;
        self
    }

    /// One-shot encode: encode all frames and return bytes.
    pub fn encode(self, frames: Vec<FrameInput>) -> Result<Vec<u8>> {
        let mut encoder = self.build()?;
        for frame in frames {
            encoder.add_frame(frame)?;
        }
        encoder.finish()
    }

    /// One-shot encode: encode all frames into provided buffer.
    pub fn encode_into(self, frames: Vec<FrameInput>, out: &mut Vec<u8>) -> Result<()> {
        let bytes = self.encode(frames)?;
        out.extend_from_slice(&bytes);
        Ok(())
    }

    /// One-shot encode: encode all frames to a writer.
    #[cfg(feature = "std")]
    pub fn encode_to<W: Write>(self, frames: Vec<FrameInput>, mut dest: W) -> Result<()> {
        let bytes = self.encode(frames)?;
        dest.write_all(&bytes).map_err(|e| at!(GifError::from(e)))?;
        Ok(())
    }

    /// Create a streaming encoder for frame-by-frame encoding.
    pub fn build(self) -> Result<Encoder<'a>> {
        Encoder::from_request(self)
    }
}
