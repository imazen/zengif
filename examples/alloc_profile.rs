//! Memory and time profiler for zengif encode/decode operations.
//!
//! This example measures resource usage across different:
//! - Image sizes (128x128, 256x256, 512x512, 1024x1024)
//! - Content types (solid, gradient, photo-like, noise)
//! - Quantizers (imagequant, quantizr, color_quant, none)
//!
//! Run with:
//! ```bash
//! cargo run --release --all-features --example alloc_profile
//! ```
//!
//! For detailed heap allocation tracking, use heaptrack:
//! ```bash
//! heaptrack cargo run --release --all-features --example alloc_profile
//! heaptrack_print heaptrack.alloc_profile.*.zst
//! ```

use std::io::{self, Write};
use std::time::Instant;

use enough::Unstoppable;
#[cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant",))]
use zengif::EncoderConfig;
#[cfg(any(feature = "imagequant", feature = "quantizr"))]
use zengif::heuristics::{QuantizerType, estimate_encode};
use zengif::{Decoder, FrameInput, Limits, Rgba, heuristics::estimate_decode};

#[cfg(feature = "color_quant")]
use zengif::ColorQuantQuantizer;
#[cfg(feature = "imagequant")]
use zengif::ImagequantQuantizer;
#[cfg(feature = "quantizr")]
use zengif::QuantizrQuantizer;

/// Test image sizes
const SIZES: &[(u32, u32, &str)] = &[
    (128, 128, "128x128"),
    (256, 256, "256x256"),
    (512, 512, "512x512"),
    (1024, 1024, "1024x1024"),
];

/// Frame counts for animation tests
const FRAME_COUNTS: &[u32] = &[1, 5, 10];

// =============================================================================
// Image generators for different content types
// =============================================================================

/// Generate solid color image (best case for compression)
fn generate_solid(width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
    // Vary color slightly per frame to avoid identical frames
    let r = ((frame_index * 50) % 256) as u8;
    let g = 100u8;
    let b = 150u8;
    vec![Rgba::rgb(r, g, b); (width * height) as usize]
}

/// Generate gradient image (typical baseline for benchmarks)
fn generate_gradient(width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let offset = (frame_index * 20) % 256;
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255 / width.max(1) + offset) % 256) as u8;
            let g = ((y * 255 / height.max(1) + offset / 2) % 256) as u8;
            let b = (((x + y) * 127 / (width + height).max(1) + offset / 3) % 256) as u8;
            pixels.push(Rgba::rgb(r, g, b));
        }
    }
    pixels
}

/// Generate photo-like image (typical web content - many colors, structure)
fn generate_photo_like(width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
    use std::f32::consts::{E, PI};

    let mut pixels = Vec::with_capacity((width * height) as usize);
    let offset = frame_index as f32 * 0.1;

    for y in 0..height {
        for x in 0..width {
            // Simulate photo content with smooth areas + detail
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;

            // Base color from position
            let r = ((fx * 200.0 + (fy * PI + offset).sin() * 30.0).clamp(0.0, 255.0)) as u8;
            let g = ((fy * 180.0 + (fx * E + offset).cos() * 40.0).clamp(0.0, 255.0)) as u8;
            let b = (((fx + fy) * 100.0 + ((fx * fy * 10.0) + offset).sin() * 50.0)
                .clamp(0.0, 255.0)) as u8;

            pixels.push(Rgba::rgb(r, g, b));
        }
    }
    pixels
}

/// Generate noise image (worst case - maximum entropy)
fn generate_noise(width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    // Simple LCG for deterministic "random" numbers
    let mut seed = 12345u64 + frame_index as u64 * 1000000;

    for _ in 0..(width * height) {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let r = (seed >> 16) as u8;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let g = (seed >> 16) as u8;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = (seed >> 16) as u8;
        pixels.push(Rgba::rgb(r, g, b));
    }
    pixels
}

/// Content type for profiling
#[derive(Debug, Clone, Copy)]
enum ContentType {
    Solid,
    Gradient,
    PhotoLike,
    Noise,
}

impl ContentType {
    fn name(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::PhotoLike => "photo",
            Self::Noise => "noise",
        }
    }

    fn generate(self, width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
        match self {
            Self::Solid => generate_solid(width, height, frame_index),
            Self::Gradient => generate_gradient(width, height, frame_index),
            Self::PhotoLike => generate_photo_like(width, height, frame_index),
            Self::Noise => generate_noise(width, height, frame_index),
        }
    }
}

const CONTENT_TYPES: &[ContentType] = &[
    ContentType::Solid,
    ContentType::Gradient,
    ContentType::PhotoLike,
    ContentType::Noise,
];

