//! Allocation helpers honoring an allocation-fallibility policy per call site.
//!
//! A GIF decode mixes two allocation regimes:
//!
//! * **Big, untrusted-sized buffers** (the canvas / output pixel buffer, the
//!   previous-frame disposal buffer) default to the *fallible* `try_reserve`
//!   path — a malicious Logical Screen Descriptor or frame header can demand
//!   gigabytes, so we want a graceful [`GifError::AllocationFailed`] rather than
//!   an abort. zengif's zero-trust design already routes these through
//!   [`Stats::try_alloc`](crate::stats::Stats::try_alloc) + `try_reserve`.
//! * **Small, bounded scratch** (a single fill buffer, the fixed LZW dictionary
//!   the `gif` crate owns internally) defaults to the *infallible* `vec!` path —
//!   a single `calloc` is faster and the size is bounded, not attacker-controlled
//!   in any unbounded way.
//!
//! The policy is a **3-mode, per-site override** of that default:
//! [`AllocPref::Fallible`] / [`AllocPref::Infallible`] force one path everywhere;
//! [`AllocPref::CodecDefault`] keeps each site's own default. The helper
//! signatures therefore take the caller's preference *and* the site default, and
//! resolve them together.
//!
//! ## Why a local enum (not `zencodec::AllocPreference` directly)
//!
//! zengif's decode path is `zencodec`-free (the `zencodec` feature is *not*
//! default, and the direct [`decode_gif`](crate::decode_gif) API must build
//! without it). Threading a `zencodec` type through the decode allocation sites
//! would force a `zencodec` dependency onto that path. So we carry a small local
//! [`AllocPref`] on [`Limits`](crate::limits::Limits) and map
//! `zencodec::AllocPreference` → [`AllocPref`] only at the `zencodec` decode
//! boundary (see `codec.rs`). The 3-mode semantics are identical.

#[cfg(not(feature = "std"))]
use alloc::vec;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use whereat::at;

use crate::error::{GifError, Result};
use crate::limits::Limits;
use crate::stats::Stats;

/// Per-site allocation-fallibility preference (zengif-local mirror of
/// `zencodec::AllocPreference`).
///
/// Carried on [`Limits`](crate::limits::Limits) so it travels with the rest of
/// the resource governance the decoder already threads to every allocation
/// site. `Copy` + a `CodecDefault` default so existing code and struct literals
/// are unaffected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AllocPref {
    /// Keep each site's own default fallibility (big untrusted buffers
    /// fallible, small bounded scratch infallible). Preserves existing
    /// behaviour. Default.
    #[default]
    CodecDefault,
    /// Force the fallible path: `try_reserve`, returning a graceful
    /// [`GifError::AllocationFailed`] instead of aborting. Prefer for untrusted
    /// input.
    Fallible,
    /// Force the infallible path: `vec!` / `Vec::with_capacity` (faster — a
    /// single `calloc` for the zeroed case) at the cost of aborting on OOM.
    /// Prefer for trusted sizes and benchmarks.
    Infallible,
}

/// Resolve the 3-mode [`AllocPref`] against THIS site's default fallibility.
///
/// * [`Fallible`](AllocPref::Fallible) → always `true`.
/// * [`Infallible`](AllocPref::Infallible) → always `false`.
/// * [`CodecDefault`](AllocPref::CodecDefault) → the site default, unchanged.
#[inline]
#[must_use]
pub(crate) fn resolve_fallible(pref: AllocPref, site_default_fallible: bool) -> bool {
    match pref {
        AllocPref::Fallible => true,
        AllocPref::Infallible => false,
        AllocPref::CodecDefault => site_default_fallible,
    }
}

/// Allocate `n` elements filled with `value`, honoring the per-site fallibility
/// AND zengif's memory tracking. Generic over the element type so the typed
/// canvas / disposal buffers (`Vec<Rgba>`) and the byte buffers (`Vec<u8>`)
/// share one implementation.
///
/// * fallible → [`Stats::try_alloc`] (enforces the memory limit) then
///   `try_reserve_exact` + fill, returning [`GifError::AllocationFailed`] on
///   allocation failure (tracking is rolled back).
/// * infallible → memory-limit check (so an explicit `Infallible` can't bypass
///   the resource cap), then `vec![value; n]` (a single fast fill / `calloc`,
///   aborts on OOM).
///
/// Both paths leave the same amount tracked in `stats`, so peak/current memory
/// is identical regardless of the chosen mode.
pub(crate) fn alloc_filled<T: Clone>(
    pref: AllocPref,
    site_default_fallible: bool,
    n: usize,
    value: T,
    stats: &Stats,
    limits: &Limits,
) -> Result<Vec<T>> {
    let bytes = n.saturating_mul(core::mem::size_of::<T>());
    if resolve_fallible(pref, site_default_fallible) {
        // Fallible: limit check + try_reserve, roll back tracking on failure.
        stats.try_alloc(bytes, limits)?;
        let mut v = Vec::new();
        if v.try_reserve_exact(n).is_err() {
            stats.track_dealloc(bytes);
            return Err(at!(GifError::AllocationFailed {
                requested: bytes as u64,
            }));
        }
        v.resize(n, value);
        Ok(v)
    } else {
        stats.try_alloc(bytes, limits)?;
        Ok(vec![value; n])
    }
}

