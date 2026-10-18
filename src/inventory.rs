//! Structural walker behind `DecodeJob::inventory` (crate-private).
//!
//! Lists every byte of a GIF file as a [`Part`] and says whether the zencodec
//! decode path consumes it. No LZW decoding happens here.
//!
//! The block grammar follows the `gif` crate's `StreamingDecoder` (0.14, with
//! `allow_unknown_blocks(true)`), because that is what the decode path runs on,
//! not `detect::probe_with_limits`, which resynchronises differently (it steps
//! over an unknown byte one byte at a time):
//!
//! - `0x21 label sub-blocks…` is an extension; the label is not interpreted
//!   beyond choosing a disposition.
//! - `0x2C` + 9 bytes is an image descriptor, then an optional local colour
//!   table, a LZW minimum code size byte and sub-blocks.
//! - `0x3B` is the trailer; the decoder never reads past it.
//! - Any other introducer byte is consumed alone and the *next* byte is taken
//!   as the first sub-block length of a label-less pseudo extension. The
//!   `gif` crate does exactly that, so an unknown byte can swallow what looks
//!   like the next block's introducer.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use enough::Stop;

use crate::limits::Limits;
use crate::screen::Screen;
use crate::stats::Stats;
use zencodec::ImageFormat;
use zencodec::inventory::{
    Disposition, Inventory, InventoryError, MetadataKind, Part, PartId, PartKind, PartTag,
};

const EXT_INTRODUCER: u8 = 0x21;
const IMAGE_SEPARATOR: u8 = 0x2C;
const TRAILER: u8 = 0x3B;

const LABEL_PLAIN_TEXT: u8 = 0x01;
const LABEL_GRAPHIC_CONTROL: u8 = 0xF9;
const LABEL_COMMENT: u8 = 0xFE;
const LABEL_APPLICATION: u8 = 0xFF;

/// Longest label copied out of the file.
const LABEL_MAX: usize = 64;
/// Longest text preview in a `detail`.
const PREVIEW_MAX: usize = 64;
/// Parts between cancellation polls.
const STOP_POLL_PARTS: usize = 64;

/// The XMP "magic trailer": `01 FF FE … 01 00` followed by the sub-block
/// terminator. A length-walking reader that lands anywhere in it converges on
/// the final `00`.
/// Peak memory per part of the largest inventories, measured in review (RSS of
/// an inventory of `05 00` blocks, including `validate()`), rounded up.
pub(crate) const BYTES_PER_PART: u64 = 400;

const XMP_RAMP_LEN: usize = 258;

pub(crate) enum WalkError {
    /// The part cap was hit (or a part was malformed, which is a walker bug).
    Parts(InventoryError),
    Cancelled(enough::StopReason),
}

impl From<InventoryError> for WalkError {
    fn from(e: InventoryError) -> Self {
        Self::Parts(e)
    }
}

// The probe historically consumes stop errors as parse failures. Latch the
// original reason so the inventory cannot turn a one-shot timeout into success.
struct LatchedStop<'a> {
    inner: &'a dyn Stop,
    reason: std::sync::OnceLock<enough::StopReason>,
}
impl Stop for LatchedStop<'_> {
    fn check(&self) -> Result<(), enough::StopReason> {
        if let Some(reason) = self.reason.get() {
            return Err(*reason);
        }
        self.inner
            .check()
            .map_err(|reason| *self.reason.get_or_init(|| reason))
    }
}

/// Result of walking a run of length-prefixed sub-blocks.
struct SubBlocks {
    /// One past the terminator, or `data.len()` when truncated.
    end: usize,
    terminated: bool,
    /// Data range of the first sub-block.
    first: Option<Range<usize>>,
    /// Data range of the last non-empty sub-block.
    last: Option<Range<usize>>,
    count: usize,
    /// Sum of the sub-block data lengths.
    payload: usize,
    /// The first [`PREVIEW_MAX`] data bytes across all sub-blocks.
    preview: Vec<u8>,
}

/// Walk `len data… len data… 00` starting at `start`, the way the `gif` crate
/// does: every step advances, a sub-block that runs past the end is truncated.
fn sub_blocks(data: &[u8], start: usize) -> SubBlocks {
    let mut out = SubBlocks {
        end: data.len(),
        terminated: false,
        first: None,
        last: None,
        count: 0,
        payload: 0,
        preview: Vec::new(),
    };
    let mut p = start;
    loop {
        let Some(&n) = data.get(p) else {
            out.end = data.len();
            return out;
        };
        if n == 0 {
            out.end = p + 1;
            out.terminated = true;
            return out;
        }
        let from = p + 1;
        let to = from + usize::from(n);
        if to > data.len() {
            out.end = data.len();
            return out;
        }
        if out.first.is_none() {
            out.first = Some(from..to);
        }
        out.last = Some(from..to);
        out.count += 1;
        out.payload += usize::from(n);
        let room = PREVIEW_MAX - out.preview.len().min(PREVIEW_MAX);
        out.preview
            .extend_from_slice(&data[from..to.min(from + room)]);
        p = to;
    }
}

