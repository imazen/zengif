//! Cross-check of the GIF inventory against `exiftool -v3` (an independent
//! implementation), run when `INVENTORY_ORACLE_EXIFTOOL` names the binary
//! (`just inventory-oracle` sets it).
//!
//! `exiftool -v3` prints one line per block (`Application Extension:`,
//! `Graphic Control:`, `Image:`, `Text:`, comment text) but no byte offsets.
//! The check therefore has two halves: the sequence of units exiftool reports
//! must equal the sequence of top-level blocks in the inventory, and the
//! values exiftool prints must equal the values read from the bytes at the
//! inventory's offsets, which pins each unit's offset and length. Coverage
//! of every other byte is the inventory's own `validate` guarantee.
#![cfg(feature = "inventory")]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use zencodec::decode::{DecodeJob, DecoderConfig};
use zencodec::inventory::{Inventory, PartKind, PartTag};
use zengif::GifDecoderConfig;

#[derive(Debug, PartialEq, Eq)]
enum Ev {
    /// `NETSCAPE/2.0`: 8-byte name, `/`, 3-byte authentication code, control bytes removed.
    App(String),
    /// Delay in centiseconds.
    Gce(u32),
    Image(u16, u16, u16, u16),
    Text(u16, u16, u16, u16),
    Comment(String),
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// Events exiftool reports, in file order.
fn exiftool_events(bin: &str, path: &Path) -> Vec<Ev> {
    let out = Command::new(bin)
        .args(["-v3"])
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {bin}: {e}"));
    assert!(out.status.success(), "{bin} failed on {}", path.display());
    let text = String::from_utf8_lossy(&out.stdout);
    let nums = |s: &str| -> Vec<u16> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|t| !t.is_empty())
            .map(|t| t.parse().unwrap())
            .collect()
    };
    let mut evs = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Application Extension: ") {
            evs.push(Ev::App(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix("Graphic Control: delay=") {
            let secs: f64 = rest.trim().parse().unwrap();
            evs.push(Ev::Gce((secs * 100.0).round() as u32));
        } else if let Some(rest) = line.strip_prefix("Image: ") {
            let n = nums(rest);
            evs.push(Ev::Image(n[0], n[1], n[2], n[3]));
        } else if let Some(rest) = line.strip_prefix("Text: ") {
            let n = nums(rest);
            evs.push(Ev::Text(n[0], n[1], n[2], n[3]));
        } else if let Some(rest) = line.strip_prefix("  Comment = ") {
            evs.push(Ev::Comment(rest.to_string()));
        }
    }
    evs
}

/// The same events, read from the bytes at the inventory's top-level offsets.
fn inventory_events(data: &[u8], inv: &Inventory) -> Vec<Ev> {
    let mut evs = Vec::new();
    for p in inv.parts().iter().filter(|p| p.parent().is_none()) {
        if p.kind() != PartKind::Block {
            continue;
        }
        let r = p.range().start as usize..p.range().end as usize;
        let b = &data[r];
        match p.tag() {
            // exiftool reports application extensions whose first block is 11 bytes.
            PartTag::Code(0xFF) if b.len() >= 14 && b[2] == 0x0B => {
                assert_walk_ends_at_range(b, "application extension");
                let mut name = String::new();
                for (i, &c) in b[3..14].iter().enumerate() {
                    if i == 8 {
                        name.push('/');
                    }
                    if c >= 0x20 {
                        name.push(c as char);
                    }
                }
                evs.push(Ev::App(name));
            }
            // ... graphic control extensions with a 4-byte block.
            PartTag::Code(0xF9) if b.len() >= 8 && b[2] == 4 => {
                assert_walk_ends_at_range(b, "graphic control");
                evs.push(Ev::Gce(u32::from(le16(b, 4))));
            }
            PartTag::Code(0x2C) => {
                assert_eq!(b.len(), 10, "image descriptor is 10 bytes");
                evs.push(Ev::Image(le16(b, 1), le16(b, 3), le16(b, 5), le16(b, 7)));
            }
            PartTag::Code(0x01) if b.len() >= 15 && b[2] == 12 => {
                assert_walk_ends_at_range(b, "plain text");
                evs.push(Ev::Text(le16(b, 3), le16(b, 5), le16(b, 7), le16(b, 9)));
            }
            PartTag::Code(0xFE) => {
                assert_walk_ends_at_range(b, "comment");
                let mut text = Vec::new();
                let mut at = 2;
                while let Some(&n) = b.get(at) {
                    if n == 0 {
                        break;
                    }
                    text.extend_from_slice(&b[at + 1..at + 1 + usize::from(n)]);
                    at += 1 + usize::from(n);
                }
                if !text.is_empty() {
                    evs.push(Ev::Comment(String::from_utf8_lossy(&text).into_owned()));
                }
            }
            _ => {}
        }
    }
    evs
}

