//! Probe-vs-decoder parity regression tests (#21, #24, #25).
#![cfg(feature = "std")]

use std::borrow::Cow;

use enough::Unstoppable;
use zencodec::ImageSequence;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zengif::{GifDecoderConfig, Limits, Repeat, Rgba, decode_gif, detect};

/// Minimal LZW stream for a 1x1 frame encoding palette index 0.
const LZW_1X1_IDX0: &[u8] = &[0x02, 0x02, 0x44, 0x01, 0x00];

fn header_1x1() -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&1u16.to_le_bytes());
    g.extend_from_slice(&1u16.to_le_bytes());
    g.extend_from_slice(&[0x80, 0x00, 0x00]); // 2-entry GCT
    g.extend_from_slice(&[255, 0, 0, 0, 0, 255]); // color 0 = red, color 1 = blue
    g
}

fn push_frame_1x1(g: &mut Vec<u8>) {
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    g.extend_from_slice(LZW_1X1_IDX0);
}

fn push_netscape(g: &mut Vec<u8>, repeats: u16) {
    g.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    g.extend_from_slice(b"NETSCAPE2.0");
    let r = repeats.to_le_bytes();
    g.extend_from_slice(&[0x03, 0x01, r[0], r[1], 0x00]);
}

/// Issue #24: GCE with multiple sub-blocks applies the last 4-byte sub-block,
/// matching the `gif` crate for both transparency (`has_alpha`) and delay (`total_duration_cs`).
#[test]
fn gce_multi_subblock_probe_matches_decoder() {
    // Exact reproducer bytes from #24: 3-byte first sub-block, 4-byte transparent second sub-block.
    let mut g = header_1x1();
    g.extend_from_slice(&[
        0x21, 0xF9, 0x03, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00,
    ]);
    push_frame_1x1(&mut g);
    g.push(0x3B);

    let (_, frames, _) = decode_gif(&g, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pixels, vec![Rgba::new(0, 0, 0, 0)]);

    let probe = detect::probe(&g).unwrap();
    assert!(probe.has_transparency);
    assert_eq!(probe.total_duration_cs, 0);

    let cfg = GifDecoderConfig::new();
    let job_probe = cfg.clone().job().probe(&g).unwrap();
    assert!(job_probe.has_alpha);
    let full_probe = cfg.clone().job().probe_full(&g).unwrap();
    assert!(full_probe.has_alpha);
    let output = cfg.decode(&g).unwrap();
    assert!(output.info().has_alpha);
    assert_eq!(
        output.pixels().to_contiguous_bytes().unwrap().as_ref(),
        &[0, 0, 0, 0]
    );

    // Last sub-block also supplies the frame delay in centiseconds.
    let mut g_delay = header_1x1();
    g_delay.extend_from_slice(&[
        0x21, 0xF9, 0x03, 0x00, 0x07, 0x00, 0x04, 0x01, 0x2A, 0x00, 0x00, 0x00,
    ]);
    push_frame_1x1(&mut g_delay);
    g_delay.push(0x3B);

    let (_, delayed_frames, _) = decode_gif(&g_delay, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(delayed_frames[0].delay, 42);
    assert_eq!(delayed_frames[0].pixels, vec![Rgba::new(0, 0, 0, 0)]);
    let p_delay = detect::probe(&g_delay).unwrap();
    assert!(p_delay.has_transparency);
    assert_eq!(p_delay.total_duration_cs, 42);

    // First 4-byte sub-block transparent + delay=50, last 4-byte sub-block opaque + delay=10.
    let mut g_override = header_1x1();
    g_override.extend_from_slice(&[
        0x21, 0xF9, 0x04, 0x01, 0x32, 0x00, 0x00, 0x04, 0x00, 0x0A, 0x00, 0x00, 0x00,
    ]);
    push_frame_1x1(&mut g_override);
    g_override.push(0x3B);

    let (_, override_frames, _) = decode_gif(&g_override, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(override_frames[0].delay, 10);
    assert_eq!(override_frames[0].pixels, vec![Rgba::rgb(255, 0, 0)]);
    let p_override = detect::probe(&g_override).unwrap();
    assert!(!p_override.has_transparency);
    assert_eq!(p_override.total_duration_cs, 10);
    assert!(
        !GifDecoderConfig::new()
            .job()
            .probe(&g_override)
            .unwrap()
            .has_alpha
    );
}

/// Issue #25: NETSCAPE2.0 after the first image descriptor (or after an unknown
/// block introducer before the first image) is ignored by the `gif` crate and
/// must also be ignored by `probe()`.
#[test]
fn netscape_after_first_non_extension_matches_animation_decoder() {
    // NETSCAPE2.0 placed between frame 1 and frame 2.
    let mut after_frame1 = header_1x1();
    push_frame_1x1(&mut after_frame1);
    push_netscape(&mut after_frame1, 4); // would mean 5 total plays if read
    push_frame_1x1(&mut after_frame1);
    after_frame1.push(0x3B);

    let (meta, frames, _) = decode_gif(&after_frame1, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(meta.repeat, Repeat::Once);

    let p = detect::probe(&after_frame1).unwrap();
    assert_eq!(p.frame_count, 2);
    assert_eq!(p.repeat, None);

    let cfg = GifDecoderConfig::new();
    let probe_info = cfg.clone().job().probe(&after_frame1).unwrap();
    assert_eq!(
        probe_info.sequence,
        ImageSequence::Animation {
            frame_count: Some(2),
            loop_count: Some(1),
            random_access: false,
        }
    );

    let mut anim = cfg
        .job()
        .animation_frame_decoder(Cow::Borrowed(&after_frame1), &[])
        .unwrap();
    assert_eq!(anim.info().sequence, probe_info.sequence);
    let mut yielded = 0;
    while anim.render_next_frame(None).unwrap().is_some() {
        yielded += 1;
    }
    assert_eq!(yielded, 2);
    assert_eq!(anim.loop_count(), Some(1));

    // NETSCAPE2.0 after an unknown introducer byte before frame 1.
    let mut after_unknown = header_1x1();
    after_unknown.extend_from_slice(&[0x05, 0x00]);
    push_netscape(&mut after_unknown, 4);
    push_frame_1x1(&mut after_unknown);
    push_frame_1x1(&mut after_unknown);
    after_unknown.push(0x3B);

    let (meta_u, frames_u, _) = decode_gif(&after_unknown, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(frames_u.len(), 2);
    assert_eq!(meta_u.repeat, Repeat::Once);
    assert_eq!(detect::probe(&after_unknown).unwrap().repeat, None);
    let mut anim_u = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&after_unknown), &[])
        .unwrap();
    while anim_u.render_next_frame(None).unwrap().is_some() {}
    assert_eq!(anim_u.loop_count(), Some(1));
    assert_eq!(
        anim_u.info().sequence,
        GifDecoderConfig::new()
            .job()
            .probe(&after_unknown)
            .unwrap()
            .sequence
    );

    // NETSCAPE2.0 before frame 1 (even after another 0x21 extension) IS applied everywhere.
    let mut before_frame1 = header_1x1();
    before_frame1.extend_from_slice(&[0x21, 0xFE, 0x03, b'a', b'b', b'c', 0x00]);
    push_netscape(&mut before_frame1, 4);
    push_frame_1x1(&mut before_frame1);
    push_frame_1x1(&mut before_frame1);
    before_frame1.push(0x3B);

    let (meta_ok, _, _) = decode_gif(&before_frame1, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(meta_ok.repeat, Repeat::Count(4));
    assert_eq!(detect::probe(&before_frame1).unwrap().repeat, Some(4));
    let mut anim_ok = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&before_frame1), &[])
        .unwrap();
    while anim_ok.render_next_frame(None).unwrap().is_some() {}
    assert_eq!(anim_ok.loop_count(), Some(5));
}

/// Issue #21: stray byte between frames follows `gif` crate `allow_unknown_blocks(true)`
/// sub-block grammar so `detect::probe` agrees with `decode_gif` on frame count.
#[test]
fn stray_zero_byte_between_frames_matches_decode_gif() {
    let mut g = header_1x1();
    push_frame_1x1(&mut g);
    g.push(0x00); // stray byte before frame 2's 0x2C (taken as sub-block length 44)
    push_frame_1x1(&mut g);
    g.push(0x3B);

    let (_, frames, _) = decode_gif(&g, Limits::none(), &Unstoppable).unwrap();
    assert_eq!(frames.len(), 1);

    let p = detect::probe(&g).unwrap();
    assert_eq!(p.frame_count, 1);
    assert!(!p.is_animated);

    let info = GifDecoderConfig::new().job().probe(&g).unwrap();
    assert_eq!(info.sequence, ImageSequence::Single);

    let full_info = GifDecoderConfig::new().job().probe_full(&g).unwrap();
    assert_eq!(full_info.sequence, ImageSequence::Single);
}