/// Printable-ASCII copy of up to `max` bytes; everything else becomes `.`.
fn printable(bytes: &[u8], max: usize) -> String {
    bytes
        .iter()
        .take(max)
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

fn le16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

/// What the first pass learned about the whole file; the second pass uses it
/// to decide dispositions that depend on later blocks.
#[derive(Default)]
struct Facts {
    /// Graphic Control extensions that are the last one before an image
    /// descriptor the decoder reaches.
    gce_used: BTreeSet<usize>,
    /// NETSCAPE2.0 extensions whose loop count reaches the caller.
    netscape_used: BTreeSet<usize>,
    /// Some frame has no local colour table, so the global one is read.
    gct_used: bool,
    /// The gif crate fails inside its header phase (`Decoder::new` errors), so
    /// `probe()`, `decode()` and the animation decoder all fail.
    header_fatal: bool,
    /// Frames whose LZW data decodes.
    good_frames: usize,
    /// Largest LZW minimum code size among the frames that read the global
    /// table: no index above `2^m - 1` can reach it.
    gct_max_m: u8,
}

struct Walker<'a> {
    data: &'a [u8],
    inv: Inventory,
    stop: &'a dyn Stop,
    pushed: usize,
    /// Pass-1 results (empty during pass 1).
    facts: Facts,
    /// Pass-1 collection.
    found: Facts,
    pending_gce: Option<usize>,
    /// Well-formed NETSCAPE2.0 extensions: (offset, before the first image).
    netscapes: Vec<(usize, bool)>,
    /// First frame the decoder rejects; later frames are never reached.
    fatal: Option<usize>,
    /// Offset of the Graphic Control that caused `fatal`, when it did.
    fatal_gce: Option<usize>,
    /// Image descriptors seen so far.
    images: usize,
    /// The decoder's own limits (`Limits::check_*` decide, nothing is copied).
    limits: &'a Limits,
    /// The logical screen is refused by the dimension limits.
    fatal_screen: bool,
    /// Delay of the Graphic Control waiting for its image, in centiseconds.
    pending_delay: u16,
    /// Running animation duration, in milliseconds.
    elapsed_ms: u64,
    /// The gif crate reads application extensions only until the first block
    /// that is not an extension (image, trailer or unknown introducer).
    header_over: bool,
}

impl Walker<'_> {
    fn push(&mut self, part: Part) -> Result<PartId, WalkError> {
        self.pushed += 1;
        if self.pushed.is_multiple_of(STOP_POLL_PARTS) {
            self.stop.check().map_err(WalkError::Cancelled)?;
        }
        Ok(self.inv.push(None, part)?)
    }

    fn push_child(&mut self, parent: PartId, part: Part) -> Result<PartId, WalkError> {
        self.pushed += 1;
        Ok(self.inv.push(Some(parent), part)?)
    }

    fn block(
        &mut self,
        range: Range<usize>,
        tag: PartTag,
        disposition: Disposition,
        label: Option<String>,
        detail: Option<String>,
    ) -> Result<PartId, WalkError> {
        let mut part = Part::new(
            PartKind::Block,
            tag,
            range.start as u64..range.end as u64,
            disposition,
        );
        if let Some(l) = label {
            part = part.with_label(l);
        }
        if let Some(d) = detail {
            part = part.with_detail(d);
        }
        self.push(part)
    }
}

fn new_walker<'a>(
    data: &'a [u8],
    stop: &'a dyn Stop,
    max_parts: u32,
    limits: &'a Limits,
    facts: Facts,
) -> Walker<'a> {
    Walker {
        data,
        inv: Inventory::new(ImageFormat::Gif, data.len() as u64).with_max_parts(max_parts),
        stop,
        pushed: 0,
        facts,
        found: Facts::default(),
        pending_gce: None,
        netscapes: Vec::new(),
        fatal: None,
        fatal_gce: None,
        limits,
        fatal_screen: false,
        pending_delay: 0,
        elapsed_ms: 0,
        images: 0,
        header_over: false,
    }
}

/// Inventory `data` as a GIF file, with at most `max_parts` parts, under the
/// decoder's `limits`.
///
/// Two passes: the first learns which Graphic Control and NETSCAPE2.0
/// extensions are overwritten or orphaned, whether the global colour table is
/// read and how far its indices reach; the second emits the parts with those
/// facts.
pub(crate) fn walk(
    data: &[u8],
    stop: &dyn Stop,
    max_parts: u32,
    limits: &Limits,
) -> Result<Inventory, WalkError> {
    let latched = LatchedStop {
        inner: stop,
        reason: std::sync::OnceLock::new(),
    };
    let stop: &dyn Stop = &latched;
    stop.check().map_err(WalkError::Cancelled)?;
    if data.is_empty() {
        return Ok(new_walker(data, stop, max_parts, limits, Facts::default()).inv);
    }
    // Pass 1 lives in its own scope so its parts are freed before pass 2.
    let (mut facts, netscapes) = {
        let mut first = new_walker(data, stop, max_parts, limits, Facts::default());
        walk_inner(&mut first)?;
        (first.found, first.netscapes)
    };
    // Which loop count reaches the caller, and only if `Decoder::new`
    // succeeds (otherwise `probe()`, `decode()` and the animation decoder all
    // fail):
    // - the animation decoder reports the last one before the header phase
    //   ends, once a frame decodes;
    // - `probe()` reports what detect.rs kept, the same call `probe()` makes,
    //   and only for an animation.
    if !facts.header_fatal {
        if facts.good_frames > 0 {
            facts.netscape_used.extend(
                netscapes
                    .iter()
                    .filter(|(_, early)| *early)
                    .map(|(at, _)| *at)
                    .next_back(),
            );
        }
        if let Ok((p, Some(at))) = crate::detect::probe_located(data, limits, stop)
            && p.is_animated
        {
            facts.netscape_used.insert(at);
        }
    }

    stop.check().map_err(WalkError::Cancelled)?;
    let mut w = new_walker(data, stop, max_parts, limits, facts);
    walk_inner(&mut w)?;
    w.inv.fill_gaps(None, Disposition::Trailing)?;
    Ok(w.inv)
}

fn malformed_rest(w: &mut Walker<'_>, from: usize, why: &str) -> Result<(), WalkError> {
    if from < w.data.len() {
        w.push(
            Part::new(
                PartKind::Gap,
                PartTag::None,
                from as u64..w.data.len() as u64,
                Disposition::Malformed,
            )
            .with_detail(why),
        )?;
    }
    Ok(())
}

