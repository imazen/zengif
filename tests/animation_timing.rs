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
use zencodec::ImageSequence;
use zencodec::animation::FrameDuration;
use zencodec::decode::{AnimationFrameDecoder, Decode, DecodeJob, DecoderConfig};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, EncoderConfig};
use zengif::{GifDecoderConfig, GifEncoderConfig};
use zenpixels::{PixelBuffer, PixelDescriptor};

fn pixels() -> PixelBuffer {
    PixelBuffer::from_vec(
        [255, 0, 0, 255].repeat(16),
        4,
        4,
        PixelDescriptor::RGBA8_SRGB,
    )
    .unwrap()
}
fn wire_repeat(bytes: &[u8]) -> Option<u16> {
    let offset = bytes.windows(11).position(|w| w == b"NETSCAPE2.0")?;
    assert_eq!(&bytes[offset + 11..offset + 13], &[3, 1]);
    Some(u16::from_le_bytes(
        bytes[offset + 13..offset + 15].try_into().unwrap(),
    ))
}
fn assert_plays(sequence: &ImageSequence, expected: u32) {
    assert!(
        matches!(sequence, ImageSequence::Animation { loop_count: Some(n), .. } if *n == expected),
        "{sequence:?}"
    );
}

#[test]
fn total_plays_are_converted_to_native_repeats_on_every_decode_path() {
    let pixels = pixels();
    for plays in [0, 1, 2, 3, 65535, 65536] {
        let mut encoder = GifEncoderConfig::new()
            .job()
            .with_loop_count(Some(plays))
            .animation_frame_encoder()
            .unwrap();
        for milliseconds in [10, 20] {
            encoder
                .push_frame_timed(
                    pixels.as_slice(),
                    FrameDuration::from_millis(milliseconds),
                    None,
                )
                .unwrap();
        }
        let encoded = encoder.finish(None).unwrap();
        assert_eq!(
            wire_repeat(encoded.data()),
            match plays {
                0 => Some(0),
                1 => None,
                n => Some((n - 1) as u16),
            }
        );
        let config = GifDecoderConfig::new();
        let probe = config.clone().job().probe(encoded.data()).unwrap();
        assert_plays(&probe.sequence, plays);
        let full_probe = config.clone().job().probe_full(encoded.data()).unwrap();
        assert_plays(&full_probe.sequence, plays);
        let still = config
            .clone()
            .job()
            .decoder(Cow::Borrowed(encoded.data()), &[])
            .unwrap()
            .decode()
            .unwrap();
        assert_plays(&still.info().sequence, plays);
        let mut decoder = config
            .job()
            .animation_frame_decoder(Cow::Borrowed(encoded.data()), &[])
            .unwrap();
        assert_eq!(decoder.loop_count(), Some(plays));
        assert_plays(&decoder.info().sequence, plays);
        while decoder.render_next_frame(None).unwrap().is_some() {
            assert_eq!(decoder.loop_count(), Some(plays));
        }
    }
    for plays in [65537, u32::MAX] {
        assert!(
            GifEncoderConfig::new()
                .job()
                .with_loop_count(Some(plays))
                .animation_frame_encoder()
                .is_err()
        );
    }
}

#[test]
fn exact_delays_preserve_zero_and_u16_max_and_reject_before_accepting() {
    let pixels = pixels();
    let mut encoder = GifEncoderConfig::new()
        .job()
        .animation_frame_encoder()
        .unwrap();
    for duration in [
        FrameDuration::new(1, 1000).unwrap(),
        FrameDuration::new(65536, 100).unwrap(),
        FrameDuration::new(u64::MAX, 1).unwrap(),
    ] {
        assert!(
            encoder
                .push_frame_timed(pixels.as_slice(), duration, None)
                .is_err()
        );
    }
    assert!(
        encoder
            .push_frame(pixels.as_slice(), u32::MAX, None)
            .is_err()
    );
    for centiseconds in [0, 1, 65535] {
        encoder
            .push_frame_timed(
                pixels.as_slice(),
                FrameDuration::new(centiseconds, 100).unwrap(),
                None,
            )
            .unwrap();
    }
    let encoded = encoder.finish(None).unwrap();
    let mut decoder = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(encoded.data()), &[])
        .unwrap();
    for centiseconds in [0, 1, 65535] {
        let frame = decoder.render_next_frame(None).unwrap().unwrap();
        assert_eq!(
            frame.duration(),
            FrameDuration::new(centiseconds, 100).unwrap()
        );
    }
    assert!(decoder.render_next_frame(None).unwrap().is_none());
}

