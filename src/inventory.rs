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
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use enough::Stop;
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

struct Walker<'a> {
    data: &'a [u8],
    inv: Inventory,
    stop: &'a dyn Stop,
    pushed: usize,
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

/// Inventory `data` as a GIF file.
pub(crate) fn walk(data: &[u8], stop: &dyn Stop) -> Result<Inventory, WalkError> {
    let len = data.len();
    let mut w = Walker {
        data,
        inv: Inventory::new(ImageFormat::Gif, len as u64),
        stop,
        pushed: 0,
    };
    if len == 0 {
        return Ok(w.inv);
    }
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
    w.push(
        Part::new(
            PartKind::Header,
            PartTag::None,
            0..13,
            Disposition::Structure,
        )
        .with_label(version)
        .with_detail(alloc::format!(
            "screen {sw}x{sh}, background index {}, pixel aspect byte {}; the gif crate \
                 parses the last two, zencodec output does not carry them",
            data[11],
            data[12]
        )),
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
        w.block(
            pos..end,
            PartTag::None,
            Disposition::ImageData,
            Some("global colour table".into()),
            Some(alloc::format!("{} entries", size / 3)),
        )?;
        pos = end;
    }

    let mut frames = 0usize;
    while pos < len {
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
                match image(w, pos, frames, (sw, sh))? {
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

/// Image descriptor, optional local colour table, image data. Returns the next
/// offset, or `None` when the file ended inside it.
fn image(
    w: &mut Walker<'_>,
    pos: usize,
    frame: usize,
    screen: (u16, u16),
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

    // The decode path enables `check_frame_consistency` and rejects empty
    // frames (decode/mod.rs `next_frame`, `ZeroDimensionFrame`).
    let problem = if fw == 0 || fh == 0 {
        Some("zero-sized frame; the decoder rejects it")
    } else if u32::from(left) + u32::from(fw) > u32::from(screen.0)
        || u32::from(top) + u32::from(fh) > u32::from(screen.1)
    {
        Some("frame outside the logical screen; the decoder rejects it")
    } else {
        None
    };
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
    w.block(
        pos..desc_end,
        PartTag::Code(u32::from(IMAGE_SEPARATOR)),
        if problem.is_some() {
            Disposition::Malformed
        } else {
            Disposition::Structure
        },
        Some("image descriptor".into()),
        Some(detail),
    )?;

    let mut at = desc_end;
    if flags & 0x80 != 0 && at < len {
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
        w.block(
            at..end,
            PartTag::None,
            Disposition::ImageData,
            Some("local colour table".into()),
            Some(alloc::format!("frame {frame}, {} entries", size / 3)),
        )?;
        at = end;
    }

    let Some(&min_code) = data.get(at) else {
        return Ok(None);
    };
    let sb = sub_blocks(data, at + 1);
    // `StreamingDecoder::check_code_size`: 1..=11.
    let (disposition, note) = if !sb.terminated {
        (Disposition::Malformed, "; truncated image data")
    } else if !(1..=11).contains(&min_code) {
        (
            Disposition::Malformed,
            "; invalid LZW minimum code size, the decoder rejects it",
        )
    } else {
        (Disposition::ImageData, "")
    };
    w.block(
        at..sb.end,
        PartTag::None,
        disposition,
        Some("image data".into()),
        Some(alloc::format!(
            "frame {frame}: LZW minimum code size {min_code}, {} sub-blocks, {} bytes{note}",
            sb.count,
            sb.payload
        )),
    )?;
    Ok(Some(sb.end))
}

/// An introducer byte the `gif` crate does not know. It consumes the byte,
/// then reads the next byte as the first sub-block length.
fn unknown_block(w: &mut Walker<'_>, pos: usize, introducer: u8) -> Result<usize, WalkError> {
    let sb = sub_blocks(w.data, pos + 1);
    let (disposition, note) = if sb.terminated {
        (Disposition::Unknown, "")
    } else {
        (Disposition::Malformed, "; truncated")
    };
    w.block(
        pos..sb.end,
        PartTag::Code(u32::from(introducer)),
        disposition,
        None,
        Some(alloc::format!(
            "unknown block introducer 0x{introducer:02X}; the gif crate reads the following bytes \
             as sub-blocks ({} sub-blocks, {} bytes){note}",
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
        LABEL_GRAPHIC_CONTROL => {
            // The `gif` crate requires the last sub-block to hold exactly 4
            // bytes (`read_control_extension`), else decoding fails.
            let ok = sb.terminated && sb.last.as_ref().is_some_and(|r| r.len() == 4);
            let detail = match (&sb.last, ok) {
                (Some(r), true) => {
                    let b = &data[r.clone()];
                    alloc::format!(
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
                    )
                }
                _ => alloc::format!(
                    "control extension with a wrong length{trunc}; the decoder rejects it"
                ),
            };
            // Frame delay, disposal and the transparent index feed
            // `AnimationFrame` timing and the composited alpha.
            let d = if ok {
                Disposition::Metadata(MetadataKind::Animation)
            } else {
                Disposition::Malformed
            };
            (Some("Graphic Control".into()), d, detail)
        }
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

    let (disposition, detail): (Disposition, String) = match ident {
        Some(b"NETSCAPE2.0") => match loop_count(data, &second) {
            // `StreamingDecoder` -> `Reader::repeat`; surfaced as
            // `loop_count` by the probe and the animation decoder.
            Some(n) => (
                Disposition::Metadata(MetadataKind::Animation),
                alloc::format!("loop count {n} (0 = forever)"),
            ),
            None => (
                Disposition::Dropped,
                "NETSCAPE2.0 without a loop-count sub-block".into(),
            ),
        },
        Some(b"ANIMEXTS1.0") => (
            Disposition::Skipped,
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
                "ICC profile, {} bytes; the gif crate buffers it, zengif never surfaces it{trunc}",
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
                "XMP extension without the 258-byte magic trailer; the gif crate buffers it, \
                 zengif never surfaces it{trunc}"
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
        .with_detail("the gif crate buffers the packet (xmp_metadata); zengif never surfaces it"),
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
