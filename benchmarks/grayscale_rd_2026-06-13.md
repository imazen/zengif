# Grayscale fast path — rate/distortion analysis vs all quantizer backends (#4)

**Date:** 2026-06-13 · **Host:** lilith · **Corpus:** `/mnt/v/imazen-26`, the 28
strictly-grayscale images (exact full-resolution `R==G==B`, Pillow). Backends:
`zenquant`, `quantette`, `quantizr`, `imagequant`, `color_quant`.

**Question:** can the native gray fast path reach **0% net compression loss vs
every quantizer backend** without sacrificing fidelity, speed, or correctness?

## Finding 1 — the gray path is the optimal *lossless* encoder (0% loss)

At matched (lossless) fidelity, the gray fast path is **byte-for-byte identical
to the lossless-capable backends** on every image:

```
gray == quantizr@q100   : 28/28
gray == imagequant@q100 : 28/28
gray <= both            : 28/28
```

This is structural, not luck: an 8-bit grayscale image has ≤256 distinct levels,
so the exact gray palette *is* the palette a lossless quantizer converges to;
LZW size is invariant to palette permutation, so the encoded streams match. There
is **no lossless byte left to recover** — the gray path already sits on the
lossless optimum, at ~8.5× the speed (see `grayscale_corpus_2026-06-13.tsv`).

## Finding 2 — the lossy backends win bytes only by discarding fidelity, and an SSE-optimal 1D quantizer cannot cleanly match them

The only backend results smaller than the gray path are **lossy** (`lossless=false`):
`zenquant`, `quantette`, `color_quant`, and `imagequant@q80` drop/merge gray
levels. Closing that gap means going lossy — sliding the fidelity factor the goal
forbids.

It also is **not** cleanly achievable, because **minimizing GIF size ≠ minimizing
SSE.** An exact optimal 1D scalar quantizer (DP, contiguous clusters) does *not*
rate/distortion-dominate the rate-aware backends. Measured (q80):

| image | backend | backend (size @ SSE) | best 1D point at ≤ that SSE | verdict |
|---|---|---|---|---|
| chart | zenquant | 24660 @ 13481 | K=64 → **19001** | 1D dominates (−23%) |
| doc f1040sa | zenquant | 555762 @ 1.57e6 | K=32 → **426516** | 1D dominates (−23%) |
| photo ian | zenquant | 8312043 @ 35075 | only K=256 (SSE 0) → 8322588 | **1D cannot reach** (+0.13%) |
| chart | quantette | 13139 @ 85035 | K=32 → 14971 | 1D loses (+14%) |
| photo ian | imagequant | 2375817 @ 1.47e8 | K=24 → 2416519 | 1D loses (+1.7%) |

The lossy backends sometimes find (size, distortion) points **below** the
SSE-optimal 1D curve — they trade a little SSE for a more LZW-compressible index
field. So no simple scalar quantizer reaches "≤ every backend."

## Conclusion

- **0% compression loss vs every backend at matched fidelity is achieved** and is
  byte-exact-optimal: the gray path equals the best lossless encoder on 28/28.
- Beating the *lossy* backends would require discarding gray levels (sliding
  fidelity) and isn't cleanly possible anyway (SSE-optimal ≠ rate-optimal for
  GIF). It is therefore left out of scope by the "don't let other factors slide"
  constraint.
- No code change improves the lossless result (it is already byte-optimal); the
  property is guarded by `tests/grayscale.rs::gray_path_never_larger_than_lossless_quantizer`.

Reproduction: encode each raw via `encode_gif` (gray path) and
`encode_gif_with_quantizer` per backend at q100/d0 and q80/d0.5; compare bytes and
byte-exact round-trip. Optimal-1D sweep: exact DP k-means over the 256-bin gray
histogram, K ∈ {2..256}, encoded via a K-level gray palette.
