#![cfg(feature = "std")]
use enough::{Stop, StopReason};
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};

fn animation() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 17, 13, &[0, 0, 0, 255, 255, 255]).unwrap();
        for value in [0, 1, 0] {
            let frame = gif::Frame {
                width: 17,
                height: 13,
                delay: 1,
                buffer: Cow::Owned(vec![value; 17 * 13]),
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    bytes
}
struct Toggle(Arc<AtomicBool>);
impl Stop for Toggle {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::Relaxed) {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[test]
fn animation_retains_owned_job_cancellation_and_poison_state() {
    for owned in [false, true] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut decoder = zengif::GifDecoderConfig::new()
            .job()
            .with_stop(zencodec::StopToken::new(Toggle(cancelled.clone())))
            .animation_frame_decoder(Cow::Owned(animation()), &[])
            .unwrap();
        cancelled.store(true, Ordering::Relaxed);
        let err = if owned {
            decoder.render_next_frame_owned(None).err()
        } else {
            decoder.render_next_frame(None).err()
        }
        .expect("job cancellation discarded");
        assert!(matches!(
            err.error().category(),
            zencodec::ErrorCategory::Stopped(StopReason::Cancelled)
        ));
        cancelled.store(false, Ordering::Relaxed);
        assert!(
            decoder.render_next_frame(None).is_err(),
            "decoder resumed after failure"
        );
    }
}
struct Pulse(AtomicUsize);
impl Stop for Pulse {
    fn check(&self) -> Result<(), StopReason> {
        // One cancellation pulse makes the old Interrupted implementation retry
        // and succeed instead of hanging, so the regression is bounded.
        if self.0.fetch_add(1, Ordering::Relaxed) == 1 {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}
#[test]
fn compressed_read_cancellation_is_terminal_and_keeps_its_reason() {
    let stop = Pulse(AtomicUsize::new(0));
    let result = zengif::Decoder::new(
        std::io::Cursor::new(animation()),
        zengif::Limits::default(),
        &stop,
    );
    let error = match result {
        Ok(_) => panic!("read_exact retried cancellation"),
        Err(e) => e,
    };
    assert!(
        matches!(
            error.error(),
            zengif::GifError::Cancelled(StopReason::TimedOut)
        ),
        "{error:?}"
    );
    assert_eq!(stop.0.load(Ordering::Relaxed), 2);
}
struct AfterOne(AtomicUsize);
impl Stop for AfterOne {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.fetch_add(1, Ordering::Relaxed) > 0 {
            Err(StopReason::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[test]
fn borrowed_cancellation_is_polled_inside_skip_loop() {
    let mut decoder = zengif::GifDecoderConfig::new()
        .job()
        .with_start_frame_index(2)
        .animation_frame_decoder(Cow::Owned(animation()), &[])
        .unwrap();
    assert!(
        decoder
            .render_next_frame(Some(&AfterOne(AtomicUsize::new(0))))
            .is_err()
    );
}

#[test]
fn exact_frame_limit_allows_eof_but_rejects_an_extra_frame() {
    for mode in 0..3 {
        for limit in [2, 3] {
            let mut limits = zengif::Limits::default();
            limits.max_frame_count = Some(limit);
            let mut decoder = zengif::Decoder::new(
                std::io::Cursor::new(animation()),
                limits,
                &enough::Unstoppable,
            )
            .unwrap();
            // The take API transfers the canvas and is intentionally final.
            for _ in 1..limit {
                assert!(decoder.next_frame().unwrap().is_some());
            }
            let mut next = || match mode {
                0 => decoder.next_frame().map(|x| x.is_some()),
                1 => decoder.next_frame_take().map(|x| x.is_some()),
                _ => decoder.with_next_frame(|_, _, _| ()).map(|x| x.is_some()),
            };
            assert!(next().unwrap());
            if limit == 3 {
                assert!(
                    !next().unwrap(),
                    "EOF was counted as another frame, mode={mode}"
                );
                assert!(!next().unwrap());
            } else {
                assert!(next().is_err(), "extra frame accepted, mode={mode}");
            }
        }
    }
}
