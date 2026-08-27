//! Native grayscale fast path.
//!
//! An 8-bit grayscale image has at most 256 distinct gray values (0-255), so a
//! GIF palette can represent it *exactly* — no color-distance search, no
//! k-means, no dithering needed. This module detects grayscale frames (every
//! opaque pixel has `r == g == b`) and builds that exact palette directly,
//! skipping the general RGBA quantizer entirely.
//!
//! For genuinely grayscale content (document scans, plots, diagrams, and the
//! `GRAY8_SRGB` / `GRAYF32_LINEAR` codec inputs that decode to `r == g == b`)
//! this is both faster *and* lossless. Frames that contain any real color fall
//! back to the configured quantizer via the `None` / `Option` return values.
//!
//! The fast path is engaged at the encoder level (see `encode::encoder`) for
//! both per-frame and shared-palette modes; it is not a [`QuantizerTrait`]
//! backend, because palette selection here is exact and content-driven rather
//! than a configurable algorithm.
//!
//! [`QuantizerTrait`]: super::QuantizerTrait

use super::QuantizedFrame;
use crate::types::Rgba;

/// An exact grayscale palette plus the value→index lookup table used to remap
/// frames against it.
///
/// Built once (per frame in per-frame mode, or once across all buffered frames
/// in shared-palette mode) by [`GrayPalette::try_build`], then applied to each
/// frame with [`GrayPalette::remap`].
pub(crate) struct GrayPalette {
    /// RGB color table for the GIF (3 bytes per entry).
    palette: Vec<u8>,
    /// Maps any 8-bit gray value to its palette index (nearest present gray).
    ///
    /// Defined for all 256 values so remapping is a single, branch-free array
    /// lookup with no bounds check possible to fail.
    lut: [u8; 256],
    /// Index of the dedicated transparent slot, if one was reserved.
    transparent_index: Option<u8>,
}

impl GrayPalette {
    /// Try to build an exact grayscale palette covering every supplied frame.
    ///
    /// Returns `None` — signalling "let the general quantizer handle it" — when:
    /// - any opaque pixel has unequal channels (the content is not grayscale), or
    /// - the frames have no opaque pixels at all (degenerate), or
    /// - a transparent slot is required but all 256 gray levels are in use, so
    ///   there is no room for it without dropping a level (corrupting either a
    ///   gray value or the transparency — neither is acceptable).
    ///
    /// `force_transparent` reserves a transparent slot even when the supplied
    /// frames contain no transparent pixels yet. Shared-palette mode sets this
    /// when later frames may introduce transparency via frame differencing
    /// (the buffered frames seen here are the raw, fully-opaque inputs).
    pub(crate) fn try_build(frames: &[&[Rgba]], force_transparent: bool) -> Option<Self> {
        let mut present = [false; 256];
        let mut saw_transparent = false;

        for &frame in frames {
            for &p in frame {
                if p.a == 0 {
                    // Transparent pixels carry no gray level; they map to the
                    // reserved transparent slot regardless of their RGB.
                    saw_transparent = true;
                } else if p.r == p.g && p.g == p.b {
                    present[p.r as usize] = true;
                } else {
                    // Genuine color — not a grayscale frame set.
                    return None;
                }
            }
        }

        // Present gray values in ascending order (deterministic palette layout).
        let grays: Vec<u8> = (0..256u16)
            .filter(|&v| present[v as usize])
            .map(|v| v as u8)
            .collect();

        if grays.is_empty() {
            // Nothing opaque to encode — leave it to the general path.
            return None;
        }

        let reserve_transparent = force_transparent || saw_transparent;
        let mut entry_count = grays.len();
        let transparent_index = if reserve_transparent {
            if entry_count >= 256 {
                // All 256 gray levels used; no room for a transparent slot.
                return None;
            }
            let ti = entry_count as u8;
            entry_count += 1;
            Some(ti)
        } else {
            None
        };

        // Build the color table: one [g, g, g] per gray level, plus an optional
        // transparent slot whose RGB is never shown (only its index matters).
        let mut palette = Vec::with_capacity(entry_count * 3);
        for &g in &grays {
            palette.extend_from_slice(&[g, g, g]);
        }
        if transparent_index.is_some() {
            palette.extend_from_slice(&[0, 0, 0]);
        }

        // value → nearest-present-gray index, filled with a single ascending
        // sweep (grays is sorted, so the best candidate index is monotonic).
        let mut lut = [0u8; 256];
        let mut gi = 0usize;
        for (v, slot) in lut.iter_mut().enumerate() {
            let v = v as i32;
            while gi + 1 < grays.len() {
                let cur = (grays[gi] as i32 - v).abs();
                let nxt = (grays[gi + 1] as i32 - v).abs();
                if nxt <= cur {
                    gi += 1;
                } else {
                    break;
                }
            }
            *slot = gi as u8;
        }

        Some(Self {
            palette,
            lut,
            transparent_index,
        })
    }

