//! Integration tests comparing quantizer quality using SSIMULACRA2.
//!
//! These tests encode real PNG images to GIF with different quantizers,
//! then decode and measure the quality degradation using SSIMULACRA2.
//!
//! SSIMULACRA2 scores (higher is better, max ~100):
//! - 90+: Excellent (nearly indistinguishable)
//! - 70-90: Good (minor differences)
//! - 50-70: Fair (noticeable but acceptable)
//! - <50: Poor (significant degradation)

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use zengif::{Limits, QuantizerBackend, Rgba, Stats, Unstoppable};

/// Decode a PNG file to RGBA pixels.
fn decode_png(path: &Path) -> Option<(Vec<Rgba>, u32, u32)> {
    let file = File::open(path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;

    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;

    let width = info.width;
    let height = info.height;

    // Convert to RGBA
    let pixels: Vec<Rgba> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()]
            .chunks_exact(4)
            .map(|c| Rgba::new(c[0], c[1], c[2], c[3]))
            .collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks_exact(3)
            .map(|c| Rgba::rgb(c[0], c[1], c[2]))
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .chunks_exact(2)
            .map(|c| Rgba::new(c[0], c[0], c[0], c[1]))
            .collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .map(|&g| Rgba::rgb(g, g, g))
            .collect(),
        png::ColorType::Indexed => {
            // For indexed, we'd need to look up the palette - skip for now
            return None;
        }
    };

    Some((pixels, width, height))
}

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

/// Encode PNG pixels to GIF with a specific quantizer, decode, and measure quality.
fn test_quantizer_on_png(
    pixels: &[Rgba],
    width: u32,
    height: u32,
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

    let _stats = Stats::new();
    let limits = Limits::default();

    // Create encoder config with specific backend
    let config = zengif::EncoderConfig::new(width as u16, height as u16)
        .quantizer_backend(backend);

    // Encode to GIF
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

    let input = zengif::FrameInput::new(width as u16, height as u16, 100, pixels.to_vec());
    if encoder.add_frame(input).is_err() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            output_size: None,
        };
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

    // Decode the GIF back
    let stats2 = Stats::new();
    let (_, decoded_frames) = match zengif::decode_gif(&output, limits, &stats2, Unstoppable) {
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

    if decoded_frames.is_empty() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            output_size: Some(output_size),
        };
    }

    // Calculate SSIM2 between original and decoded
    let decoded = &decoded_frames[0];
    let score = calculate_ssim2(
        pixels,
        &decoded.pixels,
        width as usize,
        height as usize,
    );

    QuantizerResult {
        backend,
        available: true,
        ssim2_score: Some(score),
        output_size: Some(output_size),
    }
}

