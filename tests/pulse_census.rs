//! Poll census: how often each zencodec operation polls the job's
//! cancellation, and which of those polls, timing out once, end the operation
//! with the timeout. One line per operation, for comparing revisions
//! (`just pulse-census`). It asserts only that the operations that must poll
//! do. Written against the setter both the stop and the pulse APIs have,
//! `with_stop`, so the same file builds on either side of the migration.
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
#![allow(deprecated)]
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use enough::{Stop, StopReason};
use zencodec::StopToken;
use zencodec::animation::FrameDuration;
use zencodec::decode::{AnimationFrameDecoder, Decode, DecodeJob, DecodeRowSink, DecoderConfig};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zencodec::{At, CodecError, ErrorCategory};
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

/// Collects decoded rows.
#[derive(Default)]
struct Rows {
    buf: Vec<u8>,
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
        Ok(PixelSliceMut::new(&mut self.buf, width, height, stride, descriptor).unwrap())
    }
}

type Run<'a> = &'a dyn Fn(StopToken) -> Result<(), At<CodecError>>;

/// Print one census line per operation: its polls, and for each poll that
/// did not end the operation with the timeout, what happened instead.
fn census(ops: &[(&str, bool, Run<'_>)]) {
    for (what, must_poll, run) in ops {
        let seen = Arc::new(AtomicUsize::new(0));
        let token = |at| {
            StopToken::new(Polls {
                seen: seen.clone(),
                at,
            })
        };
        run(token(usize::MAX)).unwrap_or_else(|error| panic!("{what}: {error}"));
        let polls = seen.load(Ordering::Relaxed);
        let mut missed = Vec::new();
        for at in 0..polls {
            seen.store(0, Ordering::Relaxed);
            match run(token(at)) {
                Err(error)
                    if error.error().category() == ErrorCategory::Stopped(StopReason::TimedOut) => {
                }
                Err(error) => missed.push(format!("{at}:{:?}", error.error().category())),
                Ok(()) => missed.push(format!("{at}:ok")),
            }
        }
        println!(
            "census\t{what}\tpolls={polls}\tsurfaced={}\tmissed=[{}]",
            polls - missed.len(),
            missed.join(" ")
        );
        assert!(!must_poll || polls > 0, "{what}: never polled");
    }
}

#[test]
fn decode_poll_census() {
    let data = animation();
    let job = |token| GifDecoderConfig::new().job().with_stop(token);
    census(&[
        ("probe", true, &|t| job(t).probe(&data).map(drop)),
        ("probe_full", true, &|t| job(t).probe_full(&data).map(drop)),
        ("output_info", false, &|t| {
            job(t).output_info(&data).map(drop)
        }),
        ("decode", true, &|t| {
            job(t)
                .decoder(Cow::Borrowed(&data), &[])?
                .decode()
                .map(drop)
        }),
        ("push_decoder", true, &|t| {
            job(t)
                .push_decoder(Cow::Borrowed(&data), &mut Rows::default(), &[])
                .map(drop)
        }),
        ("streaming_decoder", true, &|t| {
            job(t)
                .streaming_decoder(Cow::Borrowed(&data), &[])
                .map(drop)
        }),
        ("render_next_frame", true, &|t| {
            let mut decoder = job(t).animation_frame_decoder(Cow::Borrowed(&data), &[])?;
            while decoder.render_next_frame(None)?.is_some() {}
            Ok(())
        }),
        ("render_next_frame_owned", true, &|t| {
            let mut decoder = job(t).animation_frame_decoder(Cow::Borrowed(&data), &[])?;
            while decoder.render_next_frame_owned(None)?.is_some() {}
            Ok(())
        }),
        ("render_next_frame_to_sink", true, &|t| {
            let mut decoder = job(t).animation_frame_decoder(Cow::Borrowed(&data), &[])?;
            while decoder
                .render_next_frame_to_sink(None, &mut Rows::default())?
                .is_some()
            {}
            Ok(())
        }),
        #[cfg(feature = "inventory")]
        ("inventory", true, &|t| job(t).inventory(&data).map(drop)),
    ]);
}

#[test]
fn encode_poll_census() {
    let frame = frame();
    let job = |token, shared| {
        GifEncoderConfig::new()
            .with_shared_palette(shared)
            .job()
            .with_stop(token)
    };
    let animate = |token, shared| -> Result<(), At<CodecError>> {
        let mut encoder = job(token, shared).animation_frame_encoder()?;
        encoder.push_frame(frame.as_slice(), 10, None)?;
        encoder.push_frame_exact(frame.as_slice(), FrameDuration::new(2, 100).unwrap(), None)?;
        encoder.push_frame_timed(frame.as_slice(), FrameDuration::new(1, 30).unwrap(), None)?;
        encoder.finish(None).map(drop)
    };
    census(&[
        ("still encode", true, &|t| {
            job(t, false).encoder()?.encode(frame.as_slice()).map(drop)
        }),
        ("animation", true, &|t| animate(t, false)),
        ("shared-palette animation", true, &|t| animate(t, true)),
    ]);
}