fn walk_inner(w: &mut Walker<'_>) -> Result<(), WalkError> {
    let data = w.data;
    let len = data.len();

    // Header (6) + logical screen descriptor (7). The decode path reads these
    // 13 bytes up front and refuses anything but GIF87a / GIF89a.
    let version = data.get(..6).filter(|v| *v == b"GIF87a" || *v == b"GIF89a");
    let Some(version) = version else {
        return malformed_rest(w, 0, "not a GIF87a/GIF89a header; the decoder rejects it");
    };
    let version = printable(version, 6);
    if len < 13 {
        w.push(
            Part::new(
                PartKind::Header,
                PartTag::None,
                0..len as u64,
                Disposition::Malformed,
            )
            .with_label(version)
            .with_detail("truncated before the end of the logical screen descriptor"),
        )?;
        return Ok(());
    }
    let (sw, sh) = (le16(data, 6), le16(data, 8));
    let packed = data[10];
    // `Decoder::new` runs the dimension limits (zero sizes always fail) before
    // anything else is read: every zencodec path fails, nothing after the
    // header is consumed.
    let dims_err = w.limits.check_dimensions(sw, sh).is_err();
    // `Decoder::new` then charges the RGBA canvas (`ScreenBuilder::build`) and
    // the indexed pixel buffer (`alloc_zeroed`, one byte per pixel) against
    // `max_memory`; charge the same amounts on a fresh `Stats`, allocating
    // nothing.
    let mem_err = !dims_err && {
        let stats = Stats::new();
        stats
            .try_alloc(Screen::canvas_bytes_for(sw, sh), w.limits)
            .and_then(|()| stats.try_alloc(usize::from(sw) * usize::from(sh), w.limits))
            .is_err()
    };
    let screen_err = dims_err || mem_err;
    if screen_err {
        w.fatal_screen = true;
        w.fatal = Some(0);
        w.found.header_fatal = true;
    }
    let mut header_detail = alloc::format!(
        "screen {sw}x{sh}, background index {}, pixel aspect byte {}; the gif crate \
         parses the last two, zencodec output does not carry them",
        data[11],
        data[12]
    );
    if dims_err {
        header_detail.push_str(
            "; the decoder's dimension limits reject this screen, so every zencodec path fails \
             and nothing after the header is read",
        );
    } else if mem_err {
        header_detail.push_str(
            "; the canvas and indexed buffer exceed the decoder's memory limit, so every zencodec \
             path fails and nothing after the header is read",
        );
    }
    if w.limits.max_decompression_ratio.is_some() {
        header_detail.push_str(
            "; a frame may also be refused by the decompression-ratio limit (it depends on how \
             much the reader has pulled ahead)",
        );
    }
    w.push(
        Part::new(
            PartKind::Header,
            PartTag::None,
            0..13,
            Disposition::Structure,
        )
        .with_label(version)
        .with_detail(header_detail),
    )?;
    let mut pos = 13usize;

    if packed & 0x80 != 0 && pos < len {
        let size = 3usize << (usize::from(packed & 7) + 1);
        let end = pos + size;
        if end > len {
            w.block(
                pos..len,
                PartTag::None,
                Disposition::Malformed,
                Some("global colour table".into()),
                Some("truncated".into()),
            )?;
            return Ok(());
        }
        // Read only by frames without a local table (pass 1 finds out), and
        // only up to the largest index their LZW code size can produce.
        let used = w.facts.gct_used;
        let reach = 1usize << w.facts.gct_max_m.min(11);
        colour_table(
            w,
            pos..end,
            "global colour table",
            used,
            reach,
            if used {
                alloc::format!("{} entries", size / 3)
            } else {
                alloc::format!(
                    "{} entries; every frame has a local colour table (or no frame decodes), \
                     nothing reads it",
                    size / 3
                )
            },
        )?;
        pos = end;
    }
    let has_gct = packed & 0x80 != 0;

    let mut frames = 0usize;
    while pos < len {
        if data[pos] != EXT_INTRODUCER {
            w.header_over = true;
        }
        match data[pos] {
            TRAILER => {
                w.block(
                    pos..pos + 1,
                    PartTag::Code(u32::from(TRAILER)),
                    Disposition::Structure,
                    Some("trailer".into()),
                    None,
                )?;
                pos += 1;
                if pos < len {
                    w.push(
                        Part::new(
                            PartKind::Trailer,
                            PartTag::None,
                            pos as u64..len as u64,
                            Disposition::Trailing,
                        )
                        .with_detail(alloc::format!(
                            "{} bytes after the GIF trailer; the decoder stops at the trailer",
                            len - pos
                        )),
                    )?;
                }
                return Ok(());
            }
            IMAGE_SEPARATOR => {
                frames += 1;
                match image(w, pos, frames, (sw, sh), has_gct)? {
                    Some(next) => pos = next,
                    None => return Ok(()),
                }
            }
            EXT_INTRODUCER => pos = extension(w, pos)?,
            other => pos = unknown_block(w, pos, other)?,
        }
    }
    Ok(())
}

/// A colour table: a read table is split into the entries an index can reach
/// (`reach` = `2^m` for LZW minimum code size `m`) and the rest, which no index
/// can select. The native palette APIs (`Metadata::global_palette`,
/// `ComposedFrame::palette`) carry the whole table; zencodec output does not.
fn colour_table(
    w: &mut Walker<'_>,
    range: Range<usize>,
    label: &str,
    read: bool,
    reach: usize,
    detail: String,
) -> Result<(), WalkError> {
    let entries = range.len() / 3;
    let disposition = if read {
        Disposition::ImageData
    } else if label == "global colour table" {
        Disposition::Dropped
    } else {
        Disposition::Skipped
    };
    if !read || reach >= entries {
        w.block(
            range,
            PartTag::None,
            disposition,
            Some(label.into()),
            Some(detail),
        )?;
        return Ok(());
    }
    let cut = range.start + reach * 3;
    let parent = w.push(
        Part::new(
            PartKind::Block,
            PartTag::None,
            range.start as u64..range.end as u64,
            disposition,
        )
        .with_label(label.to_owned())
        .with_body(range.start as u64..range.end as u64)
        .with_detail(detail),
    )?;
    w.push_child(
        parent,
        Part::new(
            PartKind::Block,
            PartTag::None,
            range.start as u64..cut as u64,
            Disposition::ImageData,
        )
        .with_label(label.to_owned())
        .with_detail(alloc::format!(
            "entries 0..{reach}, reachable by the frames' LZW codes"
        )),
    )?;
    w.push_child(
        parent,
        Part::new(
            PartKind::Block,
            PartTag::None,
            cut as u64..range.end as u64,
            Disposition::Dropped,
        )
        .with_label("unreachable palette entries")
        .with_detail(alloc::format!(
            "entries {reach}..{entries}: no index can reach them (LZW minimum code size limits \
             indices below {reach}); native palette APIs still carry the whole table"
        )),
    )?;
    Ok(())
}

