//! Internal-params bundle for cross-codec uniformity (`__expert` feature).
//!
//! [`InternalParams`] collects the encoder knobs that codec-calibration
//! sweeps and the picker training pipeline want to drive externally,
//! mirroring `zenjpeg::encode::internal_params::InternalParams` so a
//! single picker model can emit the same bundle shape for every codec in
//! the zen family.
//!
//! Production callers should use the per-axis builder methods on
//! [`EncoderConfig`] directly ([`quality`](EncoderConfig::quality),
//! [`dithering`](EncoderConfig::dithering),
//! [`quantizer_preference`](EncoderConfig::quantizer_preference), …).
//! Reach for [`InternalParams`] only when you need to vary calibration
//! axes from outside the codec — e.g., from a Pareto sweep harness or a
//! learned picker that emits per-image axis values.
//!
//! Each field is `Option<_>`. `None` means "leave the
//! [`EncoderConfig`]'s existing value alone." This is partial-merge, the
//! same shape every zen codec's bundle uses, so callers can override one
//! axis at a time without spelling out the rest.
//!
//! The bundled axes mirror what [`crate::sweep::SweepVariant`] varies for
//! GIF stills — the quantizer backend, dithering, and quality — plus
//! `use_transparency` (a public encoder setter a picker may want to drive
//! on an alpha-bearing corpus, even though the still-image byte sweep
//! pins it off). The backend axis uses
//! [`QuantizerBackend`](crate::quantize::QuantizerBackend) rather than the
//! cfg-gated [`Quantizer`](crate::quantize::Quantizer): `QuantizerBackend`
//! is representable regardless of the feature set, so one serialized
//! bundle drives every build (the
//! [`quantizer_preference`](EncoderConfig::quantizer_preference) contract
//! — first compiled-in backend wins, loud error if none).

#![cfg(feature = "__expert")]

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::encode::EncoderConfig;
use crate::quantize::QuantizerBackend;

/// Bundle of advanced encoder tuning knobs. Expert-only.
///
/// Intended for codec calibration sweeps and the picker training
/// pipeline. Production callers should rely on the per-axis builder
/// methods on [`EncoderConfig`] instead.
///
/// Every field is `Option<_>`. `None` means "leave the
/// [`EncoderConfig`]'s existing value alone." Apply with
/// [`EncoderConfig::with_internal_params`].
///
/// `#[non_exhaustive]` so adding a new axis is a non-breaking change.
///
/// The `quality` and `dithering` axes are gated behind a quantizer
/// backend feature (`zenquant` / `quantette` / `imagequant` / `quantizr`
/// / `color_quant`), matching the cfg on the [`EncoderConfig`] fields
/// they drive — without a quantizer backend compiled in there is no
/// quantization to tune.
///
/// ```ignore
/// # #[cfg(all(feature = "std", feature = "__expert"))]
/// # {
/// use zengif::{EncoderConfig, InternalParams, QuantizerBackend};
///
/// let cfg = EncoderConfig::new().with_internal_params(InternalParams {
///     quantizer_preference: Some(vec![QuantizerBackend::Quantizr]),
///     use_transparency: Some(false),
///     ..Default::default()
/// });
/// # }
/// ```
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct InternalParams {
    /// Quantizer backend **preference series** — "use the first of these
    /// this build compiled in" (the sweep's backend axis).
    ///
    /// Applied via [`EncoderConfig::quantizer_preference`]. Uses
    /// [`QuantizerBackend`] (not the cfg-gated `Quantizer`) so one bundle
    /// is feature-set portable: a picker can emit a backend order without
    /// knowing the consumer's compiled feature set, and encoding errors
    /// loudly if none of the series is available rather than silently
    /// substituting a different quantizer.
    pub quantizer_preference: Option<Vec<QuantizerBackend>>,

    /// Quantizer quality 0–100 (the sweep's quality dial).
    ///
    /// Applied via [`EncoderConfig::quality`] (clamped to `1..=100`).
    /// Only the optional `imagequant` backend consults it, but it is the
    /// metric grid dial every still-image sweep varies.
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    pub quality: Option<u8>,

    /// Dithering level 0.0–1.0 (the sweep's dithering axis).
    ///
    /// Applied via [`EncoderConfig::dithering`] (clamped to `0.0..=1.0`).
    /// Lower values diffuse less error and compress better; `0.0` is the
    /// re-encode-already-dithered setting.
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    pub dithering: Option<f32>,

    /// Toggle transparency optimization for unchanged pixels.
    ///
    /// Applied via [`EncoderConfig::use_transparency`]. The still-image
    /// byte sweep pins this off (it needs an alpha-bearing corpus +
    /// alpha-aware scoring), but it is a public encoder setter a picker
    /// might drive.
    pub use_transparency: Option<bool>,
}

