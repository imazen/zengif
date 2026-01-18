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
//! - **`alloc`**: Heap allocation without std (for no_std environments)
//! - **`simd`**: SIMD acceleration via wide/multiversed
//! - **`rgb-interop`**: Interop with the `rgb` crate
//! - **`imgref-interop`**: Interop with the `imgref` crate
//!
//! ### Color Quantization Backends
//!
//! Choose one or more quantization backends for encoding:
//!
//! | Feature | Quality | Speed | File Size | License | Use Case |
//! |---------|---------|-------|-----------|---------|----------|
//! | `imagequant` | **Best** | Medium | **Smallest** | AGPL-3.0 | **Recommended** - LZW-aware dithering |
//! | `quantizr` | Good | Fast | Medium | MIT | Best MIT-licensed option |
//! | `color_quant` | Good | **Fastest** | Large | MIT | High-throughput servers |
//! | `exoquant-deprecated` | Good | Slow | Medium | MIT | Legacy compatibility only |
//!
//! Configure the quantizer using the [`Quantizer`] enum:
//!
//! ```rust,ignore
//! use zengif::{EncoderConfig, Quantizer};
//!
//! // Use imagequant (recommended) for best quality
//! let config = EncoderConfig::new(100, 100)
//!     .quantizer(Quantizer::imagequant());
//!
//! // Use quantizr (MIT) for permissive licensing
//! let config = EncoderConfig::new(100, 100)
//!     .quantizer(Quantizer::quantizr_with_dithering(0.3));
//!
//! // Auto-select best available
//! let config = EncoderConfig::new(100, 100)
//!     .quantizer(Quantizer::auto());
//! ```
//!
//! Without any quantization feature, zengif is purely MIT/Apache-2.0 licensed.
//!
//! [imagequant-license]: https://supso.org/projects/pngquant

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]
#![warn(clippy::all)]

#[cfg(feature = "alloc")]
extern crate alloc;

// For tests in no_std mode
#[cfg(all(test, not(feature = "std")))]
extern crate std;

// Crate info for whereat error tracing
whereat::define_at_crate_info!();

// I/O abstraction for no_std compatibility
pub mod io;

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
pub use decode::{decode_gif, Decoder, DecoderRead, FrameIterator};
pub use encode::{encode_gif, Encoder, EncoderConfig, PaletteStrategy};
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub use encode::{encode_gif_shared_palette, encode_gif_with_quantizer};
pub use error::{GifError, Result};
pub use limits::Limits;
#[cfg(feature = "color_quant")]
pub use quantize::ColorQuantQuantizer;
#[cfg(feature = "exoquant-deprecated")]
pub use quantize::ExoquantQuantizer;
#[cfg(feature = "imagequant")]
pub use quantize::ImagequantQuantizer;
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub use quantize::Quantizer;
#[cfg(feature = "quantizr")]
pub use quantize::QuantizrQuantizer;
pub use quantize::{QuantizeConfig, QuantizedFrame, QuantizerBackend, QuantizerTrait};
pub use screen::{Screen, ScreenBuilder};
pub use stats::{
    tracked_vec_filled, tracked_vec_with_capacity, Stats, StatsSnapshot, TrackedAlloc,
};
pub use types::{
    ComposedFrame, DisposalMethod, FrameInput, Metadata, Palette, RawFrame, Repeat, Rgba,
};

// Re-export enough for user convenience
pub use enough::{Stop, Unstoppable};
