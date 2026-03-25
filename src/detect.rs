//! GIF source analysis and re-encoding recommendations.
//!
//! Analyzes a GIF file's structure to determine palette usage, animation
//! characteristics, and re-encoding opportunities — all from header parsing
//! without full pixel decoding.
//!
//! # Example
//!
//! ```rust,ignore
//! use zengif::detect::{probe, FormatSuggestion};
//!
//! let gif_data = std::fs::read("animation.gif").unwrap();
//! let info = probe(&gif_data).unwrap();
//!
//! println!("{}x{}, {} frames", info.width, info.height, info.frame_count);
//! println!("Global palette: {} colors", info.global_palette_size);
//!
//! for suggestion in &info.suggestions {
//!     println!("  - {:?}", suggestion);
//! }
//! ```

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// Result of probing a GIF file.
#[derive(Debug, Clone)]
pub struct GifProbe {
    /// Canvas width.
    pub width: u16,
    /// Canvas height.
    pub height: u16,
    /// GIF version (b"87a" or b"89a").
    pub version: [u8; 3],
    /// Number of colors in the global palette (0 if none).
    pub global_palette_size: u16,
    /// Whether the image is animated (more than 1 frame).
    pub is_animated: bool,
    /// Number of frames.
    pub frame_count: u32,
    /// Total duration in centiseconds (sum of all frame delays).
    pub total_duration_cs: u32,
    /// Whether any frame uses transparency.
    pub has_transparency: bool,
    /// Whether any frame uses local color tables (per-frame palettes).
    pub has_local_palettes: bool,
    /// Maximum local palette size encountered (0 if none).
    pub max_local_palette_size: u16,
    /// Whether any frame uses interlacing.
    pub has_interlacing: bool,
    /// Background color index (if global palette present).
    pub background_color_index: Option<u8>,
    /// Loop/repeat behavior from NETSCAPE extension.
    ///
    /// - `None` — no NETSCAPE extension found (GIF87a or single-frame GIF89a).
    ///   Treat as play-once.
    /// - `Some(0)` — loop forever.
    /// - `Some(n)` — loop `n` times.
    pub repeat: Option<u16>,
    /// Suggestions for re-encoding or format conversion.
    pub suggestions: Vec<FormatSuggestion>,
}

/// Suggestions for handling this GIF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSuggestion {
    /// Static GIF — PNG would be smaller and higher quality.
    ConvertToPng,
    /// Animated GIF — APNG or animated WebP would be smaller.
    ConvertToApng,
    /// Animated GIF — WebP animation would be much smaller.
    ConvertToWebPAnim,
    /// Palette has unused entries — fewer colors would compress better.
    ReducePalette,
    /// Per-frame palettes found — shared palette would compress better.
    UseSharedPalette,
    /// Very short animation (few frames) — consider static image.
    FewFrames,
    /// Already well-structured — re-encoding may help with better LZW.
    ReencodeForSize,
}

/// Errors that can occur during GIF probing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProbeError {
    /// Data is too short to be a GIF file.
    TooShort,
    /// Missing GIF signature.
    NotGif,
    /// GIF structure is truncated.
    Truncated,
}

