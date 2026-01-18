//! Core types for GIF processing.

use core::fmt;

/// RGBA pixel (4 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Rgba {
    /// Red component.
    pub r: u8,
    /// Green component.
    pub g: u8,
    /// Blue component.
    pub b: u8,
    /// Alpha component (255 = opaque, 0 = transparent).
    pub a: u8,
}

impl Rgba {
    /// Fully transparent pixel.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// Fully opaque black pixel.
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    /// Fully opaque white pixel.
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    /// Create a new RGBA pixel.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Create an opaque RGB pixel.
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Check if pixel is fully transparent.
    #[inline]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }

    /// Check if pixel is fully opaque.
    #[inline]
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// Convert from byte slice (must be exactly 4 bytes).
    #[inline]
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        Self {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
            a: bytes[3],
        }
    }

    /// Convert to byte array.
    #[inline]
    pub fn to_bytes(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

// RGB crate interop
#[cfg(feature = "rgb-interop")]
impl From<rgb::RGBA8> for Rgba {
    #[inline]
    fn from(c: rgb::RGBA8) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }
}

#[cfg(feature = "rgb-interop")]
impl From<Rgba> for rgb::RGBA8 {
    #[inline]
    fn from(c: Rgba) -> Self {
        rgb::RGBA8 {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }
}

#[cfg(feature = "rgb-interop")]
impl From<rgb::RGB8> for Rgba {
    #[inline]
    fn from(c: rgb::RGB8) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: 255,
        }
    }
}

#[cfg(feature = "rgb-interop")]
impl From<Rgba> for rgb::RGB8 {
    /// Note: Alpha channel is discarded.
    #[inline]
    fn from(c: Rgba) -> Self {
        rgb::RGB8 {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

/// GIF disposal method.
///
/// Determines what happens to the canvas area after displaying a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
#[repr(u8)]
pub enum DisposalMethod {
    /// No disposal specified (treat as Keep).
    #[default]
    Unspecified = 0,

    /// Keep the frame on the canvas (do not dispose).
    Keep = 1,

    /// Restore the canvas area to the background color.
    Background = 2,

    /// Restore the canvas area to the previous frame's content.
    Previous = 3,
}

impl DisposalMethod {
    /// Parse from raw GIF disposal value (0-7).
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Unspecified,
            1 => Self::Keep,
            2 => Self::Background,
            3 => Self::Previous,
            // Values 4-7 are reserved; treat as Keep per spec
            _ => Self::Keep,
        }
    }

    /// Convert to raw GIF value.
    pub fn to_raw(self) -> u8 {
        self as u8
    }
}

impl From<gif::DisposalMethod> for DisposalMethod {
    fn from(d: gif::DisposalMethod) -> Self {
        match d {
            gif::DisposalMethod::Any => Self::Unspecified,
            gif::DisposalMethod::Keep => Self::Keep,
            gif::DisposalMethod::Background => Self::Background,
            gif::DisposalMethod::Previous => Self::Previous,
        }
    }
}

// Note: We intentionally do NOT implement From<DisposalMethod> for gif::DisposalMethod
// to avoid leaking the gif crate dependency in our public API.

/// Loop/repeat behavior for animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Repeat {
    /// Play once, do not loop.
    Once,

    /// Loop forever.
    #[default]
    Infinite,

    /// Loop a specific number of times.
    Count(u16),
}

impl From<gif::Repeat> for Repeat {
    fn from(r: gif::Repeat) -> Self {
        match r {
            gif::Repeat::Finite(0) => Self::Once,
            gif::Repeat::Finite(n) => Self::Count(n),
            gif::Repeat::Infinite => Self::Infinite,
        }
    }
}

// Note: We intentionally do NOT implement From<Repeat> for gif::Repeat
// to avoid leaking the gif crate dependency in our public API.

/// A color palette (up to 256 colors).
#[derive(Clone)]
pub struct Palette {
    /// Colors in RGB format (3 bytes each).
    colors: Vec<Rgba>,
}

impl Palette {
    /// Create a new palette from RGB byte slice.
    ///
    /// The slice must have a length divisible by 3.
    pub fn from_rgb_bytes(bytes: &[u8]) -> Self {
        let colors = bytes
            .chunks_exact(3)
            .map(|c| Rgba::rgb(c[0], c[1], c[2]))
            .collect();
        Self { colors }
    }

    /// Create a new palette from RGBA colors.
    pub fn from_rgba(colors: Vec<Rgba>) -> Self {
        Self { colors }
    }

    /// Create an empty palette.
    pub fn empty() -> Self {
        Self { colors: Vec::new() }
    }

