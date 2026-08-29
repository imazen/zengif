# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **CI was red on `Clippy` and `Feature permutations`** (run 33249204557) after
  `8a2b785a` triggered the first full run in a while. Neither failure came from
  that commit; it exposed two pieces of standing debt.
  - `Clippy (all features)` failed on three `chunks_exact_to_as_chunks`
    errors in `tests/quantizer_quality.rs` — new on stable Rust 1.98, and
    denied by `-D warnings`. The PNG→RGBA helper now uses
    `as_chunks::<N>().0.iter()` at all three sites (N = 4/3/2 for RGBA, RGB
    and grayscale+alpha). MSRV is 1.93, well past the 1.88 that stabilised
    `as_chunks`. Byte-identity was measured, not assumed: the pre-change and
    post-change helpers were run side by side over 32 synthetic PNGs (every
    colour type the helper handles × 8 sizes including 1×1, single-row,
    single-column and prime dimensions) plus the 5 real corpus images up to
    8.5 MP, and all 37 decoded outputs matched exactly with identical
    FNV-1a-128 digests; the 8 Indexed inputs returned `None` from both. The
    dropped-tail semantics `as_chunks` shares with `chunks_exact` were checked
    exhaustively over all 195 (length ≤ 64, N ∈ {2,3,4}) combinations,
    remainders included. Full suite, all nine feature permutations and both
    clippy configurations green afterwards.

- **The `Fuzz regression` CI job could not fail.** It ran
  `cargo test --test fuzz_regression 2>/dev/null || echo "No regression test
  found…"` inside an `if [ -d fuzz/regression ]` guard, so a genuinely failing
  suite, a missing corpus, and a missing harness all reported green.
  `tests/fuzz_regression.rs` has existed the whole time, so the fallback was
  masking real failures rather than covering a missing target. The step is now
  a bare `cargo test --test fuzz_regression`, and the harness asserts at least
  `MIN_SEEDS` (2) replayable seeds are present — `zenutils_fuzz::RegressionSuite`
  treats a missing or empty seed dir as a clean no-op, so an emptied or renamed
  `fuzz/regression/` previously passed without replaying anything.
  Mutation-verified: removing the corpus and injecting a panic into the `decode`
  target each fail the test with exit code 101.

- **Frame-diff transparency markers were silently encoded onto opaque palette
  entries, repainting unchanged animation regions** (issue #14, 2026-08-26
  ultracode sweep, three adversarially verified findings): (1) the shared
  zenquant palette carried no transparent slot for fully-opaque multi-frame
  sources, so diff markers (a==0) were remapped onto ordinary colors — the
  backend now reserves a dedicated slot (255-color cap + appended entry) for
  multi-frame or transparent streams; (2) caller-supplied pass-through
  palettes with no transparent entry nearest-RGB'd markers onto the darkest
  entry — differencing is now disabled for such palettes (whole frames,
  still exact); (3) the quantette backend declared the opaque entry nearest
  to black as the GIF transparent index, making legitimately dark pixels
  see-through — it now reserves a dedicated transparent slot and never
  aliases it onto a color entry. A backend-independent guard in
  `prepare_frame_quantized` re-encodes the full frame (or errors) whenever
  transparent pixels come back without a transparent index, so no future
  backend can silently repaint. `Encoder::finish()` with zero frames now
  returns `InvalidEncoderState` instead of panicking.
- New decode-verifying regression suite (`tests/sweep_regressions.rs`) with
  sparse-change content whose diff rectangle is full of markers; the
  shared-palette invariant is mutation-verified (fails with both defense
  layers disabled). Cleared the new clippy `-D warnings` wall.

### QUEUED BREAKING CHANGES
<!-- Breaking changes that will ship together in the next major (or minor for 0.x)
     release. Add items here as you discover them. Do NOT ship these piecemeal —
     batch them. Confirmed breaking via `cargo semver-checks` against the published
     0.7.3 baseline (2 major-requiring findings below). -->

- The `zencodec` trait impls (`EncoderConfig` / `EncodeJob` / `Encoder` /
  `AnimationFrameEncoder` / `DecoderConfig` / `DecodeJob` / `Decode` /
  `StreamingDecode` / `AnimationFrameDecoder`) now declare `type Error =
  At<zencodec::CodecError>` instead of `At<GifError>` (imazen/zengif#13, "Pattern
  B" envelope). See the `### Changed` entry below for the full rationale;
  zengif's own native API (`Decoder`, `EncodeRequest`, `Encoder`, `decode_gif` /
  `encode_gif`) is unaffected and still returns `At<GifError>`.
