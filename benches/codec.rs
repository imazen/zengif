//! Benchmarks for zengif codec operations

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use zengif::{
    Decoder, EncodeRequest, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable,
};

/// Simple 4-color palette for benchmarking.
fn benchmark_palette() -> Palette {
    Palette::from_rgba(vec![
        Rgba::rgb(255, 0, 0),   // Red
        Rgba::rgb(0, 255, 0),   // Green
        Rgba::rgb(0, 0, 255),   // Blue
        Rgba::rgb(255, 255, 0), // Yellow
        Rgba::TRANSPARENT,      // Transparent
    ])
}

/// Create a synthetic GIF with solid color frames for benchmarking.
fn create_test_gif(width: u16, height: u16, frame_count: usize) -> Vec<u8> {
    let colors = [
        Rgba::rgb(255, 0, 0),   // Red
        Rgba::rgb(0, 255, 0),   // Green
        Rgba::rgb(0, 0, 255),   // Blue
        Rgba::rgb(255, 255, 0), // Yellow
    ];
    let palette = benchmark_palette();

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::none();
    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .unwrap();

    for i in 0..frame_count {
        let color = colors[i % colors.len()];
        let pixels: Vec<Rgba> = (0..width as usize * height as usize)
            .map(|_| color)
            .collect();
        let frame = FrameInput::with_palette(width, height, 10, pixels, palette.clone());
        encoder.add_frame(frame).unwrap();
    }
    encoder.finish().unwrap()
}

/// Create a GIF with transparency (checkerboard pattern).
fn create_transparent_gif(width: u16, height: u16, frame_count: usize) -> Vec<u8> {
    let palette = benchmark_palette();
    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::none();
    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .unwrap();

    for frame_idx in 0..frame_count {
        let pixels: Vec<Rgba> = (0..width as usize * height as usize)
            .enumerate()
            .map(|(i, _)| {
                let x = i % width as usize;
                let y = i / width as usize;
                // Checkerboard with animation
                if (x + y + frame_idx) % 2 == 0 {
                    Rgba::rgb(255, 0, 0)
                } else {
                    Rgba::TRANSPARENT
                }
            })
            .collect();
        let frame = FrameInput::with_palette(width, height, 10, pixels, palette.clone());
        encoder.add_frame(frame).unwrap();
    }
    encoder.finish().unwrap()
}

fn decode_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");

    // Different sizes
    for (width, height) in [(256, 256), (512, 512), (1024, 1024)] {
        let frame_count = 10;
        let gif_data = create_test_gif(width, height, frame_count);
        let bytes_per_frame = width as u64 * height as u64 * 4;

        group.throughput(Throughput::Bytes(bytes_per_frame * frame_count as u64));
        group.bench_with_input(
            BenchmarkId::new("solid", format!("{}x{}x{}", width, height, frame_count)),
            &gif_data,
            |b, data| {
                b.iter(|| {
                    let cursor = std::io::Cursor::new(data);
                    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
                    while let Some(frame) = decoder.next_frame().unwrap() {
                        black_box(frame);
                    }
                });
            },
        );
    }

    // With transparency (slower path)
    let gif_data = create_transparent_gif(512, 512, 10);
    let bytes_per_frame = 512u64 * 512 * 4;
    group.throughput(Throughput::Bytes(bytes_per_frame * 10));
    group.bench_with_input(
        BenchmarkId::new("transparent", "512x512x10"),
        &gif_data,
        |b, data| {
            b.iter(|| {
                let cursor = std::io::Cursor::new(data);
                let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
                while let Some(frame) = decoder.next_frame().unwrap() {
                    black_box(frame);
                }
            });
        },
    );

    group.finish();
}

