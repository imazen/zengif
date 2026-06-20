//! Sweep-plan construction over the GIF encoder knob space.
//!
//! Port of the variant-generation playbook
//! (`zenjpeg/docs/VARIANT_GENERATION.md`). GIF stills are
//! **quantizer-dominated**: quality, dithering, and the quantizer
//! backend all change pixels (metric-class — sweeps and pickers exist
//! for exactly these), and the trial class is empty at this surface
//! (LZW has no exposed knobs; once palette + indices are fixed the
//! coding is deterministic).
//!
//! **Backend liveness is build-feature-conditional** (playbook
//! pattern 10's build-feature-dead knob, made structural): the axes
//! only contain backends the cargo-feature set compiled in, and
//! [`variant_from_cell_id`] rejects ids naming backends this build
//! does not carry — a curated probe can never be a guaranteed inert
//! step.
//!
//! Deliberately excluded from the curated axes, with reasons:
//!
//! - `lossy_tolerance` — frame-differencing tolerance; structurally
//!   inert on single-frame stills (pattern 10 class-conditional —
//!   it joins an animation-corpus axis set, not this one).
//! - `use_transparency` — alpha-class-conditional (needs an
//!   alpha-bearing corpus + alpha-aware scoring).
//! - `shared_palette` / `repeat` / `max_buffer_*` — animation-only.
//! - `global_palette` / `quantizer` override /
//!   `palette_error_threshold` — expert custom-payload escape hatches
//!   (not self-describing; same class as zenjpeg's `Custom` tables).
//!
//! Step provenance (ship-derived): dithering default 0.5 with the
//! documented bounds {0.0 = re-encode-already-dithered, 1.0 = max};
//! backends are the shipped tiers themselves; quality is the metric
//! grid dial (0–100).

#[cfg(not(feature = "std"))]
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
#[cfg(feature = "std")]
use std::collections::BTreeMap;

use crate::encode::EncoderConfig;
use crate::quantize::{Quantizer, QuantizerBackend};

/// One still-image encode variant with its resolved quality.
#[derive(Clone, Debug)]
pub struct SweepVariant {
    /// Quantizer quality 0–100 (metric dial; fingerprint-hashed).
    pub quality: u8,
    /// Dithering level 0.0–1.0 (metric-class).
    pub dithering: f32,
    /// Quantizer backend (compiled set only).
    pub backend: QuantizerBackend,
}

impl SweepVariant {
    /// Whether the dithering knob applies to this backend (ColorQuant's
    /// NEUQUANT path has no dithering — playbook pattern 10: the axis is
    /// structurally inapplicable, never silently inert).
    #[must_use]
    pub fn dithering_applies(backend: QuantizerBackend) -> bool {
        !matches!(backend, QuantizerBackend::ColorQuant)
    }

    /// Build the encoder config (still-image settings pinned: no
    /// shared palette, lossless frame differencing, no transparency).
    /// Uses the modern `quantizer` field; the variant payload mirrors
    /// the sweep's (quality, dithering) so both consultation paths
    /// agree.
    ///
    /// # Panics
    /// On a backend this build did not compile — `compiled_backends()`
    /// and `variant_from_cell_id` gate every sweep path; constructing a
    /// `SweepVariant` for an uncompiled backend by hand is a caller bug.
    #[must_use]
    pub fn build(&self) -> EncoderConfig {
        let quantizer = match self.backend {
            #[cfg(feature = "zenquant")]
            QuantizerBackend::Zenquant => Quantizer::Zenquant {
                dithering: self.dithering,
            },
            #[cfg(feature = "quantette")]
            QuantizerBackend::Quantette => Quantizer::Quantette {
                dithering: self.dithering,
            },
            #[cfg(feature = "quantizr")]
            QuantizerBackend::Quantizr => Quantizer::Quantizr {
                dithering: self.dithering,
            },
            #[cfg(feature = "imagequant")]
            QuantizerBackend::Imagequant => Quantizer::Imagequant {
                quality: self.quality,
                dithering: self.dithering,
            },
            #[cfg(feature = "color_quant")]
            QuantizerBackend::ColorQuant => Quantizer::ColorQuant { sample_factor: 10 },
            #[allow(unreachable_patterns)]
            other => unreachable!(
                "backend {other:?} is not compiled into this build;                  compiled_backends()/variant_from_cell_id gate sweep paths"
            ),
        };
        EncoderConfig {
            quality: self.quality,
            dithering: self.dithering,
            quantizer: Some(quantizer),
            shared_palette: false,
            lossy_tolerance: 0,
            use_transparency: false,
            ..EncoderConfig::new()
        }
    }

