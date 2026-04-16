//! Decode benchmark: zengif vs gif-rs at multiple resolutions.
//!
//! Uses zenbench's interleaved execution for stable paired comparisons.
//! Synthetic single-frame GIF fixtures at 256x256, 1024x1024, and 4096x4096.

use std::hint::black_box;
use zenbench::{BenchGroup, Suite, Throughput};
use zengif::{
    Decoder, EncodeRequest, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable,
};

/// Generate a single-frame GIF with a 256-color gradient palette.
///
/// The content is a horizontal gradient that exercises most palette entries,
/// giving the LZW encoder realistic code table growth (not degenerate
/// solid-color input that compresses to almost nothing).
fn generate_gradient_gif(width: u16, height: u16) -> Vec<u8> {
    let palette = Palette::from_rgba(
        (0..256)
            .map(|i| {
                let i = i as u8;
                // Smooth RGB gradient across the palette
                Rgba::rgb(i, 255 - i, (i.wrapping_mul(7)) ^ 0xAA)
            })
            .collect(),
    );

    let config = EncoderConfig::new().repeat(Repeat::Once);
    let limits = Limits::none();
    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .unwrap();

    let w = width as usize;
    let h = height as usize;
    let pixels: Vec<Rgba> = (0..w * h)
        .map(|i| {
            let x = i % w;
            let y = i / w;
            // Mix x and y coordinates for non-trivial LZW input
            let idx = ((x ^ y) & 0xFF) as u8;
            palette.colors()[idx as usize]
        })
        .collect();

    let frame = FrameInput::with_palette(width, height, 0, pixels, palette);
    encoder.add_frame(frame).unwrap();
    encoder.finish().unwrap()
}

/// Decode using zengif: produces composited RGBA frames.
fn decode_zengif(data: &[u8]) -> usize {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, Limits::none(), &Unstoppable).unwrap();
    let mut total_pixels = 0usize;
    while let Some(frame) = decoder.next_frame().unwrap() {
        total_pixels += frame.pixels.len();
        black_box(&frame.pixels);
    }
    total_pixels
}

/// Decode using the gif crate directly: RGBA output, unlimited memory.
fn decode_gif_rs(data: &[u8]) -> usize {
    let cursor = std::io::Cursor::new(data);
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    opts.set_memory_limit(gif::MemoryLimit::Unlimited);
    let mut decoder = opts.read_info(cursor).unwrap();
    let mut total_pixels = 0usize;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        total_pixels += frame.buffer.len() / 4;
        black_box(&frame.buffer);
    }
    total_pixels
}

fn decode_benchmarks(suite: &mut Suite) {
    // Pre-generate fixtures. Cache them so each benchmark iteration just
    // decodes (doesn't re-encode).
    let sizes: &[(u16, &str)] = &[(256, "256x256"), (1024, "1024x1024"), (4096, "4096x4096")];

    for &(dim, label) in sizes {
        let gif_data = generate_gradient_gif(dim, dim);
        let pixels = dim as u64 * dim as u64;
        let rgba_bytes = pixels * 4;
        let data_len = gif_data.len();

        eprintln!(
            "[fixture] {label}: {} bytes compressed, {pixels} pixels ({} MiB RGBA)",
            data_len,
            rgba_bytes / (1024 * 1024)
        );

        let group_name = format!("gif_decode_{label}");

        // Clone data for each closure
        let zen_data = gif_data.clone();
        let gif_data_clone = gif_data;

        suite.group(&group_name, move |g: &mut BenchGroup| {
            g.throughput(Throughput::Bytes(rgba_bytes));

            let zd = zen_data.clone();
            g.bench("zengif", move |b| {
                b.iter(|| {
                    decode_zengif(&zd);
                });
            });

            let gd = gif_data_clone.clone();
            g.bench("gif-rs", move |b| {
                b.iter(|| {
                    decode_gif_rs(&gd);
                });
            });
        });
    }
}

zenbench::main!(decode_benchmarks);
