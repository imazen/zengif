# Variant Generation: zengif's adoption of the zenjpeg patterns

Written 2026-06-11. Codec-neutral patterns:
`zenjpeg/docs/VARIANT_GENERATION.md`. Code: `src/sweep.rs` (gated on
any quantizer feature), `tests/sweep_validate.rs` (normal suite, ~2 s).

GIF stills are **quantizer-dominated**: quality, dithering, and the
backend all change pixels (metric-class), and the trial class is empty
at this surface (LZW exposes no knobs). The signature pattern here is
**pattern 10's build-feature liveness made structural**: the axes
contain only compiled backends, and `variant_from_cell_id` REJECTS ids
naming backends this build doesn't carry — executing such a cell would
silently encode with a different quantizer, which is exactly the
silent-wrong-encode class the fp verification exists to kill.

- **Excluded with reasons** (module docs): `lossy_tolerance`
  (frame-differencing — structurally inert on stills; an
  animation-corpus axis), `use_transparency` (alpha-class),
  `shared_palette`/`repeat`/buffers (animation-only),
  `global_palette`/`quantizer` override/`palette_error_threshold`
  (custom-payload escape hatches, not self-describing).
- **Id grammar** (pattern 7): `gif-<backend>[-d<dither>]_q<q>`,
  Display-lossless numbers, totality + uncompiled-backend rejection
  tests.
- **Fingerprint**: backend + quality + dithering (all metric-class —
  nothing to exclude); still-image pins ride `build()`.
- **Validation** (patterns 6/14/15): every dev≤1 cell encodes AND
  decodes with one frame and matching dims on bands/noise/odd-509×381/
  tiny; every step live vs the default stratum; and the
  palette-representable leg (≤256 colors) must roundtrip **pixel-exact
  at q100/d0 through every compiled backend** — GIF is lossy, but on
  representable content the quantizer has nothing to lose, so drift
  there is a broken backend, not "lossy".
- **Step 8 (executor wiring)**: open — zenmetrics has no
  `CodecKind::Zengif` yet (decode support + CodecKind + plan arms);
  tracked as the adoption's one remaining step.

## Known limits

- Animation variant space (disposal, frame differencing,
  shared-palette strategies, `lossy_tolerance`) is a separate axis set
  needing an animation corpus — unmodeled here.
- Alpha/transparency axes need an alpha corpus + alpha-aware metric.
- Backend availability differs per build; fleet sweeps must pin the
  feature set in the run manifest (the parser enforces the per-build
  half of this).
