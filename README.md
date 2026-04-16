# zengif [![CI](https://img.shields.io/github/actions/workflow/status/imazen/zengif/ci.yml?style=flat-square&label=CI)](https://github.com/imazen/zengif/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/zengif?style=flat-square)](https://crates.io/crates/zengif) [![lib.rs](https://img.shields.io/crates/v/zengif?style=flat-square&label=lib.rs&color=blue)](https://lib.rs/crates/zengif) [![docs.rs](https://img.shields.io/docsrs/zengif?style=flat-square)](https://docs.rs/zengif) [![codecov](https://img.shields.io/codecov/c/github/imazen/zengif?style=flat-square)](https://codecov.io/gh/imazen/zengif) [![MSRV](https://img.shields.io/badge/MSRV-1.93-blue?style=flat-square)](https://doc.rust-lang.org/cargo/reference/manifest.html#the-rust-version-field) [![license](https://img.shields.io/crates/l/zengif?style=flat-square)](https://github.com/imazen/zengif#license)

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
zengif = { version = "0.7", default-features = false, features = ["std", "quantette"] }
```

Without *any* quantizer feature, zengif is MIT/Apache-2.0 but encoding requires pre-indexed frames.

## no_std / WASM

For WASM or embedded, disable the default `std` feature:

```toml
zengif = { version = "0.7", default-features = false }
```

You get core types (`Rgba`, `Limits`, `GifError`, etc.) but not the codec. Useful when you need to share types between WASM and native code.

## Performance

Approximate throughput on AMD Ryzen 9 5900X (single-threaded, not independently verified -- run `benches/codec.rs` to reproduce):

| Operation | Throughput |
|-----------|------------|
| Decode (composited) | ~150 MB/s |
| Encode (quantized) | ~40 MB/s |
| Encode (pre-indexed) | ~200 MB/s |

## Image tech I maintain

| | |
|:--|:--|
| State of the art codecs* | [zenjpeg] · [zenpng] · [zenwebp] · **zengif** · [zenavif] ([rav1d-safe] · [zenrav1e] · [zenavif-parse] · [zenavif-serialize]) · [zenjxl] ([jxl-encoder] · [zenjxl-decoder]) · [zentiff] · [zenbitmaps] · [heic] · [zenraw] · [zenpdf] · [ultrahdr] · [mozjpeg-rs] · [webpx] |
| Compression | [zenflate] · [zenzop] |
| Processing | [zenresize] · [zenfilters] · [zenquant] · [zenblend] |
| Metrics | [zensim] · [fast-ssim2] · [butteraugli] · [resamplescope-rs] · [codec-eval] · [codec-corpus] |
| Pixel types & color | [zenpixels] · [zenpixels-convert] · [linear-srgb] · [garb] |
| Pipeline | [zenpipe] · [zencodec] · [zencodecs] · [zenlayout] · [zennode] |
| ImageResizer | [ImageResizer] (C#) — 24M+ NuGet downloads across all packages |
| [Imageflow][] | Image optimization engine (Rust) — [.NET][imageflow-dotnet] · [node][imageflow-node] · [go][imageflow-go] — 9M+ NuGet downloads across all packages |
| [Imageflow Server][] | [The fast, safe image server](https://www.imazen.io/) (Rust+C#) — 552K+ NuGet downloads, deployed by Fortune 500s and major brands |

<sub>* as of 2026</sub>

### General Rust awesomeness

[archmage] · [magetypes] · [enough] · [whereat] · [zenbench] · [cargo-copter]

[And other projects](https://www.imazen.io/open-source) · [GitHub @imazen](https://github.com/imazen) · [GitHub @lilith](https://github.com/lilith) · [lib.rs/~lilith](https://lib.rs/~lilith) · [NuGet](https://www.nuget.org/profiles/imazen) (over 30 million downloads / 87 packages)

## AI-Generated Code Notice

Developed with Claude (Anthropic). Not all code manually reviewed. Review critical paths before production use.

## License

zengif itself is MIT or Apache-2.0, at your option.

**Default features pull in AGPL code.** The `zenquant` quantizer (enabled by default) is AGPL-3.0-or-later. The `imagequant` quantizer is GPL-3.0-or-later ([commercial license available from upstream](https://pngquant.org)). For fully permissive licensing, disable defaults and use `quantette`, `quantizr`, or `color_quant`.

---

**100% safe Rust** - `#![forbid(unsafe_code)]`

[zenjpeg]: https://github.com/imazen/zenjpeg
[zenpng]: https://github.com/imazen/zenpng
[zenwebp]: https://github.com/imazen/zenwebp
[zenavif]: https://github.com/imazen/zenavif
[zenjxl]: https://github.com/imazen/zenjxl
[zentiff]: https://github.com/imazen/zentiff
[zenbitmaps]: https://github.com/imazen/zenbitmaps
[heic]: https://github.com/imazen/heic-decoder-rs
[zenraw]: https://github.com/imazen/zenraw
[zenpdf]: https://github.com/imazen/zenpdf
[ultrahdr]: https://github.com/imazen/ultrahdr
[jxl-encoder]: https://github.com/imazen/jxl-encoder
[zenjxl-decoder]: https://github.com/imazen/zenjxl-decoder
[rav1d-safe]: https://github.com/imazen/rav1d-safe
[zenrav1e]: https://github.com/imazen/zenrav1e
[mozjpeg-rs]: https://github.com/imazen/mozjpeg-rs
[zenavif-parse]: https://github.com/imazen/zenavif-parse
[zenavif-serialize]: https://github.com/imazen/zenavif-serialize
[webpx]: https://github.com/imazen/webpx
[zenflate]: https://github.com/imazen/zenflate
[zenzop]: https://github.com/imazen/zenzop
[zenresize]: https://github.com/imazen/zenresize
[zenfilters]: https://github.com/imazen/zenfilters
[zenquant]: https://github.com/imazen/zenquant
[zenblend]: https://github.com/imazen/zenblend
[zensim]: https://github.com/imazen/zensim
[fast-ssim2]: https://github.com/imazen/fast-ssim2
[butteraugli]: https://github.com/imazen/butteraugli
[zenpixels]: https://github.com/imazen/zenpixels
[zenpixels-convert]: https://github.com/imazen/zenpixels
[linear-srgb]: https://github.com/imazen/linear-srgb
[garb]: https://github.com/imazen/garb
[zenpipe]: https://github.com/imazen/zenpipe
[zencodec]: https://github.com/imazen/zencodec
[zencodecs]: https://github.com/imazen/zencodecs
[zenlayout]: https://github.com/imazen/zenlayout
[zennode]: https://github.com/imazen/zennode
[Imageflow]: https://github.com/imazen/imageflow
[Imageflow Server]: https://github.com/imazen/imageflow-server
[imageflow-dotnet]: https://github.com/imazen/imageflow-dotnet
[imageflow-node]: https://github.com/imazen/imageflow-node
[imageflow-go]: https://github.com/imazen/imageflow-go
[ImageResizer]: https://github.com/imazen/resizer
[archmage]: https://github.com/imazen/archmage
[magetypes]: https://github.com/imazen/archmage
[enough]: https://github.com/imazen/enough
[whereat]: https://github.com/lilith/whereat
[zenbench]: https://github.com/imazen/zenbench
[cargo-copter]: https://github.com/imazen/cargo-copter
[resamplescope-rs]: https://github.com/imazen/resamplescope-rs
[codec-eval]: https://github.com/imazen/codec-eval
[codec-corpus]: https://github.com/imazen/codec-corpus
