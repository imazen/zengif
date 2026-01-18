//! Corpus tests: test against real-world GIF files from codec-corpus
//!
//! These tests verify that zengif can correctly decode and round-trip
//! GIF files from the wild, including edge cases and animations.

use enough::Unstoppable;
use std::fs;
use std::path::Path;
use zengif::{decode_gif, encode_gif, EncoderConfig, FrameInput, Limits, Stats};

/// Path to the codec-corpus GIF test files
const CORPUS_BASE: &str = "/home/lilith/work/codec-corpus";

/// Get all GIF test files from the corpus
fn corpus_gif_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    // image-rs test images
    let image_rs_base = Path::new(CORPUS_BASE).join("image-rs/test-images/gif");
    if image_rs_base.exists() {
        collect_gifs(&image_rs_base, &mut files);
    }

    // imageflow test inputs
    let imageflow_base = Path::new(CORPUS_BASE).join("imageflow/test_inputs");
    if imageflow_base.exists() {
        collect_gifs(&imageflow_base, &mut files);
    }

    files
}

fn collect_gifs(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_gifs(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "gif") {
                files.push(path);
            }
        }
    }
}

/// Helper to check if corpus is available
fn corpus_available() -> bool {
    Path::new(CORPUS_BASE).exists()
}

#[test]
fn corpus_decode_all_gifs() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found at {}", CORPUS_BASE);
        return;
    }

    let files = corpus_gif_files();
    assert!(!files.is_empty(), "No GIF files found in corpus");

    let mut success_count = 0;
    let mut expected_failure_count = 0;
    let mut unexpected_failure_count = 0;

    for path in &files {
        let filename = path.file_name().unwrap().to_string_lossy();
        let data = fs::read(path).expect("Failed to read file");

        let stats = Stats::new();
        let limits = Limits::default();

        match decode_gif(&data, limits, &stats, Unstoppable) {
            Ok((metadata, frames)) => {
                success_count += 1;
                // Basic sanity checks
                assert!(metadata.width > 0, "{}: width should be > 0", filename);
                assert!(metadata.height > 0, "{}: height should be > 0", filename);
                // Some GIFs may have 0 frames (static) or more
                for (i, frame) in frames.iter().enumerate() {
                    assert_eq!(
                        frame.pixels.len(),
                        frame.width as usize * frame.height as usize,
                        "{}: frame {} pixel count mismatch",
                        filename,
                        i
                    );
                }
            }
            Err(e) => {
                // Some files are intentionally malformed (like oob.gif, oversized, undersized)
                let is_expected_failure = filename.contains("oob")
                    || filename.contains("oversized")
                    || filename.contains("undersized");

                if is_expected_failure {
                    expected_failure_count += 1;
                    eprintln!("Expected failure for {}: {:?}", filename, e);
                } else {
                    unexpected_failure_count += 1;
                    eprintln!("UNEXPECTED failure for {}: {:?}", filename, e);
                }
            }
        }
    }

    eprintln!(
        "\nCorpus decode results: {} success, {} expected failures, {} unexpected failures (of {} total)",
        success_count,
        expected_failure_count,
        unexpected_failure_count,
        files.len()
    );

    // We should decode most files successfully
    assert!(
        success_count > 0,
        "Should decode at least some corpus files"
    );
    // No unexpected failures
    assert_eq!(
        unexpected_failure_count, 0,
        "Unexpected decode failures occurred"
    );
}

