//! Error types for zengif with whereat integration for production tracing.

#[cfg(not(feature = "std"))]
use alloc::string::String;

use core::fmt;

use whereat::At;

/// Result type alias using `At<GifError>` for automatic location tracking.
pub type Result<T> = core::result::Result<T, At<GifError>>;

/// Type alias for encoding errors (for API clarity).
pub type EncodeError = GifError;

/// Type alias for decoding errors (for API clarity).
pub type DecodeError = GifError;

/// All possible errors in zengif operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum GifError {
    // === Header/Format Errors ===
    /// Invalid GIF header (not GIF87a or GIF89a).
    InvalidHeader,

    /// Unsupported GIF version.
    UnsupportedVersion {
        /// The version bytes found.
        version: [u8; 3],
    },

    /// Invalid logical screen descriptor.
    InvalidScreenDescriptor,

    // === Frame Errors ===
    /// Frame bounds exceed canvas size.
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
    MissingPalette {
        /// Frame index.
        frame_index: usize,
    },

    /// Invalid disposal method value.
    InvalidDisposalMethod {
        /// The invalid value.
        value: u8,
    },

    // === LZW/Decompression Errors ===
    /// Malformed LZW data.
    MalformedLzw {
        /// Description of the error.
        message: &'static str,
    },

    /// LZW minimum code size is invalid.
    InvalidMinCodeSize {
        /// The invalid value.
        value: u8,
    },

    // === Limit Errors ===
    /// Image dimensions exceed configured limits.
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
    TotalPixelsTooLarge {
        /// Actual pixel count.
        pixels: u64,
        /// Maximum allowed.
        max_pixels: u64,
    },

    /// Too many frames in animation.
    TooManyFrames {
        /// Actual frame count.
        count: u64,
        /// Maximum allowed.
        max: u64,
    },

    /// File size exceeds limit.
    FileTooLarge {
        /// Actual size in bytes.
        size: u64,
        /// Maximum allowed in bytes.
        max: u64,
    },

    /// Memory limit exceeded during operation.
    MemoryLimitExceeded {
        /// Current memory usage in bytes.
        current: u64,
        /// Configured limit in bytes.
        limit: u64,
    },

    /// Allocation failed.
    AllocationFailed {
        /// Requested size in bytes.
        requested: u64,
    },

    /// Decompression ratio exceeded (potential zip bomb).
    DecompressionRatioExceeded {
        /// Compressed size.
        compressed: u64,
        /// Decompressed size.
        decompressed: u64,
        /// Maximum allowed ratio.
        max_ratio: f64,
    },

    // === Encoding Errors ===
    /// Frame dimensions don't match encoder canvas.
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
    QuantizationFailed {
        /// Description of the error.
        message: &'static str,
    },

    /// Encoder is in an invalid state.
    InvalidEncoderState {
        /// Description of the state.
        message: &'static str,
    },

    // === I/O Errors ===
    /// Unexpected end of file.
    UnexpectedEof,

    /// I/O error during read or write.
    #[cfg(feature = "std")]
    Io {
        /// The underlying I/O error kind.
        kind: std::io::ErrorKind,
        /// Optional context message.
        context: Option<&'static str>,
    },

    /// I/O error during read or write (no_std version).
    #[cfg(not(feature = "std"))]
    Io {
        /// Optional context message.
        context: Option<&'static str>,
    },

    // === Cancellation ===
    /// Operation was cancelled via Stop trait.
    Cancelled,

    // === Wrapped Errors ===
    /// Error from underlying gif crate.
    GifCrate {
        /// Description of the gif crate error.
        message: String,
    },

    /// Unsupported codec operation.
    UnsupportedOperation(zencodec_types::UnsupportedOperation),
}

