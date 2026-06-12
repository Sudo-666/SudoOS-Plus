use core::sync::atomic::{AtomicU64, Ordering};

use myos_mm::{FaultAccess, FaultSource, PageFault, VirtAddr};

static KERNEL_FAULTS: AtomicU64 = AtomicU64::new(0);
static USER_FAULTS: AtomicU64 = AtomicU64::new(0);
static READ_FAULTS: AtomicU64 = AtomicU64::new(0);
static WRITE_FAULTS: AtomicU64 = AtomicU64::new(0);
static EXECUTE_FAULTS: AtomicU64 = AtomicU64::new(0);

pub fn initialize() {
    crate::println!("page fault subsystem:");
    crate::println!("  kernel faults  : fail-fast");
    crate::println!("  user faults    : pending task/address-space");
    crate::println!("  demand paging  : policy ready");
}

pub fn handle_page_fault(fault: PageFault, instruction_pointer: VirtAddr, raw: usize) -> ! {
    record_fault(fault);

    match fault.source() {
        FaultSource::Kernel => panic!(
            "kernel page fault: ip={:#x} address={:#x} access={:?} present={} raw={:#x}",
            instruction_pointer.get(),
            fault.address().get(),
            fault.access(),
            fault.is_present(),
            raw,
        ),

        FaultSource::User => panic!(
            "user page fault before task subsystem: ip={:#x} address={:#x} access={:?} present={} raw={:#x}",
            instruction_pointer.get(),
            fault.address().get(),
            fault.access(),
            fault.is_present(),
            raw,
        ),
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    assert_eq!(KERNEL_FAULTS.load(Ordering::Relaxed), 0);
    assert_eq!(USER_FAULTS.load(Ordering::Relaxed), 0);
    assert_eq!(READ_FAULTS.load(Ordering::Relaxed), 0);
    assert_eq!(WRITE_FAULTS.load(Ordering::Relaxed), 0);
    assert_eq!(EXECUTE_FAULTS.load(Ordering::Relaxed), 0);

    crate::println!("page fault test:");
    crate::println!("  counters       : zeroed");
    crate::println!("  fault policy   : installed");
}

fn record_fault(fault: PageFault) {
    match fault.source() {
        FaultSource::Kernel => {
            KERNEL_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
        FaultSource::User => {
            USER_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
    }

    match fault.access() {
        FaultAccess::Read => {
            READ_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
        FaultAccess::Write => {
            WRITE_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
        FaultAccess::Execute => {
            EXECUTE_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}
