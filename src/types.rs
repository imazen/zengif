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

/// GIF disposal method.
///
/// Determines what happens to the canvas area after displaying a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

impl From<DisposalMethod> for gif::DisposalMethod {
    fn from(d: DisposalMethod) -> Self {
        match d {
            DisposalMethod::Unspecified => gif::DisposalMethod::Any,
            DisposalMethod::Keep => gif::DisposalMethod::Keep,
            DisposalMethod::Background => gif::DisposalMethod::Background,
            DisposalMethod::Previous => gif::DisposalMethod::Previous,
        }
    }
}

/// Loop/repeat behavior for animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

impl From<Repeat> for gif::Repeat {
    fn from(r: Repeat) -> Self {
        match r {
            Repeat::Once => gif::Repeat::Finite(0),
            Repeat::Infinite => gif::Repeat::Infinite,
            Repeat::Count(n) => gif::Repeat::Finite(n),
        }
    }
}

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
}

/// Metadata about a GIF file.
#[derive(Debug, Clone, Default)]
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
pub struct FrameInput {
    /// Frame delay in centiseconds.
    pub delay: u16,

    /// RGBA pixel data.
    pub pixels: Vec<Rgba>,

    /// Frame width (must match encoder canvas).
    pub width: u16,

    /// Frame height (must match encoder canvas).
    pub height: u16,
}

impl FrameInput {
    /// Create a new frame input.
    pub fn new(width: u16, height: u16, delay: u16, pixels: Vec<Rgba>) -> Self {
        Self {
            delay,
            pixels,
            width,
            height,
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
