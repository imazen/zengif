# zengif build commands

# Default recipe
default: check

# Full check: format, clippy, test
check: fmt clippy test

# Format code
fmt:
    cargo fmt

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
