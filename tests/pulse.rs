//! The operation pulse reaches every zencodec operation. A job's pulse stops
//! the probes, every decoder (sink and streaming included) and both encoders;
//! a call's pulse stops each animation call, including the exact-timing pushes
//! and the sink render; the deprecated `with_stop` still cancels, through the
//! pulse it builds. Each stop keeps its reason.
#![cfg(all(
    feature = "std",
    any(
        feature = "zenquant",
        feature = "quantette",
        feature = "quantizr",
        feature = "imagequant",
        feature = "color_quant"
    )
))]
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use enough::{Stop, StopReason};
use zencodec::animation::FrameDuration;
use zencodec::decode::{AnimationFrameDecoder, Decode, DecodeJob, DecodeRowSink, DecoderConfig};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zencodec::how_far::StopOnly;
use zencodec::{At, CodecError, ErrorCategory, SharedPulse};
use zengif::{GifDecoderConfig, GifEncoderConfig};
use zenpixels::{PixelBuffer, PixelDescriptor, PixelSliceMut};

/// Three 17x13 frames.
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

/// Opaque 16x16 RGBA with enough colors to quantize.
fn frame() -> PixelBuffer {
    let bytes = (0..16 * 16u32)
        .flat_map(|i| [(i * 37) as u8, (i * 113 + i / 5) as u8, (i * 67) as u8, 255])
        .collect();
    PixelBuffer::from_vec(bytes, 16, 16, PixelDescriptor::RGBA8_SRGB).unwrap()
}

/// Counts its polls; poll number `at` reports a timeout, once.
struct Polls {
    seen: Arc<AtomicUsize>,
    at: usize,
}

impl Stop for Polls {
    fn check(&self) -> Result<(), StopReason> {
        if self.seen.fetch_add(1, Ordering::Relaxed) == self.at {
            Err(StopReason::TimedOut)
        } else {
            Ok(())
        }
    }
}

fn pulse(seen: &Arc<AtomicUsize>, at: usize) -> SharedPulse {
    SharedPulse::new(StopOnly::new(Polls {
        seen: seen.clone(),
        at,
    }))
}

/// Always timed out.
struct Expired;

impl Stop for Expired {
    fn check(&self) -> Result<(), StopReason> {
        Err(StopReason::TimedOut)
    }
}

#[track_caller]
fn assert_timed_out<T>(what: &str, result: Result<T, At<CodecError>>) {
    match result {
        Ok(_) => panic!("{what}: finished under a stopped pulse"),
        Err(error) => assert_eq!(
            error.error().category(),
            ErrorCategory::Stopped(StopReason::TimedOut),
            "{what}: {error}"
        ),
    }
}

/// Collects decoded rows.
#[derive(Default)]
struct Rows {
    buf: Vec<u8>,
    rows: u32,
}

impl DecodeRowSink for Rows {
    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, zencodec::decode::SinkError> {
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.buf.resize(height as usize * stride, 0);
        self.rows += height;
        Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor).unwrap())
    }
}

type Run<'a> = &'a dyn Fn(SharedPulse) -> Result<(), At<CodecError>>;

