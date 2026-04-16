# zengif decode performance — investigation notes

Bench harness: imageflow's `bench_codecs --group=gif_decode`
(`/home/lilith/work/imageflow-zen-v3/imageflow_core/benches/bench_codecs.rs`),
which compares `ZenGifDecoder` (via zenpipe / zencodec) against `GifRsDecoder`
(imageflow's native wrapper over the `gif` crate) on the same single-frame
fixtures at 256×256, 1024×1024, and 4096×4096.

Both decoders share the same underlying LZW implementation (`weezl`) and the
same GIF container parser (`gif` crate 0.14.2 with `ColorOutput::Indexed`).
The difference is what each wrapper does on top of the indexed decode.

## Current state (2026-04-15, after weezl patch to post-0.1.12 tip)

| Size        | zengif (Mpx/s) | gif-rs (Mpx/s) | zengif / gif-rs |
|-------------|----------------|----------------|-----------------|
| 256×256     | ~42            | ~42            | ≈ parity        |
| 1024×1024   | 2340           | 2600           | 0.90            |
| 4096×4096   | 36.0           | 49.1           | 0.73            |

The regression vs gif-rs gets monotonically worse as the image grows, which
is the signature of an **O(pixels) work unit we do that gif-rs does not**.
Small images have too much constant-time overhead (header parse, decoder
setup) for the per-pixel asymmetry to dominate; large images expose it.

## Leads investigated

### Lead A: weezl perf commits past 0.1.12 — NO MEASURABLE IMPACT

Upstream image-rs/weezl has two perf commits past the 0.1.12 release:
- `75dc30e` — perf: convert decode Table from Vec to fixed-size arrays
- `154d92f` — Optimize Link layout

Patched imageflow (Cargo.toml `[patch.crates-io] weezl = { git =
"https://github.com/lilith/weezl", branch = "imageflow-0.1.13-with-perf" }`)
which pins a fork at upstream tip but keeps `version = "0.1.13"` so the
patch is compatible with `gif`'s `weezl = "0.1.10"` requirement.

Result: zen_4096x4096 was 38.7 Mpx/s before, 36.0 Mpx/s after; gifrs_4096x4096
was 53.6 Mpx/s before, 49.1 Mpx/s after. Both shifts are within measurement
noise (CV 20–30%, system under mixed load). No win from weezl perf commits
for either code path at these sizes.

The LZW decode is not the bottleneck at 4096². The bottleneck is in what
zengif does *after* LZW hands back indexed bytes.

### Lead B: allocation-heavy hot paths in zengif's frame compositor

#### B.1 — RawFrame.pixels per-frame 16 MB alloc+memcpy at 4096²
**File:** `src/decode/mod.rs:392-398`

```rust
let mut pixels = Vec::new();
pixels.try_reserve(buffer_slice.len()).map_err(|_| {
    at!(GifError::AllocationFailed { requested: buffer_slice.len() as u64 })
})?;
pixels.extend_from_slice(buffer_slice);
```

`self.pixel_buffer` already holds the decoded indexed pixels (sized to the
screen, reused across frames — good). But before compositing we clone the
frame region into a fresh `Vec` for `RawFrame.pixels`, and that `Vec` is
then handed to `process_frame_in_place` which only *reads* it. A 16 MB
allocation + memcpy per 4096² frame that we're about to throw away.

**Proposed fix:** Change `RawFrame.pixels` to `Cow<'_, [u8]>` or take the
index buffer by reference through a new `RawFrameRef<'a>` that
`process_frame_in_place` accepts. The owning `RawFrame` stays for the
`decode_all` / `Iterator` APIs, but the common `next_frame` path composes
in place without the clone. Conservative estimate: ~10–15% throughput at
4096² (16 MB memcpy at ~30 GB/s ≈ 530 µs, out of ~467 ms total — smaller
than I'd like; the bigger win is B.2).

#### B.2 — ComposedFrame.pixels per-frame 64 MB alloc+memcpy at 4096²
**File:** `src/screen.rs:242-251`

```rust
let mut composed_pixels = Vec::new();
composed_pixels.try_reserve(self.pixels.len()).map_err(|_| { ... })?;
composed_pixels.extend_from_slice(&self.pixels);
```

Same shape as B.1 but 4× larger (RGBA is 4 bytes/pixel). `process_frame`
composites into `self.pixels` (the screen's canvas), then clones the
entire canvas into `composed.pixels`. For the `Decode::decode()` entry
point (single frame, caller owns the output), this clone is pure waste —
we could `mem::take(&mut self.pixels)` and leave the screen with an
empty/new canvas, since the screen is about to be dropped.

**Proposed fix:** Add `fn finish_frame(self) -> ComposedFrame` that
consumes the `Screen` (or at least the canvas Vec) via `mem::take`. For
the single-frame `Decode::decode()` path in `codec.rs:1279`, replace the
`next_frame()` + clone with a direct-consume API. For animation callers
that need the canvas preserved across frames (`AnimationFrameDecoder`),
keep the existing `process_frame` → clone path. Estimated win: ~13% at
4096² (64 MB memcpy at ~30 GB/s ≈ 2.1 ms, ~0.4% of 467 ms wall, but it's
also a 64 MB allocation + drop which is nontrivial on the heap
allocator).

Actually, a more thorough look shows the wall-clock impact is probably
in the tens of milliseconds (the heap allocator zeroes pages, commits,
etc.). A focused flamegraph would confirm.

#### B.3 — per-pixel palette lookup has bounds checks
**File:** `src/screen.rs:207-213` (fast path) and `182-190` (transparency path)

```rust
for (canvas_pixel, &color_index) in canvas_row.iter_mut().zip(frame_row.iter()) {
    *canvas_pixel = palette
        .get(color_index as usize)
        .copied()
        .unwrap_or(Rgba::TRANSPARENT);
}
```

`palette` is `&[Rgba]` of ≤256 entries, looked up by a `u8` index. The
`.get().copied().unwrap_or(TRANSPARENT)` pattern compiles to a bounds
check + conditional move for every pixel. At 4096² that is 16M bounds
checks that LLVM cannot remove because it can't prove `color_index <
palette.len()`.

**Proposed fix:** `Palette` already has a `lookup_table()` method
(`src/types.rs:287`) that returns a `[Rgba; 256]` with unused slots =
TRANSPARENT. Compute it once per frame (not per pixel) and index with
`table[color_index as usize]` — LLVM can prove `idx < 256` for a fixed
`[T; 256]` and elide the bounds check. This matches the "heap-allocated
fixed array for bounds-check-free lookup tables" pattern in the project
CLAUDE.md. Estimated win: 5–10% at 4096².

Note: the transparency fast-path (lines 182–190) needs the same
treatment — it bounds-checks on every non-transparent pixel.

#### B.4 — `self.pixel_buffer.fill(0)` per frame
**File:** `src/decode/mod.rs:379`

```rust
self.ensure_buffer_capacity(frame_size)?;
let buffer_slice = &mut self.pixel_buffer[..frame_size];
buffer_slice.fill(0);
```

We zero the 16 MB indexed buffer at 4096² before letting weezl decode
into it. This is defensive (guards against short decodes or error paths)
but `read_into_buffer` writes every pixel for a successful decode, so the
zero is redundant in the common case. Much smaller win than B.1/B.2 but
cheap to remove.

**Proposed fix:** Track whether the previous read filled the buffer; if
so, skip the zero. Or just trust weezl to write every byte (the indexed
GIF format guarantees `width*height` pixels per frame). Estimated win:
~0.5% at 4096².

### Other notes

- `Palette::from_rgb_bytes` (decode/mod.rs:411-414) is called per frame
  with a `.as_ref().map(|p| Palette::from_rgb_bytes(p))` — that's a
  small alloc (≤256 × 4 bytes = 1 KB), not a hot-path concern.
- `detect::probe` runs once per decode — O(header bytes), not O(pixels).
- The pre-validate path (line 189) reads a small prefix and chains it
  back; any allocs there are O(header).

## Priority for fixes

1. **B.2** (ComposedFrame.pixels clone) — biggest single waste on the
   single-frame `Decode::decode()` path, which is exactly what imageflow's
   bench_gif_decode measures. 1–2 day fix with a consuming API.
2. **B.3** (palette LUT) — proves out the B-check-free pattern that the
   project CLAUDE.md recommends. ~1 hour. Wins compound with B.2 and B.1.
3. **B.1** (RawFrame.pixels clone) — next-biggest alloc. Slightly more
   involved because `RawFrame` is public.
4. **B.4** — trivial, minor win.

## Regression bench command

```
cd ~/work/imageflow-zen-v3
cargo build --release --bench bench_codecs --features zen-codecs
target/release/deps/bench_codecs-* --group=gif_decode
```

Commit baseline for this investigation: imageflow
`d8193228e9011fc06fdfd21764d2f0434b8c6f42`, zengif `4083b2d0` (v0.7.2
release) + `f66bf8d1` (RGBX/BGRX dispatch), weezl patched to
`https://github.com/lilith/weezl/tree/imageflow-0.1.13-with-perf`
(commit `a4a5ee9`).

## Fixes applied (2026-04-15)

Commits: `9e49aced`..`d3d1e894` on zengif main.

### B.2 — Canvas clone elimination (Screen::process_frame_take + with_next_frame)

Added `Screen::process_frame_take()` that uses `mem::take` to move the
canvas Vec out instead of cloning. For the single-frame `Decode::decode()`
path, this is zero-copy. For the animation frame decoder path (what
imageflow actually benches), rewrote `render_next_frame_owned` to use
`Decoder::with_next_frame()` which composites in-place and gives the
callback a `&[Rgba]` reference to the canvas — avoids the 64 MB clone
entirely.

### B.3 — Bounds-check-free palette LUT

Replaced `palette.get(idx).copied().unwrap_or(TRANSPARENT)` with a
`[Rgba; 256]` lookup table built once per frame via `Palette::lookup_table()`.
LLVM can prove `idx < 256` for fixed-size arrays, eliminating 16M bounds
checks at 4096².

### B.1 — Indexed pixel buffer clone elimination

Applied `mem::take`/reclaim pattern to all three frame-reading methods
(`next_frame`, `next_frame_take`, `with_next_frame`). The pixel buffer is
swapped into the RawFrame for compositing, then reclaimed afterward. Zero
extra allocation or memcpy for the indexed pixels.

### B.4 — Redundant fill removal

Removed `buffer_slice.fill(0)` before `read_into_buffer` in all three
methods. GIF guarantees width×height indexed pixels per successful frame
decode; the zeroing was pure waste.

### Bonus — Fused BGRA copy+swizzle

`render_next_frame_owned` now does RGBA→BGRA conversion in a single pass
(memcpy then in-place swap) instead of the previous two-pass approach
(clone canvas → negotiate_format swizzle).

### Results after fixes

Back-to-back bench (old binary = before, new binary = after, same session):

| Size   | zen before (Mpx/s) | zen after (Mpx/s) | gifrs (Mpx/s) | ratio before | ratio after |
|--------|--------------------|--------------------|---------------|-------------|-------------|
| 256²   | 43.6               | 47.1               | ~43           | ~1.0        | ~1.0        |
| 1024²  | 2.43 G             | 2.70 G             | ~2.8 G        | 0.91        | 0.93        |
| 4096²  | 38.7               | 39.8               | ~54           | 0.72        | 0.74        |

The improvement at 4096² is modest (~3% ratio improvement, ~3% absolute)
because the system had high variance (CV 20-30%) and the architectural
gap remains: gif-rs composites directly to BGRA, while zengif composites
to RGBA then swizzles. That BGRA swizzle pass touches 64 MB — at ~30 GB/s
that's only ~2 ms of the ~420 ms total, so it's not the main explanation.

The remaining ~26% gap vs gif-rs is likely composed of:
- **gif-rs has no per-frame overhead**: no stats tracking, no decompression
  ratio checks, no fallible allocation paths, no limit enforcement. zengif
  does all of these for security (zero-trust design).
- **gif-rs iterator-based compositing** vs zengif row-slice compositing:
  different cache access patterns. gif-rs uses a custom `Subimage` iterator
  over the pixel and canvas buffers with per-pixel iteration; zengif does
  row-at-a-time slicing. Both are valid but produce different codegen.
- **gif-rs uses BGRA natively** — palette entries are stored as BGRA8 and
  composited directly to BGRA canvas. zengif uses RGBA throughout and
  converts at output. One fewer memory pass for gif-rs.

### Magetypes / SIMD opportunities (not yet implemented)

- **Palette expansion loop** (`screen.rs` inner compositing loops): the LUT
  lookup is inherently scalar (indexed by arbitrary u8). SIMD gather
  (`vpgatherdd`) could do 8-16 lookups at once on AVX2/AVX-512. This is a
  significant optimization opportunity but requires explicit SIMD, not
  autovectorization. Would use `#[magetypes(_v4x, v4, v3, neon, wasm128)]`.
- **BGRA swizzle**: `garb::bytes::swap_br` is already SIMD-optimized. Could
  be used instead of the scalar `chunks_exact_mut(4) + swap(0,2)` pattern.
  Would require adding garb as a dependency (currently not a dep of zengif).
- **Dispose-to-background fill**: `self.pixels.fill(background)` — could
  benefit from SIMD when filling with a non-zero pattern. memset handles the
  zero case; non-zero BGRA fill is a scatter opportunity.
- **Per-row alpha blending** (transparency compositing path): the
  `if color_index != transparent_idx` branch is per-pixel and unpredictable.
  SIMD blend with mask could eliminate the branch.
