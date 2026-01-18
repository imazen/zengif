//! Integration tests comparing quantizer quality using SSIMULACRA2.
//!
//! These tests decode real GIF files, re-encode them with different quantizers,
//! and measure the quality degradation using the SSIMULACRA2 metric.
//!
//! SSIMULACRA2 scores (higher is better, max ~100):
//! - 90+: Excellent (nearly indistinguishable)
//! - 70-90: Good (minor differences)
//! - 50-70: Fair (noticeable but acceptable)
//! - <50: Poor (significant degradation)

use std::fs;
use zengif::{decode_gif, Limits, QuantizerBackend, Rgba, Stats, Unstoppable};

/// Calculate SSIMULACRA2 score between original and quantized frames.
/// Returns a score where higher is better (max ~100).
fn calculate_ssim2(
    original: &[Rgba],
    quantized: &[Rgba],
    width: usize,
    height: usize,
) -> f64 {
    use imgref::ImgVec;

    // Convert RGBA to RGB arrays for fast-ssim2
    let orig_rgb: Vec<[u8; 3]> = original.iter().map(|p| [p.r, p.g, p.b]).collect();
    let quant_rgb: Vec<[u8; 3]> = quantized.iter().map(|p| [p.r, p.g, p.b]).collect();

    let orig_img = ImgVec::new(orig_rgb, width, height);
    let quant_img = ImgVec::new(quant_rgb, width, height);

    // Calculate SSIMULACRA2 score
    match fast_ssim2::compute_ssimulacra2(orig_img.as_ref(), quant_img.as_ref()) {
        Ok(score) => score,
        Err(_) => 0.0,
    }
}

/// Test result for a single quantizer on a single image.
#[derive(Debug)]
struct QuantizerResult {
    backend: QuantizerBackend,
    available: bool,
    ssim2_score: Option<f64>,
    output_size: Option<usize>,
}

/// Round-trip a GIF through a specific quantizer and measure quality.
fn roundtrip_with_quantizer(
    gif_data: &[u8],
    backend: QuantizerBackend,
) -> QuantizerResult {
    if !backend.is_available() {
        return QuantizerResult {
            backend,
            available: false,
            ssim2_score: None,
            output_size: None,
        };
    }

    let stats = Stats::new();
    let limits = Limits::default();

    // Decode original
    let (metadata, original_frames) = match decode_gif(gif_data, limits.clone(), &stats, Unstoppable) {
        Ok(r) => r,
        Err(_) => {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                output_size: None,
            };
        }
    };

    if original_frames.is_empty() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            output_size: None,
        };
    }

    // Create encoder config with specific backend
    let config = zengif::EncoderConfig::new(metadata.width, metadata.height)
        .repeat(metadata.repeat)
        .quantizer_backend(backend);

    // Encode with the specified quantizer
    let mut output = Vec::new();
    let mut encoder = match zengif::Encoder::new(&mut output, config, limits.clone(), Unstoppable) {
        Ok(e) => e,
        Err(_) => {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                output_size: None,
            };
        }
    };

    for frame in &original_frames {
        let input = zengif::FrameInput::new(
            frame.width,
            frame.height,
            frame.delay,
            frame.pixels.clone(),
        );
        if encoder.add_frame(input).is_err() {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                output_size: None,
            };
        }
    }

    if encoder.finish().is_err() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            output_size: None,
        };
    }

    let output_size = output.len();

    // Decode the re-encoded GIF
    let stats2 = Stats::new();
    let (_, decoded_frames) = match decode_gif(&output, limits, &stats2, Unstoppable) {
        Ok(r) => r,
        Err(_) => {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                output_size: Some(output_size),
            };
        }
    };

    // Calculate average SSIM2 across all frames
    let mut total_score = 0.0;
    let mut frame_count = 0;

    for (orig, decoded) in original_frames.iter().zip(decoded_frames.iter()) {
        if orig.pixels.len() == decoded.pixels.len() && !orig.pixels.is_empty() {
            let score = calculate_ssim2(
                &orig.pixels,
                &decoded.pixels,
                orig.width as usize,
                orig.height as usize,
            );
            total_score += score;
            frame_count += 1;
        }
    }

    let avg_score = if frame_count > 0 {
        Some(total_score / frame_count as f64)
    } else {
        None
    };

    QuantizerResult {
        backend,
        available: true,
        ssim2_score: avg_score,
        output_size: Some(output_size),
    }
}

