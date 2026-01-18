//! GIF streaming decoder.
//!
//! Provides a streaming decoder that produces composited RGBA frames
//! with proper disposal method handling.

use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use enough::Stop;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::screen::{Screen, ScreenBuilder};
use crate::stats::Stats;
use crate::types::{ComposedFrame, DisposalMethod, Metadata, Palette, RawFrame, Repeat, Rgba};

/// A reader wrapper that counts bytes read.
///
/// Used to track compressed input size for decompression ratio checks.
struct CountingRead<R> {
    inner: R,
    bytes_read: Arc<AtomicU64>,
}

impl<R> CountingRead<R> {
    fn new(inner: R) -> (Self, Arc<AtomicU64>) {
        let bytes_read = Arc::new(AtomicU64::new(0));
        (
            Self {
                inner,
                bytes_read: Arc::clone(&bytes_read),
            },
            bytes_read,
        )
    }
}

impl<R: Read> Read for CountingRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// GIF header size: magic (6) + logical screen descriptor (7) = 13 bytes
const GIF_HEADER_SIZE: usize = 13;

/// Pre-validate the GIF header before passing to the gif crate.
///
/// This allows us to check dimensions before the gif crate allocates memory.
/// Returns (width, height) on success.
fn pre_validate_header<R: BufRead>(reader: &mut R, limits: &Limits) -> Result<(u16, u16)> {
    // Peek at the header (don't consume it)
    let buf = reader.fill_buf().map_err(|e| {
        at!(GifError::Io {
            kind: e.kind(),
            context: Some("reading GIF header")
        })
    })?;

    if buf.len() < GIF_HEADER_SIZE {
        return Err(at!(GifError::GifCrate {
            message: "truncated header".to_string()
        }));
    }

    // Validate magic (GIF87a or GIF89a)
    if &buf[0..3] != b"GIF" {
        return Err(at!(GifError::InvalidHeader));
    }

    let version = &buf[3..6];
    if version != b"87a" && version != b"89a" {
        return Err(at!(GifError::UnsupportedVersion {
            version: [version[0], version[1], version[2]]
        }));
    }

    // Read dimensions from Logical Screen Descriptor
    let width = u16::from_le_bytes([buf[6], buf[7]]);
    let height = u16::from_le_bytes([buf[8], buf[9]]);

    // Pre-check dimensions BEFORE the gif crate can allocate
    limits.check_dimensions(width, height)?;

    Ok((width, height))
}

/// Streaming GIF decoder.
///
/// Decodes a GIF file frame by frame, producing composited RGBA output
/// with proper disposal method and transparency handling.
pub struct Decoder<R: Read, S: Stop> {
    /// Underlying gif crate reader (wrapped in CountingRead + BufReader for pre-validation).
    reader: gif::Decoder<BufReader<CountingRead<R>>>,

    /// Compositing screen.
    screen: Screen,

    /// Current frame index.
    frame_index: usize,

    /// Buffer for reading indexed pixels.
    pixel_buffer: Vec<u8>,

    /// Limits configuration.
    limits: Limits,

    /// Reference to stats tracker.
    stats_ref: StatsRef,

    /// Cancellation checker.
    stop: S,

    /// Whether we've finished reading all frames.
    finished: bool,

    /// Cached metadata.
    metadata: Metadata,

    /// Counter for compressed bytes read (for decompression ratio check).
    bytes_read: Arc<AtomicU64>,

    /// Counter for total decompressed bytes output.
    bytes_decompressed: u64,
}

/// Reference to stats for decoder.
///
/// We can't store a reference with the same lifetime as the struct,
/// so we use an enum to handle owned or borrowed stats.
enum StatsRef {
    /// Owned stats (decoder manages lifecycle).
    Owned(Stats),
    /// Pointer to external stats (caller manages lifecycle).
    /// Safety: Caller must ensure stats outlives decoder.
    External(*const Stats),
}

impl StatsRef {
    fn get(&self) -> &Stats {
        match self {
            StatsRef::Owned(s) => s,
            StatsRef::External(p) => unsafe { &**p },
        }
    }
}

