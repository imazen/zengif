//! Screen compositing for GIF animation.
//!
//! The screen maintains the canvas state and applies disposal methods
//! to produce correctly composited frames.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::disposal::Disposal;
use crate::error::Result;
use crate::limits::Limits;
use crate::stats::Stats;
use crate::types::{ComposedFrame, Palette, RawFrame, Rgba};

/// GIF compositing screen.
///
/// Maintains the canvas state and applies frames with proper
/// disposal method handling to produce composited output.
#[non_exhaustive]
pub struct Screen {
    /// Canvas width.
    width: u16,

    /// Canvas height.
    height: u16,

    /// Current canvas pixels (RGBA).
    pixels: Vec<Rgba>,

    /// Global palette (if any).
    global_palette: Option<Palette>,

    /// Background color.
    background: Rgba,

    /// Pending disposal from last frame.
    disposal: Disposal,

    /// Reference to stats for memory tracking.
    /// We store sizes but track via Stats passed to methods.
    canvas_bytes: usize,
}

impl Screen {
    /// Create a new screen with the given dimensions.
    pub fn new(
        width: u16,
        height: u16,
        global_palette: Option<Palette>,
        background_index: Option<u8>,
        stats: &Stats,
        limits: &Limits,
    ) -> Result<Self> {
        // Validate dimensions first
        limits.check_dimensions(width, height)?;

        let pixel_count = width as usize * height as usize;
        let canvas_bytes = pixel_count * core::mem::size_of::<Rgba>();

        // Track memory allocation
        stats.try_alloc(canvas_bytes, limits)?;

        // Determine background color
        let background = match (background_index, &global_palette) {
            (Some(idx), Some(palette)) => palette.get_or_transparent(idx),
            _ => Rgba::TRANSPARENT,
        };

        // Initialize canvas with background color (fallible)
        let mut pixels = Vec::new();
        pixels.try_reserve(pixel_count).map_err(|_| {
            stats.track_dealloc(canvas_bytes); // Undo tracking
            whereat::at!(crate::error::GifError::AllocationFailed {
                requested: canvas_bytes
            })
        })?;
        pixels.resize(pixel_count, background);

        Ok(Self {
            width,
            height,
            pixels,
            global_palette,
            background,
            disposal: Disposal::default(),
            canvas_bytes,
        })
    }

