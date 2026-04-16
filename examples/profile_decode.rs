//! Callgrind profiling target for 4096x4096 GIF decode.
//!
//! Two-phase design: first run generates the fixture, second run decodes only.
//!
//! Usage:
//! ```sh
//! cargo build --release --example profile_decode
//!
//! # Phase 1: generate fixture (run WITHOUT callgrind)
//! target/release/examples/profile_decode --generate /tmp/zengif-4096.gif
//!
//! # Phase 2: profile decode only
//! valgrind --tool=callgrind --callgrind-out-file=/tmp/zengif-callgrind.out \
//!   target/release/examples/profile_decode /tmp/zengif-4096.gif
//! callgrind_annotate /tmp/zengif-callgrind.out > /tmp/zengif-callgrind-summary.txt
//! ```

use std::hint::black_box;
use zengif::{
    Decoder, EncodeRequest, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable,
};

/// Generate a single-frame 4096x4096 GIF with a 256-color gradient palette.
/// Same fixture as decode_bench.rs.
fn generate_4096x4096_gif() -> Vec<u8> {
    let width: u16 = 4096;
    let height: u16 = 4096;

    let palette = Palette::from_rgba(
        (0..256)
            .map(|i| {
                let i = i as u8;
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
            let idx = ((x ^ y) & 0xFF) as u8;
            palette.colors()[idx as usize]
        })
        .collect();

    let frame = FrameInput::with_palette(width, height, 0, pixels, palette);
    encoder.add_frame(frame).unwrap();
    encoder.finish().unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 3 && args[1] == "--generate" {
        // Phase 1: generate fixture to file
        eprintln!("Generating 4096x4096 GIF fixture...");
        let gif_data = generate_4096x4096_gif();
        std::fs::write(&args[2], &gif_data).unwrap();
        eprintln!("Wrote {} bytes to {}", gif_data.len(), args[2]);
        return;
    }

    if args.len() >= 2 && args[1] != "--generate" {
        // Phase 2: decode from file (the profiling target)
        let path = &args[1];
        let gif_data = std::fs::read(path).unwrap();
        eprintln!("Read {} bytes from {}, decoding...", gif_data.len(), path);

        let cursor = std::io::Cursor::new(&gif_data);
        let mut decoder = Decoder::new(cursor, Limits::none(), &Unstoppable).unwrap();
        let mut total_pixels = 0usize;
        while let Some(frame) = decoder.next_frame().unwrap() {
            total_pixels += frame.pixels.len();
            black_box(&frame.pixels);
        }
        eprintln!("Decoded {total_pixels} pixels");
        return;
    }

    eprintln!("Usage:");
    eprintln!("  profile_decode --generate <output.gif>   Generate fixture");
    eprintln!("  profile_decode <input.gif>               Decode (profile target)");
    std::process::exit(1);
}