// Safety: Stats is Sync, so our reference is safe to share
unsafe impl Send for StatsRef {}
unsafe impl Sync for StatsRef {}

impl<R: Read, S: Stop> Decoder<R, S> {
    /// Create a new decoder from a reader.
    ///
    /// # Arguments
    /// * `reader` - The GIF data source
    /// * `limits` - Size and memory limits
    /// * `stats` - Memory tracking statistics
    /// * `stop` - Cancellation checker
    pub fn new(reader: R, limits: Limits, stats: &Stats, stop: S) -> Result<Self> {
        // Check for cancellation
        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Wrap in CountingRead to track compressed bytes, then BufReader for pre-validation
        let (counting_reader, bytes_read) = CountingRead::new(reader);
        let mut buf_reader = BufReader::new(counting_reader);

        // Pre-validate header and check dimensions BEFORE gif crate can allocate
        let (width, height) = pre_validate_header(&mut buf_reader, &limits)?;

        // Configure gif decoder
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        options.allow_unknown_blocks(true);

        // Set memory limit based on our limits
        if let Some(max_pixels) = limits.max_total_pixels {
            if let Some(limit) = std::num::NonZeroU64::new(max_pixels) {
                options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
            }
        }

        // Parse the GIF (header already validated, dimensions already checked)
        let gif_reader = options
            .read_info(buf_reader)
            .map_err(|e| at!(GifError::from(e)))?;

        // Extract metadata
        let global_palette = gif_reader.global_palette().map(Palette::from_rgb_bytes);
        let background_index = gif_reader.bg_color().map(|c| c as u8);

        let metadata = Metadata {
            width,
            height,
            global_palette: global_palette.clone(),
            background_color_index: background_index,
            repeat: Repeat::Infinite, // Updated after first frame from NETSCAPE extension
            frame_count: 0,           // Unknown until we read all frames
            comments: Vec::new(),     // Note: gif crate doesn't expose comment extensions
        };

        // Create the compositing screen
        let screen = ScreenBuilder::from_decoder(&gif_reader).build(stats, &limits)?;

        // Allocate pixel buffer (fallible)
        let buffer_size = width as usize * height as usize;
        let buffer_bytes = buffer_size;
        stats.try_alloc(buffer_bytes, &limits)?;

        let mut pixel_buffer = Vec::new();
        pixel_buffer.try_reserve(buffer_size).map_err(|_| {
            stats.track_dealloc(buffer_bytes); // Undo tracking
            at!(GifError::AllocationFailed {
                requested: buffer_bytes
            })
        })?;
        pixel_buffer.resize(buffer_size, 0u8);

        Ok(Self {
            reader: gif_reader,
            screen,
            frame_index: 0,
            pixel_buffer,
            limits,
            stats_ref: StatsRef::External(stats as *const Stats),
            stop,
            finished: false,
            metadata,
            bytes_read,
            bytes_decompressed: 0,
        })
    }

