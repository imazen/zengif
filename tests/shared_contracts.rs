#![cfg(feature = "std")]

// PixelView encoding requires one of the configured palette quantizers.
#[cfg(any(
    feature = "zenquant",
    feature = "quantette",
    feature = "imagequant",
    feature = "quantizr",
    feature = "color_quant"
))]
mod encode_checks {
    use std::borrow::Cow;
    use zencodec::Metadata;
    use zencodec::decode::{Decode, DecodeJob, DecodePolicy, DecoderConfig};
    use zencodec::encode::{EncodeJob, EncodePolicy, EncoderConfig};
    use zencodec_testkit as tk;
    use zengif::{GifDecoderConfig as D, GifEncoderConfig as E};

    #[test]
    fn rational_timing_is_exact_or_rounded_on_the_centisecond_clock() {
        use zencodec::animation::FrameDuration;
        let cs = |n| Some(FrameDuration::new(n, 100).unwrap());
        let img = tk::TestImage::rgb8_gradient(4, 3);
        // Exact: whole centiseconds (including 0 and u16::MAX) are kept;
        // anything finer or larger is rejected without accepting the frame.
        tk::check_animation_timing(
            E::new(),
            D::new(),
            img.as_slice(),
            true,
            &[
                (FrameDuration::new(0, 100).unwrap(), cs(0)),
                (FrameDuration::new(1, 30).unwrap(), None),
                (FrameDuration::from_millis(10), cs(1)),
                (FrameDuration::new(65536, 100).unwrap(), None),
                (FrameDuration::new(65535, 100).unwrap(), cs(65535)),
            ],
        )
        .unwrap();
        // Timed: cumulative endpoints round to the clock (3.33, 6.67, 10 cs).
        tk::check_animation_timing(
            E::new(),
            D::new(),
            img.as_slice(),
            false,
            &[
                (FrameDuration::new(1, 30).unwrap(), cs(3)),
                (FrameDuration::new(u64::MAX, 1).unwrap(), None),
                (FrameDuration::new(1, 30).unwrap(), cs(4)),
                (FrameDuration::new(1, 30).unwrap(), cs(3)),
            ],
        )
        .unwrap();
    }

    #[test]
    fn animation_begin_rejects_and_binds_declared_spec() {
        use zencodec::decode::AnimationFrameDecoder as _;
        use zencodec::encode::AnimationFrameEncoder as _;
        use zencodec::{CodecErrorExt, UnsupportedOperation, animation::FrameDuration};
        use zenpixels::{PixelFormat, PixelView, Primaries};

        let spec = PixelFormat::RGBA8_SRGB;
        let pixels = [11, 22, 33, 255];
        let srgb = PixelView::from_bytes(&pixels, 1, 1, &zenpixels::formats::RGBA8_SRGB).unwrap();
        let p3_spec = PixelFormat::RGBA8_SRGB.with_primaries(Primaries::DisplayP3);
        let p3 = PixelView::from_bytes(&pixels, 1, 1, &p3_spec).unwrap();
        let bgra = PixelView::from_bytes(&[33, 22, 11, 255], 1, 1, &zenpixels::formats::BGRA8_SRGB)
            .unwrap();
        let expected = tk::ExpectedError::Unsupported(UnsupportedOperation::PixelFormat);
        tk::check_animation_begin_refusal(E::new(), srgb.clone(), p3.clone(), expected).unwrap();

        let mut encoder = E::new().job().animation_frame_encoder().unwrap();
        let rejected = encoder
            .begin(1, 1, p3.format().clone(), Some(2))
            .unwrap_err();
        assert_eq!(
            rejected.unsupported_operation(),
            Some(&UnsupportedOperation::PixelFormat)
        );
        assert!(encoder.pending_output().is_empty());
        encoder.begin(1, 1, spec, Some(1)).unwrap();
        let delay = FrameDuration::new(7, 100).unwrap();
        for wrong in [p3, bgra] {
            let rejected = encoder.push_frame_exact(wrong, delay, None).unwrap_err();
            assert_eq!(
                rejected.unsupported_operation(),
                Some(&UnsupportedOperation::PixelFormat)
            );
            assert!(encoder.pending_output().is_empty());
        }
        encoder.push_frame_exact(srgb, delay, None).unwrap();
        let output = encoder.finish(None).unwrap();
        let mut decoder = D::new()
            .job()
            .animation_frame_decoder(Cow::Borrowed(output.data()), &[])
            .unwrap();
        let frame = decoder.render_next_frame(None).unwrap().unwrap();
        assert_eq!(frame.pixels().row(0), pixels);
        assert_eq!(frame.duration(), delay);
        assert_eq!(frame.frame_index(), 0);
        assert_eq!(frame.pts(), None);
        assert!(decoder.render_next_frame(None).unwrap().is_none());
    }

