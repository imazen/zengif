//! Encode peak-memory probe — one GIF encode, report measured peak RSS (VmHWM).
//!
//! The ENCODE counterpart to the decode-side probes used by the heaptrack /
//! VmHWM sweep that calibrates each zen codec's encode peak-memory model
//! (`heuristics::estimate_encode`, surfaced as `estimate_encode_resources`)
//! against measured reality, *per effort level*, instead of the current
//! structural guess (`ENCODE_FIXED_OVERHEAD + base_bpp·pixels + quantizer
//! overhead`).
//!
//!   cargo build -p zengif --release --example mem_probe_encode
//!   # default features = ["std", "zenquant"] → zenquant quantizer backend.
//!   GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072 \
//!     ./target/release/examples/mem_probe_encode <rgb8.bin> <w> <h> <effort> <quality>
//!   heaptrack ./target/release/examples/mem_probe_encode ...   # allocator peak heap
//!
//! One encode per process — peak RSS is a per-process high-water mark, so the
//! input must come from a cheap file read (raw RGB8 bin), never an in-process
//! decode (whose own peak would pollute VmHWM above the encode peak).
//!
//! GIF is paletted: the input RGB8 is widened to RGBA (alpha=255) into a single
//! still `FrameInput`, then quantized to a ≤256-colour palette + LZW-compressed.
//! Memory drivers for a single still frame: the RGBA frame buffer (w·h·4), the
//! colour quantizer's working set (histogram / k-means — the dominant term,
//! ~24 B/px + ~1.7 MB fixed for the imagequant-class backends), the LZW
//! dictionary, and the output byte vector. There is no prev-frame/canvas buffer
//! for a single frame (those are animation/decode-side costs).
//!
//! EFFORT AXIS: GIF has no dedicated "effort 0/1/2" dial. The CPU/quantization
//! knob is the quantizer `quality` (u8, 1..=100) plus `dithering` (0.0..=1.0),
//! which drive quantization effort. This probe sweeps `quality` as the effort
//! axis. NOTE: the current estimate model (`estimate_encode`) is INDEPENDENT of
//! quality — its peak depends only on (width, height, frame_count, quantizer
//! type). So memory is NOT expected to vary across quality for a fixed backend;
//! the sweep exists to confirm that empirically and to calibrate the
//! per-quantizer B/px + fixed-overhead constants. (arg4 `effort` is a free label
//! carried into the TSV; arg5 `quality` is the actual u8 fed to the quantizer.)
//!
//! TSV row:
//!   w  h  pixels  mode  effort  quality  out_bytes  pre_rss_kb  vmhwm_kb  marginal_kb
//! `mode` is always `gif`. `marginal_kb = VmHWM − pre_rss` isolates the encode's
//! own working set (what the model predicts).

use enough::Unstoppable;
use std::hint::black_box;
use zengif::{EncoderConfig, FrameInput, Limits, Repeat, Rgba, encode_gif};

