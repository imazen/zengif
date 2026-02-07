//! Test if hybrid palette mode triggers per-frame fallback on diverse GIFs

use imgref::ImgVec;
use std::fs;
use std::path::Path;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};

fn decode_to_frames(data: &[u8]) -> Option<(u16, u16, Vec<Vec<Rgba>>)> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).ok()?;
    let meta = decoder.metadata().clone();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().ok()? {
        frames.push(frame.pixels.clone());
    }
    Some((meta.width, meta.height, frames))
}

fn compute_ssim2(orig: &[Rgba], enc: &[Rgba], w: usize, h: usize) -> f64 {
    let o: Vec<[u8; 3]> = orig.iter().map(|p| [p.r, p.g, p.b]).collect();
    let e: Vec<[u8; 3]> = enc.iter().map(|p| [p.r, p.g, p.b]).collect();
    fast_ssim2::compute_ssimulacra2(ImgVec::new(o, w, h).as_ref(), ImgVec::new(e, w, h).as_ref())
        .unwrap_or(-1.0)
}

fn main() {
    let test_dir = Path::new("/tmp/gif-testset");

    println!(
        "{:<45} {:>10} {:>6} {:>8} {:>8} {:>8} {:>6} {:>6}",
        "File", "Dims", "Frms", "Sh KB", "Hyb KB", "PF KB", "ShSSIM", "HySSIM"
    );
    println!("{}", "-".repeat(115));

    let mut entries: Vec<_> = fs::read_dir(test_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "gif").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();

        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let (w, h, orig_frames) = match decode_to_frames(&data) {
            Some(x) => x,
            None => continue,
        };

        if orig_frames.is_empty() {
            continue;
        }

        let inputs: Vec<_> = orig_frames
            .iter()
            .map(|p| FrameInput::new(w, h, 10, p.clone()))
            .collect();

        // Shared only (no fallback)
        let mut out_shared = Vec::new();
        {
            let config = EncoderConfig::new()
                .repeat(Repeat::Infinite)
                .shared_palette(true)
                .palette_error_threshold(None); // Disable fallback
            let mut enc =
                Encoder::new(&mut out_shared, 4, 4, config, Limits::none(), Unstoppable).unwrap();
            for frame in &inputs {
                enc.add_frame(frame.clone()).unwrap();
            }
            enc.finish().unwrap();
        }

        // Hybrid mode (with fallback at threshold=15)
        let mut out_hybrid = Vec::new();
        {
            let config = EncoderConfig::new()
                .repeat(Repeat::Infinite)
                .shared_palette(true)
                .palette_error_threshold(Some(15.0));
            let mut enc =
                Encoder::new(&mut out_hybrid, 4, 4, config, Limits::none(), Unstoppable).unwrap();
            for frame in &inputs {
                enc.add_frame(frame.clone()).unwrap();
            }
            enc.finish().unwrap();
        }

        // Per-frame only
        let mut out_perframe = Vec::new();
        {
            let config = EncoderConfig::new()
                .repeat(Repeat::Infinite)
                .shared_palette(false);
            let mut enc =
                Encoder::new(&mut out_perframe, 4, 4, config, Limits::none(), Unstoppable).unwrap();
            for frame in &inputs {
                enc.add_frame(frame.clone()).unwrap();
            }
            enc.finish().unwrap();
        }

        // Compute quality for shared and hybrid
        let (_, _, sh_frames) = decode_to_frames(&out_shared).unwrap_or((0, 0, vec![]));
        let (_, _, hy_frames) = decode_to_frames(&out_hybrid).unwrap_or((0, 0, vec![]));

        let ww = w as usize;
        let hh = h as usize;

        let sh_ssim: f64 = if !sh_frames.is_empty() {
            let scores: Vec<f64> = orig_frames
                .iter()
                .zip(sh_frames.iter())
                .map(|(o, e)| compute_ssim2(o, e, ww, hh))
                .filter(|&s| s >= 0.0)
                .collect();
            scores.iter().sum::<f64>() / scores.len() as f64
        } else {
            -1.0
        };

        let hy_ssim: f64 = if !hy_frames.is_empty() {
            let scores: Vec<f64> = orig_frames
                .iter()
                .zip(hy_frames.iter())
                .map(|(o, e)| compute_ssim2(o, e, ww, hh))
                .filter(|&s| s >= 0.0)
                .collect();
            scores.iter().sum::<f64>() / scores.len() as f64
        } else {
            -1.0
        };

        // Check if hybrid triggered fallback (file size different from shared)
        let fallback = if out_hybrid.len() != out_shared.len() {
            "*"
        } else {
            ""
        };

        println!(
            "{:<45} {:>4}x{:<4} {:>6} {:>7}KB {:>7}KB {:>7}KB {:>6.1} {:>6.1} {}",
            &name[..name.len().min(45)],
            w,
            h,
            orig_frames.len(),
            out_shared.len() / 1024,
            out_hybrid.len() / 1024,
            out_perframe.len() / 1024,
            sh_ssim,
            hy_ssim,
            fallback
        );
    }

    println!("\n* = hybrid triggered per-frame fallback (size differs from shared-only)");
}
