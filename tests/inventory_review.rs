//! Reproducers from the independent review of the GIF inventory (PR 23), with
//! the assertions that hold once each finding is fixed. The principle: a part
//! reported as consumed must not contain bytes the decoder never reads.
#![cfg(feature = "std")]

mod common;

use std::borrow::Cow;

use common::LZW_1X1;
use enough::Unstoppable;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, MetadataKind, Part, PartTag};
use zengif::GifDecoderConfig;

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

/// `w` x 1 screen, global table of 2 (red, blue).
fn header(w: u8) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&[w, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    g
}

fn frame(g: &mut Vec<u8>, left: u8) {
    g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    g.extend_from_slice(LZW_1X1);
}

fn netscape(loops: u16) -> Vec<u8> {
    common::app_ext(b"NETSCAPE2.0", &[&[1, loops as u8, (loops >> 8) as u8]])
}

fn gce(delay: u8, transparent: bool) -> Vec<u8> {
    vec![0x21, 0xF9, 4, u8::from(transparent), delay, 0, 0, 0]
}

fn native(g: &[u8]) -> (zengif::Metadata, Vec<zengif::ComposedFrame>) {
    let (m, f, _) = zengif::decode_gif(g, zengif::Limits::none(), &Unstoppable).unwrap();
    (m, f)
}

fn part_at(i: &Inventory, start: u64) -> &Part {
    i.parts()
        .iter()
        .find(|p| p.parent.is_none() && p.range.start == start)
        .unwrap_or_else(|| panic!("no top-level part at {start}\n{i}"))
}

/// The innermost part covering `at`.
fn leaf_at(i: &Inventory, at: u64) -> &Part {
    i.parts()
        .iter()
        .rfind(|p| p.range.start <= at && at < p.range.end)
        .unwrap_or_else(|| panic!("nothing covers {at}\n{i}"))
}

fn find(hay: &[u8], needle: &[u8]) -> u64 {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .expect("needle present") as u64
}

// ── finding 2: bytes after the LZW end code / pixel count ────────────

#[test]
fn lzw_tail_is_not_image_data() {
    let secret = b"SERIAL=CAM-0042-SECRET";
    let mut clean = header(1);
    frame(&mut clean, 0);
    clean.push(0x3B);

    let mut dirty = header(1);
    dirty.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    dirty.extend_from_slice(&[0x02, 0x02, 0x44, 0x01]);
    dirty.push(secret.len() as u8);
    dirty.extend_from_slice(secret);
    dirty.extend_from_slice(&[0x00, 0x3B]);

    // Same pixels with and without the tail.
    let (_, a) = native(&clean);
    let (_, b) = native(&dirty);
    assert_eq!(a[0].pixels, b[0].pixels);

    let i = inv(&dirty);
    let holder = leaf_at(&i, find(&dirty, secret));
    assert!(!holder.disposition.is_consumed(), "{i}");
    assert_eq!(holder.disposition, D::Dropped);
    // The real data (min code size, the sub-block with the end code) stays ImageData.
    assert_eq!(leaf_at(&i, 29).disposition, D::ImageData, "{i}");

    // A clean file has no tail part at all.
    assert!(
        !inv(&clean)
            .parts()
            .iter()
            .any(|p| p.label.as_deref() == Some("image data tail"))
    );
}

#[test]
fn data_after_the_pixel_count_is_not_image_data() {
    // 1x1 frame whose stream is clear, 0, 0 (a second pixel the buffer has no
    // room for), end code: the extra code and the end code are never read.
    // 3-bit codes: 100 000 000 101 -> bytes 0x04, 0x0A... build from bits.
    let codes: [u16; 5] = [4, 0, 0, 0, 5];
    let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
    for c in codes {
        acc |= u32::from(c) << n;
        n += 3;
        while n >= 8 {
            bytes.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        bytes.push(acc as u8);
    }
    let secret = b"HIDDEN-AFTER-PIXELS";
    let mut g = header(1);
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00, 0x02]);
    g.push(bytes.len() as u8);
    g.extend_from_slice(&bytes);
    g.push(secret.len() as u8);
    g.extend_from_slice(secret);
    g.extend_from_slice(&[0x00, 0x3B]);
    assert_eq!(native(&g).1.len(), 1);
    let i = inv(&g);
    assert!(
        !leaf_at(&i, find(&g, secret)).disposition.is_consumed(),
        "{i}"
    );
    assert!(i.parts().iter().any(|p| p.disposition == D::Dropped), "{i}");
}