fn encode_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");

    // Pre-create frames for encoding benchmark
    for (width, height) in [(256, 256), (512, 512)] {
        let frame_count = 10;
        let palette = benchmark_palette();
        let frames: Vec<FrameInput> = (0..frame_count)
            .map(|i| {
                let colors = [
                    Rgba::rgb(255, 0, 0),
                    Rgba::rgb(0, 255, 0),
                    Rgba::rgb(0, 0, 255),
                ];
                let color = colors[i % colors.len()];
                let pixels: Vec<Rgba> = (0..width as usize * height as usize)
                    .map(|_| color)
                    .collect();
                FrameInput::with_palette(width, height, 10, pixels, palette.clone())
            })
            .collect();

        let bytes_per_frame = width as u64 * height as u64 * 4;
        group.throughput(Throughput::Bytes(bytes_per_frame * frame_count as u64));
        group.bench_with_input(
            BenchmarkId::new("solid", format!("{}x{}x{}", width, height, frame_count)),
            &frames,
            |b, frames| {
                b.iter(|| {
                    let config = EncoderConfig::new().repeat(Repeat::Infinite);
                    let limits = Limits::none();
                    let mut encoder = EncodeRequest::new(&config, width, height)
                        .limits(&limits)
                        .stop(&Unstoppable)
                        .build()
                        .unwrap();
                    for frame in frames {
                        encoder.add_frame(frame.clone()).unwrap();
                    }
                    black_box(encoder.finish().unwrap())
                });
            },
        );
    }

    group.finish();
}

/// Create a GIF with noisy frames that have random "dirty" regions.
/// This tests frame differencing effectiveness - only the changed region should be encoded.
fn create_noisy_animation(
    width: u16,
    height: u16,
    frame_count: usize,
    dirty_region_size: u16,
) -> Vec<u8> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Deterministic "random" for reproducibility
    fn pseudo_random(seed: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        hasher.finish()
    }

    let palette = Palette::from_rgba(
        (0..256)
            .map(|i| {
                let r = pseudo_random(i as u64) as u8;
                let g = pseudo_random(i as u64 + 1000) as u8;
                let b = pseudo_random(i as u64 + 2000) as u8;
                Rgba::rgb(r, g, b)
            })
            .collect(),
    );

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::none();
    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .unwrap();

    // Create base frame with noise
    let mut base_pixels: Vec<Rgba> = (0..width as usize * height as usize)
        .map(|i| {
            let idx = pseudo_random(i as u64) as usize % 256;
            palette.colors()[idx]
        })
        .collect();

    // First frame is the full base
    let frame = FrameInput::with_palette(width, height, 10, base_pixels.clone(), palette.clone());
    encoder.add_frame(frame).unwrap();

    // Subsequent frames modify a random region
    for frame_idx in 1..frame_count {
        let seed = frame_idx as u64 * 12345;
        let region_x = (pseudo_random(seed) % (width - dirty_region_size) as u64) as u16;
        let region_y = (pseudo_random(seed + 1) % (height - dirty_region_size) as u64) as u16;

        // Modify only the dirty region
        for dy in 0..dirty_region_size as usize {
            for dx in 0..dirty_region_size as usize {
                let x = region_x as usize + dx;
                let y = region_y as usize + dy;
                let pixel_idx = y * width as usize + x;
                let color_idx = pseudo_random((frame_idx * 1000 + pixel_idx) as u64) as usize % 256;
                base_pixels[pixel_idx] = palette.colors()[color_idx];
            }
        }

        let frame =
            FrameInput::with_palette(width, height, 10, base_pixels.clone(), palette.clone());
        encoder.add_frame(frame).unwrap();
    }
    encoder.finish().unwrap()
}

