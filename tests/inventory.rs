//! Structural inventory (`DecodeJob::inventory`) conformance and pinned fixtures.
//!
//! Depends on the `zencodec` inventory API (imazen/zencodec#133).
#![cfg(feature = "std")]

use enough::Unstoppable;
use zencodec::decode::{DecodeJob, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, MetadataKind, PartKind, PartTag};
use zengif::GifDecoderConfig;

mod common;
use common::{LZW_1X1, app_ext, fixture};

fn inventory_of(data: &[u8]) -> Inventory {
    GifDecoderConfig::new()
        .job()
        .inventory(data)
        .expect("inventory")
        .expect("Some")
}

/// (parent index, kind, tag, start, end, disposition, label) per part.
type Row = (
    Option<usize>,
    PartKind,
    PartTag,
    u64,
    u64,
    D,
    Option<String>,
);

fn rows(inv: &Inventory) -> Vec<Row> {
    inv.parts()
        .iter()
        .map(|p| {
            (
                p.parent().map(|id| id.index()),
                p.kind(),
                p.tag().clone(),
                p.range().start,
                p.range().end,
                p.disposition(),
                p.label().as_ref().map(|l| l.to_string()),
            )
        })
        .collect()
}

fn decode_frames(g: &[u8]) -> Result<usize, String> {
    zengif::decode_gif(g, zengif::Limits::none(), &Unstoppable)
        .map(|(_, f, _)| f.len())
        .map_err(|e| e.to_string())
}

fn conformance_dir() -> std::path::PathBuf {
    codec_corpus::Corpus::new()
        .expect("codec-corpus")
        .get("gif-conformance")
        .expect("gif-conformance set")
}

