# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
  - `imagequant`: Highest quality, smallest files (AGPL-3.0, commercial license available).
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

[0.4.0]: https://github.com/imazen/zengif/releases/tag/v0.4.0
[0.3.0]: https://github.com/imazen/zengif/releases/tag/v0.3.0
[0.2.1]: https://github.com/imazen/zengif/releases/tag/v0.2.1
[0.2.0]: https://github.com/imazen/zengif/releases/tag/v0.2.0
[0.1.0]: https://github.com/imazen/zengif/releases/tag/v0.1.0