/// Test quantizer quality on a PNG file.
fn test_quantizer_quality_on_png(path: &Path) -> Vec<QuantizerResult> {
    let (pixels, width, height) = match decode_png(path) {
        Some(data) => data,
        None => return vec![],
    };

    let backends = [
        QuantizerBackend::Imagequant,
        QuantizerBackend::Exoquant,
        QuantizerBackend::Quantizr,
        QuantizerBackend::ColorQuant,
    ];

    backends
        .into_iter()
        .map(|backend| test_quantizer_on_png(&pixels, width, height, backend))
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
    use std::path::PathBuf;

    fn corpus_path() -> Option<PathBuf> {
        let path = PathBuf::from(env!("HOME")).join("work/codec-corpus");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    #[test]
    fn quantizer_quality_kodak_01() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        let path = corpus.join("kodak/1.png");
        if !path.exists() {
            println!("Skipping: {} not found", path.display());
            return;
        }

        let results = test_quantizer_quality_on_png(&path);

        println!("\n=== Quantizer Quality: kodak/1.png ===");
        for result in &results {
            if result.available {
                if let Some(score) = result.ssim2_score {
                    println!(
                        "{:?}: SSIM2={:.2}, size={} bytes",
                        result.backend,
                        score,
                        result.output_size.unwrap_or(0)
                    );
                    // Kodak images are complex - expect some degradation but still reasonable
                    assert!(
                        score > 40.0,
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
    fn quantizer_quality_kodak_comparison() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        // Test on several Kodak images
        let test_images = ["1.png", "8.png", "19.png", "24.png"];

        println!("\n=== Quantizer Quality Comparison (Kodak Images) ===\n");
        println!(
            "{:<15} {:>12} {:>12} {:>12} {:>12}",
            "Image", "imagequant", "exoquant", "quantizr", "color_quant"
        );
        println!("{}", "-".repeat(67));

        let mut totals = [0.0f64; 4];
        let mut counts = [0usize; 4];

        for image in test_images {
            let path = corpus.join("kodak").join(image);
            if !path.exists() {
                continue;
            }

            let results = test_quantizer_quality_on_png(&path);

            let scores: Vec<String> = results
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    if r.available {
                        r.ssim2_score
                            .map(|s| {
                                totals[i] += s;
                                counts[i] += 1;
                                format!("{:.1}", s)
                            })
                            .unwrap_or_else(|| "err".to_string())
                    } else {
                        "n/a".to_string()
                    }
                })
                .collect();

            println!(
                "{:<15} {:>12} {:>12} {:>12} {:>12}",
                image,
                scores.first().unwrap_or(&String::new()),
                scores.get(1).unwrap_or(&String::new()),
                scores.get(2).unwrap_or(&String::new()),
                scores.get(3).unwrap_or(&String::new()),
            );
        }

        // Print averages
        println!("{}", "-".repeat(67));
        let avgs: Vec<String> = totals
            .iter()
            .zip(counts.iter())
            .map(|(&t, &c)| {
                if c > 0 {
                    format!("{:.1}", t / c as f64)
                } else {
                    "n/a".to_string()
                }
            })
            .collect();

        println!(
            "{:<15} {:>12} {:>12} {:>12} {:>12}",
            "AVERAGE", avgs[0], avgs[1], avgs[2], avgs[3]
        );

        println!("\nNote: Higher SSIM2 scores are better (max ~100)");
        println!("GIF is limited to 256 colors, so some degradation is expected.");
    }

    #[test]
    fn quantizer_quality_gradients() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        // Gradients are particularly challenging for quantization
        let path = corpus.join("imageflow/test_inputs/gradients.png");
        if !path.exists() {
            println!("Skipping: {} not found", path.display());
            return;
        }

        let results = test_quantizer_quality_on_png(&path);

        println!("\n=== Quantizer Quality: gradients.png (challenging) ===");
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
    fn quantizer_output_size_comparison() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        let test_images = ["1.png", "8.png"];

        println!("\n=== Quantizer Output Size Comparison ===\n");
        println!(
            "{:<15} {:>12} {:>12} {:>12} {:>12}",
            "Image", "imagequant", "exoquant", "quantizr", "color_quant"
        );
        println!("{}", "-".repeat(67));

        for image in test_images {
            let path = corpus.join("kodak").join(image);
            if !path.exists() {
                continue;
            }

            let results = test_quantizer_quality_on_png(&path);

            let sizes: Vec<String> = results
                .iter()
                .map(|r| {
                    if r.available {
                        r.output_size
                            .map(|s| format!("{:.1}KB", s as f64 / 1024.0))
                            .unwrap_or_else(|| "err".to_string())
                    } else {
                        "n/a".to_string()
                    }
                })
                .collect();

            println!(
                "{:<15} {:>12} {:>12} {:>12} {:>12}",
                image,
                sizes.first().unwrap_or(&String::new()),
                sizes.get(1).unwrap_or(&String::new()),
                sizes.get(2).unwrap_or(&String::new()),
                sizes.get(3).unwrap_or(&String::new()),
            );
        }
    }
}
