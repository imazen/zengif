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

    /// Frame has zero width or height (invalid, would cause infinite decode loop).
    #[error("frame {frame_index} has zero dimension ({frame_width}x{frame_height})")]
    ZeroDimensionFrame {
        /// Frame index.
        frame_index: usize,
        /// Frame width.
        frame_width: u16,
        /// Frame height.
        frame_height: u16,
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
    /// Operation was cancelled via a [`enough::Stop`] token.
    ///
    /// Carries the [`enough::StopReason`] so callers (and the
    /// [`CategorizedError`](zencodec::CategorizedError) mapping below) can
    /// distinguish an explicit cancellation from a timeout instead of
    /// collapsing both into one undifferentiated "cancelled" state.
    #[error("operation cancelled: {0}")]
    Cancelled(enough::StopReason),

    // === Wrapped Errors ===
    /// Error from underlying gif crate (malformed bitstream content).
    #[error("gif crate error: {message}")]
    GifCrate {
        /// Description of the gif crate error.
        message: String,
    },

    /// A caller-supplied decode row-sink rejected a write while decoding into it.
    ///
    /// This is an output-side failure (the sink could not accept the decoded
    /// rows), not malformed input — distinct from [`GifError::GifCrate`].
    #[cfg(feature = "std")]
    #[error("decode sink write failed: {message}")]
    SinkWrite {
        /// Description of the sink error.
        message: String,
    },

    /// Unsupported codec operation.
    #[cfg(feature = "std")]
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
            // An incomplete/truncated LZW stream missing its terminator — this is
            // truncation, not corrupt bitstream *content*, so it must categorize
            // the same way `UnexpectedEof` does (never as opaque `GifCrate`/
            // MalformedImage, which would misattribute a short read as corrupt
            // input instead of incomplete input).
            DecodingError::EndCodeNotFound => GifError::UnexpectedEof,
            // Real allocator failure (the `gif` crate could not internally
            // allocate a buffer) — distinct from a *configured* memory cap below.
            DecodingError::OutOfMemory => GifError::AllocationFailed { requested: 0 },
            // The `gif` crate's own `set_memory_limit()` cap was tripped — a
            // configured ceiling, not a true allocation failure. Neither variant
            // carries the byte counts, so both use `0` as the existing
            // `AllocationFailed`/`MemoryLimitExceeded` sentinel pattern already
            // does elsewhere in this crate.
            DecodingError::MemoryLimit => GifError::MemoryLimitExceeded {
                current: 0,
                limit: 0,
            },
            DecodingError::LzwError(e) => GifError::GifCrate {
                message: e.to_string(),
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
    fn from(reason: enough::StopReason) -> Self {
        GifError::Cancelled(reason)
    }
}

#[cfg(feature = "std")]
impl From<zencodec::UnsupportedOperation> for GifError {
    fn from(op: zencodec::UnsupportedOperation) -> Self {
        GifError::UnsupportedOperation(op)
    }
}

// Codec-agnostic error taxonomy (zencodec PR #103, reshaped to the two-level
// origin-first taxonomy by PR #116). Maps every `GifError` variant to exactly
// one coarse `ErrorCategory` so consumers can route on the category (HTTP
// status, retry policy, logging) without naming this enum.
#[cfg(feature = "std")]
impl zencodec::CategorizedError for GifError {
    fn codec_name(&self) -> Option<&'static str> {
        Some("zengif")
    }

    fn category(&self) -> zencodec::ErrorCategory {
        use zencodec::ErrorCategory as C;
        use zencodec::ImageError as Img;
        use zencodec::InternalKind as Int;
        use zencodec::InvalidKind as Inv;
        use zencodec::LimitKind as L;
        use zencodec::RequestError as Req;
        use zencodec::ResourceError as Res;
        use zencodec::UnsupportedImageKind as UImg;
        match self {
            // === Malformed / corrupt bitstream content ===
            GifError::InvalidHeader
            | GifError::InvalidScreenDescriptor
            | GifError::InvalidFrameBounds { .. }
            | GifError::ZeroDimensionFrame { .. }
            | GifError::MissingPalette { .. }
            | GifError::InvalidDisposalMethod { .. }
            | GifError::MalformedLzw { .. }
            | GifError::InvalidMinCodeSize { .. }
            | GifError::GifCrate { .. } => C::Image(Img::Malformed),

            // === Caller-invocation fault: a caller-supplied pixel buffer whose
            // declared dimensions don't match the encoder's canvas. The bytes
            // aren't the problem — the caller passed the wrong-shaped buffer —
            // so this is a Request-origin fault, not an Image-origin one.
            GifError::FrameDimensionMismatch { .. } => C::Request(Req::Invalid(Inv::Buffer)),

            // === Truncated input ===
            GifError::UnexpectedEof => C::Image(Img::UnexpectedEof),

            // === Format/version not handled at all ===
            GifError::UnsupportedVersion { .. } => C::Image(Img::Unsupported(UImg::Type)),

            // === Resource limits (pick the closest LimitKind) ===
            // Per-dimension width/height cap. Attribute to whichever axis
            // actually violated its own configured max (the `check_dimensions`
            // construction sites in `limits.rs` only ever populate one side of
            // this strictly, since the first violated check returns early), so
            // the *real* offending dimension survives instead of always
            // reading Width. The tie-break arms below cover the pathological
            // `PixelSlice`→u16 overflow-clamp construction in `codec.rs`, where
            // both `max_width`/`max_height` are clamped to the same `u16::MAX`
            // sentinel regardless of which axis actually overflowed: there the
            // offending axis is pinned exactly at that ceiling while the other
            // axis is comfortably under it.
            GifError::DimensionsTooLarge {
                width,
                height,
                max_width,
                max_height,
            } => {
                if width > max_width {
                    C::Resource(Res::Limits(L::Width))
                } else if height > max_height {
                    C::Resource(Res::Limits(L::Height))
                } else if width == max_width && height < max_height {
                    C::Resource(Res::Limits(L::Width))
                } else if height == max_height && width < max_width {
                    C::Resource(Res::Limits(L::Height))
                } else {
                    // Fully ambiguous (both axes genuinely tied at their max, or
                    // neither) — arbitrary default, matches historical behavior.
                    C::Resource(Res::Limits(L::Width))
                }
            }
            GifError::TotalPixelsTooLarge { .. } => C::Resource(Res::Limits(L::TotalPixels)),
            GifError::TooManyFrames { .. } => C::Resource(Res::Limits(L::Frames)),
            GifError::FileTooLarge { .. } => C::Resource(Res::Limits(L::InputSize)),
            GifError::MemoryLimitExceeded { .. } => C::Resource(Res::Limits(L::Memory)),
            // Zip-bomb guard: routed directly to the dedicated DecompressionRatio
            // kind (added alongside this taxonomy reshape) instead of the
            // closest-fit Memory kind, so an anti-DoS decompression-bomb signal
            // is distinguishable from an absolute memory-budget cap.
            GifError::DecompressionRatioExceeded { .. } => {
                C::Resource(Res::Limits(L::DecompressionRatio))
            }
            GifError::AnimationTooLong { .. } => C::Resource(Res::Limits(L::Duration)),
            GifError::OutputTooLarge { .. } => C::Resource(Res::Limits(L::OutputSize)),

            // === Allocation failure (distinct from a configured limit) ===
            GifError::AllocationFailed { .. } => C::Resource(Res::OutOfMemory),

            // === I/O and output-sink failures ===
            // A truncated input stream surfaces as an `UnexpectedEof` io kind
            // (e.g. `read_exact` past the end of a short slice). That is
            // incomplete client input, so it must categorize as image-origin
            // `UnexpectedEof` — never `Io`, which would misattribute truncation
            // as an infrastructure/codec fault (5xx) instead of a
            // malformed-request (4xx) condition. Other io kinds carry their
            // real `std::io::ErrorKind` through `CodecIoKind` instead of
            // collapsing to opaque.
            GifError::Io { kind, .. } => match kind {
                std::io::ErrorKind::UnexpectedEof => C::Image(Img::UnexpectedEof),
                _ => C::Io((*kind).into()),
            },
            GifError::SinkWrite { .. } => C::Io(zencodec::CodecIoKind::opaque()),

            // === Cancellation ===
            // `Lifecycle` carries the `StopReason` itself now, so an explicit
            // cancellation and a timeout are distinguishable via the payload
            // without a separate match here — no lossy collapse, and any future
            // `StopReason` variant (it is `#[non_exhaustive]`) flows through
            // unchanged.
            GifError::Cancelled(reason) => C::Lifecycle(*reason),

            // === Caller API-protocol violations ===
            GifError::InvalidEncoderState { .. } => C::Request(Req::Invalid(Inv::State)),

            // === Internal failures ===
            // Quantization failure originates in an external quantizer backend
            // (imagequant/quantizr/color_quant/zenquant) this codec doesn't
            // control — an unclassified dependency failure, not a broken
            // invariant in zengif's own logic.
            GifError::QuantizationFailed { .. } => C::Internal(Int::Dependency),

            // === Delegate to the zencodec cause type ===
            GifError::UnsupportedOperation(op) => op.category(),
        }
    }
}

