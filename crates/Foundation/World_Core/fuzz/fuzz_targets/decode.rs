//! `serialize::decode` on arbitrary bytes: it must return a tree or an error,
//! never panic, and never allocate more than the input can justify (decision
//! 0004). A counting allocator records the peak heap use during the call;
//! hostile length fields that the decoder trusted would show up as a peak far
//! beyond the input's size.

#![no_main]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{body_of, seal};
use libfuzzer_sys::fuzz_target;
use world_core::serialize::decode;

/// Fixed overhead a decode may use whatever the input: the first 16 KiB page
/// of each arena, the page lists and the decoder's small temporaries.
const BASE_ALLOWANCE: usize = 1 << 20;

/// Heap bytes a decode may use per input byte. The densest valid encodings
/// (leaves at about 12 bytes for 16 bytes of node plus palette words) stay
/// within a few bytes per byte; this leaves a wide margin for transient
/// blocks while still catching any allocation sized from an untrusted field.
const BYTES_PER_INPUT_BYTE: usize = 256;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

impl Counting {
    fn grew(size: usize) {
        let now = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(now, Ordering::Relaxed);
    }
}

// SAFETY: every method forwards to the system allocator unchanged; the
// counters only observe sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            Self::grew(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as above.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            Self::grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
            Self::grew(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Decodes `bytes`, checking the peak heap use of the call.
fn decode_within_budget(bytes: &[u8]) {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let result = decode(bytes);
    let peak = PEAK.load(Ordering::Relaxed) - before;
    let allowed = BASE_ALLOWANCE + BYTES_PER_INPUT_BYTE * bytes.len();
    assert!(
        peak <= allowed,
        "decode of {} bytes peaked at {peak} heap bytes (allowed {allowed}); ok = {}",
        bytes.len(),
        result.is_ok()
    );
    drop(result);
}

fuzz_target!(|data: &[u8]| {
    // The raw bytes exercise the header and checksum checks; almost every
    // mutation fails the checksum, so the body is also decoded behind a
    // freshly computed valid header and checksum.
    decode_within_budget(data);
    decode_within_budget(&seal(body_of(data)));
});
