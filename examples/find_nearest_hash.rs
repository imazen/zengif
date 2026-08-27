//! Correctness harness: decode each corpus GIF, then re-encode every frame
//! through the `find_nearest` palette-mapping path (`FrameInput::with_palette`),
//! and print `sha256(encoded_bytes)  name`.
//!
//! Run on the baseline and optimized trees; the hashes MUST match byte-for-byte.
//! The encoded GIF bytes depend on the palette index `find_nearest` returns for
//! every pixel, so identical hashes prove identical indices (including ties).

use std::path::Path;
use zengif::{
    EncodeRequest, EncoderConfig, FrameInput, Limits, Palette, Repeat, Rgba, Unstoppable,
    decode_gif,
};

fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{:08x}", word));
    }
    out
}

/// Deterministic 64-color cube palette so `find_nearest` does real distance work
/// (few exact matches => both the chunked-min and early-exit paths get exercised).
fn cube_palette() -> Palette {
    let mut colors = Vec::with_capacity(64);
    for r in 0..4u32 {
        for g in 0..4u32 {
            for b in 0..4u32 {
                colors.push(Rgba::rgb((r * 85) as u8, (g * 85) as u8, (b * 85) as u8));
            }
        }
    }
    Palette::from_rgba(colors)
}

fn process(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let (_meta, frames, _stats) = decode_gif(&data, Limits::none(), &Unstoppable).ok()?;
    if frames.is_empty() {
        return None;
    }
    let width = frames[0].width;
    let height = frames[0].height;
    let palette = cube_palette();

    let config = EncoderConfig::new().repeat(Repeat::Infinite);
    let limits = Limits::none();
    let mut encoder = EncodeRequest::new(&config, width, height)
        .limits(&limits)
        .stop(&Unstoppable)
        .build()
        .ok()?;

    let mut any = false;
    for frame in &frames {
        if frame.width != width || frame.height != height {
            continue; // skip offset/cropped frames for a clean canvas-sized mapping
        }
        let fi = FrameInput::with_palette(
            width,
            height,
            frame.delay.max(1),
            frame.pixels.clone(),
            palette.clone(),
        );
        encoder.add_frame(fi).ok()?;
        any = true;
    }
    if !any {
        return None;
    }
    let out = encoder.finish().ok()?;
    Some(sha256(&out))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/codec-corpus");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut paths: Vec<_> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "gif").unwrap_or(false))
                .collect();
            paths.sort();
            args = paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
        }
    }
    let mut count = 0;
    for a in &args {
        let p = Path::new(a);
        match process(p) {
            Some(hash) => {
                println!("{}  {}", hash, p.file_name().unwrap().to_string_lossy());
                count += 1;
            }
            None => {
                println!("SKIP  {}", p.file_name().unwrap().to_string_lossy());
            }
        }
    }
    eprintln!("hashed {} files", count);
}