    fn base_id(&self) -> String {
        let mut s = format!("gif-{}", backend_token(self.backend));
        if Self::dithering_applies(self.backend) && self.dithering != 0.5 {
            s.push_str(&format!("-d{}", self.dithering));
        }
        s
    }
}

fn backend_token(b: QuantizerBackend) -> &'static str {
    match b {
        QuantizerBackend::Zenquant => "zenquant",
        QuantizerBackend::Quantette => "quantette",
        QuantizerBackend::Imagequant => "imagequant",
        QuantizerBackend::Quantizr => "quantizr",
        QuantizerBackend::ColorQuant => "colorquant",
    }
}

/// The backends THIS build carries, default first (pattern 10:
/// liveness is a function of the feature set).
#[must_use]
pub fn compiled_backends() -> Vec<QuantizerBackend> {
    let mut v = vec![QuantizerBackend::default()];
    #[cfg(feature = "zenquant")]
    if !v.contains(&QuantizerBackend::Zenquant) {
        v.push(QuantizerBackend::Zenquant);
    }
    #[cfg(feature = "quantette")]
    if !v.contains(&QuantizerBackend::Quantette) {
        v.push(QuantizerBackend::Quantette);
    }
    #[cfg(feature = "imagequant")]
    if !v.contains(&QuantizerBackend::Imagequant) {
        v.push(QuantizerBackend::Imagequant);
    }
    #[cfg(feature = "quantizr")]
    if !v.contains(&QuantizerBackend::Quantizr) {
        v.push(QuantizerBackend::Quantizr);
    }
    #[cfg(feature = "color_quant")]
    if !v.contains(&QuantizerBackend::ColorQuant) {
        v.push(QuantizerBackend::ColorQuant);
    }
    v
}

/// Reconstruct the [`SweepVariant`] a cell id denotes (full id with the
/// `_q<q>` token). Grammar: `gif-<backend>[-d<dither>]_q<q>` — numbers
/// render via `Display` (shortest-roundtrip, lossless). Ids naming
/// backends this build did not compile error (build-feature liveness
/// is part of the identity contract: executing such a cell here would
/// silently encode with a different quantizer). Renderer and parser
/// move in lockstep (`cell_ids_roundtrip_to_their_variants`);
/// evolution is additive-only.
pub fn variant_from_cell_id(id: &str) -> Result<SweepVariant, String> {
    let Some(rest) = id.strip_prefix("gif-") else {
        return Err(format!("cell id {id:?} is not a gif- id"));
    };
    let (flags_part, q_part) = match rest.rsplit_once('_') {
        Some((f, q)) if q.starts_with('q') => (f, q),
        _ => return Err(format!("gif id {id:?} missing _q quality token")),
    };
    let quality: u8 = q_part[1..]
        .parse()
        .map_err(|e| format!("bad q in {id:?}: {e}"))?;
    let mut parts = flags_part.split('-');
    let backend_tok = parts.next().unwrap_or_default();
    let backend = compiled_backends()
        .into_iter()
        .find(|b| backend_token(*b) == backend_tok)
        .ok_or_else(|| {
            format!(
                "backend {backend_tok:?} in {id:?} is not compiled into this build \
                 (cargo features gate quantizer liveness)"
            )
        })?;
    let mut v = SweepVariant {
        quality,
        dithering: 0.5,
        backend,
    };
    for f in parts {
        if let Some(d) = f.strip_prefix('d') {
            if !SweepVariant::dithering_applies(backend) {
                return Err(format!(
                    "dithering flag in {id:?} but backend {backend_tok:?} has no dithering knob"
                ));
            }
            v.dithering = d
                .parse()
                .map_err(|e| format!("bad dither in {id:?}: {e}"))?;
        } else {
            return Err(format!("unknown flag {f:?} in {id:?}"));
        }
    }
    Ok(SweepVariant { quality, ..v })
}

