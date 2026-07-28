# Palette-index → RGBA expansion — 2026-07-28

Platform: Apple Silicon (aarch64, NEON), darwin 25.5.0
Bench: `benches/expand_palette.rs` (zenbench, interleaved arms)

`expand_palette_row` / `expand_palette_row_transparent` (`src/screen.rs`) run once per row per
frame during compositing. They had never been measured, and the doc comment claimed the
fixed-16 chunking "lets LLVM unroll or vectorize the inner loop". The vectorize half is false
on ARM: the body is a 256-entry LUT gather and **AArch64 has no gather instruction**. Comment
corrected in the same commit.

## Measured (current implementation)

Throughput is bytes of RGBA written.

| case | opaque | transparent | transparent cost |
|---|---|---|---|
| row1920 / pal16 noise | 9.43 GB/s | 9.01 GB/s | +4.2–5.5% |
| row1920 / pal16 runs8 | 9.67 GB/s | 9.04 GB/s | +6.9–7.6% |
| row1920 / pal64 noise | 10.4 GB/s | 9.51 GB/s | +8.7–10.0% |
| row1920 / pal256 noise | 11.5 GB/s | 10.4 GB/s | +9.4–11.0% |
| row1920 / pal256 runs8 | 10.3 GB/s | 9.35 GB/s | +10.4–11.8% |
| px4096 / pal256 noise | 8.63 GB/s | 7.93 GB/s | +9.1–10.1% |

That is roughly **one pixel per cycle** — the scalar load/store limit for a dependent gather.
There is nothing left to win by restructuring the scalar loop; LLVM's unrolling is already
doing the available work.

## The real win, measured but BLOCKED

For palettes of ≤16 colors — a large share of real GIFs — NEON's `vqtbl1q_u8` does the whole
lookup in one instruction per channel. Deinterleave the first 16 palette entries into R/G/B/A
byte tables once, then per 16 pixels: one `vld1q_u8` of the indices, four `vqtbl1q_u8`, one
`vst4q_u8` to interleave the result back to RGBA.

Prototype measured against the shipping scalar kernel, 1920-px row, 16-color noise, 200k
iterations, arms interleaved to share thermal conditions:

```
bit-identical: true
scalar : 485 ns/row  15.8 GB/s
neon   : 110 ns/row  69.9 GB/s   speedup 4.41x
```

**4.41×, bit-identical.** At 69.9 GB/s it is at this host's single-core memory-bandwidth
ceiling, i.e. it becomes optimal rather than merely faster.

### Why it is not implemented

Two things block it, both deliberate:

1. zengif is `#![forbid(unsafe_code)]` (`src/lib.rs:152`), so raw `core::arch` intrinsics are
   not available here.
2. **magetypes has no table-lookup primitive.** `u8x16` exposes splat/load/min/max/blend/
   compare/shift/bitmask but nothing that maps to `vqtbl1q_u8`. Verified there is no `vqtbl*`
   anywhere in magetypes; the only `_mm_shuffle_epi8` uses are internal to `x86_v3.rs` for an
   unrelated pack.

So the fix is not a zengif change — it is a **missing primitive in magetypes**, and adding one
is a public-API addition to a foundational crate, which needs sign-off.

### What the primitive should be

A 16-entry byte table lookup on `u8x16`. It maps 1:1 to all three target ISAs:

| backend | instruction |
|---|---|
| neon | `vqtbl1q_u8` |
| x86 (ssse3+/v3) | `_mm_shuffle_epi8` / `_mm256_shuffle_epi8` |
| wasm128 | `i8x16_swizzle` |
| scalar | indexed loop |

One contract detail must be pinned rather than inherited: out-of-range indices differ across
ISAs. NEON and wasm zero the lane for index ≥ 16; x86 `pshufb` only zeroes when bit 7 is set
and otherwise takes `idx & 15`. So the primitive should either require indices < 16 as a
documented precondition, or mask on x86 to make zeroing uniform. Picking "must be < 16" keeps
every backend at one instruction.

Beyond GIF this unblocks the whole LUT-indexed kernel class the workspace has been unable to
vectorize on ARM — palette expand, byte transforms, quantization LUTs — which the archmage
examples list as target shapes.

Palettes of 17–64 colors have a NEON-only answer (`vqtbl4q_u8`, 64-byte table, still one
instruction) with no single-instruction x86 equivalent, so they would need a separate
NEON-specific path and are out of scope for a portable primitive.
