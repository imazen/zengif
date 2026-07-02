# zengif [![CI](https://img.shields.io/github/actions/workflow/status/imazen/zengif/ci.yml?style=flat-square&label=CI)](https://github.com/imazen/zengif/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/zengif?style=flat-square)](https://crates.io/crates/zengif) [![lib.rs](https://img.shields.io/crates/v/zengif?style=flat-square&label=lib.rs&color=blue)](https://lib.rs/crates/zengif) [![docs.rs](https://img.shields.io/docsrs/zengif?style=flat-square)](https://docs.rs/zengif) [![license](https://img.shields.io/crates/l/zengif?style=flat-square)](#license) [![MSRV](https://img.shields.io/badge/MSRV-1.93-blue?style=flat-square)](https://doc.rust-lang.org/cargo/reference/manifest.html#the-rust-version-field) [![codecov](https://img.shields.io/codecov/c/github/imazen/zengif?style=flat-square)](https://codecov.io/gh/imazen/zengif)

zengif is a GIF codec built for servers: zero-trust decoding of untrusted uploads, bounded and tracked memory, cooperative cancellation, frame-by-frame streaming, and complete animation support (every disposal method, transparency, timing, and loop count). Pure Rust, `#![forbid(unsafe_code)]`, `no_std`-compatible (core types without `std`), built on the well-maintained [`gif`](https://crates.io/crates/gif) crate.

> **Licensing note:** The default features include `zenquant` (AGPL-3.0-or-later).
> A plain `cargo add zengif` pulls in AGPL-licensed code. For MIT/Apache-2.0-only
> licensing, use `default-features = false` and select a permissive quantizer
> (e.g., `quantette`, `quantizr`, or `color_quant`). See [Quantizer options](#quantizer-options).

## Quick start

```toml
[dependencies]
zengif = "0.7"
```

### Decode a GIF

```rust
use zengif::{decode_gif, Limits, Unstoppable};

fn main() -> zengif::Result<()> {
    let bytes = std::fs::read("animation.gif")?;

    // One-shot, in-memory decode. Every frame is composited to a full-canvas
    // RGBA buffer; Limits::default() applies server-safe ceilings (see below).
    let (meta, frames, _stats) = decode_gif(&bytes, Limits::default(), &Unstoppable)?;

    println!("{}x{}, {} frames", meta.width, meta.height, frames.len());
    for frame in &frames {
        // frame.pixels: Vec<Rgba> — full canvas, disposal + transparency applied
        // frame.delay:  u16       — frame delay in centiseconds (1/100 s)
    }
    Ok(())
}
```

For large or untrusted streams, decode frame-by-frame instead of materializing the
whole `Vec` — `Decoder::new` takes any `std::io::Read`:

```rust
use zengif::{Decoder, Limits, Unstoppable};

fn main() -> zengif::Result<()> {
    let file = std::io::BufReader::new(std::fs::File::open("animation.gif")?);
    let mut decoder = Decoder::new(file, Limits::default(), &Unstoppable)?;

    while let Some(frame) = decoder.next_frame()? {
        // frame.index, frame.delay, frame.pixels (full-canvas Vec<Rgba>)
    }
    println!("peak buffer usage: {} bytes", decoder.stats().peak());
    Ok(())
}
```

### Encode a GIF

```rust
use zengif::{EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};

fn main() -> zengif::Result<()> {
    let (width, height) = (100, 100);

    // Three solid-color frames.
    let red:   Vec<Rgba> = vec![Rgba::rgb(255, 0, 0); width as usize * height as usize];
    let green: Vec<Rgba> = vec![Rgba::rgb(0, 255, 0); width as usize * height as usize];
    let blue:  Vec<Rgba> = vec![Rgba::rgb(0, 0, 255); width as usize * height as usize];

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::default();

    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()?;

    encoder.add_frame(FrameInput::new(width, height, 50, red))?;   // 500ms
    encoder.add_frame(FrameInput::new(width, height, 50, green))?; // 500ms
    encoder.add_frame(FrameInput::new(width, height, 50, blue))?;  // 500ms

    let output: Vec<u8> = encoder.finish()?;
    std::fs::write("output.gif", &output)?;
    Ok(())
}
```

The three layers — `EncoderConfig` (knobs) → `EncodeRequest<'a>` (dimensions +
limits + cancellation) → `Encoder<'a>` (streaming `add_frame`/`finish`) — keep the
common path short while exposing every knob on the config.

### Decode → re-encode (transcode)

A server that re-compresses uploaded GIFs decodes to composited frames, optionally
processes them, then re-encodes. The decoder hands you **full-canvas** RGBA frames and the
loop count; you feed those straight back into the encoder. Loop count and per-frame timing
carry across the round-trip by reading `metadata.repeat` and each frame's `delay`:

```rust
use zengif::{
    decode_gif, EncodeRequest, EncoderConfig, FrameInput, Limits, Unstoppable,
};

fn transcode(input: &[u8]) -> zengif::Result<Vec<u8>> {
    // Build one Limits posture and reuse it (Limits is Clone).
    let limits = Limits::default().max_dimensions(4096, 4096);

    // 1. Decode. `decode_gif` reads every frame, so `meta.repeat` (the loop
    //    count, parsed from the NETSCAPE extension during iteration) is final.
    let (meta, frames, _stats) = decode_gif(input, limits.clone(), &Unstoppable)?;

    // 2. Carry the source loop count into the encoder config.
    //    meta.repeat is a `Repeat` (Once | Infinite | Count(n)) — pass it directly.
    //    `.for_round_trip()` zeroes dithering + shares one palette to minimise bloat
    //    when re-encoding already-quantized content. (It needs a quantizer feature,
    //    which the default build has; drop it if you built `--no-default-features`.)
    let config = EncoderConfig::new()
        .repeat(meta.repeat)
        .for_round_trip();

    let mut encoder = EncodeRequest::new(&config, meta.width, meta.height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()?;

    // 3. Re-encode. Each `ComposedFrame` is already the full canvas size, so it
    //    maps 1:1 onto a full-canvas `FrameInput`. `frame.delay` (centiseconds)
    //    carries the original timing; the encoder recomputes frame differencing
    //    and offsets internally — you always supply whole-canvas frames.
    for frame in frames {
        encoder.add_frame(FrameInput::new(
            frame.width,   // == meta.width  (canvas dims)
            frame.height,  // == meta.height
            frame.delay,   // centiseconds, preserved from source
            frame.pixels,  // Vec<Rgba>, full-canvas composited
        ))?;
    }

    encoder.finish()
}
```

**Key points for round-tripping correctly:**

- **Feed composited (full-canvas) frames back, not sub-frames.** `ComposedFrame` is the
  result *after* disposal + transparency are applied, so its `pixels` is always
  `width * height` for the full canvas. There is **no offset field** — and you don't need
  one. The encoder derives per-frame dirty rectangles and offsets itself from successive
  full-canvas frames. (If you only need the bytes, `frame.as_bytes()` gives a zero-copy
  `&[u8]` RGBA view.)
- **Loop count.** Read it from `meta.repeat` after decode and pass it to
  `EncoderConfig::repeat(..)`. In the streaming `Decoder` path the NETSCAPE loop extension
  is parsed *during* frame iteration, so `decoder.metadata().repeat` (and `decoder.repeat()`)
  is only final after you've read the frames; `decode_gif` reads them all for you, so its
  returned `meta.repeat` is already correct.
- **Timing.** Each `ComposedFrame.delay` is in centiseconds (1/100 s); `FrameInput.delay`
  uses the same unit, so timing is preserved exactly when you copy the field across.
- **For large/streaming inputs**, swap `decode_gif` for `Decoder::new(reader, limits, &stop)`
  and pull frames with `next_frame()` instead of materializing the whole `Vec`. Read the
  loop count *after* the iteration completes.

### `ComposedFrame` fields

`decoder.next_frame()` / `decode_gif` yield `ComposedFrame`:

| Field | Type | Meaning |
|-------|------|---------|
| `index` | `usize` | 0-based frame index |
| `width` | `u16` | **Canvas** width (not a sub-frame width) |
| `height` | `u16` | **Canvas** height |
| `delay` | `u16` | Frame delay in centiseconds (1/100 s) |
| `pixels` | `Vec<Rgba>` | Full-canvas composited RGBA, length `width * height` |
| `palette` | `Option<Palette>` | Effective palette (local if present, else global) — handy for pass-through re-encoding |

Because `pixels` is full-canvas (disposal + transparency already applied), you size a
`FrameInput` with the same `width`/`height` and never deal with frame offsets on the
re-encode side.

## What zengif adds over `gif`

zengif wraps the well-maintained [`gif`](https://crates.io/crates/gif) crate and layers on
the pieces a service handling untrusted GIF uploads needs:

- **Frame compositing built in** — every decoded frame arrives as a full-canvas RGBA buffer
  with disposal methods and 1-bit transparency already applied. (The `gif` crate exposes raw
  sub-frames; [`gif-dispose`](https://crates.io/crates/gif-dispose) is the usual companion
  for compositing — zengif folds that step in.)
- **Bounded, tracked memory** — header-first validation rejects oversized inputs *before*
  allocation, every large allocation is fallible, and usage is counted against a configurable
  ceiling and exposed via `Stats`.
- **Cooperative cancellation** — stop a decode or encode mid-stream when a client disconnects,
  via the [`enough`](https://crates.io/crates/enough) `Stop` trait.
- **Error tracing** — every error carries its `file:line` capture site (via
  [`whereat`](https://crates.io/crates/whereat)) for structured production logs.
- **Quantized encoding** — pluggable palette quantizers with frame differencing and
  shared-palette modes for small animated output, plus an automatic byte-exact fast path for
  grayscale content.

## Memory protection

**`Limits::default()` is bomb-protected, not unbounded.** zengif's `Limits::default()`
already enforces server-safe ceilings, so the quick-start examples above are guarded out of
the box — you do not have to opt in to protection. The defaults are:

| Limit | `Limits::default()` value |
|-------|---------------------------|
| Max dimensions | 16384 × 16384 |
| Max total pixels | 120 megapixels |
| Max frame count | 10,000 |
| Max file size | 100 MB |
| Max memory | 1 GB |
| Max decompression ratio (zip-bomb guard) | 1000× |
| Max animation duration | unbounded (`None`) |
| Max output bytes | unbounded (`None`) |

For an untrusted-GIF proxy you almost certainly want **tighter** caps than the defaults.
Start from `Limits::default()` and clamp down — every setter is a `#[must_use]` chainable
builder:

```rust
use zengif::Limits;

let limits = Limits::default()
    .max_dimensions(4096, 4096)       // Reject huge canvases
    .max_frame_count(1000)            // Limit animation length
    .max_memory(256 * 1024 * 1024)    // 256 MB peak memory
    .max_animation_ms(30_000);        // Reject >30s animations (off by default)
```

The decoder rejects oversized dimensions **from the header, before allocating** (via
`pre_validate_header`), and enforces the memory/frame-count/decompression-ratio caps as
each frame is read.

`Limits::none()` opts out of all bounds — **only for trusted inputs.** Never hand
`Limits::none()` to data you didn't produce.

`Limits` is `#[derive(Clone)]` (and `Debug`, `#[non_exhaustive]`), so you can build one
posture once and reuse it for both decode and encode:

```rust
use zengif::Limits;

let limits = Limits::default().max_dimensions(4096, 4096);
let decode_limits = limits.clone();   // for Decoder::new / decode_gif
let encode_limits = limits;           // for EncodeRequest::limits
```

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

## Error diagnostics

When something goes wrong, you get the full story:

```
Error: InvalidFrameBounds { frame_left: 0, frame_top: 0, frame_width: 5000,
                            frame_height: 5000, canvas_width: 100, canvas_height: 100 }
   at src/decode/frame.rs:142:9
      ╰─ validating frame 3
   at src/decode/mod.rs:89:5
      ╰─ in decode_frame
```

To branch on the failure in code, borrow the inner error with `e.error()` and read the
capture site with `e.location()` (`GifError` is `#[non_exhaustive]`, so keep a wildcard arm):

```rust
use zengif::{decode_gif, GifError, Limits, Unstoppable};

// `decode_gif` is the one-shot for in-memory `&[u8]`; the streaming `Decoder::new`
// above takes any `std::io::Read` (wrap a slice with `std::io::Cursor::new(bytes)`).
match decode_gif(gif_bytes, Limits::default(), &Unstoppable) {
    Ok((meta, frames, _stats)) => { /* meta.width, meta.height, frames: Vec<ComposedFrame> */ }
    Err(e) => {
        if let Some(loc) = e.location() {       // whereat capture site (file:line)
            eprintln!("gif decode failed at {}:{}", loc.file(), loc.line());
        }
        match e.error() {
            GifError::Cancelled(_) => { /* a Stop token fired — HTTP 499 */ }
            GifError::DimensionsTooLarge { .. }
            | GifError::TotalPixelsTooLarge { .. }
            | GifError::FileTooLarge { .. }
            | GifError::MemoryLimitExceeded { .. }
            | GifError::DecompressionRatioExceeded { .. }
            | GifError::TooManyFrames { .. } => { /* resource limit — HTTP 413 */ }
            other => eprintln!("malformed GIF: {other:?}"), // HTTP 400
        }
    }
}
```

## Encoding and quantizers

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

When every opaque pixel of a frame is gray (`R == G == B` — document scans, plots, line
art), the encoder takes an **automatic, byte-exact fast path**: it builds the exact 8-bit
gray palette directly and skips the general histogram + k-means search. It engages only at
lossless intent (`quality == 100` / `with_lossless(true)`), is content-detected with one
early-exiting scan, and never costs bytes — color frames fall straight through to the
configured quantizer. See the [grayscale rate/distortion analysis](https://github.com/imazen/zengif/blob/main/benchmarks/grayscale_rd_2026-06-13.md).

### Quantizer options

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

You get core types (`Rgba`, `Limits`, `GifError`, etc.) but not the codec (decode/encode need
`std::io`). Useful when you need to share types between WASM and native code. The core types
are verified to compile for `wasm32-unknown-unknown`.

<!-- crates.io:skip-start -->
## Benchmarks

Benchmark sources live in [`benches/`](https://github.com/imazen/zengif/tree/main/benches)
(`codec.rs`, `decode_bench.rs`); committed results and their provenance are under
[`benchmarks/`](https://github.com/imazen/zengif/tree/main/benchmarks) — see
[`benchmarks/README.md`](https://github.com/imazen/zengif/blob/main/benchmarks/README.md)
for the index and reproduction steps. All runs use runtime SIMD dispatch (**no**
`-C target-cpu=native`).

Measured, committed findings:

- **Grayscale fast path** ([`grayscale_rd_2026-06-13.md`](https://github.com/imazen/zengif/blob/main/benchmarks/grayscale_rd_2026-06-13.md)) —
  on the 28 strictly-grayscale images of the imazen-26 corpus, the gray fast path is
  **byte-for-byte identical to the best lossless backend on all 28** and **~8.5× faster**
  (mean) than the `zenquant` baseline, with byte-exact round-trips. It already sits on the
  lossless optimum (LZW size is invariant to palette order), so there is no lossless byte
  left to recover.
- **Encode peak memory** ([`zengif_encode_mem_2026-06-23.tsv`](https://github.com/imazen/zengif/blob/main/benchmarks/zengif_encode_mem_2026-06-23.tsv)) —
  single-frame VmHWM scales as roughly **1.6 MB + 41.5 B/px** (zenquant / imagequant resource
  profile), the model the `heuristics` resource estimator is calibrated against.

Reproduce the microbenchmarks:

```sh
git clone https://github.com/imazen/zengif && cd zengif
cargo bench --bench codec          # encode/decode groups (zenbench, criterion-compat)
cargo bench --bench decode_bench   # decode-focused groups
```
<!-- crates.io:skip-end -->

## AI-generated code notice

Developed with Claude (Anthropic). Not all code has been manually reviewed. Review critical paths before production use.

## License

zengif itself is **MIT OR Apache-2.0**, at your option.

**Default features pull in AGPL code.** The `zenquant` quantizer (enabled by default) is
AGPL-3.0-or-later. The `imagequant` quantizer is GPL-3.0-or-later
([commercial license available from upstream](https://pngquant.org)). For fully permissive
licensing, disable defaults and use `quantette`, `quantizr`, or `color_quant`:

```toml
zengif = { version = "0.7", default-features = false, features = ["std", "quantette"] }
```

- Apache License 2.0 — [LICENSE-APACHE](https://github.com/imazen/zengif/blob/main/LICENSE-APACHE)
- MIT License — [LICENSE-MIT](https://github.com/imazen/zengif/blob/main/LICENSE-MIT)

## Image tech I maintain

| | |
|:--|:--|
| **Codecs** ¹ | [zenjpeg] · [zenpng] · [zenwebp] · **zengif** · [zenavif] · [zenjxl] · [zenbitmaps] · [heic] · [zentiff] · [zenpdf] · [zensvg] · [zenjp2] · [zenraw] · [ultrahdr] |
| Codec internals | [zenjxl-decoder] · [jxl-encoder] · [zenrav1e] · [rav1d-safe] · [zenavif-parse] · [zenavif-serialize] |
| Compression | [zenflate] · [zenzop] · [zenzstd] |
| Processing | [zenresize] · [zenquant] · [zenblend] · [zenfilters] · [zensally] · [zentone] |
| Pixels & color | [zenpixels] · [zenpixels-convert] · [linear-srgb] · [garb] |
| Pipeline & framework | [zenpipe] · [zencodec] · [zencodecs] · [zenlayout] · [zennode] · [zenwasm] · [zentract] |
| Metrics | [zensim] · [fast-ssim2] · [butteraugli] · [zenmetrics] · [resamplescope-rs] |
| Pickers & ML | [zenanalyze] · [zenpredict] · [zenpicker] |
| Products | [Imageflow] image engine ([.NET][imageflow-dotnet] · [Node][imageflow-node] · [Go][imageflow-go]) · [Imageflow Server] · [ImageResizer] (C#) |

<sub>¹ pure-Rust, `#![forbid(unsafe_code)]` codecs, as of 2026</sub>

### General Rust awesomeness

[zenbench] · [archmage] · [magetypes] · [enough] · [whereat] · [cargo-copter]

[Open source](https://www.imazen.io/open-source) · [@imazen](https://github.com/imazen) · [@lilith](https://github.com/lilith) · [lib.rs/~lilith](https://lib.rs/~lilith)

[zenjpeg]: https://github.com/imazen/zenjpeg
[zenpng]: https://github.com/imazen/zenpng
[zenwebp]: https://github.com/imazen/zenwebp
[zenavif]: https://github.com/imazen/zenavif
[zenjxl]: https://github.com/imazen/zenjxl
[zenbitmaps]: https://github.com/imazen/zenbitmaps
[heic]: https://github.com/imazen/heic
[zentiff]: https://github.com/imazen/zentiff
[zenpdf]: https://github.com/imazen/zenpdf
[zensvg]: https://github.com/imazen/zenextras
[zenjp2]: https://github.com/imazen/zenextras
[zenraw]: https://github.com/imazen/zenraw
[ultrahdr]: https://github.com/imazen/ultrahdr
[zenjxl-decoder]: https://github.com/imazen/zenjxl-decoder
[jxl-encoder]: https://github.com/imazen/jxl-encoder
[zenrav1e]: https://github.com/imazen/zenrav1e
[rav1d-safe]: https://github.com/imazen/rav1d-safe
[zenavif-parse]: https://github.com/imazen/zenavif-parse
[zenavif-serialize]: https://github.com/imazen/zenavif-serialize
[zenflate]: https://github.com/imazen/zenflate
[zenzop]: https://github.com/imazen/zenzop
[zenzstd]: https://github.com/imazen/zenzstd
[zenresize]: https://github.com/imazen/zenresize
[zenquant]: https://github.com/imazen/zenquant
[zenblend]: https://github.com/imazen/zenblend
[zenfilters]: https://github.com/imazen/zenfilters
[zensally]: https://github.com/imazen/zensally
[zentone]: https://github.com/imazen/zentone
[zenpixels]: https://github.com/imazen/zenpixels
[zenpixels-convert]: https://github.com/imazen/zenpixels
[linear-srgb]: https://github.com/imazen/linear-srgb
[garb]: https://github.com/imazen/garb
[zenpipe]: https://github.com/imazen/zenpipe
[zencodec]: https://github.com/imazen/zencodec
[zencodecs]: https://github.com/imazen/zencodecs
[zenlayout]: https://github.com/imazen/zenlayout
[zennode]: https://github.com/imazen/zennode
[zenwasm]: https://github.com/imazen/zenwasm
[zentract]: https://github.com/imazen/zentract
[zensim]: https://github.com/imazen/zensim
[fast-ssim2]: https://github.com/imazen/fast-ssim2
[butteraugli]: https://github.com/imazen/butteraugli
[zenmetrics]: https://github.com/imazen/zenmetrics
[resamplescope-rs]: https://github.com/imazen/resamplescope-rs
[zenanalyze]: https://github.com/imazen/zenanalyze
[zenpredict]: https://github.com/imazen/zenanalyze
[zenpicker]: https://github.com/imazen/zenanalyze
[zenbench]: https://github.com/imazen/zenbench
[archmage]: https://github.com/imazen/archmage
[magetypes]: https://github.com/imazen/archmage
[enough]: https://github.com/imazen/enough
[whereat]: https://github.com/lilith/whereat
[cargo-copter]: https://github.com/imazen/cargo-copter
[Imageflow]: https://github.com/imazen/imageflow
[Imageflow Server]: https://github.com/imazen/imageflow-dotnet-server
[ImageResizer]: https://github.com/imazen/resizer
[imageflow-dotnet]: https://github.com/imazen/imageflow-dotnet
[imageflow-node]: https://github.com/imazen/imageflow-node
[imageflow-go]: https://github.com/imazen/imageflow-go
