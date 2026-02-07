#![cfg(any(
    feature = "imagequant",
    feature = "quantizr",
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
//! Integration tests comparing quantizer quality using SSIMULACRA2.
//!
//! These tests encode real PNG images to GIF with different quantizers,
//! then decode and measure the quality degradation using multiple metrics.
//!
//! SSIMULACRA2 scores (higher is better, max ~100):
//! - 90+: Excellent (nearly indistinguishable)
//! - 70-90: Good (minor differences)
//! - 50-70: Fair (noticeable but acceptable)
//! - <50: Poor (significant degradation)
//!
//! MSE (Mean Squared Error, lower is better, 0 = identical):
//! - <50: Excellent
//! - 50-200: Good
//! - 200-500: Fair
//! - >500: Poor

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, Instant};
use zengif::{Limits, QuantizerBackend, Rgba, Unstoppable};

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
fn calculate_ssim2(original: &[Rgba], quantized: &[Rgba], width: usize, height: usize) -> f64 {
    use imgref::ImgVec;

    // Convert RGBA to RGB arrays for fast-ssim2
    let orig_rgb: Vec<[u8; 3]> = original.iter().map(|p| [p.r, p.g, p.b]).collect();
    let quant_rgb: Vec<[u8; 3]> = quantized.iter().map(|p| [p.r, p.g, p.b]).collect();

    let orig_img = ImgVec::new(orig_rgb, width, height);
    let quant_img = ImgVec::new(quant_rgb, width, height);

    // Calculate SSIMULACRA2 score
    fast_ssim2::compute_ssimulacra2(orig_img.as_ref(), quant_img.as_ref()).unwrap_or(0.0)
}

/// Calculate MSE (Mean Squared Error) between original and quantized frames.
/// Returns a value where lower is better (0 = identical).
/// We use this as a simple secondary metric.
fn calculate_mse(original: &[Rgba], quantized: &[Rgba]) -> f64 {
    if original.len() != quantized.len() || original.is_empty() {
        return f64::MAX;
    }

    let sum: f64 = original
        .iter()
        .zip(quantized.iter())
        .map(|(o, q)| {
            let dr = (o.r as f64 - q.r as f64).powi(2);
            let dg = (o.g as f64 - q.g as f64).powi(2);
            let db = (o.b as f64 - q.b as f64).powi(2);
            dr + dg + db
        })
        .sum();

    sum / (original.len() as f64 * 3.0)
}

/// Test result for a single quantizer on a single image.
#[derive(Debug, Clone)]
struct QuantizerResult {
    backend: QuantizerBackend,
    available: bool,
    ssim2_score: Option<f64>,
    mse_score: Option<f64>,
    output_size: Option<usize>,
    encode_time: Option<Duration>,
}

/// Encode PNG pixels to GIF with a specific quantizer, decode, and measure quality.
#[allow(deprecated)] // Testing deprecated quantizer_backend API
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
            mse_score: None,
            output_size: None,
            encode_time: None,
        };
    }

    let limits = Limits::default();

    // Create encoder config with specific backend
    let config = zengif::EncoderConfig::new(width as u16, height as u16).quantizer_backend(backend);

    // Encode to GIF with timing
    let mut output = Vec::new();
    let start = Instant::now();

    let mut encoder = match zengif::Encoder::new(
        &mut output,
        width,
        height,
        config,
        limits.clone(),
        Unstoppable,
    ) {
        Ok(e) => e,
        Err(_) => {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                mse_score: None,
                output_size: None,
                encode_time: None,
            };
        }
    };

    let input = zengif::FrameInput::new(width as u16, height as u16, 100, pixels.to_vec());
    if encoder.add_frame(input).is_err() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            mse_score: None,
            output_size: None,
            encode_time: None,
        };
    }

    if encoder.finish().is_err() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            mse_score: None,
            output_size: None,
            encode_time: None,
        };
    }

    let encode_time = start.elapsed();
    let output_size = output.len();

    // Decode the GIF back
    let (_, decoded_frames, _stats) = match zengif::decode_gif(&output, limits, Unstoppable) {
        Ok(r) => r,
        Err(_) => {
            return QuantizerResult {
                backend,
                available: true,
                ssim2_score: None,
                mse_score: None,
                output_size: Some(output_size),
                encode_time: Some(encode_time),
            };
        }
    };

    if decoded_frames.is_empty() {
        return QuantizerResult {
            backend,
            available: true,
            ssim2_score: None,
            mse_score: None,
            output_size: Some(output_size),
            encode_time: Some(encode_time),
        };
    }

    // Calculate metrics between original and decoded
    let decoded = &decoded_frames[0];
    let ssim2 = calculate_ssim2(pixels, &decoded.pixels, width as usize, height as usize);
    let mse = calculate_mse(pixels, &decoded.pixels);

    QuantizerResult {
        backend,
        available: true,
        ssim2_score: Some(ssim2),
        mse_score: Some(mse),
        output_size: Some(output_size),
        encode_time: Some(encode_time),
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
    feature = "exoquant-deprecated",
    feature = "color_quant"
))]
mod quality_tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus_path() -> Option<PathBuf> {
        // Try HOME (Unix) then USERPROFILE (Windows)
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        let path = PathBuf::from(home).join("work/codec-corpus");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Aggregate results across multiple images
    #[derive(Default)]
    struct AggregateResults {
        ssim2_sum: [f64; 4],
        mse_sum: [f64; 4],
        size_sum: [usize; 4],
        time_sum: [Duration; 4],
        counts: [usize; 4],
    }

