//! Empirical validation of the curated sweep axes (`zengif::sweep`) —
//! playbook patterns 6 + 14 + 15 (`zenjpeg/docs/VARIANT_GENERATION.md`).
//!
//! Gates: every cell must encode AND decode (pattern 14) with matching
//! dimensions and exactly one frame; every curated step must change
//! output bytes vs the default stratum somewhere (liveness). GIF is
//! quantizer-lossy, so there is no exactness gate — but the palette-
//! friendly corpus leg (≤256 distinct colors) must roundtrip pixels
//! EXACTLY at quality 100 with dithering 0 (the quantizer has nothing
//! to lose there; if that drifts, a backend is broken, not "lossy").
//! Corpus: palette bands / noise / odd 509×381 / tiny (pattern 15:
//! odd dims exercise width-edge paths).

// Mirrors the lib's gating: `zengif::sweep` needs a quantizer backend,
// `encode_gif`/`decode_gif` need std.
#![cfg(all(
    feature = "std",
    any(
        feature = "zenquant",
        feature = "quantette",
        feature = "imagequant",
        feature = "quantizr",
        feature = "color_quant"
    )
))]

use zengif::sweep::{QualityGrid, SweepAxes, plan};
use zengif::{FrameInput, Limits, Rgba, decode_gif, encode_gif};

fn px(r: u8, g: u8, b: u8) -> Rgba {
    Rgba { r, g, b, a: 255 }
}

fn bands(w: usize, h: usize) -> Vec<Rgba> {
    let palette = [
        px(220, 50, 47),
        px(38, 139, 210),
        px(133, 153, 0),
        px(181, 137, 0),
        px(42, 161, 152),
        px(253, 246, 227),
    ];
    (0..w * h)
        .map(|i| palette[((i % w) / 17 + (i / w) / 23) % palette.len()])
        .collect()
}

fn noise(w: usize, h: usize, mut state: u32) -> Vec<Rgba> {
    (0..w * h)
        .map(|_| {
            let mut n = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            };
            px(n(), n(), n())
        })
        .collect()
}

struct Image {
    name: &'static str,
    w: u16,
    h: u16,
    pixels: Vec<Rgba>,
}

fn corpus() -> Vec<Image> {
    vec![
        Image {
            name: "bands256",
            w: 256,
            h: 256,
            pixels: bands(256, 256),
        },
        Image {
            name: "noise256",
            w: 256,
            h: 256,
            pixels: noise(256, 256, 0x9e37_79b9),
        },
        Image {
            name: "odd509x381",
            w: 509,
            h: 381,
            pixels: bands(509, 381),
        },
        Image {
            name: "tiny48",
            w: 48,
            h: 48,
            pixels: noise(48, 48, 0x1234_5678),
        },
    ]
}

fn encode_cell(v: &zengif::sweep::SweepVariant, img: &Image) -> Vec<u8> {
    let frame = FrameInput::new(img.w, img.h, 0, img.pixels.clone());
    encode_gif(
        vec![frame],
        img.w,
        img.h,
        v.build(),
        Limits::none(),
        &enough::Unstoppable,
    )
    .unwrap_or_else(|e| panic!("encode failed on {}: {e:?}", img.name))
}

#[test]
fn sweep_cells_decode_and_steps_are_live() {
    let p = plan(
        &SweepAxes::modes_full(),
        &QualityGrid::Explicit(vec![10, 50, 85]),
    );
    assert!(p.cells[0].id.starts_with("gif-"));
    assert_eq!(p.cells[0].deviations, 0);
    let images = corpus();
    let mut failures: Vec<String> = Vec::new();

    let subset: Vec<usize> = p
        .cells
        .iter()
        .enumerate()
        .take_while(|(_, c)| c.deviations <= 1)
        .map(|(i, _)| i)
        .collect();

    // bytes[cell][image]
    let mut bytes: Vec<Vec<usize>> = vec![Vec::new(); p.cells.len()];
    for &ci in &subset {
        let cell = &p.cells[ci];
        for img in &images {
            let gif = encode_cell(&cell.variant, img);
            // Pattern 14: every cell must decode.
            match decode_gif(&gif, Limits::none(), &enough::Unstoppable) {
                Ok((meta, frames, _stats)) => {
                    if (meta.width, meta.height) != (img.w, img.h) {
                        failures.push(format!(
                            "DIMS: {} on {}: {}x{}",
                            cell.id, img.name, meta.width, meta.height
                        ));
                    }
                    if frames.len() != 1 {
                        failures.push(format!(
                            "FRAMES: {} on {}: {}",
                            cell.id,
                            img.name,
                            frames.len()
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "UNDECODABLE CELL: {} on {}: {e:?}",
                    cell.id, img.name
                )),
            }
            bytes[ci].push(gif.len());
        }
    }

    // Liveness: every dev-1 stratum must differ from the default stratum
    // somewhere (compare per-q rows: cells are (stratum × q) flattened —
    // match on the q suffix).
    let q_of = |id: &str| id.rsplit_once("_q").map(|(_, q)| q.to_string());
    for &ci in &subset {
        let c = &p.cells[ci];
        if c.deviations != 1 {
            continue;
        }
        let q = q_of(&c.id).unwrap();
        let base = subset.iter().find(|&&bi| {
            p.cells[bi].deviations == 0 && q_of(&p.cells[bi].id).as_deref() == Some(q.as_str())
        });
        let Some(&bi) = base else { continue };
        if bytes[ci] == bytes[bi] {
            failures.push(format!(
                "INERT STEP: {} byte-matched the default stratum on every image",
                c.id
            ));
        }
    }

    // Exactness on representable content: ≤256-color bands at q100/d0
    // must roundtrip pixel-exact through every compiled backend.
    for backend in zengif::sweep::compiled_backends() {
        let v = zengif::sweep::SweepVariant {
            quality: 100,
            dithering: 0.0,
            backend,
        };
        let img = &images[0]; // bands256: 6 distinct colors
        let gif = encode_cell(&v, img);
        let (_, frames, _) = decode_gif(&gif, Limits::none(), &enough::Unstoppable).unwrap();
        let decoded = &frames[0].pixels;
        if decoded.len() != img.pixels.len()
            || decoded
                .iter()
                .zip(&img.pixels)
                .any(|(a, b)| (a.r, a.g, a.b) != (b.r, b.g, b.b))
        {
            failures.push(format!(
                "PALETTE-REPRESENTABLE ROUNDTRIP MISMATCH: backend {backend:?} at q100/d0"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} hard failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
