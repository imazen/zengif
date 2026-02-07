//! Palette strategies and frame differencing logic.

use crate::types::Rgba;

/// Strategy for palette selection during encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PaletteStrategy {
    /// Each frame gets its own optimal 256-color palette.
    ///
    /// Pros: Best color accuracy per frame.
    /// Cons: Can cause flickering between frames, larger file size.
    #[default]
    PerFrame,

    /// Compute a single shared palette from all frames.
    ///
    /// Pros: No flickering, better LZW compression.
    /// Cons: May lose some color accuracy, requires pre-collecting all frames.
    ///
    /// Use `encode_gif_shared_palette` for this strategy.
    Shared,

    /// Use the provided global palette without re-quantizing.
    ///
    /// Best for round-trip encoding when preserving the original palette.
    /// Falls back to PerFrame if no global palette is set.
    Global,
}

/// Result of frame differencing analysis.
#[derive(Debug, Clone)]
pub(super) struct DiffResult {
    /// Left offset of the changed region.
    pub(super) left: u16,
    /// Top offset of the changed region.
    pub(super) top: u16,
    /// Width of the changed region.
    pub(super) width: u16,
    /// Height of the changed region.
    pub(super) height: u16,
    /// Pixels for the changed region with unchanged pixels marked transparent.
    pub(super) pixels: Vec<Rgba>,
}

/// Reusable scratch buffer for frame operations.
/// This avoids repeated allocations during encoding.
#[derive(Debug, Default)]
pub(super) struct ScratchBuffer {
    /// Buffer for diff pixels - reused across frames
    pub(super) diff_pixels: Vec<Rgba>,
    /// Buffer for frame pixels when cloning is needed
    pub(super) frame_pixels: Vec<Rgba>,
}

/// Check if two pixels are similar within a tolerance.
/// Returns true if all RGBA channels differ by at most `tolerance`.
#[inline(always)]
fn pixels_similar(a: Rgba, b: Rgba, tolerance: u8) -> bool {
    if tolerance == 0 {
        return a == b;
    }
    let dr = (a.r as i16 - b.r as i16).unsigned_abs() as u8;
    let dg = (a.g as i16 - b.g as i16).unsigned_abs() as u8;
    let db = (a.b as i16 - b.b as i16).unsigned_abs() as u8;
    let da = (a.a as i16 - b.a as i16).unsigned_abs() as u8;
    dr <= tolerance && dg <= tolerance && db <= tolerance && da <= tolerance
}

/// Compare current frame to previous and find the minimal changed region.
///
/// Returns None if the entire frame has changed (no optimization possible).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn compute_frame_diff(
    current: &[Rgba],
    previous: &[Rgba],
    width: u16,
    height: u16,
) -> Option<DiffResult> {
    let w = width as usize;
    let h = height as usize;

    // Find bounding box of changed pixels
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if current[idx] != previous[idx] {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    // No changes at all - shouldn't happen in practice but handle gracefully
    if min_x > max_x || min_y > max_y {
        // Emit a 1x1 transparent frame at origin
        return Some(DiffResult {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            pixels: vec![Rgba::TRANSPARENT],
        });
    }

    let diff_width = (max_x - min_x + 1) as u16;
    let diff_height = (max_y - min_y + 1) as u16;

    // If the changed region is the entire frame, no optimization benefit
    if diff_width == width && diff_height == height {
        return None;
    }

    // Extract the changed region, marking unchanged pixels as transparent
    let mut pixels = Vec::with_capacity(diff_width as usize * diff_height as usize);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let idx = y * w + x;
            if current[idx] == previous[idx] {
                // Unchanged pixel - mark transparent
                pixels.push(Rgba::TRANSPARENT);
            } else {
                // Changed pixel - keep as is
                pixels.push(current[idx]);
            }
        }
    }

    Some(DiffResult {
        left: min_x as u16,
        top: min_y as u16,
        width: diff_width,
        height: diff_height,
        pixels,
    })
}

/// Compare current frame to previous and find the minimal changed region.
/// Uses a scratch buffer to avoid allocations.
///
/// Returns None if the entire frame has changed (no optimization possible).
/// Compute RGB RMSE between original RGBA pixels and palette-mapped output.
///
/// Skips fully transparent pixels (alpha == 0) since they're invisible.
/// Returns RMSE in 0-255 RGB space (0 = perfect, ~5 = invisible, ~20 = visible).
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
pub(super) fn compute_remap_rmse(original: &[Rgba], indices: &[u8], palette_rgb: &[u8]) -> f32 {
    let mut total = 0u64;
    let mut count = 0u64;
    for (orig, &idx) in original.iter().zip(indices.iter()) {
        // Skip transparent pixels — they're invisible
        if orig.a == 0 {
            continue;
        }
        let base = idx as usize * 3;
        if base + 2 >= palette_rgb.len() {
            continue;
        }
        let dr = orig.r as i64 - palette_rgb[base] as i64;
        let dg = orig.g as i64 - palette_rgb[base + 1] as i64;
        let db = orig.b as i64 - palette_rgb[base + 2] as i64;
        total += (dr * dr + dg * dg + db * db) as u64;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((total as f64) / (count as f64)).sqrt() as f32
}

pub(super) fn compute_frame_diff_pooled(
    current: &[Rgba],
    previous: &[Rgba],
    width: u16,
    height: u16,
    tolerance: u8,
    scratch: &mut ScratchBuffer,
) -> Option<DiffResult> {
    let w = width as usize;
    let h = height as usize;

    // Find bounding box of changed pixels
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !pixels_similar(current[idx], previous[idx], tolerance) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    // No changes at all - shouldn't happen in practice but handle gracefully
    if min_x > max_x || min_y > max_y {
        // Emit a 1x1 transparent frame at origin
        scratch.diff_pixels.clear();
        scratch.diff_pixels.push(Rgba::TRANSPARENT);
        return Some(DiffResult {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            pixels: core::mem::take(&mut scratch.diff_pixels),
        });
    }

    let diff_width = (max_x - min_x + 1) as u16;
    let diff_height = (max_y - min_y + 1) as u16;

    // If the changed region is the entire frame, no optimization benefit
    if diff_width == width && diff_height == height {
        return None;
    }

    // Extract the changed region, marking unchanged pixels as transparent
    // Reuse the scratch buffer
    scratch.diff_pixels.clear();
    let region_size = diff_width as usize * diff_height as usize;
    if scratch.diff_pixels.capacity() < region_size {
        scratch
            .diff_pixels
            .reserve(region_size - scratch.diff_pixels.capacity());
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let idx = y * w + x;
            if pixels_similar(current[idx], previous[idx], tolerance) {
                // Unchanged pixel (within tolerance) - mark transparent
                scratch.diff_pixels.push(Rgba::TRANSPARENT);
            } else {
                // Changed pixel - keep as is
                scratch.diff_pixels.push(current[idx]);
            }
        }
    }

    // Take ownership of the buffer (will be returned to scratch on next call)
    Some(DiffResult {
        left: min_x as u16,
        top: min_y as u16,
        width: diff_width,
        height: diff_height,
        pixels: core::mem::take(&mut scratch.diff_pixels),
    })
}
