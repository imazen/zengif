use std::fs;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn main() {
    let path = "/tmp/gif-testset/cat_typing_420x375x629.gif";
    let data = fs::read(path).unwrap();
    
    // Decode original - frame 0
    let cursor = std::io::Cursor::new(&data);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
    let metadata = decoder.metadata().clone();
    let width = metadata.width;
    let height = metadata.height;
    
    let original_frame0 = decoder.next_frame().unwrap().unwrap();
    let mut all_frames = vec![original_frame0.clone()];
    while let Some(frame) = decoder.next_frame().unwrap() {
        all_frames.push(frame);
    }
    
    // Prepare inputs
    let frame_inputs: Vec<_> = all_frames.iter()
        .map(|f| FrameInput::new(width, height, f.delay, f.pixels.clone()))
        .collect();
    
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
    
    // Decode re-encoded - frame 0
    let cursor = std::io::Cursor::new(&output);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
    let reencoded_frame0 = decoder.next_frame().unwrap().unwrap();
    
    // Compare pixels
    println!("Frame 0 comparison ({}x{} = {} pixels):", width, height, original_frame0.pixels.len());
    
    let mut mismatches = 0;
    let mut first_mismatches = Vec::new();
    
    for (i, (orig, reenc)) in original_frame0.pixels.iter().zip(reencoded_frame0.pixels.iter()).enumerate() {
        if orig.r != reenc.r || orig.g != reenc.g || orig.b != reenc.b {
            mismatches += 1;
            if first_mismatches.len() < 10 {
                let x = i % width as usize;
                let y = i / width as usize;
                first_mismatches.push((x, y, *orig, *reenc));
            }
        }
    }
    
    println!("Mismatched pixels: {}/{} ({:.1}%)", 
        mismatches, 
        original_frame0.pixels.len(),
        100.0 * mismatches as f64 / original_frame0.pixels.len() as f64);
    
    if !first_mismatches.is_empty() {
        println!("\nFirst mismatches:");
        for (x, y, orig, reenc) in &first_mismatches {
            println!("  ({:4},{:4}): ({:3},{:3},{:3}) -> ({:3},{:3},{:3})",
                x, y, orig.r, orig.g, orig.b, reenc.r, reenc.g, reenc.b);
        }
    }
    
    // Also check alpha
    let alpha_mismatches: usize = original_frame0.pixels.iter()
        .zip(reencoded_frame0.pixels.iter())
        .filter(|(o, r)| o.a != r.a)
        .count();
    println!("\nAlpha mismatches: {}", alpha_mismatches);
    
    // Check frame 1 too
    let mut decoder = Decoder::new(std::io::Cursor::new(&data), Limits::none(), Unstoppable).unwrap();
    decoder.next_frame().unwrap(); // Skip frame 0
    let original_frame1 = decoder.next_frame().unwrap().unwrap();
    
    let mut decoder = Decoder::new(std::io::Cursor::new(&output), Limits::none(), Unstoppable).unwrap();
    decoder.next_frame().unwrap(); // Skip frame 0  
    let reencoded_frame1 = decoder.next_frame().unwrap().unwrap();
    
    let mismatches1: usize = original_frame1.pixels.iter()
        .zip(reencoded_frame1.pixels.iter())
        .filter(|(o, r)| o.r != r.r || o.g != r.g || o.b != r.b)
        .count();
    
    println!("\nFrame 1 mismatched pixels: {}/{} ({:.1}%)",
        mismatches1,
        original_frame1.pixels.len(),
        100.0 * mismatches1 as f64 / original_frame1.pixels.len() as f64);
}