fn gifs_in(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(gifs_in(&p));
        } else if p.extension().is_some_and(|x| x == "gif") {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn repo_corpus() -> Vec<std::path::PathBuf> {
    gifs_in(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus"))
}

// ── pinned fixture ───────────────────────────────────────────────────

#[test]
fn fixture_parts_are_pinned() {
    let g = fixture();
    let inv = inventory_of(&g);
    inv.validate().unwrap();
    let blk = PartKind::Block;
    let c = PartTag::Code;
    let anim = D::Metadata(MetadataKind::Animation);
    let s = |x: &str| Some(x.to_string());
    let expected: Vec<Row> = vec![
        (
            None,
            PartKind::Header,
            PartTag::None,
            0,
            13,
            D::Structure,
            s("GIF89a"),
        ),
        (
            None,
            blk,
            PartTag::None,
            13,
            19,
            D::ImageData,
            s("global colour table"),
        ),
        (None, blk, c(0xFF), 19, 38, anim, s("NETSCAPE2.0")),
        (None, blk, c(0xFF), 38, 57, D::Unknown, s("ANIMEXTS1.0")),
        (None, blk, c(0xFE), 57, 67, D::Skipped, s("Comment")),
        (None, blk, c(0x01), 67, 86, D::Skipped, s("Plain Text")),
        (None, blk, c(0xFF), 86, 104, D::Unknown, s("ACMEPRIV1.0")),
        (None, blk, c(0x99), 104, 109, D::Unknown, None),
        (None, blk, c(0xFF), 109, 433, D::Skipped, s("XMP DataXMP")),
        (
            Some(8),
            blk,
            PartTag::None,
            123,
            175,
            D::Skipped,
            s("XMP packet"),
        ),
        (
            Some(8),
            blk,
            PartTag::None,
            175,
            433,
            D::Padding,
            s("XMP magic trailer"),
        ),
        (None, blk, c(0xFF), 433, 453, D::Skipped, s("ICCRGBG1012")),
        (None, blk, c(0xF9), 453, 461, anim, s("Graphic Control")),
        (
            None,
            blk,
            c(0x2C),
            461,
            471,
            D::Structure,
            s("image descriptor"),
        ),
        (
            None,
            blk,
            PartTag::None,
            471,
            477,
            D::ImageData,
            s("local colour table"),
        ),
        (
            None,
            blk,
            PartTag::None,
            477,
            482,
            D::ImageData,
            s("image data"),
        ),
        (
            None,
            blk,
            c(0xF9),
            482,
            490,
            D::Structure,
            s("Graphic Control"),
        ),
        (
            Some(16),
            blk,
            PartTag::None,
            484,
            488,
            anim,
            s("Graphic Control"),
        ),
        (
            Some(16),
            blk,
            PartTag::None,
            488,
            489,
            D::Dropped,
            s("unread byte"),
        ),
        (
            Some(16),
            blk,
            PartTag::None,
            489,
            490,
            D::Structure,
            s("terminator"),
        ),
        (
            None,
            blk,
            c(0x2C),
            490,
            500,
            D::Structure,
            s("image descriptor"),
        ),
        (
            None,
            blk,
            PartTag::None,
            500,
            505,
            D::ImageData,
            s("image data"),
        ),
        (None, blk, c(0x05), 505, 510, D::Unknown, None),
        (None, blk, c(0x3B), 510, 511, D::Structure, s("trailer")),
        (
            None,
            PartKind::Trailer,
            PartTag::None,
            511,
            521,
            D::Trailing,
            None,
        ),
    ];
    assert_eq!(rows(&inv), expected, "{inv}");
    // The XMP extension declares its body, so the children tile it.
    assert_eq!(inv.parts()[8].body(), Some(123..433));
}

#[test]
fn fixture_matches_the_decoder() {
    let g = fixture();
    // Two frames, loop count 3 from NETSCAPE2.0, no comments surfaced.
    let (meta, frames, _) = zengif::decode_gif(&g, zengif::Limits::none(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(meta.repeat, zengif::Repeat::Count(3));
    assert!(meta.comments.is_empty(), "comments are never surfaced");
    let inv = inventory_of(&g);
    let image_data = inv
        .parts()
        .iter()
        .filter(|p| p.label() == Some("image data") && p.disposition() == D::ImageData)
        .count();
    assert_eq!(image_data, frames.len());
}

/// An unknown block introducer between frames: the decoder resynchronises the
/// way the inventory says, and still finds the second frame.
#[test]
fn unknown_introducer_between_frames_matches_the_decoder() {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[4, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    let frame = |g: &mut Vec<u8>, left: u8| {
        g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, 0x00]);
        g.extend_from_slice(LZW_1X1);
    };
    frame(&mut g, 0);
    // Malformed, not Unknown: probe() steps one byte and would read `2C` as an image.
    // The unknown byte is consumed alone; `03 2C AA BB 00` is then read as one
    // sub-block of 3 bytes plus the terminator, so the `2C` is not an image.
    g.extend_from_slice(&[0x05, 0x03, 0x2C, 0xAA, 0xBB, 0x00]);
    frame(&mut g, 1);
    g.push(0x3B);
    assert_eq!(decode_frames(&g), Ok(2));
    let inv = inventory_of(&g);
    inv.validate().unwrap();
    let r = rows(&inv);
    assert!(
        r.iter()
            .any(|x| x.2 == PartTag::Code(0x05) && x.3 == 34 && x.4 == 40 && x.5 == D::Malformed),
        "{inv}"
    );
    assert_eq!(
        r.iter().filter(|x| x.2 == PartTag::Code(0x2C)).count(),
        2,
        "{inv}"
    );
}

/// A stray `00` between blocks: the decoder swallows the following bytes as a
/// pseudo extension and ends the stream, and the inventory says the same.
#[test]
fn stray_zero_byte_swallows_the_rest_like_the_decoder() {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[4, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    let frame = |g: &mut Vec<u8>, left: u8| {
        g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, 0x00]);
        g.extend_from_slice(LZW_1X1);
    };
    frame(&mut g, 0);
    g.push(0x00);
    frame(&mut g, 1);
    g.push(0x3B);
    assert_eq!(decode_frames(&g), Ok(1), "the decoder stops after frame 1");
    let inv = inventory_of(&g);
    inv.validate().unwrap();
    let last = inv.parts().last().unwrap();
    assert_eq!(*last.tag(), PartTag::Code(0));
    assert_eq!(last.disposition(), D::Malformed, "{inv}");
    assert_eq!(last.range(), 34..g.len() as u64);
}

// ── suspected bugs (see the inventory lane report) ───────────────────

/// `detect::probe` and the decoder disagree about a stray `00` between frames:
/// the probe reports 2 frames, the decoder yields 1.
#[test]
fn probe_and_decoder_disagree_about_a_stray_zero_byte() {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[4, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    for left in [0u8, 1] {
        g.extend_from_slice(&[0x2C, left, 0, 0, 0, 1, 0, 1, 0, 0x00]);
        g.extend_from_slice(LZW_1X1);
        if left == 0 {
            g.push(0x00);
        }
    }
    g.push(0x3B);
    assert_eq!(zengif::detect::probe(&g).unwrap().frame_count, 2);
    assert_eq!(decode_frames(&g), Ok(1));
}

/// ANIMEXTS1.0 carries a loop count that browsers honour; zengif ignores it.
#[test]
fn animexts_loop_count_is_ignored() {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0, 255, 0, 0, 0, 0, 255]);
    g.extend(app_ext(b"ANIMEXTS1.0", &[&[1, 3, 0]]));
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    g.extend_from_slice(LZW_1X1);
    g.push(0x3B);
    let (meta, _, _) = zengif::decode_gif(&g, zengif::Limits::none(), &Unstoppable).unwrap();
    // Same as a file with no loop extension at all: play once.
    assert_eq!(
        meta.repeat,
        zengif::Repeat::Once,
        "ANIMEXTS1.0 loop count 3 is not read"
    );
    assert_eq!(zengif::detect::probe(&g).unwrap().repeat, None);
}

/// A well-formed XMP extension (packet + 258-byte ramp) is consumed correctly
/// by the length-walking probe, so it is not a bug there.
#[test]
fn xmp_extension_does_not_confuse_the_probe() {
    let g = fixture();
    let probe = zengif::detect::probe(&g).unwrap();
    assert_eq!(probe.frame_count, 2);
    assert_eq!(probe.repeat, Some(3));
}

// ── robustness of the walker itself ──────────────────────────────────

#[test]
fn xmp_without_magic_trailer_stays_one_part() {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[1, 0, 1, 0, 0x00, 0, 0]);
    g.extend(app_ext(b"XMP DataXMP", &[b"<x/>"]));
    g.push(0x3B);
    let inv = inventory_of(&g);
    inv.validate().unwrap();
    let x = inv
        .parts()
        .iter()
        .find(|p| p.label() == Some("XMP DataXMP"))
        .unwrap();
    assert_eq!(x.disposition(), D::Skipped);
    assert!(x.body().is_none());
}

