//! zennode node definitions for GIF encoding.
//!
//! Defines [`EncodeGif`] with parameters for dithering and palette size.

use zennode::*;

/// GIF encoding with dithering and palette size options.
///
/// GIF supports up to 256 colors per frame. The `max_colors` parameter
/// controls palette size, and `dithering` controls the strength of
/// error-diffusion dithering used to approximate colors outside the palette.
///
/// JSON API: `{ "dithering": 0.5, "max_colors": 256 }`
/// RIAPI: `?gif.dithering=0.5&gif.max_colors=256`
#[derive(Node, Clone, Debug)]
#[node(id = "zengif.encode", group = Encode, role = Encode)]
#[node(tags("codec", "gif", "lossless", "encode", "animation"))]
pub struct EncodeGif {
    /// Generic quality 0-100 (mapped via with_generic_quality at execution time).
    ///
    /// When set (>= 0), this value is passed through zencodec's
    /// `with_generic_quality()` which maps it to the codec's native
    /// quality scale. For GIF, this primarily affects palette size
    /// and dithering behavior.
    #[param(range(0..=100), default = -1, step = 1)]
    #[param(unit = "", section = "Main", label = "Quality")]
    #[kv("quality")]
    pub quality: i32,

    /// Dithering strength (0.0 = none, 1.0 = full).
    ///
    /// Controls how aggressively error-diffusion dithering is applied.
    /// Higher values reduce color banding but add noise that compresses
    /// poorly. For animated GIFs, lower values (0.2-0.5) often produce
    /// smaller files with less inter-frame flicker.
    #[param(range(0.0..=1.0), default = 0.5, step = 0.05)]
    #[param(unit = "", section = "Main", label = "Dithering")]
    #[kv("gif.dithering", "dithering")]
    pub dithering: f32,

    /// Maximum palette size (2-256 colors).
    ///
    /// GIF palettes can hold up to 256 entries. Reducing this value
    /// forces a smaller palette, which can significantly reduce file
    /// size at the cost of color accuracy.
    #[param(range(2..=256), default = 256, step = 1)]
    #[param(unit = "colors", section = "Main", label = "Max Colors")]
    #[kv("gif.max_colors", "max_colors")]
    pub max_colors: i32,
}

impl Default for EncodeGif {
    fn default() -> Self {
        Self {
            quality: -1,
            dithering: 0.5,
            max_colors: 256,
        }
    }
}

/// Registration function for aggregating crates.
pub fn register(registry: &mut NodeRegistry) {
    registry.register(&ENCODE_GIF_NODE);
}

/// All GIF zennode definitions.
pub static ALL: &[&dyn NodeDef] = &[&ENCODE_GIF_NODE];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_metadata() {
        let schema = ENCODE_GIF_NODE.schema();
        assert_eq!(schema.id, "zengif.encode");
        assert_eq!(schema.group, NodeGroup::Encode);
        assert_eq!(schema.role, NodeRole::Encode);
        assert!(schema.tags.contains(&"gif"));
        assert!(schema.tags.contains(&"animation"));
        assert!(schema.tags.contains(&"codec"));
        assert!(schema.tags.contains(&"lossless"));
        assert!(schema.tags.contains(&"encode"));
    }

    #[test]
    fn param_count_and_names() {
        let schema = ENCODE_GIF_NODE.schema();
        let names: Vec<&str> = schema.params.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"quality"));
        assert!(names.contains(&"dithering"));
        assert!(names.contains(&"max_colors"));
    }

    #[test]
    fn defaults() {
        let node = ENCODE_GIF_NODE.create_default().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(-1)));
        assert_eq!(node.get_param("dithering"), Some(ParamValue::F32(0.5)));
        assert_eq!(node.get_param("max_colors"), Some(ParamValue::I32(256)));
    }

    #[test]
    fn from_kv_dithering() {
        let mut kv = KvPairs::from_querystring("gif.dithering=0.3&gif.max_colors=128");
        let node = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("dithering"), Some(ParamValue::F32(0.3)));
        assert_eq!(node.get_param("max_colors"), Some(ParamValue::I32(128)));
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn from_kv_alias() {
        // "dithering" and "max_colors" are aliases for "gif.dithering" and "gif.max_colors"
        let mut kv = KvPairs::from_querystring("dithering=0.7&max_colors=64");
        let node = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("dithering"), Some(ParamValue::F32(0.7)));
        assert_eq!(node.get_param("max_colors"), Some(ParamValue::I32(64)));
    }

    #[test]
    fn from_kv_generic_quality() {
        let mut kv = KvPairs::from_querystring("quality=80");
        let node = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(80)));
    }

    #[test]
    fn from_kv_no_match() {
        let mut kv = KvPairs::from_querystring("w=800&h=600");
        let result = ENCODE_GIF_NODE.from_kv(&mut kv).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn json_round_trip() {
        let mut params = ParamMap::new();
        params.insert("quality".into(), ParamValue::I32(75));
        params.insert("dithering".into(), ParamValue::F32(0.8));
        params.insert("max_colors".into(), ParamValue::I32(128));

        let node = ENCODE_GIF_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(75)));
        assert_eq!(node.get_param("dithering"), Some(ParamValue::F32(0.8)));
        assert_eq!(node.get_param("max_colors"), Some(ParamValue::I32(128)));

        // Round-trip
        let exported = node.to_params();
        let node2 = ENCODE_GIF_NODE.create(&exported).unwrap();
        assert_eq!(node2.get_param("quality"), Some(ParamValue::I32(75)));
        assert_eq!(node2.get_param("dithering"), Some(ParamValue::F32(0.8)));
        assert_eq!(node2.get_param("max_colors"), Some(ParamValue::I32(128)));
    }

    #[test]
    fn downcast_to_concrete() {
        let node = ENCODE_GIF_NODE.create_default().unwrap();
        let enc = node.as_any().downcast_ref::<EncodeGif>().unwrap();
        assert_eq!(enc.quality, -1);
        assert_eq!(enc.max_colors, 256);
        assert!((enc.dithering - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn registry_integration() {
        let mut registry = NodeRegistry::new();
        register(&mut registry);
        assert!(registry.get("zengif.encode").is_some());

        let result = registry.from_querystring("gif.dithering=0.4&gif.max_colors=64");
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.instances[0].schema().id, "zengif.encode");

        // generic quality also triggers the node
        let result2 = registry.from_querystring("quality=80");
        assert_eq!(result2.instances.len(), 1);
        assert_eq!(result2.instances[0].schema().id, "zengif.encode");
    }
}
