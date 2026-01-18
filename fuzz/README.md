# zengif Fuzzing

This directory contains fuzzing infrastructure for zengif using cargo-fuzz (libFuzzer).

## Quick Start

```bash
# Install cargo-fuzz if not already installed
cargo install cargo-fuzz

# Run the main decode fuzzer
cargo fuzz run fuzz_decode

# Run with the GIF dictionary for smarter mutations
cargo fuzz run fuzz_decode -- -dict=fuzz/gif.dict

# Run with seed corpus
cargo fuzz run fuzz_decode fuzz/corpus/seed/
```

## Fuzz Targets

| Target | Description | Coverage Focus |
|--------|-------------|----------------|
| `fuzz_decode` | Full decode via `decode_gif()` | Header parsing, LZW, compositing |
| `fuzz_decode_streaming` | Streaming decode via `Decoder` | Frame iteration, state machine |
| `fuzz_roundtrip` | Encode → Decode consistency | Encoder, palette handling |
| `fuzz_limits` | Limits enforcement with arbitrary configs | Memory bounds, dimension checks |

## Building a Larger Corpus

Download additional corpus files from public sources:

```bash
./fuzz/download_corpus.sh
cargo fuzz run fuzz_decode fuzz/corpus/merged/
```

Sources include:
- [dvyukov/go-fuzz-corpus](https://github.com/dvyukov/go-fuzz-corpus/tree/master/gif) - Go fuzzer corpus
- [peterdn/gif-test-suite](https://github.com/peterdn/gif-test-suite) - Systematic GIF test cases

## Recommended Parameters

```bash
# Fast iteration (smaller inputs, more mutations)
cargo fuzz run fuzz_decode -- -max_len=10240 -dict=fuzz/gif.dict

# Thorough coverage (larger inputs)
cargo fuzz run fuzz_decode -- -max_len=1048576 -dict=fuzz/gif.dict

# Parallel fuzzing (use all cores)
cargo fuzz run fuzz_decode -- -jobs=0 -workers=0 -dict=fuzz/gif.dict

# Run for specific duration (1 hour)
cargo fuzz run fuzz_decode -- -max_total_time=3600 -dict=fuzz/gif.dict
```

## Dictionary

`gif.dict` contains GIF-specific tokens that help the fuzzer generate valid-looking inputs:
- Magic headers (`GIF87a`, `GIF89a`)
- Block types and extension labels
- Common LZW code sizes
- Disposal method patterns
- NETSCAPE loop extension

## Interpreting Crashes

Crashes are saved to `fuzz/artifacts/<target>/`. To reproduce:

```bash
cargo fuzz run fuzz_decode fuzz/artifacts/fuzz_decode/crash-xxx

# Minimize the crash input
cargo fuzz tmin fuzz_decode fuzz/artifacts/fuzz_decode/crash-xxx
```

## CVEs and Edge Cases Targeted

Based on historical GIF decoder vulnerabilities:

| CVE | Issue | Test Coverage |
|-----|-------|---------------|
| CVE-2025-27598 | OOB write from crafted frame length | `fuzz_limits` |
| CVE-2021-44648 | Heap overflow with LZW min code = 12 | `fuzz_decode` |
| CVE-2019-15133 | Divide by zero with height = 0 | `fuzz_decode` |
| CVE-2017-14450 | Buffer overflow in image parsing | `fuzz_decode` |

The seed corpus includes:
- Dimension bombs (`dimension_bomb.gif`, `large_dimensions.gif`)
- Out-of-bounds frames (`oob.gif`, `issue_1455_oversized.gif`)
- All disposal methods (`any-disposal.gif`, `mixed-disposal.gif`)
- Interlaced images (`interlaced.gif`)
- Transparency handling (`alpha_gif_a.gif`)

## Coverage Report

Generate coverage report:

```bash
cargo fuzz coverage fuzz_decode
# Results in fuzz/coverage/fuzz_decode/
```

## CI Integration

For continuous fuzzing, consider:
- [OSS-Fuzz](https://github.com/google/oss-fuzz) - Google's fuzzing infrastructure
- [cifuzz](https://github.com/CodeIntelligenceTesting/cifuzz) - CI/CD integration

## Sanitizers

By default, AddressSanitizer is enabled. Options:

```bash
# Disable sanitizer (2x faster, less thorough)
cargo fuzz run fuzz_decode --sanitizer none

# Enable MemorySanitizer
cargo fuzz run fuzz_decode --sanitizer memory

# Enable ThreadSanitizer (if multi-threaded)
cargo fuzz run fuzz_decode --sanitizer thread
```

Since zengif has minimal unsafe code (only in `types.rs` for byte reinterpretation),
`--sanitizer none` is reasonable for faster iteration, but keep ASan enabled for
thorough testing.
