//! Replay seed inputs from `fuzz/regression/` through every fuzz target
//! entry point. Shared scaffolding lives in `zenutils-fuzz`.
//!
//! The decode/encode entry points exercised here (`decode_gif`, `encode_gif`,
//! `Decoder`, `EncoderConfig`) are gated behind the `std` feature, so this whole
//! test compiles to nothing without it (e.g. `--no-default-features`).
#![cfg(feature = "std")]

use std::io::Cursor;
use std::path::Path;
use zengif::{
    Decoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable, decode_gif, encode_gif,
};
use zenutils_fuzz::RegressionSuite;

/// Lower bound on the replayable seed corpus committed under `fuzz/regression/`.
///
/// `RegressionSuite` treats a missing or empty seed directory as a clean no-op,
/// so an emptied, renamed, or never-checked-out corpus would let this test pass
/// without replaying a single seed. Pinning the floor makes that a loud failure.
/// Raise this when seeds are added; only lower it when deleting seeds on purpose.
const MIN_SEEDS: usize = 2;

/// Count the files `RegressionSuite::run` will actually replay, using its own
/// filters: recurse into subdirectories, skip dotfiles, `*.md` and `*.txt`.
fn replayable_seeds(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut found = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            found += replayable_seeds(&path);
        } else if path.is_file() {
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".md") && !lower.ends_with(".txt") {
                found += 1;
            }
        }
    }
    found
}

/// Fail loudly when the corpus this suite exists to replay is not there.
fn assert_corpus_present() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/regression");
    let found = replayable_seeds(&dir);
    assert!(
        found >= MIN_SEEDS,
        "{} holds {found} replayable seeds, expected at least {MIN_SEEDS} — \
         the committed regression corpus is missing or was renamed, which would \
         otherwise let this test pass without replaying anything",
        dir.display()
    );
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

#[test]
fn fuzz_regression() {
    assert_corpus_present();
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
            while let Ok(Some(_frame)) = decoder.next_frame() {
                frame_count += 1;
                if frame_count >= 100 {
                    break;
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
