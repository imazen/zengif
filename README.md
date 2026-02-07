# zengif

[![CI](https://github.com/imazen/zengif/actions/workflows/ci.yml/badge.svg)](https://github.com/imazen/zengif/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/zengif.svg)](https://crates.io/crates/zengif)
[![Documentation](https://docs.rs/zengif/badge.svg)](https://docs.rs/zengif)
[![codecov](https://codecov.io/gh/imazen/zengif/branch/main/graph/badge.svg)](https://codecov.io/gh/imazen/zengif)
[![License](https://img.shields.io/crates/l/zengif.svg)](LICENSE-MIT)

A GIF codec built for servers: streaming, memory-safe, and production-ready.

## Getting Started

```bash
cargo add zengif
```

### Decode a GIF

```rust
use zengif::{Decoder, Limits, Unstoppable};
use std::fs::File;
use std::io::BufReader;

fn main() -> zengif::Result<()> {
    let file = File::open("animation.gif")?;
    let reader = BufReader::new(file);

    let mut decoder = Decoder::new(reader, Limits::default(), Unstoppable)?;

    println!("{}x{}, {} frames",
        decoder.metadata().width,
        decoder.metadata().height,
        decoder.metadata().frame_count_hint.unwrap_or(0));

    while let Some(frame) = decoder.next_frame()? {
        // frame.pixels: Vec<Rgba> - fully composited with transparency
        // frame.delay: u16 - delay in centiseconds (100ths of a second)
        println!("Frame {}: {}ms delay", frame.index, frame.delay as u32 * 10);
    }

    Ok(())
}
```

### Encode a GIF

```rust
use zengif::{EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};

fn main() -> zengif::Result<()> {
    let width = 100;
    let height = 100;

    // Create 3 frames of solid colors
    let red: Vec<Rgba> = (0..width*height).map(|_| Rgba::rgb(255, 0, 0)).collect();
    let green: Vec<Rgba> = (0..width*height).map(|_| Rgba::rgb(0, 255, 0)).collect();
    let blue: Vec<Rgba> = (0..width*height).map(|_| Rgba::rgb(0, 0, 255)).collect();

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::default();

    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()?;

    encoder.add_frame(FrameInput::new(width, height, 50, red))?;   // 500ms
    encoder.add_frame(FrameInput::new(width, height, 50, green))?; // 500ms
    encoder.add_frame(FrameInput::new(width, height, 50, blue))?;  // 500ms

    let output = encoder.finish()?;

    std::fs::write("output.gif", &output)?;
    Ok(())
}
```

## Why zengif?

If you're building a server that handles untrusted GIF uploads, you need:

- **Memory limits** - Reject oversized images before allocating
- **Cancellation** - Stop processing if the request is cancelled
- **Error context** - Know exactly where parsing failed
- **Correct compositing** - Handle all disposal methods and transparency

zengif builds on the excellent [`gif`](https://crates.io/crates/gif) crate, adding these production features:

| Feature | gif crate | zengif |
|---------|-----------|--------|
| Streaming decode | ✅ | ✅ |
| Memory limits | ✅ | ✅ |
| Frame compositing | ❌ (use gif-dispose) | ✅ built-in |
| Cooperative cancellation | ❌ | ✅ |
| Error tracing (file:line) | ❌ | ✅ |
| High-quality encoding | ❌ | ✅ (optional) |

## Memory Protection

Protect your server from malicious inputs:

```rust
use zengif::Limits;

let limits = Limits::default()
    .max_dimensions(4096, 4096)       // Reject huge canvases
    .max_frame_count(1000)            // Limit animation length
    .max_memory(256 * 1024 * 1024);   // 256 MB peak memory
```

The decoder will return an error before allocating if limits would be exceeded.

## Cancellation

For web servers, you often need to stop processing if the client disconnects:

```rust
use almost_enough::Stopper;
use zengif::{Decoder, Limits};

let stop = Stopper::new();
let stop_for_handler = stop.clone();

// In your request handler, if client disconnects:
stop_for_handler.cancel();

// The decoder will return GifError::Cancelled at the next check point
let mut decoder = Decoder::new(reader, Limits::default(), stop)?;
```

## Error Diagnostics

When something goes wrong, you get the full story:

```
Error: InvalidFrameBounds { frame_left: 0, frame_top: 0, frame_width: 5000,
                            frame_height: 5000, canvas_width: 100, canvas_height: 100 }
   at src/decode/frame.rs:142:9
      ╰─ validating frame 3
   at src/decode/mod.rs:89:5
      ╰─ in decode_frame
```

## High-Quality Encoding

For the smallest file sizes, enable the `imagequant` feature:

```bash
cargo add zengif --features imagequant
```

```rust
use zengif::{EncoderConfig, Quantizer};

let config = EncoderConfig::new()
    .quantizer(Quantizer::imagequant());  // Best quality, smallest files
```

### Quantizer Options

| Feature | License | Quality | Speed |
|---------|---------|---------|-------|
| `imagequant` | AGPL-3.0* | Best | Medium |
| `quantizr` | MIT | Good | Fast |
| `color_quant` | MIT | Good | Fastest |

*[imagequant](https://github.com/ImageOptim/libimagequant) is AGPL-3.0. [Commercial license available from upstream](https://supso.org/projects/pngquant).

**Why imagequant produces smaller files:** imagequant's superior quality comes from more compressible dithering and quantization patterns. The resulting palettes compress dramatically better in the LZW stage, producing smaller final GIF files compared to other quantizers at the same visual quality.

**Without any quantizer feature, zengif is MIT/Apache-2.0 licensed.**

## no_std / WASM

For WASM or embedded, disable the default `std` feature:

```toml
zengif = { version = "0.4", default-features = false }
```

You get core types (`Rgba`, `Limits`, `GifError`, etc.) but not the codec. Useful when you need to share types between WASM and native code.

## Performance

On AMD Ryzen 9 5900X:

| Operation | Throughput |
|-----------|------------|
| Decode (composited) | ~150 MB/s |
| Encode (quantized) | ~40 MB/s |
| Encode (pre-indexed) | ~200 MB/s |

## License

MIT or Apache-2.0, at your option.

The optional `imagequant` feature uses [libimagequant](https://github.com/ImageOptim/libimagequant) (AGPL-3.0). A [commercial license is available from the upstream author](https://supso.org/projects/pngquant) for closed-source use. Alternatively, use `quantizr` or `color_quant` for fully permissive licensing.

---

**100% safe Rust** - `#![forbid(unsafe_code)]`
