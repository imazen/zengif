//! Focused memory profiler for accurate per-operation measurements.
//!
//! Uses a tracking allocator to measure peak memory for each encode/decode operation.
//!
//! Run with:
//! ```bash
//! cargo run --release --all-features --example memory_profile
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use enough::Unstoppable;
#[cfg(any(feature = "imagequant", feature = "quantizr", feature = "color_quant"))]
use zengif::EncoderConfig;
use zengif::{Decoder, FrameInput, Limits, Rgba};

#[cfg(feature = "color_quant")]
use zengif::ColorQuantQuantizer;
#[cfg(feature = "imagequant")]
use zengif::ImagequantQuantizer;
#[cfg(feature = "quantizr")]
use zengif::QuantizrQuantizer;

// =============================================================================
// Tracking allocator
// =============================================================================

/// Global allocator that tracks current and peak memory usage.
/// Uses saturating arithmetic to avoid overflow issues.
struct TrackingAllocator {
    inner: System,
    current: AtomicUsize,
    peak: AtomicUsize,
    baseline: AtomicUsize,
}

impl TrackingAllocator {
    const fn new() -> Self {
        Self {
            inner: System,
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            baseline: AtomicUsize::new(0),
        }
    }

    /// Reset peak tracking. Sets baseline to current allocation level.
    fn reset(&self) {
        let current = self.current.load(Ordering::SeqCst);
        self.baseline.store(current, Ordering::SeqCst);
        self.peak.store(current, Ordering::SeqCst);
    }

    /// Get peak memory above baseline since last reset.
    fn peak(&self) -> usize {
        let peak = self.peak.load(Ordering::SeqCst);
        let baseline = self.baseline.load(Ordering::SeqCst);
        peak.saturating_sub(baseline)
    }
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = self.inner.alloc(layout);
        if !ptr.is_null() {
            let size = layout.size();
            let new_current = self.current.fetch_add(size, Ordering::SeqCst) + size;
            // Update peak if this is a new high
            loop {
                let current_peak = self.peak.load(Ordering::SeqCst);
                if new_current <= current_peak {
                    break;
                }
                if self
                    .peak
                    .compare_exchange_weak(
                        current_peak,
                        new_current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    break;
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Use saturating_sub to prevent underflow
        self.current
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.saturating_sub(layout.size()))
            })
            .ok();
        self.inner.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old_size = layout.size();
        let new_ptr = self.inner.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            // Adjust current: subtract old, add new using saturating arithmetic
            if new_size > old_size {
                let diff = new_size - old_size;
                let new_current = self.current.fetch_add(diff, Ordering::SeqCst) + diff;
                loop {
                    let current_peak = self.peak.load(Ordering::SeqCst);
                    if new_current <= current_peak {
                        break;
                    }
                    if self
                        .peak
                        .compare_exchange_weak(
                            current_peak,
                            new_current,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        break;
                    }
                }
            } else {
                let diff = old_size - new_size;
                self.current
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                        Some(v.saturating_sub(diff))
                    })
                    .ok();
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator::new();

// =============================================================================
// Image generators
// =============================================================================

fn generate_photo_like(width: u32, height: u32, frame_index: u32) -> Vec<Rgba> {
    use std::f32::consts::{E, PI};
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let offset = frame_index as f32 * 0.1;

    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            let r = ((fx * 200.0 + (fy * PI + offset).sin() * 30.0).clamp(0.0, 255.0)) as u8;
            let g = ((fy * 180.0 + (fx * E + offset).cos() * 40.0).clamp(0.0, 255.0)) as u8;
            let b = (((fx + fy) * 100.0 + ((fx * fy * 10.0) + offset).sin() * 50.0)
                .clamp(0.0, 255.0)) as u8;
            pixels.push(Rgba::rgb(r, g, b));
        }
    }
    pixels
}

// =============================================================================
// Measurement functions
// =============================================================================

struct Measurement {
    name: String,
    width: u32,
    height: u32,
    frames: u32,
    time_us: u64,
    peak_bytes: usize,
    throughput_mpixels: f64,
    bytes_per_pixel: f64,
}

