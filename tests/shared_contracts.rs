#![cfg(feature = "std")]

// PixelSlice encoding requires one of the configured palette quantizers.
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
