//! Memory usage statistics tracking.
//!
//! Provides thread-safe tracking of zengif's own buffer allocations
//! (canvas, pixel buffers, etc). Note: This does not include allocations
//! made internally by the gif crate or quantizer libraries.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{GifError, Result};
use crate::limits::Limits;
use whereat::at;

/// Thread-safe memory usage statistics.
///
/// Track allocations and deallocations to monitor memory usage
/// during GIF processing. All operations are atomic and safe
/// for concurrent use.
#[derive(Debug)]
#[non_exhaustive]
pub struct Stats {
    /// Current memory usage in bytes.
    current_bytes: AtomicUsize,

    /// Peak memory usage in bytes.
    peak_bytes: AtomicUsize,

    /// Total bytes allocated (cumulative).
    total_allocated: AtomicUsize,

    /// Total bytes deallocated (cumulative).
    total_deallocated: AtomicUsize,

    /// Number of allocation operations.
    alloc_count: AtomicUsize,

    /// Number of deallocation operations.
    dealloc_count: AtomicUsize,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    /// Create a new stats tracker with zero usage.
    pub fn new() -> Self {
        Self {
            current_bytes: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
            total_allocated: AtomicUsize::new(0),
            total_deallocated: AtomicUsize::new(0),
            alloc_count: AtomicUsize::new(0),
            dealloc_count: AtomicUsize::new(0),
        }
    }

    /// Track an allocation of `bytes`.
    ///
    /// Updates current usage and peak if this is a new high.
    pub fn track_alloc(&self, bytes: usize) {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.total_allocated.fetch_add(bytes, Ordering::Relaxed);

        let prev = self.current_bytes.fetch_add(bytes, Ordering::Relaxed);
        let new_current = prev + bytes;

        // Update peak if we've exceeded it
        // Use compare-exchange loop to handle concurrent updates
        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while new_current > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                new_current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }
    }

    /// Track a deallocation of `bytes`.
    ///
    /// Uses saturating subtraction to prevent wrapping underflow in release
    /// builds if deallocations are ever mismatched (e.g., double-free tracking
    /// or untracked allocations).
    pub fn track_dealloc(&self, bytes: usize) {
        self.dealloc_count.fetch_add(1, Ordering::Relaxed);
        self.total_deallocated.fetch_add(bytes, Ordering::Relaxed);
        self.current_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(bytes))
            })
            .ok();
    }

    /// Get current memory usage in bytes.
    pub fn current(&self) -> usize {
        self.current_bytes.load(Ordering::Relaxed)
    }

    /// Get peak memory usage in bytes.
    pub fn peak(&self) -> usize {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Get total bytes allocated (cumulative).
    pub fn total_allocated(&self) -> usize {
        self.total_allocated.load(Ordering::Relaxed)
    }

    /// Get total bytes deallocated (cumulative).
    pub fn total_deallocated(&self) -> usize {
        self.total_deallocated.load(Ordering::Relaxed)
    }

    /// Get number of allocation operations.
    pub fn alloc_count(&self) -> usize {
        self.alloc_count.load(Ordering::Relaxed)
    }

    /// Get number of deallocation operations.
    pub fn dealloc_count(&self) -> usize {
        self.dealloc_count.load(Ordering::Relaxed)
    }

    /// Reset all statistics to zero.
    pub fn reset(&self) {
        self.current_bytes.store(0, Ordering::Relaxed);
        self.peak_bytes.store(0, Ordering::Relaxed);
        self.total_allocated.store(0, Ordering::Relaxed);
        self.total_deallocated.store(0, Ordering::Relaxed);
        self.alloc_count.store(0, Ordering::Relaxed);
        self.dealloc_count.store(0, Ordering::Relaxed);
    }

    /// Check if allocating `bytes` would exceed the limit.
    ///
    /// Returns error if limit would be exceeded, otherwise tracks the allocation.
    pub fn try_alloc(&self, bytes: usize, limits: &Limits) -> Result<()> {
        if let Some(max_memory) = limits.max_memory {
            let current = self.current() as u64;
            let new_total = current.saturating_add(bytes as u64);
            if new_total > max_memory {
                return Err(at!(GifError::MemoryLimitExceeded {
                    current: new_total,
                    limit: max_memory,
                }));
            }
        }
        self.track_alloc(bytes);
        Ok(())
    }

    /// Create a snapshot of current statistics.
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            current_bytes: self.current(),
            peak_bytes: self.peak(),
            total_allocated: self.total_allocated(),
            total_deallocated: self.total_deallocated(),
            alloc_count: self.alloc_count(),
            dealloc_count: self.dealloc_count(),
        }
    }
}

/// Immutable snapshot of statistics at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsSnapshot {
    /// Current memory usage in bytes.
    pub current_bytes: usize,
    /// Peak memory usage in bytes.
    pub peak_bytes: usize,
    /// Total bytes allocated (cumulative).
    pub total_allocated: usize,
    /// Total bytes deallocated (cumulative).
    pub total_deallocated: usize,
    /// Number of allocation operations.
    pub alloc_count: usize,
    /// Number of deallocation operations.
    pub dealloc_count: usize,
}

impl core::fmt::Display for StatsSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "current: {} bytes, peak: {} bytes, allocs: {}, deallocs: {}",
            self.current_bytes, self.peak_bytes, self.alloc_count, self.dealloc_count
        )
    }
}

