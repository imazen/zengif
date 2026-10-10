//! Round-2 review probes for the GIF inventory (PR 23) with the assertions that hold once
//! each finding is fixed, plus a differential check of the LZW walk against weezl.
#![cfg(feature = "std")]

use std::borrow::Cow;
use std::collections::HashMap;

use enough::Unstoppable;
use zencodec::ImageSequence;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, MetadataKind, Part};
use zengif::GifDecoderConfig;

const LZW_1X1: &[u8] = &[0x02, 0x02, 0x44, 0x01, 0x00];
const ANIM: D = D::Metadata(MetadataKind::Animation);

fn inv(data: &[u8]) -> Inventory {
    let i = GifDecoderConfig::new()
        .job()
        .inventory(data)
        .expect("inventory")
        .expect("Some");
    i.validate().unwrap_or_else(|e| panic!("{e}\n{i}"));
    i
}

fn header(w: u8) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&[w, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    g
}

fn frame_at(g: &mut Vec<u8>, left: u8, flags: u8) {
    g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, flags]);
    g.extend_from_slice(LZW_1X1);
}

fn netscape(loops: u16) -> Vec<u8> {
    let mut v = vec![0x21, 0xFF, 0x0B];
    v.extend_from_slice(b"NETSCAPE2.0");
    let l = loops.to_le_bytes();
    v.extend_from_slice(&[3, 1, l[0], l[1], 0]);
    v
}

fn gce(delay: u8, transparent: bool) -> Vec<u8> {
    vec![0x21, 0xF9, 4, u8::from(transparent), delay, 0, 0, 0]
}

fn native(g: &[u8]) -> Result<(zengif::Metadata, Vec<zengif::ComposedFrame>), String> {
    zengif::decode_gif(g, zengif::Limits::none(), &Unstoppable)
        .map(|(m, f, _)| (m, f))
        .map_err(|e| e.to_string())
}

fn part_at(i: &Inventory, start: u64) -> &Part {
    i.parts()
        .iter()
        .find(|p| p.parent.is_none() && p.range.start == start)
        .unwrap_or_else(|| panic!("no top-level part at {start}\n{i}"))
}

fn leaf_at(i: &Inventory, at: u64) -> &Part {
    i.parts()
        .iter()
        .rfind(|p| p.range.start <= at && at < p.range.end)
        .unwrap()
}