    /// Create a decoder with owned stats.
    pub fn with_owned_stats(reader: R, limits: Limits, stop: S) -> Result<Self> {
        let stats = Stats::new();

        // Check for cancellation
        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Wrap in CountingRead to track compressed bytes, then BufReader for pre-validation
        let (counting_reader, bytes_read) = CountingRead::new(reader);
        let mut buf_reader = BufReader::new(counting_reader);

        // Pre-validate header and check dimensions BEFORE gif crate can allocate
        let (width, height) = pre_validate_header(&mut buf_reader, &limits)?;

        // Configure gif decoder
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        options.allow_unknown_blocks(true);

        // Parse the GIF (header already validated, dimensions already checked)
        let gif_reader = options
            .read_info(buf_reader)
            .map_err(|e| at!(GifError::from(e)))?;

        // Extract metadata
        let global_palette = gif_reader.global_palette().map(Palette::from_rgb_bytes);
        let background_index = gif_reader.bg_color().map(|c| c as u8);

        let metadata = Metadata {
            width,
            height,
            global_palette: global_palette.clone(),
            background_color_index: background_index,
            repeat: Repeat::Infinite,
            frame_count: 0,
            comments: Vec::new(),
        };

        // Create the compositing screen
        let screen = ScreenBuilder::from_decoder(&gif_reader).build(&stats, &limits)?;

        // Allocate pixel buffer (fallible)
        let buffer_size = width as usize * height as usize;
        stats.try_alloc(buffer_size, &limits)?;

        let mut pixel_buffer = Vec::new();
        pixel_buffer.try_reserve(buffer_size).map_err(|_| {
            stats.track_dealloc(buffer_size); // Undo tracking
            at!(GifError::AllocationFailed {
                requested: buffer_size
            })
        })?;
        pixel_buffer.resize(buffer_size, 0u8);

        Ok(Self {
            reader: gif_reader,
            screen,
            frame_index: 0,
            pixel_buffer,
            limits,
            stats_ref: StatsRef::Owned(stats),
            stop,
            finished: false,
            metadata,
            bytes_read,
            bytes_decompressed: 0,
        })
    }

    /// Get the canvas width.
    pub fn width(&self) -> u16 {
        self.screen.width()
    }

    /// Get the canvas height.
    pub fn height(&self) -> u16 {
        self.screen.height()
    }

    /// Get the metadata.
    ///
    /// Note: The `repeat` field is updated as frames are read, since the
    /// NETSCAPE extension is parsed during frame iteration.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Get the current repeat/loop setting.
    ///
    /// This value may change as frames are read, since the NETSCAPE
    /// extension is parsed during frame iteration.
    pub fn repeat(&self) -> Repeat {
        Repeat::from(self.reader.repeat())
    }

    /// Get the stats.
    pub fn stats(&self) -> &Stats {
        self.stats_ref.get()
    }

    /// Check if decoding is finished.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Get the current frame index.
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// Read and compose the next frame.
    ///
    /// Returns `None` when all frames have been read.
    pub fn next_frame(&mut self) -> Result<Option<ComposedFrame>> {
        if self.finished {
            return Ok(None);
        }

        // Check for cancellation periodically
        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Check frame count limit
        self.limits.check_frame_count(self.frame_index)?;

        // Try to read the next frame info
        let frame_info = match self.reader.next_frame_info() {
            Ok(Some(info)) => info.clone(),
            Ok(None) => {
                self.finished = true;
                return Ok(None);
            }
            Err(e) => {
                return Err(at!(GifError::from(e)));
            }
        };

        // Validate frame bounds
        if frame_info.left as u32 + frame_info.width as u32 > self.screen.width() as u32
            || frame_info.top as u32 + frame_info.height as u32 > self.screen.height() as u32
        {
            // Frame extends beyond canvas - this is technically invalid but common
            // We'll clip it during compositing
        }

        // Read frame pixels
        let frame_size = frame_info.width as usize * frame_info.height as usize;
        let buffer_slice = &mut self.pixel_buffer[..frame_size];
        buffer_slice.fill(0);

        self.reader
            .read_into_buffer(buffer_slice)
            .map_err(|e| at!(GifError::from(e)))?;

        // Track decompressed bytes and check ratio (zip bomb protection)
        self.bytes_decompressed += frame_size as u64;
        let bytes_read = self.bytes_read.load(Ordering::Relaxed);
        self.limits
            .check_decompression_ratio(bytes_read, self.bytes_decompressed)?;

        // Create RawFrame (fallible pixel copy)
        let mut pixels = Vec::new();
        pixels.try_reserve(buffer_slice.len()).map_err(|_| {
            at!(GifError::AllocationFailed {
                requested: buffer_slice.len()
            })
        })?;
        pixels.extend_from_slice(buffer_slice);

        let raw_frame = RawFrame {
            index: self.frame_index,
            left: frame_info.left,
            top: frame_info.top,
            width: frame_info.width,
            height: frame_info.height,
            delay: frame_info.delay,
            disposal: DisposalMethod::from(frame_info.dispose),
            transparent: frame_info.transparent,
            needs_user_input: frame_info.needs_user_input,
            interlaced: frame_info.interlaced,
            palette: frame_info
                .palette
                .as_ref()
                .map(|p| Palette::from_rgb_bytes(p)),
            pixels,
        };

        // Compose the frame
        let stats = self.stats_ref.get();
        let composed = self.screen.process_frame(&raw_frame, stats, &self.limits)?;

        self.frame_index += 1;

        // Update metadata with repeat value (parsed from NETSCAPE extension during frame read)
        self.metadata.repeat = Repeat::from(self.reader.repeat());

        Ok(Some(composed))
    }

