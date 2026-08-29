//! The frame-iteration APIs, the `max_file_size` limit, and composed-frame
//! memory accounting, exercised over real multi-frame GIFs from the checked-in
//! corpus.
//!
//! Each of these covers a defect where the obvious caller-visible behaviour and
//! the implementation disagreed:
//!
//! * `next_frame_take` moves the compositing canvas out of the `Screen`, so the
//!   natural `while let Some(f) = d.next_frame_take()? {}` loop indexed an empty
//!   canvas on frame 2 and panicked.
//! * `Limits::max_file_size` had exactly one caller — the zencodec adapter in
//!   `codec.rs`. Neither `decode_gif` nor the streaming `Decoder` enforced it,
//!   while four fuzz targets and two regression tests set it.
//! * `process_frame` charged every composed frame against `Stats` and nothing
//!   ever released it, so `next_frame` hit the memory cap partway through a long
//!   animation while true peak usage stayed flat — and `with_next_frame`, which
//!   charges nothing, had no such cap at all.

#![cfg(feature = "std")]

use enough::Unstoppable;
use std::io::Cursor;
use zengif::{Decoder, GifError, Limits, decode_gif};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/codec-corpus");

fn corpus(name: &str) -> Vec<u8> {
    let path = format!("{CORPUS}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("checked-in corpus file {path} must exist: {e}"))
}

/// Multi-frame fixtures, with the frame count `decode_gif` reports for each.
/// Every one of these exercises a different disposal mix.
fn multi_frame_fixtures() -> Vec<(&'static str, usize)> {
    let names = [
        "any-disposal.gif",
        "mixed-disposal.gif",
        "large-gif-anim-combine.gif",
        "large-gif-anim-full-frame-replace.gif",
    ];
    names
        .iter()
        .map(|name| {
            let data = corpus(name);
            let (_, frames, _) = decode_gif(&data, Limits::none(), &Unstoppable)
                .unwrap_or_else(|e| panic!("{name} must decode: {e:?}"));
            assert!(
                frames.len() > 1,
                "{name} is meant to be a multi-frame fixture, got {} frame(s)",
                frames.len()
            );
            (*name, frames.len())
        })
        .collect()
}

// ── P0-1: the natural iteration loop ──────────────────────────────────────

/// **The regression.** Driving `next_frame_take` the obvious way must not
/// panic. `next_frame_take` moves the canvas out, so it cannot composite a
/// second frame — but "cannot" has to mean a typed error, not an out-of-range
/// slice index on an emptied canvas.
#[test]
fn next_frame_take_loop_does_not_panic() {
    for (name, frame_count) in multi_frame_fixtures() {
        let data = corpus(name);
        let mut decoder = Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable)
            .unwrap_or_else(|e| panic!("{name}: decoder must construct: {e:?}"));

        let mut taken = 0usize;
        loop {
            match decoder.next_frame_take() {
                Ok(Some(frame)) => {
                    assert_eq!(frame.index, taken, "{name}: frame index out of order");
                    assert!(!frame.pixels.is_empty(), "{name}: empty composed frame");
                    taken += 1;
                    assert!(
                        taken <= frame_count,
                        "{name}: produced more frames than exist"
                    );
                }
                Ok(None) => break,
                Err(e) => {
                    // A typed refusal is the contract for a second call on a
                    // multi-frame GIF; anything else is a real failure.
                    assert!(
                        matches!(e.error(), GifError::InvalidDecoderState { .. }),
                        "{name}: expected InvalidDecoderState, got {e:?}"
                    );
                    break;
                }
            }
        }
        assert!(taken >= 1, "{name}: no frames produced at all");
    }
}

/// A single-frame GIF must iterate to a clean `None` — the guard must not turn
/// the case `next_frame_take` exists for into an error.
#[test]
fn next_frame_take_single_frame_completes_cleanly() {
    let data = corpus("sample_1.gif");
    let mut decoder =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");

    let first = decoder
        .next_frame_take()
        .expect("first frame")
        .expect("some");
    assert_eq!(first.index, 0);
    assert!(
        decoder
            .next_frame_take()
            .expect("second call must not error on a single-frame GIF")
            .is_none(),
        "a single-frame GIF must end with None, not an error"
    );
    assert!(decoder.is_finished());
}

/// `next_frame` after `next_frame_take` hits the same emptied canvas, so the
/// guard has to cover every compositing entry point, not just the one that
/// emptied it.
#[test]
fn next_frame_after_take_is_typed_not_panic() {
    let (name, _) = multi_frame_fixtures()[0];
    let data = corpus(name);
    let mut decoder =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    decoder.next_frame_take().expect("first").expect("some");
    match decoder.next_frame() {
        Ok(None) => {}
        Ok(Some(_)) => panic!("{name}: composited a frame onto a canvas that was moved out"),
        Err(e) => assert!(
            matches!(e.error(), GifError::InvalidDecoderState { .. }),
            "{name}: expected InvalidDecoderState, got {e:?}"
        ),
    }
}

