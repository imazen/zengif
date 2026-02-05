use std::collections::HashSet;
use std::fs;
use zengif::{Decoder, Limits, Rgba, Unstoppable};

fn main() {
    let test_dir = "/tmp/gif-testset";
    
    for name in ["cat_typing", "spinner"] {
        let path = fs::read_dir(test_dir).unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().contains(name))
            .unwrap().path();
        
        let data = fs::read(&path).unwrap();
        let cursor = std::io::Cursor::new(&data);
        let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
        
        let mut all_colors: HashSet<u32> = HashSet::new();
        let mut frame_colors: Vec<usize> = Vec::new();
        
        while let Some(frame) = decoder.next_frame().unwrap() {
            let colors: HashSet<u32> = frame.pixels.iter()
                .map(|p| ((p.r as u32) << 16) | ((p.g as u32) << 8) | (p.b as u32))
                .collect();
            frame_colors.push(colors.len());
            all_colors.extend(colors);
        }
        
        println!("{}: {} total unique colors across {} frames", 
            path.file_name().unwrap().to_string_lossy(),
            all_colors.len(),
            frame_colors.len());
        
        let min_colors = frame_colors.iter().min().unwrap();
        let max_colors = frame_colors.iter().max().unwrap();
        let avg_colors: f64 = frame_colors.iter().sum::<usize>() as f64 / frame_colors.len() as f64;
        
        println!("  Per-frame colors: min={}, max={}, avg={:.1}", min_colors, max_colors, avg_colors);
        println!("  Frame color counts: {:?}", &frame_colors[..frame_colors.len().min(10)]);
        println!();
    }
}