#[test]
fn bad_inputs_are_malformed_not_errors() {
    for (name, bytes) in [
        ("empty", Vec::new()),
        ("one byte", vec![0x47]),
        ("not gif", b"RIFF....WEBPVP8 ".to_vec()),
        (
            "bad version",
            b"GIF90a\x01\x00\x01\x00\x00\x00\x00".to_vec(),
        ),
        ("header only", b"GIF89a".to_vec()),
        ("truncated lsd", b"GIF89a\x01\x00\x01".to_vec()),
        ("bad min code", {
            let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
            g.extend_from_slice(&[
                0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0x0F, 0x01, 0x00, 0x00, 0x3B,
            ]);
            g
        }),
        ("frame outside screen", {
            let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
            g.extend_from_slice(&[0x2C, 5, 0, 0, 0, 1, 0, 1, 0, 0]);
            g.extend_from_slice(LZW_1X1);
            g
        }),
        ("gce wrong length", {
            let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
            g.extend_from_slice(&[0x21, 0xF9, 2, 0, 0, 0]);
            g
        }),
        ("extension label missing", {
            let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
            g.push(0x21);
            g
        }),
    ] {
        let inv = inventory_of(&bytes);
        inv.validate()
            .unwrap_or_else(|e| panic!("{name}: {e}\n{inv}"));
        if !bytes.is_empty() {
            assert!(
                inv.parts().iter().any(|p| p.disposition() == D::Malformed),
                "{name}: expected a Malformed part\n{inv}"
            );
        }
    }
}

#[test]
fn every_prefix_and_suffix_of_the_fixture_validates() {
    let g = fixture();
    for n in 0..=g.len() {
        inventory_of(&g[..n])
            .validate()
            .unwrap_or_else(|e| panic!("prefix {n}: {e}"));
    }
    for n in 0..g.len() {
        inventory_of(&g[n..])
            .validate()
            .unwrap_or_else(|e| panic!("suffix {n}: {e}"));
    }
}

#[test]
fn many_parts_stay_valid() {
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    for _ in 0..300_000 {
        g.extend_from_slice(&[0x05, 0x00]);
    }
    let inv = inventory_of(&g);
    inv.validate().unwrap();
    assert_eq!(inv.parts().len(), 300_001);
}