/// A tracked allocation that automatically updates stats on drop.
///
/// Use this to wrap Vec or other heap allocations to ensure
/// deallocation is tracked.
pub struct TrackedAlloc<'a> {
    stats: &'a Stats,
    bytes: usize,
}

impl<'a> TrackedAlloc<'a> {
    /// Create a new tracked allocation.
    ///
    /// Assumes the allocation has already been tracked via `stats.track_alloc()`.
    pub fn new(stats: &'a Stats, bytes: usize) -> Self {
        Self { stats, bytes }
    }

    /// Get the tracked size in bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Resize the tracked allocation.
    ///
    /// Adjusts the stats to reflect the new size.
    pub fn resize(&mut self, new_bytes: usize) {
        if new_bytes > self.bytes {
            self.stats.track_alloc(new_bytes - self.bytes);
        } else if new_bytes < self.bytes {
            self.stats.track_dealloc(self.bytes - new_bytes);
        }
        self.bytes = new_bytes;
    }

    /// Forget this allocation without tracking deallocation.
    ///
    /// Use when transferring ownership elsewhere.
    pub fn forget(self) -> usize {
        let bytes = self.bytes;
        core::mem::forget(self);
        bytes
    }
}

impl Drop for TrackedAlloc<'_> {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.stats.track_dealloc(self.bytes);
        }
    }
}

/// Helper to allocate a Vec with tracking.
pub fn tracked_vec_with_capacity<T>(
    capacity: usize,
    stats: &Stats,
    limits: &Limits,
) -> Result<Vec<T>> {
    let bytes = capacity * core::mem::size_of::<T>();
    stats.try_alloc(bytes, limits)?;

    let mut vec = Vec::new();
    if vec.try_reserve(capacity).is_err() {
        stats.track_dealloc(bytes); // Undo the tracking
        return Err(at!(GifError::AllocationFailed {
            requested: bytes as u64,
        }));
    }

    Ok(vec)
}

/// Helper to allocate a Vec filled with a value.
pub fn tracked_vec_filled<T: Clone>(
    len: usize,
    value: T,
    stats: &Stats,
    limits: &Limits,
) -> Result<Vec<T>> {
    let mut vec = tracked_vec_with_capacity(len, stats, limits)?;
    vec.resize(len, value);
    Ok(vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_tracking() {
        let stats = Stats::new();
        assert_eq!(stats.current(), 0);
        assert_eq!(stats.peak(), 0);

        stats.track_alloc(1000);
        assert_eq!(stats.current(), 1000);
        assert_eq!(stats.peak(), 1000);

        stats.track_alloc(500);
        assert_eq!(stats.current(), 1500);
        assert_eq!(stats.peak(), 1500);

        stats.track_dealloc(1000);
        assert_eq!(stats.current(), 500);
        assert_eq!(stats.peak(), 1500); // Peak unchanged

        stats.track_dealloc(500);
        assert_eq!(stats.current(), 0);
        assert_eq!(stats.peak(), 1500);
    }

    #[test]
    fn try_alloc_with_limit() {
        let stats = Stats::new();
        let limits = Limits::default().max_memory(1000);

        assert!(stats.try_alloc(500, &limits).is_ok());
        assert_eq!(stats.current(), 500);

        assert!(stats.try_alloc(400, &limits).is_ok());
        assert_eq!(stats.current(), 900);

        // This should fail
        let result = stats.try_alloc(200, &limits);
        assert!(result.is_err());
        assert_eq!(stats.current(), 900); // Unchanged on failure
    }

    #[test]
    fn tracked_alloc_drop() {
        let stats = Stats::new();
        stats.track_alloc(1000);

        {
            let _tracked = TrackedAlloc::new(&stats, 1000);
            assert_eq!(stats.current(), 1000);
        }

        // After drop, should be deallocated
        assert_eq!(stats.current(), 0);
    }

    #[test]
    fn tracked_alloc_forget() {
        let stats = Stats::new();
        stats.track_alloc(1000);

        let tracked = TrackedAlloc::new(&stats, 1000);
        let bytes = tracked.forget();

        assert_eq!(bytes, 1000);
        assert_eq!(stats.current(), 1000); // Not deallocated
    }

    #[test]
    fn snapshot() {
        let stats = Stats::new();
        stats.track_alloc(1000);
        stats.track_alloc(500);
        stats.track_dealloc(300);

        let snap = stats.snapshot();
        assert_eq!(snap.current_bytes, 1200);
        assert_eq!(snap.peak_bytes, 1500);
        assert_eq!(snap.alloc_count, 2);
        assert_eq!(snap.dealloc_count, 1);
    }

    #[test]
    fn reset() {
        let stats = Stats::new();
        stats.track_alloc(1000);
        stats.reset();

        assert_eq!(stats.current(), 0);
        assert_eq!(stats.peak(), 0);
        assert_eq!(stats.alloc_count(), 0);
    }

    #[test]
    fn dealloc_underflow_saturates_to_zero() {
        let stats = Stats::new();
        stats.track_alloc(100);
        // Deallocate more than was allocated — must not wrap
        stats.track_dealloc(200);
        assert_eq!(stats.current(), 0, "should saturate at 0, not wrap");

        // Deallocate from zero — still safe
        stats.track_dealloc(50);
        assert_eq!(stats.current(), 0);
    }
}