impl fmt::Display for GifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GifError::InvalidHeader => write!(f, "invalid GIF header"),
            GifError::UnsupportedVersion { version } => {
                write!(
                    f,
                    "unsupported GIF version: {:?}",
                    core::str::from_utf8(version).unwrap_or("???")
                )
            }
            GifError::InvalidScreenDescriptor => write!(f, "invalid logical screen descriptor"),
            GifError::InvalidFrameBounds {
                frame_left,
                frame_top,
                frame_width,
                frame_height,
                canvas_width,
                canvas_height,
            } => {
                write!(
                    f,
                    "frame bounds ({}, {}, {}x{}) exceed canvas size ({}x{})",
                    frame_left, frame_top, frame_width, frame_height, canvas_width, canvas_height
                )
            }
            GifError::MissingPalette { frame_index } => {
                write!(f, "frame {} is missing color palette", frame_index)
            }
            GifError::InvalidDisposalMethod { value } => {
                write!(f, "invalid disposal method value: {}", value)
            }
            GifError::MalformedLzw { message } => write!(f, "malformed LZW data: {}", message),
            GifError::InvalidMinCodeSize { value } => {
                write!(f, "invalid LZW minimum code size: {}", value)
            }
            GifError::DimensionsTooLarge {
                width,
                height,
                max_width,
                max_height,
            } => {
                write!(
                    f,
                    "dimensions {}x{} exceed limit {}x{}",
                    width, height, max_width, max_height
                )
            }
            GifError::TotalPixelsTooLarge { pixels, max_pixels } => {
                write!(f, "total pixels {} exceeds limit {}", pixels, max_pixels)
            }
            GifError::TooManyFrames { count, max } => {
                write!(f, "frame count {} exceeds limit {}", count, max)
            }
            GifError::FileTooLarge { size, max } => {
                write!(f, "file size {} bytes exceeds limit {} bytes", size, max)
            }
            GifError::MemoryLimitExceeded { current, limit } => {
                write!(
                    f,
                    "memory usage {} bytes exceeds limit {} bytes",
                    current, limit
                )
            }
            GifError::AllocationFailed { requested } => {
                write!(f, "allocation of {} bytes failed", requested)
            }
            GifError::DecompressionRatioExceeded {
                compressed,
                decompressed,
                max_ratio,
            } => {
                let ratio = *decompressed as f64 / (*compressed).max(1) as f64;
                write!(
                    f,
                    "decompression ratio {:.1}x exceeds limit {:.1}x",
                    ratio, max_ratio
                )
            }
            GifError::FrameDimensionMismatch {
                expected_width,
                expected_height,
                actual_width,
                actual_height,
            } => {
                write!(
                    f,
                    "frame size {}x{} doesn't match expected {}x{}",
                    actual_width, actual_height, expected_width, expected_height
                )
            }
            GifError::QuantizationFailed { message } => {
                write!(f, "color quantization failed: {}", message)
            }
            GifError::InvalidEncoderState { message } => {
                write!(f, "encoder in invalid state: {}", message)
            }
            GifError::UnexpectedEof => write!(f, "unexpected end of file"),
            #[cfg(feature = "std")]
            GifError::Io { kind, context } => {
                if let Some(ctx) = context {
                    write!(f, "I/O error ({:?}): {}", kind, ctx)
                } else {
                    write!(f, "I/O error: {:?}", kind)
                }
            }
            #[cfg(not(feature = "std"))]
            GifError::Io { context } => {
                if let Some(ctx) = context {
                    write!(f, "I/O error: {}", ctx)
                } else {
                    write!(f, "I/O error")
                }
            }
            GifError::Cancelled => write!(f, "operation cancelled"),
            GifError::GifCrate { message } => write!(f, "gif crate error: {}", message),
            GifError::UnsupportedOperation(op) => write!(f, "unsupported operation: {}", op),
        }
    }
}

impl core::error::Error for GifError {}

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

impl From<zencodec_types::UnsupportedOperation> for GifError {
    fn from(op: zencodec_types::UnsupportedOperation) -> Self {
        GifError::UnsupportedOperation(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
