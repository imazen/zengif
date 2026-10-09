//! Fuzz target for the structural inventory (`DecodeJob::inventory`).
//!
//! The walk must never panic, never error on malformed input, and always
//! produce an inventory that `validate()`s: top-level parts tile the input,
//! children nest inside parents, declared bodies are fully covered.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zencodec::decode::{DecodeJob, DecoderConfig};
use zengif::GifDecoderConfig;

fuzz_target!(|data: &[u8]| {
    let inv = GifDecoderConfig::new()
        .job()
        .inventory(data)
        .expect("inventory must not fail on malformed input")
        .expect("zengif declares inventory support");
    assert_eq!(inv.input_len(), data.len() as u64);
    inv.validate().expect("inventory must validate");
});