/// Bridge `GifError` into the shared [`CodecError`](zencodec::CodecError)
/// envelope (zencodec PR #103, "Pattern B").
///
/// zengif's own native API keeps `At<GifError>`; the zencodec **trait** impls
/// (see `crate::codec`) return `At<CodecError>` so a generic consumer recovers
/// the [`ErrorCategory`](zencodec::ErrorCategory) *and* the codec name even
/// after `Dyn*` dispatch erases the concrete error to `Box<dyn Error>`.
///
/// `.start_at()` begins the location trace; [`CodecError::of`](zencodec::CodecError::of)
/// then maps the located `At<GifError>` to `At<CodecError>`, keeping the trace on
/// the outside and reading the category *and* `codec_name()` from the `GifError`
/// value — which becomes the envelope's retained detail. With this in place, `?`
/// on any `Result<_, GifError>` auto-wraps into the envelope.
#[cfg(feature = "std")]
impl From<GifError> for At<zencodec::CodecError> {
    #[track_caller]
    fn from(e: GifError) -> Self {
        use whereat::ErrorAtExt;
        zencodec::CodecError::of(e.start_at())
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

    #[cfg(feature = "std")]
    #[test]
    fn error_category_mapping() {
        use zencodec::{
            CategorizedError, ErrorCategory as C, ImageError as Img, InternalKind as Int,
            InvalidKind as Inv, LimitKind as L, RequestError as Req, ResourceError as Res,
        };

        assert_eq!(GifError::InvalidHeader.codec_name(), Some("zengif"));

        // Malformed bitstream content.
        assert_eq!(GifError::InvalidHeader.category(), C::Image(Img::Malformed));
        assert_eq!(
            GifError::InvalidScreenDescriptor.category(),
            C::Image(Img::Malformed)
        );
        assert_eq!(
            GifError::MalformedLzw { message: "bad" }.category(),
            C::Image(Img::Malformed)
        );
        assert_eq!(
            GifError::GifCrate {
                message: "x".into()
            }
            .category(),
            C::Image(Img::Malformed)
        );

        // Truncated input.
        assert_eq!(
            GifError::UnexpectedEof.category(),
            C::Image(Img::UnexpectedEof)
        );

        // Unhandled format/version.
        assert_eq!(
            GifError::UnsupportedVersion { version: *b"GIF" }.category(),
            C::Image(Img::Unsupported(zencodec::UnsupportedImageKind::Type))
        );

        // Caller-invocation fault: wrong-geometry pixel buffer handed to the
        // encoder — a Request-origin fault, not Image-origin.
        assert_eq!(
            GifError::FrameDimensionMismatch {
                expected_width: 4,
                expected_height: 4,
                actual_width: 9,
                actual_height: 9,
            }
            .category(),
            C::Request(Req::Invalid(Inv::Buffer))
        );

        // Resource limits map to the closest LimitKind. Width-vs-height
        // detection (audit finding #3): whichever axis actually violated its
        // own configured max is attributed — not always Width.
        assert_eq!(
            GifError::DimensionsTooLarge {
                width: 9,
                height: 9,
                max_width: 4,
                max_height: 4,
            }
            .category(),
            C::Resource(Res::Limits(L::Width))
        );
        // Height-only violation: width is within its own max (4 <= 4); only
        // height exceeds (9 > 4) — must categorize as Height, never Width.
        assert_eq!(
            GifError::DimensionsTooLarge {
                width: 4,
                height: 9,
                max_width: 4,
                max_height: 4,
            }
            .category(),
            C::Resource(Res::Limits(L::Height))
        );
        // Tie at the same ceiling (mirrors the PixelSlice->u16 overflow-clamp
        // construction in codec.rs): the pinned axis with the other
        // comfortably under its own max is attributed.
        assert_eq!(
            GifError::DimensionsTooLarge {
                width: u16::MAX,
                height: 100,
                max_width: u16::MAX,
                max_height: u16::MAX,
            }
            .category(),
            C::Resource(Res::Limits(L::Width))
        );
        assert_eq!(
            GifError::DimensionsTooLarge {
                width: 100,
                height: u16::MAX,
                max_width: u16::MAX,
                max_height: u16::MAX,
            }
            .category(),
            C::Resource(Res::Limits(L::Height))
        );
        assert_eq!(
            GifError::TotalPixelsTooLarge {
                pixels: 9,
                max_pixels: 4,
            }
            .category(),
            C::Resource(Res::Limits(L::TotalPixels))
        );
        assert_eq!(
            GifError::TooManyFrames { count: 9, max: 4 }.category(),
            C::Resource(Res::Limits(L::Frames))
        );
        assert_eq!(
            GifError::FileTooLarge { size: 9, max: 4 }.category(),
            C::Resource(Res::Limits(L::InputSize))
        );
        assert_eq!(
            GifError::MemoryLimitExceeded {
                current: 9,
                limit: 4,
            }
            .category(),
            C::Resource(Res::Limits(L::Memory))
        );
        // Decompression-ratio bomb guard (audit finding #6): routed to the
        // dedicated DecompressionRatio kind, not the closest-fit Memory kind.
        assert_eq!(
            GifError::DecompressionRatioExceeded {
                compressed: 1,
                decompressed: 99,
                max_ratio: 10.0,
            }
            .category(),
            C::Resource(Res::Limits(L::DecompressionRatio))
        );
        assert_eq!(
            GifError::AnimationTooLong {
                duration_ms: 9,
                max_ms: 4,
            }
            .category(),
            C::Resource(Res::Limits(L::Duration))
        );
        assert_eq!(
            GifError::OutputTooLarge { size: 9, max: 4 }.category(),
            C::Resource(Res::Limits(L::OutputSize))
        );

        // Allocation, sink, state, internal.
        assert_eq!(
            GifError::AllocationFailed { requested: 9 }.category(),
            C::Resource(Res::OutOfMemory)
        );
        assert_eq!(
            GifError::SinkWrite {
                message: "x".into()
            }
            .category(),
            C::Io(zencodec::CodecIoKind::opaque())
        );
        assert_eq!(
            GifError::InvalidEncoderState { message: "x" }.category(),
            C::Request(Req::Invalid(Inv::State))
        );
        assert_eq!(
            GifError::QuantizationFailed { message: "x" }.category(),
            C::Internal(Int::Dependency)
        );
        assert_eq!(
            GifError::Cancelled(enough::StopReason::Cancelled).category(),
            C::Lifecycle(enough::StopReason::Cancelled)
        );
        assert_eq!(
            GifError::Cancelled(enough::StopReason::TimedOut).category(),
            C::Lifecycle(enough::StopReason::TimedOut)
        );

        // Delegated zencodec cause type.
        assert_eq!(
            GifError::UnsupportedOperation(zencodec::UnsupportedOperation::AnimationEncode)
                .category(),
            C::Request(Req::Unsupported(
                zencodec::UnsupportedOperation::AnimationEncode
            ))
        );

        // The `At<E>` blanket impl forwards the category and codec name.
        let traced = whereat::at!(GifError::InvalidHeader);
        assert_eq!(traced.category(), C::Image(Img::Malformed));
    }

    /// `gif::DecodingError` variant-by-variant mapping (audit findings #1, #2):
    /// `EndCodeNotFound` (an incomplete LZW stream) must categorize as
    /// truncation, never opaque malformed; `OutOfMemory` (a real allocator
    /// failure) and `MemoryLimit` (the `gif` crate's own configured cap) must
    /// land on distinct `GifError` variants instead of collapsing together.
    #[cfg(feature = "std")]
    #[test]
    fn gif_decoding_error_conversion_distinguishes_causes() {
        use zencodec::{
            CategorizedError, ErrorCategory as C, ImageError as Img, LimitKind as L,
            ResourceError as Res,
        };

        let end_code: GifError = gif::DecodingError::EndCodeNotFound.into();
        assert!(matches!(end_code, GifError::UnexpectedEof));
        assert_eq!(end_code.category(), C::Image(Img::UnexpectedEof));

        let real_oom: GifError = gif::DecodingError::OutOfMemory.into();
        assert!(matches!(real_oom, GifError::AllocationFailed { .. }));
        assert_eq!(real_oom.category(), C::Resource(Res::OutOfMemory));

        let configured_cap: GifError = gif::DecodingError::MemoryLimit.into();
        assert!(matches!(
            configured_cap,
            GifError::MemoryLimitExceeded { .. }
        ));
        assert_eq!(
            configured_cap.category(),
            C::Resource(Res::Limits(L::Memory))
        );
    }
}
