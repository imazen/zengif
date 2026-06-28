# zengif benchmarks

Committed benchmark results and their provenance. Each file records the date, host,
and source commit it came from; this index summarizes what each one measures and how
to reproduce it.

**Integrity rules these follow** (see `~/work/claudehints/topics/benchmarking.md`):

- Built and run **without** `-C target-cpu=native` — runtime SIMD dispatch is what
  ships, so that is what we measure.
- I/O is excluded from timed regions (corpus is loaded into RAM before the measured
  loop; encode writes into a `Vec<u8>`).
- Real, named corpora — no synthetic gradients, no Kodak overfit.
- No fabricated or extrapolated numbers; memory is read from VmHWM / heaptrack, not
  scaled from another size.

The runnable microbenchmarks live in [`../benches`](../benches) (`codec.rs`,
`decode_bench.rs`) and use [zenbench](https://github.com/imazen/zenbench) in
criterion-compatible mode:

```sh
git clone https://github.com/imazen/zengif && cd zengif
cargo bench --bench codec
cargo bench --bench decode_bench
```

These compare zengif's own paths and quantizer backends against each other and track
resource use; they are not a rate/distortion shoot-out against other GIF crates.

---

## Grayscale fast path (issue #4)

When every opaque pixel of a frame is gray (`R == G == B`), the encoder builds the
exact 8-bit gray palette directly and skips the general histogram + k-means quantizer.
It engages only at lossless intent (`quality == 100`).

| File | What it is |
|------|------------|
| [`grayscale_rd_2026-06-13.md`](grayscale_rd_2026-06-13.md) | Rate/distortion analysis: gray path vs every quantizer backend |
| [`grayscale_corpus_2026-06-13.tsv`](grayscale_corpus_2026-06-13.tsv) | Per-image speed + size run behind the analysis |

- **Corpus:** `/mnt/v/imazen-26`, the 28 strictly-grayscale images (exact
  full-resolution `R == G == B`, detected with Pillow) out of 1069.
- **Backends:** `zenquant`, `quantette`, `quantizr`, `imagequant`, `color_quant`.
- **Host:** lilith. **Source commit:** `61c9fcb` (speed/size run).
- **Headline result:** the gray path is **byte-for-byte identical to the best
  lossless backend on all 28** images and **~8.5× faster (mean)** than the `zenquant`
  baseline, with byte-exact round-trips. Because LZW size is invariant to palette
  permutation, an 8-bit gray image's exact palette already *is* the lossless optimum —
  there is no lossless byte left to recover, and matching the *lossy* backends would
  mean discarding gray levels (sliding fidelity), which the fast path deliberately
  does not do.
- **Guard:** `tests/grayscale.rs::gray_path_never_larger_than_lossless_quantizer`.
- **Reproduce:** encode each raw via `encode_gif` (gray path) and
  `encode_gif_with_quantizer` per backend at q100/d0 and q80/d0.5; compare bytes and
  byte-exact round-trip.

## Encode peak memory

| File | What it is |
|------|------------|
| [`zengif_encode_mem_2026-06-23.tsv`](zengif_encode_mem_2026-06-23.tsv) | Single-frame encode peak-memory (VmHWM marginal) sweep |

- **Host:** lilith. **Source commit:** `cf71114`. **Toolchain:** rustc 1.96.0.
- **Fixture:** `codec-corpus/jxl/reference/conformance/bike.png`, widened
  Rgb8 → Rgba then quantized to GIF; quantizer `zenquant` (imagequant resource profile).
- **Grid:** size {256..4096}² × quality {20, 50, 80}, 1 rep.
- **Fit:** marginal VmHWM `= a + b·pixels` is roughly quality-independent;
  pooled fit **≈ 1.6 MB + 41.5 B/px** (R² = 1.000). This is the model the
  `heuristics` resource estimator is calibrated against.

## `Palette::find_nearest` ARM A/B — FALSIFIED (reverted)

A branchless chunked-min rewrite of the palette-mapping inner loop was tried and
**reverted**: it regressed the dominant encode paths on ARM. Kept here as a record so
the experiment is not repeated blindly.

| File | What it is |
|------|------------|
| [`zengif_find_nearest_arm_2026-05-29.tsv`](zengif_find_nearest_arm_2026-05-29.tsv) | ARM A/B throughput numbers |
| [`zengif_find_nearest_arm_2026-05-29.meta`](zengif_find_nearest_arm_2026-05-29.meta) | Method, toolchain, full result table, verdict |
| [`zengif_find_nearest_correctness_2026-05-29.tsv`](zengif_find_nearest_correctness_2026-05-29.tsv) | Byte-identical-output correctness proof for the rewrite |

- **Box:** Hetzner cax21 (ARM Neoverse-N1, aarch64, 4 cores, 7.5 GB RAM).
- **Toolchain:** cargo 1.96.0, `RUSTFLAGS=''` (runtime SIMD dispatch, no
  `target-cpu=native`). **Build:** `cargo build --release -j2 --bench codec`.
- **Result:** the rewrite **regressed** `encode/solid` and `memory_allocation` by
  ~30% (solid frames hit an exact palette match at a low index where the scalar
  loop's per-element early-exit returns almost immediately, whereas the chunked-min
  must finish an 8-entry chunk first), while only marginally helping dirty-region
  animation encode (~2–6%, within single-shot noise). Output was proven byte-identical
  to the scalar scan first. **Verdict: not a win on ARM; reverted.**
- The correctness/A-B harness lives at `examples/find_nearest_hash.rs` for any future
  attempt (e.g. a cheap per-element exact-match exit before the vector body).
