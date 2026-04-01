# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.1] - 2026-04-01

### Fixed

- **Memory safety**: Replaced `Box::leak` with `Cow`-owned storage in Encoder to eliminate leaked memory
- **Stats accuracy**: Use `saturating_sub` in `Stats::track_dealloc` to prevent underflow
- **Quantette support**: Added quantette to all cfg gates; fixed transparent index out-of-range panic
- **Transparency**: Fixed transparent pixel mapping in `quantize_frame`; initialized canvas as transparent
- **Error instrumentation**: Upgraded from `.start_at()` to `at!()` macro for better error tracing

### Changed

- Bumped dependencies: zencodec 0.1.12, zenpixels 0.2.2, linear-srgb 0.6.7, archmage, enough, whereat
- Set correct minimum versions for zenflate and linear-srgb

### Added

- Expanded transparency and background disposal test coverage
- Reduced fuzz artifacts for malformed GIF inputs

## [0.7.0] - 2026-03-31

### Changed

- Three-layer encoding API: `EncoderConfig` → `EncodeRequest<'a>` → `Encoder<'a>`
- Modularized encode module into focused submodules (palette, config, request, encoder)
- Added zenquant quantization backend (perceptual masking, AGPL-3.0)
- Added quantette quantization backend (Oklab k-means, MIT/Apache-2.0)
- Upgraded to Rust 2024 edition, MSRV 1.93
- Added zenpixels and linear-srgb dependencies for pixel type integration

## [0.6.0] - 2026-02-06

### Changed - BREAKING

- **API Refactoring**: Three-layer encoding API for better ergonomics and lifetime management
  - `EncoderConfig` no longer takes dimensions (now configuration-only)
  - New `EncodeRequest` builder pattern: `EncodeRequest::new(&config, width, height).limits(&limits).stop(&stop).build()`
  - `Encoder` now uses lifetime-bound references instead of generics (`Encoder<'a>` instead of `Encoder<W, S>`)
  - `Encoder::finish()` now returns `Vec<u8>` instead of writing to a mutable buffer
  - Migration: `Encoder::new(&mut buf, w, h, cfg, lim, stop)` → `EncodeRequest::new(&cfg, w, h).limits(&lim).stop(&stop).build()`

### Internal

- Modularized encode module: Split 2674-line file into focused modules (palette, config, request, encoder)
- Improved code organization and maintainability


## [0.5.0] - 2026-02-04

### Fixed

- **Critical**: Fixed transparent pixel handling in shared palette mode. Frame differencing marks unchanged pixels as transparent, but the shared palette (built from original frames) had no transparent entry. This caused the quantizer to map transparent pixels to nearest color (usually dark gray), creating severe visual artifacts on playback. Quality improved from SSIM ~40 to ~100 on affected GIFs.
- **Correctness**: Fixed `QuantizeResult` reuse in quantizr backend. Previously, each frame was re-quantized independently even in shared palette mode, which was both slower and could produce incorrect palette indices.

### Added

- **Hybrid palette mode**: New `palette_error_threshold` option enables automatic per-frame palette fallback when a frame's RMSE exceeds the threshold. This catches outlier frames that don't fit the shared palette while keeping most frames on the global palette (no flicker, better compression).
- **Lossy frame differencing**: New `lossy_tolerance` option treats pixels within tolerance of the previous frame as unchanged, reducing dirty region size and improving compression at slight quality cost.
- **RMSE analysis examples**: `rmse_analysis.rs` and `size_analysis.rs` for testing palette quality.

### Changed

- **Default `shared_palette` is now `true`**: Most animations benefit from shared palettes (reduced flicker, smaller files). Use `.shared_palette(false)` for the old behavior.
- **Default `palette_error_threshold` lowered to 5.0**: Testing revealed threshold 15 was too permissive; frames with noticeable color distortion (RMSE 5-10) weren't triggering fallback. The new default catches problematic frames while not over-triggering.
- **Dimension-aware buffer frames**: Default `max_buffer_frames` now scales based on frame dimensions (smaller frames = more frames buffered for better palette sampling).

### Performance

- Shared palette mode is now ~4x faster for multi-frame remapping due to `QuantizeResult` reuse.
- `Screen::reset()` uses `slice::fill()` instead of per-pixel loop.
- Memory pooling for encoder scratch buffers.

## [0.4.0] - 2025-02-04

### Changed

