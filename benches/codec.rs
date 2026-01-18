//! Benchmarks for zengif codec operations

use criterion::{criterion_group, criterion_main, Criterion};

fn decode_benchmark(_c: &mut Criterion) {
    // TODO: Implement decode benchmarks
}

fn encode_benchmark(_c: &mut Criterion) {
    // TODO: Implement encode benchmarks
}

criterion_group!(benches, decode_benchmark, encode_benchmark);
criterion_main!(benches);
