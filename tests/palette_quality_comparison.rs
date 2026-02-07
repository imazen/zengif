//! Test shared vs per-frame palette quality using SSIMULACRA2
#![cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
#![allow(dead_code)]

use imgref::ImgVec;
use std::fs;
use std::path::Path;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};

struct QualityResult {
    name: String,
    width: u16,
    height: u16,
    frame_count: usize,
    shared_size: usize,
    perframe_size: usize,
    shared_ssim2_avg: f64,
    shared_ssim2_worst: f64,
    perframe_ssim2_avg: f64,
    perframe_ssim2_worst: f64,
}

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
    // Convert to RGB arrays for fast-ssim2
    let orig_rgb: Vec<[u8; 3]> = original.iter().map(|p| [p.r, p.g, p.b]).collect();
    let enc_rgb: Vec<[u8; 3]> = encoded.iter().map(|p| [p.r, p.g, p.b]).collect();

    let orig_img = ImgVec::new(orig_rgb, width, height);
    let enc_img = ImgVec::new(enc_rgb, width, height);

    fast_ssim2::compute_ssimulacra2(orig_img.as_ref(), enc_img.as_ref()).unwrap_or(-1.0)
}

fn test_gif_quality(path: &Path) -> Option<QualityResult> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let data = fs::read(path).ok()?;

    // Decode original
    let (width, height, original_frames) = decode_to_frames(&data)?;
    if original_frames.is_empty() {
        return None;
    }
    let frame_count = original_frames.len();

    // Prepare frame inputs
    let frame_inputs: Vec<FrameInput> = original_frames
        .iter()
        .map(|pixels| FrameInput::new(width, height, 10, pixels.clone()))
        .collect();

    // Encode with shared palette
    let mut output_shared = Vec::new();
    {
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(true);
        let mut encoder = Encoder::new(
            &mut output_shared,
            width,
            height,
            config,
            Limits::none(),
            Unstoppable,
        )
        .ok()?;
        for frame in &frame_inputs {
            encoder.add_frame(frame.clone()).ok()?;
        }
        encoder.finish().ok()?;
    }

    // Encode with per-frame palette
    let mut output_perframe = Vec::new();
    {
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(false);
        let mut encoder = Encoder::new(
            &mut output_perframe,
            width,
            height,
            config,
            Limits::none(),
            Unstoppable,
        )
        .ok()?;
        for frame in &frame_inputs {
            encoder.add_frame(frame.clone()).ok()?;
        }
        encoder.finish().ok()?;
    }

    // Decode both outputs
    let (_, _, shared_frames) = decode_to_frames(&output_shared)?;
    let (_, _, perframe_frames) = decode_to_frames(&output_perframe)?;

    // Compute SSIM2 for each frame
    let w = width as usize;
    let h = height as usize;

    let mut shared_scores = Vec::new();
    let mut perframe_scores = Vec::new();

    let n = frame_count
        .min(shared_frames.len())
        .min(perframe_frames.len());
    for i in 0..n {
        let shared_score = compute_ssim2(&original_frames[i], &shared_frames[i], w, h);
        let perframe_score = compute_ssim2(&original_frames[i], &perframe_frames[i], w, h);

        if shared_score >= 0.0 {
            shared_scores.push(shared_score);
        }
        if perframe_score >= 0.0 {
            perframe_scores.push(perframe_score);
        }
    }

    if shared_scores.is_empty() || perframe_scores.is_empty() {
        return None;
    }

    let shared_avg = shared_scores.iter().sum::<f64>() / shared_scores.len() as f64;
    let shared_worst = shared_scores.iter().cloned().fold(f64::INFINITY, f64::min);
    let perframe_avg = perframe_scores.iter().sum::<f64>() / perframe_scores.len() as f64;
    let perframe_worst = perframe_scores
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);

    Some(QualityResult {
        name,
        width,
        height,
        frame_count,
        shared_size: output_shared.len(),
        perframe_size: output_perframe.len(),
        shared_ssim2_avg: shared_avg,
        shared_ssim2_worst: shared_worst,
        perframe_ssim2_avg: perframe_avg,
        perframe_ssim2_worst: perframe_worst,
    })
}

#[test]
fn compare_palette_quality() {
    let test_dir = Path::new("/tmp/gif-testset");
    if !test_dir.exists() {
        eprintln!("Test directory not found, skipping");
        return;
    }

    println!("\n=== Palette Quality Comparison (SSIMULACRA2) ===");
    println!("Higher SSIM2 = better (100=identical, <70=visible loss)\n");

    let mut results = Vec::new();

    for entry in fs::read_dir(test_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "gif").unwrap_or(false) {
            print!("Testing {:?}... ", path.file_name().unwrap());
            std::io::Write::flush(&mut std::io::stdout()).ok();
            if let Some(result) = test_gif_quality(&path) {
                println!("OK");
                results.push(result);
            } else {
                println!("SKIP");
            }
        }
    }

    results.sort_by_key(|r| (r.width as u64) * (r.height as u64));

    println!(
        "\n{:<32} {:>9} {:>5} {:>7} {:>7} {:>8} {:>8}",
        "File", "Dims", "Frms", "Sh KB", "PF KB", "Sh SSIM", "PF SSIM"
    );
    println!("{}", "-".repeat(85));

    for r in &results {
        let size_win = if r.shared_size <= r.perframe_size {
            "S"
        } else {
            "P"
        };
        let qual_win = if r.shared_ssim2_avg >= r.perframe_ssim2_avg - 0.5 {
            "S"
        } else {
            "P"
        };

        println!(
            "{:<32} {:>4}x{:<4} {:>5} {:>6}KB {:>6}KB {:>8.1} {:>8.1}  {}{}",
            &r.name[..r.name.len().min(32)],
            r.width,
            r.height,
            r.frame_count,
            r.shared_size / 1024,
            r.perframe_size / 1024,
            r.shared_ssim2_avg,
            r.perframe_ssim2_avg,
            size_win,
            qual_win
        );
    }

    println!("\n=== Worst-Frame Quality ===\n");
    println!(
        "{:<32} {:>10} {:>10} {:>10}",
        "File", "Sh Worst", "PF Worst", "Diff"
    );
    println!("{}", "-".repeat(65));

    for r in &results {
        let diff = r.shared_ssim2_worst - r.perframe_ssim2_worst;
        println!(
            "{:<32} {:>10.1} {:>10.1} {:>+10.1}",
            &r.name[..r.name.len().min(32)],
            r.shared_ssim2_worst,
            r.perframe_ssim2_worst,
            diff
        );
    }

    // Summary
    println!("\n=== Summary ===");
    let shared_smaller = results
        .iter()
        .filter(|r| r.shared_size <= r.perframe_size)
        .count();
    let shared_better_q = results
        .iter()
        .filter(|r| r.shared_ssim2_avg >= r.perframe_ssim2_avg - 0.5)
        .count();
    println!("Shared wins SIZE: {}/{}", shared_smaller, results.len());
    println!(
        "Shared wins/ties QUALITY (within 0.5): {}/{}",
        shared_better_q,
        results.len()
    );
}