    /// Get canvas width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Get canvas height.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Get the current canvas pixels.
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }

    /// Get the global palette.
    pub fn global_palette(&self) -> Option<&Palette> {
        self.global_palette.as_ref()
    }

    /// Get the background color.
    pub fn background(&self) -> Rgba {
        self.background
    }

    /// Process a raw frame without returning composed pixels.
    ///
    /// Use `pixels()` to access the composed canvas after calling this.
    /// This avoids the memcpy overhead of `process_frame` for streaming
    /// callers who don't need to keep frames in memory.
    ///
    /// Returns frame metadata (index, delay) on success.
    pub fn process_frame_in_place(
        &mut self,
        frame: &RawFrame,
        stats: &Stats,
        limits: &Limits,
    ) -> Result<(usize, u16)> {
        // 1. Apply disposal from previous frame
        self.disposal
            .apply(&mut self.pixels, self.width, self.background, stats);

        // 2. Get the palette for this frame
        let palette = frame
            .palette
            .as_ref()
            .or(self.global_palette.as_ref())
            .map(|p| p.colors())
            .unwrap_or(&[]);

        // 3. Validate frame bounds (clip if necessary, but warn)
        let (left, top, width, height) =
            self.clip_frame_bounds(frame.left, frame.top, frame.width, frame.height);

        // 4. Set up disposal for this frame (before blitting)
        self.disposal = Disposal::new(
            frame.disposal,
            left,
            top,
            width,
            height,
            &self.pixels,
            self.width,
            stats,
            limits,
        )?;

        // 5. Blit the frame onto the canvas
        if width > 0 && height > 0 && !frame.pixels.is_empty() {
            // Pre-compute offsets
            let frame_x_offset = (left - frame.left) as usize;
            let frame_y_offset = (top - frame.top) as usize;
            let canvas_stride = self.width as usize;
            let frame_stride = frame.width as usize;
            let row_width = width as usize;

            // Choose fast path based on transparency
            if let Some(transparent_idx) = frame.transparent {
                // Slow path: need to check transparency for each pixel
                for y in 0..height as usize {
                    let canvas_row_start = (top as usize + y) * canvas_stride + left as usize;
                    let frame_row_start = (frame_y_offset + y) * frame_stride + frame_x_offset;

                    // Get row slices (bounds-check once per row)
                    let frame_row_end = (frame_row_start + row_width).min(frame.pixels.len());
                    if frame_row_start >= frame.pixels.len() {
                        continue;
                    }
                    let frame_row = &frame.pixels[frame_row_start..frame_row_end];
                    let canvas_row =
                        &mut self.pixels[canvas_row_start..canvas_row_start + frame_row.len()];

                    for (canvas_pixel, &color_index) in canvas_row.iter_mut().zip(frame_row.iter())
                    {
                        if color_index != transparent_idx {
                            *canvas_pixel = palette
                                .get(color_index as usize)
                                .copied()
                                .unwrap_or(Rgba::TRANSPARENT);
                        }
                    }
                }
            } else {
                // Fast path: no transparency check needed
                for y in 0..height as usize {
                    let canvas_row_start = (top as usize + y) * canvas_stride + left as usize;
                    let frame_row_start = (frame_y_offset + y) * frame_stride + frame_x_offset;

                    // Get row slices
                    let frame_row_end = (frame_row_start + row_width).min(frame.pixels.len());
                    if frame_row_start >= frame.pixels.len() {
                        continue;
                    }
                    let frame_row = &frame.pixels[frame_row_start..frame_row_end];
                    let canvas_row =
                        &mut self.pixels[canvas_row_start..canvas_row_start + frame_row.len()];

                    for (canvas_pixel, &color_index) in canvas_row.iter_mut().zip(frame_row.iter())
                    {
                        *canvas_pixel = palette
                            .get(color_index as usize)
                            .copied()
                            .unwrap_or(Rgba::TRANSPARENT);
                    }
                }
            }
        }

        Ok((frame.index, frame.delay))
    }

    /// Process a raw frame and return the composited result.
    ///
    /// This applies the pending disposal from the previous frame,
    /// blits the new frame, and sets up disposal for the next frame.
    ///
    /// Note: This copies the entire canvas. For streaming use cases
    /// where frames don't need to be kept, use `process_frame_in_place`
    /// followed by `pixels()` to avoid the copy.
    pub fn process_frame(
        &mut self,
        frame: &RawFrame,
        stats: &Stats,
        limits: &Limits,
    ) -> Result<ComposedFrame> {
        // Process frame in place (does all the compositing work)
        let (index, delay) = self.process_frame_in_place(frame, stats, limits)?;

        // Create the composed frame (copy of current canvas, fallible)
        let composed_bytes = self.pixels.len() * core::mem::size_of::<Rgba>();
        stats.try_alloc(composed_bytes, limits)?;

        let mut composed_pixels = Vec::new();
        composed_pixels
            .try_reserve(self.pixels.len())
            .map_err(|_| {
                stats.track_dealloc(composed_bytes); // Undo tracking
                whereat::at!(crate::error::GifError::AllocationFailed {
                    requested: composed_bytes
                })
            })?;
        composed_pixels.extend_from_slice(&self.pixels);

        // Get effective palette (local if present, else global)
        let effective_palette = frame
            .palette
            .as_ref()
            .or(self.global_palette.as_ref())
            .cloned();

        let composed = ComposedFrame {
            index,
            width: self.width,
            height: self.height,
            delay,
            pixels: composed_pixels,
            palette: effective_palette,
        };

        Ok(composed)
    }

    /// Clip frame bounds to canvas.
    ///
    /// Returns (left, top, width, height) clipped to canvas bounds.
    fn clip_frame_bounds(
        &self,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
    ) -> (u16, u16, u16, u16) {
        // Clip left edge
        let clipped_left = left.min(self.width);

        // Clip top edge
        let clipped_top = top.min(self.height);

        // Clip right edge (width)
        let max_width = self.width.saturating_sub(clipped_left);
        let clipped_width = width.min(max_width);

        // Clip bottom edge (height)
        let max_height = self.height.saturating_sub(clipped_top);
        let clipped_height = height.min(max_height);

        (clipped_left, clipped_top, clipped_width, clipped_height)
    }

    /// Reset the screen to initial state.
    pub fn reset(&mut self, _stats: &Stats) {
        // Fill with background (slice::fill is faster than per-pixel loop)
        self.pixels.fill(self.background);

        // Clear pending disposal
        self.disposal = Disposal::default();
    }

    /// Get memory usage of the screen.
    pub fn memory_usage(&self) -> usize {
        self.canvas_bytes + self.disposal.memory_usage()
    }

    /// Track deallocation when screen is dropped.
    pub fn dealloc(&self, stats: &Stats) {
        stats.track_dealloc(self.canvas_bytes);
    }
}

