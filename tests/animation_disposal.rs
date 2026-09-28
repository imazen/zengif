#![cfg(feature = "std")]
use std::borrow::Cow;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zengif::{EncoderConfig, FrameInput, Limits, Rgba};

fn canvases(w: u16, h: u16) -> Vec<Vec<Rgba>> {
    let size = usize::from(w) * usize::from(h);
    let clear = vec![Rgba::TRANSPARENT; size];
    let red = vec![Rgba::rgb(255, 0, 0); size];
    let mut changed = red.clone();
    changed[0] = Rgba::rgb(0, 0, 255);
    let mut edge = clear.clone();
    edge[size - 1] = Rgba::rgb(0, 255, 0);
    vec![clear.clone(), red, changed, clear.clone(), edge, clear]
}
fn verify(bytes: &[u8], expected: &[Vec<Rgba>]) {
    verify_wire_disposal(bytes, expected);
    let config = zengif::GifDecoderConfig::new();
    let mut decoder = config
        .job()
        .animation_frame_decoder(Cow::Borrowed(bytes), &[])
        .unwrap();
    for (index, pixels) in expected.iter().enumerate() {
        let frame = decoder.render_next_frame_owned(None).unwrap().unwrap();
        assert_eq!(frame.frame_index(), index as u32);
        assert_eq!(frame.duration_ms(), (index as u32 + 1) * 10);
        let actual = frame.pixels();
        let expected: Vec<_> = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        for y in 0..actual.rows() {
            let stride = actual.width() as usize * 4;
            assert_eq!(
                actual.row(y),
                &expected[y as usize * stride..(y as usize + 1) * stride],
                "frame {index} row {y}"
            );
        }
    }
    assert!(decoder.render_next_frame_owned(None).unwrap().is_none());
}

// Independent command-level oracle. In particular, disposal-to-background
// without a transparent index restores the logical screen's opaque background,
// as FFmpeg gifdec.c stores in stored_bg_color. Do not reuse zengif's compositor.
fn verify_wire_disposal(bytes: &[u8], expected: &[Vec<Rgba>]) {
    let options = gif::DecodeOptions::new();
    let mut reader = options.read_info(std::io::Cursor::new(bytes)).unwrap();
    let (w, h) = (usize::from(reader.width()), usize::from(reader.height()));
    let global = reader.global_palette().unwrap_or(&[]).to_vec();
    let bg = reader.bg_color().unwrap_or(0);
    let background = if global.len() >= 3 * (bg + 1) {
        Rgba::rgb(global[3 * bg], global[3 * bg + 1], global[3 * bg + 2])
    } else {
        Rgba::TRANSPARENT
    };
    let mut canvas = vec![Rgba::TRANSPARENT; w * h];
    let mut index = 0;
    while let Some(frame) = reader.read_next_frame().unwrap() {
        let palette = frame.palette.as_deref().unwrap_or(&global);
        for y in 0..usize::from(frame.height) {
            for x in 0..usize::from(frame.width) {
                let value = frame.buffer[y * usize::from(frame.width) + x];
                if Some(value) != frame.transparent {
                    let offset = usize::from(value) * 3;
                    canvas[(y + usize::from(frame.top)) * w + x + usize::from(frame.left)] =
                        Rgba::rgb(palette[offset], palette[offset + 1], palette[offset + 2]);
                }
            }
        }
        assert_eq!(&canvas, &expected[index], "wire display {index}");
        if frame.dispose == gif::DisposalMethod::Background {
            let clear = if frame.transparent.is_some() {
                Rgba::TRANSPARENT
            } else {
                background
            };
            for y in 0..usize::from(frame.height) {
                let start = (y + usize::from(frame.top)) * w + usize::from(frame.left);
                canvas[start..start + usize::from(frame.width)].fill(clear);
            }
        } else {
            assert!(matches!(
                frame.dispose,
                gif::DisposalMethod::Keep | gif::DisposalMethod::Any
            ));
        }
        index += 1;
    }
    assert_eq!(index, expected.len());
}

#[cfg(feature = "zenquant")]
#[test]
fn full_canvas_erasure_survives_palette_windows_and_diff_modes() {
    for (w, h) in [(1, 1), (17, 13)] {
        let source = canvases(w, h);
        for window in [0, 1, 3, 50] {
            for diff in [false, true] {
                let config = EncoderConfig::new()
                    .dithering(0.0)
                    .use_transparency(diff)
                    .shared_palette(window != 0)
                    .max_buffer_frames(window.max(1));
                let frames = source
                    .iter()
                    .enumerate()
                    .map(|(i, p)| FrameInput::new(w, h, i as u16 + 1, p.clone()))
                    .collect();
                let output =
                    zengif::encode_gif(frames, w, h, config, Limits::none(), &enough::Unstoppable)
                        .unwrap();
                verify(&output, &source);
            }
        }
        let frames = source
            .iter()
            .enumerate()
            .map(|(i, p)| FrameInput::new(w, h, i as u16 + 1, p.clone()))
            .collect();
        let output = zengif::encode_gif_shared_palette(
            frames,
            w,
            h,
            EncoderConfig::new().dithering(0.0),
            Limits::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        verify(&output, &source);
    }
}

#[test]
fn supplied_palettes_preserve_clearing_without_a_quantizer() {
    let source = canvases(17, 13);
    let palette = zengif::Palette::from_rgba(vec![
        Rgba::TRANSPARENT,
        Rgba::rgb(255, 0, 0),
        Rgba::rgb(0, 255, 0),
        Rgba::rgb(0, 0, 255),
    ]);
    let frames = source
        .iter()
        .enumerate()
        .map(|(i, p)| FrameInput::with_palette(17, 13, i as u16 + 1, p.clone(), palette.clone()))
        .collect();
    let config = EncoderConfig::new().use_transparency(true);
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    let config = config.shared_palette(false);
    let output =
        zengif::encode_gif(frames, 17, 13, config, Limits::none(), &enough::Unstoppable).unwrap();
    verify(&output, &source);
}

#[test]
fn clearing_a_display_using_all_256_colors_reserves_a_real_transparent_slot() {
    let gradient: Vec<_> = (0..=255).map(|v| Rgba::rgb(v, v, v)).collect();
    let clear = vec![Rgba::TRANSPARENT; 256];
    let frames = vec![
        FrameInput::with_palette(
            16,
            16,
            1,
            gradient.clone(),
            zengif::Palette::from_rgba(gradient.clone()),
        ),
        FrameInput::with_palette(
            16,
            16,
            2,
            clear.clone(),
            zengif::Palette::from_rgba(vec![Rgba::TRANSPARENT]),
        ),
    ];
    let config = EncoderConfig::new();
    #[cfg(any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    ))]
    let config = config.shared_palette(false);
    let bytes =
        zengif::encode_gif(frames, 16, 16, config, Limits::none(), &enough::Unstoppable).unwrap();
    // GIF needs one transparent index in the disposed frame. All colors are
    // equally frequent here: black merges into gray 1, with every other sample
    // unchanged. The following canvas must be truly transparent on the wire.
    let mut expected = gradient;
    expected[0] = Rgba::rgb(1, 1, 1);
    verify(&bytes, &[expected, clear]);
}
