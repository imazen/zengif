//! Replay seed inputs from `fuzz/regression/` through every fuzz target
//! entry point. Shared scaffolding lives in `zen-fuzz-regress`.

use std::io::Cursor;
use zen_fuzz_regress::RegressionSuite;
use zengif::{decode_gif, encode_gif, Decoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn fuzz_limits() -> Limits {
    Limits::default()
        .max_dimensions(1024, 1024)
        .max_total_pixels(1_000_000)
        .max_frame_count(100)
        .max_file_size(1024 * 1024)
        .max_memory(10 * 1024 * 1024)
        .max_decompression_ratio(100.0)
}

#[test]
fn fuzz_regression() {
    RegressionSuite::new("fuzz/regression")
        .target("decode", |input| {
            let _ = decode_gif(input, fuzz_limits(), &Unstoppable);
        })
        .target("decode_streaming", |input| {
            let reader = Cursor::new(input);
            let Ok(mut decoder) = Decoder::new(reader, fuzz_limits(), &Unstoppable) else {
                return;
            };
            let mut frame_count = 0;
            loop {
                match decoder.next_frame() {
                    Ok(Some(_frame)) => {
                        frame_count += 1;
                        if frame_count >= 100 {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        })
        .target("roundtrip", |input| {
            let limits = Limits::default()
                .max_dimensions(256, 256)
                .max_total_pixels(65536)
                .max_frame_count(10)
                .max_file_size(512 * 1024)
                .max_memory(5 * 1024 * 1024)
                .max_decompression_ratio(100.0);
            let (metadata, frames, _stats) = match decode_gif(input, limits.clone(), &Unstoppable) {
                Ok(result) if !result.1.is_empty() => result,
                _ => return,
            };
            let frame_inputs: Vec<FrameInput> = frames
                .iter()
                .map(|f| FrameInput::new(f.width, f.height, f.delay.max(1), f.pixels.clone()))
                .collect();
            let config = EncoderConfig::new().repeat(Repeat::Once);
            let Ok(output) = encode_gif(
                frame_inputs,
                metadata.width,
                metadata.height,
                config,
                limits.clone(),
                &Unstoppable,
            ) else {
                return;
            };
            if !output.is_empty() {
                let redecode_limits = limits.max_decompression_ratio(10000.0);
                let _ = decode_gif(&output, redecode_limits, &Unstoppable);
            }
        })
        .run();
}