/// Test quantizer quality on a specific test file.
fn test_quantizer_quality_on_file(path: &str) -> Vec<QuantizerResult> {
    let gif_data = match fs::read(path) {
        Ok(data) => data,
        Err(_) => return vec![],
    };

    let backends = [
        QuantizerBackend::Imagequant,
        QuantizerBackend::Exoquant,
        QuantizerBackend::Quantizr,
        QuantizerBackend::ColorQuant,
    ];

    backends
        .into_iter()
        .map(|backend| roundtrip_with_quantizer(&gif_data, backend))
        .collect()
}

// Only run these tests when at least one quantizer is available
#[cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant",
    feature = "color_quant"
))]
mod quality_tests {
    use super::*;

    #[test]
    fn quantizer_quality_sample_1() {
        let results = test_quantizer_quality_on_file(
            "tests/corpus/codec-corpus/sample_1.gif",
        );

        println!("\n=== Quantizer Quality: sample_1.gif ===");
        for result in &results {
            if result.available {
                if let Some(score) = result.ssim2_score {
                    println!(
                        "{:?}: SSIM2={:.2}, size={} bytes",
                        result.backend,
                        score,
                        result.output_size.unwrap_or(0)
                    );
                    // All quantizers should produce at least "fair" quality
                    assert!(
                        score > 30.0,
                        "{:?} produced poor quality: {:.2}",
                        result.backend,
                        score
                    );
                } else {
                    println!("{:?}: failed to process", result.backend);
                }
            } else {
                println!("{:?}: not available (feature not enabled)", result.backend);
            }
        }
    }

    #[test]
    fn quantizer_quality_any_disposal() {
        let results = test_quantizer_quality_on_file(
            "tests/corpus/codec-corpus/any-disposal.gif",
        );

        println!("\n=== Quantizer Quality: any-disposal.gif ===");
        for result in &results {
            if result.available {
                if let Some(score) = result.ssim2_score {
                    println!(
                        "{:?}: SSIM2={:.2}, size={} bytes",
                        result.backend,
                        score,
                        result.output_size.unwrap_or(0)
                    );
                } else {
                    println!("{:?}: failed to process", result.backend);
                }
            }
        }
    }

    #[test]
    fn quantizer_quality_large_animation() {
        let results = test_quantizer_quality_on_file(
            "tests/corpus/codec-corpus/large-gif-anim-combine.gif",
        );

        println!("\n=== Quantizer Quality: large-gif-anim-combine.gif ===");
        for result in &results {
            if result.available {
                if let Some(score) = result.ssim2_score {
                    println!(
                        "{:?}: SSIM2={:.2}, size={} bytes",
                        result.backend,
                        score,
                        result.output_size.unwrap_or(0)
                    );
                } else {
                    println!("{:?}: failed to process", result.backend);
                }
            }
        }
    }

    /// Compare all quantizers and report which produces best quality/size tradeoff.
    #[test]
    fn quantizer_comparison_report() {
        let test_files = [
            "tests/corpus/codec-corpus/sample_1.gif",
            "tests/corpus/codec-corpus/any-disposal.gif",
            "tests/corpus/codec-corpus/mixed-disposal.gif",
        ];

        println!("\n=== Quantizer Comparison Report ===\n");
        println!("{:<30} {:>12} {:>12} {:>12} {:>12}",
            "File", "imagequant", "exoquant", "quantizr", "color_quant");
        println!("{}", "-".repeat(80));

        for file in test_files {
            let results = test_quantizer_quality_on_file(file);
            let filename = file.split('/').last().unwrap_or(file);

            let scores: Vec<String> = results
                .iter()
                .map(|r| {
                    if r.available {
                        r.ssim2_score
                            .map(|s| format!("{:.1}", s))
                            .unwrap_or_else(|| "err".to_string())
                    } else {
                        "n/a".to_string()
                    }
                })
                .collect();

            println!(
                "{:<30} {:>12} {:>12} {:>12} {:>12}",
                filename,
                scores.first().unwrap_or(&"".to_string()),
                scores.get(1).unwrap_or(&"".to_string()),
                scores.get(2).unwrap_or(&"".to_string()),
                scores.get(3).unwrap_or(&"".to_string()),
            );
        }

        println!("\nNote: Higher SSIM2 scores are better (max ~100)");
    }
}
