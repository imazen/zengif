//! Round-4 review probes for the GIF inventory (PR 23) with the assertions that hold once
//! each finding is fixed, plus tests for the decoder limits and palette reach.
#![cfg(feature = "inventory")]

use std::borrow::Cow;

use enough::Unstoppable;
use zencodec::ImageSequence;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, MetadataKind, Part};
use zengif::GifDecoderConfig;

const LZW_1X1: &[u8] = &[0x02, 0x02, 0x44, 0x01, 0x00];
const ANIM: D = D::Metadata(MetadataKind::Animation);

fn inv_with(
    cfg: &GifDecoderConfig,
    limits: Option<zencodec::ResourceLimits>,
    data: &[u8],
) -> Inventory {
    let job = cfg.clone().job();
    let job = match limits {
        Some(l) => job.with_limits(l),
        None => job,
    };
    let i = job.inventory(data).expect("inventory");
    i.validate().unwrap_or_else(|e| panic!("{e}\n{i}"));
    i
}
fn inv(data: &[u8]) -> Inventory {
    inv_with(&GifDecoderConfig::new(), None, data)
}

fn screen(w: u16, h: u16) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&w.to_le_bytes());
    g.extend_from_slice(&h.to_le_bytes());
    g.extend_from_slice(&[0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    g
}

fn frame_at(g: &mut Vec<u8>, left: u8) {
    g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, 0]);
    g.extend_from_slice(LZW_1X1);
}

fn netscape(loops: u16) -> Vec<u8> {
    let mut v = vec![0x21, 0xFF, 0x0B];
    v.extend_from_slice(b"NETSCAPE2.0");
    let l = loops.to_le_bytes();
    v.extend_from_slice(&[3, 1, l[0], l[1], 0]);
    v
}

fn part_at(i: &Inventory, start: u64) -> &Part {
    i.parts()
        .iter()
        .find(|p| p.parent().is_none() && p.range().start == start)
        .unwrap_or_else(|| panic!("no top-level part at {start}\n{i}"))
}

fn probe_loop(g: &[u8]) -> Result<Option<u32>, String> {
    GifDecoderConfig::new()
        .job()
        .probe(g)
        .map(|info| match info.sequence {
            ImageSequence::Animation { loop_count, .. } => loop_count,
            _ => None,
        })
        .map_err(|e| e.to_string())
}

fn anim_loop(g: &[u8]) -> Result<Option<u32>, String> {
    let mut d = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(g), &[])
        .map_err(|e| e.to_string())?;
    loop {
        match d.render_next_frame(None) {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => return Err(format!("after frames: {e}")),
        }
    }
    Ok(d.loop_count())
}

/// "last overall" counts the walker's descriptors; detect.rs resynchronises
/// one byte at a time and can swallow the second frame and the NETSCAPE.
#[test]
fn n1_detect_rs_counts_fewer_frames_netscape_overclaimed() {
    let mut g = screen(2, 1);
    frame_at(&mut g, 0);
    g.extend_from_slice(&[0x07, 0x01, 0x21, 0x00]); // gif crate: unknown block, 1 sub-block [21]
    frame_at(&mut g, 1);
    let ns = g.len();
    g.extend(netscape(5));
    g.push(0x3B);
    let p = GifDecoderConfig::new().job().probe(&g).unwrap();
    println!("probe sequence {:?}", p.sequence);
    let a = anim_loop(&g);
    println!("animation decoder loop_count {a:?}");
    assert_eq!(probe_loop(&g).unwrap(), None, "probe reports no loop count");
    assert_ne!(a.clone().unwrap(), Some(5));
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, ns as u64).disposition(), D::Dropped, "{i}");
}