- `GifError::Cancelled` is now `Cancelled(enough::StopReason)` (was a unit
  variant) — `cargo semver-checks` flags this as `enum_unit_variant_changed_kind`
  (major-requiring). The payload lets `CategorizedError::category()` distinguish
  an explicit cancellation (`ErrorCategory::Cancelled`) from a timeout
  (`ErrorCategory::TimedOut`) instead of collapsing both into one category. Any
  `match`/`matches!` on `GifError::Cancelled` needs the payload:
  `GifError::Cancelled(_)`.
- Found in passing while confirming the above via `cargo semver-checks`:
  `ProbeError` (in `src/detect.rs`, already merged, predates this entry) gained
  a `TooManyFrames { count, max }` struct variant and a `Cancelled` unit
  variant since the published 0.7.3 — `cargo semver-checks` flags this as
  `enum_discriminants_undefined_non_unit_variant` (also major-requiring).
  Riding along with this 0.8.0 bump rather than shipping separately.
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

### Documentation

- README overhaul: canonical zen badge row + crosslink footer, `Quick start`
  with current `decode_gif`/`Decoder`/`EncodeRequest` API, credit-forward "what
  zengif adds over `gif`" (replacing the competitive scorecard), and a measured
  `Benchmarks` section (grayscale fast path + encode peak-memory) sourced from
  committed `benchmarks/` data — removed the prior unverified throughput table.
  Split the crates.io README into generated `README.crates.md` (no badges,
  absolute links; `readme = "README.crates.md"`) and added `benchmarks/README.md`
  indexing the committed results with provenance + reproduction.

### Changed