/// Builder for creating a Screen.
#[non_exhaustive]
pub struct ScreenBuilder {
    width: u16,
    height: u16,
    global_palette: Option<Palette>,
    background_index: Option<u8>,
}

impl ScreenBuilder {
    /// Create a new screen builder from a gif decoder.
    #[cfg(feature = "std")]
    pub(crate) fn from_decoder<R: std::io::Read>(decoder: &gif::Decoder<R>) -> Self {
        let global_palette = decoder.global_palette().map(Palette::from_rgb_bytes);

        Self {
            width: decoder.width(),
            height: decoder.height(),
            global_palette,
            background_index: decoder.bg_color().map(|c| c as u8),
        }
    }

    /// Create a new screen builder with explicit dimensions.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            global_palette: None,
            background_index: None,
        }
    }

    /// Set the global palette.
    pub fn global_palette(mut self, palette: Palette) -> Self {
        self.global_palette = Some(palette);
        self
    }

    /// Set the background color index.
    pub fn background_index(mut self, index: u8) -> Self {
        self.background_index = Some(index);
        self
    }

    /// Build the screen.
    pub fn build(self, stats: &Stats, limits: &Limits) -> Result<Screen> {
        Screen::new(
            self.width,
            self.height,
            self.global_palette,
            self.background_index,
            stats,
            limits,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DisposalMethod;

    fn make_palette() -> Palette {
        Palette::from_rgba(vec![
            Rgba::rgb(255, 0, 0),   // 0: red
            Rgba::rgb(0, 255, 0),   // 1: green
            Rgba::rgb(0, 0, 255),   // 2: blue
            Rgba::rgb(255, 255, 0), // 3: yellow
        ])
    }

    #[allow(clippy::too_many_arguments)]
    fn make_raw_frame(
        index: usize,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        pixels: Vec<u8>,
        disposal: DisposalMethod,
        transparent: Option<u8>,
    ) -> RawFrame {
        RawFrame {
            index,
            left,
            top,
            width,
            height,
            delay: 10,
            disposal,
            transparent,
            needs_user_input: false,
            interlaced: false,
            palette: None,
            pixels,
        }
    }

    #[test]
    fn screen_creation() {
        let stats = Stats::new();
        let limits = Limits::none();

        let screen = Screen::new(4, 4, Some(make_palette()), Some(0), &stats, &limits).unwrap();

        assert_eq!(screen.width(), 4);
        assert_eq!(screen.height(), 4);

        // Should be filled with background color (red, index 0)
        assert_eq!(screen.pixels()[0], Rgba::rgb(255, 0, 0));
    }

    #[test]
    fn process_simple_frame() {
        let stats = Stats::new();
        let limits = Limits::none();

        let mut screen = Screen::new(4, 4, Some(make_palette()), None, &stats, &limits).unwrap();

        // Frame that covers entire canvas
        let frame = make_raw_frame(
            0,
            0,
            0,
            4,
            4,
            vec![0; 16], // All red
            DisposalMethod::Keep,
            None,
        );

        let composed = screen.process_frame(&frame, &stats, &limits).unwrap();

        assert_eq!(composed.width, 4);
        assert_eq!(composed.height, 4);
        assert_eq!(composed.pixels.len(), 16);

        // All pixels should be red
        for pixel in &composed.pixels {
            assert_eq!(*pixel, Rgba::rgb(255, 0, 0));
        }
    }

    #[test]
    fn process_frame_with_transparency() {
        let stats = Stats::new();
        let limits = Limits::none();

        // Initialize with green background
        let mut screen = Screen::new(4, 4, Some(make_palette()), Some(1), &stats, &limits).unwrap();

        // Frame with some transparent pixels
        let frame = make_raw_frame(
            0,
            1,
            1,
            2,
            2,
            vec![0, 1, 1, 2], // red, transparent, transparent, blue
            DisposalMethod::Keep,
            Some(1), // green is transparent
        );

        let composed = screen.process_frame(&frame, &stats, &limits).unwrap();

        // Check specific pixels
        // (1,1) = red
        assert_eq!(composed.pixels[5], Rgba::rgb(255, 0, 0));
        // (2,1) = should be background (green) because index 1 is transparent
        assert_eq!(composed.pixels[6], Rgba::rgb(0, 255, 0));
        // (1,2) = should be background (green)
        assert_eq!(composed.pixels[9], Rgba::rgb(0, 255, 0));
        // (2,2) = blue
        assert_eq!(composed.pixels[10], Rgba::rgb(0, 0, 255));
    }

    #[test]
    fn disposal_background_sequence() {
        let stats = Stats::new();
        let limits = Limits::none();

        let mut screen = Screen::new(4, 4, Some(make_palette()), Some(1), &stats, &limits).unwrap();

        // First frame: red square with Background disposal
        let frame1 = make_raw_frame(
            0,
            1,
            1,
            2,
            2,
            vec![0, 0, 0, 0], // All red
            DisposalMethod::Background,
            None,
        );

        let _ = screen.process_frame(&frame1, &stats, &limits).unwrap();

        // Second frame: different position
        let frame2 = make_raw_frame(
            1,
            0,
            0,
            2,
            2,
            vec![2, 2, 2, 2], // All blue
            DisposalMethod::Keep,
            None,
        );

        let composed = screen.process_frame(&frame2, &stats, &limits).unwrap();

        // Frame1 covered (1,1)-(2,2) = indices 5,6,9,10
        // Frame2 covered (0,0)-(1,1) = indices 0,1,4,5
        // Overlap is at index 5 (position 1,1)

        // Index 6 (2,1): was red from frame1, restored to green by disposal, not touched by frame2
        assert_eq!(composed.pixels[6], Rgba::rgb(0, 255, 0));
        // Index 9 (1,2): was red from frame1, restored to green by disposal, not touched by frame2
        assert_eq!(composed.pixels[9], Rgba::rgb(0, 255, 0));

        // Index 5 (1,1): was red, restored to green, then overwritten by frame2's blue
        assert_eq!(composed.pixels[5], Rgba::rgb(0, 0, 255));

        // The area from frame2 should be blue
        assert_eq!(composed.pixels[0], Rgba::rgb(0, 0, 255));
        assert_eq!(composed.pixels[1], Rgba::rgb(0, 0, 255));
    }

    #[test]
    fn disposal_previous_sequence() {
        let stats = Stats::new();
        let limits = Limits::none();

        let mut screen = Screen::new(4, 4, Some(make_palette()), Some(1), &stats, &limits).unwrap();

        // First frame: blue in corner, Keep disposal
        let frame1 = make_raw_frame(
            0,
            0,
            0,
            2,
            2,
            vec![2, 2, 2, 2], // All blue
            DisposalMethod::Keep,
            None,
        );

        let _ = screen.process_frame(&frame1, &stats, &limits).unwrap();

        // Second frame: red overlay with Previous disposal
        let frame2 = make_raw_frame(
            1,
            1,
            1,
            2,
            2,
            vec![0, 0, 0, 0], // All red
            DisposalMethod::Previous,
            None,
        );

        let _ = screen.process_frame(&frame2, &stats, &limits).unwrap();

        // Third frame: should see the state before frame2
        let frame3 = make_raw_frame(
            2,
            3,
            3,
            1,
            1,
            vec![3], // Yellow
            DisposalMethod::Keep,
            None,
        );

        let composed = screen.process_frame(&frame3, &stats, &limits).unwrap();

        // Position (1,1) should be back to blue (from frame1), not red
        assert_eq!(composed.pixels[5], Rgba::rgb(0, 0, 255));
    }

    #[test]
    fn clip_oversized_frame() {
        let stats = Stats::new();
        let limits = Limits::none();

        let mut screen = Screen::new(4, 4, Some(make_palette()), None, &stats, &limits).unwrap();

        // Frame that extends beyond canvas
        let frame = make_raw_frame(
            0,
            2,
            2,
            4, // Would extend to x=6
            4, // Would extend to y=6
            vec![0; 16],
            DisposalMethod::Keep,
            None,
        );

        // Should not panic, just clip
        let result = screen.process_frame(&frame, &stats, &limits);
        assert!(result.is_ok());
    }
}
