//! Error types for zengif with whereat integration for production tracing.

#[cfg(not(feature = "std"))]
use alloc::string::String;

use whereat::At;

/// Result type alias using `At<GifError>` for automatic location tracking.
pub type Result<T> = core::result::Result<T, At<GifError>>;

/// Type alias for encoding errors (for API clarity).
pub type EncodeError = GifError;

/// Type alias for decoding errors (for API clarity).
pub type DecodeError = GifError;

#[cfg(feature = "std")]
fn io_display(kind: std::io::ErrorKind, context: Option<&'static str>) -> String {
    match context {
        Some(ctx) => format!("I/O error ({kind:?}): {ctx}"),
        None => format!("I/O error: {kind:?}"),
    }
}

#[cfg(not(feature = "std"))]
fn io_display_no_std(context: Option<&'static str>) -> &'static str {
    match context {
        Some(ctx) => ctx,
        None => "I/O error",
    }
}

/// All possible errors in zengif operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GifError {
    // === Header/Format Errors ===
    /// Invalid GIF header (not GIF87a or GIF89a).
    #[error("invalid GIF header")]
    InvalidHeader,

    /// Unsupported GIF version.
    #[error("unsupported GIF version: {}", core::str::from_utf8(version).unwrap_or("???"))]
    UnsupportedVersion {
        /// The version bytes found.
        version: [u8; 3],
    },

    /// Invalid logical screen descriptor.
    #[error("invalid logical screen descriptor")]
    InvalidScreenDescriptor,

    // === Frame Errors ===
    /// Frame bounds exceed canvas size.
    #[error(
        "frame bounds ({frame_left}, {frame_top}, {frame_width}x{frame_height}) exceed canvas size ({canvas_width}x{canvas_height})"
    )]
    InvalidFrameBounds {
        /// Frame left position.
        frame_left: u16,
        /// Frame top position.
        frame_top: u16,
        /// Frame width.
        frame_width: u16,
        /// Frame height.
        frame_height: u16,
        /// Canvas width.
        canvas_width: u16,
        /// Canvas height.
        canvas_height: u16,
    },

    /// Frame is missing a required color palette.
    #[error("frame {frame_index} is missing color palette")]
    MissingPalette {
        /// Frame index.
        frame_index: usize,
    },

    /// Invalid disposal method value.
    #[error("invalid disposal method value: {value}")]
    InvalidDisposalMethod {
        /// The invalid value.
        value: u8,
    },

    // === LZW/Decompression Errors ===
    /// Malformed LZW data.
    #[error("malformed LZW data: {message}")]
    MalformedLzw {
        /// Description of the error.
        message: &'static str,
    },

    /// LZW minimum code size is invalid.
    #[error("invalid LZW minimum code size: {value}")]
    InvalidMinCodeSize {
        /// The invalid value.
        value: u8,
    },

    // === Limit Errors ===
    /// Image dimensions exceed configured limits.
    #[error("dimensions {width}x{height} exceed limit {max_width}x{max_height}")]
    DimensionsTooLarge {
        /// Actual width.
        width: u16,
        /// Actual height.
        height: u16,
        /// Maximum allowed width.
        max_width: u16,
        /// Maximum allowed height.
        max_height: u16,
    },

    /// Total pixel count exceeds limit.
    #[error("total pixels {pixels} exceeds limit {max_pixels}")]
    TotalPixelsTooLarge {
        /// Actual pixel count.
        pixels: u64,
        /// Maximum allowed.
        max_pixels: u64,
    },

    /// Too many frames in animation.
    #[error("frame count {count} exceeds limit {max}")]
    TooManyFrames {
        /// Actual frame count.
        count: u64,
        /// Maximum allowed.
        max: u64,
    },

    /// File size exceeds limit.
    #[error("file size {size} bytes exceeds limit {max} bytes")]
    FileTooLarge {
        /// Actual size in bytes.
        size: u64,
        /// Maximum allowed in bytes.
        max: u64,
    },

    /// Memory limit exceeded during operation.
    #[error("memory usage {current} bytes exceeds limit {limit} bytes")]
    MemoryLimitExceeded {
        /// Current memory usage in bytes.
        current: u64,
        /// Configured limit in bytes.
        limit: u64,
    },

    /// Allocation failed.
    #[error("allocation of {requested} bytes failed")]
    AllocationFailed {
        /// Requested size in bytes.
        requested: u64,
    },

    /// Decompression ratio exceeded (potential zip bomb).
    #[error("decompression ratio {ratio:.1}x exceeds limit {max_ratio:.1}x", ratio = *decompressed as f64 / (*compressed).max(1) as f64)]
    DecompressionRatioExceeded {
        /// Compressed size.
        compressed: u64,
        /// Decompressed size.
        decompressed: u64,
        /// Maximum allowed ratio.
        max_ratio: f64,
    },

    /// Animation duration exceeds limit.
    #[error("animation duration {duration_ms}ms exceeds limit {max_ms}ms")]
    AnimationTooLong {
        /// Actual cumulative duration in milliseconds.
        duration_ms: u64,
        /// Maximum allowed duration in milliseconds.
        max_ms: u64,
    },

    /// Encoded output size exceeds limit.
    #[error("output size {size} bytes exceeds limit {max} bytes")]
    OutputTooLarge {
        /// Actual output size in bytes.
        size: u64,
        /// Maximum allowed output size in bytes.
        max: u64,
    },

    // === Encoding Errors ===
    /// Frame dimensions don't match encoder canvas.
    #[error(
        "frame size {actual_width}x{actual_height} doesn't match expected {expected_width}x{expected_height}"
    )]
    FrameDimensionMismatch {
        /// Expected width.
        expected_width: u16,
        /// Expected height.
        expected_height: u16,
        /// Actual width.
        actual_width: u16,
        /// Actual height.
        actual_height: u16,
    },

    /// Color quantization failed.
    #[error("color quantization failed: {message}")]
    QuantizationFailed {
        /// Description of the error.
        message: &'static str,
    },

    /// Encoder is in an invalid state.
    #[error("encoder in invalid state: {message}")]
    InvalidEncoderState {
        /// Description of the state.
        message: &'static str,
    },

    // === I/O Errors ===
    /// Unexpected end of file.
    #[error("unexpected end of file")]
    UnexpectedEof,

    /// I/O error during read or write.
    #[cfg(feature = "std")]
    #[error("{}", io_display(*.kind, *context))]
    Io {
        /// The underlying I/O error kind.
        kind: std::io::ErrorKind,
        /// Optional context message.
        context: Option<&'static str>,
    },

    /// I/O error during read or write (no_std version).
    #[cfg(not(feature = "std"))]
    #[error("{}", io_display_no_std(*context))]
    Io {
        /// Optional context message.
        context: Option<&'static str>,
    },

    // === Cancellation ===
    /// Operation was cancelled via Stop trait.
    #[error("operation cancelled")]
    Cancelled,

    // === Wrapped Errors ===
    /// Error from underlying gif crate.
    #[error("gif crate error: {message}")]
    GifCrate {
        /// Description of the gif crate error.
        message: String,
    },

    /// Unsupported codec operation.
    #[cfg(feature = "zencodec")]
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(zencodec::UnsupportedOperation),
}

