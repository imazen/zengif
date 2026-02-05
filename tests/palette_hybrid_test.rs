//! Test hybrid palette mode (shared with per-frame fallback)
#![cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]

use imgref::ImgVec;
use std::fs;
use std::path::Path;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};

fn decode_to_frames(data: &[u8]) -> Option<(u16, u16, Vec<Vec<Rgba>>)> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).ok()?;
    let metadata = decoder.metadata().clone();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().ok()? {
        frames.push(frame.pixels.clone());
    }
    Some((metadata.width, metadata.height, frames))
}

fn compute_ssim2(original: &[Rgba], encoded: &[Rgba], width: usize, height: usize) -> f64 {
    let orig_rgb: Vec<[u8; 3]> = original.iter().map(|p| [p.r, p.g, p.b]).collect();
    let enc_rgb: Vec<[u8; 3]> = encoded.iter().map(|p| [p.r, p.g, p.b]).collect();
    let orig_img = ImgVec::new(orig_rgb, width, height);
    let enc_img = ImgVec::new(enc_rgb, width, height);
    fast_ssim2::compute_ssimulacra2(orig_img.as_ref(), enc_img.as_ref()).unwrap_or(-1.0)
}

#[test]
fn test_hybrid_palette_quality() {
    let test_dir = Path::new("/tmp/gif-testset");
    if !test_dir.exists() {
        eprintln!("Test directory not found");
        return;
    }

    // Test the problematic GIFs that showed quality loss
    let problem_gifs = ["spinner", "cat_typing"];

    println!("\n=== Hybrid Palette Mode Test ===");
    println!("Testing if palette_error_threshold fixes quality issues\n");

    for entry in fs::read_dir(test_dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();

        if !problem_gifs.iter().any(|p| name.contains(p)) {
            continue;
        }

        let data = fs::read(&path).unwrap();
        let (width, height, original_frames) = decode_to_frames(&data).unwrap();
        let w = width as usize;
        let h = height as usize;

        let frame_inputs: Vec<FrameInput> = original_frames
            .iter()
            .map(|pixels| FrameInput::new(width, height, 10, pixels.clone()))
            .collect();

        println!("=== {} ({} frames) ===\n", name, frame_inputs.len());

        // Test different modes
        for (mode_name, shared, threshold) in [
            ("Per-frame only", false, None),
            ("Shared only", true, None),
            ("Hybrid (threshold=15)", true, Some(15.0)),
            ("Hybrid (threshold=10)", true, Some(10.0)),
            ("Hybrid (threshold=5)", true, Some(5.0)),
        ] {
            let mut output = Vec::new();
            {
                let mut config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
                config = config.shared_palette(shared);
                if let Some(t) = threshold {
                    config = config.palette_error_threshold(Some(t));
                } else if shared {
                    config = config.palette_error_threshold(None); // Disable fallback
                }
                let mut encoder =
                    Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();
                for frame in &frame_inputs {
                    encoder.add_frame(frame.clone()).unwrap();
                }
                encoder.finish().unwrap();
            }

            // Decode and measure quality
            let (_, _, encoded_frames) = decode_to_frames(&output).unwrap();

            let mut scores = Vec::new();
            for i in 0..original_frames.len().min(encoded_frames.len()) {
                let score = compute_ssim2(&original_frames[i], &encoded_frames[i], w, h);
                if score >= 0.0 {
                    scores.push(score);
                }
            }

            let avg = scores.iter().sum::<f64>() / scores.len() as f64;
            let worst = scores.iter().cloned().fold(f64::INFINITY, f64::min);

            println!(
                "{:<25} {:>7}KB  avg={:>5.1}  worst={:>5.1}",
                mode_name,
                output.len() / 1024,
                avg,
                worst
            );
        }
        println!();
    }
}
