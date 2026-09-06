# GIF ARM audit, 2026-09-06

Coverage: four single-frame XOR-pattern decode sizes, ten palette-expansion
cases, and one 64 MiB RGBA swizzle. Default-quantizer encoding measurements
are still pending. This is not an animation, photographic-content, or
quantization-quality sweep. No production codec implementation changed.

Apple M4 Pro, macOS, Rust 1.98.0 / LLVM 22, runtime dispatch without
`target-cpu=native`, four build/Rayon/OMP threads, `nice -n 19`. The baseline
codec is `6c456243`; retained decode fixtures were added by `64e2e832`.
The local lockfile needed refreshing to match current manifests before the
baseline build; [the resolver log](gif-cargo-update.log) records the updates.

## Decode and swizzle

| XOR fixture | zengif canvas clone | zengif in-place | gif-rs RGBA |
|---|---:|---:|---:|
| 64×64 | 17.70 µs | 16.95 µs | 20.71 µs |
| 256×256 | 162.15 µs | 154.96 µs | 189.01 µs |
| 1024×1024 | 2.76 ms | 2.66 ms | 3.31 ms |
| 4096×4096 | 28.73 ms | 27.39 ms | 35.11 ms |

These compare API paths, not SIMD versus scalar. `zengif-inplace` avoids
cloning the composited canvas. The gif-rs arm requests RGBA output directly.
The fixture generator supplies its palette, so generating these files does
not exercise the default quantizer. Retained files and SHA256 values are in
[fixtures.pointer.md](fixtures.pointer.md).

The existing garb RGBA→BGRA path took 1.93 ms versus 8.39 ms for the scalar
channel-swap loop on the 4096×4096 buffer. Full paired statistics, build
output, and `/usr/bin/time -l` resource measurements: [decode log](gif-decode.log).

## Palette expansion

At widths 1920 and 4096, the existing opaque 16-pixel unrolled kernel beat
the straightforward scalar lookup loop in all ten cases: 16/64/256-color
noise plus 16/256-color runs of eight. Paired improvements range from
7.10% to 9.49%. Exact opaque output assertions passed before timing.
The transparent arm has different semantics and is not an interchangeable
opaque implementation. See [palette log](gif-palette.log).

The [opaque assembly](palette-opaque.asm) uses scalar indexed table loads
and stores, with loop unrolling. This measurement supports retaining the
existing implementation over the tested scalar alternative. It does not
establish a hardware limit or rule out every possible SIMD algorithm.

Use `just arm-bench-macos decode_bench` or
`just arm-bench-macos expand_palette` to reproduce.
