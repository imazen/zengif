//! Configurable limits for GIF decoding and encoding.
//!
//! These limits protect against malicious or malformed inputs that could
//! cause excessive memory usage or processing time.

use crate::error::{GifError, Result};
use whereat::at;

/// Configuration for decode/encode limits.
///
/// All limits are optional; `None` means unlimited.
///
/// # Example
///
/// ```rust
/// use zengif::Limits;
///
/// // Start with defaults and customize
/// let limits = Limits::default()
///     .max_dimensions(4096, 4096)
///     .max_frame_count(100)
///     .max_memory(256 * 1024 * 1024);  // 256 MB
///
/// // Or start with no limits for trusted inputs
/// let unlimited = Limits::none();
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Limits {
    /// Maximum canvas width in pixels.
    pub max_width: Option<u16>,

    /// Maximum canvas height in pixels.
    pub max_height: Option<u16>,

    /// Maximum total pixels (width * height).
    /// Useful for limiting memory even with odd aspect ratios.
    pub max_total_pixels: Option<u64>,

    /// Maximum number of frames in an animation.
    pub max_frame_count: Option<u64>,

    /// Maximum input file size in bytes.
    pub max_file_size: Option<u64>,

    /// Maximum memory usage in bytes.
    /// Checked during allocation via Stats.
    pub max_memory: Option<u64>,

    /// Maximum decompression ratio (decompressed / compressed).
    /// Protection against zip bombs.
    pub max_decompression_ratio: Option<f64>,

    /// Maximum total animation duration in milliseconds.
    /// Checked cumulatively as frames are decoded or encoded.
    pub max_animation_ms: Option<u64>,

    /// Maximum encoded output size in bytes.
    /// Checked after encoding completes.
    pub max_output_bytes: Option<u64>,

    /// Per-site allocation-fallibility preference for the decode path.
    ///
    /// Internal carrier (`pub(crate)`): the `zencodec` decode path sets it from
    /// `zencodec::ResourceLimits::prefer_fallible_allocations`; the direct
    /// [`decode_gif`](crate::decode_gif) API and all other constructors leave it
    /// [`AllocPref::CodecDefault`](crate::alloc_util::AllocPref::CodecDefault),
    /// so each allocation site keeps its own default (big untrusted buffers
    /// fallible, small bounded scratch infallible). See [`crate::alloc_util`].
    pub(crate) alloc_pref: crate::alloc_util::AllocPref,
}

impl Default for Limits {
    /// Default limits suitable for server-side use.
    ///
    /// - Max dimensions: 16384 x 16384
    /// - Max total pixels: 120 megapixels
    /// - Max frames: 10,000
    /// - Max file size: 100 MB
    /// - Max memory: 1 GB
    /// - Max decompression ratio: 1000x
    /// - Max animation duration: none (unlimited)
    /// - Max output bytes: none (unlimited)
    fn default() -> Self {
        Self {
            max_width: Some(16384),
            max_height: Some(16384),
            max_total_pixels: Some(120_000_000),
            max_frame_count: Some(10_000),
            max_file_size: Some(100 * 1024 * 1024),
            max_memory: Some(1024 * 1024 * 1024),
            max_decompression_ratio: Some(1000.0),
            max_animation_ms: None,
            max_output_bytes: None,
            alloc_pref: crate::alloc_util::AllocPref::CodecDefault,
        }
    }
}