/// Each operation polls the job's pulse, and a stop at its first or last
/// poll ends it with the pulse's reason.
fn check_job_pulse(ops: &[(&str, Run<'_>)]) {
    for (what, run) in ops {
        let seen = Arc::new(AtomicUsize::new(0));
        run(pulse(&seen, usize::MAX)).unwrap_or_else(|error| panic!("{what}: {error}"));
        let polls = seen.load(Ordering::Relaxed);
        assert!(polls > 0, "{what}: never polled the job's pulse");
        for at in [0, polls - 1] {
            seen.store(0, Ordering::Relaxed);
            assert_timed_out(
                &format!("{what}, poll {at} of {polls}"),
                run(pulse(&seen, at)),
            );
        }
    }
}

#[test]
fn a_job_pulse_reaches_every_decode_operation() {
    let data = animation();
    let job = |pulse| GifDecoderConfig::new().job().with_pulse(pulse);
    check_job_pulse(&[
        ("probe", &|p| job(p).probe(&data).map(drop)),
        ("probe_full", &|p| job(p).probe_full(&data).map(drop)),
        ("decode", &|p| {
            job(p)
                .decoder(Cow::Borrowed(&data), &[])?
                .decode()
                .map(drop)
        }),
        ("push_decoder", &|p| {
            let mut rows = Rows::default();
            job(p).push_decoder(Cow::Borrowed(&data), &mut rows, &[])?;
            assert_eq!(rows.rows, 13);
            Ok(())
        }),
        ("streaming_decoder", &|p| {
            job(p)
                .streaming_decoder(Cow::Borrowed(&data), &[])
                .map(drop)
        }),
        ("animation_frame_decoder", &|p| {
            let mut decoder = job(p).animation_frame_decoder(Cow::Borrowed(&data), &[])?;
            while decoder.render_next_frame(None)?.is_some() {}
            Ok(())
        }),
        #[cfg(feature = "inventory")]
        ("inventory", &|p| job(p).inventory(&data).map(drop)),
    ]);
}

#[test]
fn a_job_pulse_reaches_both_encoders() {
    let frame = frame();
    let job = |pulse, shared| {
        GifEncoderConfig::new()
            .with_shared_palette(shared)
            .job()
            .with_pulse(pulse)
    };
    let animate = |pulse, shared| -> Result<(), At<CodecError>> {
        let mut encoder = job(pulse, shared).animation_frame_encoder()?;
        encoder.push_frame(frame.as_slice(), 10, None)?;
        encoder.push_frame_exact(frame.as_slice(), FrameDuration::new(2, 100).unwrap(), None)?;
        encoder.push_frame_timed(frame.as_slice(), FrameDuration::new(1, 30).unwrap(), None)?;
        encoder.finish(None).map(drop)
    };
    check_job_pulse(&[
        ("still encode", &|p| {
            job(p, false).encoder()?.encode(frame.as_slice()).map(drop)
        }),
        ("animation", &|p| animate(p, false)),
        ("shared-palette animation", &|p| animate(p, true)),
    ]);
}

#[test]
fn a_call_pulse_stops_each_animation_call() {
    let frame = frame();
    let expired = StopOnly::borrowed(&Expired);
    let stopped = Some(&expired as &dyn zencodec::Pulse);
    let encoder = || {
        GifEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .unwrap()
    };
    let pixels = frame.as_slice();
    let duration = FrameDuration::new(2, 100).unwrap();
    assert_timed_out("push_frame", encoder().push_frame(pixels, 20, stopped));
    assert_timed_out(
        "push_frame_exact",
        encoder().push_frame_exact(frame.as_slice(), duration, stopped),
    );
    assert_timed_out(
        "push_frame_timed",
        encoder().push_frame_timed(frame.as_slice(), duration, stopped),
    );
    let mut finishing = encoder();
    finishing.push_frame(frame.as_slice(), 20, None).unwrap();
    assert_timed_out("finish", finishing.finish(stopped));

    let data = animation();
    let decoder = || {
        GifDecoderConfig::new()
            .job()
            .animation_frame_decoder(Cow::Borrowed(&data), &[])
            .unwrap()
    };
    assert_timed_out("render_next_frame", decoder().render_next_frame(stopped));
    assert_timed_out(
        "render_next_frame_owned",
        decoder().render_next_frame_owned(stopped),
    );
    let mut rows = Rows::default();
    assert_timed_out(
        "render_next_frame_to_sink",
        decoder().render_next_frame_to_sink(stopped, &mut rows),
    );
    assert_eq!(rows.rows, 0, "a stopped render wrote rows");
}

/// A stopped timed push leaves no trace: the rounding phase, the frame count
/// and the file are those of the encode without it.
#[test]
fn a_stopped_timed_push_keeps_the_timing_phase() {
    let frame = frame();
    let expired = StopOnly::borrowed(&Expired);
    let third = FrameDuration::new(1, 30).unwrap();
    let encode = |stop_second: bool| {
        let mut encoder = GifEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .unwrap();
        let mut durations = Vec::new();
        for index in 0..4 {
            if stop_second && index == 1 {
                assert_timed_out(
                    "timed push",
                    encoder.push_frame_timed(frame.as_slice(), third, Some(&expired)),
                );
            }
            durations.push(
                encoder
                    .push_frame_timed(frame.as_slice(), third, None)
                    .unwrap(),
            );
        }
        (durations, encoder.finish(None).unwrap().data().to_vec())
    };
    assert_eq!(encode(true), encode(false));
}

#[test]
#[allow(deprecated)] // Exercises the legacy cancellation-only setters on purpose.
fn the_legacy_stop_setter_still_cancels() {
    use zencodec::decode::DynDecoderConfig;
    use zencodec::{CodecErrorExt, StopToken};
    let data = animation();
    let frame = frame();
    let decode = || {
        GifDecoderConfig::new()
            .job()
            .with_stop(StopToken::new(Expired))
    };
    assert_timed_out("probe", decode().probe(&data));
    assert_timed_out(
        "decode",
        decode()
            .decoder(Cow::Borrowed(&data), &[])
            .and_then(|decoder| decoder.decode()),
    );
    assert_timed_out(
        "animation_frame_decoder",
        decode().animation_frame_decoder(Cow::Borrowed(&data), &[]),
    );
    let encode = || {
        GifEncoderConfig::new()
            .job()
            .with_stop(StopToken::new(Expired))
    };
    assert_timed_out(
        "still encode",
        encode()
            .encoder()
            .and_then(|encoder| encoder.encode(frame.as_slice())),
    );
    let mut encoder = encode().animation_frame_encoder().unwrap();
    assert_timed_out("push_frame", encoder.push_frame(frame.as_slice(), 20, None));

    let config = GifDecoderConfig::new();
    let dynamic: &dyn DynDecoderConfig = &config;
    let mut job = dynamic.dyn_job();
    job.set_stop(StopToken::new(Expired));
    let error = job.probe(&data).unwrap_err();
    assert_eq!(
        error.error_category(),
        Some(ErrorCategory::Stopped(StopReason::TimedOut))
    );
}
