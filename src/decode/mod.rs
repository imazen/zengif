//! GIF streaming decoder.
//!
//! Provides a streaming decoder that produces composited RGBA frames
//! with proper disposal method handling.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use enough::Stop;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::screen::{Screen, ScreenBuilder};
use crate::stats::Stats;
use crate::types::{ComposedFrame, DisposalMethod, Metadata, Palette, RawFrame, Repeat, Rgba};

/// A reader wrapper that counts bytes read and checks for cancellation.
///
/// Used to:
/// 1. Track compressed input size for decompression ratio checks
/// 2. Check Stop on every read, enabling cancellation during LZW decompression
///
/// # Cancellation Latency
///
/// The gif crate internally uses a BufReader (8KB default). Once data is buffered,
/// LZW decompression proceeds without further reads. This means:
/// - We check Stop every time BufReader refills (~8KB of compressed data)
/// - During LZW decompression of that 8KB, we cannot cancel
/// - For adversarial input (high compression ratio), latency could be significant
///
/// For truly responsive cancellation, the gif crate would need native Stop support.
struct StopCheckingRead<'a, R> {
    inner: R,
    bytes_read: Arc<AtomicUsize>,
    stop: &'a dyn Stop,
}

impl<'a, R> StopCheckingRead<'a, R> {
    fn new(inner: R, stop: &'a dyn Stop) -> (Self, Arc<AtomicUsize>) {
        let bytes_read = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner,
                bytes_read: Arc::clone(&bytes_read),
                stop,
            },
            bytes_read,
        )
    }
}

impl<R: Read> Read for StopCheckingRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Check for cancellation on every read
        if self.stop.check().is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }

        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}

/// Check if a gif crate DecodingError is an unexpected EOF.
///
/// Used to tolerate GIF files missing the trailer byte (0x3B).
/// Many real-world GIFs lack this byte, and browsers display them fine.
fn is_unexpected_eof(err: &gif::DecodingError) -> bool {
    matches!(err, gif::DecodingError::UnexpectedEof)
}

/// GIF header size: magic (6) + logical screen descriptor (7) = 13 bytes
const GIF_HEADER_SIZE: usize = 13;

/// Pre-validate the GIF header before passing to the gif crate.
///
/// This allows us to check dimensions before the gif crate allocates memory.
/// Returns (header_bytes, width, height, pixel_aspect_ratio_byte) on success.
/// The header bytes must be chained back to the reader before passing to gif crate.
fn pre_validate_header<R: Read>(
    reader: &mut R,
    limits: &Limits,
) -> Result<([u8; GIF_HEADER_SIZE], u16, u16, u8)> {
    let mut buf = [0u8; GIF_HEADER_SIZE];
    reader
        .read_exact(&mut buf)
        .map_err(|e| at!(GifError::from(e)))?;

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

    // Pixel aspect ratio byte (position 12 in the header)
    let pixel_aspect_ratio_byte = buf[12];

    // Pre-check dimensions BEFORE the gif crate can allocate
    limits.check_dimensions(width, height)?;

    Ok((buf, width, height, pixel_aspect_ratio_byte))
}

/// Reader type that chains the pre-read header bytes with the rest of the stream.
type ChainedReader<'a, R> =
    std::io::Chain<std::io::Cursor<[u8; GIF_HEADER_SIZE]>, StopCheckingRead<'a, R>>;

/// Streaming GIF decoder.
///
/// Decodes a GIF file frame by frame, producing composited RGBA output
/// with proper disposal method and transparency handling.
pub struct Decoder<'a, R: Read> {
    /// Underlying gif crate reader. Header bytes are chained back after pre-validation,
    /// and Stop is checked on every read for cancellation during LZW decompression.
    reader: gif::Decoder<ChainedReader<'a, R>>,

    /// Compositing screen.
    screen: Screen,

    /// Current frame index.
    frame_index: usize,

    /// Buffer for reading indexed pixels.
    pixel_buffer: Vec<u8>,

    /// Limits configuration.
    limits: Limits,

    /// Memory usage statistics (owned by decoder).
    stats: Stats,

    /// Cancellation checker.
    stop: &'a dyn Stop,

    /// Whether we've finished reading all frames.
    finished: bool,

    /// Cached metadata.
    metadata: Metadata,

    /// Counter for compressed bytes read (for decompression ratio check).
    bytes_read: Arc<AtomicUsize>,

    /// Counter for total decompressed bytes output.
    bytes_decompressed: u64,

    /// Cumulative animation duration in milliseconds (for max_animation_ms enforcement).
    cumulative_duration_ms: u64,
}

