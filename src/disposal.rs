//! GIF disposal method implementation.
//!
//! Disposal methods determine what happens to the canvas area covered by a frame
//! after that frame has been displayed and before the next frame is shown.

use crate::error::Result;
use crate::limits::Limits;
use crate::stats::Stats;
use crate::types::{DisposalMethod, Rgba};

/// Tracks disposal state for a frame region.
///
/// This captures the information needed to "undo" a frame's effect on the canvas
/// according to its disposal method.
#[derive(Debug)]
pub struct Disposal {
    /// The disposal method to apply.
    method: DisposalMethod,

    /// Saved pixels for Previous disposal (only allocated when needed).
    saved_pixels: Option<Vec<Rgba>>,

    /// Frame region bounds.
    left: u16,
    top: u16,
    width: u16,
    height: u16,
}

impl Default for Disposal {
    fn default() -> Self {
        Self {
            method: DisposalMethod::Keep,
            saved_pixels: None,
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl Disposal {
    /// Create a new disposal tracker for a frame.
    ///
    /// For `Previous` disposal, this saves the current canvas region
    /// so it can be restored later.
    pub fn new(
        method: DisposalMethod,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        canvas: &[Rgba],
        canvas_width: u16,
        stats: &Stats,
        limits: &Limits,
    ) -> Result<Self> {
        let saved_pixels = if method == DisposalMethod::Previous {
            // Save the current canvas region
            let region_size = width as usize * height as usize;
            let byte_size = region_size * core::mem::size_of::<Rgba>();

            // Check memory limit before allocating
            stats.try_alloc(byte_size, limits)?;

            let mut saved = Vec::with_capacity(region_size);

            // Extract the region from the canvas
            for y in 0..height as usize {
                let canvas_y = top as usize + y;
                let row_start = canvas_y * canvas_width as usize + left as usize;
                let row_end = row_start + width as usize;
                saved.extend_from_slice(&canvas[row_start..row_end]);
            }

            Some(saved)
        } else {
            None
        };

        Ok(Self {
            method,
            saved_pixels,
            left,
            top,
            width,
            height,
        })
    }

    /// Apply the disposal to restore the canvas.
    ///
    /// This should be called BEFORE blitting the next frame.
    pub fn apply(
        &mut self,
        canvas: &mut [Rgba],
        canvas_width: u16,
        background: Rgba,
        stats: &Stats,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }

        match self.method {
            DisposalMethod::Unspecified | DisposalMethod::Keep => {
                // Do nothing - keep the frame content
            }
            DisposalMethod::Background => {
                // Fill the region with background color
                for y in 0..self.height as usize {
                    let canvas_y = self.top as usize + y;
                    let row_start = canvas_y * canvas_width as usize + self.left as usize;
                    let row_end = row_start + self.width as usize;
                    for pixel in &mut canvas[row_start..row_end] {
                        *pixel = background;
                    }
                }
            }
            DisposalMethod::Previous => {
                // Restore the saved region
                if let Some(saved) = self.saved_pixels.take() {
                    let byte_size = saved.len() * core::mem::size_of::<Rgba>();

                    let mut src_idx = 0;
                    for y in 0..self.height as usize {
                        let canvas_y = self.top as usize + y;
                        let row_start = canvas_y * canvas_width as usize + self.left as usize;
                        let row_end = row_start + self.width as usize;
                        canvas[row_start..row_end]
                            .copy_from_slice(&saved[src_idx..src_idx + self.width as usize]);
                        src_idx += self.width as usize;
                    }

                    // Track deallocation when saved pixels are dropped
                    stats.track_dealloc(byte_size);
                }
            }
        }
    }

    /// Get the disposal method.
    #[allow(dead_code)]
    pub fn method(&self) -> DisposalMethod {
        self.method
    }

    /// Check if this disposal has saved pixels.
    #[allow(dead_code)]
    pub fn has_saved_pixels(&self) -> bool {
        self.saved_pixels.is_some()
    }

    /// Get memory usage of saved pixels.
    pub fn memory_usage(&self) -> usize {
        self.saved_pixels
            .as_ref()
            .map(|v| v.len() * core::mem::size_of::<Rgba>())
            .unwrap_or(0)
    }
}

/// Blit (copy) indexed frame pixels onto an RGBA canvas.
///
/// Handles transparency by skipping pixels with the transparent index.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn blit_indexed(
    canvas: &mut [Rgba],
    canvas_width: u16,
    frame_left: u16,
    frame_top: u16,
    frame_width: u16,
    frame_height: u16,
    frame_pixels: &[u8],
    palette: &[Rgba],
    transparent_index: Option<u8>,
) {
    for y in 0..frame_height as usize {
        let canvas_y = frame_top as usize + y;
        let canvas_row_start = canvas_y * canvas_width as usize;

        for x in 0..frame_width as usize {
            let frame_idx = y * frame_width as usize + x;
            let color_index = frame_pixels[frame_idx];

            // Skip transparent pixels
            if Some(color_index) == transparent_index {
                continue;
            }

            // Get color from palette
            let color = palette
                .get(color_index as usize)
                .copied()
                .unwrap_or(Rgba::TRANSPARENT);

            let canvas_x = frame_left as usize + x;
            let canvas_idx = canvas_row_start + canvas_x;
            canvas[canvas_idx] = color;
        }
    }
}

/// Blit RGBA frame pixels onto an RGBA canvas.
///
/// Handles transparency by alpha blending or skipping fully transparent pixels.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn blit_rgba(
    canvas: &mut [Rgba],
    canvas_width: u16,
    frame_left: u16,
    frame_top: u16,
    frame_width: u16,
    frame_height: u16,
    frame_pixels: &[Rgba],
    blend: bool,
) {
    for y in 0..frame_height as usize {
        let canvas_y = frame_top as usize + y;
        let canvas_row_start = canvas_y * canvas_width as usize;

        for x in 0..frame_width as usize {
            let frame_idx = y * frame_width as usize + x;
            let src = frame_pixels[frame_idx];

            // Skip fully transparent pixels
            if src.a == 0 {
                continue;
            }

            let canvas_x = frame_left as usize + x;
            let canvas_idx = canvas_row_start + canvas_x;

            if blend && src.a < 255 {
                // Alpha blend
                let dst = canvas[canvas_idx];
                canvas[canvas_idx] = alpha_blend(src, dst);
            } else {
                canvas[canvas_idx] = src;
            }
        }
    }
}