- **BREAKING**: `Decoder::new()` now takes 3 arguments instead of 4. The `&Stats` parameter is removed - the decoder owns its stats internally.
- **BREAKING**: `decode_gif()` now returns a 3-tuple `(Metadata, Vec<ComposedFrame>, Stats)` instead of 2-tuple.
- Access stats during decode via `decoder.stats()` method.
- Added `#![forbid(unsafe_code)]` - the entire crate is now 100% safe Rust.

### Removed

- **BREAKING**: Removed unused `simd` feature (was a placeholder with no actual SIMD code).
- All `unsafe` code removed. The use-after-free bug with external `&Stats` is fixed by having the decoder own its stats.

### Migration from 0.3

```rust
// Old (0.3):
let stats = Stats::new();
let mut decoder = Decoder::new(reader, limits, &stats, Unstoppable)?;
// ... decode ...
println!("Peak: {}", stats.peak());

// New (0.4):
let mut decoder = Decoder::new(reader, limits, Unstoppable)?;
// ... decode ...
println!("Peak: {}", decoder.stats().peak());
```

## [0.3.0] - 2025-01-23

### Added

- **no_std support**: Core types work without std library (use `default-features = false`).
- **`heuristics` module**: Estimate memory/time requirements before decoding.
- **WASM support**: Verified working on `wasm32-unknown-unknown` (144KB release build).
- Uses `core::error::Error` (stabilized in Rust 1.81) instead of `std::error::Error`.

### Changed

- Minimum supported Rust version is now 1.81 (for `core::error::Error`).
- Dev dependencies gated for WASM compatibility.

## [0.2.1] - 2025-01-20

### Fixed

- Removed misleading memory tracking claim from README.
- Fixed comparison table accuracy.

## [0.2.0] - 2025-01-19

### Added

- **Multiple quantization backends**: Choose from different color quantizers:
  - `imagequant`: Highest quality, smallest files (GPL-3.0-or-later, commercial license available).
  - `quantizr`: Fast, good quality (MIT).
  - `color_quant`: NEUQUANT algorithm, fastest (MIT).
  - `exoquant-deprecated`: K-Means quantizer (MIT, use quantizr instead).

### Changed

- Renamed `quantize` feature to `imagequant` for clarity.
- Each quantization backend is now a separate feature flag.

## [0.1.0] - 2025-01-18

### Added

- **Streaming decode/encode API**: Process GIFs without loading entire files into memory.
  - `Decoder` for frame-by-frame streaming decode with disposal handling.
  - `Encoder` for progressive frame encoding with optional quantization.
  - `decode_gif()` and `encode_gif()` convenience functions.

- **Complete animation support**:
  - All disposal methods (Keep, Background, Previous).
  - Per-frame transparency handling.
  - Timing preservation on round-trip.
  - Loop count (NETSCAPE extension) support.

- **Memory safety and limits**:
  - Configurable `Limits` for dimensions, frame count, file size, memory.
  - `Stats` for tracking buffer allocations.
  - Fallible allocations throughout (`try_reserve()`).
  - Zip bomb protection via decompression ratio limits.

- **Production-ready error handling**:
  - Integration with [`whereat`](https://crates.io/crates/whereat) for error location tracking.
  - Integration with [`enough`](https://crates.io/crates/enough) for cooperative cancellation.
  - Detailed error messages with context.

- **Zero-trust input validation**:
  - Header validation before allocation.
  - Frame bounds checking.
  - Malformed LZW handling.

- **Optional features**:
  - `rgb-interop`: Interop with `rgb` crate types.
  - `imgref-interop`: Interop with `imgref` crate types.

- **Frame optimization for encoding**:
  - Frame differencing to find minimal changed regions.
  - Transparency optimization for unchanged pixels.

### Performance

- Decode: ~150 MB/s composited throughput.
- Encode (pre-indexed): ~200 MB/s.
- Encode (quantized): ~40 MB/s.

[0.7.1]: https://github.com/imazen/zengif/releases/tag/v0.7.1
[0.7.0]: https://github.com/imazen/zengif/releases/tag/v0.7.0
[0.6.0]: https://github.com/imazen/zengif/releases/tag/v0.6.0
[0.5.0]: https://github.com/imazen/zengif/releases/tag/v0.5.0
[0.4.0]: https://github.com/imazen/zengif/releases/tag/v0.4.0
[0.3.0]: https://github.com/imazen/zengif/releases/tag/v0.3.0
[0.2.1]: https://github.com/imazen/zengif/releases/tag/v0.2.1
[0.2.0]: https://github.com/imazen/zengif/releases/tag/v0.2.0
[0.1.0]: https://github.com/imazen/zengif/releases/tag/v0.1.0
