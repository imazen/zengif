//! Analyze file size impact of different thresholds.

use std::fs;
use zengif::{
    decode_gif, EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Unstoppable,
};

fn encode_with_threshold(orig_data: &[u8], threshold: Option<f32>) -> (usize, String) {
    let (meta, orig_frames, _) = decode_gif(orig_data, Limits::none(), Unstoppable).unwrap();
    let inputs: Vec<_> = orig_frames
        .iter()
        .map(|f| FrameInput::new(meta.width, meta.height, 10, f.pixels.clone()))
        .collect();

    let output = {
        let config = EncoderConfig::new()
            .repeat(Repeat::Infinite)
            .shared_palette(true)
            .palette_error_threshold(threshold);
        let limits = Limits::none();
        let mut enc = EncodeRequest::new(&config, 4, 4)
            .limits(&limits)
            .stop(&Unstoppable)
            .build()
            .unwrap();
        for inp in &inputs {
            enc.add_frame(inp.clone()).unwrap();
        }
        enc.finish().unwrap()
    };

    let label = threshold.map_or("∞".to_string(), |t| format!("{}", t as i32));
    (output.len(), label)
}

fn main() {
    let test_dir = "/tmp/gif-testset";
    let mut entries: Vec<_> = fs::read_dir(test_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "gif"))
        .collect();
    entries.sort_by_key(|e| e.path());

    println!(
        "{:<40} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "GIF", "orig", "T=∞", "T=15", "T=10", "T=5", "T=2"
    );
    println!("{}", "-".repeat(95));

    let thresholds = [None, Some(15.0f32), Some(10.0), Some(5.0), Some(2.0)];

    for entry in &entries {
        let path = entry.path();
        let name = path.file_stem().unwrap().to_string_lossy();
        let data = fs::read(&path).unwrap();
        let orig_size = data.len();

        let sizes: Vec<_> = thresholds
            .iter()
            .map(|&t| encode_with_threshold(&data, t).0)
            .collect();

        // Only show if there's variation
        let all_same = sizes.iter().all(|&s| s == sizes[0]);
        let marker = if all_same { " " } else { "*" };

        println!(
            "{:<40} {:>6}K {:>6}K {:>6}K {:>6}K {:>6}K {:>6}K{}",
            &name[..name.len().min(40)],
            orig_size / 1024,
            sizes[0] / 1024,
            sizes[1] / 1024,
            sizes[2] / 1024,
            sizes[3] / 1024,
            sizes[4] / 1024,
            marker
        );
    }
}