    /// Get the number of colors.
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// Check if the palette is empty.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// Get a color by index.
    pub fn get(&self, index: u8) -> Option<Rgba> {
        self.colors.get(index as usize).copied()
    }

    /// Get a color by index, with transparent fallback.
    pub fn get_or_transparent(&self, index: u8) -> Rgba {
        self.get(index).unwrap_or(Rgba::TRANSPARENT)
    }

    /// Get all colors as a slice.
    pub fn colors(&self) -> &[Rgba] {
        &self.colors
    }

    /// Convert to RGB bytes (for gif crate).
    pub fn to_rgb_bytes(&self) -> Vec<u8> {
        self.colors.iter().flat_map(|c| [c.r, c.g, c.b]).collect()
    }

    /// Find the nearest palette color index for an RGBA color.
    ///
    /// Uses squared Euclidean distance in RGB space. For transparent pixels,
    /// returns the transparent index if one exists in the palette.
    pub fn find_nearest(&self, color: Rgba) -> u8 {
        if self.colors.is_empty() {
            return 0;
        }

        // For transparent pixels, find a transparent palette entry if available
        if color.a < 128 {
            if let Some(idx) = self.find_transparent_index() {
                return idx;
            }
        }

        // Find nearest by RGB distance
        let mut best_idx = 0u8;
        let mut best_dist = u32::MAX;

        for (idx, pc) in self.colors.iter().enumerate() {
            let dr = (color.r as i32 - pc.r as i32).unsigned_abs();
            let dg = (color.g as i32 - pc.g as i32).unsigned_abs();
            let db = (color.b as i32 - pc.b as i32).unsigned_abs();
            let dist = dr * dr + dg * dg + db * db;

            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                if dist == 0 {
                    break; // Exact match
                }
            }
        }

        best_idx
    }

    /// Find the index of the most transparent color in the palette.
    ///
    /// Returns None if no color has alpha < 128.
    pub fn find_transparent_index(&self) -> Option<u8> {
        self.colors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.a < 128)
            .max_by_key(|(_, c)| 255 - c.a)
            .map(|(i, _)| i as u8)
    }

    /// Map RGBA pixels to palette indices.
    ///
    /// Returns (indexed_pixels, transparent_index).
    pub fn map_pixels(&self, pixels: &[Rgba]) -> (Vec<u8>, Option<u8>) {
        let transparent_index = self.find_transparent_index();
        let indexed: Vec<u8> = pixels.iter().map(|p| self.find_nearest(*p)).collect();
        (indexed, transparent_index)
    }
}

impl fmt::Debug for Palette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Palette")
            .field("len", &self.colors.len())
            .finish()
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::empty()
    }
}

/// Raw frame data as read from GIF (before compositing).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RawFrame {
    /// Frame index (0-based).
    pub index: usize,

    /// Left position on canvas.
    pub left: u16,

    /// Top position on canvas.
    pub top: u16,

    /// Frame width.
    pub width: u16,

    /// Frame height.
    pub height: u16,

    /// Delay in centiseconds (1/100th of a second).
    pub delay: u16,

    /// Disposal method for this frame.
    pub disposal: DisposalMethod,

    /// Transparent color index (if any).
    pub transparent: Option<u8>,

    /// Whether user input is required before continuing.
    pub needs_user_input: bool,

    /// Interlaced encoding.
    pub interlaced: bool,

    /// Local palette (if present, overrides global).
    pub palette: Option<Palette>,

    /// Indexed pixel data (one byte per pixel, referencing palette).
    pub pixels: Vec<u8>,
}

impl RawFrame {
    /// Get the pixel buffer size.
    pub fn buffer_size(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// Composited frame with RGBA pixels.
///
/// This is the result of applying disposal methods and transparency
/// to produce the actual visible frame.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ComposedFrame {
    /// Frame index (0-based).
    pub index: usize,

    /// Canvas width.
    pub width: u16,

    /// Canvas height.
    pub height: u16,

    /// Delay in centiseconds (1/100th of a second).
    pub delay: u16,

    /// RGBA pixel data (4 bytes per pixel).
    pub pixels: Vec<Rgba>,

    /// The effective palette used for this frame.
    ///
    /// This is either the frame's local palette (if present) or the global palette.
    /// Useful for pass-through encoding where you want to preserve the original
    /// palette after processing (e.g., resizing) the RGBA pixels.
    pub palette: Option<Palette>,
}

impl ComposedFrame {
    /// Create a new composed frame.
    pub fn new(index: usize, width: u16, height: u16, delay: u16, pixels: Vec<Rgba>) -> Self {
        Self {
            index,
            width,
            height,
            delay,
            pixels,
            palette: None,
        }
    }

