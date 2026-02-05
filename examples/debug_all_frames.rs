use std::fs;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn main() {
    for (name, path) in [
        ("cat_typing", "/tmp/gif-testset/cat_typing_420x375x629.gif"),
        ("spinner", "/tmp/gif-testset/spinner_256x256x202.gif"),
    ] {
        let data = fs::read(path).unwrap();
        
        // Decode all original frames
        let cursor = std::io::Cursor::new(&data);
        let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
        let metadata = decoder.metadata().clone();
        let width = metadata.width;
        let height = metadata.height;
        
        let mut original_frames = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            original_frames.push(frame);
        }
        
        // Encode with shared palette
        let frame_inputs: Vec<_> = original_frames.iter()
            .map(|f| FrameInput::new(width, height, f.delay, f.pixels.clone()))
            .collect();
        
        let mut output = Vec::new();
        {
            let config = EncoderConfig::new(width, height)
                .repeat(Repeat::Infinite)
                .shared_palette(true);
            let mut encoder = Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();
            for frame in &frame_inputs {
                encoder.add_frame(frame.clone()).unwrap();
            }
            encoder.finish().unwrap();
        }
        
        // Decode re-encoded
        let cursor = std::io::Cursor::new(&output);
        let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
        let mut reencoded_frames = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            reencoded_frames.push(frame);
        }
        
        println!("=== {} ({} frames) ===", name, original_frames.len());
        
        let n = original_frames.len().min(reencoded_frames.len());
        for i in 0..n {
            let mismatches: usize = original_frames[i].pixels.iter()
                .zip(reencoded_frames[i].pixels.iter())
                .filter(|(o, r)| o.r != r.r || o.g != r.g || o.b != r.b)
                .count();
            
            let pct = 100.0 * mismatches as f64 / original_frames[i].pixels.len() as f64;
            if pct > 0.0 || i < 5 || i == n-1 {
                println!("  Frame {:2}: {:6} mismatches ({:5.1}%)", i, mismatches, pct);
            }
        }
        println!();
    }
}
