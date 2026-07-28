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

## A NEON table-lookup path — MEASURED AND REJECTED

`vqtbl1q_u8` does a 16-entry byte table lookup in one instruction, so for small palettes you
can deinterleave the palette into R/G/B/A byte tables once, then per 16 pixels do one
`vld1q_u8` of the indices, four table lookups, and one `vst4q_u8` to interleave back to RGBA.

Prototyped against the shipping scalar kernel, 1920-px row, 100k iterations, arms interleaved,
output asserted bit-identical at every size:

| palette | scalar | NEON TBL | result |
|---|---|---|---|
| 16 | 20.4 GB/s | 73.4 GB/s | **3.59×** |
| 64 | 26.3 GB/s | 34.8 GB/s | 1.32× |
| 256 | 26.3 GB/s | 5.6 GB/s | **0.21× — 5× SLOWER** |

**Verdict: do not implement.** The win exists only at ≤16 colors and inverts badly at 256.

### Why it collapses above 64

`vqtbl1q_u8` covers a 16-byte table and `vqtbl4q_u8` a 64-byte table, both in one instruction.
There is nothing wider. A 256-entry lookup therefore needs **four** `vqtbl4q_u8` per channel
(each covering one 64-entry block, out-of-range lanes returning zero) OR'd together. That is
16 table registers live *per channel*; across four channels it needs 64, and NEON has 32. The
tables reload from memory every iteration, which costs more than the scalar gather it was
meant to replace.

The 64-colour case fits in registers but only reaches 1.32×, because four `vqtbl4q_u8` still
tie up 16 of the 32 registers and leave little for the rest of the loop.

### Why that settles it

Real-world GIFs are predominantly >64 colours (photographic and video-derived content is
almost always the full 256). The in-repo test corpus is *not* evidence either way — it is
synthetic minimal fixtures, 10 of 20 files being 2-colour, with a single 256-colour file — so
it cannot be used to argue the small-palette case is common.

So the scalar loop is the correct implementation for the case that matters, and the
`#![forbid(unsafe_code)]` + missing-magetypes-primitive blockers are moot: there is nothing
worth unblocking. No new public API was added to magetypes or garb, and the kernel is not
duplicated into zengif/zenpng.

### For the record: what such a primitive would have looked like

(Retained only so a future session does not re-derive it before finding the rejection above.)


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

