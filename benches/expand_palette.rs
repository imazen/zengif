//! Palette-index -> RGBA expansion: the hot loop of frame compositing.
//!
//! `expand_palette_row` runs once per row per frame. Its doc comment claims the
//! fixed-size 16-pixel chunks let LLVM "unroll or vectorize the inner loop",
//! but the body is a 256-entry LUT gather and AArch64 has NO gather
//! instruction, so on ARM it cannot vectorize — only unroll. This measures
//! what it actually achieves.
//!
//! Palette size is swept because real GIFs are mostly far below 256 colors,
//! and index locality changes the LUT's cache behaviour. Run structure is
//! swept because GIF content is highly run-structured (flat regions), which
//! changes the load pattern.

use zenbench::prelude::*;
use zengif::__bench_expand as k;
use zengif::Rgba;

fn lut() -> [Rgba; 256] {
    let mut l = [Rgba::default(); 256];
    for (i, e) in l.iter_mut().enumerate() {
        let i = i as u8;
        *e = Rgba {
            r: i.wrapping_mul(7),
            g: i.wrapping_mul(13),
            b: i.wrapping_mul(29),
            a: 255,
        };
    }
    l
}

/// `ncolors` distinct indices; `run` = mean run length (1 = pure noise).
fn indices(n: usize, ncolors: u32, run: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let mut s = 0x9e37_79b9u32;
    while v.len() < n {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let idx = ((s >> 16) % ncolors) as u8;
        for _ in 0..run.max(1) {
            if v.len() == n {
                break;
            }
            v.push(idx);
        }
    }
    v
}

fn bench_expand(suite: &mut Suite) {
    // One 1920-wide row is the realistic call granularity; 4096 px shows the
    // steady-state rate without per-call overhead dominating.
    for &(wlabel, w) in &[("row1920", 1920usize), ("px4096", 4096)] {
        for &(clabel, ncolors, run) in &[
            ("pal16_noise", 16u32, 1usize),
            ("pal16_runs8", 16, 8),
            ("pal64_noise", 64, 1),
            ("pal256_noise", 256, 1),
            ("pal256_runs8", 256, 8),
        ] {
            let idx: &'static [u8] = Box::leak(indices(w, ncolors, run).into_boxed_slice());
            let l: &'static [Rgba; 256] = Box::leak(Box::new(lut()));

            suite.compare(format!("expand/{wlabel}/{clabel}"), |g| {
                // 4 bytes written per pixel.
                g.throughput(Throughput::Bytes((w * 4) as u64));
                g.bench("opaque", move |b| {
                    let mut canvas = vec![Rgba::default(); w];
                    b.iter(move || k::opaque(&mut canvas, idx, l))
                });
                g.bench("transparent", move |b| {
                    let mut canvas = vec![Rgba::default(); w];
                    b.iter(move || k::transparent(&mut canvas, idx, l, 0))
                });
            });
        }
    }
}

zenbench::main!(bench_expand);
