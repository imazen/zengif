# API Feedback - zencodecs Integration

**Date:** 2026-02-06
**Context:** Implementing GIF codec adapter in zencodecs (src/codecs/gif.rs)

## Issues Encountered

### 1. Structured Rgba vs flat bytes
**Issue:** `decode_gif()` returns `Vec<ComposedFrame>` where `pixels: Vec<Rgba>`, but zencodecs API uses flat `Vec<u8>` for pixel data
**Solution:** Convert using `flat_map(|rgba| [rgba.r, rgba.g, rgba.b, rgba.a])`
**Feedback:** The structured `Rgba` type is safer and more explicit, but requires conversion when integrating with APIs that use flat byte arrays. Consider providing both:
- Current `Vec<Rgba>` API for type safety
- Optional convenience method like `pixels_as_bytes(&self) -> &[u8]` or `into_bytes(self) -> Vec<u8>`

### 2. Encoder expects Vec<Rgba>
**Issue:** `FrameInput::new()` takes `Vec<Rgba>`, but zencodecs provides flat `&[u8]`
**Solution:** Convert using `chunks_exact(4).map(|c| Rgba::new(c[0], c[1], c[2], c[3]))`
**Feedback:** Same as above - the typed API is good, but a convenience constructor would help:
```rust
impl FrameInput {
    pub fn from_bytes(width: u16, height: u16, delay: u16, rgba_bytes: &[u8]) -> Self {
        // ... conversion logic
    }
}
```

### 3. Compilation error in zengif encoder.rs
**Issue:** Line 7 had unconditional import of feature-gated function: `use super::config::default_buffer_frames;`
**Solution:** Wrapped import in `#[cfg(any(feature = "imagequant", ...))]`
**Feedback:** This was a bug in zengif itself, already fixed upstream.

## Current Implementation

```rust
// Decode - convert Vec<Rgba> to Vec<u8>
let (metadata, mut frames, _stats) = zengif::decode_gif(data, limits, enough::Unstoppable)?;
let frame = frames.remove(0);
let pixels = frame
    .pixels
    .into_iter()
    .flat_map(|rgba| [rgba.r, rgba.g, rgba.b, rgba.a])
    .collect();

// Encode - convert &[u8] to Vec<Rgba>
let rgba_pixels: Vec<zengif::Rgba> = match layout {
    PixelLayout::Rgba8 => pixels
        .chunks_exact(4)
        .map(|c| zengif::Rgba::new(c[0], c[1], c[2], c[3]))
        .collect(),
    PixelLayout::Rgb8 => pixels
        .chunks_exact(3)
        .map(|c| zengif::Rgba::new(c[0], c[1], c[2], 255))
        .collect(),
    // ... BGRA, BGR conversions
};
let frame = zengif::FrameInput::new(width_u16, height_u16, 10, rgba_pixels);
```

## What Worked Well

- **Convenience function:** `decode_gif()` provides everything needed in one call
- **Rich metadata:** `Metadata` struct has all the GIF-specific info (loop count, background color, etc.)
- **Type safety:** `Rgba` struct prevents byte-ordering bugs
- **Clear frame model:** `ComposedFrame` includes all frame-level data (dimensions, delay, pixels, palette)
- **Dimension validation:** Using `u16` for dimensions documents GIF's 65535 limit in the type system

## Recommendations

1. **Add byte array conversions:**
   ```rust
   impl ComposedFrame {
       pub fn into_bytes(self) -> Vec<u8> { /* flat_map */ }
       pub fn pixels_as_bytes(&self) -> &[u8] { /* reinterpret cast */ }
   }

   impl FrameInput {
       pub fn from_rgba_bytes(width: u16, height: u16, delay: u16, bytes: &[u8]) -> Self {
           // ... chunk and convert
       }
   }
   ```

2. **Document the tradeoff:** Add a comment explaining why `Vec<Rgba>` is preferred over `Vec<u8>` (type safety, clarity), but acknowledge the conversion cost for interop.

3. **Consider unsafe cast (carefully):** For zero-copy scenarios, could provide unsafe method to reinterpret `&[Rgba]` as `&[u8]` (same memory layout, just different types). Would need `#[repr(C)]` on `Rgba`.

## Overall Assessment

**Good API design, minor friction on integration.** The structured `Rgba` type is conceptually cleaner than flat bytes, but requires conversion when interfacing with byte-oriented APIs. The conversions are straightforward but add allocation overhead. Consider adding convenience methods for common interop patterns while keeping the type-safe API as the primary interface.