/// `with_next_frame` too.
#[test]
fn with_next_frame_after_take_is_typed_not_panic() {
    let (name, _) = multi_frame_fixtures()[0];
    let data = corpus(name);
    let mut decoder =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    decoder.next_frame_take().expect("first").expect("some");
    match decoder.with_next_frame(|_, _, px| px.len()) {
        Ok(None) => {}
        Ok(Some(_)) => panic!("{name}: composited a frame onto a canvas that was moved out"),
        Err(e) => assert!(
            matches!(e.error(), GifError::InvalidDecoderState { .. }),
            "{name}: expected InvalidDecoderState, got {e:?}"
        ),
    }
}

/// The supported multi-frame loops must keep working unchanged.
#[test]
fn next_frame_loop_still_walks_every_frame() {
    for (name, frame_count) in multi_frame_fixtures() {
        let data = corpus(name);
        let mut decoder =
            Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
        let mut seen = 0usize;
        while let Some(frame) = decoder
            .next_frame()
            .unwrap_or_else(|e| panic!("{name}: {e:?}"))
        {
            assert_eq!(frame.index, seen);
            seen += 1;
        }
        assert_eq!(seen, frame_count, "{name}: next_frame lost frames");

        let mut decoder =
            Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
        let mut seen = 0usize;
        while decoder
            .with_next_frame(|_, _, _| ())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"))
            .is_some()
        {
            seen += 1;
        }
        assert_eq!(seen, frame_count, "{name}: with_next_frame lost frames");
    }
}

// ── P0-2: max_file_size on the native paths ───────────────────────────────

/// **The regression.** `decode_gif` never consulted `max_file_size`, so the
/// four fuzz targets and two regression tests that set it were asserting
/// against a limit that did nothing.
#[test]
fn decode_gif_enforces_max_file_size() {
    let data = corpus("large-gif-anim-combine.gif");
    let under = data.len() as u64;

    let limits = Limits::none().max_file_size(under - 1);
    // Map to () first: a failure here would otherwise Debug-print every decoded
    // pixel of a 1000x1000 animation into the test log.
    let err = decode_gif(&data, limits, &Unstoppable)
        .map(|_| ())
        .expect_err("a file larger than max_file_size must be rejected");
    match err.error() {
        GifError::FileTooLarge { size, max } => {
            assert_eq!(*size, data.len() as u64);
            assert_eq!(*max, under - 1);
        }
        other => panic!("expected FileTooLarge, got {other:?}"),
    }

    // Exactly at the limit is allowed — the check is `>`, not `>=`.
    decode_gif(&data, Limits::none().max_file_size(under), &Unstoppable)
        .expect("a file exactly at max_file_size must decode");
}

/// The streaming decoder does not know the input length up front, so it has to
/// enforce the cap against bytes actually consumed, and it must trip before
/// running to completion on an oversized stream.
#[test]
fn streaming_decoder_enforces_max_file_size() {
    let data = corpus("large-gif-anim-combine.gif");
    let limits = Limits::none().max_file_size(64);

    let result = (|| -> zengif::Result<usize> {
        let mut decoder = Decoder::new(Cursor::new(&data[..]), limits, &Unstoppable)?;
        let mut n = 0;
        while decoder.next_frame()?.is_some() {
            n += 1;
        }
        Ok(n)
    })();

    let err = result.expect_err("streaming past max_file_size must be rejected");
    assert!(
        matches!(err.error(), GifError::FileTooLarge { .. }),
        "expected FileTooLarge, got {err:?}"
    );
}

/// A generous cap must not disturb a normal decode.
#[test]
fn max_file_size_above_input_is_transparent() {
    for (name, frame_count) in multi_frame_fixtures() {
        let data = corpus(name);
        let limits = Limits::none().max_file_size(data.len() as u64 * 4);
        let (_, frames, _) = decode_gif(&data, limits, &Unstoppable)
            .unwrap_or_else(|e| panic!("{name} must still decode: {e:?}"));
        assert_eq!(frames.len(), frame_count, "{name}: frame count changed");
    }
}

// ── P0-3: composed-frame accounting ───────────────────────────────────────

/// **The regression.** Every `next_frame` charged a composed frame against
/// `Stats` and nothing released it, so tracked "current" memory grew without
/// bound across an animation even though the caller dropped each frame. The
/// accounting must reflect frames the caller is actually still holding.
#[test]
fn dropped_composed_frames_are_untracked() {
    let data = corpus("large-gif-anim-full-frame-replace.gif");
    let mut decoder =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");

    let mut after_first = None;
    let mut frames = 0usize;
    while let Some(frame) = decoder.next_frame().expect("decode") {
        frames += 1;
        let current = decoder.stats().current();
        if after_first.is_none() {
            after_first = Some(current);
        }
        drop(frame); // the caller does not retain it
        let after_drop = decoder.stats().current();
        assert!(
            after_drop <= after_first.unwrap(),
            "tracked memory grew to {after_drop} by frame {frames} while holding no frames \
             (was {} after the first)",
            after_first.unwrap()
        );
    }
    assert!(frames > 1, "need a multi-frame fixture");
}

