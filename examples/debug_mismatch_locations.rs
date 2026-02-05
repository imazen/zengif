use std::fs;
use zengif::{Decoder, Encoder, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable};

fn main() {
    let path = "/tmp/gif-testset/cat_typing_420x375x629.gif";
    let data = fs::read(path).unwrap();
    
    // Decode original
    let cursor = std::io::Cursor::new(&data);
    let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
    let meta = decoder.metadata().clone();
    let (w, h) = (meta.width as usize, meta.height as usize);
    
    let mut original_frames = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        original_frames.push(frame);
    }
    
    // Encode with shared palette
    let frame_inputs: Vec<_> = original_frames.iter()
        .map(|f| FrameInput::new(meta.width, meta.height, f.delay, f.pixels.clone()))
        .collect();
    
    let mut output = Vec::new();
    {
        let config = EncoderConfig::new(meta.width, meta.height)
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
    
    // Analyze frame 5 (representative)
    let frame_idx = 5;
    let orig = &original_frames[frame_idx].pixels;
    let reenc = &reencoded_frames[frame_idx].pixels;
    
    println!("Frame {} mismatch analysis ({}x{}):", frame_idx, w, h);
    
    // Find mismatch locations
    let mut mismatch_coords: Vec<(usize, usize)> = Vec::new();
    for i in 0..orig.len() {
        if orig[i].r != reenc[i].r || orig[i].g != reenc[i].g || orig[i].b != reenc[i].b {
            let x = i % w;
            let y = i / w;
            mismatch_coords.push((x, y));
        }
    }
    
    println!("Total mismatches: {}", mismatch_coords.len());
    
    // Bounding box
    if !mismatch_coords.is_empty() {
        let min_x = mismatch_coords.iter().map(|c| c.0).min().unwrap();
        let max_x = mismatch_coords.iter().map(|c| c.0).max().unwrap();
        let min_y = mismatch_coords.iter().map(|c| c.1).min().unwrap();
        let max_y = mismatch_coords.iter().map(|c| c.1).max().unwrap();
        
        println!("Bounding box: ({}, {}) to ({}, {})", min_x, min_y, max_x, max_y);
        println!("Bounding box size: {}x{}", max_x - min_x + 1, max_y - min_y + 1);
        
        // Check if mismatches form edges (adjacent to matching pixels with different colors)
        let mut edge_mismatches = 0;
        for &(x, y) in &mismatch_coords {
            // Check if any neighbor has different color in original
            let idx = y * w + x;
            let orig_color = (orig[idx].r, orig[idx].g, orig[idx].b);
            
            for (dx, dy) in [(-1i32, 0), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx >= 0 && nx < w as i32 && ny >= 0 && ny < h as i32 {
                    let nidx = ny as usize * w + nx as usize;
                    let neighbor_color = (orig[nidx].r, orig[nidx].g, orig[nidx].b);
                    if neighbor_color != orig_color {
                        edge_mismatches += 1;
                        break;
                    }
                }
            }
        }
        println!("Mismatches at color edges: {}/{} ({:.1}%)", 
            edge_mismatches, mismatch_coords.len(),
            100.0 * edge_mismatches as f64 / mismatch_coords.len() as f64);
        
        // Sample mismatches
        println!("\nSample mismatches:");
        for &(x, y) in mismatch_coords.iter().take(10) {
            let idx = y * w + x;
            println!("  ({:3},{:3}): ({:3},{:3},{:3}) -> ({:3},{:3},{:3})",
                x, y, orig[idx].r, orig[idx].g, orig[idx].b,
                reenc[idx].r, reenc[idx].g, reenc[idx].b);
        }
    }
}
