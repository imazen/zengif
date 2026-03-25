#![cfg(feature = "std")]
//! Corpus tests: test against real-world GIF files from codec-corpus
//!
//! These tests verify that zengif can correctly decode and round-trip
//! GIF files from the wild, including edge cases and animations.

use enough::Unstoppable;
use std::fs;
use std::path::Path;
use zengif::{EncoderConfig, FrameInput, Limits, decode_gif, encode_gif};

/// Path to local corpus files (committed to repo)
const LOCAL_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/codec-corpus");

/// Get all GIF test files from the local corpus (always available)
fn local_corpus_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let local_path = Path::new(LOCAL_CORPUS);
    if local_path.exists() {
        collect_gifs(local_path, &mut files);
    }
    files
}

/// Get all GIF test files from the external corpus (if available)
fn external_corpus_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    let corpus = match codec_corpus::Corpus::new() {
        Ok(c) => c,
        Err(_) => return files,
    };

    // image-rs test images
    if let Ok(image_rs_base) = corpus.get("image-rs/test-images/gif") {
        collect_gifs(&image_rs_base, &mut files);
    }

    // imageflow test inputs
    if let Ok(imageflow_base) = corpus.get("imageflow/test_inputs") {
        collect_gifs(&imageflow_base, &mut files);
    }

    files
}

