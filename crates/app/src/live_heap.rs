//! The live Rust heap, for the allocation profiling build (spec 0055).
//!
//! hotpath's allocation mode reports bytes allocated per call, not what is still alive. This
//! allocator wraps the system one and counts live and peak bytes, so a run can tell the Rust heap
//! apart from GPU, font and driver memory. It exists only under `hotpath-profiling-alloc`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct LiveHeapAllocator;

fn grow(bytes: usize) {
    let live = LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

fn shrink(bytes: usize) {
    LIVE_BYTES.fetch_sub(bytes, Ordering::Relaxed);
}

// SAFETY: every call forwards to `System` with the caller's own layout and pointer, so the
// `GlobalAlloc` contract holds exactly as for `System`; the counters only observe sizes.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for LiveHeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds the `GlobalAlloc::alloc` contract for `layout`.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds the `GlobalAlloc::alloc_zeroed` contract for `layout`.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from this allocator with `layout`, as the caller guarantees.
        unsafe { System.dealloc(pointer, layout) };
        shrink(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `pointer` came from this allocator with `layout`, as the caller guarantees.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            shrink(layout.size());
            grow(new_size);
        }
        moved
    }
}

/// Prints the live and peak heap to stderr every `interval` from a thread of its own.
pub(crate) fn report_every(interval: Duration) {
    let spawned = std::thread::Builder::new()
        .name("k8sboard-heap".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(interval);
                eprintln!(
                    "[heap] live={:.1} MiB peak={:.1} MiB",
                    mebibytes(LIVE_BYTES.load(Ordering::Relaxed)),
                    mebibytes(PEAK_BYTES.load(Ordering::Relaxed)),
                );
            }
        });
    if let Err(error) = spawned {
        eprintln!("[heap] reporter not started: {error}");
    }
}

fn mebibytes(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grow_raises_the_peak_and_shrink_gives_the_bytes_back() {
        // Other threads allocate too, so only bounds that hold for any interleaving are asserted.
        grow(1 << 30);
        assert!(PEAK_BYTES.load(Ordering::Relaxed) >= 1 << 30);
        assert!(LIVE_BYTES.load(Ordering::Relaxed) >= 1 << 30);
        shrink(1 << 30);
        assert!(LIVE_BYTES.load(Ordering::Relaxed) < 1 << 30);
    }
}