// =============================================================================
// Profiling infrastructure
// =============================================================================

#[derive(Debug)]
struct ProfileResult {
    operation: String,
    content: &'static str,
    size: &'static str,
    frames: u32,
    quantizer: &'static str,
    time_us: u64,
    peak_memory: usize,
    alloc_count: usize,
    output_bytes: usize,
    pixels: u64,
}

impl ProfileResult {
    fn throughput_mpixels(&self) -> f64 {
        if self.time_us == 0 {
            return 0.0;
        }
        (self.pixels as f64) / (self.time_us as f64 / 1_000_000.0) / 1_000_000.0
    }

    fn bytes_per_pixel(&self) -> f64 {
        if self.pixels == 0 {
            return 0.0;
        }
        self.peak_memory as f64 / self.pixels as f64
    }
}

fn print_header() {
    println!(
        "{:<10} {:<8} {:<10} {:>6} {:<12} {:>10} {:>12} {:>8} {:>10} {:>10}",
        "Operation",
        "Content",
        "Size",
        "Frames",
        "Quantizer",
        "Time (µs)",
        "Peak (bytes)",
        "Allocs",
        "Output",
        "Mpix/s"
    );
    println!("{}", "-".repeat(105));
}

fn print_result(r: &ProfileResult) {
    println!(
        "{:<10} {:<8} {:<10} {:>6} {:<12} {:>10} {:>12} {:>8} {:>10} {:>10.1}",
        r.operation,
        r.content,
        r.size,
        r.frames,
        r.quantizer,
        r.time_us,
        r.peak_memory,
        r.alloc_count,
        r.output_bytes,
        r.throughput_mpixels()
    );
}

// =============================================================================
// Decode profiling
// =============================================================================

fn profile_decode(
    content: ContentType,
    width: u32,
    height: u32,
    frame_count: u32,
) -> ProfileResult {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    // First, encode a GIF to decode
    #[allow(unused_variables)]
    let frames: Vec<FrameInput> = (0..frame_count)
        .map(|i| {
            let px = content.generate(width, height, i);
            FrameInput::new(width as u16, height as u16, 10, px)
        })
        .collect();

    #[cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant",))]
    let gif_data = {
        let config = EncoderConfig::new();
        zengif::encode_gif(
            frames,
            width.try_into().unwrap(),
            height.try_into().unwrap(),
            config,
            Limits::default(),
            &Unstoppable,
        )
        .unwrap()
    };

    #[cfg(not(any(feature = "imagequant", feature = "quantizr", feature = "color_quant",)))]
    let gif_data = Vec::new();

    if gif_data.is_empty() {
        return ProfileResult {
            operation: "decode".into(),
            content: content.name(),
            size: size_name(width),
            frames: frame_count,
            quantizer: "n/a",
            time_us: 0,
            peak_memory: 0,
            alloc_count: 0,
            output_bytes: 0,
            pixels,
        };
    }

    // Profile decode
    let start = Instant::now();

    let cursor = std::io::Cursor::new(&gif_data);
    let mut decoder = Decoder::new(cursor, Limits::default(), &Unstoppable).unwrap();
    let decoded_frames = decoder.decode_all().unwrap();

    let elapsed = start.elapsed();
    let output_bytes: usize = decoded_frames.iter().map(|f| f.pixels.len() * 4).sum();

    ProfileResult {
        operation: "decode".into(),
        content: content.name(),
        size: size_name(width),
        frames: frame_count,
        quantizer: "n/a",
        time_us: elapsed.as_micros() as u64,
        peak_memory: decoder.stats().peak(),
        alloc_count: decoder.stats().alloc_count(),
        output_bytes,
        pixels,
    }
}

// =============================================================================
// Encode profiling
// =============================================================================

#[cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant",))]
fn profile_encode_with_quantizer<Q: zengif::QuantizerTrait>(
    content: ContentType,
    width: u32,
    height: u32,
    frame_count: u32,
    quantizer_name: &'static str,
    quantizer: Q,
) -> ProfileResult {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    let frames: Vec<FrameInput> = (0..frame_count)
        .map(|i| {
            let px = content.generate(width, height, i);
            FrameInput::new(width as u16, height as u16, 10, px)
        })
        .collect();

    let config = EncoderConfig::new().dithering(0.5);

    // Profile encode
    let start = Instant::now();

    let output = zengif::encode_gif_with_quantizer(
        frames,
        width.try_into().unwrap(),
        height.try_into().unwrap(),
        config,
        Limits::default(),
        &Unstoppable,
        quantizer,
    )
    .unwrap();

    let elapsed = start.elapsed();

    ProfileResult {
        operation: "encode".into(),
        content: content.name(),
        size: size_name(width),
        frames: frame_count,
        quantizer: quantizer_name,
        time_us: elapsed.as_micros() as u64,
        peak_memory: 0, // Encoder doesn't track internal stats
        alloc_count: 0,
        output_bytes: output.len(),
        pixels,
    }
}