/// How far the LZW decoder reads.
enum Lzw {
    /// The decoder stops after this offset (end code or last pixel).
    Ended(usize),
    /// The sub-blocks end before the pixel count: the decoder errors
    /// ("image truncated"), whether or not an end code came first.
    Short,
    /// An invalid code: the decoder errors.
    Bad,
}

/// Follow the LZW code stream of the sub-blocks at `start` without producing
/// pixels, to find the byte where the decoder stops reading: the end code, or
/// the code that completes `total` pixels. Tracks only the code width, the
/// dictionary size and a 4096-entry table of string lengths (about 8 KB), and
/// runs in O(compressed bytes).
fn lzw_extent(data: &[u8], start: usize, min_code: u8, total: u64) -> Lzw {
    let clear = 1u16 << min_code;
    let eoi = clear + 1;
    let first_free = eoi + 1;
    let mut lens = [0u16; 4096];
    let len_of = |lens: &[u16; 4096], c: u16| if c < clear { 1 } else { lens[usize::from(c)] };
    let mut size = u32::from(min_code) + 1;
    let mut next = first_free;
    let mut prev: Option<u16> = None;
    let mut pixels = 0u64;
    // Offset after the byte that completed the pixel count. The end code that
    // normally follows is read with the data (an encoder always writes it).
    let mut full_at: Option<usize> = None;
    let (mut acc, mut nbits) = (0u32, 0u32);
    let mut p = start;
    while let Some(&n) = data.get(p) {
        if n == 0 {
            break;
        }
        let end = p + 1 + usize::from(n);
        for o in p + 1..end {
            let Some(&b) = data.get(o) else {
                return Lzw::Short;
            };
            acc |= u32::from(b) << nbits;
            nbits += 8;
            while nbits >= size {
                let code = (acc & ((1 << size) - 1)) as u16;
                acc >>= size;
                nbits -= size;
                if full_at.is_some() {
                    // The decoder does not stop at the last pixel: it reads and
                    // validates the next code if its bits are in the current chunk,
                    // so an invalid one fails the frame even though every pixel
                    // was produced. Whether it reads it depends on chunking, so
                    // assume it does.
                    if code != eoi
                        && code != clear
                        && (code > next || (code == next && next >= 4096))
                    {
                        return Lzw::Bad;
                    }
                    return Lzw::Ended(o + 1);
                }
                if code == clear {
                    size = u32::from(min_code) + 1;
                    next = first_free;
                    prev = None;
                    continue;
                }
                if code == eoi {
                    return Lzw::Short;
                }
                match prev {
                    None => {
                        if code >= clear {
                            return Lzw::Bad;
                        }
                        pixels += 1;
                    }
                    Some(pv) => {
                        if code > next || (code == next && next >= 4096) {
                            return Lzw::Bad;
                        }
                        let new_len = len_of(&lens, pv).saturating_add(1);
                        pixels += u64::from(if code < next {
                            len_of(&lens, code)
                        } else {
                            new_len
                        });
                        if next < 4096 {
                            lens[usize::from(next)] = new_len;
                            next += 1;
                            // weezl bumps once the entry just derived reaches `max_code`
                            // (`(1 << size) - 1`); at minimum code size 1 that is one
                            // entry earlier than `next == 1 << size`.
                            if u32::from(next) >= 1 << size && size < 12 {
                                size += 1;
                            }
                        }
                    }
                }
                prev = Some(code);
                if pixels >= total {
                    full_at = Some(o + 1);
                }
            }
        }
        p = end;
    }
    full_at.map_or(Lzw::Short, Lzw::Ended)
}

