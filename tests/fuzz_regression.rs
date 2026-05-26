//! Fuzz crash regression suite (DEDUP-J template, ported from zenwebp).
//!
//! Runs every file in `fuzz/regression/` through every decoder entry point that
//! has a fuzz target. Each seed file is a previously-found crash that has been
//! fixed; this test ensures none of them re-introduce a panic.
//!
//! Reproduces what the `fuzz_decode`, `fuzz_decode_streaming`, `fuzz_limits`,
//! and `fuzz_roundtrip` fuzz targets do, but as a regular `cargo test` — no
//! nightly toolchain needed. Failures here mean a regression of a
//! previously-fixed bug.
//!
//! To add a new seed: drop the (preferably minimized) crash file into
//! `fuzz/regression/<target_name>/` (or directly into `fuzz/regression/`),
//! no other action required.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use zengif::{decode_gif, encode_gif, Decoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn regression_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/regression")
}

/// Recursively collect every regular file under `dir`. Skips dotfiles and
/// silently tolerates a missing directory.
fn collect_seeds(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let read = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return,
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_file() => out.push(path),
            Ok(t) if t.is_dir() => collect_seeds(&path, out),
            _ => {}
        }
    }
}

fn fuzz_limits() -> Limits {
    Limits::default()
        .max_dimensions(1024, 1024)
        .max_total_pixels(1_000_000)
        .max_frame_count(100)
        .max_file_size(1024 * 1024)
        .max_memory(10 * 1024 * 1024)
        .max_decompression_ratio(100.0)
}

fn run_decode(input: &[u8]) {
    // Mirrors fuzz_decode.rs.
    let _ = decode_gif(input, fuzz_limits(), &Unstoppable);
}

fn run_decode_streaming(input: &[u8]) {
    // Mirrors fuzz_decode_streaming.rs.
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
}

fn run_roundtrip(input: &[u8]) {
    // Mirrors fuzz_roundtrip.rs (tighter limits for the encode step).
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
}

#[test]
fn fuzz_regression_seeds_do_not_panic() {
    let dir = regression_dir();
    let mut seeds = Vec::new();
    collect_seeds(&dir, &mut seeds);

    if seeds.is_empty() {
        eprintln!(
            "note: no regression seeds found under {} — nothing to check",
            dir.display()
        );
        return;
    }

    for path in seeds {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        let input = fs::read(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));

        // Each entry point may return Err but must not panic. If any panics,
        // the test fails with the seed name in the unwind message.
        run_decode(&input);
        run_decode_streaming(&input);
        run_roundtrip(&input);

        eprintln!("ok: {name} ({} bytes)", input.len());
    }
}