    impl AggregateResults {
        fn add(&mut self, results: &[QuantizerResult]) {
            for (i, r) in results.iter().enumerate() {
                if r.available {
                    if let Some(s) = r.ssim2_score {
                        self.ssim2_sum[i] += s;
                    }
                    if let Some(d) = r.mse_score {
                        self.mse_sum[i] += d;
                    }
                    if let Some(sz) = r.output_size {
                        self.size_sum[i] += sz;
                    }
                    if let Some(t) = r.encode_time {
                        self.time_sum[i] += t;
                    }
                    if r.ssim2_score.is_some() {
                        self.counts[i] += 1;
                    }
                }
            }
        }

        fn avg_ssim2(&self, i: usize) -> Option<f64> {
            if self.counts[i] > 0 {
                Some(self.ssim2_sum[i] / self.counts[i] as f64)
            } else {
                None
            }
        }

        fn avg_mse(&self, i: usize) -> Option<f64> {
            if self.counts[i] > 0 {
                Some(self.mse_sum[i] / self.counts[i] as f64)
            } else {
                None
            }
        }

        fn avg_size(&self, i: usize) -> Option<usize> {
            self.size_sum[i].checked_div(self.counts[i])
        }

        fn avg_time_ms(&self, i: usize) -> Option<f64> {
            if self.counts[i] > 0 {
                Some(self.time_sum[i].as_secs_f64() * 1000.0 / self.counts[i] as f64)
            } else {
                None
            }
        }
    }

    #[test]
    fn quantizer_benchmark_cid22() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        let cid_path = corpus.join("CID22/CID22-512/training");
        if !cid_path.exists() {
            println!("Skipping: CID22 corpus not found at {}", cid_path.display());
            return;
        }

