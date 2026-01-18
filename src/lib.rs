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
//! ```rust,ignore
//! use zengif::{GifDecoder, DecodeLimits, Stats};
//! use enough::Unstoppable;
//!
//! let limits = DecodeLimits::default();
//! let stats = Stats::new();
//!
//! let decoder = GifDecoder::new(reader, limits, &stats, Unstoppable)?;
//!
//! for frame in decoder.frames() {
//!     let frame = frame?;
//!     // frame.pixels is composited RGBA
//! }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Crate info for whereat error tracing
whereat::define_at_crate_info!();

// Module declarations (to be implemented)
// mod decode;
// mod encode;
// mod disposal;
// mod screen;
// mod error;
// mod limits;
// mod stats;
// mod types;

// Re-exports
// pub use decode::GifDecoder;
// pub use encode::GifEncoder;
// pub use error::{GifError, Result};
// pub use limits::DecodeLimits;
// pub use stats::Stats;
// pub use types::{Frame, ComposedFrame, Metadata, Repeat};

// Re-export enough for user convenience
pub use enough::{Stop, Unstoppable};
