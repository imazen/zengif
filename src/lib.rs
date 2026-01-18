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
mod screen;
mod stats;
mod types;

// Public API
pub use decode::{decode_gif, Decoder, FrameIterator};
pub use encode::{encode_gif, Encoder, EncoderConfig};
pub use error::{GifError, Result};
pub use limits::Limits;
pub use screen::{Screen, ScreenBuilder};
pub use stats::{
    tracked_vec_filled, tracked_vec_with_capacity, Stats, StatsSnapshot, TrackedAlloc,
};
pub use types::{
    ComposedFrame, DisposalMethod, FrameInput, Metadata, Palette, RawFrame, Repeat, Rgba,
};

// Re-export enough for user convenience
pub use enough::{Stop, Unstoppable};