// ── LZW differential: an encoder that mirrors weezl's decoder state ──────────

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}
impl Bits {
    fn put(&mut self, code: u16, size: u32) {
        self.acc |= u32::from(code) << self.n;
        self.n += size;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn flush(&mut self) {
        if self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

/// GIF LZW, code widths following weezl's decoder (bump when the entry just
/// derived is >= max_code). Returns the raw code stream and the number of raw
/// bytes up to and including the byte holding the end code's last bit.
fn lzw_encode(min: u8, px: &[u8], clear_every: usize) -> (Vec<u8>, usize) {
    let clear = 1u16 << min;
    let eoi = clear + 1;
    let first_free = eoi + 1;
    let mut b = Bits {
        out: Vec::new(),
        acc: 0,
        n: 0,
    };
    let mut dict: HashMap<(u16, u8), u16> = HashMap::new();
    let mut enc_next = first_free;
    let mut dec_size = u32::from(min) + 1;
    let mut dec_next = first_free;
    let mut first = true;
    let emit =
        |b: &mut Bits, code: u16, dec_size: &mut u32, dec_next: &mut u16, first: &mut bool| {
            b.put(code, *dec_size);
            if code == clear {
                *dec_size = u32::from(min) + 1;
                *dec_next = first_free;
                *first = true;
                return;
            }
            if *first {
                *first = false;
                return;
            }
            if *dec_next < 4096 {
                if u32::from(*dec_next) >= (1 << *dec_size) - 1 && *dec_size < 12 {
                    *dec_size += 1;
                }
                *dec_next += 1;
            }
        };
    emit(&mut b, clear, &mut dec_size, &mut dec_next, &mut first);
    let mut w = u16::from(px[0]);
    let mut since_clear = 0usize;
    for &c in &px[1..] {
        if let Some(&code) = dict.get(&(w, c)) {
            w = code;
            continue;
        }
        emit(&mut b, w, &mut dec_size, &mut dec_next, &mut first);
        if enc_next < 4096 {
            dict.insert((w, c), enc_next);
            enc_next += 1;
        }
        w = u16::from(c);
        since_clear += 1;
        if clear_every > 0 && since_clear >= clear_every {
            emit(&mut b, clear, &mut dec_size, &mut dec_next, &mut first);
            dict.clear();
            enc_next = first_free;
            since_clear = 0;
        }
    }
    emit(&mut b, w, &mut dec_size, &mut dec_next, &mut first);
    emit(&mut b, eoi, &mut dec_size, &mut dec_next, &mut first);
    b.flush();
    let n = b.out.len();
    (b.out, n)
}

/// Split raw LZW bytes into sub-blocks; return the bytes and the offset (relative
/// to the first length byte) one past the raw byte at index `raw_end - 1`.
fn sub_block(raw: &[u8], raw_end: usize) -> (Vec<u8>, usize) {
    let mut v = Vec::new();
    let mut cut = 0;
    let mut i = 0;
    for chunk in raw.chunks(255) {
        v.push(chunk.len() as u8);
        for &x in chunk {
            v.push(x);
            i += 1;
            if i == raw_end {
                cut = v.len();
            }
        }
    }
    (v, cut)
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

/// One-frame GIF with min code size `min`, the given pixels, and `junk`
/// appended as extra sub-blocks after the code stream.
fn lzw_gif(
    min: u8,
    w: u16,
    h: u16,
    px: &[u8],
    clear_every: usize,
    junk: &[u8],
) -> (Vec<u8>, usize, usize, usize) {
    let n = min.saturating_sub(1);
    let entries = 1usize << (n + 1);
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&w.to_le_bytes());
    g.extend_from_slice(&h.to_le_bytes());
    g.extend_from_slice(&[0x80 | n, 0, 0]);
    for i in 0..entries {
        g.extend_from_slice(&[i as u8, 255 - i as u8, (i as u8).wrapping_mul(7)]);
    }
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
    g.extend_from_slice(&w.to_le_bytes());
    g.extend_from_slice(&h.to_le_bytes());
    g.push(0);
    let at = g.len();
    g.push(min);
    let (raw, raw_end) = lzw_encode(min, px, clear_every);
    let (sb, cut) = sub_block(&raw, raw_end);
    let cut_abs = at + 1 + cut;
    g.extend_from_slice(&sb);
    for chunk in junk.chunks(255) {
        g.push(chunk.len() as u8);
        g.extend_from_slice(chunk);
    }
    g.push(0);
    let data_end = g.len();
    g.push(0x3B);
    (g, at, cut_abs, data_end)
}

#[test]
fn q1_lzw_walk_matches_weezl_differentially() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let sizes: &[(u16, u16)] = &[(1, 1), (3, 2), (16, 16), (64, 64), (200, 100)];
    let mut report = Vec::new();
    let mut checked = 0;
    let mut skipped = 0;
    for min in 1u8..=8 {
        for &(w, h) in sizes {
            for mode in 0..3 {
                let n = usize::from(w) * usize::from(h);
                let colours = 1u64 << min;
                let px: Vec<u8> = (0..n)
                    .map(|i| match mode {
                        0 => (rng.next() % colours) as u8,
                        1 => ((i / 7) as u64 % colours) as u8,
                        _ => ((i % 3 == 0) as u64 * (rng.next() % colours)) as u8,
                    })
                    .collect();
                let clear_every = if mode == 2 { 50 } else { 0 };
                let junk = b"JUNK-AFTER-END-CODE: owner=someone";
                let (g, at, cut, data_end) = lzw_gif(min, w, h, &px, clear_every, junk);
                // The encoder is only trusted if the real decoder agrees with it.
                // At min code size 1 GIF LZW cannot express every stream (code 4
                // does not fit 2 bits); keep only the cases the decoder reproduces.
                let decoded_ok = native(&g).is_ok_and(|(_, f)| {
                    f[0].pixels.iter().enumerate().all(|(i, p)| {
                        let v = px[i];
                        (p.r, p.g, p.b) == (v, 255 - v, v.wrapping_mul(7))
                    })
                });
                if !decoded_ok {
                    assert_eq!(
                        min, 1,
                        "min {min} {w}x{h} mode {mode}: encoder disagrees with the decoder"
                    );
                    skipped += 1;
                    continue;
                }
                let i = inv(&g);
                let head = part_at(&i, at as u64);
                let ok_head = head.label.as_deref() == Some("image data")
                    && head.disposition == D::ImageData
                    && head.range.end == cut as u64;
                let tail_ok = i.parts().iter().any(|p| {
                    p.label.as_deref() == Some("image data tail")
                        && p.disposition == D::Dropped
                        && p.range == (cut as u64..data_end as u64)
                });
                checked += 1;
                if !(ok_head && tail_ok) {
                    report.push(format!(
                        "min {min} {w}x{h} mode {mode}: expected head {at}..{cut}, tail {cut}..{data_end}; got {:?} {:?} {:?}",
                        head.label, head.disposition, head.range
                    ));
                }
            }
        }
    }
    println!(
        "{checked} cases ({skipped} min-1 cases the decoder cannot reproduce skipped), {} mismatches",
        report.len()
    );
    for r in &report {
        println!("{r}");
    }
    assert!(
        report.is_empty(),
        "{} of {checked} cases mismatch",
        report.len()
    );
}

/// Differential against weezl (the LZW decoder the gif crate runs): for streams weezl's
/// own encoder produced (min code size 2..=8) and streams from the mirror encoder above
/// (min code size 1, which weezl's encoder rejects), the head/tail cut must sit exactly
/// after the byte holding the end code, with the appended junk in the tail.
#[test]
fn q1b_lzw_cut_matches_weezl_decoder_consumption() {
    use weezl::{BitOrder, decode::Decoder, encode::Encoder};
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut cases = 0;
    for min in 1u8..=8 {
        for &npx in &[1usize, 6, 256, 4096, 20000] {
            for mode in 0..2 {
                let colours = 1u64 << min;
                let px: Vec<u8> = (0..npx)
                    .map(|i| {
                        if mode == 0 {
                            (rng.next() % colours) as u8
                        } else {
                            ((i / 5) as u64 % colours) as u8
                        }
                    })
                    .collect();
                let stream = if min >= 2 {
                    Encoder::new(BitOrder::Lsb, min).encode(&px).unwrap()
                } else {
                    lzw_encode(min, &px, 0).0
                };
                // weezl must reproduce the pixels from the stream we cut.
                if Decoder::new(BitOrder::Lsb, min)
                    .decode(&stream)
                    .ok()
                    .as_deref()
                    != Some(&px[..])
                {
                    assert_eq!(min, 1, "weezl rejects its own encoder's stream");
                    continue;
                }
                // Square-ish frame of exactly npx pixels on a (w x h) screen.
                let (w, h) = (npx as u16, 1u16);
                if npx > usize::from(u16::MAX) {
                    continue;
                }
                let mut g = b"GIF89a".to_vec();
                g.extend_from_slice(&w.to_le_bytes());
                g.extend_from_slice(&h.to_le_bytes());
                let n = min.saturating_sub(1);
                g.extend_from_slice(&[0x80 | n, 0, 0]);
                g.extend(std::iter::repeat_n(0u8, 3 << (n + 1)));
                g.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
                g.extend_from_slice(&w.to_le_bytes());
                g.extend_from_slice(&h.to_le_bytes());
                g.push(0);
                let at = g.len();
                g.push(min);
                let (sb, cut) = sub_block(&stream, stream.len());
                let cut_abs = (at + 1 + cut) as u64;
                g.extend_from_slice(&sb);
                g.extend_from_slice(&[19]);
                g.extend_from_slice(b"JUNK AFTER END CODE");
                g.push(0);
                let data_end = g.len() as u64;
                g.push(0x3B);
                let i = inv(&g);
                let head = part_at(&i, at as u64);
                assert_eq!(
                    head.range.end, cut_abs,
                    "min {min} px {npx} mode {mode}\n{i}"
                );
                assert_eq!(head.disposition, D::ImageData);
                let tail = part_at(&i, cut_abs);
                assert_eq!(tail.label.as_deref(), Some("image data tail"), "{i}");
                assert_eq!(tail.range.end, data_end);
                assert_eq!(tail.disposition, D::Dropped);
                cases += 1;
            }
        }
    }
    assert!(cases >= 60, "only {cases} cases ran");
}

/// EOI before the pixel count: the gif crate errors ("image truncated"); the
/// inventory still calls the frame's data ImageData and keeps later frames
/// consumed.
#[test]
fn q2_eoi_before_pixel_count_is_fatal_but_consumed() {
    // 2x1 screen, frame 1 is 2x1 but its LZW data holds 1 pixel then EOI.
    let mut g = header(2);
    g.extend(netscape(0));
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 1, 0, 0]);
    let f1 = g.len();
    g.extend_from_slice(LZW_1X1);
    let f2 = g.len();
    frame_at(&mut g, 1, 0);
    g.push(0x3B);
    let e = native(&g).expect_err("decoder rejects the short frame");
    println!("native: {e}");
    let z = GifDecoderConfig::new().decode(&g);
    println!(
        "zencodec decode: {:?}",
        z.as_ref().err().map(|e| e.to_string())
    );
    assert!(z.is_err());
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, f1 as u64).disposition, D::Malformed, "{i}");
    assert_eq!(
        part_at(&i, f2 as u64).disposition,
        D::Skipped,
        "frame 2 never decoded\n{i}"
    );
}