    #[test]
    fn policy_and_inventory_contracts() {
        let img = tk::TestImage::rgb8_gradient(4, 3);
        let enc = E::new();
        let bytes = enc.clone().job().encode(img.as_slice()).unwrap();
        let expected = D::new()
            .job()
            .decoder(Cow::Borrowed(bytes.data()), &[])
            .unwrap()
            .decode()
            .unwrap()
            .metadata();
        for (policy, accepted) in [
            (EncodePolicy::none(), true),
            (EncodePolicy::strip_all(), false),
        ] {
            tk::check_encode_policy_admission(
                enc.clone(),
                D::new(),
                img.as_slice(),
                Metadata::none(),
                EncodePolicy::none(),
                policy,
                accepted,
                &expected,
            )
            .unwrap();
        }
        for (policy, accepted) in [
            (DecodePolicy::none(), true),
            (DecodePolicy::strict(), false),
        ] {
            tk::check_decode_policy_admission(
                D::new(),
                bytes.data(),
                DecodePolicy::none(),
                policy,
                accepted,
                &expected,
            )
            .unwrap();
        }
        tk::check_inventory_dispatch(D::new(), bytes.data()).unwrap();
        #[cfg(feature = "inventory")]
        tk::check_inventory_input_limit(D::new(), bytes.data()).unwrap();
    }
}

#[test]
fn decoder_contracts_without_requiring_a_quantizer() {
    use zencodec::decode::{Decode, DecodeJob, DecodePolicy, DecoderConfig};
    let bytes = include_bytes!("corpus/codec-corpus/sample_1.gif");
    let dec = zengif::GifDecoderConfig::new();
    let metadata = dec
        .clone()
        .job()
        .decoder(std::borrow::Cow::Borrowed(bytes), &[])
        .unwrap()
        .decode()
        .unwrap()
        .metadata();
    zencodec_testkit::check_decode_policy_admission(
        dec.clone(),
        bytes,
        DecodePolicy::none(),
        DecodePolicy::strict(),
        false,
        &metadata,
    )
    .unwrap();
    zencodec_testkit::check_inventory_dispatch(dec.clone(), bytes).unwrap();
    #[cfg(feature = "inventory")]
    zencodec_testkit::check_inventory_input_limit(dec, bytes).unwrap();
}

/// Write one-pixel patches without GCE transparency or canvas clearing.
fn preference_fixture(width: u16, patches: &[(u16, u16)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut encoder = gif::Encoder::new(&mut data, width, 1, &[11, 22, 33, 44, 55, 66]).unwrap();
    for &(left, duration) in patches {
        encoder
            .write_frame(&gif::Frame {
                left,
                width: 1,
                height: 1,
                delay: duration,
                dispose: gif::DisposalMethod::Keep,
                buffer: std::borrow::Cow::Owned(vec![left as u8]),
                ..gif::Frame::default()
            })
            .unwrap();
    }
    encoder.into_inner().unwrap();
    data
}

#[test]
fn ranked_preferences_preserve_alpha_and_deliver_exact_animation_timing() {
    use zencodec::{OrientationHint, animation::FrameDuration, decode::FrameCompletion};
    use zencodec_testkit::{DisplayFrameCase, check_animation_delivery, check_decode_preferences};
    use zengif::GifDecoderConfig;
    use zenpixels::{PixelFormat, PixelView};

    // The first patch leaves a clear pixel and the second makes the display
    // fully opaque, without any transparent index on either frame.
    let data = preference_fixture(2, &[(0, 0), (1, u16::MAX)]);
    let rgba = PixelFormat::RGBA8_SRGB;
    let rgb = PixelFormat::RGB8_SRGB;
    let bgra = PixelFormat::BGRA8_SRGB;
    let partial = [11, 22, 33, 255, 0, 0, 0, 0];
    let opaque = [11, 22, 33, 255, 44, 55, 66, 255];
    for (preferred, opaque_bytes, opaque_spec) in [
        (
            vec![rgba.clone(), rgb.clone(), bgra],
            opaque.as_slice(),
            &zenpixels::formats::RGBA8_SRGB,
        ),
        (
            vec![rgb.clone()],
            &[11, 22, 33, 44, 55, 66][..],
            &zenpixels::formats::RGB8_SRGB,
        ),
    ] {
        let first = PixelView::from_bytes(&partial, 2, 1, &zenpixels::formats::RGBA8_SRGB).unwrap();
        check_decode_preferences(GifDecoderConfig::new(), &data, &preferred, first.clone())
            .unwrap();
        check_animation_delivery(
            GifDecoderConfig::new(),
            &data,
            &preferred,
            OrientationHint::Preserve,
            Some(1),
            &[
                DisplayFrameCase {
                    pixels: first,
                    timing: FrameCompletion {
                        duration: FrameDuration::new(0, 100).unwrap(),
                        frame_index: 0,
                        pts: None,
                    },
                },
                DisplayFrameCase {
                    pixels: PixelView::from_bytes(opaque_bytes, 2, 1, opaque_spec).unwrap(),
                    timing: FrameCompletion {
                        duration: FrameDuration::new(u64::from(u16::MAX), 100).unwrap(),
                        frame_index: 1,
                        pts: None,
                    },
                },
            ],
        )
        .unwrap();
    }
}

#[test]
fn verified_opaque_still_prefers_rgb_when_ranked_first() {
    let data = preference_fixture(1, &[(0, 7)]);
    let expected =
        zenpixels::PixelView::from_bytes(&[11, 22, 33], 1, 1, &zenpixels::formats::RGB8_SRGB)
            .unwrap();
    zencodec_testkit::check_decode_preferences(
        zengif::GifDecoderConfig::new(),
        &data,
        &[
            zenpixels::PixelFormat::RGB8_SRGB,
            zenpixels::PixelFormat::RGBA8_SRGB,
        ],
        expected,
    )
    .unwrap();
}
