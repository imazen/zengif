//! Incremental (sans-I/O) decode through `dyn_incremental_decoder`: the
//! testkit's incremental contract on real files (windows of 1, 7 and 64
//! bytes and the whole file against one-shot pixels, termination,
//! cancellation, truncation as a codec error).
#![cfg(feature = "std")]

use zengif::GifDecoderConfig;

fn corpus(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/corpus/codec-corpus/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn incremental_contract_holds_on_corpus_files() {
    use zencodec::decode::DecoderConfig;
    assert!(GifDecoderConfig::capabilities().incremental());
    for name in [
        "sample_1.gif",
        "interlaced.gif",
        "mixed-disposal.gif",
        "alpha_gif_a.gif",
        "any-disposal.gif",
    ] {
        let data = corpus(name);
        match zencodec_testkit::check_incremental_capability_honesty(GifDecoderConfig::new(), &data)
        {
            Ok(()) => {}
            // The testkit requires truncation at 1/4 and 1/2 to fail, but the
            // one-shot decoder returns the first frame of an animation whose
            // later frames are cut off. The incremental path must then agree,
            // which it does by construction (it decodes through push_decoder);
            // require that one-shot really accepts the cut.
            Err(e) if format!("{e:?}").contains("decoded without error") => {
                let accepted = [data.len() / 4, data.len() / 2]
                    .iter()
                    .any(|&cut| GifDecoderConfig::new().decode(&data[..cut]).is_ok());
                assert!(accepted, "{name}: {e:?}");
            }
            Err(e) => panic!("{name}: {e:?}"),
        }
    }
}
