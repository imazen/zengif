# zengif Public API Ablation Report

**Date:** 2026-06-11  
**Snapshot commit:** 7d3a339b13cafefa8cbc3dd8f8db1bfcab8e76e5  
**Snapshot:** 846 items (default = std+zenquant), 1126 items (all-except-_*); quantizer-backend features add ~280  
**Scan template:** `grep -r "<sym>" /home/lilith/work --include="*.rs" -l | grep -v zen/zengif/ | grep -v zen-arm-src/ | grep -v pre-filter/ | grep -v .jplag/ | grep -v target/`  
**Consumers checked:** zenpipe/zencodecs, imageflow, coefficient, zenquant, hdr-corpus-convert, zenpng  
**Mode:** REPORT ONLY — no source or manifest changes.

---

## Summary

| Category | Count |
|----------|-------|
| Total named items (unique, default section) | 455 |
| Flagged for action | 5 items / groups |
| % of total | ~1.1% |
| Clear mistakes confirmed | 4 (A/B) + 1 informational |
| Items kept (consumers found or structurally necessary) | 450+ |

Threshold: 10% to aggregate. At 1.1% flagged, individual item-level reporting is appropriate.

---

## Grep Evidence

All commands run 2026-06-11. Excludes: zen-arm-src (stale box snapshot), pre-filter (stale snapshot), .jplag (comparison copy), zen-arm-results (stale measurement outputs), target/ (build output).

```
TrackedAlloc                  → 0 files
tracked_vec_filled            → 0 files
tracked_vec_with_capacity     → 0 files
encode_gif_with_quantizer     → 0 files
encode_gif_shared_palette     → 0 files
zengif::Screen                → 0 files (354 generic hits are CSS/wgpu/UI unrelated)
zengif::ScreenBuilder         → 0 files
zengif::heuristics            → 0 files (13 hits are in zenjpeg/zenwebp's own heuristics.rs files)
zengif::GifEncoderConfig::inner → 0 files
GifStreamingDecoder           → 0 files (+ zen-arm-src only)
QuantizeConfig                → 0 files (external)
QuantizedFrame                → 0 files (external)
QuantizerBackend              → 0 files (external)
QuantizerTrait                → 0 files (external)
StatsSnapshot                 → 0 files

Confirmed consumers:
decode_gif              → coefficient, zenquant, zencodecs (via Decoder facade)
Palette::from_rgb_bytes → zenquant/tests/integration.rs (deliberate use)
EncoderConfig           → coefficient, zencodecs, zenquant examples
FrameInput              → coefficient, zenquant, zencodecs
Rgba                    → coefficient, zenquant, zencodecs
GifDecoderConfig / GifEncoderConfig → zencodecs (re-exported)
heuristics module       → 13 hits but ALL are other codecs' own heuristics.rs, NOT zengif::heuristics
```

---

## Module Table

### `zengif` (root) — default features (std+zenquant)

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `TrackedAlloc<'a>` | 0 | **A** (`#[doc(hidden)]`) | Internal RAII guard for stats bookkeeping. Docs describe it as "wrap Vec or other heap allocations to ensure deallocation is tracked." This is plumbing for internal use within zengif's canvas/screen compositing. No external consumer. `tracked_vec_filled` and `tracked_vec_with_capacity` are convenience wrappers around it. |
| `tracked_vec_filled<T>()` | 0 | **A** (`#[doc(hidden)]`) | Internal helper. No external consumer. `Stats + Limits` as parameters make it internal infra. |
| `tracked_vec_with_capacity<T>()` | 0 | **A** (`#[doc(hidden)]`) | Same as above. |
| `encode_gif_shared_palette()` | 0 | **B** (pub(crate) candidate) | Feature-gated convenience function (requires quantizer). No external consumer. The same result is achievable via `EncodeRequest` + `EncoderConfig::shared_palette(true)`. The function may confuse users who see two non-obvious entry points (`encode_gif` vs `encode_gif_shared_palette`). Could be removed in next breaking release. |
| `encode_gif_with_quantizer<Q>()` | 0 | **B** (pub(crate) candidate) | Same reasoning. The `EncodeRequest` pattern is the intended ergonomic entry point; this generic function duplicates it without obvious advantage for callers. |