    /// The RGB color table bytes (for use as a local or global GIF color table).
    pub(crate) fn palette_bytes(&self) -> &[u8] {
        &self.palette
    }

    /// The reserved transparent slot index, if any.
    ///
    /// `None` means this palette has no transparent entry — the caller must not
    /// feed it frame-differenced pixels (which mark unchanged areas `a == 0`),
    /// because there is no index to map them to. Used by the encoder to disable
    /// frame differencing for a full 256-level gray palette.
    pub(crate) fn transparent_index(&self) -> Option<u8> {
        self.transparent_index
    }

    /// Remap a frame's pixels to palette indices.
    ///
    /// Exact for grayscale pixels (`r == g == b` maps to that level's index).
    /// Transparent pixels (`a == 0`) map to the reserved transparent slot when
    /// one exists. Any stray non-gray pixel maps to the nearest gray by its red
    /// channel — graceful, never out of range — though in practice this only
    /// fires for frames the builder already accepted as grayscale.
    pub(crate) fn remap(&self, pixels: &[Rgba]) -> QuantizedFrame {
        let mut indexed = Vec::with_capacity(pixels.len());
        match self.transparent_index {
            Some(ti) => {
                for &p in pixels {
                    if p.a == 0 {
                        indexed.push(ti);
                    } else {
                        indexed.push(self.lut[p.r as usize]);
                    }
                }
            }
            None => {
                for &p in pixels {
                    indexed.push(self.lut[p.r as usize]);
                }
            }
        }

        QuantizedFrame {
            palette: self.palette.clone(),
            pixels: indexed,
            transparent_index: self.transparent_index,
        }
    }
}