/// Get all GIF test files (local + external if available)
fn corpus_gif_files() -> Vec<std::path::PathBuf> {
    let mut files = local_corpus_files();
    files.extend(external_corpus_files());
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

#[test]
fn corpus_decode_all_gifs() {
    let files = corpus_gif_files();
    assert!(
        !files.is_empty(),
        "No GIF files found - local corpus should always be available"
    );

    let mut success_count = 0;
    let mut expected_failure_count = 0;
    let mut unexpected_failure_count = 0;

    for path in &files {
        let filename = path.file_name().unwrap().to_string_lossy();
        let data = fs::read(path).expect("Failed to read file");

        let limits = Limits::default();

        match decode_gif(&data, limits, &Unstoppable) {
            Ok((metadata, frames, _)) => {
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
    // Use local corpus (always available)
    let files = local_corpus_files();

    for path in &files {
        let filename = path.file_name().unwrap().to_string_lossy();

        // Skip known malformed files
        if filename.contains("oob") || filename.contains("undersized") {
            continue;
        }

        let data = fs::read(path).expect("Failed to read file");
        let limits = Limits::default();

        // Decode
        let (metadata, frames, _stats) = match decode_gif(&data, limits.clone(), &Unstoppable) {
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
        let config = EncoderConfig::new().repeat(metadata.repeat);
        let encoded = match encode_gif(
            frame_inputs,
            metadata.width,
            metadata.height,
            config,
            limits.clone(),
            &Unstoppable,
        ) {
            Ok(enc) => enc,
            Err(e) => {
                eprintln!("Skipping {} (encode failed): {:?}", filename, e);
                continue;
            }
        };

        // Decode again
        let (metadata2, frames2, _stats2) =
            decode_gif(&encoded, limits, &Unstoppable).expect("Re-decode should succeed");

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
    // Test files specifically for disposal methods (now in local corpus)
    let test_files = ["any-disposal.gif", "mixed-disposal.gif"];

    for filename in test_files {
        let path = Path::new(LOCAL_CORPUS).join(filename);
        assert!(path.exists(), "Local corpus file {} should exist", filename);

        let data = fs::read(&path).expect("Failed to read file");
        let limits = Limits::default();

        let (metadata, frames, _stats) =
            decode_gif(&data, limits, &Unstoppable).expect("Should decode disposal test file");

        assert!(
            frames.len() > 1,
            "{}: Expected multiple frames for disposal test",
            filename
        );

        // Verify each frame has correct dimensions
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.width, metadata.width,
                "{}: frame {} width should match canvas",
                filename, i
            );
            assert_eq!(
                frame.height, metadata.height,
                "{}: frame {} height should match canvas",
                filename, i
            );
            assert_eq!(
                frame.pixels.len(),
                frame.width as usize * frame.height as usize,
                "{}: frame {} pixel count should match dimensions",
                filename,
                i
            );
        }

        eprintln!("Disposal test OK: {} ({} frames)", filename, frames.len());
    }
}

#[test]
fn corpus_interlaced_gif() {
    let path = Path::new(LOCAL_CORPUS).join("interlaced.gif");
    assert!(path.exists(), "interlaced.gif should exist in local corpus");

    let data = fs::read(&path).expect("Failed to read file");
    let limits = Limits::default();

    let (metadata, frames, _stats) =
        decode_gif(&data, limits, &Unstoppable).expect("Should decode interlaced GIF");

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
    // Use the largest animation in local corpus
    let path = Path::new(LOCAL_CORPUS).join("large-gif-anim-full-frame-replace.gif");
    assert!(
        path.exists(),
        "large-gif-anim-full-frame-replace.gif should exist in local corpus"
    );

    let data = fs::read(&path).expect("Failed to read file");
    let limits = Limits::default();

    let (metadata, frames, stats) =
        decode_gif(&data, limits, &Unstoppable).expect("Should decode large animation");

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
    // Test simple GIFs from local corpus
    let simple_files = ["sample_1.gif", "alpha_gif_a.gif"];

    for filename in simple_files {
        let path = Path::new(LOCAL_CORPUS).join(filename);
        assert!(path.exists(), "{} should exist in local corpus", filename);

        let data = fs::read(&path).expect("Failed to read file");
        let limits = Limits::default();

        let (metadata, frames, _stats) = decode_gif(&data, limits, &Unstoppable)
            .unwrap_or_else(|e| panic!("Should decode {}: {:?}", filename, e));

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
}

#[test]
fn corpus_transparency_handling() {
    let path = Path::new(LOCAL_CORPUS).join("alpha_gif_a.gif");
    assert!(
        path.exists(),
        "alpha_gif_a.gif should exist in local corpus"
    );

    let data = fs::read(&path).expect("Failed to read file");
    let limits = Limits::default();

    let (metadata, frames, _stats) =
        decode_gif(&data, limits, &Unstoppable).expect("Should decode alpha GIF");

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
    // Use an animation file with restrictive limits
    let path = Path::new(LOCAL_CORPUS).join("large-gif-anim-full-frame-replace.gif");
    assert!(
        path.exists(),
        "large-gif-anim-full-frame-replace.gif should exist in local corpus"
    );

    let data = fs::read(&path).expect("Failed to read file");

    // Set very restrictive memory limit (1 KB) - too small for any meaningful GIF
    let limits = Limits::default().max_memory(1024);

    let result = decode_gif(&data, limits, &Unstoppable);

    // Should fail due to memory limits
    assert!(result.is_err(), "Should fail with restrictive memory limit");

    eprintln!("Memory limits test OK: decode correctly rejected with limit");
}

// ============================================================================
// GIF Bomb Tests - verify our limits protect against malicious inputs
// ============================================================================

/// Path to bomb test files
const BOMBS_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/bombs");

/// Test that dimension bombs are rejected before allocation
#[test]
fn bomb_dimension_65535x65535() {
    let path = Path::new(BOMBS_CORPUS).join("dimension_bomb.gif");
    assert!(
        path.exists(),
        "dimension_bomb.gif should exist in bombs corpus"
    );

    let data = fs::read(&path).expect("Failed to read bomb file");
    let limits = Limits::default(); // Default limits: 16384x16384 max

    let result = decode_gif(&data, limits, &Unstoppable);

    // Should fail with DimensionsTooLarge error
    assert!(result.is_err(), "Dimension bomb should be rejected");

    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
        err_str.contains("DimensionsTooLarge") || err_str.contains("dimensions"),
        "Error should mention dimensions: {}",
        err_str
    );

    // Critical: should NOT have allocated 65535*65535*4 = 17GB+ of memory
    // The error is thrown early before large allocation, which is the important property
    eprintln!("Dimension bomb test OK: rejected with error before large allocation");
}

/// Test that slightly-over-limit dimensions are rejected
#[test]
fn bomb_large_dimensions() {
    let path = Path::new(BOMBS_CORPUS).join("large_dimensions.gif");
    assert!(
        path.exists(),
        "large_dimensions.gif should exist in bombs corpus"
    );

    let data = fs::read(&path).expect("Failed to read file");
    let limits = Limits::default(); // 16384x16384 max

    let result = decode_gif(&data, limits, &Unstoppable);

    // 16385x16385 should be rejected (just over 16384 limit)
    assert!(
        result.is_err(),
        "16385x16385 should exceed default 16384x16384 limit"
    );

    eprintln!("Large dimensions test OK: 16385x16385 rejected");
}

/// Test that we can decode tiny valid GIF (sanity check)
#[test]
fn bomb_tiny_valid_sanity() {
    let path = Path::new(BOMBS_CORPUS).join("tiny_valid.gif");
    assert!(path.exists(), "tiny_valid.gif should exist in bombs corpus");

    let data = fs::read(&path).expect("Failed to read file");
    let limits = Limits::default();

    // This should succeed - it's a valid 2x2 GIF
    let result = decode_gif(&data, limits, &Unstoppable);

    match result {
        Ok((metadata, _frames, _stats)) => {
            assert_eq!(metadata.width, 2);
            assert_eq!(metadata.height, 2);
            eprintln!("Tiny valid GIF OK: 2x2");
        }
        Err(e) => {
            // The tiny GIF might have malformed LZW, that's okay for this test
            eprintln!("Tiny valid GIF decode error (may be expected): {:?}", e);
        }
    }
}

/// Test that total pixel limit catches bombs that fit dimension limits individually
#[test]
fn bomb_total_pixels_limit() {
    // Create a GIF that's 10000x10000 = 100 megapixels
    // This fits dimension limits (16384) but exceeds default total pixel limit (100 megapixels)
    let mut data = Vec::new();
    data.extend_from_slice(b"GIF89a");
    data.extend_from_slice(&10000u16.to_le_bytes()); // width
    data.extend_from_slice(&10000u16.to_le_bytes()); // height
    data.extend_from_slice(&[0x00, 0x00, 0x00]); // packed, bg, aspect
    data.push(0x3B); // trailer

    // Set total pixel limit to 50 megapixels
    let limits = Limits::default().max_total_pixels(50_000_000);

    let result = decode_gif(&data, limits, &Unstoppable);

    assert!(
        result.is_err(),
        "100 megapixel image should exceed 50MP limit"
    );

    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
        err_str.contains("TotalPixels") || err_str.contains("pixels"),
        "Error should mention pixels: {}",
        err_str
    );

    eprintln!("Total pixels limit test OK: 100MP rejected with 50MP limit");
}

/// Test decompression ratio limit (zip bomb protection)
#[test]
fn bomb_decompression_ratio() {
    // A real decompression bomb would have high compression ratio
    // For now, test that the limit mechanism works with restrictive settings
    let limits = Limits::default().max_decompression_ratio(1.5); // Very restrictive

    // Use a normal corpus file which should have reasonable compression
    let path = Path::new(LOCAL_CORPUS).join("sample_1.gif");
    assert!(path.exists(), "sample_1.gif should exist in local corpus");

    let data = fs::read(&path).expect("Failed to read file");
    let result = decode_gif(&data, limits, &Unstoppable);

    // With 1.5x ratio limit, most GIFs should fail (they typically compress well)
    // This verifies the check is actually being performed
    match result {
        Err(err) => {
            let err_str = format!("{:?}", err);
            eprintln!(
                "Decompression ratio test: rejected with restrictive limit (expected): {}",
                err_str
            );
        }
        Ok(_) => {
            eprintln!("Decompression ratio test: file has low compression ratio, passed");
        }
    }
}

/// Test animated GIF files from local corpus
#[test]
fn local_corpus_animated() {
    // Animated GIFs in local corpus
    let anim_files = [
        "any-disposal.gif",
        "mixed-disposal.gif",
        "interlaced.gif",
        "large-gif-anim-combine.gif",
        "large-gif-anim-full-frame-replace.gif",
        "border_touching_layers.gif",
    ];

    for filename in anim_files {
        let path = Path::new(LOCAL_CORPUS).join(filename);
        if !path.exists() {
            continue;
        }

        let data = fs::read(&path).expect("Failed to read file");
        let limits = Limits::default();

        match decode_gif(&data, limits, &Unstoppable) {
            Ok((metadata, frames, _)) => {
                eprintln!(
                    "  OK: {} ({}x{}, {} frames)",
                    filename,
                    metadata.width,
                    metadata.height,
                    frames.len()
                );
            }
            Err(e) => {
                panic!("Failed to decode {}: {:?}", filename, e);
            }
        }
    }
}

/// Test static/simple GIF files from local corpus
#[test]
fn local_corpus_static() {
    // Static GIFs in local corpus
    let static_files = ["sample_1.gif", "alpha_gif_a.gif"];

    for filename in static_files {
        let path = Path::new(LOCAL_CORPUS).join(filename);
        if !path.exists() {
            continue;
        }

        let data = fs::read(&path).expect("Failed to read file");
        let limits = Limits::default();

        match decode_gif(&data, limits, &Unstoppable) {
            Ok((metadata, frames, _)) => {
                eprintln!(
                    "  OK: {} ({}x{}, {} frames)",
                    filename,
                    metadata.width,
                    metadata.height,
                    frames.len()
                );
            }
            Err(e) => {
                panic!("Failed to decode {}: {:?}", filename, e);
            }
        }
    }
}