### `zengif::heuristics` module

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `QuantizerType` enum | 0 | KEEP | Part of the `estimate_encode_*` API surface, which is the primary value of the heuristics module. No issue. |
| Whole module | 0 real (13 false positives in other codecs' own heuristics files) | KEEP | The heuristics module is a deliberate public surface providing memory and timing estimates before encoding. Zero external users currently, but this is a greenfield codec — the API is forward-looking. Not a mistake. |

### `zengif::codec` (private module, `GifStreamingDecoder` leaks)

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `GifStreamingDecoder` | 0 | **Informational** | Same structural leak as `PngStreamingDecoder` in zenpng. The `mod codec` is private but `GifStreamingDecoder` is `pub struct` within it, leaking via `GifDecodeJob::StreamDec` (associated type) and `GifDecodeJob::streaming_decoder()` return type. No external consumer. Candidate for sealing with `#[doc(hidden)]` on the struct or an opaque wrapper in a future API pass. Not proposing a breaking change here — marking informational. |

### `zengif` root — always-on `quantize` re-exports

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `QuantizeConfig` | 0 (external) | KEEP | Parameter type for `QuantizerTrait::quantize_frame()`. Necessary for any caller implementing a custom quantizer. |
| `QuantizedFrame` | 0 (external) | KEEP | Return type for `QuantizerTrait` methods. Same reasoning. |
| `QuantizerBackend` | 0 (external) | KEEP | Enum used in `Quantizer` builder to select backend. Used via `EncoderConfig::quantizer()`. |
| `QuantizerTrait` | 0 (external) | KEEP | Trait needed for `encode_gif_with_quantizer<Q: QuantizerTrait>`. Removing it would make custom quantizers impossible. |

### `zengif` root — zero-consumer `Screen`/`ScreenBuilder`

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `Screen`, `ScreenBuilder` | 0 real (354 unrelated hits) | KEEP | The GIF canvas compositing API. Intentionally public for callers that need to implement custom compositing (e.g., frame-by-frame animation renderers that want direct compositor access). The CLAUDE.md describes this as a core building block. Zero current external use does not make it wrong — it's a deliberate advanced API surface. |

### `zengif` root — `GifEncoderConfig::inner/inner_mut`

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `GifEncoderConfig::inner()` | 0 | **A** (`#[doc(hidden)]`) | Returns `&EncoderConfig` — exposes the internal wrapped config. Callers should use `GifEncoderConfig`'s own builder API. No external consumer. |
| `GifEncoderConfig::inner_mut()` | 0 | **A** (`#[doc(hidden)]`) | Same. Mutable access to internals. No external consumer. |

---

## Items Confirmed Safe (Selected)

| Item | Reason to keep |
|------|----------------|
| `decode_gif()` | Used by coefficient, zenquant, zencodecs (via Decoder). Widely consumed. |
| `Palette`, `Palette::from_rgb_bytes()` | Used by zenquant integration tests. |
| `EncoderConfig`, `EncodeRequest`, `Encoder` | Used by coefficient, zencodecs, zenquant. Core API. |
| `RawFrame` | Used in `Metadata::total_duration_centiseconds()` signature. Necessary for timing calculations without full decode. |
| `Screen`, `ScreenBuilder` | Deliberate advanced compositing API. Not a mistake despite zero external callers today. |
| `heuristics::estimate_*` | Part of the forward-looking planning API. No current callers but intentional. |
| `Stats`, `StatsSnapshot` | Returned by `decode_gif()` and `Decoder::stats()`. Necessary for memory tracking callers. |
| `GifDecoderConfig`, `GifEncoderConfig` | Used by zencodecs as re-exports. |
| `DecodeError = GifError`, `EncodeError = GifError` type aliases | Legitimate ergonomic aliases for the unified error type. |
| `ZenquantQuantizer`, `ImagequantQuantizer`, etc. | Each quantizer backend is the deliberate API for its feature. |
| `encode_gif_shared_palette` | Flagged B (no consumers), but not an obvious mistake — callers may discover it; decision on removal deferred to breaking release. |

---

## Top-5 Digest

1. **`TrackedAlloc<'a>`** — A: #[doc(hidden)]. Zero external consumers. Internal RAII guard for stats. `tracked_vec_filled` and `tracked_vec_with_capacity` are its helpers — same action.
2. **`tracked_vec_filled<T>()`** / **`tracked_vec_with_capacity<T>()`** — A: #[doc(hidden)]. Internal allocation helpers surfaced unnecessarily.
3. **`encode_gif_shared_palette()`** — B: pub(crate) candidate (queued breaking). Redundant entry point; `EncodeRequest` covers the same use case.
4. **`encode_gif_with_quantizer<Q>()`** — B: pub(crate) candidate (queued breaking). Same reasoning; generic form adds API complexity without a real consumer.
5. **`GifEncoderConfig::inner()` / `inner_mut()`** — A: #[doc(hidden)]. Exposing internal `EncoderConfig` directly. Zero external consumers.

(Informational: `GifStreamingDecoder` structural leakage via `GifDecodeJob::StreamDec` associated type — same issue as zenpng, not proposing a breaking change.)

---

## Action Summary

| Action class | Items | Breaking? |
|-------------|-------|-----------|
| A — `#[doc(hidden)]` / `#[deprecated]` | 5 items (TrackedAlloc, tracked_vec_filled, tracked_vec_with_capacity, inner, inner_mut) | No |
| B — pub(crate) / remove | 2 functions (encode_gif_shared_palette, encode_gif_with_quantizer) | **Yes** (semver-breaking) — queue in QUEUED BREAKING CHANGES |
| Informational | 1 (GifStreamingDecoder) | N/A |

**B items must wait for next breaking release.** Add to `## QUEUED BREAKING CHANGES` in CHANGELOG.md `[Unreleased]` section.

**Note on quantizer-backend API (~280 extra items in all-features):** The ImagequantQuantizer, QuantetteQuantizer, QuantizrQuantizer, and ColorQuantQuantizer concrete structs added by their feature flags are all intentional — each is the deliberate API surface for its backend. No issues found in that section.