/// The other direction: detect.rs counts a bogus frame inside an unknown
/// block, so probe() reports the loop count of a NETSCAPE: the inventory asks
/// detect.rs itself and keeps it as metadata.
#[test]
fn n1b_detect_rs_counts_more_frames_netscape_underclaimed() {
    let mut g = screen(1, 1);
    frame_at(&mut g, 0);
    let ns = g.len();
    g.extend(netscape(5)); // after the header phase, single real frame
    // gif crate: unknown 05, sub-block of 10 bytes that look like a descriptor to detect.rs
    g.extend_from_slice(&[0x05, 0x0A, 0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0x00]);
    g.push(0x3B);
    let pl = probe_loop(&g);
    println!("probe loop {pl:?}");
    let i = inv(&g);
    println!("{i}");
    // Native repeats 5 = 6 plays.
    assert_eq!(pl.unwrap(), Some(6), "probe() reports 6 plays");
    assert_eq!(part_at(&i, ns as u64).disposition(), ANIM, "{i}");
}

/// A zero-sized logical screen: Decoder::new rejects it (check_dimensions), so
/// every zencodec path fails; nothing after the header is consumed.
#[test]
fn n2_zero_width_screen_nothing_decodes() {
    let mut g = screen(0, 1);
    let ns = g.len();
    g.extend(netscape(5));
    frame_at(&mut g, 0);
    frame_at(&mut g, 0);
    g.push(0x3B);
    let pe = GifDecoderConfig::new()
        .job()
        .probe(&g)
        .err()
        .map(|e| e.to_string());
    println!("probe: {pe:?}");
    assert!(pe.is_some());
    assert!(GifDecoderConfig::new().decode(&g).is_err());
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, 0).disposition(), D::Structure);
    assert!(
        part_at(&i, 0)
            .detail()
            .unwrap()
            .contains("dimension limits"),
        "{i}"
    );
    assert_eq!(part_at(&i, 13).disposition(), D::Dropped, "{i}");
    assert_eq!(part_at(&i, ns as u64).disposition(), D::Dropped, "{i}");
    assert!(
        i.parts()
            .iter()
            .all(|p| !matches!(p.disposition(), D::ImageData | D::Metadata(_))),
        "{i}"
    );
}

/// A screen wider than the default max_width (16384): every zencodec path
/// fails, so no part is consumed.
#[test]
fn n3_screen_over_default_dimension_limit() {
    let mut g = screen(20000, 1);
    let gct = 13u64;
    let ns = g.len();
    g.extend(netscape(5));
    let f1 = g.len();
    frame_at(&mut g, 0);
    frame_at(&mut g, 1);
    g.push(0x3B);
    let pe = GifDecoderConfig::new()
        .job()
        .probe(&g)
        .err()
        .map(|e| e.to_string());
    let de = GifDecoderConfig::new()
        .decode(&g)
        .err()
        .map(|e| e.to_string());
    println!("probe: {pe:?}\ndecode: {de:?}");
    assert!(pe.is_some() && de.is_some());
    // Native API without limits decodes it: the file itself is fine.
    assert!(zengif::decode_gif(&g, zengif::Limits::none(), &Unstoppable).is_ok());
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, gct).disposition(), D::Dropped, "{i}");
    assert_eq!(part_at(&i, ns as u64).disposition(), D::Dropped, "{i}");
    assert_eq!(part_at(&i, f1 as u64 + 10).disposition(), D::Skipped, "{i}");
    // The job's own limit is ignored the same way.
    let lim = zencodec::ResourceLimits::none().with_max_width(1);
    let mut h = screen(2, 1);
    frame_at(&mut h, 0);
    h.push(0x3B);
    assert!(
        GifDecoderConfig::new()
            .job()
            .with_limits(lim)
            .probe(&h)
            .is_err()
    );
    let j = inv_with(&GifDecoderConfig::new(), Some(lim), &h);
    assert_eq!(part_at(&j, 29).disposition(), D::Skipped, "{j}");
}