/// Past `DEFAULT_MAX_PARTS` the walk fails with a resource error instead of
/// growing without bound.
#[test]
fn part_cap_is_a_resource_error() {
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    for _ in 0..(zencodec::inventory::DEFAULT_MAX_PARTS as usize + 10) {
        g.extend_from_slice(&[0x05, 0x00]);
    }
    let err = GifDecoderConfig::new()
        .job()
        .inventory(&g)
        .expect_err("cap error");
    assert!(err.to_string().to_lowercase().contains("memory"), "{err}");
}

// ── conformance ──────────────────────────────────────────────────────

#[test]
fn check_inventory_on_the_fixture() {
    zencodec_testkit::check_inventory(GifDecoderConfig::new(), &fixture()).unwrap();
}

/// `check_all` does not apply (the palette quantizer is lossy, so its pixel
/// round trip cannot be exact); run the inventory check on real encoder output.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
#[test]
fn check_inventory_on_encoder_output() {
    use zengif::{EncoderConfig, FrameInput, Repeat, Rgba, encode_gif};
    let (w, h) = (24u16, 16u16);
    for frame_count in [1usize, 3] {
        let frames: Vec<FrameInput> = (0..frame_count)
            .map(|f| {
                let px: Vec<Rgba> = (0..usize::from(w) * usize::from(h))
                    .map(|i| Rgba::rgb((i * 7 + f * 60) as u8, (i * 3) as u8, (f * 90) as u8))
                    .collect();
                FrameInput::new(w, h, 10 + f as u16, px)
            })
            .collect();
        let bytes = encode_gif(
            frames,
            w,
            h,
            EncoderConfig::new().repeat(Repeat::Infinite),
            zengif::Limits::default(),
            &Unstoppable,
        )
        .expect("encode");
        zencodec_testkit::check_inventory(GifDecoderConfig::new(), &bytes)
            .unwrap_or_else(|e| panic!("{frame_count} frames: {e}"));
        assert!(
            !inventory_of(&bytes)
                .parts()
                .iter()
                .any(|p| p.label() == Some("image data tail"))
        );
    }
}

#[test]
fn check_inventory_on_the_conformance_corpus() {
    let files = gifs_in(&conformance_dir());
    assert!(files.len() >= 39, "found {} conformance files", files.len());
    let mut valid = 0;
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let rel = f.display().to_string();
        let has_image = GifDecoderConfig::new().decode(&bytes).is_ok();
        if has_image {
            zencodec_testkit::check_inventory(GifDecoderConfig::new(), &bytes)
                .unwrap_or_else(|e| panic!("{rel}: {e}"));
            valid += 1;
        } else {
            // Files the decoder rejects still need a valid inventory.
            let inv = inventory_of(&bytes);
            inv.validate()
                .unwrap_or_else(|e| panic!("{rel}: {e}\n{inv}"));
        }
    }
    assert!(valid >= 30, "only {valid} decodable conformance files");
}

#[test]
fn check_inventory_on_the_repo_corpus() {
    let files = repo_corpus();
    assert!(files.len() >= 20, "found {} repo corpus files", files.len());
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let rel = f.display().to_string();
        if GifDecoderConfig::new().decode(&bytes).is_ok() {
            zencodec_testkit::check_inventory(GifDecoderConfig::new(), &bytes)
                .unwrap_or_else(|e| panic!("{rel}: {e}"));
        } else {
            let inv = inventory_of(&bytes);
            inv.validate()
                .unwrap_or_else(|e| panic!("{rel}: {e}\n{inv}"));
        }
    }
}

/// Conformant encoders leave nothing after the LZW end code, so no corpus
/// file or encoder output has an `image data tail` part, and every frame's
/// image data is consumed.
#[test]
fn well_formed_files_have_no_image_data_tail() {
    let mut files = gifs_in(&conformance_dir().join("valid"));
    files.extend(repo_corpus());
    let mut with_tail = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        if GifDecoderConfig::new().decode(&bytes).is_err() {
            continue;
        }
        let inv = inventory_of(&bytes);
        if inv
            .parts()
            .iter()
            .any(|p| p.label() == Some("image data tail"))
        {
            with_tail.push(f.display().to_string());
        }
    }
    assert!(with_tail.is_empty(), "unexpected tails: {with_tail:?}");
}