/// NETSCAPE2.0 after an unknown introducer byte but before the first image:
/// the gif crate's header phase already ended, probe() reads the later one.
#[test]
fn q3_netscape_after_unknown_block_before_first_image() {
    let mut g = header(2);
    g.extend_from_slice(&[0x05, 0x00]); // no 21/2C/3B inside: Unknown, both walkers agree
    let ns3 = g.len();
    g.extend(netscape(3));
    frame_at(&mut g, 0, 0);
    g.extend(netscape(7));
    frame_at(&mut g, 1, 0);
    g.push(0x3B);
    let (m, f) = native(&g).unwrap();
    assert_eq!(f.len(), 2);
    println!("native repeat {:?}", m.repeat);
    let probe = match GifDecoderConfig::new().job().probe(&g).unwrap().sequence {
        ImageSequence::Animation { loop_count, .. } => loop_count,
        other => panic!("{other:?}"),
    };
    let mut d = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&g[..]), &[])
        .unwrap();
    while d.render_next_frame(None).unwrap().is_some() {}
    let anim = d.loop_count();
    println!("probe {probe:?}, animation decoder {anim:?}");
    assert_eq!(probe, Some(7));
    assert_ne!(anim, Some(3));
    assert_ne!(m.repeat, zengif::Repeat::Count(3));
    let i = inv(&g);
    println!("{i}");
    assert_eq!(
        part_at(&i, ns3 as u64).disposition,
        D::Dropped,
        "loop 3 reaches nobody\n{i}"
    );
    assert_eq!(part_at(&i, ns3 as u64 + 19 + 15).disposition, ANIM, "{i}");
}