- **Adopt zencodec's reshaped two-level origin-first `ErrorCategory` taxonomy
  (zencodec PR #116, `caterr-reshape`, rev `2427387f`).** The flat 17-variant
  `ErrorCategory` is now `Image(ImageError)` / `Request(RequestError)` /
  `Resource(ResourceError)` / `Policy(PolicyKind)` / `Lifecycle(StopReason)` /
  `Io(CodecIoKind)` / `Internal(InternalKind)`. `GifError::category()`
  (`src/error.rs`) was rewritten variant-by-variant against the new shape, and
  `ProbeError` (`src/detect.rs`) gained its own `CategorizedError` impl (it had
  none before — see the `### Added` entry below). No `zengif` API changed —
  only the *returned* `zencodec::ErrorCategory` shape differs for any consumer
  matching on it. `zencodec` / `zencodec-testkit` bumped to the same git rev;
  added `zencodec/std` to zengif's own `std` feature forward so
  `ErrorCategory::Io`'s `CodecIoKind` can carry a real `std::io::ErrorKind`.
  Several mapping bugs found auditing the rewrite are recorded under
  `### Fixed` below.
- **The `zencodec` trait impls now return `At<zencodec::CodecError>` (the
  envelope, "Pattern B") instead of `At<GifError>` (imazen/zengif#13).**
  Corrects the earlier Pattern A: with the bare native error type, a generic
  consumer lost the `ErrorCategory` and codec name the moment `Dyn*` dispatch
  erased the error to `Box<dyn Error>` (there was no shared concrete type to
  downcast to). Every `zencodec` trait impl in `src/codec.rs`
  (`EncoderConfig` / `EncodeJob` / `Encoder` / `AnimationFrameEncoder` /
  `DecoderConfig` / `DecodeJob` / `Decode` / `StreamingDecode` /
  `AnimationFrameDecoder`, plus the `GifDecoderConfig::probe_header` /
  `probe_full` / `decode` convenience methods) now declares `type Error =
  At<CodecError>` and wraps via a one-line `impl From<GifError> for
  At<CodecError>` bridge (`CodecError::of(e.start_at())`, reading category +
  `codec_name()` from `GifError`) for direct constructions, and
  `.map_err(CodecError::of)` at the native-API boundary (preserving the
  `whereat` trace). `GifError` is unchanged and is retained as the envelope's
  typed **detail** (recover it via `CodecError::detail()` / downcast); its
  `CategorizedError` impl is the category source. **zengif's own native API
  (`Decoder`, `EncodeRequest`, `Encoder`, `decode_gif` / `encode_gif`, the
  `error::Result` alias) keeps `At<GifError>` — only the `zencodec` adapter
  boundary changed.** A new forcing test
  (`codec::tests::envelope_category_survives_dyn_erasure`) drives zengif through
  `DynDecoderConfig`, erases to `BoxedError`, and asserts
  `error_category() == Some(MalformedImage)` and codec `Some("zengif")` survive.
- **BREAKING — `GifError::Cancelled` is now `Cancelled(enough::StopReason)`**
  (was a unit variant). The payload preserves *why* the operation stopped so
  `CategorizedError::category()` can map an explicit cancellation to
  `ErrorCategory::Cancelled` and a timeout to the distinct
  `ErrorCategory::TimedOut`, instead of collapsing both into one
  undifferentiated "cancelled" category. Every `stop.check()` call site across
  `src/codec.rs`, `src/decode/mod.rs`, `src/encode/{mod,encoder}.rs`, and the
  quantizer backends (`src/quantize/*_impl.rs`) now threads the `StopReason`
  through (`map_err(|r| at!(GifError::Cancelled(r)))` or the bare
  `map_err(GifError::Cancelled)` constructor). Any downstream `match`/`matches!`
  on `GifError::Cancelled` must add the payload: `GifError::Cancelled(_)`. A new
  forcing test (`codec::tests::envelope_cancelled_category_survives_dyn_erasure`)
  drives a pre-cancelled `Stopper` through the same `Dyn` erasure path as
  `envelope_category_survives_dyn_erasure` and asserts
  `ErrorCategory::Cancelled` + codec name survive `Box<dyn Error>` erasure.
- **BREAKING — `zencodec` is now a REQUIRED dependency; the optional `zencodec`
  cargo feature has been removed.** zencodec is foundational enough that gating
  it created dead-code / dual-build friction, so it is now an unconditional
  dependency (always compiled). The `zencodec` cargo feature is gone; the
  std-only codec glue (`GifEncoderConfig` / `GifDecoderConfig`, the
  `CategorizedError` / `SourceEncodingDetails` impls, the `SinkWrite` /
  `UnsupportedOperation` error variants) is now gated on the existing `std`
  feature — which the removed `zencodec` feature already implied — so no_std /
  wasm builds are unaffected. Downstream users on `features = ["zencodec"]` must
  drop that token; the integration now ships by default (any build with `std`).
- **deps: migrate to published `zencodec 0.1.24` estimate API; drop git-rev
  patch.** Removed the temporary `[patch.crates-io] zencodec = { git, rev =
  "0f71295" }` now that `zencodec 0.1.24` is on crates.io. Migrated the
  `estimate_encode_resources` mapping in `src/codec.rs` for the refined
  `ResourceEstimate`: `new(peak, wall_ms: u64)` (was `f32`),
  `with_peak_max(max)` (the `min` arg is gone), and dropped the removed
  `with_output_bytes`. `cargo update -p zencodec` pulled published 0.1.24.
- **deps: migrate to published `zencodec 0.1.26` + `zencodec-testkit 0.1.0`;
  retire the temporary taxonomy git patches entirely.** Removed
  `[patch.crates-io] zencodec = { git, rev = "44ca79279b" }` (root
  `Cargo.toml` + `fuzz/Cargo.toml`) now that `zencodec 0.1.26` — which ships
  the `CategorizedError`/`ErrorCategory` taxonomy (PR #103/#116) and the
  `Lifecycle` → `Stopped` rename this repo already adopted against the
  unreleased rev — is on crates.io. No source changes needed: the released
  API is identical to the git-pinned rev zengif was already built against.
  The `zencodec-testkit` dev-dependency is now a plain crates.io dep
  (`"0.1.0"`, its `^0.1.26` zencodec requirement unifies with ours from the
  registry). Interim history, preserved for archeology: dropping the patch
  while the testkit was still git-pinned split the graph into two distinct
  `zencodec` crates (every conformance test failed E0277 "multiple different
  versions of crate `zencodec`" on CI); a tag-pinned patch
  (`tag = "v0.1.26"`, content-identical to the published crate) bridged the
  gap until the testkit published, then both the git pin and the patch were
  retired.

### Added

- **`ProbeError` (`src/detect.rs`) now implements `zencodec::CategorizedError`**
  so probe failures (`probe` / `probe_with_limits`) are routable by category
  through a dyn-erased boundary, matching `GifError`'s existing impl. `TooShort`
  / `Truncated` → `Image(UnexpectedEof)`, `NotGif` → `Image(Malformed)`,
  `TooManyFrames` → `Resource(Limits(Frames))`, `Cancelled` →
  `Lifecycle(StopReason::Cancelled)` (this variant carries no `StopReason`
  payload, so it always reads as a plain cancellation).
- **Wire the `zencodec-testkit` `check_decode_truncation_series` conformance
  check (zencodec PR #112)** into the test suite
  (`tests/decode_truncation_series.rs`): feeds a valid GIF, truncates it at a
  deterministic series of prefixes, decodes each through the dyn-erased path, and
  asserts every `ErrorCategory` is in the incomplete-input set (never
  panic/OOM/Internal). `zencodec` + `zencodec-testkit` are pinned to the same git
  rev `c3220d51` until 0.1.26 + testkit publish.

- **Adopt the `zencodec` `CategorizedError` taxonomy (PR #103) (d3b3666).**
  `GifError` now `impl zencodec::CategorizedError` (compiled with the `std` feature — the default) with
  `codec_name() = Some("zengif")` and an exhaustive `category()` mapping every variant
  to one coarse `ErrorCategory` — so consumers route on the category (HTTP
  status, retry policy, logging) without naming the enum. Limits map to the
  closest `LimitKind` (`TooManyFrames`→`Frames`, `MemoryLimitExceeded`/
  `DecompressionRatioExceeded`→`Memory`, `FileTooLarge`→`InputSize`,
  `OutputTooLarge`→`OutputSize`, `AnimationTooLong`→`Duration`,
  `TotalPixelsTooLarge`→`TotalPixels`, `DimensionsTooLarge`→`Width`); the
  `UnsupportedOperation` arm delegates to the zencodec cause type. Added a new
  `GifError::SinkWrite { message }` variant (→ `ErrorCategory::Io(_)`) split out of
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

- **`gif::DecodingError::EndCodeNotFound` (an incomplete LZW stream missing its
  terminator) now categorizes as truncation** (`Image(UnexpectedEof)`) instead
  of the opaque `GifCrate` → `Image(Malformed)` path — it is a truncated read,
  not corrupt bitstream content.
- **`gif::DecodingError::OutOfMemory` and `MemoryLimit` no longer collapse into
  the same `GifError` variant.** `OutOfMemory` (a real allocator failure) now
  maps to `AllocationFailed` → `Resource(OutOfMemory)`; `MemoryLimit` (the
  `gif` crate's own configured cap) now maps to `MemoryLimitExceeded` →
  `Resource(Limits(Memory))`. Previously both collapsed to the same
  `AllocationFailed { requested: 0 }`, discarding the distinction the `gif`
  crate itself makes between the two causes.
- **`DimensionsTooLarge` now attributes the axis that actually violated its
  configured max** (Width vs. Height) instead of always reporting Width,
  regardless of which dimension exceeded its cap.
- **`FrameDimensionMismatch` (a caller-supplied wrong-geometry pixel buffer)
  now categorizes as `Request(Invalid(Buffer))`** instead of `Image(Malformed)`
  — it is an invocation fault (the caller passed a buffer of the wrong shape),
  not corrupt image bytes.
- **`DecompressionRatioExceeded` (zip-bomb guard) now routes to the dedicated
  `Resource(Limits(DecompressionRatio))` kind** instead of the closest-fit
  `Memory` kind, so an anti-DoS decompression-bomb signal is distinguishable
  from an absolute memory-budget cap.
- **`GifError::Io`'s non-EOF `std::io::ErrorKind`s now carry their real kind**
  via `CodecIoKind::from` instead of collapsing to opaque.
- **Truncated input now categorizes as `ErrorCategory::UnexpectedEof` instead of
  `Io`.** A short/truncated stream surfaces as a `std::io::ErrorKind::UnexpectedEof`
  → `GifError::Io`, which `category()` previously mapped to the opaque `Io`
  category — misattributing incomplete client input as an infrastructure/codec
  fault (5xx) rather than a malformed-request (4xx) condition. Surfaced by the new
  `check_decode_truncation_series` conformance check.

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