#[test]
fn invalid_lzw_code_is_malformed() {
    let mut g = header(1);
    g.extend_from_slice(&[
        0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00, 0x02, 0x01, 0xFF, 0x00, 0x3B,
    ]);
    assert!(GifDecoderConfig::new().decode(&g).is_err());
    let i = inv(&g);
    assert_eq!(leaf_at(&i, 29).disposition, D::Malformed, "{i}");
}

// ── findings 1 and 4: extra sub-blocks, overwritten and orphaned ─────

#[test]
fn extra_subblocks_in_consumed_extensions_are_dropped() {
    let secret: &[u8] = b"OWNER=Jane Doe; GPS=47.6,-122.3";
    let build = |dirty: bool| {
        let mut g = header(2);
        g.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        g.extend_from_slice(b"NETSCAPE2.0");
        g.extend_from_slice(&[3, 1, 4, 0]);
        if dirty {
            g.push(secret.len() as u8);
            g.extend_from_slice(secret);
        }
        g.push(0);
        g.extend_from_slice(&[0x21, 0xF9]);
        if dirty {
            g.push(secret.len() as u8);
            g.extend_from_slice(secret);
        }
        g.extend_from_slice(&[4, 0, 9, 0, 0, 0]);
        frame(&mut g, 0);
        g.extend(gce(9, false));
        frame(&mut g, 1);
        g.push(0x3B);
        g
    };
    let (clean, dirty) = (build(false), build(true));
    let (mc, fc) = native(&clean);
    let (md, fd) = native(&dirty);
    assert_eq!(mc.repeat, md.repeat);
    for (x, y) in fc.iter().zip(&fd) {
        assert_eq!((x.delay, &x.pixels), (y.delay, &y.pixels));
    }
    let i = inv(&dirty);
    let first = find(&dirty, secret);
    let second = dirty
        .windows(secret.len())
        .enumerate()
        .filter(|(_, w)| *w == secret)
        .map(|(k, _)| k as u64)
        .nth(1)
        .unwrap();
    for at in [first, second] {
        assert_eq!(leaf_at(&i, at).disposition, D::Dropped, "{i}");
    }
    // The sub-blocks the decoder does read are still metadata.
    let ns_loop = 19 + 2 + 12; // introducer, label, ident sub-block
    assert_eq!(leaf_at(&i, ns_loop as u64 + 1).disposition, ANIM, "{i}");
    let used_gce = i
        .parts()
        .iter()
        .filter(|p| p.disposition == ANIM && p.label.as_deref() == Some("Graphic Control"))
        .count();
    assert_eq!(used_gce, 2, "{i}");
    // A clean file keeps both extensions as single parts.
    let c = inv(&clean);
    assert_eq!(part_at(&c, 19).disposition, ANIM);
}

#[test]
fn overwritten_gce_is_dropped() {
    let mut g = header(2);
    g.extend(netscape(0));
    g.extend(gce(100, false)); // overwritten
    g.extend(gce(7, false));
    frame(&mut g, 0);
    g.extend(gce(9, false));
    frame(&mut g, 1);
    g.push(0x3B);
    let (_, f) = native(&g);
    assert_eq!((f[0].delay, f[1].delay), (7, 9));
    let first = (13 + 6 + 19) as u64;
    let i = inv(&g);
    assert_eq!(part_at(&i, first).disposition, D::Dropped, "{i}");
    assert_eq!(part_at(&i, first + 8).disposition, ANIM, "{i}");
}

#[test]
fn gce_after_the_last_frame_is_dropped() {
    let mut b = header(2);
    b.extend(netscape(0));
    frame(&mut b, 0);
    frame(&mut b, 1);
    let at = b.len() as u64;
    b.extend(gce(55, true));
    b.push(0x3B);
    let i = inv(&b);
    assert_eq!(part_at(&i, at).disposition, D::Dropped, "{i}");
}