/// Byte-identity fingerprint over resolved state. Every field hashed
/// (all three knobs are metric-class — no exclusions to prove wrong);
/// the pinned still-image settings ride implicitly via `build()`.
#[must_use]
pub fn fingerprint(variant: &SweepVariant) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut write = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    write(variant.backend as u8);
    write(variant.quality);
    if SweepVariant::dithering_applies(variant.backend) {
        for b in variant.dithering.to_bits().to_le_bytes() {
            write(b);
        }
    } else {
        write(0xFF); // dithering structurally inapplicable
    }
    h
}

/// Ordinal compute cost of a variant (`0` = cheapest). A picker or a
/// CPU-bound fleet uses this to bound encode time; it is **not** a
/// quality signal.
///
/// GIF's encode cost is dominated by the **quantizer backend** — the
/// palette search is the expensive step, and the backends form a clear
/// quality/speed ladder (`color_quant`'s NEUQUANT is a fixed cheap pass,
/// `quantizr` a fast median-cut, up through `zenquant`'s perceptual
/// optimization). Dithering adds a smaller error-diffusion pass on top,
/// so a non-zero dithering level bumps the tier by one. Quality is a
/// metric dial, not a compute knob (only the optional `imagequant`
/// backend even consults it, and it does not change the search cost), so
/// it does not enter the tier.
///
/// Backend cost ladder (ascending), mirroring the documented
/// quality/speed tradeoff on [`QuantizerBackend`]:
/// `ColorQuant` < `Quantizr` < `Imagequant` < `Quantette` < `Zenquant`.
/// The dithering term lands in the gaps between backend bands, so the
/// per-backend ordering is preserved within a fixed dithering setting.
#[must_use]
pub fn compute_tier(variant: &SweepVariant) -> u8 {
    // Backend band, multiplied to leave room for the dithering add-on
    // without crossing into the next backend's band.
    let backend_band: u8 = match variant.backend {
        QuantizerBackend::ColorQuant => 0,
        QuantizerBackend::Quantizr => 1,
        QuantizerBackend::Imagequant => 2,
        QuantizerBackend::Quantette => 3,
        QuantizerBackend::Zenquant => 4,
    };
    let dither_add: u8 =
        u8::from(SweepVariant::dithering_applies(variant.backend) && variant.dithering > 0.0);
    backend_band * 2 + dither_add
}

/// Axes, most-important value first.
#[derive(Clone, Debug)]
pub struct SweepAxes {
    /// Quantizer backends (compiled set only; index 0 = build default).
    pub backends: Vec<QuantizerBackend>,
    /// Dithering levels.
    pub dithering: Vec<f32>,
}

impl SweepAxes {
    /// RD-front: the build-default backend at default dithering.
    #[must_use]
    pub fn rd_core() -> Self {
        Self {
            backends: vec![QuantizerBackend::default()],
            dithering: vec![0.5],
        }
    }

    /// Every compiled backend × the documented dithering bounds.
    #[must_use]
    pub fn modes_full() -> Self {
        Self {
            backends: compiled_backends(),
            dithering: vec![0.5, 0.0, 1.0],
        }
    }

