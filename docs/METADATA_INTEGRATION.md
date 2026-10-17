# Metadata inventory integration

Reuse [PR23](https://github.com/imazen/zengif/pull/23), revision
`36c7ff1ddae01cdb8fcc803d5b77db0478e1834b`, including its decoder-disagreement
regressions and independent ExifTool checks. `inventory` is default-off and
implies `std`, like the GIF decoder. Without it, the trait method remains callable
and returns typed unsupported. The useful normal codec API does not need it.

The adapter returns the shared `Inventory` directly. Invalid part construction
preserves `InventoryError`; the imported memory-derived part cap still reports
`MemoryLimitExceeded`. Changing the global cap's classification to the shared
`InventoryParts` limit is pending owner approval of its existing test.

A one-shot stop reason is latched across the internal probe, which otherwise
reduces cancellation to a failed parse. Empty and malformed inputs also poll.
The new test injects a timeout at every observed poll on empty, malformed and
multi-extension GIF input and requires the same typed reason.

## Validation

Native and i686 inventory, review and stable fuzz-replay suites passed. Strict
clippy, default-off behavior and no-default-feature compilation passed. ExifTool
agreed on 263 reported units across 63 corpus files, plus the synthetic fixture.
The standalone inventory fuzz target compiles; public snapshots are regenerated.

The inventory describes the existing decoder/probe consumption differences.
It does not decode LZW pixels, and its existing interlace and palette limitations
remain explicitly covered by the imported review tests. No new binary fixtures
were needed for this integration.

## Published surface

`just metadata-api-check` compares explicit rustdoc JSON against crates.io 0.7.3
with a forced patch comparison despite the already-unreleased 0.8.0 bump.
cargo-semver-checks 0.50.0 reports 221 passing, two failing and 31 skipped checks:
`ProbeError` now has payload variants, and `GifError::Cancelled` has a payload.
Those definitions predate this integration on main `ebb0d42d`.

Manual comparison adds nine codec-trait errors changed from `At<GifError>` to
`At<CodecError>`, also already on main, and the retired `zencodec` Cargo feature.
Rustdoc comparison does not check removed Cargo features or reliably detect
these associated-type changes. Inventory itself is absent from 0.7.3; its new
feature/method implementation introduces no further published signature break.
The queued-breaking changelog retains the earlier changes. This is not a
compatibility pass for all of main or a release approval.
