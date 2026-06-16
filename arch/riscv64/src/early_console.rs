use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MMIO_BASE: usize = 0x1000_0000;
pub const MMIO_SIZE: usize = 0x1000;

const UART_TRANSMIT_HOLDING: usize = 0;
const UART_LINE_STATUS: usize = 5;

const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;

static RUNTIME_MAPPING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Publishes the high-half UART alias after the final Sv39 root is active.
pub fn activate_runtime_mapping() {
    assert!(
        crate::memory::paging::translation_is_enabled(),
        "RISC-V UART runtime mapping requires Sv39",
    );
    RUNTIME_MAPPING_ACTIVE.store(true, Ordering::Release);
}

/// Returns the UART virtual base valid in the currently published boot phase.
pub fn virtual_base() -> usize {
    if RUNTIME_MAPPING_ACTIVE.load(Ordering::Acquire) {
        crate::memory::layout::EARLY_UART_FIXMAP.get()
    } else {
        MMIO_BASE
    }
}

pub fn write_byte(byte: u8) {
    while line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let transmit_register = (virtual_base() + UART_TRANSMIT_HOLDING) as *mut u8;

    // SAFETY: the selected boot-phase UART mapping is supervisor-only and live.
    unsafe {
        write_volatile(transmit_register, byte);
    }
}

fn line_status() -> u8 {
    let status_register = (virtual_base() + UART_LINE_STATUS) as *const u8;

    // SAFETY: the address is the QEMU virt UART line-status register through
    // either the bootstrap identity mapping or the final high-half fixmap.
    unsafe { read_volatile(status_register) }
}