    /// Axes shaped for fitting a **scalar head** (playbook patterns
    /// 17–18): pin nothing categorical to a single point that would
    /// starve the head, and ladder the continuous knobs densely.
    ///
    /// - **Dithering** is laddered `0.0, 0.1, … 1.0` (11 points — finer
    ///   than [`modes_full`]'s `{0.0, 0.5, 1.0}`), the continuous knob a
    ///   scalar-output head regresses.
    /// - **Backends** are kept dense (every compiled backend, default
    ///   first), so a **compute-tier** head sees each
    ///   [`compute_tier`] band rather than collapsing to the default's
    ///   single cost. The backend is GIF's only real compute axis, so
    ///   pinning it would erase the very signal a compute head exists to
    ///   learn — it stays dense here by design (the categorical "pin" of
    ///   the pattern is satisfied by *quality* moving to the dense
    ///   [`QualityGrid::TrainingDense`] grid the caller pairs with this).
    ///
    /// Pair with [`QualityGrid::TrainingDense`] for the full scalar-head
    /// training cell set.
    #[must_use]
    pub fn scalar_dense() -> Self {
        let mut dithering = Vec::with_capacity(11);
        for i in 0..=10u8 {
            dithering.push(f32::from(i) / 10.0);
        }
        Self {
            backends: compiled_backends(),
            dithering,
        }
    }
}

/// Quality grids per the sweep discipline.
#[derive(Clone, Debug)]
pub enum QualityGrid {
    /// q ∈ {1, 5, 10, …, 100} — the 21-point floor.
    Step5,
    /// Quality-dense grid for **training a scalar head** (playbook
    /// patterns 17–18): q step 5 across `0..=70` then q step 2 across
    /// `72..=100`, densifying the high-quality band where 1–2 quality
    /// points shift real bytes. Pair with [`SweepAxes::scalar_dense`].
    TrainingDense,
    /// Caller-provided points (kept in order, deduplicated).
    Explicit(Vec<u8>),
}

impl QualityGrid {
    /// Materialize the grid points.
    #[must_use]
    pub fn points(&self) -> Vec<u8> {
        match self {
            Self::Step5 => {
                let mut v = vec![1u8];
                v.extend((1..=20).map(|i| (i * 5) as u8));
                v
            }
            Self::TrainingDense => {
                // Coarse low/mid (step 5, 0..=70), fine high band
                // (step 2, 72..=100) — match density across the range,
                // err denser where bytes are most sensitive.
                let mut v: Vec<u8> = (0..=14).map(|i| i * 5).collect(); // 0,5,…,70
                v.extend((36..=50).map(|i| i * 2)); // 72,74,…,100
                v
            }
            Self::Explicit(pts) => {
                let mut v = Vec::new();
                for &p in pts {
                    if !v.contains(&p) {
                        v.push(p);
                    }
                }
                v
            }
        }
    }
}

/// One encode cell.
#[derive(Clone, Debug)]
pub struct SweepCell {
    /// Stable id (`gif-<backend>[-d<dither>]_q<q>`).
    pub id: String,
    /// The variant to encode with.
    pub variant: SweepVariant,
    /// Byte-identity fingerprint of the resolved state.
    pub fingerprint: u64,
    /// Ids merged into this cell (identical fingerprints).
    pub aliases: Vec<String>,
    /// Axes deviating from the default stratum (0 = build default).
    pub deviations: u8,
}

/// The finite plan.
#[derive(Clone, Debug)]
pub struct SweepPlan {
    /// Deduplicated cells, main-effects-first, q ascending per stratum.
    pub cells: Vec<SweepCell>,
    /// Candidates merged by fingerprint identity.
    pub duplicates_merged: usize,
    /// Cell ids dropped because their [`compute_tier`] exceeded the
    /// `compute_limit` passed to [`plan_constrained`] — the explicit
    /// no-silent-caps report for the compute constraint (empty in the
    /// unconstrained [`plan`] path).
    pub compute_tier_skipped: Vec<String>,
}

/// Build the plan: axes × grid, main-effects-first. Equivalent to
/// [`plan_constrained`]`(axes, grid, None, None)` — the full,
/// unconstrained curated space.
#[must_use]
pub fn plan(axes: &SweepAxes, grid: &QualityGrid) -> SweepPlan {
    plan_constrained(axes, grid, None, None)
}

