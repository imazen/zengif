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

## Quick Commands

```bash
just check      # fmt + clippy + test (ALL targets)
just fmt        # format only
just clippy     # clippy with all targets and features
just test       # run tests
just outdated   # check dependency versions
just bench      # run benchmarks
just doc        # generate docs
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
- `gif` (0.14.x) - Base GIF codec (we wrap/extend, may fork if needed)
- `gif-dispose` (5.x) - Reference for disposal implementation (may inline)
- `whereat` - Error tracing
- `enough` - Cancellation support
- `imagequant` (4.x) - High-quality color quantization for encoding

### Optional/Feature-gated
- `gifski` / `gifski-lite` - Reference for high-quality encoding (study, don't depend)
- `wide` + `multiversed` - SIMD acceleration (feature = "simd")

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

(none yet - new project)

## Critical TODOs

### 1. Fallible Allocations (BLOCKING for production)
**Status: NOT IMPLEMENTED**

Currently using infallible `vec![]` and `Vec::new()` throughout (28 occurrences). Per global CLAUDE.md rules, all allocations must be fallible using `try_reserve()` or `Vec::try_with_capacity()`.

Files needing conversion:
- `src/decode/mod.rs` - 5 infallible allocations
- `src/encode/mod.rs` - 7 infallible allocations
- `src/screen.rs` - 10 infallible allocations
- `src/disposal.rs` - 3 infallible allocations
- `src/types.rs` - 2 infallible allocations

Only `src/stats.rs` has one `try_reserve` usage in a helper function.

### 2. File Size Efficiency (NOT IMPLEMENTED)
**Status: Basic encoding only - no optimizations**

Current encoder does NOT:
- Use frame differencing (only encode changed pixels)
- Mark unchanged pixels as transparent between frames
- Compute minimal bounding box for changed regions
- Use optimal disposal method selection
- Apply delta frame encoding

The `previous_frame` field exists but is never used for optimization.

Gifski achieves 30-50% smaller files through these techniques. Our output is likely 2-3x larger than optimal.

### 3. Codec-Corpus Testing (NOT IMPLEMENTED)
**Status: No real-world GIF corpus testing**

Should test against `codec-corpus` crate with:
- Real-world GIF samples
- Edge case files
- Malformed files from the wild
- Performance regression tracking

### 4. RGB/imgref Interop (NOT IMPLEMENTED)
**Status: Custom Rgba type only, no ecosystem interop**

Should add:
- `From`/`Into` traits for `rgb::RGBA8`
- `imgref::ImgVec` / `imgref::ImgRef` support
- Feature-gated to avoid mandatory dependencies

### 5. Decompression Ratio Check (NOT IMPLEMENTED)
**Status: Limits field exists but never checked**

`Limits::max_decompression_ratio` exists but the check is never performed during decode. This leaves zip-bomb protection incomplete.

## Investigation Notes

(none yet - new project)