/// The unit's own sub-block walk (from index 2) must end exactly at the part's
/// end: pins the length of the unit with no tool-reported offsets.
fn assert_walk_ends_at_range(b: &[u8], what: &str) {
    let mut at = 2;
    while let Some(&n) = b.get(at) {
        if n == 0 {
            break;
        }
        at += 1 + usize::from(n);
    }
    assert_eq!(
        at + 1,
        b.len(),
        "{what}: sub-block walk ends at {at}, part is {} bytes",
        b.len()
    );
}

fn gifs_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(gifs_in(&p));
        } else if p.extension().is_some_and(|x| x == "gif") {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn inventory_agrees_with_exiftool() {
    // Decided by the caller (justfile / CI job that has exiftool installed).
    let Some(bin) = std::env::var_os("INVENTORY_ORACLE_EXIFTOOL") else {
        return;
    };
    let bin = bin.to_str().expect("utf-8 path").to_string();

    let conformance = codec_corpus::Corpus::new()
        .expect("codec-corpus")
        .get("gif-conformance")
        .expect("gif-conformance");
    let mut files = gifs_in(&conformance.join("valid"));
    files.extend(gifs_in(&conformance.join("edge-cases")));
    files.extend(gifs_in(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus"),
    ));
    assert!(files.len() >= 50, "only {} files", files.len());

    println!("{:<48} {:>7} {:>7} result", "file", "units", "bytes");
    let mut checked = 0usize;
    let mut units = 0usize;
    let mut failures = Vec::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let inv = GifDecoderConfig::new().job().inventory(&data).unwrap();
        inv.validate().unwrap();
        let ours = inventory_events(&data, &inv);
        let theirs = exiftool_events(&bin, f);
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let ok = ours == theirs;
        println!(
            "{name:<48} {:>7} {:>7} {}",
            theirs.len(),
            data.len(),
            if ok { "match" } else { "DIFFERS" }
        );
        if !ok {
            failures.push(format!(
                "{}:\n  ours   {ours:?}\n  theirs {theirs:?}",
                f.display()
            ));
        }
        checked += 1;
        units += theirs.len();
    }
    println!("{checked} files, {units} exiftool-reported units");

    // The synthetic fixture (every unit type, private and unknown units,
    // trailing junk) goes through the same comparison.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let fixture_path = dir.join("inventory-fixture.gif");
    let data = common::fixture();
    std::fs::write(&fixture_path, &data).unwrap();
    let inv = GifDecoderConfig::new().job().inventory(&data).unwrap();
    let (ours, theirs) = (
        inventory_events(&data, &inv),
        exiftool_events(&bin, &fixture_path),
    );
    println!(
        "fixture: {} units, {}",
        theirs.len(),
        if ours == theirs { "match" } else { "DIFFERS" }
    );
    if ours != theirs {
        failures.push(format!("fixture:\n  ours   {ours:?}\n  theirs {theirs:?}"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn oracle_helpers_read_a_known_file() {
    // Not gated on exiftool: checks inventory_events on a hand-built file.
    let mut g = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
    g.extend_from_slice(&[0x21, 0xF9, 4, 0, 7, 0, 0, 0]);
    g.extend_from_slice(&[0x21, 0xFE, 2, b'h', b'i', 0]);
    g.extend_from_slice(&[
        0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3B,
    ]);
    let inv = GifDecoderConfig::new().job().inventory(&g).unwrap();
    assert_eq!(
        inventory_events(&g, &inv),
        vec![Ev::Gce(7), Ev::Comment("hi".into()), Ev::Image(0, 0, 1, 1)]
    );
}