// Stats is always owned by the decoder - no unsafe raw pointers.

impl<'a, R: Read> Decoder<'a, R> {
    /// Create a new decoder from a reader.
    ///
    /// The decoder owns its memory statistics internally. Use `stats()`
    /// to access memory usage information after decoding.
    ///
    /// # Arguments
    /// * `reader` - The GIF data source
    /// * `limits` - Size and memory limits
    /// * `stop` - Cancellation checker (checked on every read)
    pub fn new(reader: R, limits: Limits, stop: &'a dyn Stop) -> Result<Self> {
        let stats = Stats::new();

        // Check for cancellation
        stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Wrap in StopCheckingRead to track bytes and enable cancellation during reads
        let (mut stop_reader, bytes_read) = StopCheckingRead::new(reader, stop);

        // Pre-validate header and check dimensions BEFORE gif crate can allocate
        let (header, width, height, par_byte) = pre_validate_header(&mut stop_reader, &limits)?;

        // Chain header bytes back with the rest of the stream
        let chained = std::io::Cursor::new(header).chain(stop_reader);

        // Configure gif decoder with safety checks
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        options.allow_unknown_blocks(true);
        options.check_frame_consistency(true); // Validate frame bounds against canvas

        // Set memory limit based on our limits (convert pixels to bytes)
        if let Some(max_pixels) = limits.max_total_pixels {
            // Each pixel is 1 byte (indexed) in gif crate's buffer
            if let Some(limit) = core::num::NonZeroU64::new(max_pixels) {
                options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
            }
        }

        // Parse the GIF (header already validated, dimensions already checked)
        let gif_reader = options
            .read_info(chained)
            .map_err(|e| at!(GifError::from(e)))?;

        // Extract metadata
        let global_palette = gif_reader.global_palette().map(Palette::from_rgb_bytes);
        let background_index = gif_reader.bg_color().map(|c| c as u8);

        // Pixel aspect ratio: if byte is 0, square pixels (None).
        // Otherwise ratio = (byte + 15) / 64.
        let pixel_aspect_ratio = if par_byte == 0 {
            None
        } else {
            Some((par_byte as f32 + 15.0) / 64.0)
        };

        let metadata = Metadata {
            width,
            height,
            global_palette: global_palette.clone(),
            background_color_index: background_index,
            repeat: Repeat::Infinite, // Updated after first frame from NETSCAPE extension
            frame_count: 0,           // Unknown until we read all frames
            comments: Vec::new(),     // Note: gif crate doesn't expose comment extensions
            pixel_aspect_ratio,
        };

        // Create the compositing screen
        let screen = ScreenBuilder::from_decoder(&gif_reader).build(&stats, &limits)?;

        // Allocate the indexed pixel buffer. Sized from the (untrusted) screen
        // dimensions → default fallible; honours an explicit `Infallible` for
        // trusted/benchmark paths. Memory tracking (limit enforcement) is
        // handled inside the helper.
        let buffer_size = width as usize * height as usize;
        let pixel_buffer =
            crate::alloc_util::alloc_zeroed(limits.alloc_pref, true, buffer_size, &stats, &limits)?;

        Ok(Self {
            reader: gif_reader,
            screen,
            frame_index: 0,
            pixel_buffer,
            limits,
            stats,
            stop,
            finished: false,
            metadata,
            bytes_read,
            bytes_decompressed: 0,
            cumulative_duration_ms: 0,
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
        &self.stats
    }

    /// Consume the decoder and return the owned stats.
    pub fn into_stats(self) -> Stats {
        self.stats
    }

    /// Check if decoding is finished.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Get the current frame index.
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// Ensure the pixel buffer is large enough for the given frame size.
    ///
    /// The growth is sized from the (untrusted) frame dimensions → default
    /// fallible; an explicit `Infallible` resizes directly (faster, aborts on
    /// OOM). Either way the memory-limit check via `Stats::try_alloc` runs
    /// first so an `Infallible` preference can't bypass the resource cap.
    fn ensure_buffer_capacity(&mut self, needed: usize) -> Result<()> {
        if self.pixel_buffer.len() >= needed {
            return Ok(());
        }
        // Need to grow the buffer
        let additional = needed - self.pixel_buffer.len();
        self.stats.try_alloc(additional, &self.limits)?;
        if crate::alloc_util::resolve_fallible(self.limits.alloc_pref, true)
            && self.pixel_buffer.try_reserve(additional).is_err()
        {
            self.stats.track_dealloc(additional);
            return Err(at!(GifError::AllocationFailed {
                requested: additional as u64
            }));
        }
        self.pixel_buffer.resize(needed, 0);
        Ok(())
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
        self.limits.check_frame_count(self.frame_index as u64)?;

        // Try to read the next frame info
        let frame_info = match self.reader.next_frame_info() {
            Ok(Some(info)) => info.clone(),
            Ok(None) => {
                self.finished = true;
                return Ok(None);
            }
            Err(e) => {
                // Tolerate missing trailer: if we already decoded frames and hit EOF
                // between frames, treat it as end-of-stream. This matches browser
                // behavior for GIFs without the 0x3B trailer byte.
                // See: https://github.com/image-rs/image-gif/issues/138
                if self.frame_index > 0 && is_unexpected_eof(&e) {
                    self.finished = true;
                    return Ok(None);
                }
                return Err(at!(GifError::from(e)));
            }
        };

        // Reject zero-dimension frames: the gif crate's LZW decoder loops
        // forever when given a zero-length output buffer because it keeps
        // reading LZW data but can never write decoded bytes.
        if frame_info.width == 0 || frame_info.height == 0 {
            return Err(at!(GifError::ZeroDimensionFrame {
                frame_index: self.frame_index,
                frame_width: frame_info.width,
                frame_height: frame_info.height,
            }));
        }

        // Validate frame bounds
        if frame_info.left as u32 + frame_info.width as u32 > self.screen.width() as u32
            || frame_info.top as u32 + frame_info.height as u32 > self.screen.height() as u32
        {
            // Frame extends beyond canvas - this is technically invalid but common
            // We'll clip it during compositing
        }

        // Read frame pixels - ensure buffer is large enough for this frame
        let frame_size = frame_info.width as usize * frame_info.height as usize;
        self.ensure_buffer_capacity(frame_size)?;
        let buffer_slice = &mut self.pixel_buffer[..frame_size];
        // Note: no fill(0) needed — read_into_buffer writes exactly frame_size
        // bytes on success (GIF guarantees width*height indexed pixels per frame).

        self.reader
            .read_into_buffer(buffer_slice)
            .map_err(|e| at!(GifError::from(e)))?;

        // Track decompressed bytes and check ratio (zip bomb protection)
        self.bytes_decompressed += frame_size as u64;
        let bytes_read = self.bytes_read.load(Ordering::Relaxed);
        self.limits
            .check_decompression_ratio(bytes_read as u64, self.bytes_decompressed)?;

        // Avoid cloning the indexed pixel buffer: swap the pixel_buffer into
        // the RawFrame, compose, then swap it back. process_frame only borrows
        // the frame immutably, so the pixels are returned intact.
        let pixels = core::mem::take(&mut self.pixel_buffer);

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

        // Compose the frame (clones canvas for multi-frame safety)
        let stats = &self.stats;
        let composed = self.screen.process_frame(&raw_frame, stats, &self.limits)?;

        // Reclaim the pixel buffer
        self.pixel_buffer = raw_frame.pixels;

        // Track cumulative animation duration (delay is in centiseconds)
        self.cumulative_duration_ms += frame_info.delay as u64 * 10;
        self.limits
            .check_animation_duration(self.cumulative_duration_ms)?;

        self.frame_index += 1;

        // Update metadata with repeat value (parsed from NETSCAPE extension during frame read)
        self.metadata.repeat = Repeat::from(self.reader.repeat());

        Ok(Some(composed))
    }

    /// Read, compose, and return the next frame, moving the canvas pixels
    /// out instead of cloning them.
    ///
    /// This is a zero-copy variant of `next_frame` that avoids both the
    /// indexed pixel buffer clone (B.1) and the RGBA canvas clone (B.2).
    /// After calling this, the decoder's Screen canvas is empty and the
    /// decoder should not be used for further compositing.
    ///
    /// Use this for single-frame decode paths where the decoder will be
    /// dropped immediately after.
    pub fn next_frame_take(&mut self) -> Result<Option<ComposedFrame>> {
        if self.finished {
            return Ok(None);
        }

        // Check for cancellation periodically
        self.stop.check().map_err(|_| at!(GifError::Cancelled))?;

        // Check frame count limit
        self.limits.check_frame_count(self.frame_index as u64)?;

        // Try to read the next frame info
        let frame_info = match self.reader.next_frame_info() {
            Ok(Some(info)) => info.clone(),
            Ok(None) => {
                self.finished = true;
                return Ok(None);
            }
            Err(e) => {
                if self.frame_index > 0 && is_unexpected_eof(&e) {
                    self.finished = true;
                    return Ok(None);
                }
                return Err(at!(GifError::from(e)));
            }
        };

        // Reject zero-dimension frames (same as next_frame — prevents
        // infinite loop in the gif crate's LZW decoder).
        if frame_info.width == 0 || frame_info.height == 0 {
            return Err(at!(GifError::ZeroDimensionFrame {
                frame_index: self.frame_index,
                frame_width: frame_info.width,
                frame_height: frame_info.height,
            }));
        }

        // Read frame pixels
        let frame_size = frame_info.width as usize * frame_info.height as usize;
        self.ensure_buffer_capacity(frame_size)?;
        let buffer_slice = &mut self.pixel_buffer[..frame_size];

        self.reader
            .read_into_buffer(buffer_slice)
            .map_err(|e| at!(GifError::from(e)))?;

        // Track decompressed bytes and check ratio (zip bomb protection)
        self.bytes_decompressed += frame_size as u64;
        let bytes_read = self.bytes_read.load(Ordering::Relaxed);
        self.limits
            .check_decompression_ratio(bytes_read as u64, self.bytes_decompressed)?;

        // Avoid cloning the indexed pixel buffer (B.1): swap the pixel_buffer
        // into the RawFrame, compose, then swap it back. This is zero-copy
        // for the indexed pixels.
        let pixels = core::mem::take(&mut self.pixel_buffer);

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

        // Compose the frame, taking the canvas pixels (zero-copy B.2)
        let stats = &self.stats;
        let composed = self
            .screen
            .process_frame_take(&raw_frame, stats, &self.limits)?;

        // Reclaim the pixel buffer for potential future use
        self.pixel_buffer = raw_frame.pixels;

        // Track cumulative animation duration (delay is in centiseconds)
        self.cumulative_duration_ms += frame_info.delay as u64 * 10;
        self.limits
            .check_animation_duration(self.cumulative_duration_ms)?;

        self.frame_index += 1;

        // Update metadata with repeat value
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
        self.limits.check_frame_count(self.frame_index as u64)?;

        // Try to read the next frame info
        let frame_info = match self.reader.next_frame_info() {
            Ok(Some(info)) => info.clone(),
            Ok(None) => {
                self.finished = true;
                return Ok(None);
            }
            Err(e) => {
                // Tolerate missing trailer (same as next_frame)
                if self.frame_index > 0 && is_unexpected_eof(&e) {
                    self.finished = true;
                    return Ok(None);
                }
                return Err(at!(GifError::from(e)));
            }
        };

        // Reject zero-dimension frames (same as next_frame — prevents
        // infinite loop in the gif crate's LZW decoder).
        if frame_info.width == 0 || frame_info.height == 0 {
            return Err(at!(GifError::ZeroDimensionFrame {
                frame_index: self.frame_index,
                frame_width: frame_info.width,
                frame_height: frame_info.height,
            }));
        }

        // Read frame pixels - ensure buffer is large enough for this frame
        let frame_size = frame_info.width as usize * frame_info.height as usize;
        self.ensure_buffer_capacity(frame_size)?;
        let buffer_slice = &mut self.pixel_buffer[..frame_size];
        // Note: no fill(0) needed — read_into_buffer writes exactly frame_size bytes.

        self.reader
            .read_into_buffer(buffer_slice)
            .map_err(|e| at!(GifError::from(e)))?;

        // Track decompressed bytes and check ratio (zip bomb protection)
        self.bytes_decompressed += frame_size as u64;
        let bytes_read = self.bytes_read.load(Ordering::Relaxed);
        self.limits
            .check_decompression_ratio(bytes_read as u64, self.bytes_decompressed)?;

        // Avoid cloning the indexed pixel buffer: swap into RawFrame, compose,
        // then swap back. process_frame_in_place only borrows immutably.
        let pixels = core::mem::take(&mut self.pixel_buffer);

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
        let stats = &self.stats;
        let (index, delay) = self
            .screen
            .process_frame_in_place(&raw_frame, stats, &self.limits)?;

        // Reclaim the pixel buffer
        self.pixel_buffer = raw_frame.pixels;

        // Track cumulative animation duration (delay is in centiseconds)
        self.cumulative_duration_ms += frame_info.delay as u64 * 10;
        self.limits
            .check_animation_duration(self.cumulative_duration_ms)?;

        self.frame_index += 1;

        // Update metadata with repeat value (parsed from NETSCAPE extension during frame read)
        self.metadata.repeat = Repeat::from(self.reader.repeat());

        // Call user callback with reference to composed pixels
        Ok(Some(f(index, delay, self.screen.pixels())))
    }

    /// Create an iterator over all frames.
    pub fn frames(self) -> FrameIterator<'a, R> {
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
                    requested: core::mem::size_of::<ComposedFrame>() as u64
                })
            })?;
            frames.push(frame);
        }

        Ok(frames)
    }
}

