//! Shared GIF fixtures for the inventory tests (built in code, never committed as binaries).
#![allow(dead_code)]

/// Minimal LZW image data for a 1x1 frame: min code size 2, one sub-block, terminator.
pub const LZW_1X1: &[u8] = &[0x02, 0x02, 0x44, 0x01, 0x00];

pub fn app_ext(ident: &[u8; 11], sub_blocks: &[&[u8]]) -> Vec<u8> {
    let mut v = vec![0x21, 0xFF, 0x0B];
    v.extend_from_slice(ident);
    for sb in sub_blocks {
        v.push(sb.len() as u8);
        v.extend_from_slice(sb);
    }
    v.push(0);
    v
}

/// The XMP extension: identifier, raw packet, then the 258-byte magic trailer.
pub fn xmp_ext(packet: &[u8]) -> Vec<u8> {
    let mut v = vec![0x21, 0xFF, 0x0B];
    v.extend_from_slice(b"XMP DataXMP");
    v.extend_from_slice(packet);
    v.push(0x01);
    v.extend((1..=255u8).rev());
    v.extend_from_slice(&[0x00, 0x00]);
    v
}

/// Every unit type GIF defines, plus an unknown extension label, an unknown
/// application extension, an unknown block introducer and trailing junk.
pub fn fixture() -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&[2, 0, 1, 0, 0x80, 0, 0]); // 2x1, global table of 2
    g.extend_from_slice(&[255, 0, 0, 0, 0, 255]); // global colour table
    g.extend(app_ext(b"NETSCAPE2.0", &[&[1, 3, 0]]));
    g.extend(app_ext(b"ANIMEXTS1.0", &[&[1, 2, 0]]));
    g.extend_from_slice(&[0x21, 0xFE, 3, b'h', b'e', b'l', 2, b'l', b'o', 0]); // comment
    g.extend_from_slice(&[
        0x21, 0x01, 12, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 2, b'H', b'i', 0,
    ]);
    g.extend(app_ext(b"ACMEPRIV1.0", &[&[0xAA, 0xBB]]));
    g.extend_from_slice(&[0x21, 0x99, 1, 0xAA, 0]); // unknown extension label
    g.extend(xmp_ext(
        b"<?xpacket begin='x'?><x:xmpmeta/><?xpacket end='w'?>",
    ));
    g.extend(app_ext(b"ICCRGBG1012", &[&[1, 2, 3, 4]]));
    g.extend_from_slice(&[0x21, 0xF9, 4, 0x09, 10, 0, 1, 0]); // GCE: disposal 2, delay 10, transp 1
    // Frame 1 with a local colour table.
    g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x80]);
    g.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    g.extend_from_slice(LZW_1X1);
    g.extend_from_slice(&[0x21, 0xF9, 4, 0x00, 5, 0, 0, 0]); // GCE: delay 5
    // Frame 2 without one.
    g.extend_from_slice(&[0x2C, 1, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    g.extend_from_slice(LZW_1X1);
    g.extend_from_slice(&[0x05, 0x02, 0xAA, 0xBB, 0x00]); // unknown introducer
    g.push(0x3B);
    g.extend_from_slice(b"TRAILING!!");
    g
}