    /// Create a new composed frame with an explicit palette.
    ///
    /// Use this when you want to preserve the original palette for pass-through encoding.
    pub fn with_palette(
        index: usize,
        width: u16,
        height: u16,
        delay: u16,
        pixels: Vec<Rgba>,
        palette: Palette,
    ) -> Self {
        Self {
            index,
            width,
            height,
            delay,
            pixels,
            palette: Some(palette),
        }
    }

    /// Get the expected pixel count.
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Get a pixel at (x, y).
    pub fn get_pixel(&self, x: u16, y: u16) -> Option<Rgba> {
        if x < self.width && y < self.height {
            let idx = y as usize * self.width as usize + x as usize;
            self.pixels.get(idx).copied()
        } else {
            None
        }
    }

    /// Get the pixel data as raw RGBA bytes.
    pub fn as_bytes(&self) -> &[u8] {
        // Safety: Rgba is repr(C) with 4 u8 fields
        unsafe {
            core::slice::from_raw_parts(self.pixels.as_ptr() as *const u8, self.pixels.len() * 4)
        }
    }

    /// Convert to imgref `ImgVec<RGBA8>`.
    #[cfg(feature = "imgref-interop")]
    pub fn into_imgvec(self) -> imgref::ImgVec<rgb::RGBA8> {
        let rgba8_pixels: Vec<rgb::RGBA8> = self
            .pixels
            .into_iter()
            .map(|p| rgb::RGBA8::new(p.r, p.g, p.b, p.a))
            .collect();
        imgref::ImgVec::new(rgba8_pixels, self.width as usize, self.height as usize)
    }

    /// Get an imgref ImgRef view of the pixels.
    ///
    /// Note: This requires the pixel data to be reinterpreted as RGBA8.
    /// Since Rgba and rgb::RGBA8 have the same memory layout (repr(C)),
    /// this is safe.
    #[cfg(feature = "imgref-interop")]
    pub fn as_imgref(&self) -> imgref::ImgRef<'_, rgb::RGBA8> {
        // Safety: Rgba and rgb::RGBA8 have identical repr(C) layout
        let rgba8_slice: &[rgb::RGBA8] = unsafe {
            core::slice::from_raw_parts(
                self.pixels.as_ptr() as *const rgb::RGBA8,
                self.pixels.len(),
            )
        };
        imgref::ImgRef::new(rgba8_slice, self.width as usize, self.height as usize)
    }
}

/// Create a ComposedFrame from imgref ImgVec.
#[cfg(feature = "imgref-interop")]
impl From<(usize, u16, imgref::ImgVec<rgb::RGBA8>)> for ComposedFrame {
    fn from((index, delay, img): (usize, u16, imgref::ImgVec<rgb::RGBA8>)) -> Self {
        let width = img.width() as u16;
        let height = img.height() as u16;
        let pixels: Vec<Rgba> = img
            .into_buf()
            .into_iter()
            .map(|p| Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        Self {
            index,
            width,
            height,
            delay,
            pixels,
            palette: None,
        }
    }
}

/// Metadata about a GIF file.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Metadata {
    /// Canvas width.
    pub width: u16,

    /// Canvas height.
    pub height: u16,

    /// Global color palette (if present).
    pub global_palette: Option<Palette>,

    /// Background color index (if global palette exists).
    pub background_color_index: Option<u8>,

    /// Repeat/loop behavior.
    pub repeat: Repeat,

    /// Total number of frames.
    pub frame_count: usize,

    /// Comments embedded in the GIF.
    pub comments: Vec<String>,
}

impl Metadata {
    /// Get the background color.
    pub fn background_color(&self) -> Rgba {
        match (self.background_color_index, &self.global_palette) {
            (Some(idx), Some(palette)) => palette.get_or_transparent(idx),
            _ => Rgba::TRANSPARENT,
        }
    }

    /// Calculate total animation duration in centiseconds.
    pub fn total_duration_centiseconds(&self, frames: &[impl AsRef<RawFrame>]) -> u64 {
        frames.iter().map(|f| f.as_ref().delay as u64).sum()
    }
}

/// Frame input for encoding.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FrameInput {
    /// Frame delay in centiseconds.
    pub delay: u16,

    /// RGBA pixel data.
    pub pixels: Vec<Rgba>,

    /// Frame width (must match encoder canvas).
    pub width: u16,

    /// Frame height (must match encoder canvas).
    pub height: u16,

    /// Optional palette for this frame.
    ///
    /// When provided, RGBA pixels are mapped to the nearest palette color
    /// instead of being quantized. This is useful for:
    /// - Pass-through resizing (preserving original palettes after resize)
    /// - Round-trip encoding with known palettes
    /// - Avoiding quantization overhead when palette is already known
    ///
    /// If None, the encoder will quantize the frame (or use global palette).
    pub palette: Option<Palette>,
}

