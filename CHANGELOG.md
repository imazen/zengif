# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **deps: migrate to published `zencodec 0.1.24` estimate API; drop git-rev
  patch.** Removed the temporary `[patch.crates-io] zencodec = { git, rev =
  "0f71295" }` now that `zencodec 0.1.24` is on crates.io. Migrated the
  `estimate_encode_resources` mapping in `src/codec.rs` for the refined
  `ResourceEstimate`: `new(peak, wall_ms: u64)` (was `f32`),
  `with_peak_max(max)` (the `min` arg is gone), and dropped the removed
  `with_output_bytes`. `cargo update -p zencodec` pulled published 0.1.24.

### Added

- **Adopt the `zencodec` `CategorizedError` taxonomy (PR #103) (7b07c42).**
  `GifError` now `impl zencodec::CategorizedError` (gated on the `zencodec` feature) with
  `CODEC_NAME = "zengif"` and an exhaustive `category()` mapping every variant
  to one coarse `ErrorCategory` — so consumers route on the category (HTTP
  status, retry policy, logging) without naming the enum. Limits map to the
  closest `LimitKind` (`TooManyFrames`→`Frames`, `MemoryLimitExceeded`/
  `DecompressionRatioExceeded`→`Memory`, `FileTooLarge`→`InputSize`,
  `OutputTooLarge`→`OutputSize`, `AnimationTooLong`→`Duration`,
  `TotalPixelsTooLarge`→`TotalPixels`, `DimensionsTooLarge`→`Width`); the
  `UnsupportedOperation` arm delegates to the zencodec cause type. Added a new
  `GifError::SinkWrite { message }` variant (→ `ErrorCategory::Io`) split out of
  the opaque `GifCrate` catch-all for the two decode-row-sink failure sites
  (`push_decoder`, `wrap_sink_error` in `src/codec.rs`) — a sink write failure
  is an output-side error, not a malformed image. Additive (`#[non_exhaustive]`
  enum + opt-in trait); behind a **temporary `[patch.crates-io]` pin** to the
  unreleased `cancellation-classification-99` branch — remove the patch and bump
  the `zencodec` dependency once `zencodec 0.1.26` ships.
- **`GifDecoderConfig::estimate_decode_resources(&ImageCharacteristics,
  &ComputeEnvironment)`** overrides the `zencodec::DecoderConfig` default,
  delegating to the calibrated `heuristics::estimate_decode` (per-frame_count:
  RGBA canvas + 1-byte indexed buffer + previous-frame disposal backup + the
  fixed overhead that includes the ~12 KB LZW dictionary, plus all buffered
  output frames for `decode_all`). Serial → `ThreadingInformation::SERIAL`,
  cores folded in via `at_cores`.
- **Honor `zencodec::AllocPreference` (3-mode, per-site) at untrusted decode
  allocations.** New internal `src/alloc_util.rs` carries a local 3-mode
  `AllocPref` (the decode path is `zencodec`-free, so the policy is mapped from
  `ResourceLimits::prefer_fallible_allocations` → `AllocPref` only at the
  `zencodec` decode boundary, in `limits_from_resource`/`merge_resource_limits`).
  The canvas/output buffer, the grow-on-resize indexed buffer, the screen
  canvas, the per-frame composed-frame copy, and the previous-frame disposal
  backup default to the fallible `try_reserve` path (graceful
  `GifError::AllocationFailed` on a malicious Logical Screen / frame header);
  an explicit `Infallible` forces the fast `vec!` path for trusted/benchmark
  inputs; `CodecDefault` (and any future non_exhaustive variant) keeps each
  site's own default. All paths enforce the memory limit via `Stats::try_alloc`.
  The direct `decode_gif` API is unchanged (`CodecDefault`).

### Fixed

- **GIF encode now consults `zencodec::resolve_color_emit` and retains
  `with_metadata`** instead of silently discarding both. GIF embeds no
  ICC/CICP/EXIF/XMP, so the resolved `ColorEmitPlan` carries nothing to the
  bitstream (output is unchanged, always sRGB) — but running the resolver under
  the policy's `ColorEmitPolicy` puts GIF on the same color-emission contract as
  the other codecs and confirms color-managed input is dropped gracefully (no
  error). `with_metadata` retains the metadata (was a no-op stub); GIF can
  represent none of its carriers, so nothing is emitted (the loop count, the one
  GIF-representable signal, travels via `with_loop_count`).

- `GifEncoderConfig::estimate_encode_resources(&ImageCharacteristics,
  &ComputeEnvironment)` overrides the `zencodec::EncoderConfig` default,
  delegating to the calibrated `heuristics::estimate_encode` (per-frame_count,
  per-quantizer) and folding in cores via `ResourceEstimate::at_cores`. The GIF
  encode core is serial, so the estimate reports
  `ThreadingInformation::SERIAL`.
- `InternalParams` cross-codec bundle (`__expert`). `zengif::InternalParams`
  (`quantizer_preference` + `quality` + `dithering` + `use_transparency`, all
  `Option<_>`; `quality`/`dithering` are quantizer-backend-feature-gated) +
  `EncoderConfig::with_internal_params`, gated behind the new pure-visibility
  `__expert` feature — mirrors `zenjpeg`'s bundle so one picker model drives
  every zen codec with the same Option-bundle shape. Fields mirror the
  `sweep::SweepVariant` axes (backend/dithering/quality) plus `use_transparency`;
  the backend axis uses the feature-portable `QuantizerBackend` preference series.
  No new tunables (fields route through existing public setters).
- Native grayscale fast path (#4): when every opaque pixel is gray
  (`r == g == b`) — `GRAY8_SRGB` / `GRAYF32_LINEAR` codec input, document
  scans, plots, diagrams — the encoder builds the exact 8-bit gray
  palette directly and skips the general RGBA quantizer's histogram +
  k-means + color-distance search. It is a **lossless** optimization, so it
  is gated on lossless intent (`quality == 100`, which `with_lossless(true)`
  now also sets); at lower quality the configured rate-aware quantizer runs,
  since it is smaller and is what the caller asked for — so engaging the
  fast path **never costs bytes**. Engaged in both per-frame and
  shared-palette modes; detection is content-driven (one early-exiting scan)
  so color frames fall straight through to the configured quantizer at no
  cost. The win is **speed and guaranteed losslessness**, validated on the
  28 strictly-grayscale images in the
  imazen-26 corpus (exact full-res R==G==B; bilevel patent line-art at
  146 MP, continuous-tone photos at 12–33 MP, document scans, charts,
  alpha; zenquant baseline): **~8.5× faster mean** (≈8× on photos/patents,
  ≈13–16× on document scans) and **byte-exact on all 28**, where the
  quantizer is lossless on only 9 (it is lossy on every continuous-tone
  image). On **compression: 0% loss at matched fidelity** — the gray path
  is byte-for-byte identical to the optimal lossless backends
  (`quantizr`, `imagequant@q100`) on all 28 images, i.e. it already sits on
  the lossless optimum (LZW size is invariant to palette order, and a gray
  image's exact palette *is* what a lossless quantizer converges to). The
  only smaller results come from backends running **lossy** (discarding
  gray levels); an SSE-optimal 1D scalar quantizer cannot cleanly match
  them either, because GIF rate ≠ SSE (they trade fidelity for a more
  compressible index field). So the gray path is not made smaller — there
  is no lossless byte to recover, and going lossy is out of scope. See
  `benchmarks/grayscale_rd_2026-06-13.md` (RD analysis) and
  `benchmarks/grayscale_corpus_2026-06-13.tsv` (speed/size run); guarded by
  `gray_path_never_larger_than_lossless_quantizer`. A reserved transparent slot
  keeps multi-frame frame-differencing correct; a single ≥64 MB frame that
  flushes mid-stream reserves no slot (so a full 256-level image stays
  lossless), and if more frames follow such a flush, differencing is
  disabled rather than corrupting unrepresentable transparent pixels.

- `EncoderConfig::quantizer_preference` (+ builder) — the soft-intent
  counterpart to the two-spelling quantizer model: `Quantizer`
  (cfg-gated variants) is the REQUIRED choice that fails to compile
  without the backend's feature; `QuantizerBackend` is the
  always-representable vocabulary, so a preference SERIES can be
  configured/serialized without knowing the consumer's feature set.
  Resolution picks the first compiled entry
  (`QuantizerBackend::first_available`); an explicit series with no
  compiled entry errors loudly — never silently substituted. Precedence:
  required `quantizer` > preference series > deprecated
  `quantizer_backend` > `auto()`.
- Sweep compute-budget surface (`sweep`), porting the variant-generation
  playbook patterns 17–18 (`zenjpeg/docs/VARIANT_GENERATION.md`): a public
  `compute_tier(&SweepVariant) -> u8` that orders cells by encode cost
  (quantizer-backend-dominated — `ColorQuant` < `Quantizr` < `Imagequant`
  < `Quantette` < `Zenquant`, plus a small term for non-zero dithering;
  quality does not enter, being a metric dial), `SweepAxes::scalar_dense()`
  (dithering laddered `0.0..=1.0` step `0.1` plus the dense backend set —
  the shape a scalar/compute head fits), `QualityGrid::TrainingDense`
  (q step 5 over `0..=70` then step 2 over `72..=100`), and
  `plan_constrained(axes, grid, compute_limit, max_deviations)` which drops
  over-budget / over-deviation cells and records the dropped ids in the new
  `SweepPlan::compute_tier_skipped` field (never silently capped); `plan()`
  now delegates to it with `(None, None)`. All additive — `plan()`'s
  signature is unchanged.

### Fixed

- Slot-less grayscale palette no longer corrupts transparent frames (#4):
  in shared-palette mode the exact gray remap is now gated on the frame
  being representable by the committed gray palette. A palette with no
  reserved transparent slot can only encode an all-opaque frame, so a
  later frame carrying transparency — from the source OR from frame
  differencing — is deferred to the per-frame quantizer (which allocates
  its own transparent slot) instead of flattening those pixels to an
  opaque gray. Unified with the hybrid RMSE fallback under one
  `needs_per_frame` path. Adds coverage for both source-transparency and
  partial-diff-after-slot-less-flush cases (9434d85).
- README: documented the `Limits::default()` security posture (it is
  bomb-protected, NOT unbounded — 16384² dims / 120 MP / 10k frames /
  100 MB file / 1 GB memory / 1000× decompression guard), that `Limits`
  is `Clone` (reuse one posture for decode + encode), and added a
  complete decode→re-encode (transcode) example covering full-canvas
  `ComposedFrame` feedback, loop-count carry via `Metadata::repeat` →
  `EncoderConfig::repeat`, and centisecond timing preservation. Also
  documents `ComposedFrame`'s fields (full-canvas `pixels`, no offset).
  Found via an insulated external-developer usability test that had only
  the README.

### QUEUED BREAKING CHANGES

- Remove the deprecated `quantizer_backend` field — superseded by
  `quantizer` (required) + `quantizer_preference` (series); ship with
  the next 0.x minor.


- `sweep` module (any quantizer feature): variant-generation playbook
  adoption — metric-class axes (quality grid × dithering × compiled
  backends), build-feature liveness made structural (uncompiled-backend
  ids REJECTED by the parser), `gif-<backend>[-d<v>]_q<q>` grammar with
  totality test, main-effects-first plan. `tests/sweep_validate.rs`
  gates per-cell decodability, step liveness, and pixel-exact roundtrip
  of palette-representable content at q100/d0 through every compiled
  backend. Adoption record: `docs/VARIANT_GENERATION.md` (step-8
  zenmetrics wiring tracked as open).

### QUEUED BREAKING CHANGES
<!-- Breaking changes that will ship together in the next major (or minor for 0.x) release.
     Add items here as you discover them. Do NOT ship these piecemeal — batch them. -->

### Added
- Versioned public-API surface snapshot at `docs/public-api/zengif.txt`, regenerated by `tests/public_api_doc.rs` on every `cargo test` (`ZEN_API_DOC=check` verifies in CI, `=off` skips); justfile `api-doc` / `api-doc-check` recipes.

### Changed
- Removed `tests/**` and `benches/**` from the `include` list in `Cargo.toml`; published tarball now ships only `src/`, `examples/`, and the standard metadata files — downstream consumers never build the test suite or benches.

### Investigated (no shipped change)
- Tried a branchless chunked-min rewrite of `Palette::find_nearest` (pack each
  candidate as `(dist << 8) | index`, take a running `min` over `[Rgba; 8]`
  chunks to auto-vectorize). Output was proven byte-identical to the scalar scan
  (`benchmarks/zengif_find_nearest_correctness_2026-05-29.tsv`, 162 tests pass),
  but an on-box ARM A/B (Neoverse-N1) showed it **regresses the dominant
  encode/solid and memory_allocation paths ~30%** while helping dirty-region
  animation encode only ~2-6% — so it was **reverted**. Root cause: solid frames
  hit an exact palette match at a low index where the scalar loop's per-element
  `dist == 0` early-exit returns almost immediately, whereas the chunked-min must
  finish an 8-entry chunk first. Full numbers + a follow-up approach (cheap
  per-element exact-match exit before the vector body) are recorded in
  `benchmarks/zengif_find_nearest_arm_2026-05-29.{tsv,meta}`. The correctness/A-B
  harness lives at `examples/find_nearest_hash.rs` for the next attempt.

### Fixed
- `tests/fuzz_regression.rs` now gated on the `std` feature so the
  `Feature permutations / no-default-features` CI job compiles (the
  `decode_gif`/`encode_gif`/`Decoder` symbols it imports are std-gated);
  rewrote the streaming-decode loop to satisfy `clippy::while_let_loop`
  under `-D warnings` (behaviour identical, 100-frame cap preserved);
  fixed stale doc-comment crate name `zen-fuzz-regress` → `zenutils-fuzz`.
- `codec` and `decode_bench` benches now declare `required-features =
  ["std"]` so `cargo check --all-targets --no-default-features` is clean.

### Changed
- `tests/fuzz_regression.rs` now uses the shared `zenutils-fuzz`
  test-helper crate (DEDUP-J2). Behaviour is unchanged — same
  `fuzz/regression/` seeds, same three targets (`decode`,
  `decode_streaming`, `roundtrip`), same panic-propagation failure
  semantics. The in-file `collect_seeds` scaffolding is now provided
  by `RegressionSuite`.

### Added

- `tests/fuzz_regression.rs` regression-harness template ported from
  zenwebp (DEDUP-J). Walks `fuzz/regression/` (incl. per-target subdirs)
  and runs every seed through `decode_gif`, streaming `Decoder`, and the
  `encode_gif` roundtrip on the stable toolchain — no nightly required.
  Drop minimized crash files into `fuzz/regression/` to gate future
  regressions of fixed bugs.

## [0.7.3] - 2026-04-17

### Changed

- Bump zencodec to 0.1.19 (release prep)

## [0.7.2] - 2026-04-10

### Fixed

- Reject zero-dimension GIF images in `check_dimensions` (5323302)
- Relax decompression ratio limit in roundtrip fuzz target (d5eb050)

### Changed

- Bumped zencodec to 0.1.13 (f678856)

### Added

- Nightly fuzz workflow (60s on push, 5min nightly) (dba2d7a)
- Committed fuzz/Cargo.lock for reproducible fuzz builds (afcad3e)
- Gitignore tooling noise; exclude from packages (b2d388a)

## [0.7.1] - 2026-04-01

### Fixed

- **Memory safety**: Replaced `Box::leak` with `Cow`-owned storage in Encoder to eliminate leaked memory
- **Stats accuracy**: Use `saturating_sub` in `Stats::track_dealloc` to prevent underflow
- **Quantette support**: Added quantette to all cfg gates; fixed transparent index out-of-range panic
- **Transparency**: Fixed transparent pixel mapping in `quantize_frame`; initialized canvas as transparent
- **Error instrumentation**: Upgraded from `.start_at()` to `at!()` macro for better error tracing

### Changed

- Bumped dependencies: zencodec 0.1.12, zenpixels 0.2.2, linear-srgb 0.6.7, archmage, enough, whereat
- Set correct minimum versions for zenflate and linear-srgb

### Added

- Expanded transparency and background disposal test coverage
- Reduced fuzz artifacts for malformed GIF inputs

## [0.7.0] - 2026-03-31

### Changed

- Three-layer encoding API: `EncoderConfig` → `EncodeRequest<'a>` → `Encoder<'a>`
- Modularized encode module into focused submodules (palette, config, request, encoder)
- Added zenquant quantization backend (perceptual masking, AGPL-3.0)
- Added quantette quantization backend (Oklab k-means, MIT/Apache-2.0)
- Upgraded to Rust 2024 edition, MSRV 1.93
- Added zenpixels and linear-srgb dependencies for pixel type integration

## [0.6.0] - 2026-02-06

### Changed - BREAKING

- **API Refactoring**: Three-layer encoding API for better ergonomics and lifetime management
  - `EncoderConfig` no longer takes dimensions (now configuration-only)
  - New `EncodeRequest` builder pattern: `EncodeRequest::new(&config, width, height).limits(&limits).stop(&stop).build()`
  - `Encoder` now uses lifetime-bound references instead of generics (`Encoder<'a>` instead of `Encoder<W, S>`)
  - `Encoder::finish()` now returns `Vec<u8>` instead of writing to a mutable buffer
  - Migration: `Encoder::new(&mut buf, w, h, cfg, lim, stop)` → `EncodeRequest::new(&cfg, w, h).limits(&lim).stop(&stop).build()`

### Internal

- Modularized encode module: Split 2674-line file into focused modules (palette, config, request, encoder)
- Improved code organization and maintainability


## [0.5.0] - 2026-02-04

### Fixed

- **Critical**: Fixed transparent pixel handling in shared palette mode. Frame differencing marks unchanged pixels as transparent, but the shared palette (built from original frames) had no transparent entry. This caused the quantizer to map transparent pixels to nearest color (usually dark gray), creating severe visual artifacts on playback. Quality improved from SSIM ~40 to ~100 on affected GIFs.
- **Correctness**: Fixed `QuantizeResult` reuse in quantizr backend. Previously, each frame was re-quantized independently even in shared palette mode, which was both slower and could produce incorrect palette indices.

### Added

- **Hybrid palette mode**: New `palette_error_threshold` option enables automatic per-frame palette fallback when a frame's RMSE exceeds the threshold. This catches outlier frames that don't fit the shared palette while keeping most frames on the global palette (no flicker, better compression).
- **Lossy frame differencing**: New `lossy_tolerance` option treats pixels within tolerance of the previous frame as unchanged, reducing dirty region size and improving compression at slight quality cost.
- **RMSE analysis examples**: `rmse_analysis.rs` and `size_analysis.rs` for testing palette quality.

### Changed

- **Default `shared_palette` is now `true`**: Most animations benefit from shared palettes (reduced flicker, smaller files). Use `.shared_palette(false)` for the old behavior.
- **Default `palette_error_threshold` lowered to 5.0**: Testing revealed threshold 15 was too permissive; frames with noticeable color distortion (RMSE 5-10) weren't triggering fallback. The new default catches problematic frames while not over-triggering.
- **Dimension-aware buffer frames**: Default `max_buffer_frames` now scales based on frame dimensions (smaller frames = more frames buffered for better palette sampling).

### Performance

- Shared palette mode is now ~4x faster for multi-frame remapping due to `QuantizeResult` reuse.
- `Screen::reset()` uses `slice::fill()` instead of per-pixel loop.
- Memory pooling for encoder scratch buffers.

## [0.4.0] - 2025-02-04

### Changed

- **BREAKING**: `Decoder::new()` now takes 3 arguments instead of 4. The `&Stats` parameter is removed - the decoder owns its stats internally.
- **BREAKING**: `decode_gif()` now returns a 3-tuple `(Metadata, Vec<ComposedFrame>, Stats)` instead of 2-tuple.
- Access stats during decode via `decoder.stats()` method.
- Added `#![forbid(unsafe_code)]` - the entire crate is now 100% safe Rust.

### Removed

- **BREAKING**: Removed unused `simd` feature (was a placeholder with no actual SIMD code).
- All `unsafe` code removed. The use-after-free bug with external `&Stats` is fixed by having the decoder own its stats.

### Migration from 0.3

```rust
// Old (0.3):
let stats = Stats::new();
let mut decoder = Decoder::new(reader, limits, &stats, Unstoppable)?;
// ... decode ...
println!("Peak: {}", stats.peak());

// New (0.4):
let mut decoder = Decoder::new(reader, limits, Unstoppable)?;
// ... decode ...
println!("Peak: {}", decoder.stats().peak());
```

## [0.3.0] - 2025-01-23

### Added

- **no_std support**: Core types work without std library (use `default-features = false`).
- **`heuristics` module**: Estimate memory/time requirements before decoding.
- **WASM support**: Verified working on `wasm32-unknown-unknown` (144KB release build).
- Uses `core::error::Error` (stabilized in Rust 1.81) instead of `std::error::Error`.

### Changed

- Minimum supported Rust version is now 1.81 (for `core::error::Error`).
- Dev dependencies gated for WASM compatibility.

## [0.2.1] - 2025-01-20

### Fixed

- Removed misleading memory tracking claim from README.
- Fixed comparison table accuracy.

## [0.2.0] - 2025-01-19

### Added

- **Multiple quantization backends**: Choose from different color quantizers:
  - `imagequant`: Highest quality, smallest files (GPL-3.0-or-later, commercial license available).
  - `quantizr`: Fast, good quality (MIT).
  - `color_quant`: NEUQUANT algorithm, fastest (MIT).
  - `exoquant-deprecated`: K-Means quantizer (MIT, use quantizr instead).

### Changed

- Renamed `quantize` feature to `imagequant` for clarity.
- Each quantization backend is now a separate feature flag.

## [0.1.0] - 2025-01-18

### Added

- **Streaming decode/encode API**: Process GIFs without loading entire files into memory.
  - `Decoder` for frame-by-frame streaming decode with disposal handling.
  - `Encoder` for progressive frame encoding with optional quantization.
  - `decode_gif()` and `encode_gif()` convenience functions.

- **Complete animation support**:
  - All disposal methods (Keep, Background, Previous).
  - Per-frame transparency handling.
  - Timing preservation on round-trip.
  - Loop count (NETSCAPE extension) support.

- **Memory safety and limits**:
  - Configurable `Limits` for dimensions, frame count, file size, memory.
  - `Stats` for tracking buffer allocations.
  - Fallible allocations throughout (`try_reserve()`).
  - Zip bomb protection via decompression ratio limits.

- **Production-ready error handling**:
  - Integration with [`whereat`](https://crates.io/crates/whereat) for error location tracking.
  - Integration with [`enough`](https://crates.io/crates/enough) for cooperative cancellation.
  - Detailed error messages with context.

- **Zero-trust input validation**:
  - Header validation before allocation.
  - Frame bounds checking.
  - Malformed LZW handling.

- **Optional features**:
  - `rgb-interop`: Interop with `rgb` crate types.
  - `imgref-interop`: Interop with `imgref` crate types.

- **Frame optimization for encoding**:
  - Frame differencing to find minimal changed regions.
  - Transparency optimization for unchanged pixels.

### Performance

- Decode: ~150 MB/s composited throughput.
- Encode (pre-indexed): ~200 MB/s.
- Encode (quantized): ~40 MB/s.

[0.7.2]: https://github.com/imazen/zengif/releases/tag/v0.7.2
[0.7.1]: https://github.com/imazen/zengif/releases/tag/v0.7.1
[0.7.0]: https://github.com/imazen/zengif/releases/tag/v0.7.0
[0.6.0]: https://github.com/imazen/zengif/releases/tag/v0.6.0
[0.5.0]: https://github.com/imazen/zengif/releases/tag/v0.5.0
[0.4.0]: https://github.com/imazen/zengif/releases/tag/v0.4.0
[0.3.0]: https://github.com/imazen/zengif/releases/tag/v0.3.0
[0.2.1]: https://github.com/imazen/zengif/releases/tag/v0.2.1
[0.2.0]: https://github.com/imazen/zengif/releases/tag/v0.2.0
[0.1.0]: https://github.com/imazen/zengif/releases/tag/v0.1.0