#[test]
fn overwritten_netscape_is_dropped() {
    let mut g = header(2);
    g.extend(netscape(3));
    g.extend(netscape(7));
    frame(&mut g, 0);
    frame(&mut g, 1);
    g.push(0x3B);
    let (m, _) = native(&g);
    assert_eq!(m.repeat, zengif::Repeat::Count(7));
    let i = inv(&g);
    assert_eq!(part_at(&i, 19).disposition, D::Dropped, "{i}");
    assert_eq!(part_at(&i, 19 + 19).disposition, ANIM, "{i}");
}

/// NETSCAPE2.0 after the first image: `probe()` reads it, the gif crate does
/// not. The part stays consumed and says which path reads it.
#[test]
fn netscape_after_the_first_frame_names_its_reader() {
    let mut g = header(2);
    frame(&mut g, 0);
    let at = g.len() as u64;
    g.extend(netscape(5));
    frame(&mut g, 1);
    g.push(0x3B);
    let i = inv(&g);
    let p = part_at(&i, at);
    assert_eq!(p.disposition, ANIM);
    assert!(p.detail.as_deref().unwrap().contains("probe()"));
}

#[test]
fn gce_alpha_in_a_later_subblock_is_the_used_one() {
    // The gif crate applies the last sub-block (transparent index 0); the
    // earlier 3-byte sub-block is dropped.
    let mut g = header(1);
    g.extend_from_slice(&[0x21, 0xF9, 3, 0, 0, 0, 4, 0x01, 0, 0, 0, 0]);
    frame(&mut g, 0);
    g.push(0x3B);
    let i = inv(&g);
    assert_eq!(leaf_at(&i, 21).disposition, D::Dropped, "{i}");
    assert_eq!(leaf_at(&i, 19 + 6).disposition, ANIM, "{i}");
}

// ── finding 3: probe() and the decoder disagree inside unknown blocks ──

#[test]
fn bytes_probe_reads_differently_are_malformed_not_unknown() {
    let mut g = header(2);
    let stray = g.len();
    g.push(0x00);
    g.extend(netscape(5));
    g.extend_from_slice(&[0x21, 0xFE, 12]);
    g.extend_from_slice(b"resync-here!");
    g.push(0x00);
    frame(&mut g, 0);
    frame(&mut g, 1);
    g.push(0x3B);
    assert_eq!(native(&g).1.len(), 2);
    let i = inv(&g);
    let p = part_at(&i, stray as u64);
    assert_eq!(p.tag, PartTag::Code(0));
    assert_eq!(p.disposition, D::Malformed, "{i}");
    assert!(p.range.end >= (stray + 20) as u64);
    assert!(p.detail.as_deref().unwrap().contains("probe()"));

    // Transparency hidden behind a stray byte.
    let mut g = header(1);
    g.push(0x07);
    g.extend(gce(0, true));
    g.extend_from_slice(&[0x21, 0xFE, 23]);
    g.extend_from_slice(b".......................");
    g.push(0x00);
    frame(&mut g, 0);
    g.push(0x3B);
    assert!(GifDecoderConfig::new().job().probe(&g).unwrap().has_alpha);
    assert_eq!(part_at(&inv(&g), 19).disposition, D::Malformed);
}

// ── findings 5 and 6: unused table, rejected frames ──────────────────

#[test]
fn unused_global_table_is_dropped() {
    let mut a = header(1);
    a.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x80, 1, 2, 3, 4, 5, 6]);
    a.extend_from_slice(LZW_1X1);
    a.push(0x3B);
    let mut b = a.clone();
    b[13..19].copy_from_slice(b"SECRET");
    assert_eq!(native(&a).1[0].pixels, native(&b).1[0].pixels);
    assert_eq!(part_at(&inv(&b), 13).disposition, D::Dropped);
    // A frame without a local table reads it.
    let mut c = header(1);
    frame(&mut c, 0);
    c.push(0x3B);
    assert_eq!(part_at(&inv(&c), 13).disposition, D::ImageData);
}

