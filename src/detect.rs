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

use crate::Limits;
use enough::Stop;

/// Maximum block-walk iterations even when no [`Limits`] is supplied.
///
/// Each loop iteration consumes at least one byte of input (every branch
/// advances by ≥ 1 byte), so capping at `data.len() + 1` guarantees `O(input)`
/// work without introducing artificial truncation on legitimate inputs. The
/// `+ 1` lets the trailer byte be visited before the loop exits.
const PROBE_MAX_ITERATIONS_FACTOR: usize = 1;

/// Hard cap on frames discovered during a probe, independent of [`Limits`].
///
/// A 100 MB body packed with `0x2C 0x00 ... 0x00` Image Descriptors yields
/// ~9 M frames on a 13-byte header overhead. We cap at 1 M frames here so that
/// even callers passing [`Limits::none()`] cannot be made to spend unbounded
/// time accumulating frame metadata. Real-world GIFs rarely exceed a few thousand
/// frames; a 1 M cap is generous while still bounding work.
const PROBE_HARD_FRAME_CAP: u32 = 1_000_000;

/// Cancellation poll interval (frames) — keeps overhead negligible while
/// bounding cancellation latency to a handful of frames.
const PROBE_STOP_POLL_INTERVAL: u32 = 256;

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
    /// Whether the composited canvas can contain non-opaque pixels.
    ///
    /// True when any frame's Graphic Control Extension sets the
    /// transparent-colour flag, when the first frame does not cover the whole
    /// logical screen (the canvas starts fully transparent, so the uncovered
    /// border composites transparent), or when any frame uses "restore to
    /// background" disposal (which clears its rectangle to transparent).
    ///
    /// Deliberately conservative: it may report `true` for a canvas a full
    /// decode would find opaque — a later frame can cover the border. The
    /// asymmetry is intentional, because this drives `ImageInfo::has_alpha`,
    /// and a caller who is told "opaque" may request `RGB8_SRGB` and lose real
    /// transparency permanently.
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
    /// - `Some(n)` — repeat `n` times after the first play (`n + 1` total).
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
    /// Frame count exceeded a configured or built-in limit while probing.
    ///
    /// This guards against malicious inputs that pack the body with trivial
    /// Image Descriptors to force unbounded CPU during structural analysis.
    TooManyFrames {
        /// Frames observed at the point of rejection.
        count: u64,
        /// The cap that was exceeded.
        max: u64,
    },
    /// Probe was cancelled via the supplied [`enough::Stop`].
    Cancelled,
}

