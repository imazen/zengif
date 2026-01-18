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