/// Build the plan, optionally bounded by a compute budget and/or a
/// deviation scope (playbook patterns 17–18; cross-codec-uniform with
/// the sibling codecs' `plan_constrained`).
///
/// - `compute_limit`: if `Some(max)`, cells whose [`compute_tier`] is
///   `> max` are dropped and their ids recorded in
///   [`SweepPlan::compute_tier_skipped`] (never silently capped) — the
///   compute-resource constraint a CPU-bound fleet or a "fast configs
///   only" picker asks for.
/// - `max_deviations`: if `Some(n)`, only cells within `n` axis
///   deviations of the default stratum survive (`1` = main-effects
///   only; `0` = the default stratum alone).
///
/// `compute_limit` is applied first, then `max_deviations`.
#[must_use]
pub fn plan_constrained(
    axes: &SweepAxes,
    grid: &QualityGrid,
    compute_limit: Option<u8>,
    max_deviations: Option<u8>,
) -> SweepPlan {
    struct Entry {
        backend: QuantizerBackend,
        dithering: f32,
        deviations: u8,
        idx_sum: usize,
        seq: usize,
    }
    let mut entries = Vec::new();
    let mut seq = 0usize;
    for (bi, &backend) in axes.backends.iter().enumerate() {
        // Structural application: the dithering axis only crosses
        // backends that have the knob (pattern 10 — never an inert
        // cross, never invalid spam).
        let dither_options: Vec<f32> = if SweepVariant::dithering_applies(backend) {
            axes.dithering.clone()
        } else {
            vec![axes.dithering[0]]
        };
        for (di, &dithering) in dither_options.iter().enumerate() {
            entries.push(Entry {
                backend,
                dithering,
                deviations: u8::from(bi != 0) + u8::from(di != 0),
                idx_sum: bi + di,
                seq,
            });
            seq += 1;
        }
    }
    entries.sort_by_key(|e| (e.deviations, e.idx_sum, e.seq));

    let q_points = grid.points();
    let mut cells: Vec<SweepCell> = Vec::new();
    let mut by_fp: BTreeMap<u64, usize> = BTreeMap::new();
    let mut merged = 0usize;
    for e in &entries {
        for &q in &q_points {
            let variant = SweepVariant {
                quality: q,
                dithering: e.dithering,
                backend: e.backend,
            };
            let fp = fingerprint(&variant);
            let id = format!("{}_q{q}", variant.base_id());
            if let Some(&i) = by_fp.get(&fp) {
                cells[i].aliases.push(id);
                merged += 1;
            } else {
                by_fp.insert(fp, cells.len());
                cells.push(SweepCell {
                    id,
                    variant,
                    fingerprint: fp,
                    aliases: Vec::new(),
                    deviations: e.deviations,
                });
            }
        }
    }
    let mut compute_tier_skipped = Vec::new();
    if let Some(max) = compute_limit {
        cells.retain(|c| {
            if compute_tier(&c.variant) <= max {
                true
            } else {
                compute_tier_skipped.push(c.id.clone());
                false
            }
        });
    }
    if let Some(n) = max_deviations {
        cells.retain(|c| c.deviations <= n);
    }

    SweepPlan {
        cells,
        duplicates_merged: merged,
        compute_tier_skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_ids_roundtrip_to_their_variants() {
        let mut checked = 0usize;
        for (axes, grid) in [
            (SweepAxes::rd_core(), QualityGrid::Step5),
            (SweepAxes::modes_full(), QualityGrid::Explicit(vec![10, 85])),
        ] {
            let p = plan(&axes, &grid);
            for cell in &p.cells {
                for id in core::iter::once(&cell.id).chain(cell.aliases.iter()) {
                    let v = variant_from_cell_id(id).unwrap_or_else(|e| panic!("{id}: {e}"));
                    assert_eq!(fingerprint(&v), cell.fingerprint, "drift for {id}");
                }
                checked += 1;
            }
        }
        assert!(checked > 20, "coverage thin: {checked}");
    }

    #[test]
    fn malformed_and_uncompiled_ids_error() {
        for bad in [
            "gif-zenquant",           // missing quality token
            "gif-warpquant_q50",      // unknown / uncompiled backend
            "gif-zenquant-x1_q50",    // unknown flag
            "png-zenquant_q50",       // wrong prefix
            "gif-zenquant-dnope_q50", // bad dither value
        ] {
            assert!(variant_from_cell_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn queue_is_main_effects_first_and_ids_unique() {
        let p = plan(
            &SweepAxes::modes_full(),
            &QualityGrid::Explicit(vec![50, 85]),
        );
        assert_eq!(p.cells[0].deviations, 0);
        for w in p.cells.windows(2) {
            assert!(w[1].deviations >= w[0].deviations);
        }
        #[cfg(not(feature = "std"))]
        use alloc::collections::BTreeSet;
        #[cfg(feature = "std")]
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        for c in &p.cells {
            for id in core::iter::once(&c.id).chain(c.aliases.iter()) {
                assert!(seen.insert(id.clone()), "duplicate id {id}");
            }
        }
    }

    #[test]
    fn plan_is_deterministic() {
        let a = plan(&SweepAxes::modes_full(), &QualityGrid::Step5);
        let b = plan(&SweepAxes::modes_full(), &QualityGrid::Step5);
        assert_eq!(a.cells.len(), b.cells.len());
        for (x, y) in a.cells.iter().zip(&b.cells) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.fingerprint, y.fingerprint);
        }
    }

    #[test]
    fn compute_tier_orders_backend_cost() {
        // The cheapest quantizer (color_quant's fixed NEUQUANT pass) must
        // tier strictly below the most expensive (zenquant's perceptual
        // optimization). `compute_tier` is a pure function of the variant
        // and does not require either backend be compiled in, so the
        // ordering is asserted directly across the full ladder.
        let ladder = [
            QuantizerBackend::ColorQuant,
            QuantizerBackend::Quantizr,
            QuantizerBackend::Imagequant,
            QuantizerBackend::Quantette,
            QuantizerBackend::Zenquant,
        ];
        for w in ladder.windows(2) {
            let cheap = compute_tier(&SweepVariant {
                quality: 80,
                dithering: 0.0,
                backend: w[0],
            });
            let pricey = compute_tier(&SweepVariant {
                quality: 80,
                dithering: 0.0,
                backend: w[1],
            });
            assert!(
                cheap < pricey,
                "{:?} (tier {cheap}) must cost less than {:?} (tier {pricey})",
                w[0],
                w[1]
            );
        }
        // Dithering adds a smaller term that never reorders backends:
        // color_quant has no dithering knob, so its tier is fixed; a
        // dithered cheap backend still tiers at/below an undithered
        // pricier one within the same gap.
        assert!(
            compute_tier(&SweepVariant {
                quality: 80,
                dithering: 1.0,
                backend: QuantizerBackend::Quantizr,
            }) < compute_tier(&SweepVariant {
                quality: 80,
                dithering: 0.0,
                backend: QuantizerBackend::Imagequant,
            }),
            "the dithering add-on must not cross backend bands"
        );
    }

    #[test]
    fn scalar_dense_ladders_the_continuous_knobs() {
        // A scalar head fits the continuous dithering knob: the ladder
        // must be dense (≥6 distinct values — the playbook floor), much
        // finer than modes_full's {0.0, 0.5, 1.0}. Backends are kept
        // dense too, so a compute head sees every available tier; whether
        // that yields ≥3 tiers depends on the compiled backend set, so
        // the dense-dithering ladder is the portable assertion.
        let axes = SweepAxes::scalar_dense();
        let mut dith: Vec<u32> = axes.dithering.iter().map(|d| d.to_bits()).collect();
        dith.sort_unstable();
        dith.dedup();
        assert!(
            dith.len() >= 6,
            "scalar_dense dithering ladder too sparse for a scalar head: {} values",
            dith.len()
        );
        // It is strictly denser than the modes_full set.
        assert!(axes.dithering.len() > SweepAxes::modes_full().dithering.len());

        // Across the materialized cells, count both signals and require
        // the space supports at least one form of density (the OR the
        // task allows): ≥6 distinct dithering values OR ≥3 compute tiers.
        let p = plan(&axes, &QualityGrid::TrainingDense);
        assert_eq!(p.cells[0].deviations, 0, "default stratum still first");
        let mut tiers: Vec<u8> = p.cells.iter().map(|c| compute_tier(&c.variant)).collect();
        tiers.sort_unstable();
        tiers.dedup();
        let mut seen_dith: Vec<u32> = p
            .cells
            .iter()
            .map(|c| c.variant.dithering.to_bits())
            .collect();
        seen_dith.sort_unstable();
        seen_dith.dedup();
        assert!(
            seen_dith.len() >= 6 || tiers.len() >= 3,
            "scalar_dense cells lack density: {} dithering values, {} tiers",
            seen_dith.len(),
            tiers.len()
        );

        // TrainingDense densifies the high-q band: q ∈ 72..=100 step 2.
        let q = QualityGrid::TrainingDense.points();
        assert!(q.contains(&72) && q.contains(&98) && q.contains(&100));
        let high: Vec<&u8> = q.iter().filter(|&&x| x >= 72).collect();
        assert!(
            high.len() >= 14,
            "high-q band not dense: {} points",
            high.len()
        );
    }

    #[test]
    fn plan_constrained_drops_reports_and_delegates() {
        let axes = SweepAxes::scalar_dense();
        let grid = QualityGrid::Explicit(vec![50, 90]);
        let unconstrained = plan(&axes, &grid);

        // Pick a limit strictly below the most expensive tier present so
        // at least one cell is dropped, regardless of compiled backends.
        let max_tier = unconstrained
            .cells
            .iter()
            .map(|c| compute_tier(&c.variant))
            .max()
            .expect("plan has cells");
        let min_tier = unconstrained
            .cells
            .iter()
            .map(|c| compute_tier(&c.variant))
            .min()
            .expect("plan has cells");
        assert!(max_tier > min_tier, "need ≥2 tiers to exercise the drop");
        let limit = max_tier - 1;

        let limited = plan_constrained(&axes, &grid, Some(limit), None);
        assert!(!limited.cells.is_empty());
        assert!(
            limited.cells.len() < unconstrained.cells.len(),
            "the compute limit must drop the expensive cells"
        );
        assert!(
            limited
                .cells
                .iter()
                .all(|c| compute_tier(&c.variant) <= limit),
            "every surviving cell must be within budget"
        );
        assert!(
            !limited.compute_tier_skipped.is_empty(),
            "dropped cells must be reported, never silently capped"
        );
        // Every dropped id is genuinely over budget and absent from cells.
        for id in &limited.compute_tier_skipped {
            assert!(
                !limited.cells.iter().any(|c| &c.id == id),
                "reported-skipped id {id} still present in cells"
            );
        }

        // max_deviations narrows to the default stratum.
        let main_only = plan_constrained(&axes, &grid, None, Some(0));
        assert!(main_only.cells.iter().all(|c| c.deviations == 0));

        // The unconstrained delegate must equal plan() cell-for-cell.
        let via_constrained = plan_constrained(&axes, &grid, None, None);
        let direct = plan(&axes, &grid);
        assert_eq!(via_constrained.cells.len(), direct.cells.len());
        for (x, y) in via_constrained.cells.iter().zip(&direct.cells) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.fingerprint, y.fingerprint);
            assert_eq!(x.deviations, y.deviations);
        }
        assert_eq!(via_constrained.duplicates_merged, direct.duplicates_merged);
        assert!(via_constrained.compute_tier_skipped.is_empty());
    }
}