/// Untracking on hand-off must not become "never track": the cap check still
/// has to see the composed frame, so `peak` records the canvas *plus* one
/// composed frame — which is the real 2x-canvas high-water mark — even though
/// `current` returns to the decoder's own footprint.
#[test]
fn composed_frame_still_counts_toward_peak() {
    let data = corpus("large-gif-anim-full-frame-replace.gif");

    let mut copying =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    while copying.next_frame().expect("decode").is_some() {}
    let copying_peak = copying.stats().peak();

    let mut in_place =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    while in_place
        .with_next_frame(|_, _, _| ())
        .expect("decode")
        .is_some()
    {}
    let in_place_peak = in_place.stats().peak();

    assert!(
        copying_peak > in_place_peak,
        "next_frame allocates a composed frame on top of the canvas, so its peak \
         ({copying_peak}) must exceed with_next_frame's ({in_place_peak})"
    );
}

/// Releasing the charge after hand-off must not disable the cap: a budget that
/// covers in-place compositing but not the extra composed frame must still be
/// refused. The cap is taken from a full `with_next_frame` run, which allocates
/// the canvas and pixel buffer but never a composed frame.
#[test]
fn composed_frame_beyond_max_memory_is_still_refused() {
    let data = corpus("large-gif-anim-full-frame-replace.gif");

    let mut in_place =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    while in_place
        .with_next_frame(|_, _, _| ())
        .expect("probe")
        .is_some()
    {}
    let cap = in_place.stats().peak() as u64;

    // The same budget must carry the whole animation in place ...
    let mut ok = Decoder::new(
        Cursor::new(&data[..]),
        Limits::none().max_memory(cap),
        &Unstoppable,
    )
    .expect("construct under the in-place budget");
    while ok
        .with_next_frame(|_, _, _| ())
        .expect("in-place decode must fit its own peak")
        .is_some()
    {}

    // ... but not a copying decode, which needs a composed frame on top of it.
    let mut tight = Decoder::new(
        Cursor::new(&data[..]),
        Limits::none().max_memory(cap),
        &Unstoppable,
    )
    .expect("construct under the in-place budget");
    let err = tight
        .next_frame()
        .expect_err("a composed frame beyond max_memory must be refused");
    assert!(
        matches!(err.error(), GifError::MemoryLimitExceeded { .. }),
        "expected MemoryLimitExceeded, got {err:?}"
    );
}

/// **The regression.** `next_frame` charged composed frames and
/// `with_next_frame` charged nothing, so the same decoder enforced a different
/// effective `max_memory` depending on which method the caller used. The budget
/// here is what in-place compositing needs plus exactly one composed frame —
/// the true high-water mark of a copying decode that retains nothing. Both APIs
/// must carry the whole animation under it.
#[test]
fn next_frame_and_with_next_frame_agree_under_a_memory_cap() {
    let data = corpus("large-gif-anim-full-frame-replace.gif");

    // What in-place compositing costs, and how big one composed frame is.
    let mut probe =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    let first = probe.next_frame().expect("probe").expect("some");
    let composed_bytes = first.pixels.len() * core::mem::size_of::<zengif::Rgba>();
    drop(first);
    let mut frame_count = 1usize;
    while probe.next_frame().expect("probe").is_some() {
        frame_count += 1;
    }
    assert!(frame_count > 1, "need a multi-frame fixture");

    let mut in_place =
        Decoder::new(Cursor::new(&data[..]), Limits::none(), &Unstoppable).expect("construct");
    while in_place
        .with_next_frame(|_, _, _| ())
        .expect("probe")
        .is_some()
    {}
    let cap = in_place.stats().peak() as u64 + composed_bytes as u64;

    let mut via_with = 0usize;
    let mut d = Decoder::new(
        Cursor::new(&data[..]),
        Limits::none().max_memory(cap),
        &Unstoppable,
    )
    .expect("construct");
    while d
        .with_next_frame(|_, _, _| ())
        .expect("with_next_frame under cap")
        .is_some()
    {
        via_with += 1;
    }

    // Frames are dropped each iteration, exactly as the with_next_frame loop
    // keeps nothing — so one composed frame of headroom must carry the whole
    // animation. Leaving each frame charged made this trip partway through.
    let mut via_next = 0usize;
    let mut d = Decoder::new(
        Cursor::new(&data[..]),
        Limits::none().max_memory(cap),
        &Unstoppable,
    )
    .expect("construct");
    loop {
        match d.next_frame() {
            Ok(Some(_)) => via_next += 1,
            Ok(None) => break,
            Err(e) => panic!(
                "next_frame ran out of budget at frame {} of {frame_count} under a cap that \
                 covers in-place compositing plus one composed frame: {e:?}",
                via_next + 1
            ),
        }
    }

    assert_eq!(
        via_next, via_with,
        "next_frame and with_next_frame disagreed on how far the same cap goes"
    );
    assert_eq!(via_next, frame_count, "the cap should allow every frame");
}
