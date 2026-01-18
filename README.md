# zengif

[![CI](https://github.com/imazen/zengif/actions/workflows/ci.yml/badge.svg)](https://github.com/imazen/zengif/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/zengif.svg)](https://crates.io/crates/zengif)
[![Documentation](https://docs.rs/zengif/badge.svg)](https://docs.rs/zengif)
[![codecov](https://codecov.io/gh/imazen/zengif/branch/main/graph/badge.svg)](https://codecov.io/gh/imazen/zengif)
[![License](https://img.shields.io/crates/l/zengif.svg)](LICENSE-MIT)

Server-side GIF codec with zero-trust design, memory bounds, streaming, and full animation transparency support.

## Why zengif?

Existing GIF libraries leave gaps when building server-side image processing:

| Feature | gif crate | gif-dispose | zengif |
|---------|-----------|-------------|--------|
| Streaming decode | ✅ | ❌ | ✅ |
| Disposal methods | ❌ | ✅ | ✅ |
| Transparency | Partial | ✅ | ✅ |
| Memory limits | Basic | ❌ | ✅ |
| Memory tracking | ❌ | ❌ | ✅ |
| Cancellation | ❌ | ❌ | ✅ |
| Error tracing | ❌ | ❌ | ✅ |
| Round-trip metadata | ❌ | ❌ | ✅ |
| High-quality encode | ❌ | ❌ | ✅ |

## Features

- **Streaming decode/encode** - Process GIFs without loading entire file into memory
- **Complete animation support** - All disposal methods (Keep, Background, Previous) + transparency working together
- **Memory bounded** - Track current/peak usage, enforce configurable limits, reject oversized inputs before allocating
- **Production ready** - Error tracing via [`whereat`](https://crates.io/crates/whereat), cancellation via [`enough`](https://crates.io/crates/enough)
- **Zero-trust design** - Validate headers, bounds-check frames, limit decompression ratio
- **Round-trip fidelity** - Timing, loop count, and metadata preserved
- **High-quality encoding** - Optional imagequant integration for small file sizes

## Quick Start

### Decoding

```rust
use zengif::{Decoder, Limits, Stats};
use enough::Unstoppable;

let limits = Limits::default()
    .max_dimensions(4096, 4096)
    .max_frame_count(1000);

let stats = Stats::new();
let cursor = std::io::Cursor::new(gif_data);

let mut decoder = Decoder::new(cursor, limits, &stats, Unstoppable)?;

while let Some(frame) = decoder.next_frame()? {
    // frame.pixels: Vec<Rgba> - composited RGBA with disposal applied
    // frame.delay: u16 - delay in centiseconds
    // frame.index: usize - frame number
}

println!("Peak memory: {} bytes", stats.peak());
```

### Encoding

```rust
use zengif::{Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba};
use enough::Unstoppable;

let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
let limits = Limits::default();

let mut output = Vec::new();
let mut encoder = Encoder::new(&mut output, config, limits, Unstoppable)?;

for frame in source_frames {
    encoder.add_frame(frame)?;
}

encoder.finish()?;
```

### Round-Trip with Metadata

```rust
use zengif::{Decoder, Encoder, Limits, Stats};
use enough::Unstoppable;

let stats = Stats::new();
let mut decoder = Decoder::new(reader, Limits::default(), &stats, Unstoppable)?;

let metadata = decoder.metadata().clone();
let mut encoder = Encoder::from_metadata(writer, &metadata, Limits::default(), Unstoppable)?;

while let Some(frame) = decoder.next_frame()? {
    // Re-encode the composed frame
    let input = FrameInput::new(frame.width, frame.height, frame.delay, frame.pixels);
    encoder.add_frame(input)?;
}

encoder.finish()?;
```

### With Cancellation

```rust
use almost_enough::Stopper;
use zengif::{Decoder, Limits, Stats};

let stop = Stopper::new();
let stop_clone = stop.clone();

// In another thread/task:
stop_clone.cancel();

// Decoder will return GifError::Cancelled
let stats = Stats::new();
let decoder = Decoder::new(reader, Limits::default(), &stats, stop)?;
```

## Memory Limits

Protect against malicious inputs:

```rust
use zengif::Limits;

let limits = Limits::default()
    .max_dimensions(8192, 8192)           // Max canvas size
    .max_total_pixels(67_108_864)         // Max 64M pixels per frame
    .max_frame_count(10_000)              // Max frames
    .max_file_size(500 * 1024 * 1024)     // 500 MB
    .max_memory(1024 * 1024 * 1024);      // 1 GB peak memory
```

## Error Handling

Errors include location and context for debugging:

```
Error: InvalidFrameBounds { frame_left: 0, frame_top: 0, frame_width: 5000,
                            frame_height: 5000, canvas_width: 100, canvas_height: 100 }
   at src/decode/frame.rs:142:9
      ╰─ validating frame 3
   at src/decode/mod.rs:89:5
      ╰─ in decode_frame
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | ✅ | Standard library support |
| `alloc` | ✅ | Heap allocation (via std) |
| `simd` | ❌ | SIMD acceleration via wide/multiversed |

### Color Quantization Backends

Choose one or more quantization backends for high-quality GIF encoding:

| Feature | License | Quality | Speed | Notes |
|---------|---------|---------|-------|-------|
| `imagequant` | AGPL-3.0 | Excellent | Medium | Best quality, [commercial license available][imagequant-license] |
| `exoquant` | MIT | Very Good | Medium | K-Means optimization, Floyd-Steinberg dithering |
| `quantizr` | MIT | Good | Fast | Good balance of quality and speed |
| `color_quant` | MIT | Good | Fast | NEUQUANT algorithm |

**Without any quantization feature, zengif is purely MIT/Apache-2.0 licensed.**

[imagequant-license]: https://supso.org/projects/pngquant

## no_std Support

```toml
[dependencies]
zengif = { version = "0.1", default-features = false, features = ["alloc"] }
```

## Performance

Benchmarks on AMD Ryzen 9 5900X:

| Operation | Throughput |
|-----------|------------|
| Decode (composited) | ~150 MB/s |
| Encode (quantized) | ~40 MB/s |
| Encode (pre-indexed) | ~200 MB/s |

## AI-Generated Code Notice

Developed with Claude (Anthropic). Not all code manually reviewed. Review critical paths before production use.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Dependency Licensing: imagequant (AGPL)

The optional `imagequant` feature uses [libimagequant](https://github.com/ImageOptim/libimagequant), which is licensed under **AGPL-3.0**. If you enable this feature, your project must comply with AGPL terms (open-source your code) **or** purchase a [commercial license](https://supso.org/projects/pngquant) from the imagequant authors.

**Alternative MIT-licensed quantizers:** Use `exoquant`, `quantizr`, or `color_quant` features instead for fully permissive licensing.

**Without any quantization feature** (the default), zengif has no AGPL dependencies and is purely MIT/Apache-2.0 licensed.
