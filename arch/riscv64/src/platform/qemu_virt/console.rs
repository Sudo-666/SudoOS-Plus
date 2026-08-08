use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

/// QEMU virt ns16550a @ 0x1000_0000,标准字节步长 16550 寄存器布局。
const UART_TRANSMIT_HOLDING: usize = 0;
const UART_LINE_STATUS: usize = 5;

const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;

pub(crate) fn write_console_byte(byte: u8) {
    while line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let transmit_register =
        (crate::early_console::virtual_base() + UART_TRANSMIT_HOLDING) as *mut u8;

    // SAFETY: the selected boot-phase UART mapping is supervisor-only and live.
    unsafe {
        write_volatile(transmit_register, byte);
    }
}

fn line_status() -> u8 {
    let status_register = (crate::early_console::virtual_base() + UART_LINE_STATUS) as *const u8;

    // SAFETY: the address is the QEMU virt UART line-status register through
    // either the bootstrap identity mapping or the final high-half fixmap.
    unsafe { read_volatile(status_register) }
}