/// Iterator adapter for decoder frames.
pub struct FrameIterator<'a, R: Read> {
    decoder: Decoder<'a, R>,
}

impl<R: Read> Iterator for FrameIterator<'_, R> {
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
///
/// Returns metadata, frames, and memory usage statistics.
pub fn decode_gif(
    data: &[u8],
    limits: Limits,
    stop: &dyn Stop,
) -> Result<(Metadata, Vec<ComposedFrame>, Stats)> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, limits, stop)?;
    let frames = decoder.decode_all()?;
    let mut metadata = decoder.metadata().clone();
    metadata.frame_count = frames.len();
    // Return owned stats from decoder
    Ok((metadata, frames, decoder.into_stats()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;
    use std::io::Cursor;

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
        let limits = Limits::default();

        let cursor = Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

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
        let limits = Limits::default().max_dimensions(0, 0); // No images allowed

        let cursor = Cursor::new(MINIMAL_GIF);
        let result = Decoder::new(cursor, limits, &Unstoppable);

        assert!(result.is_err());
    }

    #[test]
    fn frame_iterator() {
        let limits = Limits::default();

        let cursor = Cursor::new(MINIMAL_GIF);
        let decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

        let frames: Vec<_> = decoder.frames().collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_ok());
    }

    #[test]
    fn decode_all() {
        let limits = Limits::default();

        let (metadata, frames, _stats) = decode_gif(MINIMAL_GIF, limits, &Unstoppable).unwrap();

        assert_eq!(metadata.width, 1);
        assert_eq!(metadata.height, 1);
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn decompression_ratio_check() {
        // Set a very low decompression ratio limit (0.1x means compressed must be
        // larger than decompressed, which is impossible for real GIFs)
        let limits = Limits::default().max_decompression_ratio(0.01);

        let cursor = Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

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
        // Normal ratio limit (1000x) should pass for typical GIFs
        let limits = Limits::default().max_decompression_ratio(1000.0);

        let cursor = Cursor::new(MINIMAL_GIF);
        let mut decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();

        // Should succeed
        let frame = decoder.next_frame().unwrap();
        assert!(frame.is_some());
    }

    #[test]
    fn pixel_aspect_ratio_zero() {
        // MINIMAL_GIF has pixel aspect ratio byte = 0 → square pixels
        let limits = Limits::default();
        let cursor = Cursor::new(MINIMAL_GIF);
        let decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();
        assert_eq!(decoder.metadata().pixel_aspect_ratio, None);
    }

    #[test]
    fn pixel_aspect_ratio_nonzero() {
        // Construct a GIF with pixel aspect ratio byte = 49
        // Expected ratio: (49 + 15) / 64 = 64 / 64 = 1.0
        let mut gif_data = MINIMAL_GIF.to_vec();
        gif_data[12] = 49; // pixel aspect ratio byte
        let limits = Limits::default();
        let cursor = Cursor::new(&gif_data);
        let decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();
        let par = decoder.metadata().pixel_aspect_ratio.unwrap();
        assert!((par - 1.0).abs() < f32::EPSILON, "expected 1.0, got {par}");
    }

    #[test]
    fn pixel_aspect_ratio_wide() {
        // Byte = 113 → ratio = (113 + 15) / 64 = 128 / 64 = 2.0
        let mut gif_data = MINIMAL_GIF.to_vec();
        gif_data[12] = 113;
        let limits = Limits::default();
        let cursor = Cursor::new(&gif_data);
        let decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();
        let par = decoder.metadata().pixel_aspect_ratio.unwrap();
        assert!((par - 2.0).abs() < f32::EPSILON, "expected 2.0, got {par}");
    }

    #[test]
    fn pixel_aspect_ratio_narrow() {
        // Byte = 1 → ratio = (1 + 15) / 64 = 16 / 64 = 0.25
        let mut gif_data = MINIMAL_GIF.to_vec();
        gif_data[12] = 1;
        let limits = Limits::default();
        let cursor = Cursor::new(&gif_data);
        let decoder = Decoder::new(cursor, limits, &Unstoppable).unwrap();
        let par = decoder.metadata().pixel_aspect_ratio.unwrap();
        assert!(
            (par - 0.25).abs() < f32::EPSILON,
            "expected 0.25, got {par}"
        );
    }
}