/// Perform alpha blending: src over dst.
#[inline]
#[allow(dead_code)]
fn alpha_blend(src: Rgba, dst: Rgba) -> Rgba {
    if src.a == 255 {
        return src;
    }
    if src.a == 0 {
        return dst;
    }

    let src_a = src.a as u32;
    let dst_a = dst.a as u32;
    let inv_src_a = 255 - src_a;

    // out_a = src_a + dst_a * (1 - src_a)
    let out_a = src_a + (dst_a * inv_src_a) / 255;

    if out_a == 0 {
        return Rgba::TRANSPARENT;
    }

    // out_c = (src_c * src_a + dst_c * dst_a * (1 - src_a)) / out_a
    let r = ((src.r as u32 * src_a + dst.r as u32 * dst_a * inv_src_a / 255) / out_a) as u8;
    let g = ((src.g as u32 * src_a + dst.g as u32 * dst_a * inv_src_a / 255) / out_a) as u8;
    let b = ((src.b as u32 * src_a + dst.b as u32 * dst_a * inv_src_a / 255) / out_a) as u8;

    Rgba::new(r, g, b, out_a as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;
    use crate::stats::Stats;

    fn make_canvas(width: u16, height: u16, fill: Rgba) -> Vec<Rgba> {
        vec![fill; width as usize * height as usize]
    }

    #[test]
    fn disposal_keep() {
        let stats = Stats::new();
        let limits = Limits::none();
        let mut canvas = make_canvas(4, 4, Rgba::WHITE);

        // Create disposal with Keep - should not save pixels
        let mut disposal = Disposal::new(
            DisposalMethod::Keep,
            1,
            1,
            2,
            2,
            &canvas,
            4,
            &stats,
            &limits,
        )
        .unwrap();

        assert!(!disposal.has_saved_pixels());

        // Modify canvas
        canvas[5] = Rgba::rgb(255, 0, 0);

        // Apply disposal - should do nothing
        disposal.apply(&mut canvas, 4, Rgba::BLACK, &stats);

        // Canvas should still have the modification
        assert_eq!(canvas[5], Rgba::rgb(255, 0, 0));
    }

    #[test]
    fn disposal_background() {
        let stats = Stats::new();
        let limits = Limits::none();
        let mut canvas = make_canvas(4, 4, Rgba::WHITE);

        let mut disposal = Disposal::new(
            DisposalMethod::Background,
            1,
            1,
            2,
            2,
            &canvas,
            4,
            &stats,
            &limits,
        )
        .unwrap();

        // Apply disposal - should fill region with background
        let background = Rgba::rgb(0, 255, 0);
        disposal.apply(&mut canvas, 4, background, &stats);

        // Check that only the region was filled
        assert_eq!(canvas[0], Rgba::WHITE); // (0,0) outside
        assert_eq!(canvas[5], background); // (1,1) inside
        assert_eq!(canvas[6], background); // (2,1) inside
        assert_eq!(canvas[9], background); // (1,2) inside
        assert_eq!(canvas[10], background); // (2,2) inside
        assert_eq!(canvas[15], Rgba::WHITE); // (3,3) outside
    }

    #[test]
    fn disposal_previous() {
        let stats = Stats::new();
        let limits = Limits::none();
        let mut canvas = make_canvas(4, 4, Rgba::WHITE);

        // Save the original state
        let mut disposal = Disposal::new(
            DisposalMethod::Previous,
            1,
            1,
            2,
            2,
            &canvas,
            4,
            &stats,
            &limits,
        )
        .unwrap();

        assert!(disposal.has_saved_pixels());

        // Modify canvas
        canvas[5] = Rgba::rgb(255, 0, 0);
        canvas[6] = Rgba::rgb(0, 255, 0);
        canvas[9] = Rgba::rgb(0, 0, 255);
        canvas[10] = Rgba::rgb(255, 255, 0);

        // Apply disposal - should restore original
        disposal.apply(&mut canvas, 4, Rgba::BLACK, &stats);

        // Region should be restored to white
        assert_eq!(canvas[5], Rgba::WHITE);
        assert_eq!(canvas[6], Rgba::WHITE);
        assert_eq!(canvas[9], Rgba::WHITE);
        assert_eq!(canvas[10], Rgba::WHITE);
    }

    #[test]
    fn blit_indexed_with_transparency() {
        let mut canvas = make_canvas(4, 4, Rgba::WHITE);

        let palette = vec![
            Rgba::rgb(255, 0, 0), // 0: red
            Rgba::rgb(0, 255, 0), // 1: green
            Rgba::rgb(0, 0, 255), // 2: blue
        ];

        let frame_pixels = vec![0, 1, 2, 1]; // 2x2 frame

        blit_indexed(
            &mut canvas,
            4,
            1,
            1,
            2,
            2,
            &frame_pixels,
            &palette,
            Some(1), // green is transparent
        );

        // Check results
        assert_eq!(canvas[5], Rgba::rgb(255, 0, 0)); // red
        assert_eq!(canvas[6], Rgba::WHITE); // skipped (transparent)
        assert_eq!(canvas[9], Rgba::rgb(0, 0, 255)); // blue
        assert_eq!(canvas[10], Rgba::WHITE); // skipped (transparent)
    }

    #[test]
    fn alpha_blend_opaque() {
        let src = Rgba::rgb(255, 0, 0);
        let dst = Rgba::rgb(0, 255, 0);
        let result = alpha_blend(src, dst);
        assert_eq!(result, src); // Fully opaque source wins
    }

    #[test]
    fn alpha_blend_transparent() {
        let src = Rgba::TRANSPARENT;
        let dst = Rgba::rgb(0, 255, 0);
        let result = alpha_blend(src, dst);
        assert_eq!(result, dst); // Fully transparent source shows dest
    }

    #[test]
    fn alpha_blend_partial() {
        let src = Rgba::new(255, 0, 0, 128); // 50% red
        let dst = Rgba::rgb(0, 255, 0); // green
        let result = alpha_blend(src, dst);

        // Should be somewhere between red and green
        assert!(result.r > 100);
        assert!(result.g > 100);
        assert_eq!(result.b, 0);
        assert!(result.a > 200); // High alpha
    }

    #[test]
    fn disposal_memory_tracking() {
        let stats = Stats::new();
        let limits = Limits::none();
        let canvas = make_canvas(4, 4, Rgba::WHITE);

        // Previous disposal should allocate
        let disposal = Disposal::new(
            DisposalMethod::Previous,
            1,
            1,
            2,
            2,
            &canvas,
            4,
            &stats,
            &limits,
        )
        .unwrap();

        let expected_bytes = 2 * 2 * core::mem::size_of::<Rgba>();
        assert_eq!(stats.current(), expected_bytes);
        assert_eq!(disposal.memory_usage(), expected_bytes);

        // Drop should deallocate
        drop(disposal);
        // Note: we don't track dealloc on drop of Disposal itself,
        // only when apply() is called. This is intentional for now.
    }
}