/// Allocate `n` zeroed bytes, honoring the per-site fallibility AND zengif's
/// memory tracking. Thin `u8` wrapper over [`alloc_filled`].
#[inline]
pub(crate) fn alloc_zeroed(
    pref: AllocPref,
    site_default_fallible: bool,
    n: usize,
    stats: &Stats,
    limits: &Limits,
) -> Result<Vec<u8>> {
    alloc_filled(pref, site_default_fallible, n, 0u8, stats, limits)
}

/// Allocate an empty `Vec<u8>` with reserved capacity for `cap` bytes, honoring
/// the per-site fallibility AND zengif's memory tracking.
///
/// * fallible → [`Stats::try_alloc`] + `try_reserve_exact`, returning
///   [`GifError::AllocationFailed`] on failure.
/// * infallible → `Stats::track_alloc` (after the limit check) +
///   `Vec::with_capacity(cap)`.
///
/// The returned `Vec` is empty (length 0); the caller fills it.
#[allow(dead_code)] // reserved for the `vec_with_capacity` accumulator sites
pub(crate) fn vec_with_capacity(
    pref: AllocPref,
    site_default_fallible: bool,
    cap: usize,
    stats: &Stats,
    limits: &Limits,
) -> Result<Vec<u8>> {
    if resolve_fallible(pref, site_default_fallible) {
        stats.try_alloc(cap, limits)?;
        let mut v = Vec::new();
        if v.try_reserve_exact(cap).is_err() {
            stats.track_dealloc(cap);
            return Err(at!(GifError::AllocationFailed {
                requested: cap as u64,
            }));
        }
        Ok(v)
    } else {
        stats.try_alloc(cap, limits)?;
        Ok(Vec::with_capacity(cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `CodecDefault` keeps each site's own default fallibility.

    #[test]
    fn codec_default_keeps_site_default_true() {
        // Big-buffer site (default fallible): CodecDefault stays fallible.
        assert!(resolve_fallible(AllocPref::CodecDefault, true));
    }

    #[test]
    fn codec_default_keeps_site_default_false() {
        // Small-scratch site (default infallible): CodecDefault stays infallible.
        assert!(!resolve_fallible(AllocPref::CodecDefault, false));
    }

    #[test]
    fn explicit_fallible_overrides_any_site_default() {
        assert!(resolve_fallible(AllocPref::Fallible, false));
        assert!(resolve_fallible(AllocPref::Fallible, true));
    }

    #[test]
    fn explicit_infallible_overrides_any_site_default() {
        assert!(!resolve_fallible(AllocPref::Infallible, true));
        assert!(!resolve_fallible(AllocPref::Infallible, false));
    }

    #[test]
    fn alloc_zeroed_all_modes_equal_bytes() {
        let stats = Stats::new();
        let limits = Limits::none();
        let a = alloc_zeroed(AllocPref::CodecDefault, true, 4096, &stats, &limits).unwrap();
        let b = alloc_zeroed(AllocPref::Infallible, true, 4096, &stats, &limits).unwrap();
        let c = alloc_zeroed(AllocPref::Fallible, false, 4096, &stats, &limits).unwrap();
        assert_eq!(a.len(), 4096);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert!(a.iter().all(|&x| x == 0));
    }

    #[test]
    fn vec_with_capacity_reserves_and_is_empty() {
        let stats = Stats::new();
        let limits = Limits::none();
        let a = vec_with_capacity(AllocPref::Infallible, false, 1024, &stats, &limits).unwrap();
        let b = vec_with_capacity(AllocPref::Fallible, false, 1024, &stats, &limits).unwrap();
        assert_eq!(a.len(), 0);
        assert_eq!(b.len(), 0);
        assert!(a.capacity() >= 1024);
        assert!(b.capacity() >= 1024);
    }

    #[test]
    fn alloc_zeroed_fallible_oom_returns_err() {
        // Request an impossibly large allocation; the fallible path must
        // return Err (mapped to AllocationFailed) rather than abort. Use
        // unlimited Limits so the failure comes from the allocator, not the
        // memory-limit check.
        let stats = Stats::new();
        let limits = Limits::none();
        let r = alloc_zeroed(AllocPref::Fallible, true, usize::MAX, &stats, &limits);
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err().error(),
            GifError::AllocationFailed { .. }
        ));
    }

    #[test]
    fn vec_with_capacity_fallible_oom_returns_err() {
        let stats = Stats::new();
        let limits = Limits::none();
        let r = vec_with_capacity(AllocPref::Fallible, true, usize::MAX, &stats, &limits);
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err().error(),
            GifError::AllocationFailed { .. }
        ));
    }
}