#[test]
fn frame_without_any_colour_table_is_malformed() {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&[1, 0, 1, 0, 0x00, 0, 0]);
    frame(&mut g, 0);
    g.push(0x3B);
    assert!(GifDecoderConfig::new().decode(&g).is_err());
    let i = inv(&g);
    assert_eq!(part_at(&i, 13).disposition, D::Malformed, "{i}");
    assert!(!part_at(&i, 23).disposition.is_consumed(), "{i}");
}

#[test]
fn frames_after_a_rejected_frame_are_not_consumed() {
    let mut g = header(2);
    frame(&mut g, 0);
    frame(&mut g, 5); // outside the 2-pixel screen: rejected
    let third = g.len() as u64;
    frame(&mut g, 1);
    g.push(0x3B);
    let i = inv(&g);
    assert_eq!(part_at(&i, 29 + 5).disposition, D::Malformed, "{i}");
    assert_eq!(part_at(&i, third).disposition, D::Skipped, "{i}");
    assert_eq!(part_at(&i, third + 10).disposition, D::Skipped, "{i}");
}

// ── from the review, unchanged in outcome ────────────────────────────

#[test]
fn leading_junk_and_lengths_past_eof() {
    let mut g = b"JUNK".to_vec();
    g.extend(header(1));
    frame(&mut g, 0);
    g.push(0x3B);
    let i = inv(&g);
    assert_eq!(i.parts().len(), 1);
    assert_eq!(i.parts()[0].disposition, D::Malformed);

    let mut g = header(1);
    frame(&mut g, 0);
    g.extend_from_slice(&[0x21, 0xFE, 0xFF, b'a', b'b']);
    let i = inv(&g);
    let last = i.parts().last().unwrap();
    assert_eq!(last.disposition, D::Malformed);
    assert_eq!(last.range.end, g.len() as u64);
}

#[test]
fn xmp_early_terminator_leaves_the_rest_unconsumed() {
    let mut g = header(1);
    g.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    g.extend_from_slice(b"XMP DataXMP");
    g.extend_from_slice(&[3, b'<', b'x', b'>', 0x00]);
    g.extend_from_slice(b"HIDDEN AFTER EARLY TERMINATOR");
    g.push(0x01);
    g.extend((1..=255u8).rev());
    g.extend_from_slice(&[0, 0]);
    frame(&mut g, 0);
    g.push(0x3B);
    let i = inv(&g);
    assert!(!leaf_at(&i, find(&g, b"HIDDEN")).disposition.is_consumed());
}

// ── finding 7: the memory limit bounds the inventory ─────────────────

fn blocks(pairs: usize) -> Vec<u8> {
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    for _ in 0..pairs {
        g.extend_from_slice(&[0x05, 0x00]);
    }
    g
}

#[test]
fn max_memory_bounds_the_part_count() {
    let limits = || zencodec::ResourceLimits::none().with_max_memory(16 << 20);
    // 16 MiB at 400 bytes per part is about 41,900 parts.
    let ok = blocks(20_000);
    assert!(
        GifDecoderConfig::new()
            .job()
            .with_limits(limits())
            .inventory(&ok)
            .is_ok()
    );
    let big = blocks(100_000);
    let err = GifDecoderConfig::new()
        .job()
        .with_limits(limits())
        .inventory(&big)
        .expect_err("over the memory-derived part cap");
    assert!(err.to_string().to_lowercase().contains("memory"), "{err}");
    // Without a limit the same file is fine.
    assert!(GifDecoderConfig::new().job().inventory(&big).is_ok());
}

/// Peak memory at the default cap (run alone under `/usr/bin/time -v`).
#[test]
fn cap_memory_probe() {
    let n: usize = std::env::var("INVENTORY_CAP_PAIRS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if n == 0 {
        return;
    }
    let g = blocks(n);
    let r = GifDecoderConfig::new().job().inventory(&g);
    println!("{}", if r.is_ok() { "ok" } else { "capped" });
}

// ── decoder agreement for the new splits ─────────────────────────────

#[test]
fn animation_decoder_still_agrees_on_the_fixture() {
    let g = common::fixture();
    let mut d = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&g), &[])
        .unwrap();
    let mut n = 0;
    while d.render_next_frame(None).unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 2);
    assert_eq!(d.loop_count(), Some(3));
}
