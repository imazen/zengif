# zengif build commands

# Default recipe
default: check

# Full check: format, clippy, test
check: fmt clippy test

# Format code + regenerate the public-API surface snapshots (docs/public-api/).
# The snapshot runner lives in the standalone apidoc/ package, so it is never
# built or run by plain `cargo test` or any CI job.
fmt:
    cargo fmt
    cargo test --manifest-path apidoc/Cargo.toml

# Regenerate the public-API surface snapshots only
api-doc:
    cargo test --manifest-path apidoc/Cargo.toml

# Verify the committed snapshots are current
api-doc-check:
    ZEN_API_DOC=check cargo test --manifest-path apidoc/Cargo.toml

# Run clippy with all targets and features
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run tests
test:
    cargo test --all-features

# Check dependency versions
outdated:
    cargo outdated

# Generate documentation
doc:
    cargo doc --no-deps --all-features

# Run benchmarks
bench:
    cargo bench --all-features

# Run specific benchmark group
bench-group GROUP:
    cargo bench --all-features -- {{GROUP}}

# Build release
build-release:
    cargo build --release --all-features

# Cross-compile and test for i686 (32-bit x86)
test-i686:
    cross test --all-features --target i686-unknown-linux-gnu

# Cross-compile and test for armv7 (32-bit ARM)
test-armv7:
    cross test --all-features --target armv7-unknown-linux-gnueabihf

# Run all cross-compiled tests
test-cross: test-i686 test-armv7

# Clean build artifacts
clean:
    cargo clean

# Check for security vulnerabilities
audit:
    cargo audit

# Run with all features for CI
ci: fmt
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features
    cargo doc --no-deps --all-features

# Feature permutation checks (includes path-dep features that CI skips)
feature-check:
    cargo test --no-default-features
    cargo test
    cargo test --features quantizr
    cargo test --features imagequant
    cargo test --features color_quant
    cargo test --features imgref-interop
    cargo test --features zenquant
    cargo test --features zencodec

# === Fuzzing (requires nightly: rustup install nightly) ===

# Run decode fuzzer (main target)
fuzz:
    cargo +nightly fuzz run fuzz_decode -- -dict=fuzz/gif.dict

# Run decode fuzzer with seed corpus
fuzz-seeded:
    cargo +nightly fuzz run fuzz_decode fuzz/corpus/seed/ -- -dict=fuzz/gif.dict

# Run streaming decode fuzzer
fuzz-streaming:
    cargo +nightly fuzz run fuzz_decode_streaming -- -dict=fuzz/gif.dict

# Run roundtrip fuzzer
fuzz-roundtrip:
    cargo +nightly fuzz run fuzz_roundtrip -- -dict=fuzz/gif.dict

# Run limits fuzzer
fuzz-limits:
    cargo +nightly fuzz run fuzz_limits

# List available fuzz targets
fuzz-list:
    cargo +nightly fuzz list

# Run fuzzer for a specific duration (e.g., just fuzz-timed 3600 for 1 hour)
fuzz-timed SECONDS:
    cargo +nightly fuzz run fuzz_decode -- -max_total_time={{SECONDS}} -dict=fuzz/gif.dict

# Download external fuzzing corpora
fuzz-download-corpus:
    ./fuzz/download_corpus.sh

# Generate coverage report from fuzzing
fuzz-coverage:
    cargo +nightly fuzz coverage fuzz_decode
    @echo "Coverage report: fuzz/coverage/fuzz_decode/"

# Build all fuzz targets (useful for CI)
fuzz-build:
    cargo +nightly fuzz build

# === Profiling ===

# Run allocation sweep profiler (content types × sizes × quantizers)
profile:
    cargo run --release --all-features --example alloc_profile

# Run memory profiler with tracking allocator (accurate B/pixel measurements)
profile-memory:
    cargo run --release --all-features --example memory_profile

# Run allocation profiler with heaptrack (Linux only)
profile-heap:
    heaptrack cargo run --release --all-features --example alloc_profile

# View heaptrack results (latest)
profile-view:
    heaptrack_gui $(ls -t heaptrack.alloc_profile.*.zst 2>/dev/null | head -1)

# Native ARM audit; full output retained under ~/tmp.
arm-bench-macos bench="decode_bench":
    mkdir -p "$HOME/tmp"
    TMPDIR="$HOME/tmp" "$HOME/work/claudehints/scripts/run-heavy" --mem 12G --jobs 4 -- cargo bench -p zengif --bench '{{bench}}' -- --format=llm > "$HOME/tmp/zengif-{{bench}}.log" 2>&1

metadata-check:
    cargo test --features inventory --test inventory --test inventory_review --test inventory_review_r4 --test inventory_review_r2 --test inventory_feature --test fuzz_regression
    cargo clippy --features inventory --lib --tests -- -D warnings
    cargo check --no-default-features
    cargo test --test inventory_feature

metadata-review-check:
    cargo test --features inventory --test inventory_review --test inventory_review_r2 --test inventory_review_r4 --test inventory_feature --test fuzz_regression
    cargo clippy --features inventory --lib --tests -- -D warnings
    cargo check --no-default-features
    cargo test --test inventory_feature

metadata-oracle:
    INVENTORY_ORACLE_EXIFTOOL="$(command -v exiftool)" cargo test --features inventory --test inventory_oracle -- --nocapture

metadata-portability:
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER=env cargo test --target i686-unknown-linux-gnu --features inventory --test inventory --test inventory_review --test inventory_review_r2 --test inventory_review_r4 --test inventory_feature --test fuzz_regression
    cargo check --manifest-path fuzz/Cargo.toml --bin inventory
    just api-doc

metadata-api-check:
    #!/usr/bin/env bash
    set -euo pipefail
    baseline=$(cargo read zengif==0.7.3 --path-only)
    RUSTC_BOOTSTRAP=1 cargo rustdoc --lib --features inventory -- -Zunstable-options --output-format json
    RUSTC_BOOTSTRAP=1 cargo rustdoc --manifest-path "$baseline/Cargo.toml" --target-dir "$PWD/target/published-api/0.7.3" --lib --features zencodec -- -Zunstable-options --output-format json
    cargo semver-checks check-release --current-rustdoc target/doc/zengif.json --baseline-rustdoc target/published-api/0.7.3/doc/zengif.json --release-type patch

metadata-api-validation:
    cargo check --manifest-path fuzz/Cargo.toml --bin inventory
    just api-doc
    just metadata-api-check

metadata-part-limit-check:
    cargo test --features inventory --test inventory part_cap_is_a_resource_error
    cargo test --features inventory --test inventory_review max_memory_bounds_the_part_count
    cargo clippy --features inventory --lib --test inventory --test inventory_review -- -D warnings

metadata-cross-corpus-check:
    cargo test --features inventory --test inventory check_inventory_on_the_conformance_corpus -- --exact
    cargo test --features inventory --test inventory well_formed_files_have_no_image_data_tail -- --exact
    actionlint .github/workflows/ci.yml

metadata-color-admission-check:
    cargo test --lib codec::tests
    cargo clippy --lib -- -D warnings
    cargo fmt --check

# Adapter timing and metadata against the pinned shared contract.
shared-contract-check:
    cargo test --features inventory --test shared_contracts
    cargo test --test shared_contracts
    cargo test --no-default-features --features std --test shared_contracts
    cargo clippy --features inventory --test shared_contracts -- -D warnings
    cargo test --features inventory --lib codec::
    cargo clippy --features inventory --lib -- -D warnings
