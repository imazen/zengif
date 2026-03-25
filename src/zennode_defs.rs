//! Zennode encode node definition for zengif.
//!
//! Provides [`EncodeGif`], a self-documenting pipeline node that bridges
//! zennode's parameter system with zengif's [`GifEncoderConfig`].
//!
//! Feature-gated behind `feature = "zennode"`.

extern crate alloc;
use alloc::string::String;

use zennode::*;

use crate::Repeat;
use crate::codec::GifEncoderConfig;

/// GIF encoder settings (zennode node).
///
/// Controls GIF encoding quality, quantization, animation loop behavior,
/// and transparency optimization. Supports both RIAPI querystring keys
/// and JSON API fields.
///
/// **RIAPI**: `?gif.quality=80&gif.dithering=0.5&gif.loop=infinite`
/// **JSON**: `{ "quality": 80, "dithering": 0.5, "loop_count": "infinite" }`
///
/// Convert to [`GifEncoderConfig`] via [`to_encoder_config()`](EncodeGif::to_encoder_config).
#[derive(Node, Clone, Debug)]
#[node(id = "zengif.encode", group = Encode, role = Encode)]
#[node(tags("gif", "encode", "animation", "palette"))]
pub struct EncodeGif {
    /// Palette quality (1-100). Higher values produce better-looking palettes
    /// at the cost of larger files and slower encoding.
    #[param(range(1.0..=100.0), default = 80.0, step = 1.0)]
    #[param(section = "Quality", label = "Palette Quality")]
    #[kv("gif.quality")]
    pub quality: f32,

    /// Dithering level (0.0 = none, 1.0 = full). Lower values produce less
    /// noise and better LZW compression. Use 0.0 for re-encoding already-dithered content.
    #[param(range(0.0..=1.0), default = 0.5, step = 0.05)]
    #[param(section = "Quality", label = "Dithering")]
    #[kv("gif.dithering", "gif.dither")]
    pub dithering: f32,

    /// Lossy frame differencing tolerance per channel (0-255, 0 = lossless).
    /// Pixels within tolerance of the previous frame are marked unchanged,
    /// reducing dirty region size and improving compression.
    #[param(range(0.0..=255.0), default = 0.0, identity = 0.0, step = 1.0)]
    #[param(section = "Quality", label = "Lossy Tolerance")]
    #[kv("gif.lossy")]
    pub lossy_tolerance: f32,

    /// Quantizer backend selection. "auto" picks the best available.
    /// Other values: "zenquant", "quantette", "imagequant", "quantizr", "color_quant".
    #[param(default = "auto")]
    #[param(section = "Advanced", label = "Quantizer")]
    #[kv("gif.quantizer")]
    pub quantizer: String,

    /// Use a shared palette across all animation frames. Reduces flicker
    /// and improves LZW compression at the cost of per-frame color accuracy.
    #[param(default = true)]
    #[param(section = "Animation", label = "Shared Palette")]
    #[kv("gif.shared_palette")]
    pub shared_palette: bool,

    /// Per-frame palette error threshold (RMSE, 0-255 RGB scale) for hybrid
    /// palette mode. Frames exceeding this threshold get their own local palette.
    #[param(range(0.0..=50.0), default = 5.0, step = 0.5)]
    #[param(section = "Animation", label = "Palette Error Threshold")]
    #[kv("gif.palette_threshold")]
    pub palette_error_threshold: f32,

    /// Animation loop behavior: "infinite", "once", or a numeric repeat count.
    #[param(default = "infinite")]
    #[param(section = "Animation", label = "Loop")]
    #[kv("gif.loop")]
    pub loop_count: String,

    /// Enable transparency optimization: unchanged pixels between frames
    /// are encoded as transparent, improving compression.
    #[param(default = true)]
    #[param(section = "Advanced", label = "Transparency Optimization")]
    #[kv("gif.transparency")]
    pub transparency_optimization: bool,
}

impl Default for EncodeGif {
    fn default() -> Self {
        Self {
            quality: 80.0,
            dithering: 0.5,
            lossy_tolerance: 0.0,
            quantizer: String::from("auto"),
            shared_palette: true,
            palette_error_threshold: 5.0,
            loop_count: String::from("infinite"),
            transparency_optimization: true,
        }
    }
}