    /// Process the next frame with a callback, without copying the canvas.
    ///
    /// This is more efficient than `next_frame()` for streaming use cases
    /// where you don't need to keep frames in memory. The callback receives
    /// the frame metadata and a reference to the composed pixels.
    ///
    /// Returns `Ok(None)` when all frames have been read.
    ///
    /// # Example
    /// ```ignore
    /// while decoder.with_next_frame(|index, delay, pixels| {
    ///     // Process pixels without copying
    ///     process_frame(pixels);
    /// })?.is_some() {}
    /// ```
    pub fn with_next_frame<F, T>(&mut self, f: F) -> Result<Option<T>>
    where
        F: FnOnce(usize, u16, &[Rgba]) -> T,
    {
        if self.finished {
            return Ok(None);
        }

        // Check for cancellation periodically
        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Check frame count limit
        self.limits.check_frame_count(self.frame_index)?;

        // Try to read the next frame info
        let frame_info = match self.reader.next_frame_info() {
            Ok(Some(info)) => info.clone(),
            Ok(None) => {
                self.finished = true;
                return Ok(None);
            }
            Err(e) => {
                return Err(at!(GifError::from(e)));
            }
        };

        // Read frame pixels
        let frame_size = frame_info.width as usize * frame_info.height as usize;
        let buffer_slice = &mut self.pixel_buffer[..frame_size];
        buffer_slice.fill(0);

        self.reader
            .read_into_buffer(buffer_slice)
            .map_err(|e| at!(GifError::from(e)))?;

        // Track decompressed bytes and check ratio (zip bomb protection)
        self.bytes_decompressed += frame_size as u64;
        let bytes_read = self.bytes_read.load(Ordering::Relaxed);
        self.limits
            .check_decompression_ratio(bytes_read, self.bytes_decompressed)?;

        // Create RawFrame (fallible pixel copy - unavoidable here)
        let mut pixels = Vec::new();
        pixels.try_reserve(buffer_slice.len()).map_err(|_| {
            at!(GifError::AllocationFailed {
                requested: buffer_slice.len()
            })
        })?;
        pixels.extend_from_slice(buffer_slice);

        let raw_frame = RawFrame {
            index: self.frame_index,
            left: frame_info.left,
            top: frame_info.top,
            width: frame_info.width,
            height: frame_info.height,
            delay: frame_info.delay,
            disposal: DisposalMethod::from(frame_info.dispose),
            transparent: frame_info.transparent,
            needs_user_input: frame_info.needs_user_input,
            interlaced: frame_info.interlaced,
            palette: frame_info
                .palette
                .as_ref()
                .map(|p| Palette::from_rgb_bytes(p)),
            pixels,
        };

        // Compose the frame in place (no canvas copy)
        let stats = self.stats_ref.get();
        let (index, delay) = self
            .screen
            .process_frame_in_place(&raw_frame, stats, &self.limits)?;

        self.frame_index += 1;

        // Update metadata with repeat value (parsed from NETSCAPE extension during frame read)
        self.metadata.repeat = Repeat::from(self.reader.repeat());

        // Call user callback with reference to composed pixels
        Ok(Some(f(index, delay, self.screen.pixels())))
    }

    /// Create an iterator over all frames.
    pub fn frames(self) -> FrameIterator<R, S> {
        FrameIterator { decoder: self }
    }

