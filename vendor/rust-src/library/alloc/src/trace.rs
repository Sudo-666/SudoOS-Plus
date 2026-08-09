//! PR-2: scalar allocation-trace hooks for the vendored alloc crate.
//!
//! On real hardware an allocation may return null without the kernel's
//! `#[global_allocator]` shim ever seeing it — the kernel-side ALLOC_RING then
//! stops short of the failing allocation, so the null is manufactured somewhere
//! inside this crate. These hooks record every allocation-layer event as a
//! (tag, value) pair into a small in-crate ring plus the single most recent
//! event, so the kernel's OOM handler can dump the exact last events before the
//! failure and prove which layer manufactured the null: the shim's raw return
//! (tag `ALLOC_IMPL_AFTER` with val == 0), a RawVec `AllocError` (tag
//! `RAW_ALLOC_ERR`), a `realloc` returning null (tag `REALLOC_NULL`), or a
//! capacity-overflow panic (tag `CAP_OVERFLOW`).
//!
//! Design constraints:
//! - Scalar only: no allocation, no formatting, no heap, no console.
//! - Fully isolated: hooks write only to in-crate atomics. Builds that never
//!   install a reader (e.g. the QEMU virt kernel) differ only by a few extra
//!   relaxed atomic stores per allocation — no UART, no fixed addresses.
//! - Reproducible: this file lives in vendor/rust-src and is committed;
//!   oscomp-prepare-rust-src.sh force-syncs it into the build sysroot.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Trace event tags (a stable ABI with kernel/src/heap.rs `ls2k_trace_tag_name`).
pub const T_ALLOC_FN: u32 = 1; // alloc(layout) free fn entered; value = layout.size()
pub const T_ALLOC_IMPL_ENTER: u32 = 2; // Global::alloc_impl entered; value = layout.size()
pub const T_ALLOC_IMPL_AFTER: u32 = 3; // alloc_impl got raw_ptr from shim; value = raw_ptr (0 = null!)
pub const T_EXCHANGE_MALLOC: u32 = 4; // Box exchange_malloc; value = size
pub const T_HANDLE_OOM: u32 = 5; // handle_alloc_error; value = size
pub const T_CAP_OVERFLOW: u32 = 6; // raw_vec capacity_overflow; value = 0
pub const T_RAW_ALLOC_OK: u32 = 7; // raw_vec try_allocate_in Ok; value = layout.size()
pub const T_RAW_ALLOC_ERR: u32 = 8; // raw_vec try_allocate_in AllocError; value = layout.size()
pub const T_REALLOC_NULL: u32 = 9; // Global::grow_impl realloc returned null; value = new_size

/// Number of trace entries kept in the ring (the tail of the event stream).
pub const RING_LEN: usize = 16;

static TRACE_IDX: AtomicUsize = AtomicUsize::new(0);
static TRACE_TAG: [AtomicUsize; RING_LEN] = [const { AtomicUsize::new(0) }; RING_LEN];
static TRACE_VAL: [AtomicUsize; RING_LEN] = [const { AtomicUsize::new(0) }; RING_LEN];
static LAST_TAG: AtomicUsize = AtomicUsize::new(0);
static LAST_VAL: AtomicUsize = AtomicUsize::new(0);

/// Record one allocation-layer event. Called on every allocation in
/// alloc.rs / raw_vec.rs. Never allocates, never faults, never prints.
#[inline(always)]
pub fn trace(tag: u32, value: usize) {
    LAST_TAG.store(tag as usize, Ordering::Relaxed);
    LAST_VAL.store(value, Ordering::Relaxed);
    let index = TRACE_IDX.fetch_add(1, Ordering::Relaxed) % RING_LEN;
    TRACE_TAG[index].store(tag as usize, Ordering::Relaxed);
    TRACE_VAL[index].store(value, Ordering::Relaxed);
}

/// Total number of trace events recorded so far.
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_count() -> usize {
    TRACE_IDX.load(Ordering::Relaxed)
}

/// Tag of the most recent trace event (0 = none yet).
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_last_tag() -> usize {
    LAST_TAG.load(Ordering::Relaxed)
}

/// Value of the most recent trace event.
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_last_val() -> usize {
    LAST_VAL.load(Ordering::Relaxed)
}

/// Number of ring slots (the kernel dumps `count - len .. count`).
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_ring_len() -> usize {
    RING_LEN
}

/// Tag of ring slot `i` (index modulo RING_LEN; unwritten slots read as 0).
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_ring_tag(i: usize) -> usize {
    TRACE_TAG[i % RING_LEN].load(Ordering::Relaxed)
}

/// Value of ring slot `i` (index modulo RING_LEN; unwritten slots read as 0).
#[no_mangle]
pub extern "C" fn sudoos_alloc_trace_ring_val(i: usize) -> usize {
    TRACE_VAL[i % RING_LEN].load(Ordering::Relaxed)
}