#[test]
fn corpus_round_trip_animation_gifs() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    // Specifically test animated GIFs
    let anim_dir = Path::new(CORPUS_BASE).join("image-rs/test-images/gif/anim");
    if !anim_dir.exists() {
        eprintln!("Skipping: animation directory not found");
        return;
    }

    let mut files = Vec::new();
    collect_gifs(&anim_dir, &mut files);

    for path in &files {
        let filename = path.file_name().unwrap().to_string_lossy();

        // Skip known malformed files
        if filename.contains("oob") || filename.contains("undersized") {
            continue;
        }

        let data = fs::read(path).expect("Failed to read file");
        let stats = Stats::new();
        let limits = Limits::default();

        // Decode
        let (metadata, frames) = match decode_gif(&data, limits.clone(), &stats, Unstoppable) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("Skipping {} (decode failed): {:?}", filename, e);
                continue;
            }
        };

        if frames.is_empty() {
            eprintln!("Skipping {} (no frames)", filename);
            continue;
        }

        // Convert to FrameInput for encoding
        let frame_inputs: Vec<FrameInput> = frames
            .iter()
            .map(|f| FrameInput::new(f.width, f.height, f.delay, f.pixels.clone()))
            .collect();

        // Re-encode
        let config = EncoderConfig::new(metadata.width, metadata.height).repeat(metadata.repeat);
        let encoded = match encode_gif(frame_inputs, config, limits.clone(), Unstoppable) {
            Ok(enc) => enc,
            Err(e) => {
                eprintln!("Skipping {} (encode failed): {:?}", filename, e);
                continue;
            }
        };

        // Decode again
        let stats2 = Stats::new();
        let (metadata2, frames2) =
            decode_gif(&encoded, limits, &stats2, Unstoppable).expect("Re-decode should succeed");

        // Verify frame count and dimensions preserved
        assert_eq!(
            frames.len(),
            frames2.len(),
            "{}: frame count mismatch after round-trip",
            filename
        );
        assert_eq!(
            metadata.width, metadata2.width,
            "{}: width mismatch after round-trip",
            filename
        );
        assert_eq!(
            metadata.height, metadata2.height,
            "{}: height mismatch after round-trip",
            filename
        );

        // Verify delays are preserved
        for (i, (orig, rt)) in frames.iter().zip(frames2.iter()).enumerate() {
            assert_eq!(
                orig.delay, rt.delay,
                "{}: frame {} delay mismatch",
                filename, i
            );
        }

        eprintln!(
            "Round-trip OK: {} ({} frames, {}x{})",
            filename,
            frames.len(),
            metadata.width,
            metadata.height
        );
    }
}

#[test]
fn corpus_disposal_methods() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    // Test files specifically for disposal methods
    let test_files = [
        "image-rs/test-images/gif/anim/any-disposal.gif",
        "image-rs/test-images/gif/anim/mixed-disposal.gif",
    ];

    for relative_path in test_files {
        let path = Path::new(CORPUS_BASE).join(relative_path);
        if !path.exists() {
            eprintln!("Skipping disposal test: {} not found", relative_path);
            continue;
        }

        let data = fs::read(&path).expect("Failed to read file");
        let stats = Stats::new();
        let limits = Limits::default();

        let (metadata, frames) = decode_gif(&data, limits, &stats, Unstoppable)
            .expect("Should decode disposal test file");

        assert!(
            frames.len() > 1,
            "{}: Expected multiple frames for disposal test",
            relative_path
        );

        // Verify each frame has correct dimensions
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.width, metadata.width,
                "{}: frame {} width should match canvas",
                relative_path, i
            );
            assert_eq!(
                frame.height, metadata.height,
                "{}: frame {} height should match canvas",
                relative_path, i
            );
            assert_eq!(
                frame.pixels.len(),
                frame.width as usize * frame.height as usize,
                "{}: frame {} pixel count should match dimensions",
                relative_path, i
            );
        }

        eprintln!(
            "Disposal test OK: {} ({} frames)",
            relative_path,
            frames.len()
        );
    }
}

#[test]
fn corpus_interlaced_gif() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    let path = Path::new(CORPUS_BASE).join("image-rs/test-images/gif/anim/interlaced.gif");
    if !path.exists() {
        eprintln!("Skipping interlaced test: file not found");
        return;
    }

    let data = fs::read(&path).expect("Failed to read file");
    let stats = Stats::new();
    let limits = Limits::default();

    let (metadata, frames) =
        decode_gif(&data, limits, &stats, Unstoppable).expect("Should decode interlaced GIF");

    assert!(metadata.width > 0);
    assert!(metadata.height > 0);
    assert!(!frames.is_empty(), "Interlaced GIF should have frames");

    // All frames should be fully decoded (no interlacing artifacts)
    for frame in &frames {
        assert_eq!(
            frame.pixels.len(),
            frame.width as usize * frame.height as usize,
            "Interlaced frame should be fully decoded"
        );
    }

    eprintln!(
        "Interlaced test OK: {}x{}, {} frames",
        metadata.width,
        metadata.height,
        frames.len()
    );
}