fn size_name(width: u32) -> &'static str {
    match width {
        128 => "128x128",
        256 => "256x256",
        512 => "512x512",
        1024 => "1024x1024",
        _ => "unknown",
    }
}

// =============================================================================
// Main profiling sweep
// =============================================================================

fn main() {
    println!(
        "╔══════════════════════════════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║                                    ZENGIF ALLOCATION PROFILER                                            ║"
    );
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!();
    println!("For detailed heap profiling, run with heaptrack:");
    println!("  heaptrack cargo run --release --all-features --example alloc_profile");
    println!();

    let mut results: Vec<ProfileResult> = Vec::new();

    // ==========================================================================
    // DECODE PROFILING
    // ==========================================================================
    println!("\n{}", "=".repeat(105));
    println!("DECODE PROFILING");
    println!("{}", "=".repeat(105));
    print_header();

    for &(width, height, _) in SIZES {
        for &content in CONTENT_TYPES {
            for &frames in FRAME_COUNTS {
                let result = profile_decode(content, width, height, frames);
                print_result(&result);
                results.push(result);
            }
        }
    }

    // ==========================================================================
    // ENCODE PROFILING (per quantizer)
    // ==========================================================================

    #[cfg(feature = "imagequant")]
    {
        println!("\n{}", "=".repeat(105));
        println!("ENCODE PROFILING - imagequant");
        println!("{}", "=".repeat(105));
        print_header();

        for &(width, height, _) in SIZES {
            for &content in CONTENT_TYPES {
                for &frames in FRAME_COUNTS {
                    let quantizer = ImagequantQuantizer::new();
                    let result = profile_encode_with_quantizer(
                        content,
                        width,
                        height,
                        frames,
                        "imagequant",
                        quantizer,
                    );
                    print_result(&result);
                    results.push(result);
                }
            }
        }
    }

    #[cfg(feature = "quantizr")]
    {
        println!("\n{}", "=".repeat(105));
        println!("ENCODE PROFILING - quantizr");
        println!("{}", "=".repeat(105));
        print_header();

        for &(width, height, _) in SIZES {
            for &content in CONTENT_TYPES {
                for &frames in FRAME_COUNTS {
                    let quantizer = QuantizrQuantizer::new();
                    let result = profile_encode_with_quantizer(
                        content, width, height, frames, "quantizr", quantizer,
                    );
                    print_result(&result);
                    results.push(result);
                }
            }
        }
    }

    #[cfg(feature = "color_quant")]
    {
        println!("\n{}", "=".repeat(105));
        println!("ENCODE PROFILING - color_quant");
        println!("{}", "=".repeat(105));
        print_header();

        for &(width, height, _) in SIZES {
            for &content in CONTENT_TYPES {
                for &frames in FRAME_COUNTS {
                    let quantizer = ColorQuantQuantizer::new();
                    let result = profile_encode_with_quantizer(
                        content,
                        width,
                        height,
                        frames,
                        "color_quant",
                        quantizer,
                    );
                    print_result(&result);
                    results.push(result);
                }
            }
        }
    }

    // ==========================================================================
    // HEURISTICS COMPARISON
    // ==========================================================================
    println!("\n{}", "=".repeat(105));
    println!("HEURISTICS vs MEASURED COMPARISON (512x512, 5 frames)");
    println!("{}", "=".repeat(105));

    let test_size = (512, 512);
    let test_frames = 5u32;

    // Find measured results for comparison
    let measured_decode: Vec<_> = results
        .iter()
        .filter(|r| r.operation == "decode" && r.size == "512x512" && r.frames == test_frames)
        .collect();

    if !measured_decode.is_empty() {
        let dec_est = estimate_decode(test_size.0, test_size.1, test_frames);

        println!("\nDecode Estimates vs Measured:");
        println!(
            "  Estimated peak memory (typ): {} bytes",
            dec_est.peak_memory_bytes
        );
        println!("  Estimated time (typ): {:.1} ms", dec_est.time_ms);
        println!();
        println!("  Measured by content type:");
        for r in &measured_decode {
            let mem_ratio = r.peak_memory as f64 / dec_est.peak_memory_bytes as f64;
            let time_ratio = (r.time_us as f64 / 1000.0) / dec_est.time_ms as f64;
            println!(
                "    {:<8}: peak={:>8} ({:.2}x est), time={:>6}µs ({:.2}x est)",
                r.content, r.peak_memory, mem_ratio, r.time_us, time_ratio
            );
        }
    }

    #[cfg(feature = "imagequant")]
    {
        let measured_encode: Vec<_> = results
            .iter()
            .filter(|r| {
                r.operation == "encode"
                    && r.size == "512x512"
                    && r.frames == test_frames
                    && r.quantizer == "imagequant"
            })
            .collect();

        if !measured_encode.is_empty() {
            let enc_est = estimate_encode(
                test_size.0,
                test_size.1,
                test_frames,
                QuantizerType::Imagequant,
            );

            println!("\nEncode (imagequant) Estimates vs Measured:");
            println!(
                "  Estimated peak memory (typ): {} bytes",
                enc_est.peak_memory_bytes
            );
            println!("  Estimated time (typ): {:.1} ms", enc_est.time_ms);
            println!();
            println!("  Measured by content type:");
            for r in &measured_encode {
                let mem_ratio = r.peak_memory as f64 / enc_est.peak_memory_bytes.max(1) as f64;
                let time_ratio = (r.time_us as f64 / 1000.0) / enc_est.time_ms.max(0.001) as f64;
                println!(
                    "    {:<8}: peak={:>8} ({:.2}x est), time={:>8}µs ({:.2}x est), out={:>6}",
                    r.content, r.peak_memory, mem_ratio, r.time_us, time_ratio, r.output_bytes
                );
            }
        }
    }

    #[cfg(feature = "quantizr")]
    {
        let measured_encode: Vec<_> = results
            .iter()
            .filter(|r| {
                r.operation == "encode"
                    && r.size == "512x512"
                    && r.frames == test_frames
                    && r.quantizer == "quantizr"
            })
            .collect();

        if !measured_encode.is_empty() {
            let enc_est = estimate_encode(
                test_size.0,
                test_size.1,
                test_frames,
                QuantizerType::Quantizr,
            );

            println!("\nEncode (quantizr) Estimates vs Measured:");
            println!(
                "  Estimated peak memory (typ): {} bytes",
                enc_est.peak_memory_bytes
            );
            println!("  Estimated time (typ): {:.1} ms", enc_est.time_ms);
            println!();
            println!("  Measured by content type:");
            for r in &measured_encode {
                let mem_ratio = r.peak_memory as f64 / enc_est.peak_memory_bytes.max(1) as f64;
                let time_ratio = (r.time_us as f64 / 1000.0) / enc_est.time_ms.max(0.001) as f64;
                println!(
                    "    {:<8}: peak={:>8} ({:.2}x est), time={:>8}µs ({:.2}x est), out={:>6}",
                    r.content, r.peak_memory, mem_ratio, r.time_us, time_ratio, r.output_bytes
                );
            }
        }
    }

    // ==========================================================================
    // SUMMARY STATISTICS
    // ==========================================================================
    println!("\n{}", "=".repeat(105));
    println!("SUMMARY STATISTICS");
    println!("{}", "=".repeat(105));

    // Group by quantizer and compute averages
    let quantizers: Vec<&str> = results
        .iter()
        .filter(|r| r.operation == "encode")
        .map(|r| r.quantizer)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for quant in quantizers {
        let quant_results: Vec<_> = results
            .iter()
            .filter(|r| r.operation == "encode" && r.quantizer == quant)
            .collect();

        if quant_results.is_empty() {
            continue;
        }

        let avg_throughput: f64 = quant_results
            .iter()
            .map(|r| r.throughput_mpixels())
            .sum::<f64>()
            / quant_results.len() as f64;

        let avg_bytes_per_pixel: f64 = quant_results
            .iter()
            .map(|r| r.bytes_per_pixel())
            .sum::<f64>()
            / quant_results.len() as f64;

        println!("\n{} encode:", quant);
        println!("  Avg throughput: {:.1} Mpix/s", avg_throughput);
        println!("  Avg memory: {:.1} bytes/pixel", avg_bytes_per_pixel);
    }

    // Decode summary
    let decode_results: Vec<_> = results.iter().filter(|r| r.operation == "decode").collect();

    if !decode_results.is_empty() {
        let avg_throughput: f64 = decode_results
            .iter()
            .map(|r| r.throughput_mpixels())
            .sum::<f64>()
            / decode_results.len() as f64;

        let avg_bytes_per_pixel: f64 = decode_results
            .iter()
            .map(|r| r.bytes_per_pixel())
            .sum::<f64>()
            / decode_results.len() as f64;

        println!("\ndecode:");
        println!("  Avg throughput: {:.1} Mpix/s", avg_throughput);
        println!("  Avg memory: {:.1} bytes/pixel", avg_bytes_per_pixel);
    }

    println!();
    io::stdout().flush().unwrap();
}
