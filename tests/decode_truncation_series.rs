//! Conformance: feed a known-good GIF, truncate it at a deterministic series of
//! prefixes, decode each through the dyn-erased path, and assert every resulting
//! `ErrorCategory` is in the incomplete-input set — never a panic / OOM / Internal.
//!
//! Driven by `zencodec_testkit::check_decode_truncation_series` (zencodec PR #112).
#![cfg(feature = "std")]

use zengif::GifDecoderConfig;

/// A complete, valid 1x1 GIF89a bitstream (header, global color table, image
/// descriptor, LZW data, block terminator, trailer). Decode-only source so the
/// check does not depend on which quantizer/encoder feature is enabled.
const VALID_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
    0x01, 0x00, 0x01, 0x00, // 1x1
    0x80, // Global color table flag, 2 colors
    0x00, // Background color index
    0x00, // Pixel aspect ratio
    0xFF, 0x00, 0x00, // Color 0: Red
    0x00, 0x00, 0x00, // Color 1: Black
    0x2C, // Image descriptor
    0x00, 0x00, 0x00, 0x00, // Left, Top
    0x01, 0x00, 0x01, 0x00, // Width, Height
    0x00, // No local color table
    0x02, // LZW minimum code size
    0x02, // Block size
    0x44, 0x01, // LZW data
    0x00, // Block terminator
    0x3B, // Trailer
];

#[test]
fn truncation_series_categorizes_as_incomplete_input() {
    // Sanity: the fixture must decode cleanly at full length.
    zengif::GifDecoderConfig::new()
        .decode(VALID_GIF)
        .expect("fixture must be a valid, fully-decodable GIF");

    zencodec_testkit::check_decode_truncation_series(GifDecoderConfig::new(), VALID_GIF)
        .expect("truncated input must categorize as incomplete, never panic/OOM/Internal");
}