impl EncoderConfig {
    /// Apply an [`InternalParams`] bundle, overriding each axis whose
    /// field is `Some(_)` and leaving the rest untouched (partial-merge).
    ///
    /// Each `Some` field routes through the corresponding builder setter,
    /// so this is exactly equivalent to calling those setters by hand.
    ///
    /// Cross-codec uniformity entry point (`__expert`-gated): mirrors
    /// `zenjpeg`'s `EncoderConfig::with_internal_params` so external
    /// pipelines can drive every zen codec with one bundle shape.
    #[must_use]
    pub fn with_internal_params(mut self, params: InternalParams) -> Self {
        if let Some(series) = params.quantizer_preference {
            self = self.quantizer_preference(series);
        }
        #[cfg(any(
            feature = "zenquant",
            feature = "quantette",
            feature = "imagequant",
            feature = "quantizr",
            feature = "color_quant"
        ))]
        if let Some(q) = params.quality {
            self = self.quality(q);
        }
        #[cfg(any(
            feature = "zenquant",
            feature = "quantette",
            feature = "imagequant",
            feature = "quantizr",
            feature = "color_quant"
        ))]
        if let Some(d) = params.dithering {
            self = self.dithering(d);
        }
        if let Some(t) = params.use_transparency {
            self = self.use_transparency(t);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> EncoderConfig {
        EncoderConfig::new()
    }

    /// Empty `InternalParams` (all `None`) leaves the config bytewise
    /// equivalent to the constructor default — debug-format equality
    /// is a coarse but reliable check that no field flipped.
    #[test]
    fn default_internal_params_is_noop() {
        let cfg = baseline();
        let cfg2 = baseline().with_internal_params(InternalParams::default());
        assert_eq!(format!("{cfg:?}"), format!("{cfg2:?}"));
    }

    #[test]
    fn quantizer_preference_field_applies() {
        let series = vec![QuantizerBackend::Quantizr, QuantizerBackend::ColorQuant];
        let cfg = baseline().with_internal_params(InternalParams {
            quantizer_preference: Some(series.clone()),
            ..Default::default()
        });
        assert_eq!(cfg.quantizer_preference.as_deref(), Some(series.as_slice()));
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn quality_field_applies_and_clamps() {
        let cfg = baseline().with_internal_params(InternalParams {
            quality: Some(42),
            ..Default::default()
        });
        assert_eq!(cfg.quality, 42);
        // The setter clamps to 1..=100.
        let clamped = baseline().with_internal_params(InternalParams {
            quality: Some(200),
            ..Default::default()
        });
        assert_eq!(clamped.quality, 100);
    }

    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    #[test]
    fn dithering_field_applies_and_clamps() {
        let cfg = baseline().with_internal_params(InternalParams {
            dithering: Some(0.25),
            ..Default::default()
        });
        assert!((cfg.dithering - 0.25).abs() < 1e-6);
        // Out-of-range clamps to 0.0..=1.0.
        let clamped = baseline().with_internal_params(InternalParams {
            dithering: Some(5.0),
            ..Default::default()
        });
        assert!((clamped.dithering - 1.0).abs() < 1e-6);
    }

    #[test]
    fn use_transparency_field_applies() {
        // Default is true; setting Some(false) must flip it.
        assert!(baseline().use_transparency);
        let cfg = baseline().with_internal_params(InternalParams {
            use_transparency: Some(false),
            ..Default::default()
        });
        assert!(!cfg.use_transparency);
    }

    #[test]
    fn unset_fields_leave_values_alone() {
        // Start from use_transparency=false, then apply a bundle that
        // doesn't touch it — the false must survive.
        let cfg = baseline()
            .use_transparency(false)
            .with_internal_params(InternalParams {
                quantizer_preference: Some(vec![QuantizerBackend::Quantizr]),
                ..Default::default()
            });
        assert!(
            !cfg.use_transparency,
            "use_transparency=None must not reset an existing false"
        );
        assert_eq!(
            cfg.quantizer_preference.as_deref(),
            Some([QuantizerBackend::Quantizr].as_slice())
        );
    }
}
