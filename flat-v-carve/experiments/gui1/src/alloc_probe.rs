//! Counting allocator for the GUI1 measurement report.
//!
//! The experiment needs whole-process Rust heap numbers instead of only the
//! bytes it happens to track by hand. This wrapper counts every Rust allocation
//! on native and WASM. It does not see the JS heap, the GPU allocations or the
//! operating system's own overhead, so a missing counter is reported as unknown
//! rather than as zero.
use serde::{Deserialize, Serialize};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

pub struct Counting;

#[inline]
fn record(size: usize) {
    let now = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    TOTAL.fetch_add(size, Ordering::Relaxed);
    PEAK.fetch_max(now, Ordering::Relaxed);
}

#[inline]
fn release(size: usize) {
    CURRENT.fetch_sub(size, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        release(layout.size());
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            if new_size >= layout.size() {
                record(new_size - layout.size());
            } else {
                release(layout.size() - new_size);
            }
        }
        moved
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    pub current_bytes: usize,
    pub peak_bytes: usize,
    pub total_bytes: usize,
    pub allocations: usize,
}

impl Counters {
    pub fn delta_peak(self, earlier: Counters) -> usize {
        self.peak_bytes.saturating_sub(earlier.peak_bytes)
    }
}

pub fn snapshot() -> Counters {
    Counters {
        current_bytes: CURRENT.load(Ordering::Relaxed),
        peak_bytes: PEAK.load(Ordering::Relaxed),
        total_bytes: TOTAL.load(Ordering::Relaxed),
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
    }
}

/// Restart the peak at the current live size so a single probe measures itself.
pub fn reset_peak() {
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_track_a_real_allocation() {
        let before = snapshot();
        let block = vec![7u8; 1 << 20];
        let during = snapshot();
        assert!(during.current_bytes >= before.current_bytes + (1 << 20));
        assert!(during.peak_bytes >= during.current_bytes);
        assert!(during.allocations > before.allocations);
        drop(block);
        assert!(snapshot().current_bytes < during.current_bytes);
    }
}
