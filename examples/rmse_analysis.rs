//! Analyze RMSE distribution across frames - matches encoder's compute_remap_rmse.

use std::fs;
use zengif::{
    decode_gif, EncodeRequest, EncoderConfig, FrameInput, Limits, Repeat, Rgba, Unstoppable,
};

fn compute_frame_rmse(orig: &[Rgba], enc: &[Rgba]) -> f64 {
    if orig.len() != enc.len() || orig.is_empty() {
        return 0.0;
    }
    let mut total = 0u64;
    let mut count = 0u64;
    for (o, e) in orig.iter().zip(enc.iter()) {
        if o.a == 0 {
            continue;
        } // Skip transparent pixels
        let dr = o.r as i64 - e.r as i64;
        let dg = o.g as i64 - e.g as i64;
        let db = o.b as i64 - e.b as i64;
        total += (dr * dr + dg * dg + db * db) as u64;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((total as f64) / (count as f64)).sqrt() // Same as encoder's formula
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
        "{:<42} {:>3}f  {:>5} {:>5} {:>5} {:>5}  >15 >10  >5  >2",
        "GIF", "", "max", "avg", "p90", "p50"
    );
    println!("{}", "-".repeat(90));

    for entry in &entries {
        let path = entry.path();
        let name = path.file_stem().unwrap().to_string_lossy();
        let data = fs::read(&path).unwrap();

        let (meta, orig_frames, _) = decode_gif(&data, Limits::none(), &Unstoppable).unwrap();
        if orig_frames.is_empty() {
            continue;
        }

        // Encode with shared palette (no fallback) and decode
        let inputs: Vec<_> = orig_frames
            .iter()
            .map(|f| FrameInput::new(meta.width, meta.height, 10, f.pixels.clone()))
            .collect();

        let output = {
            let config = EncoderConfig::new()
                .repeat(Repeat::Infinite)
                .shared_palette(true)
                .palette_error_threshold(None);
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

        let (_, enc_frames, _) = decode_gif(&output, Limits::none(), &Unstoppable).unwrap();

        // Compute per-frame RMSE
        let mut rmses: Vec<f64> = Vec::new();
        for (orig, enc) in orig_frames.iter().zip(enc_frames.iter()) {
            rmses.push(compute_frame_rmse(&orig.pixels, &enc.pixels));
        }

        // Stats
        rmses.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let max_rmse = *rmses.last().unwrap_or(&0.0);
        let avg_rmse = rmses.iter().sum::<f64>() / rmses.len().max(1) as f64;
        let p90 = rmses.get(rmses.len() * 90 / 100).copied().unwrap_or(0.0);
        let p50 = rmses.get(rmses.len() / 2).copied().unwrap_or(0.0);
        let exceed_15 = rmses.iter().filter(|&&r| r > 15.0).count();
        let exceed_10 = rmses.iter().filter(|&&r| r > 10.0).count();
        let exceed_5 = rmses.iter().filter(|&&r| r > 5.0).count();
        let exceed_2 = rmses.iter().filter(|&&r| r > 2.0).count();

        println!(
            "{:<42} {:>3}f  {:>5.1} {:>5.1} {:>5.1} {:>5.1}  {:>3} {:>3} {:>3} {:>3}",
            &name[..name.len().min(42)],
            orig_frames.len(),
            max_rmse,
            avg_rmse,
            p90,
            p50,
            exceed_15,
            exceed_10,
            exceed_5,
            exceed_2
        );
    }
}
