use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

use myos_mm::PhysAddr;

const UART_PHYS_BASE: usize = 0x1fe0_01e0;

const UART_TRANSMIT_HOLDING: usize = 0;
const UART_LINE_STATUS: usize = 5;

const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;

/// qemu_virt has no UART RX path; the poller stays a no-op.
pub(crate) const HAS_CONSOLE_RX: bool = false;

pub(crate) fn write_console_byte(byte: u8) {
    while read_line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let address = PhysAddr::new(UART_PHYS_BASE + UART_TRANSMIT_HOLDING);

    let register = crate::memory::phys_access::mmio_mut_ptr::<u8>(address)
        .expect("LoongArch UART is outside uncached DMW");

    // SAFETY:
    // 指针来自架构 MMIO 映射接口。
    unsafe {
        write_volatile(register, byte);
    }
}

fn read_line_status() -> u8 {
    let address = PhysAddr::new(UART_PHYS_BASE + UART_LINE_STATUS);

    let register = crate::memory::phys_access::mmio_ptr::<u8>(address)
        .expect("LoongArch UART is outside uncached DMW");

    // SAFETY:
    // 指针来自架构 MMIO 映射接口。
    unsafe { read_volatile(register) }
}

/// qemu_virt has no UART RX path; the placeholder keeps the platform console
/// interface uniform with the LS2K1000 board. Returns `None` unconditionally.
pub(crate) fn try_read_console_byte() -> Option<u8> {
    None
}

/// qemu_virt has no UART RX path; returns 0 so the platform console interface
/// stays uniform with the LS2K1000 board (which reads NS16550 LSR).
pub(crate) fn console_line_status() -> u32 {
    0
}