fn measure_decode(width: u32, height: u32, frame_count: u32, gif_data: &[u8]) -> Measurement {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    // Reset allocator before measurement
    ALLOCATOR.reset();

    let start = Instant::now();
    let cursor = std::io::Cursor::new(gif_data);
    let mut decoder = Decoder::new(cursor, Limits::default(), &Unstoppable).unwrap();
    let _frames = decoder.decode_all().unwrap();
    let elapsed = start.elapsed();

    let peak = ALLOCATOR.peak();
    let time_us = elapsed.as_micros() as u64;

    Measurement {
        name: "decode".into(),
        width,
        height,
        frames: frame_count,
        time_us,
        peak_bytes: peak,
        throughput_mpixels: if time_us > 0 {
            (pixels as f64) / (time_us as f64 / 1_000_000.0) / 1_000_000.0
        } else {
            0.0
        },
        bytes_per_pixel: if pixels > 0 {
            peak as f64 / pixels as f64
        } else {
            0.0
        },
    }
}

#[cfg(feature = "imagequant")]
fn measure_encode_imagequant(
    width: u32,
    height: u32,
    frame_count: u32,
    frames: Vec<FrameInput>,
) -> Measurement {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    ALLOCATOR.reset();

    let start = Instant::now();
    let config = EncoderConfig::new().dithering(0.5);
    let quantizer = ImagequantQuantizer::new();
    let _output = zengif::encode_gif_with_quantizer(
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

    let peak = ALLOCATOR.peak();
    let time_us = elapsed.as_micros() as u64;

    Measurement {
        name: "imagequant".into(),
        width,
        height,
        frames: frame_count,
        time_us,
        peak_bytes: peak,
        throughput_mpixels: if time_us > 0 {
            (pixels as f64) / (time_us as f64 / 1_000_000.0) / 1_000_000.0
        } else {
            0.0
        },
        bytes_per_pixel: if pixels > 0 {
            peak as f64 / pixels as f64
        } else {
            0.0
        },
    }
}

#[cfg(feature = "quantizr")]
fn measure_encode_quantizr(
    width: u32,
    height: u32,
    frame_count: u32,
    frames: Vec<FrameInput>,
) -> Measurement {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    ALLOCATOR.reset();

    let start = Instant::now();
    let config = EncoderConfig::new().dithering(0.5);
    let quantizer = QuantizrQuantizer::new();
    let _output = zengif::encode_gif_with_quantizer(
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

    let peak = ALLOCATOR.peak();
    let time_us = elapsed.as_micros() as u64;

    Measurement {
        name: "quantizr".into(),
        width,
        height,
        frames: frame_count,
        time_us,
        peak_bytes: peak,
        throughput_mpixels: if time_us > 0 {
            (pixels as f64) / (time_us as f64 / 1_000_000.0) / 1_000_000.0
        } else {
            0.0
        },
        bytes_per_pixel: if pixels > 0 {
            peak as f64 / pixels as f64
        } else {
            0.0
        },
    }
}

#[cfg(feature = "color_quant")]
fn measure_encode_color_quant(
    width: u32,
    height: u32,
    frame_count: u32,
    frames: Vec<FrameInput>,
) -> Measurement {
    let pixels = (width as u64) * (height as u64) * (frame_count as u64);

    ALLOCATOR.reset();

    let start = Instant::now();
    let config = EncoderConfig::new().dithering(0.5);
    let quantizer = ColorQuantQuantizer::new();
    let _output = zengif::encode_gif_with_quantizer(
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

    let peak = ALLOCATOR.peak();
    let time_us = elapsed.as_micros() as u64;

    Measurement {
        name: "color_quant".into(),
        width,
        height,
        frames: frame_count,
        time_us,
        peak_bytes: peak,
        throughput_mpixels: if time_us > 0 {
            (pixels as f64) / (time_us as f64 / 1_000_000.0) / 1_000_000.0
        } else {
            0.0
        },
        bytes_per_pixel: if pixels > 0 {
            peak as f64 / pixels as f64
        } else {
            0.0
        },
    }
}

fn print_measurement(m: &Measurement) {
    println!(
        "{:<12} {:>4}x{:<4} {:>2} frames  {:>10} µs  {:>12} bytes peak  {:>6.1} Mpix/s  {:>5.1} B/pix",
        m.name,
        m.width,
        m.height,
        m.frames,
        m.time_us,
        m.peak_bytes,
        m.throughput_mpixels,
        m.bytes_per_pixel
    );
}

fn main() {
    println!(
        "╔══════════════════════════════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║                              ZENGIF MEMORY PROFILER (Tracking Allocator)                                 ║"
    );
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!();

    let test_configs = [
        (256, 256, 1),
        (256, 256, 5),
        (512, 512, 1),
        (512, 512, 5),
        (1024, 1024, 1),
        (1024, 1024, 5),
    ];

    // First, generate test frames and encode to GIF for decode testing
    #[allow(unused_mut)]
    let mut test_gifs: Vec<((u32, u32, u32), Vec<u8>)> = Vec::new();

    println!("Preparing test data...");
    for &(width, height, frame_count) in &test_configs {
        #[allow(unused_variables)]
        let frames: Vec<FrameInput> = (0..frame_count)
            .map(|i| {
                let px = generate_photo_like(width, height, i);
                FrameInput::new(width as u16, height as u16, 10, px)
            })
            .collect();

        #[cfg(feature = "imagequant")]
        {
            let config = EncoderConfig::new();
            let quantizer = ImagequantQuantizer::new();
            let gif_data = zengif::encode_gif_with_quantizer(
                frames.clone(),
                width.try_into().unwrap(),
                height.try_into().unwrap(),
                config,
                Limits::default(),
                &Unstoppable,
                quantizer,
            )
            .unwrap();
            test_gifs.push(((width, height, frame_count), gif_data));
        }
    }
    println!("Test data prepared.\n");

    // ==========================================================================
    // DECODE MEASUREMENTS
    // ==========================================================================
    println!("{}", "=".repeat(110));
    println!("DECODE MEASUREMENTS (photo-like content)");
    println!("{}", "=".repeat(110));

    for ((width, height, frame_count), gif_data) in &test_gifs {
        let m = measure_decode(*width, *height, *frame_count, gif_data);
        print_measurement(&m);
    }

    // ==========================================================================
    // ENCODE MEASUREMENTS - imagequant
    // ==========================================================================
    #[cfg(feature = "imagequant")]
    {
        println!("\n{}", "=".repeat(110));
        println!("ENCODE MEASUREMENTS - imagequant (photo-like content)");
        println!("{}", "=".repeat(110));

        for &(width, height, frame_count) in &test_configs {
            let frames: Vec<FrameInput> = (0..frame_count)
                .map(|i| {
                    let px = generate_photo_like(width, height, i);
                    FrameInput::new(width as u16, height as u16, 10, px)
                })
                .collect();
            let m = measure_encode_imagequant(width, height, frame_count, frames);
            print_measurement(&m);
        }
    }

    // ==========================================================================
    // ENCODE MEASUREMENTS - quantizr
    // ==========================================================================
    #[cfg(feature = "quantizr")]
    {
        println!("\n{}", "=".repeat(110));
        println!("ENCODE MEASUREMENTS - quantizr (photo-like content)");
        println!("{}", "=".repeat(110));

        for &(width, height, frame_count) in &test_configs {
            let frames: Vec<FrameInput> = (0..frame_count)
                .map(|i| {
                    let px = generate_photo_like(width, height, i);
                    FrameInput::new(width as u16, height as u16, 10, px)
                })
                .collect();
            let m = measure_encode_quantizr(width, height, frame_count, frames);
            print_measurement(&m);
        }
    }

    // ==========================================================================
    // ENCODE MEASUREMENTS - color_quant
    // ==========================================================================
    #[cfg(feature = "color_quant")]
    {
        println!("\n{}", "=".repeat(110));
        println!("ENCODE MEASUREMENTS - color_quant (photo-like content)");
        println!("{}", "=".repeat(110));

        for &(width, height, frame_count) in &test_configs {
            let frames: Vec<FrameInput> = (0..frame_count)
                .map(|i| {
                    let px = generate_photo_like(width, height, i);
                    FrameInput::new(width as u16, height as u16, 10, px)
                })
                .collect();
            let m = measure_encode_color_quant(width, height, frame_count, frames);
            print_measurement(&m);
        }
    }

    // ==========================================================================
    // SUMMARY
    // ==========================================================================
    println!("\n{}", "=".repeat(110));
    println!("BYTES PER PIXEL SUMMARY (for heuristics calibration)");
    println!("{}", "=".repeat(110));
    println!();
    println!("These values should be used to update src/heuristics.rs constants.");
    println!("Memory = fixed_overhead + (pixels * bytes_per_pixel)");
    println!();

    std::io::stdout().flush().unwrap();
}