/// Frames beyond the frame-count limit are never decoded; probe() fails its
/// frame cap and reports no loop count.
#[test]
fn n4_frames_beyond_the_frame_limit() {
    let lim = zencodec::ResourceLimits::none().with_max_frames(3);
    let mut g = screen(2, 1);
    for k in 0..4 {
        frame_at(&mut g, (k % 2) as u8);
    }
    let f4 = g.len() - 15;
    let ns = g.len();
    g.extend(netscape(5));
    g.push(0x3B);
    let job = GifDecoderConfig::new().job().with_limits(lim);
    let p = job.probe(&g).unwrap();
    println!("probe with max_frames 3: {:?}", p.sequence);
    let mut d = GifDecoderConfig::new()
        .job()
        .with_limits(lim)
        .animation_frame_decoder(Cow::Borrowed(&g[..]), &[])
        .unwrap();
    let mut n = 0;
    let err = loop {
        match d.render_next_frame(None) {
            Ok(Some(_)) => n += 1,
            Ok(None) => break None,
            Err(e) => break Some(e.to_string()),
        }
    };
    println!("animation decoder: {n} frames then {err:?}");
    assert_eq!(n, 3);
    assert!(err.is_some());
    let i = inv_with(&GifDecoderConfig::new(), Some(lim), &g);
    println!("{i}");
    assert_eq!(
        part_at(&i, f4 as u64).disposition(),
        D::Malformed,
        "frame 4 refused\n{i}"
    );
    assert_eq!(part_at(&i, f4 as u64 + 10).disposition(), D::Skipped, "{i}");
    // probe() hits detect.rs's frame cap and reports nothing; the animation
    // decoder reads it before the first frame, so the early NETSCAPE is the one used.
    assert_eq!(part_at(&i, ns as u64).disposition(), D::Dropped, "{i}");
}

/// Same with the default 10,000-frame limit (no job limits at all).
#[test]
fn n4b_default_frame_limit() {
    let mut g = screen(2, 1);
    for k in 0..10_001 {
        frame_at(&mut g, (k % 2) as u8);
    }
    let last = g.len() - 15;
    g.push(0x3B);
    let mut d = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&g[..]), &[])
        .unwrap();
    let mut n = 0;
    let err = loop {
        match d.render_next_frame(None) {
            Ok(Some(_)) => n += 1,
            Ok(None) => break None,
            Err(e) => break Some(e.to_string()),
        }
    };
    println!(
        "{} bytes; animation decoder: {n} frames then {err:?}",
        g.len()
    );
    assert_eq!(n, 10_000);
    let i = inv(&g);
    assert_eq!(
        part_at(&i, last as u64).disposition(),
        D::Malformed,
        "frame 10,001 refused"
    );
    assert_eq!(part_at(&i, last as u64 + 10).disposition(), D::Skipped);
}

/// Palette entries no index can reach: with LZW minimum code size 2 the
/// decoder only produces indices 0..=3, so entries 4..=255 of a 256-entry
/// local table are never read.
#[test]
fn n5_palette_entries_beyond_the_code_size_are_unreachable() {
    let build = |secret: bool| {
        let mut g = screen(1, 1);
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x87]); // LCT, 256 entries
        let lct = g.len();
        let mut table: Vec<u8> = (0..768).map(|i| (i * 7) as u8).collect();
        if secret {
            let msg = b"OWNER=Jane Doe; SERIAL=CAM-0042; GPS=47.6,-122.3 ";
            for (k, b) in table[12..].iter_mut().enumerate() {
                *b = msg[k % msg.len()];
            }
        }
        g.extend_from_slice(&table);
        g.extend_from_slice(LZW_1X1); // min code size 2
        g.push(0x3B);
        (g, lct)
    };
    let (a, lct) = build(false);
    let (b, _) = build(true);
    let da = GifDecoderConfig::new().decode(&a).unwrap();
    let db = GifDecoderConfig::new().decode(&b).unwrap();
    assert_eq!(
        da.pixels().contiguous_bytes(),
        db.pixels().contiguous_bytes()
    );
    assert_eq!(da.info(), db.info());
    let i = inv(&b);
    println!("{i}");
    let p = part_at(&i, lct as u64);
    assert_eq!(p.range().end - p.range().start, 768);
    // Min code size 2: indices 0..=3 only. The head (12 bytes) is read, the rest is not.
    let kids: Vec<_> = i.parts().iter().filter(|q| q.parent().is_some()).collect();
    assert_eq!(kids.len(), 2, "{i}");
    assert_eq!(
        (
            kids[0].range().start,
            kids[0].range().end,
            kids[0].disposition()
        ),
        (lct as u64, lct as u64 + 12, D::ImageData),
        "{i}"
    );
    assert_eq!(
        (
            kids[1].range().start,
            kids[1].range().end,
            kids[1].disposition()
        ),
        (lct as u64 + 12, lct as u64 + 768, D::Dropped),
        "{i}"
    );
    assert!(kids[1].detail().unwrap().contains("no index can reach"));
}