/// A GCE whose first 4-byte sub-block says "transparent" and whose last says
/// opaque: probe() (detect.rs, first sub-block) reports has_alpha from the
/// bytes the inventory calls "unread sub-blocks".
#[test]
fn q4_probe_reads_the_first_gce_subblock() {
    let mut g = header(1);
    let at = g.len();
    g.extend_from_slice(&[0x21, 0xF9, 4, 0x01, 0, 0, 0, 4, 0x00, 0, 0, 0, 0]);
    frame_at(&mut g, 0, 0);
    g.push(0x3B);
    let (_, f) = native(&g).unwrap();
    println!("pixel {:?}", f[0].pixels[0]);
    assert_eq!(
        f[0].pixels[0].a, 255,
        "the decoder applies the last sub-block: opaque"
    );
    let info = GifDecoderConfig::new().job().probe(&g).unwrap();
    println!("probe has_alpha {}", info.has_alpha);
    assert!(info.has_alpha, "probe() read the first sub-block");
    let i = inv(&g);
    println!("{i}");
    let first = leaf_at(&i, (at + 3) as u64);
    assert_eq!(first.disposition, D::Dropped, "{i}");
    assert!(first.detail.as_deref().unwrap().contains("probe()"), "{i}");
}

/// Frames after a rejected frame: probe() still counts them and reads their
/// GCE and interlace flag; the inventory calls them Skipped / Dropped.
#[test]
fn q5_probe_reads_frames_after_a_rejected_frame() {
    let mut g = header(1);
    g.extend(netscape(0));
    frame_at(&mut g, 0, 0); // ok, covers the 1x1 screen
    frame_at(&mut g, 5, 0); // outside the screen: decoder stops here
    let g3 = g.len();
    g.extend(gce(0, true));
    let d3 = g.len();
    frame_at(&mut g, 0, 0x40); // interlaced
    g.push(0x3B);
    let info = GifDecoderConfig::new().job().probe(&g).unwrap();
    println!(
        "probe: has_alpha {} progressive {:?} seq {:?}",
        info.has_alpha, info.is_progressive, info.sequence
    );
    assert!(info.has_alpha);
    let frames = match info.sequence {
        ImageSequence::Animation { frame_count, .. } => frame_count,
        _ => None,
    };
    assert_eq!(frames, Some(3));
    let i = inv(&g);
    println!("{i}");
    for (at, d) in [(g3, D::Dropped), (d3, D::Skipped)] {
        let p = part_at(&i, at as u64);
        assert_eq!(p.disposition, d, "{i}");
        assert!(p.detail.as_deref().unwrap().contains("probe()"), "{i}");
    }
}