/// Image descriptor, optional local colour table, image data. Returns the next
/// offset, or `None` when the file ended inside it.
fn image(
    w: &mut Walker<'_>,
    pos: usize,
    frame: usize,
    screen: (u16, u16),
    has_gct: bool,
) -> Result<Option<usize>, WalkError> {
    let data = w.data;
    let len = data.len();
    let desc_end = pos + 10;
    if desc_end > len {
        w.block(
            pos..len,
            PartTag::Code(u32::from(IMAGE_SEPARATOR)),
            Disposition::Malformed,
            Some("image descriptor".into()),
            Some("truncated".into()),
        )?;
        return Ok(None);
    }
    let (left, top) = (le16(data, pos + 1), le16(data, pos + 3));
    let (fw, fh) = (le16(data, pos + 5), le16(data, pos + 7));
    let flags = data[pos + 9];
    let has_lct = flags & 0x80 != 0;
    w.images += 1;

    // Frames after one the decoder rejects are never reached.
    let after_fatal = w.fatal;
    // The decode path enables `check_frame_consistency` and rejects empty
    // frames (decode/mod.rs `next_frame`, `ZeroDimensionFrame`); the gif crate
    // fails a frame with no colour table at all (reader/mod.rs).
    let problem = if after_fatal.is_some() {
        None
    } else if w.limits.check_frame_count(frame as u64 - 1).is_err() {
        Some("frame count limit reached; the decoder stops before this frame")
    } else if fw == 0 || fh == 0 {
        Some("zero-sized frame; the decoder rejects it")
    } else if u32::from(left) + u32::from(fw) > u32::from(screen.0)
        || u32::from(top) + u32::from(fh) > u32::from(screen.1)
    {
        Some("frame outside the logical screen; the decoder rejects it")
    } else if !has_lct && !has_gct {
        Some("no colour table for this frame; the decoder rejects it")
    } else {
        None
    };
    if problem.is_some() && after_fatal.is_none() {
        w.fatal = Some(frame);
    }
    let reachable = after_fatal.is_none() && problem.is_none();
    // The global table counts as read only by a frame the decoder gets to
    // decode; a fatal LZW error below withdraws it again.
    let mut reads_gct = reachable && !has_lct;
    let pending = w.pending_gce.take();
    let delay_cs = if pending.is_some() {
        w.pending_delay
    } else {
        0
    };
    w.pending_delay = 0;
    if reachable && let Some(g) = pending {
        w.found.gce_used.insert(g);
    }
    // The animation duration limit is checked after each decoded frame: the
    // frame that crosses it is read but reported as an error.
    let mut dur_fail = false;
    if reachable {
        w.elapsed_ms = w.elapsed_ms.saturating_add(u64::from(delay_cs) * 10);
        dur_fail = w.limits.check_animation_duration(w.elapsed_ms).is_err();
    }
    let fatal_gce = w.fatal_gce;
    let fatal_screen = w.fatal_screen;
    let skipped_note = after_fatal.map(|n| {
        if fatal_screen {
            return String::from(
                "; the decoder rejects the logical screen (dimension or memory limits), so this is never read",
            );
        }
        if let Some(g) = fatal_gce {
            return alloc::format!(
                "; after the Graphic Control at offset {g}, which the decoder rejects, so this is \
                 never read (probe() still counts the frame and reads its interlace flag)"
            );
        }
        alloc::format!("; after frame {n}, which the decoder rejects, so this is never read (probe() still counts \
             the frame and reads its interlace flag)")
    });

    let mut detail = alloc::format!(
        "frame {frame}: left {left} top {top} {fw}x{fh}{}",
        if flags & 0x40 != 0 {
            ", interlaced"
        } else {
            ""
        }
    );
    if frame > 1 {
        detail.push_str("; reached by the animation decoder only (decode() reads frame 1)");
    }
    if let Some(p) = problem {
        detail.push_str("; ");
        detail.push_str(p);
    }
    if let Some(n) = &skipped_note {
        detail.push_str(n);
    }
    w.block(
        pos..desc_end,
        PartTag::Code(u32::from(IMAGE_SEPARATOR)),
        if problem.is_some() {
            Disposition::Malformed
        } else if after_fatal.is_some() {
            Disposition::Skipped
        } else {
            Disposition::Structure
        },
        Some("image descriptor".into()),
        Some(detail),
    )?;

    let mut at = desc_end;
    if has_lct && at < len {
        let size = 3usize << (usize::from(flags & 7) + 1);
        let end = at + size;
        if end > len {
            w.block(
                at..len,
                PartTag::None,
                Disposition::Malformed,
                Some("local colour table".into()),
                Some("truncated".into()),
            )?;
            return Ok(None);
        }
        // The table is read only when the frame's data decodes (the walk does
        // not depend on the table, so run it first), and only up to the
        // largest index the frame's LZW code size can produce.
        let m_lct = data.get(end).copied().unwrap_or(0);
        let lct_read = reachable
            && !dur_fail
            && (1..=11).contains(&m_lct)
            && sub_blocks(data, end + 1).terminated
            && matches!(
                lzw_extent(data, end + 1, m_lct, u64::from(fw) * u64::from(fh)),
                Lzw::Ended(_)
            );
        colour_table(
            w,
            at..end,
            "local colour table",
            lct_read,
            1usize << m_lct.min(11),
            alloc::format!(
                "frame {frame}, {} entries{}",
                size / 3,
                if reachable && !lct_read {
                    "; the frame's image data fails to decode, so the table is never used"
                } else {
                    ""
                }
            ),
        )?;
        at = end;
    }

    let Some(&min_code) = data.get(at) else {
        return Ok(None);
    };
    let sb = sub_blocks(data, at + 1);
    // Known limitation: interlaced frames are decoded row by row, which drives
    // weezl through a call pattern this walk does not model; on rare streams the
    // decoder then reports "image truncated" where the walk sees a complete frame.
    let interlaced_note = if flags & 0x40 != 0 {
        "; interlaced: the gif crate's row-by-row reads can fail a stream this walk accepts"
    } else {
        ""
    };
    let head = |note: &str| {
        alloc::format!(
            "frame {frame}: LZW minimum code size {min_code}, {} sub-blocks, {} bytes{note}{interlaced_note}",
            sb.count,
            sb.payload
        )
    };
    // `StreamingDecoder::check_code_size`: 1..=11.
    if !sb.terminated {
        w.block(
            at..sb.end,
            PartTag::None,
            Disposition::Malformed,
            Some("image data".into()),
            Some(head("; truncated image data")),
        )?;
        return Ok(Some(sb.end));
    }
    if !reachable {
        let note = skipped_note.unwrap_or_default();
        w.block(
            at..sb.end,
            PartTag::None,
            Disposition::Skipped,
            Some("image data".into()),
            Some(head(&note)),
        )?;
        return Ok(Some(sb.end));
    }
    if !(1..=11).contains(&min_code) {
        w.fatal = Some(frame);
        w.block(
            at..sb.end,
            PartTag::None,
            Disposition::Malformed,
            Some("image data".into()),
            Some(head(
                "; invalid LZW minimum code size, the decoder rejects it",
            )),
        )?;
        return Ok(Some(sb.end));
    }
    let extent = lzw_extent(data, at + 1, min_code, u64::from(fw) * u64::from(fh));
    let fail_why = match extent {
        Lzw::Bad => Some("; invalid LZW code, the decoder errors"),
        Lzw::Short => {
            Some("; the LZW data ends before the pixel count, the decoder errors (image truncated)")
        }
        Lzw::Ended(_) if dur_fail => Some(
            "; the animation duration limit is exceeded by this frame, the decoder reports an \
             error instead of it",
        ),
        Lzw::Ended(_) => None,
    };
    match extent {
        _ if fail_why.is_some() => {
            w.fatal = Some(frame);
            reads_gct = false;
            w.block(
                at..sb.end,
                PartTag::None,
                Disposition::Malformed,
                Some("image data".into()),
                Some(head(fail_why.unwrap_or_default())),
            )?;
        }
        // Only the terminator is left after the last byte read.
        Lzw::Ended(cut) if cut + 1 < sb.end => {
            w.found.good_frames += 1;
            w.block(
                at..cut,
                PartTag::None,
                Disposition::ImageData,
                Some("image data".into()),
                Some(head("")),
            )?;
            w.block(
                cut..sb.end,
                PartTag::None,
                Disposition::Dropped,
                Some("image data tail".into()),
                Some(alloc::format!(
                    "frame {frame}: {} bytes after the LZW end code or last pixel; the decoder \
                     reads at most one more code there (it must be valid), nothing else",
                    sb.end - cut
                )),
            )?;
        }
        Lzw::Ended(_) => {
            w.found.good_frames += 1;
            w.block(
                at..sb.end,
                PartTag::None,
                Disposition::ImageData,
                Some("image data".into()),
                Some(head("")),
            )?;
        }
        Lzw::Bad | Lzw::Short => {}
    }
    if reads_gct {
        w.found.gct_used = true;
        w.found.gct_max_m = w.found.gct_max_m.max(min_code);
    }
    Ok(Some(sb.end))
}