        // Get first 10 images for benchmarking
        let images: Vec<_> = std::fs::read_dir(&cid_path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "png").unwrap_or(false))
            .take(10)
            .collect();

        if images.is_empty() {
            println!("Skipping: No PNG files found in CID22 corpus");
            return;
        }

        println!(
            "\n=== Quantizer Benchmark: CID22 Corpus ({} images) ===\n",
            images.len()
        );

        let mut agg = AggregateResults::default();

        // Header
        println!(
            "{:<40} {:>10} {:>10} {:>10} {:>10}",
            "Image", "imagequant", "exoquant", "quantizr", "color_quant"
        );
        println!("{}", "=".repeat(100));

        for path in &images {
            let filename = path.file_name().unwrap().to_string_lossy();
            let short_name: String = if filename.len() > 38 {
                format!("{}...", &filename[..35])
            } else {
                filename.to_string()
            };

            let results = test_quantizer_quality_on_png(path);
            if results.is_empty() {
                continue;
            }

            agg.add(&results);

            // Show SSIM2 for each image
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
                "{:<40} {:>10} {:>10} {:>10} {:>10}",
                short_name, &scores[0], &scores[1], &scores[2], &scores[3],
            );
        }

        // Summary
        println!("{}", "=".repeat(100));
        println!("\n=== SUMMARY ===\n");

        let backends = ["imagequant", "exoquant", "quantizr", "color_quant"];

        // SSIM2 (higher is better)
        println!("SSIMULACRA2 (higher is better, max 100):");
        for (i, name) in backends.iter().enumerate() {
            if let Some(avg) = agg.avg_ssim2(i) {
                println!("  {:<12}: {:.2}", name, avg);
            }
        }
        println!();

        // MSE (lower is better)
        println!("MSE (lower is better, 0 = identical):");
        for (i, name) in backends.iter().enumerate() {
            if let Some(avg) = agg.avg_mse(i) {
                println!("  {:<12}: {:.1}", name, avg);
            }
        }
        println!();

        // File size
        println!("Average file size:");
        for (i, name) in backends.iter().enumerate() {
            if let Some(avg) = agg.avg_size(i) {
                println!("  {:<12}: {:.1} KB", name, avg as f64 / 1024.0);
            }
        }
        println!();

        // Encode time
        println!("Average encode time:");
        for (i, name) in backends.iter().enumerate() {
            if let Some(avg) = agg.avg_time_ms(i) {
                println!("  {:<12}: {:.1} ms", name, avg);
            }
        }
        println!();

        // Throughput (pixels per second)
        println!("Throughput (512x512 images):");
        let pixels_per_image = 512 * 512;
        for (i, name) in backends.iter().enumerate() {
            if let Some(avg_ms) = agg.avg_time_ms(i) {
                let mpix_per_sec = (pixels_per_image as f64 / 1_000_000.0) / (avg_ms / 1000.0);
                println!("  {:<12}: {:.2} Mpix/s", name, mpix_per_sec);
            }
        }

        println!("\nNote: GIF is limited to 256 colors, so quality degradation is expected.");
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
                if let (Some(ssim2), Some(mse)) = (result.ssim2_score, result.mse_score) {
                    println!(
                        "{:?}: SSIM2={:.2}, MSE={:.1}, size={}KB, time={:.1}ms",
                        result.backend,
                        ssim2,
                        mse,
                        result.output_size.unwrap_or(0) / 1024,
                        result
                            .encode_time
                            .map(|t| t.as_secs_f64() * 1000.0)
                            .unwrap_or(0.0)
                    );
                    // Kodak images are complex - expect some degradation but still reasonable
                    assert!(
                        ssim2 > 40.0,
                        "{:?} produced poor quality: {:.2}",
                        result.backend,
                        ssim2
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

        let mut agg = AggregateResults::default();

        for image in test_images {
            let path = corpus.join("kodak").join(image);
            if !path.exists() {
                continue;
            }

            let results = test_quantizer_quality_on_png(&path);
            agg.add(&results);

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
                "{:<15} {:>12} {:>12} {:>12} {:>12}",
                image, &scores[0], &scores[1], &scores[2], &scores[3],
            );
        }

        // Print averages
        println!("{}", "-".repeat(67));
        println!(
            "{:<15} {:>12} {:>12} {:>12} {:>12}",
            "AVERAGE",
            agg.avg_ssim2(0)
                .map(|v| format!("{:.1}", v))
                .unwrap_or_default(),
            agg.avg_ssim2(1)
                .map(|v| format!("{:.1}", v))
                .unwrap_or_default(),
            agg.avg_ssim2(2)
                .map(|v| format!("{:.1}", v))
                .unwrap_or_default(),
            agg.avg_ssim2(3)
                .map(|v| format!("{:.1}", v))
                .unwrap_or_default(),
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
                if let (Some(ssim2), Some(mse)) = (result.ssim2_score, result.mse_score) {
                    println!(
                        "{:?}: SSIM2={:.2}, MSE={:.1}, size={}KB",
                        result.backend,
                        ssim2,
                        mse,
                        result.output_size.unwrap_or(0) / 1024
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
                image, &sizes[0], &sizes[1], &sizes[2], &sizes[3],
            );
        }
    }

    /// Test how dithering level affects file size for quantizr
    #[test]
    #[cfg(feature = "quantizr")]
    #[allow(deprecated)] // Testing deprecated quantizer_backend API
    fn quantizr_dithering_comparison() {
        let Some(corpus) = corpus_path() else {
            println!("Skipping: codec-corpus not found");
            return;
        };

        let path = corpus.join("kodak/1.png");
        if !path.exists() {
            println!("Skipping: {} not found", path.display());
            return;
        }

        let (pixels, width, height) = match decode_png(&path) {
            Some(data) => data,
            None => {
                println!("Skipping: failed to decode PNG");
                return;
            }
        };

        println!("\n=== Quantizr Dithering Level Comparison ===\n");
        println!(
            "{:<15} {:>12} {:>12} {:>12}",
            "Dithering", "SSIM2", "File Size", "Reduction"
        );
        println!("{}", "-".repeat(55));

        let dither_levels = [0.0, 0.25, 0.5, 0.75, 1.0];
        let mut baseline_size = 0usize;

        for dither in dither_levels {
            let limits = Limits::default();

            // Create encoder with specific dithering
            let config = zengif::EncoderConfig::new(width as u16, height as u16)
                .quantizer_backend(QuantizerBackend::Quantizr)
                .dithering(dither);

            let mut output = Vec::new();
            let mut encoder = zengif::Encoder::new(
                &mut output,
                width,
                height,
                config,
                limits.clone(),
                Unstoppable,
            )
            .expect("encoder creation failed");

            let input = zengif::FrameInput::new(width as u16, height as u16, 100, pixels.clone());
            encoder.add_frame(input).expect("add_frame failed");
            encoder.finish().expect("finish failed");

            let output_size = output.len();
            if dither == 1.0 {
                baseline_size = output_size;
            }

            // Decode and measure quality
            let (_, decoded_frames, _stats) =
                zengif::decode_gif(&output, limits, Unstoppable).expect("decode failed");

            let ssim2 = if !decoded_frames.is_empty() {
                calculate_ssim2(
                    &pixels,
                    &decoded_frames[0].pixels,
                    width as usize,
                    height as usize,
                )
            } else {
                0.0
            };

            let reduction = if baseline_size > 0 {
                format!(
                    "{:+.1}%",
                    (output_size as f64 / baseline_size as f64 - 1.0) * 100.0
                )
            } else {
                "baseline".to_string()
            };

            println!(
                "{:<15} {:>12.2} {:>10.1}KB {:>12}",
                format!("{:.2}", dither),
                ssim2,
                output_size as f64 / 1024.0,
                reduction
            );
        }

        println!("\nNote: Lower dithering = smaller files, but may show banding.");
        println!("      Higher dithering = larger files, smoother gradients.");
    }
}
