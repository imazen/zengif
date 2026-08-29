//! A frame smaller than the logical screen leaves the surrounding canvas
//! transparent, whether or not any Graphic Control Extension says so.
//!
//! `GifProbe::has_transparency` was set only from the GCE transparent-colour
//! flag, so such a GIF reported "opaque". `ImageInfo::has_alpha` is derived from
//! it, and a caller who trusts that asks for `RGB8_SRGB` — at which point
//! `negotiate_format` drops the alpha channel and the transparent border
//! silently becomes opaque black. Real transparency, lost.
//!
//! These build the GIFs byte by byte so the exact structure under test — a
//! sub-screen frame, no GCE at all — is explicit rather than hoped for.

#![cfg(feature = "std")]

use enough::Unstoppable;
use zengif::{Limits, Rgba, decode_gif};

/// Build a GIF with a `screen_w`×`screen_h` logical screen containing one
/// `frame_w`×`frame_h` frame at (`left`, `top`).
///
/// `with_gce` controls whether a Graphic Control Extension is emitted at all.
/// With `false` there is no GCE, so nothing in the file declares transparency —
/// yet every canvas pixel outside the frame is transparent once composited.
fn sub_screen_gif(
    screen_w: u16,
    screen_h: u16,
    left: u16,
    top: u16,
    frame_w: u16,
    frame_h: u16,
    with_gce: bool,
) -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&screen_w.to_le_bytes());
    g.extend_from_slice(&screen_h.to_le_bytes());
    g.push(0x80); // global colour table, 2 entries
    g.push(0); // background colour index
    g.push(0); // pixel aspect ratio
    g.extend_from_slice(&[0xFF, 0x00, 0x00]); // colour 0: red
    g.extend_from_slice(&[0x00, 0x00, 0xFF]); // colour 1: blue

    if with_gce {
        // Graphic Control Extension with the transparency flag CLEAR.
        g.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    g.push(0x2C); // image descriptor
    g.extend_from_slice(&left.to_le_bytes());
    g.extend_from_slice(&top.to_le_bytes());
    g.extend_from_slice(&frame_w.to_le_bytes());
    g.extend_from_slice(&frame_h.to_le_bytes());
    g.push(0x00); // no local colour table, not interlaced

    // Uncompressed-style LZW: 2-bit codes, emit a clear code before each pixel
    // so no dictionary state is needed. Enough for the tiny frames here.
    let pixel_count = frame_w as usize * frame_h as usize;
    g.push(0x02); // LZW minimum code size
    let mut bits: Vec<u8> = Vec::new();
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    let push_code = |code: u32, width: u32, bits: &mut Vec<u8>, acc: &mut u32, nbits: &mut u32| {
        *acc |= code << *nbits;
        *nbits += width;
        while *nbits >= 8 {
            bits.push((*acc & 0xFF) as u8);
            *acc >>= 8;
            *nbits -= 8;
        }
    };
    for _ in 0..pixel_count {
        push_code(4, 3, &mut bits, &mut acc, &mut nbits); // clear
        push_code(1, 3, &mut bits, &mut acc, &mut nbits); // colour index 1 (blue)
    }
    push_code(5, 3, &mut bits, &mut acc, &mut nbits); // end of information
    if nbits > 0 {
        bits.push((acc & 0xFF) as u8);
    }
    for chunk in bits.chunks(255) {
        g.push(chunk.len() as u8);
        g.extend_from_slice(chunk);
    }
    g.push(0x00); // block terminator
    g.push(0x3B); // trailer
    g
}

/// Decode and report whether any composited pixel is not fully opaque.
fn has_transparent_pixels(data: &[u8]) -> bool {
    let (_, frames, _) = decode_gif(data, Limits::none(), &Unstoppable).expect("must decode");
    frames
        .iter()
        .any(|f| f.pixels.iter().any(|p: &Rgba| p.a != 255))
}

/// The premise: a sub-screen frame really does composite to a transparent
/// border, with or without a GCE. If this ever stops being true the tests below
/// stop meaning anything.
#[test]
fn sub_screen_frame_composites_transparent_border() {
    for with_gce in [false, true] {
        let data = sub_screen_gif(8, 8, 0, 0, 4, 4, with_gce);
        assert!(
            has_transparent_pixels(&data),
            "with_gce={with_gce}: a 4x4 frame on an 8x8 screen must leave transparent pixels"
        );
    }
}

/// **The regression.** The probe must report transparency for a canvas that
/// composites transparent pixels, not merely for one whose GCE says so.
#[test]
fn probe_reports_transparency_for_sub_screen_frames() {
    /// label, screen w/h, frame left/top, frame w/h, whether a GCE is emitted.
    type Case = (&'static str, u16, u16, u16, u16, u16, u16, bool);
    let cases: [Case; 4] = [
        ("no GCE, frame smaller than screen", 8, 8, 0, 0, 4, 4, false),
        ("GCE present but flag clear", 8, 8, 0, 0, 4, 4, true),
        ("frame offset leaves a border", 8, 8, 2, 2, 4, 4, false),
        ("frame narrower than screen", 8, 4, 0, 0, 4, 4, false),
    ];
    for (label, sw, sh, l, t, fw, fh, gce) in cases {
        let data = sub_screen_gif(sw, sh, l, t, fw, fh, gce);
        assert!(
            has_transparent_pixels(&data),
            "{label}: fixture must actually composite transparency"
        );
        let probe =
            zengif::detect::probe(&data).unwrap_or_else(|e| panic!("{label}: probe failed: {e:?}"));
        assert!(
            probe.has_transparency,
            "{label}: probe reported opaque for a canvas with transparent pixels"
        );
    }
}

/// A frame that covers the whole screen with no GCE has nothing transparent —
/// the fix must not blanket-report every GIF as transparent, or the RGB8
/// negotiation it protects would never be taken.
#[test]
fn probe_reports_opaque_for_full_screen_frames() {
    let data = sub_screen_gif(4, 4, 0, 0, 4, 4, false);
    assert!(
        !has_transparent_pixels(&data),
        "a full-screen opaque frame must composite fully opaque"
    );
    let probe = zengif::detect::probe(&data).expect("probe");
    assert!(
        !probe.has_transparency,
        "a full-screen frame with no transparency must still report opaque"
    );
}

/// The checked-in corpus must keep agreeing: nothing that composites
/// transparent pixels may be reported opaque.
#[test]
fn corpus_probe_transparency_matches_decoded_pixels() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/codec-corpus");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("gif") {
            continue;
        }
        let data = std::fs::read(&path).expect("read");
        let (Ok(probe), Ok((_, frames, _))) = (
            zengif::detect::probe(&data),
            decode_gif(&data, Limits::none(), &Unstoppable),
        ) else {
            continue; // deliberately malformed fixtures live here too
        };
        checked += 1;
        let actually_transparent = frames
            .iter()
            .any(|f| f.pixels.iter().any(|p: &Rgba| p.a != 255));
        if actually_transparent {
            assert!(
                probe.has_transparency,
                "{}: composites transparent pixels but probe reported opaque — \
                 negotiate_format would strip them to RGB8",
                path.display()
            );
        }
    }
    assert!(checked > 3, "expected several decodable corpus GIFs");
}