/// The transparent-index byte of a Graphic Control whose transparency flag is
/// clear is never read: it is a `Dropped` child.
#[test]
fn n6_gce_transparent_index_byte_unread() {
    let build = |x: u8| {
        let mut g = screen(2, 1);
        g.extend(netscape(0));
        g.extend_from_slice(&[0x21, 0xF9, 4, 0x00, 7, 0, x, 0]);
        frame_at(&mut g, 0);
        frame_at(&mut g, 1);
        g.push(0x3B);
        g
    };
    let (a, b) = (build(0), build(0xA5));
    let fa = zengif::decode_gif(&a, zengif::Limits::none(), &Unstoppable)
        .unwrap()
        .1;
    let fb = zengif::decode_gif(&b, zengif::Limits::none(), &Unstoppable)
        .unwrap()
        .1;
    for (x, y) in fa.iter().zip(&fb) {
        assert_eq!((x.delay, &x.pixels), (y.delay, &y.pixels));
    }
    let pa = GifDecoderConfig::new().job().probe(&a).unwrap();
    let pb = GifDecoderConfig::new().job().probe(&b).unwrap();
    assert_eq!((pa.has_alpha, &pa.sequence), (pb.has_alpha, &pb.sequence));
    let i = inv(&b);
    let at = 13 + 6 + 19;
    let kids: Vec<_> = i
        .parts()
        .iter()
        .filter(|q| q.range().start >= at && q.range().end <= at + 8 && q.parent().is_some())
        .collect();
    let byte = kids
        .iter()
        .find(|q| q.disposition() == D::Dropped)
        .expect("unread byte part");
    assert_eq!(byte.range(), (at + 6)..(at + 7), "{i}");
    assert!(kids.iter().any(|q| q.disposition() == ANIM), "{i}");
}

