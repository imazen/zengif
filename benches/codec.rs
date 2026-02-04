//! Benchmarks for zengif codec operations

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable};

/// Simple 4-color palette for benchmarking.
fn benchmark_palette() -> Palette {
    Palette::from_rgba(vec![
        Rgba::rgb(255, 0, 0),      // Red
        Rgba::rgb(0, 255, 0),      // Green
        Rgba::rgb(0, 0, 255),      // Blue
        Rgba::rgb(255, 255, 0),    // Yellow
        Rgba::TRANSPARENT,        // Transparent
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

    let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();

    for i in 0..frame_count {
        let color = colors[i % colors.len()];
        let pixels: Vec<Rgba> = (0..width as usize * height as usize)
            .map(|_| color)
            .collect();
        let frame = FrameInput::with_palette(width, height, 10, pixels, palette.clone());
        encoder.add_frame(frame).unwrap();
    }
    encoder.finish().unwrap();
    output
}

/// Create a GIF with transparency (checkerboard pattern).
fn create_transparent_gif(width: u16, height: u16, frame_count: usize) -> Vec<u8> {
    let palette = benchmark_palette();
    let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();

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
    encoder.finish().unwrap();
    output
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
                    let mut decoder =
                        Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
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
                    let config = EncoderConfig::new(width, height).repeat(Repeat::Infinite);
                    let mut output = Vec::with_capacity(1024 * 1024);
                    let mut encoder =
                        Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();
                    for frame in frames {
                        encoder.add_frame(frame.clone()).unwrap();
                    }
                    encoder.finish().unwrap();
                    black_box(output)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, decode_benchmark, encode_benchmark);
criterion_main!(benches);
