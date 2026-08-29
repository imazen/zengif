//! Which `GifError` variant a caller actually receives.
//!
//! `MalformedLzw` is the variant a caller matches on for "this GIF's compressed
//! image data is corrupt" — the single most likely way a damaged GIF fails. It
//! was never constructed: `From<gif::DecodingError>` routed
//! `DecodingError::LzwError` to the opaque `GifCrate` catch-all, so callers had
//! to string-match the message to tell corruption apart from anything else the
//! `gif` crate reports.

#![cfg(feature = "std")]

use enough::Unstoppable;
use zengif::{GifError, Limits, decode_gif};

/// A 4×4 GIF whose LZW stream emits `code` right after the clear code.
///
/// With a 2-bit minimum code size the clear code is 4, end-of-information is 5,
/// and 6 is the first code the dictionary can define. Anything at or above 6
/// immediately after a clear refers to a dictionary entry that does not exist
/// yet, which is exactly weezl's `InvalidCode`.
fn gif_with_lzw_code(code: u32) -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(b"GIF89a");
    g.extend_from_slice(&4u16.to_le_bytes());
    g.extend_from_slice(&4u16.to_le_bytes());
    g.push(0x80); // global colour table, 2 entries
    g.push(0);
    g.push(0);
    g.extend_from_slice(&[0xFF, 0x00, 0x00]);
    g.extend_from_slice(&[0x00, 0x00, 0xFF]);

    g.push(0x2C); // image descriptor
    g.extend_from_slice(&0u16.to_le_bytes()); // left
    g.extend_from_slice(&0u16.to_le_bytes()); // top
    g.extend_from_slice(&4u16.to_le_bytes()); // width
    g.extend_from_slice(&4u16.to_le_bytes()); // height
    g.push(0x00); // no local colour table

    g.push(0x02); // LZW minimum code size
    let mut bits: Vec<u8> = Vec::new();
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    for c in [4u32, code, code, code] {
        acc |= c << nbits;
        nbits += 3;
        while nbits >= 8 {
            bits.push((acc & 0xFF) as u8);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        bits.push((acc & 0xFF) as u8);
    }
    g.push(bits.len() as u8);
    g.extend_from_slice(&bits);
    g.push(0x00); // block terminator
    g.push(0x3B); // trailer
    g
}

/// **The regression.** Corrupt LZW data must surface as `MalformedLzw`, not as
/// the opaque `GifCrate` catch-all.
#[test]
fn corrupt_lzw_reports_malformed_lzw() {
    // 6 and 7 are both beyond the dictionary immediately after a clear code.
    for code in [6u32, 7] {
        let data = gif_with_lzw_code(code);
        let err = decode_gif(&data, Limits::none(), &Unstoppable)
            .map(|_| ())
            .expect_err("a GIF with an invalid LZW code must not decode");
        assert!(
            matches!(err.error(), GifError::MalformedLzw { .. }),
            "code {code}: expected MalformedLzw, got {err:?}"
        );
    }
}

/// The category the zencodec layer reports must be unchanged by the reroute —
/// `MalformedLzw` and `GifCrate` both mean "corrupt bitstream", so consumers
/// that switch on the category see exactly what they saw before.

#[test]
fn malformed_lzw_keeps_the_malformed_image_category() {
    use zencodec::{CategorizedError, ErrorCategory, ImageError};
    assert_eq!(
        GifError::MalformedLzw { message: "x" }.category(),
        ErrorCategory::Image(ImageError::Malformed)
    );
    assert_eq!(
        GifError::GifCrate {
            message: "x".to_string()
        }
        .category(),
        ErrorCategory::Image(ImageError::Malformed)
    );
}

/// A well-formed GIF must still decode — the reroute must not turn valid LZW
/// into an error.
#[test]
fn valid_gif_still_decodes() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/corpus/codec-corpus/sample_1.gif"
    ))
    .expect("corpus file");
    decode_gif(&data, Limits::none(), &Unstoppable).expect("a valid GIF must decode");
}