/// Memory per part for unknown blocks that carry the long probe() note.
#[test]
fn n8_long_detail_memory() {
    let n: usize = std::env::var("N8_BLOCKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mem: Option<u64> = std::env::var("N8_MEM").ok().and_then(|s| s.parse().ok());
    if n == 0 {
        return;
    }
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    for _ in 0..n {
        g.extend_from_slice(&[0x07, 0x01, 0x21, 0x00]);
    }
    let mut job = GifDecoderConfig::new().job();
    if let Some(m) = mem {
        job = job.with_limits(zencodec::ResourceLimits::none().with_max_memory(m));
    }
    match job.inventory(&g) {
        Ok(i) => {
            let detail: usize = i
                .parts()
                .iter()
                .map(|p| p.detail().map_or(0, str::len))
                .sum();
            println!(
                "ok {} parts, detail bytes {} ({} per part)",
                i.parts().len(),
                detail,
                detail / i.parts().len()
            );
        }
        Err(e) => println!("err {e}"),
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// q14 on interlaced multi-row frames through the zencodec decode path (default
/// limits), plus the other direction: overwriting the data bytes of an
/// `image data tail` must not change the decoded output.
#[test]
fn n7_interlaced_multirow_differential_and_tail_invariance() {
    use weezl::{BitOrder, encode::Encoder};
    let iters: usize = std::env::var("N7_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let mut rng = Rng(0x005E_ED0F_1234);
    let mut unsafe_interlaced = 0usize;
    let (mut ok_ok, mut bad_bad, mut safe, mut unsafe_, mut tails, mut tail_changed) =
        (0, 0, 0, 0, 0, 0);
    let mut notes = Vec::new();
    for it in 0..iters {
        let min = 2 + (rng.next() % 7) as u8;
        let w = 1 + (rng.next() % 40) as u16;
        let h = 2 + (rng.next() % 15) as u16;
        let interlaced = rng.next().is_multiple_of(2);
        let n = usize::from(w) * usize::from(h);
        let colours = 1u64 << min;
        let px: Vec<u8> = (0..n)
            .map(|i| {
                if it % 2 == 0 {
                    (rng.next() % colours) as u8
                } else {
                    ((i / 3) as u64 % colours) as u8
                }
            })
            .collect();
        let mut raw = Encoder::new(BitOrder::Lsb, min).encode(&px).unwrap();
        let kind = rng.next() % 5;
        let hh = match kind {
            1 => h + 1,          // more rows than the data holds: truncated
            2 => (h / 2).max(1), // fewer: data after the pixel count
            _ => h,
        };
        if kind == 3 {
            for _ in 0..1 + rng.next() % 3 {
                let i = (rng.next() % raw.len() as u64) as usize;
                raw[i] ^= 1 << (rng.next() % 8);
            }
        }
        if kind == 4 || kind == 0 {
            for _ in 0..rng.next() % 40 {
                raw.push(rng.next() as u8);
            }
        }
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&hh.to_le_bytes());
        g.extend_from_slice(&[0x87, 0, 0]);
        g.extend((0..768).map(|i| i as u8));
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&hh.to_le_bytes());
        g.push(if interlaced { 0x40 } else { 0 });
        let at = g.len();
        g.push(min);
        let mut data_bytes = Vec::new();
        for c in raw.chunks(255) {
            g.push(c.len() as u8);
            for &b in c {
                data_bytes.push(g.len());
                g.push(b);
            }
        }
        g.push(0);
        g.push(0x3B);
        let out = GifDecoderConfig::new().decode(&g);
        let i = inv(&g);
        let head = part_at(&i, at as u64);
        let walker_ok = head.disposition() != D::Malformed;
        match (walker_ok, out.is_ok()) {
            (true, true) => ok_ok += 1,
            (false, false) => bad_bad += 1,
            (false, true) => safe += 1,
            (true, false) => {
                unsafe_ += 1;
                if interlaced {
                    unsafe_interlaced += 1;
                }
                if notes.len() < 5 {
                    notes.push(format!("it {it}: min {min} {w}x{hh} interlaced {interlaced} kind {kind}: walker {:?}, decoder {:?}\nGIF {:02x?}", head.disposition(), out.as_ref().err().map(|e| e.to_string()), &g[13+768..]));
                }
                if let Ok(dir) = std::env::var("N7_DUMP") {
                    let _ = std::fs::write(format!("{dir}/n7-case-{it}.gif"), &g);
                    // Same file with the interlace flag cleared.
                    let mut flat = g.clone();
                    flat[at - 1] &= !0x40;
                    let flat_ok = GifDecoderConfig::new().decode(&flat).is_ok();
                    let native_ok =
                        zengif::decode_gif(&g, zengif::Limits::none(), &Unstoppable).is_ok();
                    notes.push(format!("    it {it}: same stream non-interlaced decodes: {flat_ok}; native Limits::none() interlaced decodes: {native_ok}; raw {} B", raw.len()));
                }
            }
        }
        if let (Ok(o), Some(tail)) = (
            &out,
            i.parts()
                .iter()
                .find(|p| p.label() == Some("image data tail")),
        ) {
            tails += 1;
            let reference = o.pixels().contiguous_bytes().into_owned();
            for _ in 0..4 {
                let mut m = g.clone();
                for &d in &data_bytes {
                    if (d as u64) >= tail.range().start && (d as u64) < tail.range().end {
                        m[d] = rng.next() as u8;
                    }
                }
                let ok = GifDecoderConfig::new()
                    .decode(&m)
                    .is_ok_and(|o2| o2.pixels().contiguous_bytes().into_owned() == reference);
                if !ok {
                    tail_changed += 1;
                    if notes.len() < 10 {
                        notes.push(format!("it {it}: min {min} {w}x{hh} interlaced {interlaced} kind {kind}: overwriting the tail {:?} changes the output", tail.range()));
                    }
                    break;
                }
            }
        }
    }
    println!(
        "{iters} cases: both ok {ok_ok}, both fail {bad_bad}, walker stricter {safe}, walker ok/decoder fails {unsafe_}; tails {tails}, tails that matter {tail_changed}"
    );
    for n in &notes {
        println!("  {n}");
    }
    // Non-interlaced frames: the walker and the decoder agree. Interlaced frames
    // are decoded row by row, which drives weezl through a call pattern the walk
    // does not model; on rare streams (including weezl-encoded ones with data
    // appended) the decoder reports "image truncated" where a whole-frame decode
    // succeeds. That divergence is open and awaiting a decision (see the round-4
    // report and zengif issue 26), so it is counted and printed here, not asserted.
    println!("interlaced walker-ok/decoder-fails: {unsafe_interlaced}");
    assert_eq!(unsafe_ - unsafe_interlaced, 0);
    assert_eq!(tail_changed, 0);
}

// ── tests for the fixes ──────────────────────────────────────────────

/// With the flag set the index byte is read: no carve.
#[test]
fn gce_with_transparency_keeps_the_index_byte() {
    let mut g = screen(1, 1);
    let at = g.len();
    g.extend_from_slice(&[0x21, 0xF9, 4, 0x01, 7, 0, 1, 0]);
    frame_at(&mut g, 0);
    g.push(0x3B);
    let i = inv(&g);
    assert_eq!(part_at(&i, at as u64).disposition(), ANIM, "{i}");
    assert!(i.parts().iter().all(|p| p.parent().is_none()), "{i}");
}

/// The global table is read only up to the largest code size of the frames
/// that read it.
#[test]
fn global_table_reach_follows_the_frames_code_size() {
    let build = |secret: bool| {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[1, 0, 1, 0, 0x87, 0, 0]); // 256-entry GCT
        let gct = g.len();
        let mut t: Vec<u8> = (0..768).map(|i| (i * 5) as u8).collect();
        if secret {
            t[12..].fill(b'S');
        }
        g.extend_from_slice(&t);
        g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
        g.extend_from_slice(LZW_1X1); // min code size 2
        g.push(0x3B);
        (g, gct)
    };
    let (a, gct) = build(false);
    let (b, _) = build(true);
    assert_eq!(
        GifDecoderConfig::new()
            .decode(&a)
            .unwrap()
            .pixels()
            .contiguous_bytes(),
        GifDecoderConfig::new()
            .decode(&b)
            .unwrap()
            .pixels()
            .contiguous_bytes()
    );
    let i = inv(&b);
    let kids: Vec<_> = i.parts().iter().filter(|q| q.parent().is_some()).collect();
    assert_eq!(kids.len(), 2, "{i}");
    assert_eq!(kids[0].range(), gct as u64..gct as u64 + 12);
    assert_eq!(kids[1].disposition(), D::Dropped);
}

/// The animation duration limit: the frame that crosses it fails the decoder.
#[test]
fn animation_duration_limit_fails_the_crossing_frame() {
    let lim = zencodec::ResourceLimits::none().with_max_animation_ms(1000);
    let mut g = screen(2, 1);
    g.extend_from_slice(&[0x21, 0xF9, 4, 0, 60, 0, 0, 0]); // 600 ms
    frame_at(&mut g, 0);
    g.extend_from_slice(&[0x21, 0xF9, 4, 0, 60, 0, 0, 0]); // 1200 ms total: over
    let f2 = g.len();
    frame_at(&mut g, 1);
    let f3 = g.len();
    frame_at(&mut g, 0);
    g.push(0x3B);
    let mut d = GifDecoderConfig::new()
        .job()
        .with_limits(lim)
        .animation_frame_decoder(Cow::Borrowed(&g[..]), &[])
        .unwrap();
    let mut n = 0;
    let err = loop {
        match d.render_next_frame(None) {
            Ok(Some(_)) => n += 1,
            Ok(None) => break None,
            Err(e) => break Some(e.to_string()),
        }
    };
    assert_eq!((n, err.is_some()), (1, true), "{err:?}");
    let i = inv_with(&GifDecoderConfig::new(), Some(lim), &g);
    assert_eq!(part_at(&i, f2 as u64).disposition(), D::Structure, "{i}");
    assert_eq!(
        part_at(&i, f2 as u64 + 10).disposition(),
        D::Malformed,
        "{i}"
    );
    assert_eq!(part_at(&i, f3 as u64).disposition(), D::Skipped, "{i}");
}

/// Known limitation, stated on the part: interlaced image data says the gif
/// crate's row-by-row reads can fail a stream the walk accepts.
#[test]
fn interlaced_image_data_names_the_limitation() {
    let mut g = screen(1, 1);
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x40]);
    let at = g.len();
    g.extend_from_slice(LZW_1X1);
    g.push(0x3B);
    let i = inv(&g);
    let d = part_at(&i, at as u64).detail().map(str::to_owned).unwrap();
    assert!(d.contains("row-by-row reads can fail"), "{d}");
    let mut flat = g.clone();
    flat[at - 1] = 0;
    let d = part_at(&inv(&flat), at as u64)
        .detail()
        .map(str::to_owned)
        .unwrap();
    assert!(!d.contains("row-by-row"), "{d}");
}