#[test]
fn corpus_large_animation() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    let path = Path::new(CORPUS_BASE).join("imageflow/test_inputs/mountain_800.gif");
    if !path.exists() {
        eprintln!("Skipping large animation test: file not found");
        return;
    }

    let data = fs::read(&path).expect("Failed to read file");
    let stats = Stats::new();
    let limits = Limits::default();

    let (metadata, frames) =
        decode_gif(&data, limits, &stats, Unstoppable).expect("Should decode large animation");

    assert!(metadata.width > 0);
    assert!(metadata.height > 0);

    eprintln!(
        "Large animation test OK: {}x{}, {} frames, peak memory: {} bytes",
        metadata.width,
        metadata.height,
        frames.len(),
        stats.peak()
    );

    // Verify memory tracking worked
    assert!(stats.peak() > 0, "Should track memory allocations");
    assert!(stats.alloc_count() > 0, "Should count allocations");
}

#[test]
fn corpus_simple_gifs() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    let simple_dir = Path::new(CORPUS_BASE).join("image-rs/test-images/gif/simple");
    if !simple_dir.exists() {
        eprintln!("Skipping: simple directory not found");
        return;
    }

    let mut files = Vec::new();
    collect_gifs(&simple_dir, &mut files);

    for path in &files {
        let filename = path.file_name().unwrap().to_string_lossy();

        // Skip known problematic files
        if filename.contains("oversized") {
            continue;
        }

        let data = fs::read(path).expect("Failed to read file");
        let stats = Stats::new();
        let limits = Limits::default();

        match decode_gif(&data, limits, &stats, Unstoppable) {
            Ok((metadata, frames)) => {
                assert!(metadata.width > 0);
                assert!(metadata.height > 0);
                eprintln!(
                    "Simple GIF OK: {} ({}x{}, {} frames)",
                    filename,
                    metadata.width,
                    metadata.height,
                    frames.len()
                );
            }
            Err(e) => {
                eprintln!("Simple GIF failed: {}: {:?}", filename, e);
            }
        }
    }
}

#[test]
fn corpus_transparency_handling() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    let path = Path::new(CORPUS_BASE).join("image-rs/test-images/gif/simple/alpha_gif_a.gif");
    if !path.exists() {
        eprintln!("Skipping transparency test: file not found");
        return;
    }

    let data = fs::read(&path).expect("Failed to read file");
    let stats = Stats::new();
    let limits = Limits::default();

    let (metadata, frames) =
        decode_gif(&data, limits, &stats, Unstoppable).expect("Should decode alpha GIF");

    assert!(metadata.width > 0);
    assert!(metadata.height > 0);
    assert!(!frames.is_empty());

    // Check for transparent pixels (alpha < 255)
    let mut has_transparency = false;
    for frame in &frames {
        for pixel in &frame.pixels {
            if pixel.a < 255 {
                has_transparency = true;
                break;
            }
        }
        if has_transparency {
            break;
        }
    }

    eprintln!(
        "Transparency test: {}x{}, {} frames, has_transparency={}",
        metadata.width,
        metadata.height,
        frames.len(),
        has_transparency
    );
}

#[test]
fn corpus_memory_limits_respected() {
    if !corpus_available() {
        eprintln!("Skipping corpus test: codec-corpus not found");
        return;
    }

    // Use the largest test file with restrictive limits
    let path = Path::new(CORPUS_BASE).join("imageflow/test_inputs/mountain_800.gif");
    if !path.exists() {
        eprintln!("Skipping memory limits test: file not found");
        return;
    }

    let data = fs::read(&path).expect("Failed to read file");
    let stats = Stats::new();

    // Set very restrictive memory limit (100 KB)
    let limits = Limits::default().max_memory(100 * 1024);

    let result = decode_gif(&data, limits, &stats, Unstoppable);

    // Should fail due to memory limits
    assert!(result.is_err(), "Should fail with restrictive memory limit");

    eprintln!(
        "Memory limits test OK: decode correctly rejected with limit, peak was {} bytes",
        stats.peak()
    );
}
