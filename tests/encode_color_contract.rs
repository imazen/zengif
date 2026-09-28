#![cfg(all(feature = "std", feature = "zenquant"))]
use std::sync::Arc;
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zenpixels::{
    AlphaMode, Cicp, ColorContext, ColorPrimaries, PixelBuffer, PixelDescriptor, SignalRange,
    TransferFunction,
};

fn rejected(pixels: PixelBuffer) {
    assert!(
        zengif::GifEncoderConfig::new()
            .job()
            .encoder()
            .unwrap()
            .encode(pixels.as_slice())
            .is_err(),
        "still accepted {:?}",
        pixels.descriptor()
    );
    let mut animation = zengif::GifEncoderConfig::new()
        .job()
        .animation_frame_encoder()
        .unwrap();
    assert!(
        animation.push_frame(pixels.as_slice(), 10, None).is_err(),
        "animation accepted {:?}",
        pixels.descriptor()
    );
    // Bad input must not initialize or poison an otherwise valid encoder.
    let valid =
        PixelBuffer::from_vec(vec![127, 0, 255, 255], 1, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    animation.push_frame(valid.as_slice(), 10, None).unwrap();
    animation.finish(None).unwrap();
}
#[test]
fn unsupported_color_and_alpha_are_rejected_before_admission() {
    for desc in [
        PixelDescriptor::RGBA8_SRGB.with_primaries(ColorPrimaries::DisplayP3),
        PixelDescriptor::RGBA8_SRGB.with_transfer(TransferFunction::Linear),
        PixelDescriptor::RGBA8_SRGB.with_transfer(TransferFunction::Unknown),
        PixelDescriptor::RGBA8_SRGB.with_signal_range(SignalRange::Narrow),
        PixelDescriptor::RGBA8_SRGB.with_alpha_mode(Some(AlphaMode::Premultiplied)),
    ] {
        rejected(PixelBuffer::from_vec(vec![20, 30, 40, 127], 1, 1, desc).unwrap());
    }
    let pixels =
        PixelBuffer::from_vec(vec![20, 30, 40, 255], 1, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    rejected(
        PixelBuffer::from_vec(vec![20, 30, 40, 255], 1, 1, PixelDescriptor::RGBA8_SRGB)
            .unwrap()
            .with_color_context(Arc::new(ColorContext::from_icc(Arc::<[u8]>::from([
                1, 2, 3,
            ])))),
    );
    rejected(
        pixels.with_color_context(Arc::new(ColorContext::from_cicp(Cicp::new(
            12, 13, 0, true,
        )))),
    );
}
#[test]
fn floats_must_be_declared_linear_sdr_and_finite() {
    for (desc, value) in [
        (
            PixelDescriptor::RGBF32_LINEAR.with_transfer(TransferFunction::Srgb),
            0.5f32,
        ),
        (PixelDescriptor::RGBF32_LINEAR, f32::NAN),
        (PixelDescriptor::RGBF32_LINEAR, f32::INFINITY),
        (PixelDescriptor::RGBF32_LINEAR, -0.01),
        (PixelDescriptor::RGBF32_LINEAR, 1.01),
    ] {
        let bytes: Vec<_> = [value; 3].into_iter().flat_map(f32::to_ne_bytes).collect();
        rejected(PixelBuffer::from_vec(bytes, 1, 1, desc).unwrap());
    }
}