impl EncodeGif {
    /// Convert this node into a [`GifEncoderConfig`] for use with zengif's
    /// encoding pipeline.
    pub fn to_encoder_config(&self) -> GifEncoderConfig {
        let mut config = GifEncoderConfig::new();

        // Quality
        config = config.with_quality(self.quality);

        // Lossy tolerance (f32 -> u8)
        config = config.with_lossy_tolerance(self.lossy_tolerance.clamp(0.0, 255.0) as u8);

        // Loop count
        config = config.with_repeat(self.parse_loop_count());

        // Transparency
        config = config.with_transparency(self.transparency_optimization);

        // Quantizer-gated settings
        #[cfg(any(
            feature = "zenquant",
            feature = "imagequant",
            feature = "quantizr",
            feature = "exoquant-deprecated",
            feature = "color_quant"
        ))]
        {
            config = config.with_dithering(self.dithering);
            config = config.with_shared_palette(self.shared_palette);
            config = config.with_palette_error_threshold(Some(self.palette_error_threshold));

            if let Some(q) = self.parse_quantizer() {
                config = config.with_quantizer(q);
            }
        }

        config
    }

    /// Parse the `loop_count` string into a [`Repeat`] value.
    fn parse_loop_count(&self) -> Repeat {
        match self.loop_count.to_ascii_lowercase().as_str() {
            "infinite" | "forever" | "loop" | "" => Repeat::Infinite,
            "once" | "1" | "no" | "none" => Repeat::Once,
            other => other
                .parse::<u16>()
                .map(Repeat::Count)
                .unwrap_or(Repeat::Infinite),
        }
    }

    /// Parse the `quantizer` string into a [`Quantizer`] value.
    ///
    /// Returns `None` for "auto" (let `GifEncoderConfig` use its default).
    #[cfg(any(
        feature = "zenquant",
        feature = "imagequant",
        feature = "quantizr",
        feature = "exoquant-deprecated",
        feature = "color_quant"
    ))]
    fn parse_quantizer(&self) -> Option<crate::Quantizer> {
        match self.quantizer.to_ascii_lowercase().as_str() {
            "auto" | "" => Some(crate::Quantizer::auto()),
            #[cfg(feature = "zenquant")]
            "zenquant" => Some(crate::Quantizer::zenquant()),
            #[cfg(feature = "quantette")]
            "quantette" => Some(crate::Quantizer::quantette()),
            #[cfg(feature = "imagequant")]
            "imagequant" => Some(crate::Quantizer::imagequant()),
            #[cfg(feature = "quantizr")]
            "quantizr" => Some(crate::Quantizer::quantizr()),
            #[cfg(feature = "color_quant")]
            "color_quant" => Some(crate::Quantizer::color_quant()),
            _ => None, // Unknown quantizer name, leave default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_basics() {
        let schema = ENCODE_GIF_NODE.schema();
        assert_eq!(schema.id, "zengif.encode");
        assert_eq!(schema.group, NodeGroup::Encode);
        assert_eq!(schema.role, NodeRole::Encode);
        assert!(schema.tags.contains(&"gif"));
        assert!(schema.tags.contains(&"encode"));
        assert!(schema.tags.contains(&"animation"));
        assert!(schema.tags.contains(&"palette"));

        let param_names: alloc::vec::Vec<&str> = schema.params.iter().map(|p| p.name).collect();
        assert!(param_names.contains(&"quality"));
        assert!(param_names.contains(&"dithering"));
        assert!(param_names.contains(&"lossy_tolerance"));
        assert!(param_names.contains(&"quantizer"));
        assert!(param_names.contains(&"shared_palette"));
        assert!(param_names.contains(&"palette_error_threshold"));
        assert!(param_names.contains(&"loop_count"));
        assert!(param_names.contains(&"transparency_optimization"));
    }

    #[test]
    fn default_values() {
        let node = ENCODE_GIF_NODE.create_default().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::F32(80.0)));
        assert_eq!(node.get_param("dithering"), Some(ParamValue::F32(0.5)));
        assert_eq!(
            node.get_param("lossy_tolerance"),
            Some(ParamValue::F32(0.0))
        );
        assert_eq!(
            node.get_param("quantizer"),
            Some(ParamValue::Str("auto".into()))
        );
        assert_eq!(
            node.get_param("shared_palette"),
            Some(ParamValue::Bool(true))
        );
        assert_eq!(
            node.get_param("palette_error_threshold"),
            Some(ParamValue::F32(5.0))
        );
        assert_eq!(
            node.get_param("loop_count"),
            Some(ParamValue::Str("infinite".into()))
        );
        assert_eq!(
            node.get_param("transparency_optimization"),
            Some(ParamValue::Bool(true))
        );
    }

    #[test]
    fn kv_keys_coverage() {
        let schema = ENCODE_GIF_NODE.schema();

        let quality_param = schema.params.iter().find(|p| p.name == "quality").unwrap();
        assert_eq!(quality_param.kv_keys, &["gif.quality"]);

        let dither_param = schema
            .params
            .iter()
            .find(|p| p.name == "dithering")
            .unwrap();
        assert!(dither_param.kv_keys.contains(&"gif.dithering"));
        assert!(dither_param.kv_keys.contains(&"gif.dither"));

        let loop_param = schema
            .params
            .iter()
            .find(|p| p.name == "loop_count")
            .unwrap();
        assert_eq!(loop_param.kv_keys, &["gif.loop"]);
    }

    #[test]
    fn kv_parsing_basic() {
        let mut kv = KvPairs::from_querystring("gif.quality=90&gif.loop=once");
        let node = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::F32(90.0)));
        assert_eq!(
            node.get_param("loop_count"),
            Some(ParamValue::Str("once".into()))
        );
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn kv_parsing_no_match() {
        let mut kv = KvPairs::from_querystring("w=800&h=600");
        let result = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_loop_count_variants() {
        let mut node = EncodeGif::default();

        node.loop_count = String::from("infinite");
        assert!(matches!(node.parse_loop_count(), Repeat::Infinite));

        node.loop_count = String::from("once");
        assert!(matches!(node.parse_loop_count(), Repeat::Once));

        node.loop_count = String::from("5");
        assert!(matches!(node.parse_loop_count(), Repeat::Count(5)));

        node.loop_count = String::from("forever");
        assert!(matches!(node.parse_loop_count(), Repeat::Infinite));

        node.loop_count = String::new();
        assert!(matches!(node.parse_loop_count(), Repeat::Infinite));
    }

    #[test]
    fn to_encoder_config_defaults() {
        let node = EncodeGif::default();
        let _config = node.to_encoder_config();
        // Should not panic — validates the mapping works end-to-end.
    }

    #[test]
    fn downcast() {
        let node = ENCODE_GIF_NODE.create_default().unwrap();
        let enc = node.as_any().downcast_ref::<EncodeGif>().unwrap();
        assert_eq!(enc.quality, 80.0);
        assert!(enc.transparency_optimization);
    }
}