impl core::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "data too short to be a GIF file"),
            Self::NotGif => write!(f, "not a GIF file (missing GIF87a/GIF89a signature)"),
            Self::Truncated => write!(f, "truncated GIF file"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProbeError {}

/// Probe a GIF file from its raw bytes.
///
/// Scans the GIF block structure to extract animation properties, palette
/// usage, and transparency info. No LZW decompression is performed.
pub fn probe(data: &[u8]) -> Result<GifProbe, ProbeError> {
    // GIF minimum: header(6) + logical screen descriptor(7) = 13 bytes
    if data.len() < 13 {
        return Err(ProbeError::TooShort);
    }

    // Check signature
    if &data[0..3] != b"GIF" {
        return Err(ProbeError::NotGif);
    }
    let version: [u8; 3] = [data[3], data[4], data[5]];
    if &version != b"87a" && &version != b"89a" {
        return Err(ProbeError::NotGif);
    }

    // Logical Screen Descriptor
    let width = u16::from_le_bytes([data[6], data[7]]);
    let height = u16::from_le_bytes([data[8], data[9]]);
    let packed = data[10];
    let has_global_ct = (packed & 0x80) != 0;
    let global_ct_size_bits = packed & 0x07;
    let global_palette_size = if has_global_ct {
        1u16 << (global_ct_size_bits + 1)
    } else {
        0
    };
    let background_color_index = if has_global_ct { Some(data[11]) } else { None };

    // Skip global color table
    let mut pos = 13;
    if has_global_ct {
        pos += 3 * global_palette_size as usize;
    }

    // Scan blocks
    let mut frame_count = 0u32;
    let mut total_duration_cs = 0u32;
    let mut has_transparency = false;
    let mut has_local_palettes = false;
    let mut max_local_palette_size = 0u16;
    let mut has_interlacing = false;
    let mut repeat: Option<u16> = None;
    let mut pending_gce_transparent = false;
    let mut pending_gce_delay = 0u16;

    while pos < data.len() {
        match data[pos] {
            0x3B => break, // Trailer
            0x21 => {
                // Extension block
                if pos + 2 >= data.len() {
                    break;
                }
                let label = data[pos + 1];
                pos += 2;

                if label == 0xF9 {
                    // Graphics Control Extension
                    if pos + 5 <= data.len() && data[pos] == 4 {
                        let gce_packed = data[pos + 1];
                        pending_gce_delay = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
                        pending_gce_transparent = (gce_packed & 0x01) != 0;
                    }
                }

                if label == 0xFF {
                    // Application Extension — check for NETSCAPE2.0 loop count.
                    // Layout: block_size(1)=11, "NETSCAPE2.0"(11), sub-block_size(1)=3,
                    //         sub-block_id(1)=1, loop_count(2 LE), terminator(1)=0.
                    if pos + 14 <= data.len()
                        && data[pos] == 11
                        && &data[pos + 1..pos + 12] == b"NETSCAPE2.0"
                        && pos + 17 <= data.len()
                        && data[pos + 12] == 3
                        && data[pos + 13] == 1
                    {
                        repeat = Some(u16::from_le_bytes([data[pos + 14], data[pos + 15]]));
                    }
                }

                // Skip sub-blocks
                while pos < data.len() {
                    let block_size = data[pos] as usize;
                    pos += 1;
                    if block_size == 0 {
                        break;
                    }
                    pos += block_size;
                }
            }
            0x2C => {
                // Image Descriptor
                if pos + 10 > data.len() {
                    break;
                }
                frame_count += 1;
                total_duration_cs += pending_gce_delay as u32;
                if pending_gce_transparent {
                    has_transparency = true;
                }

                let img_packed = data[pos + 9];
                let has_local_ct = (img_packed & 0x80) != 0;
                let interlace_flag = (img_packed & 0x40) != 0;

                if interlace_flag {
                    has_interlacing = true;
                }

                if has_local_ct {
                    has_local_palettes = true;
                    let local_ct_bits = img_packed & 0x07;
                    let local_size = 1u16 << (local_ct_bits + 1);
                    max_local_palette_size = max_local_palette_size.max(local_size);
                    pos += 10 + 3 * local_size as usize;
                } else {
                    pos += 10;
                }

                // Skip LZW minimum code size
                if pos >= data.len() {
                    break;
                }
                pos += 1; // LZW min code size

                // Skip sub-blocks (image data)
                while pos < data.len() {
                    let block_size = data[pos] as usize;
                    pos += 1;
                    if block_size == 0 {
                        break;
                    }
                    pos += block_size;
                }

                // Reset GCE state
                pending_gce_transparent = false;
                pending_gce_delay = 0;
            }
            _ => {
                // Unknown block — skip
                pos += 1;
            }
        }
    }

    let is_animated = frame_count > 1;

    // Build suggestions
    let mut suggestions = Vec::new();

    if !is_animated {
        suggestions.push(FormatSuggestion::ConvertToPng);
    } else {
        suggestions.push(FormatSuggestion::ConvertToApng);
        suggestions.push(FormatSuggestion::ConvertToWebPAnim);

        if frame_count <= 3 {
            suggestions.push(FormatSuggestion::FewFrames);
        }
    }

    if has_local_palettes {
        suggestions.push(FormatSuggestion::UseSharedPalette);
    }

    suggestions.push(FormatSuggestion::ReencodeForSize);

    Ok(GifProbe {
        width,
        height,
        version,
        global_palette_size,
        is_animated,
        frame_count,
        total_duration_cs,
        has_transparency,
        has_local_palettes,
        max_local_palette_size,
        has_interlacing,
        background_color_index,
        repeat,
        suggestions,
    })
}

impl GifProbe {
    /// Total animation duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.total_duration_cs as f32 / 100.0
    }

    /// Average frame rate (frames per second).
    pub fn fps(&self) -> f32 {
        if self.total_duration_cs == 0 || self.frame_count <= 1 {
            return 0.0;
        }
        self.frame_count as f32 / self.duration_secs()
    }
}

#[cfg(feature = "zencodec")]
impl zencodec::SourceEncodingDetails for GifProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        None
    }

    fn is_lossless(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_too_short() {
        assert_eq!(probe(&[]).unwrap_err(), ProbeError::TooShort);
        assert_eq!(probe(&[0; 12]).unwrap_err(), ProbeError::TooShort);
    }

    #[test]
    fn test_probe_not_gif() {
        assert_eq!(probe(&[0; 13]).unwrap_err(), ProbeError::NotGif);
        assert_eq!(probe(b"PNG89a0000000").unwrap_err(), ProbeError::NotGif);
    }

    #[test]
    fn test_probe_minimal_gif87a() {
        // Minimal GIF87a with no global color table
        let mut data = Vec::new();
        data.extend_from_slice(b"GIF87a");
        data.extend_from_slice(&10u16.to_le_bytes()); // width
        data.extend_from_slice(&10u16.to_le_bytes()); // height
        data.push(0x00); // packed: no global CT
        data.push(0); // bg color index
        data.push(0); // pixel aspect ratio
        data.push(0x3B); // trailer

        let info = probe(&data).unwrap();
        assert_eq!(info.width, 10);
        assert_eq!(info.height, 10);
        assert_eq!(info.global_palette_size, 0);
        assert_eq!(info.frame_count, 0);
        assert!(!info.is_animated);
    }
}
