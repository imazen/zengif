//! # zengif
//!
//! Server-side GIF codec with zero-trust design, memory bounds, streaming,
//! and full animation transparency support.
//!
//! ## Features
//!
//! - **Streaming decode/encode**: Process GIFs without loading entire file
//! - **Complete animation support**: Disposal methods, transparency, timing
//! - **Memory bounded**: Track and limit allocations, reject oversized inputs
//! - **Production ready**: Error tracing via `whereat`, cancellation via `enough`
//! - **Zero-trust**: Validate all inputs, handle malformed data gracefully
//!
//! ## Quick Start
//!
//! ### Decoding
//!
//! ```rust,ignore
//! use zengif::{Decoder, Limits, Stats};
//! use enough::Unstoppable;
//!
//! let limits = Limits::default();
//! let stats = Stats::new();
//!
//! let mut decoder = Decoder::new(reader, limits, &stats, Unstoppable)?;
//!
//! while let Some(frame) = decoder.next_frame()? {
//!     // frame.pixels is composited RGBA
//!     // frame.delay is in centiseconds
//! }
//! ```
//!
//! ### Encoding
//!
//! ```rust,ignore
//! use zengif::{Encoder, EncoderConfig, FrameInput, Limits, Repeat};
//! use enough::Unstoppable;
//!
//! let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
//! let limits = Limits::default();
//!
//! let mut encoder = Encoder::new(writer, config, limits, Unstoppable)?;
//!
//! for frame in frames {
//!     encoder.add_frame(frame)?;
//! }
//!
//! encoder.finish()?;
//! ```
//!
//! ## Memory Tracking
//!
//! All operations track memory usage through a `Stats` object:
//!
//! ```rust,ignore
//! let stats = Stats::new();
//! // ... use decoder/encoder ...
//! println!("Peak memory: {} bytes", stats.peak());
//! ```
//!
//! ## Cancellation
//!
//! Operations support cooperative cancellation via the `enough` crate:
//!
//! ```rust,ignore
//! use almost_enough::Stopper;
//!
//! let stop = Stopper::new();
//! let stop_clone = stop.clone();
//!
//! // In another thread:
//! stop_clone.cancel();
//!
//! // Decoder will return GifError::Cancelled
//! ```
//!
//! ## Feature Flags
//!
//! - **`std`** (default): Standard library support
//! - **`alloc`**: Heap allocation without std
//! - **`simd`**: SIMD acceleration via wide/multiversed
//! - **`rgb-interop`**: Interop with the `rgb` crate
//! - **`imgref-interop`**: Interop with the `imgref` crate
//!
//! ### Color Quantization Backends
//!
//! Choose one or more quantization backends for encoding:
//!
//! - **`imagequant`**: Highest quality (AGPL-3.0, [commercial license available][imagequant-license])
//! - **`exoquant`**: High quality K-Means (MIT)
//! - **`quantizr`**: Fast, good quality (MIT)
//! - **`color_quant`**: NEUQUANT algorithm (MIT)
//!
//! Without any quantization feature, zengif is purely MIT/Apache-2.0 licensed.
//!
//! [imagequant-license]: https://supso.org/projects/pngquant

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]
#![warn(clippy::all)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Crate info for whereat error tracing
whereat::define_at_crate_info!();

// Internal modules
mod decode;
mod disposal;
mod encode;
mod error;
mod limits;
mod quantize;
mod screen;
mod stats;
mod types;

// Public API
pub use decode::{decode_gif, Decoder, FrameIterator};
pub use encode::{encode_gif, Encoder, EncoderConfig, PaletteStrategy};
#[cfg(feature = "imagequant")]
pub use encode::{encode_gif_shared_palette, encode_gif_with_quantizer};
pub use error::{GifError, Result};
pub use limits::Limits;
#[cfg(feature = "color_quant")]
pub use quantize::ColorQuantQuantizer;
#[cfg(feature = "exoquant")]
pub use quantize::ExoquantQuantizer;
#[cfg(feature = "imagequant")]
pub use quantize::ImagequantQuantizer;
#[cfg(feature = "quantizr")]
pub use quantize::QuantizrQuantizer;
pub use quantize::{QuantizeConfig, QuantizedFrame, Quantizer, QuantizerBackend};
pub use screen::{Screen, ScreenBuilder};
pub use stats::{
    tracked_vec_filled, tracked_vec_with_capacity, Stats, StatsSnapshot, TrackedAlloc,
};
pub use types::{
    ComposedFrame, DisposalMethod, FrameInput, Metadata, Palette, RawFrame, Repeat, Rgba,
};

// Re-export enough for user convenience
pub use enough::{Stop, Unstoppable};