/// An introducer byte the `gif` crate does not know. It consumes the byte,
/// then reads the next byte as the first sub-block length.
///
/// `detect.rs` (which feeds `probe()`) instead steps over the byte and goes on
/// looking for blocks, so when the swallowed bytes contain a block introducer
/// or trailer byte the two walkers can read different things there: such a
/// block is `Malformed`, not `Unknown`.
fn unknown_block(w: &mut Walker<'_>, pos: usize, introducer: u8) -> Result<usize, WalkError> {
    let sb = sub_blocks(w.data, pos + 1);
    let probe_reads = w.data[pos + 1..sb.end]
        .iter()
        .any(|b| matches!(*b, EXT_INTRODUCER | IMAGE_SEPARATOR | TRAILER));
    let (disposition, note) = if !sb.terminated {
        (Disposition::Malformed, "; truncated")
    } else if probe_reads {
        (
            Disposition::Malformed,
            "; probe() (detect.rs) steps one byte and may read blocks (loop count, transparency, \
             frames) from these bytes that the decoder skips",
        )
    } else {
        (Disposition::Unknown, "")
    };
    w.block(
        pos..sb.end,
        PartTag::Code(u32::from(introducer)),
        disposition,
        None,
        Some(alloc::format!(
            "unknown block 0x{introducer:02X}: the gif crate reads the next bytes as {} \
             sub-blocks, {} bytes{note}",
            sb.count,
            sb.payload
        )),
    )?;
    Ok(sb.end)
}

/// Whether `data` is a NETSCAPE-style loop sub-block: `01 lo hi`.
fn loop_count(data: &[u8], sub: &Option<Range<usize>>) -> Option<u16> {
    let r = sub.clone()?;
    match data.get(r)? {
        [1, lo, hi] => Some(u16::from_le_bytes([*lo, *hi])),
        _ => None,
    }
}

/// The second sub-block of an application extension, if there is one.
fn second_sub_block(data: &[u8], first: &Range<usize>) -> Option<Range<usize>> {
    let n = usize::from(*data.get(first.end)?);
    (n != 0 && first.end + 1 + n <= data.len()).then(|| first.end + 1..first.end + 1 + n)
}

/// Split a consumed extension whose other sub-blocks the decoder never reads:
/// the sub-block(s) in `used` carry `disposition`, the rest is `Dropped`.
struct Split<'a> {
    ext: Range<usize>,
    tag: PartTag,
    label: &'a str,
    used: Range<usize>,
    disposition: Disposition,
    detail: String,
    /// A byte inside `used` the decoder does not read, with the reason.
    carve: Option<(usize, &'static str)>,
}

fn split_extension(w: &mut Walker<'_>, sp: Split<'_>) -> Result<(), WalkError> {
    let Split {
        ext,
        tag,
        label,
        used,
        disposition,
        detail,
        carve,
    } = sp;
    let (pos, end) = (ext.start, ext.end);
    let body = pos + 2..end;
    let parent = w.push(
        Part::new(
            PartKind::Block,
            tag,
            pos as u64..end as u64,
            Disposition::Structure,
        )
        .with_label(label.to_owned())
        .with_body(body.start as u64..body.end as u64)
        .with_detail("bytes in this extension that the decoder never reads are listed separately"),
    )?;
    let child = |range: Range<usize>, d: Disposition, label: &str, detail: String| {
        Part::new(
            PartKind::Block,
            PartTag::None,
            range.start as u64..range.end as u64,
            d,
        )
        .with_label(label.to_owned())
        .with_detail(detail)
    };
    if used.start > body.start {
        w.push_child(
            parent,
            child(
                body.start..used.start,
                Disposition::Dropped,
                "unread sub-blocks",
                alloc::format!(
                    "{} bytes before the sub-block the decoder reads; probe() (detect.rs) reads the first \
                     Graphic Control sub-block, so it can take a transparency flag or delay from here",
                    used.start - body.start
                ),
            ),
        )?;
    }
    match carve {
        None => {
            w.push_child(parent, child(used.clone(), disposition, label, detail))?;
        }
        Some((at, why)) => {
            if at > used.start {
                w.push_child(
                    parent,
                    child(used.start..at, disposition, label, detail.clone()),
                )?;
            }
            w.push_child(
                parent,
                child(at..at + 1, Disposition::Dropped, "unread byte", why.into()),
            )?;
            if at + 1 < used.end {
                w.push_child(parent, child(at + 1..used.end, disposition, label, detail))?;
            }
        }
    }
    if used.end + 1 < end {
        w.push_child(
            parent,
            child(
                used.end..end - 1,
                Disposition::Dropped,
                "unread sub-blocks",
                alloc::format!(
                    "{} bytes after the sub-block the decoder reads",
                    end - 1 - used.end
                ),
            ),
        )?;
    }
    w.push_child(
        parent,
        child(
            end - 1..end,
            Disposition::Structure,
            "terminator",
            String::new(),
        ),
    )?;
    Ok(())
}