    /// Decode all frames into a vector.
    ///
    /// Useful for small animations where you want all frames in memory.
    pub fn decode_all(&mut self) -> Result<Vec<ComposedFrame>> {
        let mut frames = Vec::new();

        while let Some(frame) = self.next_frame()? {
            // Check cancellation between frames
            self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

            // Fallible push
            frames.try_reserve(1).map_err(|_| {
                at!(GifError::AllocationFailed {
                    requested: core::mem::size_of::<ComposedFrame>()
                })
            })?;
            frames.push(frame);
        }

        Ok(frames)
    }
}

/// Iterator adapter for decoder frames.
pub struct FrameIterator<R: Read, S: Stop> {
    decoder: Decoder<R, S>,
}

impl<R: Read, S: Stop> Iterator for FrameIterator<R, S> {
    type Item = Result<ComposedFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.decoder.is_finished() {
            return None;
        }

        match self.decoder.next_frame() {
            Ok(Some(frame)) => Some(Ok(frame)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

/// Convenience function to decode a GIF from bytes.
pub fn decode_gif<S: Stop>(
    data: &[u8],
    limits: Limits,
    stats: &Stats,
    stop: S,
) -> Result<(Metadata, Vec<ComposedFrame>)> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, limits, stats, stop)?;
    let frames = decoder.decode_all()?;
    let mut metadata = decoder.metadata().clone();
    metadata.frame_count = frames.len();
    Ok((metadata, frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;

    // A minimal valid GIF (1x1 red pixel)
    const MINIMAL_GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00, 0x00, // Color 1: Black
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x44, 0x01, // LZW data
        0x00, // Block terminator
        0x3B, // Trailer
    ];

    #[test]
    fn decode_minimal_gif() {
        let stats = Stats::new();
        let limits = Limits::default();

        let cursor = std::io::Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &stats, Unstoppable).unwrap();

        assert_eq!(decoder.width(), 1);
        assert_eq!(decoder.height(), 1);

        let frame = decoder.next_frame().unwrap().unwrap();
        assert_eq!(frame.width, 1);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.pixels.len(), 1);

        // Should be no more frames
        assert!(decoder.next_frame().unwrap().is_none());
        assert!(decoder.is_finished());
    }

    #[test]
    fn decode_with_limits() {
        let stats = Stats::new();
        let limits = Limits::default().max_dimensions(0, 0); // No images allowed

        let cursor = std::io::Cursor::new(MINIMAL_GIF);
        let result = Decoder::new(cursor, limits, &stats, Unstoppable);

        assert!(result.is_err());
    }

    #[test]
    fn frame_iterator() {
        let stats = Stats::new();
        let limits = Limits::default();

        let cursor = std::io::Cursor::new(MINIMAL_GIF);
        let decoder = Decoder::new(cursor, limits, &stats, Unstoppable).unwrap();

        let frames: Vec<_> = decoder.frames().collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_ok());
    }

    #[test]
    fn decode_all() {
        let stats = Stats::new();
        let limits = Limits::default();

        let (metadata, frames) = decode_gif(MINIMAL_GIF, limits, &stats, Unstoppable).unwrap();

        assert_eq!(metadata.width, 1);
        assert_eq!(metadata.height, 1);
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn decompression_ratio_check() {
        let stats = Stats::new();
        // Set a very low decompression ratio limit (0.1x means compressed must be
        // larger than decompressed, which is impossible for real GIFs)
        let limits = Limits::default().max_decompression_ratio(0.01);

        let cursor = std::io::Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &stats, Unstoppable).unwrap();

        // Should fail due to decompression ratio exceeded
        let result = decoder.next_frame();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error(),
            GifError::DecompressionRatioExceeded { .. }
        ));
    }

    #[test]
    fn decompression_ratio_ok() {
        let stats = Stats::new();
        // Normal ratio limit (1000x) should pass for typical GIFs
        let limits = Limits::default().max_decompression_ratio(1000.0);

        let cursor = std::io::Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &stats, Unstoppable).unwrap();

        // Should succeed
        let frame = decoder.next_frame().unwrap();
        assert!(frame.is_some());
    }
}