impl core::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "data too short to be a GIF file"),
            Self::NotGif => write!(f, "not a GIF file (missing GIF87a/GIF89a signature)"),
            Self::Truncated => write!(f, "truncated GIF file"),
            Self::TooManyFrames { count, max } => {
                write!(f, "probe frame count {count} exceeded limit {max}")
            }
            Self::Cancelled => write!(f, "probe was cancelled"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProbeError {}

/// Probe a GIF file from its raw bytes with default safety caps.
///
/// Scans the GIF block structure to extract animation properties, palette
/// usage, and transparency info. No LZW decompression is performed.
///
/// This wrapper enforces a built-in [`PROBE_HARD_FRAME_CAP`] (1 M frames)
/// and an iteration ceiling proportional to `data.len()`, so it cannot be
/// made to spend unbounded CPU on a hostile body of trivial Image Descriptors.
/// It does not support cancellation; use [`probe_with_limits`] if you need
/// to honour a [`Stop`] token or stricter frame caps.
pub fn probe(data: &[u8]) -> Result<GifProbe, ProbeError> {
    probe_with_limits(data, &Limits::none(), &enough::Unstoppable)
}

/// Probe a GIF file from its raw bytes, honouring `limits` and `stop`.
///
/// Like [`probe()`] but additionally:
///
/// - Rejects with [`ProbeError::TooManyFrames`] once `limits.max_frame_count`
///   (or the built-in [`PROBE_HARD_FRAME_CAP`], whichever is smaller) is hit.
/// - Polls `stop` periodically and returns [`ProbeError::Cancelled`] if asked.
/// - Always bounds total work at `O(data.len())` regardless of the supplied
///   limits, so callers cannot accidentally disable the DoS protection by
///   passing [`Limits::none()`].
pub fn probe_with_limits(
    data: &[u8],
    limits: &Limits,
    stop: &dyn Stop,
) -> Result<GifProbe, ProbeError> {
    probe_with_stop_reason(data, limits, stop, &mut None)
}

// Preserve the reason for internal codec adapters without changing ProbeError's
// public unit variant. The public probe API retains its existing error shape.
pub(crate) fn probe_with_stop_reason(
    data: &[u8],
    limits: &Limits,
    stop: &dyn Stop,
    stopped: &mut Option<enough::StopReason>,
) -> Result<GifProbe, ProbeError> {
    probe_located_with_stop_reason(data, limits, stop, stopped).map(|(probe, _)| probe)
}

/// [`probe_with_limits`], also returning the offset of the `0x21` introducer of
/// the NETSCAPE2.0 extension whose loop count ended up in
/// [`GifProbe::repeat`]. The structural inventory uses it to say which bytes
/// `probe()` reads.
#[cfg(feature = "inventory")]
pub(crate) fn probe_located(
    data: &[u8],
    limits: &Limits,
    stop: &dyn Stop,
) -> Result<(GifProbe, Option<usize>), ProbeError> {
    probe_located_with_stop_reason(data, limits, stop, &mut None)
}

/// Advance past a GIF sub-block chain (`len data... 00`) starting at `pos`.
fn skip_sub_blocks(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() {
        let block_size = data[pos] as usize;
        pos += 1;
        if block_size == 0 {
            break;
        }
        pos = pos.saturating_add(block_size);
    }
    pos
}

fn probe_located_with_stop_reason(
    data: &[u8],
    limits: &Limits,
    stop: &dyn Stop,
    stopped: &mut Option<enough::StopReason>,
) -> Result<(GifProbe, Option<usize>), ProbeError> {
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

    // Frame-count cap: take the tightest of (caller-supplied, hard cap).
    // u64 throughout to mirror Limits::max_frame_count semantics.
    let frame_cap: u64 = limits
        .max_frame_count
        .map(|m| m.min(PROBE_HARD_FRAME_CAP as u64))
        .unwrap_or(PROBE_HARD_FRAME_CAP as u64);

    // Iteration cap: `data.len() + 1` is sufficient because every loop branch
    // either advances `pos` by ≥ 1 byte or breaks. Saturating add avoids
    // overflow on data.len() == usize::MAX.
    let iter_cap: usize = data
        .len()
        .saturating_mul(PROBE_MAX_ITERATIONS_FACTOR)
        .saturating_add(1);

    // Scan blocks
    let mut frame_count = 0u32;
    let mut total_duration_cs = 0u32;
    let mut has_transparency = false;
    let mut has_local_palettes = false;
    let mut max_local_palette_size = 0u16;
    let mut has_interlacing = false;
    let mut repeat: Option<u16> = None;
    let mut repeat_at: Option<usize> = None;
    let mut pending_gce_transparent = false;
    let mut pending_gce_delay = 0u16;
    let mut pending_gce_disposal = 0u8;
    // The `gif` crate stops parsing header-phase application extensions at the
    // first block whose introducer is not 0x21 (image, trailer, or unknown).
    let mut header_over = false;

    let mut iters: usize = 0;

    while pos < data.len() {
        // Hard iteration cap — defends against any future code path that
        // might fail to advance `pos`. Cheap (one branch per outer iteration).
        if iters >= iter_cap {
            break;
        }
        iters = iters.saturating_add(1);

        // Cancellation poll: cheap when Unstoppable, bounded latency otherwise.
        if frame_count.is_multiple_of(PROBE_STOP_POLL_INTERVAL)
            && let Err(reason) = stop.check()
        {
            *stopped = Some(reason);
            return Err(ProbeError::Cancelled);
        }

        if data[pos] != 0x21 {
            header_over = true;
        }

        match data[pos] {
            0x3B => break, // Trailer
            0x21 => {
                // Extension block
                if pos + 2 >= data.len() {
                    break;
                }
                let ext_start = pos;
                let label = data[pos + 1];
                pos += 2;

                // Walk sub-blocks to the 0x00 terminator. Capture the first two
                // sub-blocks (NETSCAPE2.0) and the last non-empty sub-block (the
                // `gif` crate applies only the last sub-block of a GCE).
                let mut first_sub: Option<&[u8]> = None;
                let mut second_sub: Option<&[u8]> = None;
                let mut last_sub: Option<&[u8]> = None;
                let mut sub_count = 0usize;
                let mut terminated = false;
                while pos < data.len() {
                    let block_size = data[pos] as usize;
                    pos += 1;
                    if block_size == 0 {
                        terminated = true;
                        break;
                    }
                    let Some(end) = pos.checked_add(block_size).filter(|&e| e <= data.len()) else {
                        last_sub = None;
                        pos = pos.saturating_add(block_size);
                        break;
                    };
                    let sub = &data[pos..end];
                    if sub_count == 0 {
                        first_sub = Some(sub);
                    } else if sub_count == 1 {
                        second_sub = Some(sub);
                    }
                    last_sub = Some(sub);
                    sub_count += 1;
                    pos = end;
                }

                if label == 0xF9 {
                    // Graphics Control Extension: the `gif` crate parses only
                    // the last sub-block at the 0x00 terminator and requires
                    // exactly 4 bytes.
                    pending_gce_transparent = false;
                    pending_gce_delay = 0;
                    pending_gce_disposal = 0;
                    if terminated
                        && let Some(gce) = last_sub
                        && gce.len() == 4
                    {
                        let gce_packed = gce[0];
                        pending_gce_delay = u16::from_le_bytes([gce[1], gce[2]]);
                        pending_gce_transparent = (gce_packed & 0x01) != 0;
                        pending_gce_disposal = (gce_packed >> 2) & 0x07;
                    }
                } else if label == 0xFF
                    && !header_over
                    && terminated
                    && let Some(ident) = first_sub
                    && ident == b"NETSCAPE2.0"
                    && let Some([1, lo, hi]) = second_sub
                {
                    // Application Extension — NETSCAPE2.0 loop count (only
                    // read before the first non-extension block).
                    repeat = Some(u16::from_le_bytes([*lo, *hi]));
                    repeat_at = Some(ext_start);
                }
            }
            0x2C => {
                // Image Descriptor
                if pos + 10 > data.len() {
                    break;
                }
                // Frame-count enforcement happens BEFORE we record the frame so
                // the rejection reflects the cap exactly.
                if frame_count as u64 >= frame_cap {
                    return Err(ProbeError::TooManyFrames {
                        count: frame_count as u64,
                        max: frame_cap,
                    });
                }
                frame_count += 1;
                total_duration_cs = total_duration_cs.saturating_add(pending_gce_delay as u32);
                if pending_gce_transparent {
                    has_transparency = true;
                }

                // The Graphic Control Extension's transparent-colour flag is not
                // the only way a composited canvas ends up with transparent
                // pixels, and reporting "opaque" for one that has them is a
                // pixel-loss bug: `ImageInfo::has_alpha` is derived from this,
                // and a caller who trusts it asks for `RGB8_SRGB`, at which
                // point `negotiate_format` drops the alpha channel for good.
                // Two further sources, both cheap to spot from the headers:
                //
                //  1. The canvas is initialised fully transparent (see
                //     `Screen::new` — matching browser behaviour), so if the
                //     FIRST frame does not cover the whole logical screen, the
                //     uncovered border composites transparent no matter what the
                //     GCE says. `border_touching_layers.gif` in the corpus is a
                //     real instance.
                //  2. "Restore to background" disposal (method 2) clears its
                //     rectangle to transparent in this decoder, so a later frame
                //     that does not overdraw it shows through.
                //
                // Both are deliberately conservative: they can report
                // transparency for a canvas a full decode would find opaque
                // (a later frame may cover the border), which costs an
                // unnecessary RGBA buffer. Reporting opaque for a canvas that is
                // not would cost the caller their pixels.
                // Image Descriptor layout, from the 0x2C separator at `pos`:
                // separator(1) left(2) top(2) width(2) height(2) packed(1).
                let frame_left = u16::from_le_bytes([data[pos + 1], data[pos + 2]]);
                let frame_top = u16::from_le_bytes([data[pos + 3], data[pos + 4]]);
                let frame_width = u16::from_le_bytes([data[pos + 5], data[pos + 6]]);
                let frame_height = u16::from_le_bytes([data[pos + 7], data[pos + 8]]);
                if frame_count == 1 {
                    let covers_screen = frame_left == 0
                        && frame_top == 0
                        && (frame_left as u32 + frame_width as u32) >= width as u32
                        && (frame_top as u32 + frame_height as u32) >= height as u32;
                    if !covers_screen {
                        has_transparency = true;
                    }
                }
                if pending_gce_disposal == 2 {
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
                    pos = pos.saturating_add(10 + 3 * local_size as usize);
                } else {
                    pos += 10;
                }

                // Skip LZW minimum code size
                if pos >= data.len() {
                    break;
                }
                pos += 1; // LZW min code size

                // Skip sub-blocks (image data)
                pos = skip_sub_blocks(data, pos);

                // Reset GCE state
                pending_gce_transparent = false;
                pending_gce_delay = 0;
                pending_gce_disposal = 0;
            }
            _ => {
                // Unknown block introducer: with `allow_unknown_blocks(true)` the
                // `gif` crate consumes this introducer byte and reads the
                // following bytes as a label-less sub-block chain (`len data... 00`).
                pos = skip_sub_blocks(data, pos + 1);
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

    Ok((
        GifProbe {
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
        },
        repeat_at,
    ))
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

#[cfg(feature = "std")]
impl zencodec::SourceEncodingDetails for GifProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        None
    }

    fn is_lossless(&self) -> bool {
        true
    }
}

/// Codec-agnostic error taxonomy for probe failures (audit finding #7): without
/// this impl, a generic consumer driving `probe`/`probe_with_limits` through a
/// dyn-erased or type-erased boundary has no way to route on category — it has
/// to downcast to the concrete `ProbeError` (or lose the information entirely).
#[cfg(feature = "std")]
impl zencodec::CategorizedError for ProbeError {
    fn codec_name(&self) -> Option<&'static str> {
        Some("zengif")
    }

    fn category(&self) -> zencodec::ErrorCategory {
        use zencodec::ErrorCategory as C;
        use zencodec::ImageError as Img;
        use zencodec::LimitKind as L;
        use zencodec::ResourceError as Res;
        match self {
            // Too short to even hold a GIF header — reads as an incomplete
            // prefix (truncation) rather than content that is definitively
            // *not* a GIF, since a short-but-legitimate-so-far prefix of a
            // valid GIF looks identical to this case.
            Self::TooShort => C::Image(Img::UnexpectedEof),
            // Signature/version mismatch — this is definitively not (a
            // version of) a GIF, not a truncated one.
            Self::NotGif => C::Image(Img::Malformed),
            // Structurally truncated partway through the block walk.
            Self::Truncated => C::Image(Img::UnexpectedEof),
            // Anti-DoS frame-count cap hit while probing — a resource limit,
            // not a bytes- or request-origin fault.
            Self::TooManyFrames { .. } => C::Resource(Res::Limits(L::Frames)),
            // No `StopReason` payload is tracked by this variant (unlike
            // `GifError::Cancelled`), so this always reads as a plain
            // cancellation rather than distinguishing a timeout.
            Self::Cancelled => C::Stopped(enough::StopReason::Cancelled),
        }
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

    /// 100 MB body packed with trivial Image Descriptors must NOT loop forever.
    /// Without bounding, this would emit ~9 M frames; with the hard cap we
    /// either return `TooManyFrames` or finish in `O(input)` time.
    #[cfg(feature = "std")]
    #[test]
    fn test_probe_descriptor_flood_bounded() {
        let mut data = Vec::with_capacity(13 + 100 * 1024 * 1024);
        data.extend_from_slice(b"GIF89a");
        data.extend_from_slice(&1u16.to_le_bytes()); // width
        data.extend_from_slice(&1u16.to_le_bytes()); // height
        data.push(0x00); // packed: no global CT
        data.push(0); // bg
        data.push(0); // aspect

        // Flood: each iteration writes a degenerate Image Descriptor consuming
        // 12 bytes (1 marker + 9 ID body + 1 LZW min code size + 1 terminator).
        // For test speed, write 200 K descriptors instead of 9 M but still
        // far above any realistic frame count.
        for _ in 0..200_000 {
            data.push(0x2C); // image descriptor marker
            data.extend_from_slice(&[0u8; 9]); // image descriptor body (no local CT, not interlaced)
            data.push(0x02); // LZW min code size
            data.push(0x00); // sub-block terminator
        }
        data.push(0x3B); // trailer

        let started = std::time::Instant::now();

        // 1) Default probe(): hard cap (1 M) is above 200 K, so this completes
        //    successfully but in bounded time.
        let result = probe(&data);
        assert!(result.is_ok(), "probe() must terminate on flood input");
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_secs() < 10,
            "probe() took {elapsed:?} on 200 K-descriptor flood — likely unbounded"
        );

        // 2) probe_with_limits with a tight max_frame_count rejects.
        let strict = Limits::default().max_frame_count(1024);
        let err = probe_with_limits(&data, &strict, &enough::Unstoppable).unwrap_err();
        match err {
            ProbeError::TooManyFrames { count, max } => {
                assert_eq!(max, 1024);
                assert_eq!(count, 1024);
            }
            other => panic!("expected TooManyFrames, got {other:?}"),
        }
    }

    /// Cancellation must be honoured by probe_with_limits.
    #[test]
    fn test_probe_with_limits_cancellation() {
        // Build a moderate-size flood so we can be sure we're inside the loop
        // when the stop trips.
        let mut data = Vec::with_capacity(13 + 50_000 * 12);
        data.extend_from_slice(b"GIF89a");
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0x00);
        data.push(0);
        data.push(0);
        for _ in 0..50_000 {
            data.push(0x2C);
            data.extend_from_slice(&[0u8; 9]);
            data.push(0x02);
            data.push(0x00);
        }
        data.push(0x3B);

        struct AlwaysStop;
        impl Stop for AlwaysStop {
            fn check(&self) -> Result<(), enough::StopReason> {
                Err(enough::StopReason::Cancelled)
            }
        }
        // The stop-poll interval is keyed off frame_count, so frame 0 polls
        // immediately and we get Cancelled before walking the body.
        let err = probe_with_limits(&data, &Limits::none(), &AlwaysStop).unwrap_err();
        assert_eq!(err, ProbeError::Cancelled);
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

    /// Audit finding #7: `ProbeError` must implement `CategorizedError` so a
    /// generic consumer can route on category without downcasting.
    #[cfg(feature = "std")]
    #[test]
    fn probe_error_category_mapping() {
        use zencodec::{
            CategorizedError, ErrorCategory as C, ImageError as Img, LimitKind as L,
            ResourceError as Res,
        };

        assert_eq!(ProbeError::TooShort.codec_name(), Some("zengif"));

        assert_eq!(
            ProbeError::TooShort.category(),
            C::Image(Img::UnexpectedEof)
        );
        assert_eq!(ProbeError::NotGif.category(), C::Image(Img::Malformed));
        assert_eq!(
            ProbeError::Truncated.category(),
            C::Image(Img::UnexpectedEof)
        );
        assert_eq!(
            ProbeError::TooManyFrames { count: 9, max: 4 }.category(),
            C::Resource(Res::Limits(L::Frames))
        );
        assert_eq!(
            ProbeError::Cancelled.category(),
            C::Stopped(enough::StopReason::Cancelled)
        );
    }

    fn test_gif_header_1x1() -> Vec<u8> {
        let mut g = Vec::new();
        g.extend_from_slice(b"GIF89a");
        g.extend_from_slice(&1u16.to_le_bytes());
        g.extend_from_slice(&1u16.to_le_bytes());
        g.extend_from_slice(&[0x80, 0x00, 0x00]); // 2-entry GCT
        g.extend_from_slice(&[255, 0, 0, 0, 0, 255]);
        g
    }

    fn push_1x1_frame(g: &mut Vec<u8>) {
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
        g.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    }

    fn push_netscape(g: &mut Vec<u8>, loops: u16) {
        g.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        g.extend_from_slice(b"NETSCAPE2.0");
        let l = loops.to_le_bytes();
        g.extend_from_slice(&[0x03, 0x01, l[0], l[1], 0x00]);
    }

    /// Issue #24: GCE uses the last 4-byte sub-block up to the 0x00 terminator.
    #[test]
    fn test_probe_gce_multi_subblock_uses_last() {
        // Exact GCE bytes from #24: 3-byte sub-block followed by 4-byte transparent sub-block.
        let mut g = test_gif_header_1x1();
        g.extend_from_slice(&[
            0x21, 0xF9, 0x03, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00,
        ]);
        push_1x1_frame(&mut g);
        g.push(0x3B);
        let p = probe(&g).unwrap();
        assert!(p.has_transparency);
        assert_eq!(p.total_duration_cs, 0);

        // Non-zero delay in the later 4-byte sub-block is also honoured.
        let mut g_delay = test_gif_header_1x1();
        g_delay.extend_from_slice(&[
            0x21, 0xF9, 0x03, 0x00, 0x05, 0x00, 0x04, 0x01, 0x19, 0x00, 0x00, 0x00,
        ]);
        push_1x1_frame(&mut g_delay);
        g_delay.push(0x3B);
        let p_delay = probe(&g_delay).unwrap();
        assert!(p_delay.has_transparency);
        assert_eq!(p_delay.total_duration_cs, 25);

        // Transparent first 4-byte sub-block overridden by opaque last 4-byte sub-block.
        let mut g_opaque = test_gif_header_1x1();
        g_opaque.extend_from_slice(&[
            0x21, 0xF9, 0x04, 0x01, 0x32, 0x00, 0x00, 0x04, 0x00, 0x0A, 0x00, 0x00, 0x00,
        ]);
        push_1x1_frame(&mut g_opaque);
        g_opaque.push(0x3B);
        let p_opaque = probe(&g_opaque).unwrap();
        assert!(!p_opaque.has_transparency);
        assert_eq!(p_opaque.total_duration_cs, 10);

        // 4-byte first sub-block followed by non-4-byte last sub-block must not apply first.
        let mut g_bad_last = test_gif_header_1x1();
        g_bad_last.extend_from_slice(&[
            0x21, 0xF9, 0x04, 0x01, 0x32, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
        ]);
        push_1x1_frame(&mut g_bad_last);
        g_bad_last.push(0x3B);
        let p_bad = probe(&g_bad_last).unwrap();
        assert!(!p_bad.has_transparency);
        assert_eq!(p_bad.total_duration_cs, 0);
    }

    /// Issue #25: NETSCAPE2.0 is only read before the first non-extension block.
    #[test]
    fn test_probe_netscape_only_before_first_non_extension() {
        // NETSCAPE2.0 after the first image descriptor is ignored.
        let mut after_frame = test_gif_header_1x1();
        push_1x1_frame(&mut after_frame);
        push_netscape(&mut after_frame, 5);
        push_1x1_frame(&mut after_frame);
        after_frame.push(0x3B);
        let p = probe(&after_frame).unwrap();
        assert_eq!(p.frame_count, 2);
        assert_eq!(p.repeat, None);

        // NETSCAPE2.0 after an unknown block introducer before the first image is also ignored.
        let mut after_unknown = test_gif_header_1x1();
        after_unknown.extend_from_slice(&[0x05, 0x00]);
        push_netscape(&mut after_unknown, 4);
        push_1x1_frame(&mut after_unknown);
        push_1x1_frame(&mut after_unknown);
        after_unknown.push(0x3B);
        let p_unk = probe(&after_unknown).unwrap();
        assert_eq!(p_unk.frame_count, 2);
        assert_eq!(p_unk.repeat, None);

        // NETSCAPE2.0 after another 0x21 extension before the first image IS read.
        let mut after_comment = test_gif_header_1x1();
        after_comment.extend_from_slice(&[0x21, 0xFE, 0x02, b'h', b'i', 0x00]);
        push_netscape(&mut after_comment, 4);
        push_1x1_frame(&mut after_comment);
        push_1x1_frame(&mut after_comment);
        after_comment.push(0x3B);
        let p_ok = probe(&after_comment).unwrap();
        assert_eq!(p_ok.frame_count, 2);
        assert_eq!(p_ok.repeat, Some(4));
    }

    /// Issue #21: unknown block introducer consumes its sub-block chain like `gif` crate.
    #[test]
    fn test_probe_stray_zero_and_unknown_blocks() {
        // Stray 0x00 between two frames swallows frame 2 as a sub-block length (0x2C = 44).
        let mut stray = test_gif_header_1x1();
        push_1x1_frame(&mut stray);
        stray.push(0x00);
        push_1x1_frame(&mut stray);
        stray.push(0x3B);
        let p = probe(&stray).unwrap();
        assert_eq!(p.frame_count, 1);
        assert!(!p.is_animated);

        // Double 0x00 acts as unknown introducer + immediate 0-length terminator, reaching frame 2.
        let mut double_zero = test_gif_header_1x1();
        push_1x1_frame(&mut double_zero);
        double_zero.extend_from_slice(&[0x00, 0x00]);
        push_1x1_frame(&mut double_zero);
        double_zero.push(0x3B);
        let p_double = probe(&double_zero).unwrap();
        assert_eq!(p_double.frame_count, 2);
        assert!(p_double.is_animated);

        // Unknown introducer with 0x2C inside its sub-block skips past it to the real frame 2.
        let mut wrapped = test_gif_header_1x1();
        push_1x1_frame(&mut wrapped);
        wrapped.extend_from_slice(&[0x05, 0x03, 0x2C, 0xAA, 0xBB, 0x00]);
        push_1x1_frame(&mut wrapped);
        wrapped.push(0x3B);
        let p_wrapped = probe(&wrapped).unwrap();
        assert_eq!(p_wrapped.frame_count, 2);
    }
}
