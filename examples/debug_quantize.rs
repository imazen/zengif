use std::collections::HashSet;
use std::fs;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn main() {
    let test_dir = "/tmp/gif-testset";
    
    for name in ["cat_typing", "spinner"] {
        let path = fs::read_dir(test_dir).unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().contains(name))
            .unwrap().path();
        
        let data = fs::read(&path).unwrap();
        
        // Decode original
        let cursor = std::io::Cursor::new(&data);
        let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
        let metadata = decoder.metadata().clone();
        let width = metadata.width;
        let height = metadata.height;
        
        let mut original_colors: HashSet<(u8,u8,u8)> = HashSet::new();
        let mut frame_inputs = Vec::new();
        
        while let Some(frame) = decoder.next_frame().unwrap() {
            for p in &frame.pixels {
                original_colors.insert((p.r, p.g, p.b));
            }
            frame_inputs.push(FrameInput::new(width, height, frame.delay, frame.pixels.clone()));
        }
        
        println!("=== {} ===", path.file_name().unwrap().to_string_lossy());
        println!("Original colors ({}):", original_colors.len());
        for c in original_colors.iter().take(20) {
            println!("  RGB({:3}, {:3}, {:3})", c.0, c.1, c.2);
        }
        
        // Encode with shared palette
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
        
        // Decode re-encoded version
        let cursor = std::io::Cursor::new(&output);
        let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
        
        let mut reencoded_colors: HashSet<(u8,u8,u8)> = HashSet::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            for p in &frame.pixels {
                reencoded_colors.insert((p.r, p.g, p.b));
            }
        }
        
        println!("\nRe-encoded colors ({}):", reencoded_colors.len());
        for c in reencoded_colors.iter().take(20) {
            println!("  RGB({:3}, {:3}, {:3})", c.0, c.1, c.2);
        }
        
        // Check which original colors are preserved
        let preserved: HashSet<_> = original_colors.intersection(&reencoded_colors).collect();
        let lost: HashSet<_> = original_colors.difference(&reencoded_colors).collect();
        
        println!("\nPreserved: {}/{}", preserved.len(), original_colors.len());
        if !lost.is_empty() {
            println!("Lost colors:");
            for c in lost.iter().take(10) {
                println!("  RGB({:3}, {:3}, {:3})", c.0, c.1, c.2);
            }
        }
        println!();
    }
}