// ── round 5 ──────────────────────────────────────────────────────────

/// Finding 1 residual: `Decoder::new` also enforces the memory limit when it
/// allocates the canvas. A screen within the dimension limits but over the
/// job's `max_memory` fails every zencodec path.
#[test]
fn r5_canvas_over_max_memory() {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&1024u16.to_le_bytes());
    g.extend_from_slice(&1024u16.to_le_bytes());
    g.extend_from_slice(&[0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
    g.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    g.push(0x3B);
    let lim = zencodec::ResourceLimits::none().with_max_memory(1 << 20);
    let job = || GifDecoderConfig::new().job().with_limits(lim);
    let pe = job().probe(&g).err().map(|e| e.to_string());
    println!("probe with max_memory 1 MiB: {pe:?}");
    let ok_default = GifDecoderConfig::new().job().probe(&g).is_ok();
    println!("probe with default limits ok: {ok_default}");
    let i = job().inventory(&g).unwrap();
    i.validate().unwrap();
    println!("{i}");
    let data = i
        .parts()
        .iter()
        .find(|p| p.label() == Some("image data"))
        .unwrap();
    println!("image data disposition: {:?}", data.disposition());
    assert!(pe.is_some());
    assert!(ok_default);
    assert_eq!(data.disposition(), D::Skipped, "{i}");
    let header = &i.parts()[0];
    assert!(header.detail().unwrap().contains("memory limit"), "{i}");
    // Within the limit the same file is read.
    let roomy = zencodec::ResourceLimits::none().with_max_memory(8 << 20);
    let j = GifDecoderConfig::new()
        .job()
        .with_limits(roomy)
        .inventory(&g)
        .unwrap();
    let data = j
        .parts()
        .iter()
        .find(|p| p.label() == Some("image data"))
        .unwrap();
    assert_eq!(data.disposition(), D::ImageData, "{j}");
}
