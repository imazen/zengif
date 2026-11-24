#![cfg(all(feature = "std", feature = "zenquant"))]
use std::sync::Arc;
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zenpixels::{
    AlphaMode, Cicp, ColorSpace, ImageSpec, PixelBuf, Primaries, RenderingIntent, SampleLayout,
    SignalRange, Transfer,
};

#[allow(deprecated)] // Exercise frame admission with the legacy metadata path.
fn rejected(pixels: PixelBuf, metadata: zencodec::Metadata) {
    assert!(
        zengif::GifEncoderConfig::new()
            .job()
            .with_metadata(metadata.clone())
            .encoder()
            .unwrap()
            .encode(pixels.as_view())
            .is_err(),
        "still accepted {:?}",
        pixels.spec()
    );
    let mut animation = zengif::GifEncoderConfig::new()
        .job()
        .with_metadata(metadata)
        .animation_frame_encoder()
        .unwrap();
    assert!(
        animation.push_frame(pixels.as_view(), 10, None).is_err(),
        "animation accepted {:?}",
        pixels.spec()
    );
    // Bad input must not initialize or poison an otherwise valid encoder.
    let valid =
        PixelBuf::from_bytes(vec![127, 0, 255, 255], 1, 1, ImageSpec::rgba8_srgb()).unwrap();
    // A separate frame can satisfy the job's declared color when a prior
    // frame conflicted with it.
    animation.push_frame(valid.as_view(), 10, None).unwrap();
    animation.finish(None).unwrap();
}
#[test]
fn unsupported_color_and_alpha_are_rejected_before_admission() {
    for desc in [
        ImageSpec::rgba8_srgb().with_primaries(Primaries::DisplayP3),
        ImageSpec::rgba8_srgb().with_transfer(Transfer::Linear),
        ImageSpec::rgba8_srgb().with_transfer(Transfer::Unknown),
        ImageSpec::rgba8_srgb().with_range(SignalRange::Narrow),
        ImageSpec::rgba8_srgb().with_alpha(AlphaMode::Premultiplied),
    ] {
        rejected(
            PixelBuf::from_bytes(vec![20, 30, 40, 127], 1, 1, desc).unwrap(),
            zencodec::Metadata::default(),
        );
    }
    rejected(
        PixelBuf::from_bytes(
            vec![20, 30, 40, 255],
            1,
            1,
            ImageSpec::new(
                SampleLayout::RGBA8,
                ColorSpace::Icc {
                    profile: Arc::from([1, 2, 3]),
                    channels: 3,
                    alpha: AlphaMode::Straight,
                    intent: RenderingIntent::default(),
                },
            ),
        )
        .unwrap(),
        zencodec::Metadata::default(),
    );
    let mut metadata = zencodec::Metadata::default();
    metadata.cicp = Some(Cicp::new(1, 13, 0, true));
    rejected(
        PixelBuf::from_vec([0.5f32; 4].to_vec(), 1, 1, ImageSpec::rgbaf32_linear()).unwrap(),
        metadata,
    );
}
#[test]
fn floats_must_be_declared_linear_sdr_and_finite() {
    for (desc, value) in [
        (
            ImageSpec::rgbf32_linear().with_transfer(Transfer::Srgb),
            0.5f32,
        ),
        (ImageSpec::rgbf32_linear(), f32::NAN),
        (ImageSpec::rgbf32_linear(), f32::INFINITY),
        (ImageSpec::rgbf32_linear(), -0.01),
        (ImageSpec::rgbf32_linear(), 1.01),
    ] {
        let bytes: Vec<_> = [value; 3].into_iter().flat_map(f32::to_ne_bytes).collect();
        rejected(
            PixelBuf::from_bytes(bytes, 1, 1, desc).unwrap(),
            zencodec::Metadata::default(),
        );
    }
}

#[test]
fn required_icc_and_cicp_refuse_before_output_with_explicit_srgb_fallback() {
    use zencodec::decode::DecoderConfig as _;
    use zencodec::encode::DynEncoderConfig as _;
    use zencodec::{CodecErrorExt, Metadata, MetadataFields, MetadataPolicy, UnsupportedOperation};
    use zenpixels::PixelView;

    let profile = moxcms::ColorProfile::new_srgb().encode().unwrap();
    assert_eq!(
        moxcms::ColorProfile::new_from_slice(&profile)
            .unwrap()
            .color_space,
        moxcms::DataColorSpace::Rgb,
    );
    assert!(moxcms::ColorProfile::new_from_slice(&[0; 256]).is_err());
    let source_icc = Metadata::none().with_icc(profile);
    let source_cicp = Metadata::none().with_cicp(Cicp::SRGB);
    let output_pixels = [11, 22, 33, 255];
    let view = PixelView::from_packed(&output_pixels, 1, 1, ImageSpec::rgba8_srgb()).unwrap();
    let cfg = zengif::GifEncoderConfig::new();
    for source in [source_icc, source_cicp] {
        let still = cfg
            .clone()
            .job()
            .try_with_metadata_policy(source.clone(), MetadataPolicy::PreserveExact)
            .map(|job| job.encoder());
        assert_eq!(
            still.err().unwrap().unsupported_operation(),
            Some(&UnsupportedOperation::MetadataRetention)
        );

        let mut animation = cfg.clone().dyn_job();
        let error = animation
            .try_set_metadata_policy(source.clone(), MetadataPolicy::PreserveExact)
            .unwrap_err();
        assert_eq!(
            error.unsupported_operation(),
            Some(&UnsupportedOperation::MetadataRetention)
        );

        // Discarding all requested file carriers admits the same job with
        // ordinary untagged sRGB output.
        let discard = MetadataPolicy::Custom(MetadataFields::DISCARD_ALL);
        let still = cfg
            .clone()
            .job()
            .try_with_metadata_policy(source.clone(), discard)
            .unwrap()
            .encode(view.clone())
            .unwrap();
        animation.try_set_metadata_policy(source, discard).unwrap();
        let mut animation = animation.into_animation_frame_encoder().unwrap();
        assert!(animation.pending_output().is_empty());
        animation.push_frame(view.clone(), 70, None).unwrap();
        let animated = animation.finish(None).unwrap();
        for output in [still, animated] {
            let decoded = zengif::GifDecoderConfig::new()
                .decode(output.data())
                .unwrap();
            assert_eq!(decoded.pixels().row(0), output_pixels);
            assert_eq!(decoded.pixels().spec(), &ImageSpec::rgba8_srgb());
            assert_eq!(
                decoded.buffer().origin(),
                &zenpixels::ColorOrigin::assumed()
            );
            assert_eq!(decoded.metadata().icc_profile, None);
            assert_eq!(decoded.metadata().cicp, None);
        }
    }
}
