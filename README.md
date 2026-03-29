# zengif ![CI](https://img.shields.io/github/actions/workflow/status/imazen/zengif/ci.yml?style=flat-square&label=CI) ![crates.io](https://img.shields.io/crates/v/zengif?style=flat-square) ![docs.rs](https://img.shields.io/docsrs/zengif?style=flat-square) ![codecov](https://img.shields.io/codecov/c/github/imazen/zengif?style=flat-square) ![MSRV](https://img.shields.io/badge/MSRV-1.93-blue?style=flat-square) ![license](https://img.shields.io/crates/l/zengif?style=flat-square)

A GIF codec built for servers: streaming, memory-bounded, and thoroughly tested.

> **Licensing note:** The default features include `zenquant` (AGPL-3.0-or-later).
> A plain `cargo add zengif` pulls in AGPL-licensed code. For MIT/Apache-2.0-only
> licensing, use `default-features = false` and select a permissive quantizer
> (e.g., `quantette`, `quantizr`, or `color_quant`). See [Quantizer Options](#quantizer-options).

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

    let mut decoder = Decoder::new(reader, Limits::default(), &Unstoppable)?;

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
// `almost-enough` provides a thread-safe Stopper (add it separately: cargo add almost-enough)
use almost_enough::Stopper;
use zengif::{Decoder, Limits};

let stop = Stopper::new();
let stop_for_handler = stop.clone();

// In your request handler, if client disconnects:
stop_for_handler.cancel();

// The decoder will return GifError::Cancelled at the next check point
let mut decoder = Decoder::new(reader, Limits::default(), &stop)?;
```

Any type implementing `enough::Stop` works here. zengif re-exports `Unstoppable` for cases where cancellation isn't needed.

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

With default features, `zenquant` is enabled and selected automatically:

```rust
use zengif::{EncoderConfig, Quantizer};

let config = EncoderConfig::new()
    .quantizer(Quantizer::auto());  // Picks best available (zenquant by default)
```

To use a specific quantizer, enable its feature and select it explicitly:

```bash
cargo add zengif --no-default-features --features std,imagequant
```

```rust
let config = EncoderConfig::new()
    .quantizer(Quantizer::imagequant());
```

### Quantizer Options

Auto-selection priority (top to bottom):

| Feature | License | Quality | Speed | Notes |
|---------|---------|---------|-------|-------|
| `zenquant` (default) | AGPL-3.0 | Best perceptual | Medium | Butteraugli/SSIMULACRA2 metrics |
| `quantette` | MIT/Apache-2.0 | Very good | Fast | Oklab k-means |
| `imagequant` | GPL-3.0* | Good, smallest files | Medium | Compressible dithering patterns |
| `quantizr` | MIT | Good | Fast | |
| `color_quant` | MIT | Acceptable | Fastest | Good for high-throughput |

*[imagequant](https://github.com/ImageOptim/libimagequant) is GPL-3.0-or-later. [Commercial license available from upstream](https://pngquant.org).

**Default features include AGPL code.** `cargo add zengif` enables `zenquant`, which is AGPL-3.0-or-later. For permissive-only licensing, disable default features and pick a quantizer:

```toml
zengif = { version = "0.6", default-features = false, features = ["std", "quantette"] }
```

Without *any* quantizer feature, zengif is MIT/Apache-2.0 but encoding requires pre-indexed frames.

## no_std / WASM

For WASM or embedded, disable the default `std` feature:

```toml
zengif = { version = "0.6", default-features = false }
```

You get core types (`Rgba`, `Limits`, `GifError`, etc.) but not the codec. Useful when you need to share types between WASM and native code.

## Performance

Approximate throughput on AMD Ryzen 9 5900X (single-threaded, not independently verified -- run `benches/codec.rs` to reproduce):

| Operation | Throughput |
|-----------|------------|
| Decode (composited) | ~150 MB/s |
| Encode (quantized) | ~40 MB/s |
| Encode (pre-indexed) | ~200 MB/s |

## License

zengif itself is MIT or Apache-2.0, at your option.

**Default features pull in AGPL code.** The `zenquant` quantizer (enabled by default) is AGPL-3.0-or-later. The `imagequant` quantizer is GPL-3.0-or-later ([commercial license available from upstream](https://pngquant.org)). For fully permissive licensing, disable defaults and use `quantette`, `quantizr`, or `color_quant`.

---

**100% safe Rust** - `#![forbid(unsafe_code)]`
