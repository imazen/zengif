//! # zengif
//!
//! Server-side GIF codec with zero-trust design, memory bounds, streaming,
//! and full animation transparency support.
//!
//! ## Features
//!
//! - **Streaming decode/encode**: Process GIFs without loading entire file
//! - **Complete animation support**: Disposal methods, transparency, timing
//! - **Memory bounded**: Configurable limits, reject oversized inputs
//! - **Production ready**: Error tracing via `whereat`, cancellation via `enough`
//! - **Zero-trust**: Validate all inputs, handle malformed data gracefully
//! - **no_std compatible**: Works with `alloc` only (disable `std` feature)
//!
//! ## Quick Start
//!
//! ### Decoding
//!
//! ```rust,no_run
//! use zengif::{Decoder, Limits, Unstoppable};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = std::fs::read("animation.gif")?;
//! let reader = std::io::Cursor::new(&data);
//! let limits = Limits::default();
//!
//! let mut decoder = Decoder::new(reader, limits, &Unstoppable)?;
//!
//! while let Some(frame) = decoder.next_frame()? {
//!     // frame.pixels is composited RGBA
//!     // frame.delay is in centiseconds
//! }
//!
//! // Access memory stats after decoding
//! println!("Memory used: {} bytes", decoder.stats().peak());
//! # Ok(())
//! # }
//! ```
//!
//! ### Encoding
//!
//! ```rust,no_run
//! use zengif::{EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = EncoderConfig::new().repeat(Repeat::Infinite);
//! let limits = Limits::default();
//!
//! let mut encoder = EncodeRequest::new(&config, 100, 100)
//!     .limits(&limits)
//!     .stop(&Unstoppable)
//!     .build()?;
//!
//! let pixels: Vec<Rgba> = vec![Rgba::rgb(255, 0, 0); 100 * 100];
//! encoder.add_frame(FrameInput::new(100, 100, 10, pixels))?;
//!
//! let output: Vec<u8> = encoder.finish()?;
//! std::fs::write("output.gif", &output)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Memory Tracking
//!
//! The decoder tracks buffer allocations internally. Access stats via `decoder.stats()`:
//!
//! ```rust,no_run
//! use zengif::{Decoder, Limits, Unstoppable};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = std::fs::read("animation.gif")?;
//! let reader = std::io::Cursor::new(&data);
//! let mut decoder = Decoder::new(reader, Limits::default(), &Unstoppable)?;
//!
//! while let Some(frame) = decoder.next_frame()? {
//!     // Check memory usage during decode
//!     if decoder.stats().peak() > 100_000_000 {
//!         break; // Stop if using too much memory
//!     }
//! }
//!
//! println!("Peak buffer usage: {} bytes", decoder.stats().peak());
//! # Ok(())
//! # }
//! ```
//!
//! Note: Stats tracks zengif's own allocations (canvas, pixel buffers), not allocations
//! made internally by the gif crate or quantizers.
//!
//! ## Cancellation
//!
//! Operations support cooperative cancellation via the `enough` crate:
//!
//! ```rust,no_run
//! use almost_enough::Stopper;
//! use zengif::{Decoder, Limits};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let stop = Stopper::new();
//! let stop_clone = stop.clone();
//!
//! // In another thread:
//! // stop_clone.cancel();
//!
//! let data = std::fs::read("animation.gif")?;
//! let reader = std::io::Cursor::new(&data);
//! // Pass &stop — references are Copy and implement Stop
//! let mut decoder = Decoder::new(reader, Limits::default(), &stop)?;
//! // Decoder will return GifError::Cancelled if stop is triggered
//! # Ok(())
//! # }
//! ```
//!
//! ## Feature Flags
//!
//! - **`std`** (default): Enables `std::error::Error` impl and std I/O
//! - **`imgref-interop`**: Interop with the `imgref` crate
//!
//! ### Color Quantization Backends
//!
//! Choose one or more quantization backends for encoding:
//!
//! | Feature | Quality | Speed | File Size | License | Use Case |
//! |---------|---------|-------|-----------|---------|----------|
//! | `imagequant` | **Best** | Medium | **Smallest** | GPL-3.0-or-later | **Recommended** - LZW-aware dithering |
//! | `quantizr` | Good | Fast | Medium | MIT | Best MIT-licensed option |
//! | `color_quant` | Good | **Fastest** | Large | MIT | High-throughput servers |
//! | `exoquant-deprecated` | Good | Slow | Medium | MIT | Legacy compatibility only |
//!
//! Configure the quantizer using the `Quantizer` enum:
//!
//! ```rust,ignore
//! use zengif::{EncoderConfig, Quantizer};
//!
//! // Use imagequant (recommended) for best quality
//! let config = EncoderConfig::new()
//!     .quantizer(Quantizer::imagequant());
//!
//! // Use quantizr (MIT) for permissive licensing
//! let config = EncoderConfig::new()
//!     .quantizer(Quantizer::quantizr_with_dithering(0.3));
//!
//! // Auto-select best available
//! let config = EncoderConfig::new()
//!     .quantizer(Quantizer::auto());
//! ```
//!
//! Without any quantization feature, zengif is purely MIT/Apache-2.0 licensed.
//!
//! [imagequant-license]: https://supso.org/projects/pngquant

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Crate info for whereat error tracing
whereat::define_at_crate_info!();