impl Limits {
    /// Create limits with no restrictions.
    ///
    /// **Warning**: Only use this for trusted inputs!
    pub fn none() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_total_pixels: None,
            max_frame_count: None,
            max_file_size: None,
            max_memory: None,
            max_decompression_ratio: None,
            max_animation_ms: None,
            max_output_bytes: None,
            alloc_pref: crate::alloc_util::AllocPref::CodecDefault,
        }
    }

    /// Set maximum dimensions.
    #[must_use]
    pub fn max_dimensions(mut self, width: u16, height: u16) -> Self {
        self.max_width = Some(width);
        self.max_height = Some(height);
        self
    }

    /// Set maximum total pixels.
    #[must_use]
    pub fn max_total_pixels(mut self, pixels: u64) -> Self {
        self.max_total_pixels = Some(pixels);
        self
    }

    /// Set maximum frame count.
    #[must_use]
    pub fn max_frame_count(mut self, count: u64) -> Self {
        self.max_frame_count = Some(count);
        self
    }

    /// Set maximum file size in bytes.
    #[must_use]
    pub fn max_file_size(mut self, bytes: u64) -> Self {
        self.max_file_size = Some(bytes);
        self
    }

    /// Set maximum memory usage in bytes.
    #[must_use]
    pub fn max_memory(mut self, bytes: u64) -> Self {
        self.max_memory = Some(bytes);
        self
    }

    /// Set maximum decompression ratio.
    #[must_use]
    pub fn max_decompression_ratio(mut self, ratio: f64) -> Self {
        self.max_decompression_ratio = Some(ratio);
        self
    }

    /// Set maximum total animation duration in milliseconds.
    #[must_use]
    pub fn max_animation_ms(mut self, ms: u64) -> Self {
        self.max_animation_ms = Some(ms);
        self
    }

    /// Set maximum encoded output size in bytes.
    #[must_use]
    pub fn max_output_bytes(mut self, bytes: u64) -> Self {
        self.max_output_bytes = Some(bytes);
        self
    }

    /// Check if dimensions are within limits.
    ///
    /// Rejects zero-width or zero-height images (always invalid).
    pub fn check_dimensions(&self, width: u16, height: u16) -> Result<()> {
        // Zero dimensions are always invalid — a 0x0 image is meaningless
        if width == 0 || height == 0 {
            return Err(at!(GifError::InvalidScreenDescriptor));
        }

        if let Some(max_w) = self.max_width
            && width > max_w
        {
            return Err(at!(GifError::DimensionsTooLarge {
                width,
                height,
                max_width: max_w,
                max_height: self.max_height.unwrap_or(u16::MAX),
            }));
        }

        if let Some(max_h) = self.max_height
            && height > max_h
        {
            return Err(at!(GifError::DimensionsTooLarge {
                width,
                height,
                max_width: self.max_width.unwrap_or(u16::MAX),
                max_height: max_h,
            }));
        }

        let total_pixels = width as u64 * height as u64;
        if let Some(max_pixels) = self.max_total_pixels
            && total_pixels > max_pixels
        {
            return Err(at!(GifError::TotalPixelsTooLarge {
                pixels: total_pixels,
                max_pixels,
            }));
        }

        Ok(())
    }

    /// Check if frame count is within limits.
    ///
    /// `count` is the 0-based index of the frame about to be added.
    /// So if max_frame_count is 1, we reject frame index 1 (the second frame).
    pub fn check_frame_count(&self, count: u64) -> Result<()> {
        if let Some(max) = self.max_frame_count
            && count >= max
        {
            return Err(at!(GifError::TooManyFrames { count, max }));
        }
        Ok(())
    }

    /// Check if file size is within limits.
    pub fn check_file_size(&self, size: u64) -> Result<()> {
        if let Some(max) = self.max_file_size
            && size > max
        {
            return Err(at!(GifError::FileTooLarge { size, max }));
        }
        Ok(())
    }

    /// Check if decompression ratio is within limits.
    pub fn check_decompression_ratio(&self, compressed: u64, decompressed: u64) -> Result<()> {
        if let Some(max_ratio) = self.max_decompression_ratio
            && compressed > 0
        {
            let ratio = decompressed as f64 / compressed as f64;
            if ratio > max_ratio {
                return Err(at!(GifError::DecompressionRatioExceeded {
                    compressed,
                    decompressed,
                    max_ratio,
                }));
            }
        }
        Ok(())
    }

    /// Check if cumulative animation duration is within limits.
    pub fn check_animation_duration(&self, duration_ms: u64) -> Result<()> {
        if let Some(max_ms) = self.max_animation_ms
            && duration_ms > max_ms
        {
            return Err(at!(GifError::AnimationTooLong {
                duration_ms,
                max_ms,
            }));
        }
        Ok(())
    }

    /// Check if encoded output size is within limits.
    pub fn check_output_bytes(&self, size: u64) -> Result<()> {
        if let Some(max) = self.max_output_bytes
            && size > max
        {
            return Err(at!(GifError::OutputTooLarge { size, max }));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits() {
        let limits = Limits::default();
        assert!(limits.max_width.is_some());
        assert!(limits.max_height.is_some());
    }

    #[test]
    fn check_dimensions_ok() {
        let limits = Limits::default().max_dimensions(1000, 1000);
        assert!(limits.check_dimensions(500, 500).is_ok());
        assert!(limits.check_dimensions(1000, 1000).is_ok());
    }

    #[test]
    fn check_dimensions_too_large() {
        let limits = Limits::default().max_dimensions(1000, 1000);
        let result = limits.check_dimensions(1001, 500);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error(),
            GifError::DimensionsTooLarge { .. }
        ));
    }

    #[test]
    fn check_total_pixels() {
        let limits = Limits::default().max_total_pixels(1_000_000);
        assert!(limits.check_dimensions(1000, 1000).is_ok());
        assert!(limits.check_dimensions(1001, 1000).is_err());
    }

    #[test]
    fn check_decompression_ratio() {
        let limits = Limits::default().max_decompression_ratio(100.0);
        assert!(limits.check_decompression_ratio(100, 5000).is_ok()); // 50x
        assert!(limits.check_decompression_ratio(100, 15000).is_err()); // 150x
    }

    #[test]
    fn no_limits() {
        let limits = Limits::none();
        assert!(limits.check_dimensions(u16::MAX, u16::MAX).is_ok());
        assert!(limits.check_frame_count(u64::MAX).is_ok());
    }

    #[test]
    fn builder_pattern() {
        let limits = Limits::default()
            .max_dimensions(4096, 4096)
            .max_frame_count(100)
            .max_file_size(10 * 1024 * 1024)
            .max_memory(512 * 1024 * 1024);

        assert_eq!(limits.max_width, Some(4096));
        assert_eq!(limits.max_frame_count, Some(100));
    }
}
