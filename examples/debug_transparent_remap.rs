use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable};
use std::fs;

fn main() {
    let path = "/tmp/gif-testset/cat_typing_420x375x629.gif";
    let data = fs::read(path).unwrap();
    
    // Decode
    let cursor = std::io::Cursor::new(&data);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
    let meta = decoder.metadata().clone();
    
    let mut original_frames = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        original_frames.push(frame.pixels.clone());
    }
    
    println!("Frame 0: {} pixels, {} transparent", 
        original_frames[0].len(),
        original_frames[0].iter().filter(|p| p.a == 0).count());
    
    println!("Frame 1: {} pixels, {} transparent",
        original_frames[1].len(), 
        original_frames[1].iter().filter(|p| p.a == 0).count());
    
    // Check what changed between frame 0 and 1
    let mut changed = 0;
    let mut unchanged = 0;
    for (a, b) in original_frames[0].iter().zip(original_frames[1].iter()) {
        if a == b { unchanged += 1; } else { changed += 1; }
    }
    println!("\nFrame 0 vs 1: {} changed, {} unchanged", changed, unchanged);
    
    // Now check what happens to transparent pixels in the encoded output
    let inputs: Vec<_> = original_frames.iter()
        .map(|p| FrameInput::new(meta.width, meta.height, 10, p.clone()))
        .collect();
    
    let mut output = Vec::new();
    {
        let config = EncoderConfig::new(meta.width, meta.height)
            .repeat(Repeat::Infinite)
            .shared_palette(true);
        let mut enc = Encoder::new(&mut output, config, Limits::none(), Unstoppable).unwrap();
        for frame in &inputs {
            enc.add_frame(frame.clone()).unwrap();
        }
        enc.finish().unwrap();
    }
    
    // Decode encoded output
    let cursor = std::io::Cursor::new(&output);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
    
    let encoded_frame0 = decoder.next_frame().unwrap().unwrap();
    let encoded_frame1 = decoder.next_frame().unwrap().unwrap();
    
    println!("\nEncoded frame 1: {} transparent", 
        encoded_frame1.pixels.iter().filter(|p| p.a == 0).count());
    
    // Check which "unchanged" pixels from original are now wrong
    let mut wrong_unchanged = 0;
    for i in 0..original_frames[0].len() {
        let orig0 = &original_frames[0][i];
        let orig1 = &original_frames[1][i];
        let enc1 = &encoded_frame1.pixels[i];
        
        // If pixel was unchanged in original (same in frame 0 and 1)
        if orig0 == orig1 {
            // But is now different in encoded
            if enc1.r != orig1.r || enc1.g != orig1.g || enc1.b != orig1.b {
                wrong_unchanged += 1;
                if wrong_unchanged <= 5 {
                    println!("  Pixel {}: orig=({},{},{}) enc=({},{},{})",
                        i, orig1.r, orig1.g, orig1.b, enc1.r, enc1.g, enc1.b);
                }
            }
        }
    }
    println!("\nUnchanged pixels that are now wrong: {}", wrong_unchanged);
}
