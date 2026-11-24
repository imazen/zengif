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