/// A Graphic Control extension applies to the next image descriptor; the gif
/// crate keeps only the last sub-block, and it must be 4 bytes.
fn graphic_control(w: &mut Walker<'_>, pos: usize, sb: &SubBlocks) -> Result<usize, WalkError> {
    let data = w.data;
    let end = sb.end;
    let tag = PartTag::Code(u32::from(LABEL_GRAPHIC_CONTROL));
    let label = "Graphic Control";
    let ok = sb.terminated && sb.last.as_ref().is_some_and(|r| r.len() == 4);
    let Some(r) = sb.last.clone().filter(|_| ok) else {
        // gif crate `read_control_extension` errors: later frames never decode.
        if !w.header_over {
            w.found.header_fatal = true;
        }
        if w.fatal.is_none() {
            w.fatal = Some(w.images + 1);
            w.fatal_gce = Some(pos);
        }
        let trunc = if sb.terminated { "" } else { "; truncated" };
        w.block(
            pos..end,
            tag,
            Disposition::Malformed,
            Some(label.into()),
            Some(alloc::format!(
                "control extension with a wrong length{trunc}; the decoder rejects it"
            )),
        )?;
        return Ok(end);
    };
    let b = &data[r.clone()];
    let desc = alloc::format!(
        "delay {} cs, disposal {}, transparent index {}{}",
        u16::from_le_bytes([b[1], b[2]]),
        (b[0] >> 2) & 7,
        if b[0] & 1 != 0 {
            alloc::format!("{}", b[3])
        } else {
            "none".into()
        },
        if b[0] & 2 != 0 {
            ", user input flag"
        } else {
            ""
        }
    );
    w.pending_gce = Some(pos);
    w.pending_delay = u16::from_le_bytes([b[1], b[2]]);
    if !w.facts.gce_used.contains(&pos) {
        w.block(
            pos..end,
            tag,
            Disposition::Dropped,
            Some(label.into()),
            Some(alloc::format!(
                "{desc}; applied to no frame (overwritten by a later Graphic Control, no image \
                 follows, or the decoder never reaches it); probe() still reads a Graphic Control that precedes \
                 an image descriptor it counts, including frames after a rejected one"
            )),
        )?;
        return Ok(end);
    }
    // Frame delay, disposal and the transparent index feed `AnimationFrame`
    // timing and the composited alpha.
    let anim = Disposition::Metadata(MetadataKind::Animation);
    // With the transparency flag clear the index byte is never read.
    let carve = (b[0] & 1 == 0).then_some((
        r.start + 3,
        "transparent index: unread because the transparency flag is clear",
    ));
    if sb.count == 1 && carve.is_none() {
        w.block(pos..end, tag, anim, Some(label.into()), Some(desc))?;
    } else {
        split_extension(
            w,
            Split {
                ext: pos..end,
                tag,
                label,
                used: r.start - 1..r.end,
                disposition: anim,
                detail: desc,
                carve,
            },
        )?;
    }
    Ok(end)
}

fn extension(w: &mut Walker<'_>, pos: usize) -> Result<usize, WalkError> {
    let data = w.data;
    let len = data.len();
    let Some(&label) = data.get(pos + 1) else {
        w.block(
            pos..len,
            PartTag::Code(u32::from(EXT_INTRODUCER)),
            Disposition::Malformed,
            None,
            Some("extension introducer without a label".into()),
        )?;
        return Ok(len);
    };
    let sb = sub_blocks(data, pos + 2);
    let range = pos..sb.end;
    let tag = PartTag::Code(u32::from(label));
    let trunc = if sb.terminated { "" } else { "; truncated" };

    // (label, disposition, detail)
    let (lbl, disposition, detail): (Option<String>, Disposition, String) = match label {
        LABEL_GRAPHIC_CONTROL => return graphic_control(w, pos, &sb),
        LABEL_COMMENT => (
            Some("Comment".into()),
            Disposition::Skipped,
            alloc::format!(
                "{} bytes in {} sub-blocks, starts {:?}; zengif never surfaces comments \
                 (Metadata::comments stays empty){trunc}",
                sb.payload,
                sb.count,
                printable(&sb.preview, PREVIEW_MAX)
            ),
        ),
        LABEL_PLAIN_TEXT => (
            Some("Plain Text".into()),
            Disposition::Skipped,
            alloc::format!(
                "{} bytes in {} sub-blocks; not rendered or surfaced{trunc}",
                sb.payload,
                sb.count
            ),
        ),
        LABEL_APPLICATION => return application(w, pos, sb),
        other => (
            None,
            Disposition::Unknown,
            alloc::format!(
                "unknown extension label 0x{other:02X}, {} bytes in {} sub-blocks{trunc}",
                sb.payload,
                sb.count
            ),
        ),
    };
    let disposition = if sb.terminated {
        disposition
    } else {
        Disposition::Malformed
    };
    w.block(range, tag, disposition, lbl, Some(detail))?;
    Ok(sb.end)
}

