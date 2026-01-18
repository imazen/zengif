# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-01-18

### Added

- **Streaming decode/encode API**: Process GIFs without loading entire files into memory
  - `Decoder` for frame-by-frame streaming decode
  - `Encoder` for progressive frame encoding
  - `decode_gif()` and `encode_gif()` convenience functions

- **Complete animation support**:
  - All disposal methods (Keep, Background, Previous)
  - Per-frame transparency handling
  - Timing preservation on round-trip
  - Loop count (NETSCAPE extension) support

- **Memory safety and tracking**:
  - Configurable `Limits` for dimensions, frame count, file size, memory, decompression ratio
  - `Stats` for real-time memory usage tracking
  - Fallible allocations throughout (`try_reserve()`)
  - Zip bomb protection via decompression ratio limits

- **Production-ready error handling**:
  - Integration with `whereat` for error location tracking
  - Integration with `enough` for cooperative cancellation
  - Detailed error messages with context

- **Zero-trust input validation**:
  - Header validation before allocation
  - Frame bounds checking
  - Malformed LZW handling

- **Optional features**:
  - `quantize`: High-quality color quantization via imagequant
  - `simd`: SIMD acceleration via wide/multiversed
  - `rgb-interop`: Interop with `rgb` crate types
  - `imgref-interop`: Interop with `imgref` crate types
  - `no_std` support with `alloc` feature

- **Frame optimization for encoding**:
  - Frame differencing to find minimal changed regions
  - Transparency optimization for unchanged pixels
  - Shared palette support across frames

### Performance

- Decode: ~150 MB/s composited throughput
- Encode (pre-indexed): ~200 MB/s
- Encode (quantized): ~40 MB/s
- Memory overhead: < 2x frame size

[0.1.0]: https://github.com/imazen/zengif/releases/tag/v0.1.0