#[test]
fn native_duration_and_buffer_rejections_leave_previous_frames_usable() {
    let config = zengif::EncoderConfig::new().shared_palette(false);
    let limits = zengif::Limits::none().max_animation_ms(100);
    let mut encoder = zengif::EncodeRequest::new(&config, 4, 4)
        .limits(&limits)
        .build()
        .unwrap();
    let frame = |delay, count| {
        zengif::FrameInput::new(4, 4, delay, vec![zengif::Rgba::rgb(128, 128, 128); count])
    };
    encoder.add_frame(frame(5, 16)).unwrap();
    assert!(encoder.add_frame(frame(10, 16)).is_err());
    for size in [0, 1, 15, 17] {
        let error = encoder.add_frame(frame(1, size)).unwrap_err();
        assert!(matches!(
            error.error(),
            zengif::GifError::FrameBufferLength { .. }
        ));
    }
    encoder.add_frame(frame(5, 16)).unwrap();
    let output = encoder.finish().unwrap();
    let mut decoder = GifDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Owned(output), &[])
        .unwrap();
    for _ in 0..2 {
        let frame = decoder.render_next_frame(None).unwrap().unwrap();
        assert_eq!(frame.duration(), FrameDuration::from_millis(50));
        assert_eq!(&frame.pixels().row(0)[..4], &[128, 128, 128, 255]);
    }
    assert!(decoder.render_next_frame(None).unwrap().is_none());
}

#[test]
fn cancellation_reaches_quantization_and_buffered_finish_from_both_tokens() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct PollLimit(Arc<AtomicUsize>, usize);
    impl enough::Stop for PollLimit {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.0.fetch_add(1, Ordering::Relaxed) >= self.1 {
                Err(enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    let bytes: Vec<_> = (0..128 * 128)
        .flat_map(|i| {
            [
                (i * 37) as u8,
                (i * 113 + i / 5) as u8,
                (i * 67 + i / 31) as u8,
                255,
            ]
        })
        .collect();
    let pixels = PixelBuffer::from_vec(bytes, 128, 128, PixelDescriptor::RGBA8_SRGB).unwrap();
    for shared in [false, true] {
        for job_stop in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut job = GifEncoderConfig::new().with_shared_palette(shared).job();
            if job_stop {
                job = job.with_stop(zencodec::StopToken::new(PollLimit(calls.clone(), 8)));
            }
            let mut encoder = job.animation_frame_encoder().unwrap();
            let call_token = PollLimit(calls.clone(), 8);
            let token = (!job_stop).then_some(&call_token as &dyn enough::Stop);
            if shared {
                encoder.push_frame(pixels.as_slice(), 10, token).unwrap();
                // A call token is scoped to finish; the job token stays active.
                if !job_stop {
                    calls.store(0, Ordering::Relaxed);
                }
                assert!(
                    encoder.finish(token).is_err(),
                    "shared={shared} job={job_stop}"
                );
            } else {
                // One canvas of lookahead is required to choose disposal.
                // The next push performs quantization of the first frame.
                encoder.push_frame(pixels.as_slice(), 10, token).unwrap();
                calls.store(0, Ordering::Relaxed);
                assert!(
                    encoder.push_frame(pixels.as_slice(), 10, token).is_err(),
                    "shared={shared} job={job_stop}"
                );
                assert!(
                    encoder.finish(None).is_err(),
                    "partial encode must not finalize successfully"
                );
            }
            assert!(calls.load(Ordering::Relaxed) > 8, "must reach kernel polls");
        }
    }
}
