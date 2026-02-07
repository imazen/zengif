# zengif - Project Instructions

Server-side GIF codec with zero-trust design, memory bounds, streaming, and full animation transparency support.

See [FEEDBACK.md](FEEDBACK.md) for user feedback log.

## Project Goals

**Primary use case**: Server-side image processing where:
- Input cannot be trusted (zero-trust)
- Memory usage must be bounded and tracked
- Operations must be cancellable
- Errors must be traceable in production
- Animation transparency and disposal must work correctly
- Round-trip encoding must preserve timing and metadata
- Output sizes must be reasonable (not bloated like imageflow's current gif output)
- WASM/no_std deployment (core types available without std)

## Quick Commands

```bash
just check      # fmt + clippy + test (ALL targets)
just fmt        # format only
just clippy     # clippy with all targets and features
just test       # run tests
just outdated   # check dependency versions
just bench      # run benchmarks
just doc        # generate docs
just profile    # run allocation profiler (examples/alloc_profile.rs)

# Fuzzing (requires: cargo install cargo-fuzz && rustup install nightly)
just fuzz              # run main decode fuzzer with dictionary
just fuzz-seeded       # run with seed corpus
just fuzz-streaming    # fuzz streaming decoder
just fuzz-roundtrip    # fuzz encode/decode consistency
just fuzz-limits       # fuzz limits enforcement
just fuzz-timed 3600   # run for 1 hour
just fuzz-list         # list all fuzz targets
just fuzz-build        # build fuzz targets (for CI)

# Profiling (memory/time measurement)
just profile           # run allocation profiler with sweep
just profile-heap      # run with heaptrack (Linux, requires heaptrack)
just profile-view      # view latest heaptrack results
```

## Core Requirements

### 1. Streaming Decode/Encode
- Must not require loading entire file into memory before processing
- Decoder streams frames progressively
- Encoder accepts frames progressively
- Memory usage scales with frame dimensions, not file size

### 2. Complete Animation Support
- **Disposal methods**: Keep, Background, Previous (all three properly implemented)
- **Transparency**: Per-frame transparent index handling
- **Combined**: Disposal + transparency working together correctly
- **Timing**: Frame delays preserved exactly on round-trip
- **Metadata**: Loop count, comments, app extensions preserved

### 3. Memory Bounding & Tracking
- All allocations are fallible (use `try_reserve`, `Vec::try_with_capacity`, etc.)
- Track current and peak memory usage via stats struct
- Configurable limits: max dimensions, max total pixels, max frame count, max file size
- Reject oversized inputs BEFORE allocating (validate dimensions from header first)

### 4. Error Handling
- Use `whereat` crate for production error tracing
- Use `enough` crate for cooperative cancellation support
- Errors must include:
  - Location (file:line:col)
  - Context (what operation was happening)
  - Actionable information (what limits were exceeded, etc.)

### 5. Zero-Trust Security
- Validate all header fields before use
- Check frame bounds against canvas size
- Handle malformed LZW gracefully
- Limit decompression ratio (zip bomb protection)
- Timeout/cancellation support for long operations

## Architecture

```
zengif/
├── src/
│   ├── lib.rs           # Public API, re-exports
│   ├── decode/
│   │   ├── mod.rs       # Streaming decoder
│   │   ├── reader.rs    # Low-level GIF reading
│   │   ├── lzw.rs       # LZW decompression with limits
│   │   └── frame.rs     # Frame parsing
│   ├── encode/
│   │   ├── mod.rs       # Streaming encoder
│   │   ├── writer.rs    # Low-level GIF writing
│   │   ├── lzw.rs       # LZW compression
│   │   ├── quantize.rs  # Color quantization (via imagequant)
│   │   └── frame.rs     # Frame encoding
│   ├── screen.rs        # Compositing screen (disposal + transparency)
│   ├── disposal.rs      # Disposal method implementation
│   ├── error.rs         # Error types with whereat integration
│   ├── limits.rs        # Memory/size limit configuration
│   ├── stats.rs         # Memory tracking statistics
│   └── types.rs         # Common types (Frame, Palette, etc.)
├── tests/
│   ├── round_trip.rs    # Encode -> decode preserves data
│   ├── disposal.rs      # All disposal methods tested
│   ├── transparency.rs  # Transparency handling
│   ├── combined.rs      # Disposal + transparency together
│   ├── limits.rs        # Memory limits enforced
│   ├── malformed.rs     # Malformed input handling
│   └── corpus/          # Test GIF files
├── benches/
│   └── codec.rs         # Decode/encode benchmarks
├── Cargo.toml
├── CLAUDE.md            # This file
├── FEEDBACK.md          # User feedback log
├── justfile             # Build commands
└── README.md            # Public documentation
```

## Dependencies

### Required
- `gif` (0.14.x) - Base GIF codec, no_std+alloc compatible
- `whereat` - Error tracing, no_std compatible
- `enough` - Cancellation support, no_std compatible

### Optional/Feature-gated
- `imagequant` (4.x) - High-quality color quantization (requires std)
- `quantizr` (1.x) - MIT-licensed quantization (requires std)
- `color_quant` (1.x) - Fast quantization (requires std)
- `wide` + `multiversed` - SIMD acceleration (feature = "simd")
- `rgb` / `imgref` - Ecosystem interop

### no_std Support
The `std` feature (default on) controls std dependency. Without it:
- **Available**: types, error (with `core::error::Error`), stats, limits, screen, disposal
- **Unavailable**: decode, encode, quantize, heuristics modules (require std::io)
- **Targets**: verified to compile for `wasm32-unknown-unknown`
- **Requires**: Rust 1.81+ (for `core::error::Error`)

## Reference Implementations to Study

### 1. `gif` crate (0.14.1)
- **Location**: https://github.com/image-rs/image-gif
- **Study**: Low-level GIF parsing, LZW implementation
- **Issues**: Raw frame exposure, no compositing, limited validation

### 2. `gif-dispose` crate (5.0.1)
- **Location**: https://github.com/kornelski/image-gif-dispose
- **Study**: Disposal method implementation, screen compositing
- **Code pattern**: Similar to imageflow's copy (disposal.rs, screen.rs, subimage.rs)

### 3. imageflow's gif handling
- **Location**: `/home/lilith/work/imageflow/imageflow_core/src/codecs/gif/`
- **Files**: mod.rs, disposal.rs, screen.rs, subimage.rs, bgra.rs
- **Study**: Streaming decode pattern, dimension validation
- **Issues to fix**:
  - Animation transparency doesn't work correctly during encoding
  - Output sizes are terrible (no proper quantization)
  - TODO comments indicate incomplete disposal/transparency handling
  - Allocates copy every blit for Previous disposal
  - No proper cancellation support

### 4. gifski (1.34.0)
- **Location**: https://github.com/ImageOptim/gifski
- **Study**: High-quality encoding, dithering, frame differencing
- **Key insight**: Uses imagequant for palette selection

### 5. gifski-lite (1.0.1)
- **Study**: WASM-compatible variant, simpler implementation

## API Design (Draft)

```rust
// Decoding
let limits = DecodeLimits::default()
    .max_dimensions(4096, 4096)
    .max_frame_count(1000)
    .max_file_size(100 * 1024 * 1024);

let stats = Stats::new();
let stop = Stopper::new();

let decoder = GifDecoder::new(reader, limits, &stats, &stop)?;

// Streaming frame iteration
for frame_result in decoder.frames() {
    let frame: ComposedFrame = frame_result?;
    // frame.pixels is RGBA with disposal + transparency applied
    // frame.delay, frame.index available
}

// Encoding
let encoder = GifEncoder::new(writer, width, height, &stats, &stop)?
    .with_repeat(Repeat::Infinite)
    .with_quantizer(QuantizerConfig::default());

for source_frame in frames {
    encoder.add_frame(source_frame)?;
}
encoder.finish()?;

// Round-trip with metadata preservation
let metadata = decoder.metadata();
let encoder = GifEncoder::from_metadata(writer, metadata, &stats, &stop)?;
```

## Error Types

```rust
use whereat::{At, at};

#[derive(Debug)]
pub enum GifError {
    // Decoding
    InvalidHeader,
    InvalidFrameBounds { frame_left: u16, frame_top: u16, frame_width: u16, frame_height: u16, canvas_width: u16, canvas_height: u16 },
    UnsupportedVersion([u8; 3]),
    MalformedLzw(String),
    UnexpectedEof,

    // Limits
    DimensionsTooLarge { width: u16, height: u16, max_width: u16, max_height: u16 },
    TooManyFrames { count: usize, max: usize },
    FileTooLarge { size: u64, max: u64 },
    MemoryLimitExceeded { current: usize, limit: usize },
    DecompressionRatioExceeded { ratio: f64, max_ratio: f64 },

    // Encoding
    FrameDimensionMismatch,
    QuantizationFailed(String),

    // I/O
    Io(std::io::Error),

    // Cancellation
    Cancelled,
}

pub type Result<T> = std::result::Result<T, At<GifError>>;
```

## Memory Tracking Pattern

```rust
pub struct Stats {
    current_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
}

impl Stats {
    pub fn track_alloc(&self, bytes: usize) { ... }
    pub fn track_dealloc(&self, bytes: usize) { ... }
    pub fn current(&self) -> usize { ... }
    pub fn peak(&self) -> usize { ... }
}

// Usage in code:
fn allocate_buffer(size: usize, stats: &Stats) -> Result<Vec<u8>> {
    stats.track_alloc(size);
    let mut buf = Vec::new();
    buf.try_reserve(size).map_err(|_| at!(GifError::MemoryLimitExceeded { ... }))?;
    Ok(buf)
}
```

## Testing Strategy

### Unit Tests
- Each disposal method independently
- Transparency index handling
- LZW edge cases
- Limit enforcement

### Integration Tests
- Round-trip: decode → encode → decode matches
- Real-world GIFs from test corpus
- Malformed input fuzzing

### Property Tests
- Any valid GIF round-trips correctly
- Memory never exceeds configured limits
- Cancellation stops processing promptly

### Test Corpus
Gather test GIFs demonstrating:
- All disposal methods
- Transparency
- Large dimensions
- Many frames
- Edge cases (1x1, 1 frame, etc.)

## Fuzzing

Fuzz testing infrastructure is in `fuzz/`. See `fuzz/README.md` for full documentation.

### Fuzz Targets

| Target | Description |
|--------|-------------|
| `fuzz_decode` | Full decode path via `decode_gif()` |
| `fuzz_decode_streaming` | Streaming decode via `Decoder` frame iteration |
| `fuzz_roundtrip` | Encode → Decode consistency |
| `fuzz_limits` | Limits enforcement with arbitrary configurations |

### Quick Start

```bash
cargo install cargo-fuzz
rustup install nightly
just fuzz              # Main target with GIF dictionary
just fuzz-seeded       # With seed corpus
just fuzz-timed 3600   # Run for 1 hour
```

### Corpus

Seed corpus in `fuzz/corpus/seed/` includes:
- All test GIFs from `tests/corpus/`
- Dimension bombs and malformed inputs
- Minimal valid GIFs for edge cases

External corpora can be downloaded via `just fuzz-download-corpus`:
- [dvyukov/go-fuzz-corpus](https://github.com/dvyukov/go-fuzz-corpus/tree/master/gif)
- [peterdn/gif-test-suite](https://github.com/peterdn/gif-test-suite)

### Dictionary

`fuzz/gif.dict` contains GIF-specific tokens:
- Magic headers, block types, extension labels
- LZW code sizes, disposal method patterns
- Common dimension and delay values

### Known CVEs Targeted

| CVE | Issue | Relevant Target |
|-----|-------|-----------------|
| CVE-2025-27598 | OOB write from crafted frame length | `fuzz_limits` |
| CVE-2021-44648 | Heap overflow with LZW min code = 12 | `fuzz_decode` |
| CVE-2019-15133 | Divide by zero with height = 0 | `fuzz_decode` |

## Performance Targets

- Decode: > 100 MB/s uncompressed throughput
- Encode: > 50 MB/s (limited by quantization)
- Memory overhead: < 2x frame size (screen + previous frame buffer)
- Cancellation latency: < 1ms

## Known Issues to Avoid

From studying imageflow's implementation:

1. **Don't clone palette on every frame** - Use reference or Arc
2. **Don't allocate Previous buffer unless needed** - Only for Previous disposal
3. **Handle missing palette gracefully** - Some frames use global, some local
4. **Validate frame bounds** - Frames can be positioned outside canvas (clip them)
5. **Don't trust delay values** - Clamp to reasonable range (1-65535 centiseconds)

## CI/CD

GitHub Actions workflow must include:
- Ubuntu, Windows, macOS (x64 + arm64)
- Clippy with `-D warnings`
- Format check
- All tests
- Code coverage upload (codecov)
- Benchmarks via Bencher

## License

MIT OR Apache-2.0 (dual license)

## Known Bugs

(none critical)

### Note: LZW Cancellation Latency (Documented Limitation)

The gif crate's LZW decoder cannot be cancelled mid-decompression. However, this is adequately mitigated:

1. **Dimension limits** bound maximum output per frame (CPU work is proportional to output size)
2. **gif crate's `check_frame_consistency(true)`** validates frame bounds against canvas
3. **gif crate's `set_memory_limit()`** provides additional bounds
4. **Pre-validation** rejects oversized dimensions before gif crate allocates

With default 16384x16384 max dimensions, worst-case per-frame CPU time is bounded. For tighter server limits, reduce `max_dimensions` (e.g., 4096x4096).

## Current Implementation Status (for context reset)

**DONE:**
- ✅ Streaming decode/encode API (`Decoder`, `Encoder` in src/decode/mod.rs, src/encode/mod.rs)
- ✅ All disposal methods (Keep, Background, Previous) in src/disposal.rs, src/screen.rs
- ✅ Transparency handling during decode
- ✅ Memory tracking via `Stats` (src/stats.rs)
- ✅ Configurable `Limits` (src/limits.rs)
- ✅ Cancellation via `enough` crate
- ✅ Error tracing via `whereat` crate
- ✅ Pre-validation of GIF headers before gif crate allocation
- ✅ 65 passing tests (unit, cancellation, malformed, round-trip)
- ✅ Basic example in examples/basic.rs
- ✅ Fallible allocations with try_reserve() throughout
- ✅ no_std support (core types available without std, decoder/encoder require std)

**NOT DONE (nice to have):**
- ✅ Pngquant/imagequant frame optimization (P0) - DONE 2026-01-17
- ✅ Frame differencing for small output - DONE 2026-01-17
- ✅ Decompression ratio check (P4) - DONE 2026-01-17
- ❌ Codec-corpus testing (P2)
- ✅ RGB/imgref interop (P3) - DONE 2026-01-17
- ✅ NETSCAPE extension parsing - DONE 2026-01-17
- ⚠️ Comment extraction - BLOCKED (gif crate doesn't expose comment extensions)

**Key files:**
- `src/decode/mod.rs` - Streaming decoder with `pre_validate_header()` for zero-trust
- `src/encode/mod.rs` - Encoder with frame differencing via `compute_frame_diff()`
- `src/disposal.rs` - Disposal state machine
- `src/screen.rs` - Canvas compositing
- `src/stats.rs` - Memory tracking with `try_alloc()` helper

## Critical TODOs (PRIORITIZED)

### P0: Pngquant/Imagequant Integration ✅ DONE
**Status: IMPLEMENTED (2026-01-17)**

Frame differencing is now implemented in `src/encode/mod.rs`:
- `compute_frame_diff()` compares current and previous frames
- Finds minimal bounding box of changed pixels
- Marks unchanged pixels within the region as transparent
- Sets frame offset (left, top) for cropped region
- Both quantized and simple paths use the optimization

Tests added:
- `frame_diff_finds_changed_region`
- `frame_diff_marks_unchanged_as_transparent`
- `frame_diff_no_changes`
- `frame_diff_full_change`
- `frame_diff_produces_smaller_output`

### P1: Fallible Allocations ✅ DONE
**Status: IMPLEMENTED (2026-01-17)**

All large allocations now use `try_reserve()` with proper error handling:
- `src/decode/mod.rs` - pixel buffer, frame data copy, decode_all vector
- `src/encode/mod.rs` - output buffer pre-allocation
- `src/screen.rs` - canvas initialization, composed frame pixels
- `src/disposal.rs` - saved pixels for Previous disposal

Pattern used: `stats.try_alloc()` check + `vec.try_reserve()` + `AllocationFailed` error on failure.

Note: `src/types.rs` Palette allocations remain infallible as palettes are bounded to 256 colors (~1KB max).

### P2: Codec-Corpus Testing ✅ DONE
**Status: IMPLEMENTED (2026-01-17)**

Tests in `tests/corpus.rs` validate against real-world GIF files (local copies in tests/corpus/codec-corpus/):
- 11 GIF files tested (image-rs test-images + imageflow inputs)
- Decode all: verifies all files decode without panicking
- Round-trip: animated GIFs preserve frame count, dimensions, delays
- Disposal methods: any-disposal.gif, mixed-disposal.gif
- Interlaced: interlaced.gif handling
- Large animation: large-gif-anim-full-frame-replace.gif with memory tracking
- Memory limits: enforced correctly
- Transparency: alpha_gif_a.gif handling

Also fixed buffer capacity bug discovered during testing: frames can have different dimensions than canvas, so buffer is now resized dynamically via `ensure_buffer_capacity()`.

### P3: RGB/imgref Interop ✅ DONE
**Status: IMPLEMENTED (2026-01-17)**

Added feature-gated interop:
- `rgb-interop` feature: From/Into for `rgb::RGBA8` and `rgb::RGB8`
- `imgref-interop` feature: `ComposedFrame::into_imgvec()`, `as_imgref()`, `FrameInput::from_imgvec()`, `from_imgref()`

### P4: Decompression Ratio Check ✅ DONE
**Status: IMPLEMENTED (2026-01-17)**

Added `CountingRead` wrapper in decoder to track compressed bytes read.
After each frame, checks ratio against `Limits::max_decompression_ratio` (default 1000x).
Returns `DecompressionRatioExceeded` error if exceeded.

Tests added: `decompression_ratio_check`, `decompression_ratio_ok`

### P5: Color Space / Linear Alpha Blending ⚠️ NOT NEEDED
**Status: sRGB blending is correct for GIF**

Analysis:
- GIF only has 1-bit transparency (via transparent color index)
- The main decode path (`blit_indexed`) doesn't do alpha blending - just copies non-transparent pixels
- The `alpha_blend()` in `disposal.rs` is only used for `blit_rgba()` (RGBA frame compositing)
- sRGB blending is actually correct for GIF-to-GIF operations since all pixels are in sRGB space
- Linear blending would only matter for compositing GIF onto external content, which is out of scope

Note: `linear-srgb` crate exists at `~/work/linear-srgb` if needed for future extensions.

## Investigation Notes

### Gifski Frame Optimization Algorithm ✅ IMPLEMENTED

Frame differencing now implemented in `compute_frame_diff()` in `src/encode/mod.rs`.
See P0 section above for details.

### Pngquant Integration Points ✅ ENHANCED (2026-01-17)

Round-trip bloat/degradation causes identified and fixed:

**Root causes:**
1. Per-frame palettes → flickering + poor compression
2. Full dithering (1.0) → noise that compresses poorly
3. No palette preservation → re-quantization artifacts

**Solutions implemented:**

1. **Quantizer abstraction** (`src/quantize.rs`):
   - `Quantizer` trait for pluggable quantization backends
   - `ImagequantQuantizer` implementation using imagequant
   - `encode_gif_with_quantizer()` accepts any Quantizer implementation
   - Prepares for future custom quantizer

2. **Frame-aware transparency** via `set_background()`:
   - Uses imagequant's `Image::set_background()`
   - Pixels matching previous frame after quantization → transparent
   - Smarter than manual pixel comparison (considers quantization)
   - Won't dither areas that will become transparent

3. **Encoder config options** (`src/encode/mod.rs`):
   - `encode_gif_shared_palette()` - shared palette via Histogram
   - Configurable `dithering` (0.0-1.0), default 0.5 (was 1.0)
   - `EncoderConfig::for_round_trip()` - zero dithering + shared palette
   - `PaletteStrategy` enum: PerFrame, Shared, Global

**Still future enhancements:**
- Temporal dithering: spread error across frames (opt-in only)
- Random vs deterministic dithering option
- Custom quantizer implementation (API ready)

### Buffer Capacity Bug ✅ FIXED (2026-01-17)

Discovered during corpus testing: GIF frames can have different dimensions than the canvas.
The pixel buffer was initially sized to canvas dimensions, but frames could be larger,
causing a panic when accessing beyond buffer bounds.

Fix: Added `ensure_buffer_capacity()` in `src/decode/mod.rs` that:
1. Checks if current buffer is large enough for the frame
2. If not, tracks the additional allocation via stats
3. Uses `try_reserve()` for fallible allocation
4. Resizes the buffer to accommodate the frame

This maintains the memory tracking invariants while handling variable-size frames.

## Next Session Checklist

1. Read this CLAUDE.md first
2. Run `cargo test` to verify state (80+ tests should pass)
3. Check `git log --oneline -5` for recent commits
4. All prioritized TODOs (P0-P5) are now complete
5. Future enhancements: temporal dithering, frame-to-frame palette consistency
6. Comment extraction blocked by gif crate limitation (won't fix)

## API Convergence TODOs

See `/home/lilith/work/zendiff/API_COMPARISON.md` for full cross-codec comparison.

**Three-layer pattern: EncoderConfig → EncodeRequest<'a> → Encoder (streaming only)**

Done:
- [x] Dimensions out of config ✓
- [x] `EncodeError`/`DecodeError` aliases ✓
- [x] `At<>` error wrapping ✓
- [x] `Limits` struct ✓
- [x] `u16` dimensions (GIF format limit, compile-time enforcement) ✓
- [x] `finish()`/`finish_into()` on streaming encoder ✓

Remaining:
- [ ] Add `EncodeRequest<'a>` intermediate layer
- [ ] Drop `S: Stop` generic → `&dyn Stop` on request
- [ ] Drop `W: Write` generic → output method on finish
- [ ] Add one-shot `request.encode()` convenience
- [ ] Rename `GifError` → `EncodeError`/`DecodeError` as primary types (not just aliases)
- [ ] `Limits` fields: standardize to `Option<u64>` (currently mixed `u16`/`usize`)
- [ ] Same pattern for decoder: `DecodeRequest<'a>` + `Decoder`

