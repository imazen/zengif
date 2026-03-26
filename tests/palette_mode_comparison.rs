//! Test shared vs per-frame palette on real GIFs
#![cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant"))]
#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::time::Instant;
use zengif::{Decoder, EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

struct PaletteTestResult {
    name: String,
    width: u16,
    height: u16,
    frame_count: usize,
    original_size: usize,
    shared_size: usize,
    perframe_size: usize,
    shared_time_ms: u64,
    perframe_time_ms: u64,
}

fn test_gif(path: &Path) -> Option<PaletteTestResult> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let data = fs::read(path).ok()?;

    // Decode
    let cursor = std::io::Cursor::new(&data);
    let mut decoder = Decoder::new(cursor, Limits::none(), &Unstoppable).ok()?;
    let metadata = decoder.metadata().clone();

    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().ok()? {
        frames.push(frame);
    }

    if frames.is_empty() {
        return None;
    }

    let width = metadata.width;
    let height = metadata.height;
    let frame_count = frames.len();

    // Prepare frame inputs
    let frame_inputs: Vec<FrameInput> = frames
        .iter()
        .map(|f| FrameInput::new(width, height, f.delay, f.pixels.clone()))
        .collect();

    // Test 1: Shared palette
    let start = Instant::now();
    let output_shared = {
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(true);
        let limits = Limits::none();
        let mut encoder = EncodeRequest::new(&config, width, height)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .ok()?;
        for frame in &frame_inputs {
            encoder.add_frame(frame.clone()).ok()?;
        }
        encoder.finish().ok()?
    };
    let time_shared = start.elapsed();

    // Test 2: Per-frame palette
    let start = Instant::now();
    let output_perframe = {
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(false);
        let limits = Limits::none();
        let mut encoder = EncodeRequest::new(&config, width, height)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .ok()?;
        for frame in &frame_inputs {
            encoder.add_frame(frame.clone()).ok()?;
        }
        encoder.finish().ok()?
    };
    let time_perframe = start.elapsed();

    Some(PaletteTestResult {
        name,
        width,
        height,
        frame_count,
        original_size: data.len(),
        shared_size: output_shared.len(),
        perframe_size: output_perframe.len(),
        shared_time_ms: time_shared.as_millis() as u64,
        perframe_time_ms: time_perframe.as_millis() as u64,
    })
}

#[test]
fn compare_palette_modes() {
    let test_dir = Path::new("/tmp/gif-testset");
    if !test_dir.exists() {
        eprintln!("Test directory not found, skipping");
        return;
    }

    println!("\n=== Palette Mode Comparison ===\n");

    let mut results = Vec::new();

    for entry in fs::read_dir(test_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "gif").unwrap_or(false) {
            print!("Testing {:?}... ", path.file_name().unwrap());
            if let Some(result) = test_gif(&path) {
                println!("OK");
                results.push(result);
            } else {
                println!("SKIP");
            }
        }
    }

    // Sort by pixel count
    results.sort_by_key(|r| (r.width as u64) * (r.height as u64));

    println!(
        "\n{:<40} {:>10} {:>6} {:>9} {:>9} {:>9} {:>7} {:>7}",
        "File", "Dims", "Frms", "Original", "Shared", "PerFrm", "Sh ms", "PF ms"
    );
    println!("{}", "-".repeat(105));

    for r in &results {
        let winner = if r.shared_size <= r.perframe_size {
            "<"
        } else {
            ">"
        };

        println!(
            "{:<40} {:>4}x{:<4} {:>6} {:>8}KB {:>8}KB {:>8}KB {:>7} {:>7} {}",
            &r.name[..r.name.len().min(40)],
            r.width,
            r.height,
            r.frame_count,
            r.original_size / 1024,
            r.shared_size / 1024,
            r.perframe_size / 1024,
            r.shared_time_ms,
            r.perframe_time_ms,
            winner
        );
    }

    println!("\n=== Analysis (< means shared wins, > means perframe wins) ===\n");

    let mut shared_wins = 0;
    let mut perframe_wins = 0;

    for r in &results {
        let pixels = (r.width as u64) * (r.height as u64);
        let shared_better = r.shared_size <= r.perframe_size;

        if shared_better {
            shared_wins += 1;
        } else {
            perframe_wins += 1;
        }

        let diff_kb = (r.shared_size as i64 - r.perframe_size as i64).abs() / 1024;
        let diff_pct = if shared_better {
            (1.0 - r.shared_size as f64 / r.perframe_size as f64) * 100.0
        } else {
            (1.0 - r.perframe_size as f64 / r.shared_size as f64) * 100.0
        };

        println!(
            "{:>6} px ({:>4}x{:<4}) {:>6} frames: {} by {:>4}KB ({:.1}%)",
            pixels,
            r.width,
            r.height,
            r.frame_count,
            if shared_better {
                "SHARED  "
            } else {
                "PERFRAME"
            },
            diff_kb,
            diff_pct
        );
    }

    println!(
        "\nSummary: shared wins {}, perframe wins {}",
        shared_wins, perframe_wins
    );
}