/// Per-frame grayscale fast path: build an exact palette for this single frame
/// and remap it, reserving a transparent slot only if the frame actually
/// contains transparent pixels.
///
/// Returns `None` when the frame is not grayscale, so the caller falls back to
/// the configured quantizer.
pub(crate) fn try_quantize_frame(pixels: &[Rgba]) -> Option<QuantizedFrame> {
    GrayPalette::try_build(&[pixels], false).map(|pal| pal.remap(pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(v: u8) -> Rgba {
        Rgba::rgb(v, v, v)
    }

    #[test]
    fn detects_and_quantizes_grayscale_exactly() {
        // A gradient using several distinct gray levels.
        let pixels: Vec<Rgba> = (0..64u16).map(|i| gray((i * 4) as u8)).collect();
        let qf = try_quantize_frame(&pixels).expect("grayscale should take the fast path");

        // Palette holds exactly the distinct gray levels (no transparent slot
        // since the frame is fully opaque), each as an [g, g, g] triple.
        assert_eq!(qf.palette.len() % 3, 0);
        let levels = qf.palette.len() / 3;
        assert_eq!(levels, 64);
        assert!(qf.transparent_index.is_none());

        // Every entry is a true gray, and round-tripping each pixel through its
        // index reproduces the original value exactly (lossless).
        for tri in qf.palette.as_chunks::<3>().0.iter() {
            assert_eq!(tri[0], tri[1]);
            assert_eq!(tri[1], tri[2]);
        }
        for (i, &idx) in qf.pixels.iter().enumerate() {
            let base = idx as usize * 3;
            assert_eq!(qf.palette[base], pixels[i].r, "pixel {i} mismatched");
        }
    }

    #[test]
    fn rejects_color_frames() {
        let mut pixels = vec![gray(10); 16];
        pixels[7] = Rgba::rgb(10, 20, 30); // one genuinely colored pixel
        assert!(
            try_quantize_frame(&pixels).is_none(),
            "a color pixel must force the general quantizer path"
        );
    }

    #[test]
    fn reserves_transparent_slot_when_pixels_are_transparent() {
        let mut pixels = vec![gray(128); 16];
        pixels[0] = Rgba::TRANSPARENT;
        let qf = try_quantize_frame(&pixels).expect("grayscale with transparency");

        let ti = qf
            .transparent_index
            .expect("a transparent slot is reserved");
        assert_eq!(
            qf.pixels[0], ti,
            "transparent pixel maps to the transparent slot"
        );
        // Opaque gray pixels do NOT use the transparent index.
        assert!(qf.pixels[1..].iter().all(|&idx| idx != ti));
    }

    #[test]
    fn full_range_single_frame_fits_without_transparency() {
        // All 256 gray levels, fully opaque: must fit exactly in 256 entries.
        let pixels: Vec<Rgba> = (0..256u16).map(|v| gray(v as u8)).collect();
        let pal = GrayPalette::try_build(&[&pixels], false).expect("256 grays fit");
        assert_eq!(pal.palette_bytes().len(), 256 * 3);
        assert!(pal.transparent_index.is_none());
    }

    #[test]
    fn full_range_with_forced_transparency_falls_back() {
        // 256 grays + a required transparent slot = 257 entries → no room.
        let pixels: Vec<Rgba> = (0..256u16).map(|v| gray(v as u8)).collect();
        assert!(
            GrayPalette::try_build(&[&pixels], true).is_none(),
            "no room for a transparent slot alongside all 256 gray levels"
        );
    }

    #[test]
    fn shared_palette_covers_multiple_frames() {
        let frame_a = vec![gray(0); 8];
        let frame_b = vec![gray(255); 8];
        let frame_c = vec![gray(128); 8];
        let refs: Vec<&[Rgba]> = vec![&frame_a, &frame_b, &frame_c];

        let pal = GrayPalette::try_build(&refs, true).expect("multi-frame grayscale");
        // 3 distinct grays + 1 reserved transparent slot.
        assert_eq!(pal.palette_bytes().len(), 4 * 3);
        assert!(pal.transparent_index.is_some());

        // Each frame remaps exactly against the shared palette.
        let qa = pal.remap(&frame_a);
        assert!(
            qa.pixels.iter().all(|&idx| {
                let base = idx as usize * 3;
                pal.palette_bytes()[base] == 0
            }),
            "frame A pixels map to gray 0"
        );
    }

    #[test]
    fn nearest_gray_lut_is_total() {
        // Sparse palette (only two levels): every possible value must map to a
        // valid index — the nearest of the two.
        let pixels = vec![gray(0), gray(200)];
        let pal = GrayPalette::try_build(&[&pixels], false).expect("two grays");
        // value 0 → level 0 (index 0); value 200 → level 200 (index 1);
        // value 90 is nearer 0, value 120 is nearer 200.
        assert_eq!(pal.lut[0], 0);
        assert_eq!(pal.lut[200], 1);
        assert_eq!(pal.lut[90], 0);
        assert_eq!(pal.lut[120], 1);
    }
}