fn application(w: &mut Walker<'_>, pos: usize, sb: SubBlocks) -> Result<usize, WalkError> {
    let data = w.data;
    let tag = PartTag::Code(u32::from(LABEL_APPLICATION));
    let end = sb.end;
    let trunc = if sb.terminated { "" } else { "; truncated" };
    let ident: Option<&[u8]> = sb.first.as_ref().map(|r| &data[r.clone()]);
    let label = ident.map(|i| printable(i, LABEL_MAX));
    let second = sb
        .first
        .as_ref()
        .and_then(|first| second_sub_block(data, first));

    if ident == Some(b"NETSCAPE2.0")
        && sb.terminated
        && let (Some(n), Some(sec)) = (loop_count(data, &second), second.clone())
    {
        return netscape(w, pos, &sb, n, sec, label);
    }

    let (disposition, detail): (Disposition, String) = match ident {
        Some(b"NETSCAPE2.0") => (
            Disposition::Dropped,
            "NETSCAPE2.0 without a loop-count sub-block".into(),
        ),
        // Neither the gif crate nor detect.rs reads it, exactly like any other
        // unrecognised application extension; browsers do read a loop count.
        Some(b"ANIMEXTS1.0") => (
            Disposition::Unknown,
            alloc::format!(
                "{}; browsers read it as a loop count, the gif crate and detect.rs ignore it",
                match loop_count(data, &second) {
                    Some(n) => alloc::format!("loop count {n}"),
                    None => "no loop-count sub-block".into(),
                }
            ),
        ),
        Some(b"ICCRGBG1012") => (
            Disposition::Skipped,
            alloc::format!(
                "ICC profile, {} bytes; the gif crate buffers it when it precedes the first non-extension block; zengif never surfaces it{trunc}",
                sb.payload.saturating_sub(11)
            ),
        ),
        Some(b"XMP DataXMP") => return xmp(w, pos, sb),
        Some(_) => (
            Disposition::Unknown,
            alloc::format!(
                "unrecognised application extension, {} bytes in {} sub-blocks{trunc}",
                sb.payload,
                sb.count
            ),
        ),
        None => (
            Disposition::Unknown,
            alloc::format!("application extension without an identifier{trunc}"),
        ),
    };
    let disposition = if sb.terminated {
        disposition
    } else {
        Disposition::Malformed
    };
    w.block(pos..end, tag, disposition, label, Some(detail))?;
    Ok(end)
}

/// NETSCAPE2.0 with a loop count. `probe()` reads the last one in the file;
/// the gif crate (animation decoder, native API) the last one before the first
/// image. Any other one, and any extra sub-block, reaches nothing.
fn netscape(
    w: &mut Walker<'_>,
    pos: usize,
    sb: &SubBlocks,
    n: u16,
    second: Range<usize>,
    label: Option<String>,
) -> Result<usize, WalkError> {
    let end = sb.end;
    let tag = PartTag::Code(u32::from(LABEL_APPLICATION));
    let label = label.unwrap_or_default();
    w.netscapes.push((pos, !w.header_over));
    if !w.facts.netscape_used.contains(&pos) {
        w.block(
            pos..end,
            tag,
            Disposition::Dropped,
            Some(label),
            Some(alloc::format!(
                "loop count {n}; nothing reports it (overwritten by a later NETSCAPE2.0, the decoder \
                 fails before it applies, or no frame decodes, or probe() sees a single frame)"
            )),
        )?;
        return Ok(end);
    }
    let detail = alloc::format!(
        "loop count {n} (0 = forever); probe() reports the last one in the file, the animation \
         decoder only one before the first non-extension block"
    );
    let anim = Disposition::Metadata(MetadataKind::Animation);
    if second.end + 1 == end {
        w.block(pos..end, tag, anim, Some(label), Some(detail))?;
    } else {
        // Sub-blocks after the loop count are never read.
        split_extension(
            w,
            Split {
                ext: pos..end,
                tag,
                label: &label,
                used: pos + 2..second.end,
                disposition: anim,
                detail,
                carve: None,
            },
        )?;
    }
    Ok(end)
}

/// `XMP DataXMP` is raw XMP text followed by a 258-byte ramp that makes
/// length-walking readers converge, not real sub-blocks. The `gif` crate walks
/// the lengths anyway and lands on the ramp's end, which is the extent used
/// here; the extension is split into packet and ramp only when the walk ended
/// on a ramp.
fn xmp(w: &mut Walker<'_>, pos: usize, sb: SubBlocks) -> Result<usize, WalkError> {
    let data = w.data;
    let end = sb.end;
    let tag = PartTag::Code(u32::from(LABEL_APPLICATION));
    // 0x21 0xFF 0x0B + the 11-byte identifier.
    let body_start = pos + 14;
    let ramp_start = end.wrapping_sub(XMP_RAMP_LEN);
    let ramp_ok =
        sb.terminated && end >= body_start + XMP_RAMP_LEN && is_xmp_ramp(&data[ramp_start..end]);
    if !ramp_ok {
        let trunc = if sb.terminated { "" } else { "; truncated" };
        let disposition = if sb.terminated {
            Disposition::Skipped
        } else {
            Disposition::Malformed
        };
        w.block(
            pos..end,
            tag,
            disposition,
            Some("XMP DataXMP".into()),
            Some(alloc::format!(
                "XMP extension without the 258-byte magic trailer; zengif never surfaces it{trunc}"
            )),
        )?;
        return Ok(end);
    }
    let parent = w.push(
        Part::new(
            PartKind::Block,
            tag,
            pos as u64..end as u64,
            Disposition::Skipped,
        )
        .with_label("XMP DataXMP")
        .with_body(body_start as u64..end as u64)
        .with_detail("the gif crate buffers the packet (xmp_metadata) when it precedes the first non-extension block; zengif never surfaces it"),
    )?;
    if ramp_start > body_start {
        let packet = &data[body_start..ramp_start];
        w.push_child(
            parent,
            Part::new(
                PartKind::Block,
                PartTag::None,
                body_start as u64..ramp_start as u64,
                Disposition::Skipped,
            )
            .with_label("XMP packet")
            .with_detail(alloc::format!(
                "{} bytes, starts {:?}",
                packet.len(),
                printable(packet, PREVIEW_MAX)
            )),
        )?;
    }
    w.push_child(
        parent,
        Part::new(
            PartKind::Block,
            PartTag::None,
            ramp_start as u64..end as u64,
            Disposition::Padding,
        )
        .with_label("XMP magic trailer")
        .with_detail("01 FF FE … 01 00 ramp plus the sub-block terminator"),
    )?;
    Ok(end)
}

fn is_xmp_ramp(b: &[u8]) -> bool {
    // 01, FF..=01 descending (255 values), 00, 00.
    b.len() == XMP_RAMP_LEN
        && b[0] == 1
        && (1..=255usize).all(|i| usize::from(b[i]) == 256 - i)
        && b[256] == 0
        && b[257] == 0
}