// Conversion from std::io::Error
#[cfg(feature = "std")]
impl From<std::io::Error> for GifError {
    fn from(err: std::io::Error) -> Self {
        GifError::Io {
            kind: err.kind(),
            context: None,
        }
    }
}

// Conversion from gif crate errors (only available with std feature,
// since gif crate's error types require std)
#[cfg(feature = "std")]
impl From<gif::DecodingError> for GifError {
    fn from(err: gif::DecodingError) -> Self {
        use gif::DecodingError;
        match err {
            DecodingError::Format(msg) => GifError::GifCrate {
                message: msg.to_string(),
            },
            DecodingError::Io(io_err) => GifError::Io {
                kind: io_err.kind(),
                context: Some("during GIF decoding"),
            },
            DecodingError::UnexpectedEof => GifError::UnexpectedEof,
            DecodingError::OutOfMemory | DecodingError::MemoryLimit => {
                GifError::AllocationFailed { requested: 0 }
            }
            DecodingError::LzwError(e) => GifError::GifCrate {
                message: e.to_string(),
            },
            DecodingError::EndCodeNotFound => GifError::GifCrate {
                message: "LZW end code not found".to_string(),
            },
            DecodingError::DecoderNotFound => GifError::GifCrate {
                message: "decoder not found".to_string(),
            },
            // Handle future variants of non-exhaustive enum
            #[allow(unreachable_patterns)]
            _ => GifError::GifCrate {
                message: "unknown gif decoding error".to_string(),
            },
        }
    }
}

#[cfg(feature = "std")]
impl From<gif::EncodingError> for GifError {
    fn from(err: gif::EncodingError) -> Self {
        use gif::EncodingError;
        match err {
            EncodingError::Format(msg) => GifError::GifCrate {
                message: msg.to_string(),
            },
            EncodingError::Io(io_err) => GifError::Io {
                kind: io_err.kind(),
                context: Some("during GIF encoding"),
            },
            // Handle future variants of non-exhaustive enum
            #[allow(unreachable_patterns)]
            _ => GifError::GifCrate {
                message: "unknown gif encoding error".to_string(),
            },
        }
    }
}

// Allow whereat to wrap our errors
impl From<enough::StopReason> for GifError {
    fn from(_: enough::StopReason) -> Self {
        GifError::Cancelled
    }
}

#[cfg(feature = "zencodec")]
impl From<zencodec::UnsupportedOperation> for GifError {
    fn from(op: zencodec::UnsupportedOperation) -> Self {
        GifError::UnsupportedOperation(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "std"))]
    use alloc::string::ToString;

    #[test]
    fn error_display() {
        let err = GifError::DimensionsTooLarge {
            width: 10000,
            height: 10000,
            max_width: 4096,
            max_height: 4096,
        };
        assert!(err.to_string().contains("10000x10000"));
        assert!(err.to_string().contains("4096x4096"));
    }

    #[test]
    fn error_with_whereat() {
        use whereat::at;

        fn inner() -> Result<()> {
            Err(at!(GifError::InvalidHeader))
        }

        fn outer() -> Result<()> {
            inner().map_err(|e| e.at())?;
            Ok(())
        }

        let err = outer().unwrap_err();
        // Should have 2 frames: inner() and outer()
        assert!(err.frame_count() >= 1);
    }
}
