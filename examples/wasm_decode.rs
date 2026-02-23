//! Minimal wasm32 decode example - demonstrates that full codec works in wasm
//!
//! Build with: cargo build --target wasm32-unknown-unknown --example wasm_decode --release
//!
//! This example doesn't use any wasm-bindgen or web-sys - it's just a proof that
//! the decoder compiles and runs on wasm32-unknown-unknown.

use enough::Unstoppable;
use zengif::{Limits, decode_gif};

/// Decode a GIF and return frame count
pub fn decode_and_count(data: &[u8]) -> Result<usize, String> {
    let limits = Limits::default();

    let (_metadata, frames, _stats) =
        decode_gif(data, limits, &Unstoppable).map_err(|e| format!("{}", e))?;

    Ok(frames.len())
}

fn main() {
    // Minimal valid GIF (1x1 red pixel)
    let gif_data: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1
        0x80, // Global color table flag, 2 colors
        0x00, // Background color index
        0x00, // Pixel aspect ratio
        0xFF, 0x00, 0x00, // Color 0: Red
        0x00, 0x00, 0x00, // Color 1: Black
        0x2C, // Image descriptor
        0x00, 0x00, 0x00, 0x00, // Left, Top
        0x01, 0x00, 0x01, 0x00, // Width, Height
        0x00, // No local color table
        0x02, // LZW minimum code size
        0x02, // Block size
        0x44, 0x01, // LZW data
        0x00, // Block terminator
        0x3B, // Trailer
    ];

    match decode_and_count(gif_data) {
        Ok(count) => {
            // In wasm, this won't actually print without wasm-bindgen console bindings
            // But it proves the code compiles and runs
            let _ = count;
        }
        Err(_e) => {
            // Error handling
        }
    }
}