// Internal modules
#[cfg(feature = "std")]
mod decode;
mod disposal;
#[cfg(feature = "std")]
mod encode;
mod error;
#[cfg(feature = "std")]
pub mod heuristics;
mod limits;
#[cfg(feature = "std")]
mod quantize;
mod screen;
mod stats;
mod types;

// Public API
#[cfg(feature = "std")]
pub use decode::{Decoder, FrameIterator, decode_gif};
#[cfg(feature = "std")]
pub use encode::{EncodeRequest, Encoder, EncoderConfig, PaletteStrategy, encode_gif};
#[cfg(all(
    feature = "std",
    any(
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    )
))]
pub use encode::{encode_gif_shared_palette, encode_gif_with_quantizer};
pub use error::{DecodeError, EncodeError, GifError, Result};
pub use limits::Limits;
#[cfg(all(feature = "std", feature = "color_quant"))]
pub use quantize::ColorQuantQuantizer;
#[cfg(all(feature = "std", feature = "exoquant-deprecated"))]
pub use quantize::ExoquantQuantizer;
#[cfg(all(feature = "std", feature = "imagequant"))]
pub use quantize::ImagequantQuantizer;
#[cfg(all(
    feature = "std",
    any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    )
))]
pub use quantize::Quantizer;
#[cfg(all(feature = "std", feature = "quantizr"))]
pub use quantize::QuantizrQuantizer;
#[cfg(all(feature = "std", feature = "zenquant"))]
pub use quantize::ZenquantQuantizer;
#[cfg(feature = "std")]
pub use quantize::{QuantizeConfig, QuantizedFrame, QuantizerBackend, QuantizerTrait};
pub use screen::{Screen, ScreenBuilder};
pub use stats::{
    Stats, StatsSnapshot, TrackedAlloc, tracked_vec_filled, tracked_vec_with_capacity,
};
pub use types::{
    ComposedFrame, DisposalMethod, FrameInput, Metadata, Palette, RawFrame, Repeat, Rgba,
};

#[cfg(feature = "zencodec")]
mod zencodec;
#[cfg(feature = "zencodec")]
pub use zencodec::{
    GifDecodeJob, GifDecoder, GifDecoderConfig, GifEncodeJob, GifEncoder, GifEncoderConfig,
    GifFullFrameDecoder, GifFullFrameEncoder,
};

// Re-export enough for user convenience
pub use enough::{Stop, Unstoppable};