/// Benchmark for animation with many frames and frame differencing.
fn animation_stress_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("animation_stress");
    group.sample_size(10); // Fewer samples for long benchmarks

    let width = 512u16;
    let height = 512u16;
    let frame_count = 200;

    // Test different dirty region sizes
    for dirty_size in [32, 64, 128, 256] {
        let label = format!("{}x{}x{}_dirty{}", width, height, frame_count, dirty_size);
        let bytes_per_frame = width as u64 * height as u64 * 4;

        // Benchmark encode (frame differencing)
        {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            fn pseudo_random(seed: u64) -> u64 {
                let mut hasher = DefaultHasher::new();
                seed.hash(&mut hasher);
                hasher.finish()
            }

            let palette = Palette::from_rgba(
                (0..256)
                    .map(|i| {
                        let r = pseudo_random(i as u64) as u8;
                        let g = pseudo_random(i as u64 + 1000) as u8;
                        let b = pseudo_random(i as u64 + 2000) as u8;
                        Rgba::rgb(r, g, b)
                    })
                    .collect(),
            );

            // Pre-create all frames
            let mut base_pixels: Vec<Rgba> = (0..width as usize * height as usize)
                .map(|i| {
                    let idx = pseudo_random(i as u64) as usize % 256;
                    palette.colors()[idx]
                })
                .collect();

            let mut frames = Vec::with_capacity(frame_count);
            frames.push(FrameInput::with_palette(
                width,
                height,
                10,
                base_pixels.clone(),
                palette.clone(),
            ));

            for frame_idx in 1..frame_count {
                let seed = frame_idx as u64 * 12345;
                let region_x = (pseudo_random(seed) % (width - dirty_size) as u64) as u16;
                let region_y = (pseudo_random(seed + 1) % (height - dirty_size) as u64) as u16;

                for dy in 0..dirty_size as usize {
                    for dx in 0..dirty_size as usize {
                        let x = region_x as usize + dx;
                        let y = region_y as usize + dy;
                        let pixel_idx = y * width as usize + x;
                        let color_idx =
                            pseudo_random((frame_idx * 1000 + pixel_idx) as u64) as usize % 256;
                        base_pixels[pixel_idx] = palette.colors()[color_idx];
                    }
                }

                frames.push(FrameInput::with_palette(
                    width,
                    height,
                    10,
                    base_pixels.clone(),
                    palette.clone(),
                ));
            }

            group.throughput(Throughput::Bytes(bytes_per_frame * frame_count as u64));
            group.bench_with_input(BenchmarkId::new("encode", &label), &frames, |b, frames| {
                b.iter(|| {
                    let config = EncoderConfig::new().repeat(Repeat::Infinite);
                    let limits = Limits::none();
                    let mut encoder = EncodeRequest::new(&config, width, height)
                        .limits(&limits)
                        .stop(&Unstoppable)
                        .build()
                        .unwrap();
                    for frame in frames {
                        encoder.add_frame(frame.clone()).unwrap();
                    }
                    black_box(encoder.finish().unwrap())
                });
            });
        }

        // Benchmark decode
        {
            let gif_data = create_noisy_animation(width, height, frame_count, dirty_size);
            group.throughput(Throughput::Bytes(bytes_per_frame * frame_count as u64));
            group.bench_with_input(BenchmarkId::new("decode", &label), &gif_data, |b, data| {
                b.iter(|| {
                    let cursor = std::io::Cursor::new(data);
                    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
                    while let Some(frame) = decoder.next_frame().unwrap() {
                        black_box(frame);
                    }
                });
            });
        }
    }

    group.finish();
}

/// Benchmark that measures allocation overhead - helps identify memory pooling opportunities.
fn memory_allocation_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_allocation");

    let width = 512u16;
    let height = 512u16;
    let frame_count = 50;
    let palette = benchmark_palette();

    // Pre-create frames
    let frames: Vec<FrameInput> = (0..frame_count)
        .map(|i| {
            let colors = [
                Rgba::rgb(255, 0, 0),
                Rgba::rgb(0, 255, 0),
                Rgba::rgb(0, 0, 255),
            ];
            let color = colors[i % colors.len()];
            let pixels: Vec<Rgba> = (0..width as usize * height as usize)
                .map(|_| color)
                .collect();
            FrameInput::with_palette(width, height, 10, pixels, palette.clone())
        })
        .collect();

    let bytes_per_frame = width as u64 * height as u64 * 4;
    group.throughput(Throughput::Bytes(bytes_per_frame * frame_count as u64));

    // Encoder manages its own internal buffer
    group.bench_with_input(
        BenchmarkId::new("encode", "512x512x50"),
        &frames,
        |b, frames| {
            b.iter(|| {
                let config = EncoderConfig::new().repeat(Repeat::Infinite);
                let limits = Limits::none();
                let mut encoder = EncodeRequest::new(&config, width, height)
                    .limits(&limits)
                    .stop(&Unstoppable)
                    .build()
                    .unwrap();
                for frame in frames {
                    encoder.add_frame(frame.clone()).unwrap();
                }
                black_box(encoder.finish().unwrap())
            });
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    decode_benchmark,
    encode_benchmark,
    animation_stress_benchmark,
    memory_allocation_benchmark
);
criterion_main!(benches);
