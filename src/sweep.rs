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
}

/// Quality grids per the sweep discipline.
#[derive(Clone, Debug)]
pub enum QualityGrid {
    /// q ∈ {1, 5, 10, …, 100} — the 21-point floor.
    Step5,
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
}

/// Build the plan: axes × grid, main-effects-first.
#[must_use]
pub fn plan(axes: &SweepAxes, grid: &QualityGrid) -> SweepPlan {
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
    SweepPlan {
        cells,
        duplicates_merged: merged,
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
}
