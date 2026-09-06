//! Decode benchmark: zengif vs gif-rs at multiple resolutions.
//!
//! Uses zenbench's interleaved execution for stable paired comparisons.
//! Synthetic single-frame GIF fixtures at 256x256, 1024x1024, and 4096x4096.

use std::hint::black_box;
use zenbench::{BenchGroup, Suite, Throughput};
use zengif::{
    Decoder, EncodeRequest, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable,
};

/// Generate a single-frame GIF with XOR-coordinate palette indices.
fn generate_xor_gif(width: u16, height: u16, supplied_palette: bool) -> Vec<u8> {
    let palette = Palette::from_rgba(
        (0..256)
            .map(|i| {
                let i = i as u8;
                // Smooth RGB gradient across the palette
                Rgba::rgb(i, 255 - i, (i.wrapping_mul(7)) ^ 0xAA)
            })
            .collect(),
    );

    let config = EncoderConfig::new().quality(80).repeat(Repeat::Once);
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

    let frame = if supplied_palette {
        FrameInput::with_palette(width, height, 0, pixels, palette)
    } else {
        FrameInput::new(width, height, 0, pixels)
    };
    encoder.add_frame(frame).unwrap();
    encoder.finish().unwrap()
}

/// Decode using zengif `next_frame`: clones canvas per frame.
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

/// Decode using zengif `with_next_frame`: in-place compositing, no canvas clone.
/// This is the path used by the zencodec `render_next_frame` after optimization.
fn decode_zengif_inplace(data: &[u8]) -> usize {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, Limits::none(), &Unstoppable).unwrap();
    let mut total_pixels = 0usize;
    while let Some(count) = decoder
        .with_next_frame(|_index, _delay, pixels| {
            let len = pixels.len();
            black_box(pixels);
            len
        })
        .unwrap()
    {
        total_pixels += count;
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

/// Isolated RGBA→BGRA swizzle benchmark (garb SIMD vs scalar).
///
/// Measures just the swizzle overhead, separate from decode, to
/// quantify the garb improvement for BGRA-requesting callers.
fn swizzle_scalar(data: &mut [u8]) {
    for chunk in data.as_chunks_mut::<4>().0 {
        chunk.swap(0, 2);
    }
}

fn swizzle_garb(data: &mut [u8]) {
    garb::bytes::rgba_to_bgra_inplace(data).unwrap();
}

fn swizzle_benchmarks(suite: &mut Suite) {
    // 4096x4096 RGBA = 64 MiB — the target where swizzle cost matters
    let size = 4096usize * 4096 * 4;
    let rgba_bytes = size as u64;

    suite.group("swizzle_4096x4096", move |g: &mut BenchGroup| {
        g.throughput(Throughput::Bytes(rgba_bytes));

        g.bench("scalar", move |b| {
            let mut buf = vec![0u8; size];
            b.iter(|| {
                swizzle_scalar(black_box(&mut buf));
            });
        });

        g.bench("garb", move |b| {
            let mut buf = vec![0u8; size];
            b.iter(|| {
                swizzle_garb(black_box(&mut buf));
            });
        });
    });
}

fn decode_benchmarks(suite: &mut Suite) {
    // Pre-generate fixtures. Cache them so each benchmark iteration just
    // decodes (doesn't re-encode).
    let sizes: &[(u16, &str)] = &[
        (64, "64x64"),
        (256, "256x256"),
        (1024, "1024x1024"),
        (4096, "4096x4096"),
    ];
    let artifact_dir = std::env::var_os("CODEC_BENCH_ARTIFACT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../codec-artifacts/zengif-arm-audit")
        });
    std::fs::create_dir_all(&artifact_dir).unwrap();

    for &(dim, label) in sizes {
        let gif_data = generate_xor_gif(dim, dim, true);
        std::fs::write(artifact_dir.join(format!("xor-{label}.gif")), &gif_data).unwrap();
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
        let zen_data_inplace = gif_data.clone();
        let gif_data_clone = gif_data;

        suite.group(&group_name, move |g: &mut BenchGroup| {
            g.throughput(Throughput::Bytes(rgba_bytes));

            let zd = zen_data.clone();
            g.bench("zengif", move |b| {
                b.iter(|| {
                    decode_zengif(&zd);
                });
            });

            let zd2 = zen_data_inplace.clone();
            g.bench("zengif-inplace", move |b| {
                b.iter(|| {
                    decode_zengif_inplace(&zd2);
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

    swizzle_benchmarks(suite);
}

#[cfg(target_arch = "aarch64")]
fn encode_benchmarks(suite: &mut Suite) {
    for dim in [64u16, 512] {
        suite.compare(format!("gif_encode_default_q80/{dim}x{dim}"), |g| {
            let mut reference = None;
            for (label, enabled) in [("neon", true), ("forced_scalar", false)] {
                archmage::NeonToken::dangerously_disable_token_process_wide(!enabled).unwrap();
                let fixture = generate_xor_gif(dim, dim, false);
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../codec-artifacts/zengif-arm-audit");
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(
                    path.join(format!("encode-default-q80-{dim}-{label}.gif")),
                    &fixture,
                )
                .unwrap();
                assert_eq!(decode_zengif(&fixture), dim as usize * dim as usize);
                let mut decoder =
                    Decoder::new(std::io::Cursor::new(&fixture), Limits::none(), &Unstoppable)
                        .unwrap();
                let pixels = decoder.next_frame().unwrap().unwrap().pixels;
                if let Some(expected) = &reference {
                    assert_eq!(&pixels, expected, "encoded pixel tier parity at {dim}");
                } else {
                    reference = Some(pixels);
                }
                g.bench(label, move |b| {
                    b.with_input(move || {
                        archmage::NeonToken::dangerously_disable_token_process_wide(!enabled)
                            .unwrap()
                    })
                    .run(|_| generate_xor_gif(dim, dim, false))
                });
            }
        });
    }
    archmage::NeonToken::dangerously_disable_token_process_wide(false).unwrap();
}

#[cfg(not(target_arch = "aarch64"))]
fn encode_benchmarks(_: &mut Suite) {}

zenbench::main!(decode_benchmarks, encode_benchmarks);
