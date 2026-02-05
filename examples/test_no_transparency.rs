use std::fs;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};
use imgref::ImgVec;

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
    let o: Vec<[u8;3]> = orig.iter().map(|p| [p.r, p.g, p.b]).collect();
    let e: Vec<[u8;3]> = enc.iter().map(|p| [p.r, p.g, p.b]).collect();
    fast_ssim2::compute_ssimulacra2(
        ImgVec::new(o, w, h).as_ref(),
        ImgVec::new(e, w, h).as_ref()
    ).unwrap_or(-1.0)
}

fn main() {
    for (name, path) in [
        ("cat_typing", "/tmp/gif-testset/cat_typing_420x375x629.gif"),
        ("spinner", "/tmp/gif-testset/spinner_256x256x202.gif"),
    ] {
        let data = fs::read(path).unwrap();
        let (w, h, orig_frames) = decode_to_frames(&data).unwrap();
        
        let inputs: Vec<_> = orig_frames.iter()
            .map(|p| FrameInput::new(w, h, 10, p.clone()))
            .collect();
        
        println!("=== {} ===", name);
        
        // Test different configurations
        for (mode, shared, use_trans) in [
            ("per-frame + trans", false, true),
            ("per-frame - trans", false, false),
            ("shared + trans", true, true),
            ("shared - trans", true, false),
        ] {
            let mut output = Vec::new();
            {
                let config = EncoderConfig::new(w, h)
                    .repeat(Repeat::Infinite)
                    .shared_palette(shared)
                    .use_transparency(use_trans);
                let mut enc = Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();
                for frame in &inputs {
                    enc.add_frame(frame.clone()).unwrap();
                }
                enc.finish().unwrap();
            }
            
            let (_, _, enc_frames) = decode_to_frames(&output).unwrap();
            
            let mut scores = Vec::new();
            for i in 0..orig_frames.len().min(enc_frames.len()) {
                let s = compute_ssim2(&orig_frames[i], &enc_frames[i], w as usize, h as usize);
                if s >= 0.0 { scores.push(s); }
            }
            
            let avg = scores.iter().sum::<f64>() / scores.len() as f64;
            let worst = scores.iter().cloned().fold(f64::INFINITY, f64::min);
            
            println!("  {:<20} {:>7}KB  avg={:>5.1}  worst={:>5.1}",
                mode, output.len() / 1024, avg, worst);
        }
        println!();
    }
}
