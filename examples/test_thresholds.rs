//! Test different RMSE thresholds for hybrid mode

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

fn test_gif(path: &Path, thresholds: &[f32]) {
    let name = path.file_name().unwrap().to_string_lossy();
    let data = fs::read(path).unwrap();
    let (w, h, orig_frames) = decode_to_frames(&data).unwrap();

    if orig_frames.is_empty() {
        return;
    }

    let inputs: Vec<_> = orig_frames
        .iter()
        .map(|p| FrameInput::new(w, h, 10, p.clone()))
        .collect();

    print!(
        "{:<40} {:>4}x{:<4} {:>3}f  ",
        &name[..name.len().min(40)],
        w,
        h,
        orig_frames.len()
    );

    for &threshold in thresholds {
        let mut output = Vec::new();
        {
            let mut config = EncoderConfig::new()
                .repeat(Repeat::Infinite)
                .shared_palette(true);
            if threshold > 0.0 {
                config = config.palette_error_threshold(Some(threshold));
            } else {
                config = config.palette_error_threshold(None);
            }
            let mut enc =
                Encoder::new(&mut output, 4, 4, config, Limits::none(), Unstoppable).unwrap();
            for frame in &inputs {
                enc.add_frame(frame.clone()).unwrap();
            }
            enc.finish().unwrap();
        }

        let (_, _, enc_frames) = decode_to_frames(&output).unwrap_or((0, 0, vec![]));
        let ww = w as usize;
        let hh = h as usize;

        let worst_ssim: f64 = orig_frames
            .iter()
            .zip(enc_frames.iter())
            .map(|(o, e)| compute_ssim2(o, e, ww, hh))
            .filter(|&s| s >= 0.0)
            .fold(f64::INFINITY, f64::min);

        print!("{:>5.1}/{:>5}KB  ", worst_ssim, output.len() / 1024);
    }
    println!();
}

fn main() {
    let test_dir = Path::new("/tmp/gif-testset");
    let thresholds = [0.0, 15.0, 10.0, 5.0, 2.0]; // 0 = no fallback

    // Header
    print!("{:<40} {:>10} {:>5}  ", "File", "Dims", "Frms");
    for &t in &thresholds {
        if t == 0.0 {
            print!("{:>13}  ", "NoFallback");
        } else {
            print!("{:>13}  ", format!("Thr={:.0}", t));
        }
    }
    println!();
    println!("{}", "-".repeat(120));

    // Test low-quality GIFs first
    let problem_gifs = [
        "glitch",
        "vaporwave",
        "newtons",
        "ocean",
        "rainbow",
        "space",
        "city",
    ];

    let mut entries: Vec<_> = fs::read_dir(test_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "gif").unwrap_or(false))
        .filter(|e| {
            problem_gifs
                .iter()
                .any(|p| e.path().to_string_lossy().contains(p))
        })
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        test_gif(&entry.path(), &thresholds);
    }
}