/// A Graphic Control with the wrong length is fatal in the gif crate, but the
/// walker keeps the frames after it consumed.
#[test]
fn q6_bad_gce_is_fatal_but_later_frames_stay_consumed() {
    let mut g = header(2);
    g.extend(netscape(0));
    frame_at(&mut g, 0, 0);
    g.extend_from_slice(&[0x21, 0xF9, 2, 0, 0, 0]); // wrong length
    let d2 = g.len();
    frame_at(&mut g, 1, 0);
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
    println!("animation decoder: {n} frames, then {err:?}");
    assert_eq!(n, 1);
    assert!(err.is_some());
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, d2 as u64).disposition, D::Skipped, "{i}");
}

/// The global table is "used" if any frame lacks a local table, even one the
/// decoder never reaches.
#[test]
fn q7_gct_counted_used_by_an_unreachable_frame() {
    let mut a = header(1);
    a.extend(netscape(0));
    // frame 1 with a local table
    a.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x80, 1, 2, 3, 4, 5, 6]);
    a.extend_from_slice(LZW_1X1);
    frame_at(&mut a, 5, 0); // outside the screen, no local table: never decoded
    a.push(0x3B);
    let mut b = a.clone();
    b[13..19].copy_from_slice(b"SECRET");
    let fa = GifDecoderConfig::new().decode(&a).unwrap();
    let fb = GifDecoderConfig::new().decode(&b).unwrap();
    assert_eq!(
        fa.pixels().contiguous_bytes(),
        fb.pixels().contiguous_bytes()
    );
    let i = inv(&b);
    println!("{i}");
    assert_eq!(part_at(&i, 13).disposition, D::Dropped, "{i}");
}

/// Memory with a small max_memory (run alone under /usr/bin/time -v).
#[test]
fn q8_small_max_memory() {
    let n: usize = std::env::var("Q8_PAIRS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mem: u64 = std::env::var("Q8_MEM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16 << 20);
    if n == 0 {
        return;
    }
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    for _ in 0..n {
        g.extend_from_slice(&[0x05, 0x00]);
    }
    let limits = zencodec::ResourceLimits::none().with_max_memory(mem);
    let t = std::time::Instant::now();
    match GifDecoderConfig::new()
        .job()
        .with_limits(limits)
        .inventory(&g)
    {
        Ok(Some(i)) => {
            i.validate().unwrap();
            println!("ok parts {} in {:?}", i.parts().len(), t.elapsed())
        }
        Ok(None) => unreachable!(),
        Err(e) => println!("err {e} in {:?}", t.elapsed()),
    }
}

/// Data that ends before both the end code and the pixel count (`Exhausted`):
/// the decoder errors, the inventory says ImageData and keeps going.
#[test]
fn q2b_exhausted_lzw_is_fatal_but_consumed() {
    let mut g = header(2);
    g.extend(netscape(0));
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 1, 0, 0]);
    let f1 = g.len();
    g.extend_from_slice(&[0x02, 0x01, 0x04, 0x00]); // clear, index 0, no end code
    let f2 = g.len();
    frame_at(&mut g, 1, 0);
    g.push(0x3B);
    let e = native(&g).expect_err("decoder rejects the short frame");
    println!("native: {e}");
    let i = inv(&g);
    println!("{i}");
    assert_eq!(part_at(&i, f1 as u64).disposition, D::Malformed, "{i}");
    assert_eq!(part_at(&i, f2 as u64).disposition, D::Skipped, "{i}");
}

/// Cost of the two-pass walk on one large frame (run alone, release-ish).
#[test]
fn q9_large_frame_walk_time() {
    let side: u16 = std::env::var("Q9_SIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if side == 0 {
        return;
    }
    let mut rng = Rng(7);
    let n = usize::from(side) * usize::from(side);
    let px: Vec<u8> = (0..n).map(|_| (rng.next() & 0xFF) as u8).collect();
    let (g, _, _, _) = lzw_gif(8, side, side, &px, 0, b"");
    let t = std::time::Instant::now();
    let i = inv(&g);
    println!(
        "{} bytes, {} parts, inventory+validate {:?}",
        g.len(),
        i.parts().len(),
        t.elapsed()
    );
}