impl FrameInput {
    /// Create a new frame input.
    pub fn new(width: u16, height: u16, delay: u16, pixels: Vec<Rgba>) -> Self {
        Self {
            delay,
            pixels,
            width,
            height,
            palette: None,
        }
    }

    /// Create a new frame input with a specific palette.
    ///
    /// When a palette is provided, RGBA pixels are mapped to the nearest
    /// palette color instead of being quantized. This is useful for
    /// pass-through encoding where you want to preserve the original palette.
    pub fn with_palette(
        width: u16,
        height: u16,
        delay: u16,
        pixels: Vec<Rgba>,
        palette: Palette,
    ) -> Self {
        Self {
            delay,
            pixels,
            width,
            height,
            palette: Some(palette),
        }
    }

    /// Create from raw RGBA bytes.
    pub fn from_bytes(width: u16, height: u16, delay: u16, bytes: &[u8]) -> Self {
        let pixels = bytes
            .chunks_exact(4)
            .map(|c| Rgba::new(c[0], c[1], c[2], c[3]))
            .collect();
        Self {
            delay,
            pixels,
            width,
            height,
            palette: None,
        }
    }

    /// Create from imgref `ImgVec<RGBA8>`.
    #[cfg(feature = "imgref-interop")]
    pub fn from_imgvec(delay: u16, img: imgref::ImgVec<rgb::RGBA8>) -> Self {
        let width = img.width() as u16;
        let height = img.height() as u16;
        let pixels: Vec<Rgba> = img
            .into_buf()
            .into_iter()
            .map(|p| Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        Self {
            delay,
            pixels,
            width,
            height,
            palette: None,
        }
    }

    /// Create from imgref `ImgRef<RGBA8>` (copies pixels).
    #[cfg(feature = "imgref-interop")]
    pub fn from_imgref(delay: u16, img: imgref::ImgRef<'_, rgb::RGBA8>) -> Self {
        let width = img.width() as u16;
        let height = img.height() as u16;
        let pixels: Vec<Rgba> = img
            .buf()
            .iter()
            .map(|p| Rgba::new(p.r, p.g, p.b, p.a))
            .collect();
        Self {
            delay,
            pixels,
            width,
            height,
            palette: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_basics() {
        let pixel = Rgba::rgb(255, 128, 64);
        assert!(pixel.is_opaque());
        assert!(!pixel.is_transparent());

        let transparent = Rgba::TRANSPARENT;
        assert!(transparent.is_transparent());
        assert!(!transparent.is_opaque());
    }

    #[test]
    fn disposal_from_raw() {
        assert_eq!(DisposalMethod::from_raw(0), DisposalMethod::Unspecified);
        assert_eq!(DisposalMethod::from_raw(1), DisposalMethod::Keep);
        assert_eq!(DisposalMethod::from_raw(2), DisposalMethod::Background);
        assert_eq!(DisposalMethod::from_raw(3), DisposalMethod::Previous);
        assert_eq!(DisposalMethod::from_raw(7), DisposalMethod::Keep);
    }

    #[test]
    fn palette_from_rgb() {
        let bytes = [255, 0, 0, 0, 255, 0, 0, 0, 255];
        let palette = Palette::from_rgb_bytes(&bytes);
        assert_eq!(palette.len(), 3);
        assert_eq!(palette.get(0), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(palette.get(1), Some(Rgba::rgb(0, 255, 0)));
        assert_eq!(palette.get(2), Some(Rgba::rgb(0, 0, 255)));
    }

    #[test]
    fn repeat_conversion() {
        assert_eq!(Repeat::from(gif::Repeat::Infinite), Repeat::Infinite);
        assert_eq!(Repeat::from(gif::Repeat::Finite(0)), Repeat::Once);
        assert_eq!(Repeat::from(gif::Repeat::Finite(5)), Repeat::Count(5));
    }

    #[test]
    fn composed_frame_get_pixel() {
        let pixels = vec![
            Rgba::rgb(255, 0, 0),
            Rgba::rgb(0, 255, 0),
            Rgba::rgb(0, 0, 255),
            Rgba::rgb(255, 255, 255),
        ];
        let frame = ComposedFrame::new(0, 2, 2, 10, pixels);

        assert_eq!(frame.get_pixel(0, 0), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(frame.get_pixel(1, 0), Some(Rgba::rgb(0, 255, 0)));
        assert_eq!(frame.get_pixel(0, 1), Some(Rgba::rgb(0, 0, 255)));
        assert_eq!(frame.get_pixel(1, 1), Some(Rgba::rgb(255, 255, 255)));
        assert_eq!(frame.get_pixel(2, 2), None);
    }
}