/// A `/proc/self/status` field in KiB (e.g. `VmRSS:`, `VmHWM:`).
fn status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("usage: mem_probe_encode <rgb8.bin> <w> <h> <effort> <quality 1..=100> [est]");
        std::process::exit(2);
    }
    let path = &a[1];
    let w: u32 = a[2].parse().expect("w");
    let h: u32 = a[3].parse().expect("h");
    // arg4 is a free-form effort label (carried into the TSV); arg5 is the
    // actual quantizer quality (1..=100). For GIF the "effort" IS the quality —
    // pick representative levels (e.g. low=10, mid=50, max=100) when sweeping.
    let effort: String = a[4].clone();
    let quality: u32 = a[5].parse().expect("quality");
    // GIF dimensions are u16 (format limit). The probe targets stills that fit.
    assert!(
        w >= 1 && w <= u16::MAX as u32 && h >= 1 && h <= u16::MAX as u32,
        "w/h must be in 1..=65535 (GIF u16 dimensions), got {w}x{h}"
    );
    let w16 = w as u16;
    let h16 = h as u16;

    // Estimate-only mode (`est` as a 6th arg): print what the CURRENT model
    // predicts for this cell (typ peak + max), no encode — so we can compare
    // model vs measured without an encode polluting anything.
    //
    // VERIFY: `estimate_encode` takes a `QuantizerType`. The crate's internal
    // `QuantizerType::from_encoder_config` is pub(crate); from an example we
    // can't call it, so we assume the DEFAULT build (features = std+zenquant),
    // whose backend (`zenquant`) maps to the `Imagequant` resource profile
    // (24 B/px + 1.7 MB fixed) in `heuristics::QuantizerType::from_backend`.
    // If the probe is built with a different single quantizer feature, change
    // this `QuantizerType` to match (Quantizr / ColorQuant / None).
    if a.get(6).map(String::as_str) == Some("est") {
        use zengif::heuristics::{QuantizerType, estimate_encode};
        let est = estimate_encode(w, h, 1, QuantizerType::Imagequant);
        let pixels = (w as u64) * (h as u64);
        println!(
            "{w}\t{h}\t{pixels}\t{effort}\t{quality}\tEST\tpeak_kb={}\tmax_kb={}\tpeak_bpp={:.2}\tmax_bpp={:.2}",
            est.peak_memory_bytes / 1024,
            est.peak_memory_bytes_max / 1024,
            est.peak_memory_bytes as f64 / pixels as f64,
            est.peak_memory_bytes_max as f64 / pixels as f64
        );
        return;
    }

    // Read raw RGB8 (w*h*3 bytes) and widen to RGBA (alpha=255).
    let data = std::fs::read(path).expect("read rgb8.bin");
    let px = (w as usize) * (h as usize);
    assert_eq!(
        data.len(),
        px * 3,
        "bin size {} != w*h*3 {}",
        data.len(),
        px * 3
    );
    let pixels: Vec<Rgba> = data
        .chunks_exact(3)
        .map(|c| Rgba::rgb(c[0], c[1], c[2]))
        .collect();

    // Single still frame, delay 0, no loop.
    let frame = FrameInput::new(w16, h16, 0, pixels);

    // Effort = quantizer quality (1..=100). dithering left at the EncoderConfig
    // default (0.5). shared_palette is irrelevant for a single frame (it would
    // buffer 1 frame). Repeat::Once = still image.
    //
    // VERIFY: `.quality(..)` only exists when a quantizer feature is compiled
    // in; default features include `zenquant` so it's present. If you build
    // with NO quantizer feature, drop the `.quality()` call (encode would also
    // fail — GIF needs a quantizer).
    let config = EncoderConfig::new()
        .repeat(Repeat::Once)
        .quality(quality.clamp(1, 100) as u8);

    // Baseline RSS: process + libs + the `data`/`pixels` we hold. Marginal =
    // VmHWM − pre isolates the encode's own working set (what the model
    // predicts). NOTE: the widened `pixels` Vec was moved into `frame`, so the
    // RGBA frame buffer is counted in the encode working set (it lives until
    // `encode_gif` consumes it), which is the honest accounting for a one-shot
    // encode-from-RGBA API.
    let pre = status_kb("VmRSS:");

    let out =
        encode_gif(vec![frame], w16, h16, config, Limits::none(), &Unstoppable).expect("encode");

    // High-water mark immediately after encode — VmHWM is monotonic, so it
    // reflects the peak *during* the encode.
    let peak = status_kb("VmHWM:");

    let pixels_n = (w as u64) * (h as u64);
    println!(
        "{w}\t{h}\t{pixels_n}\tgif\t{effort}\t{quality}\t{}\t{pre}\t{peak}\t{}",
        out.len(),
        peak.saturating_sub(pre)
    );
    black_box(&out);
}
