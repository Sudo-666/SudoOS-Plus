use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

/// JH7110 UART0 是 Synopsys DW_apb_uart (16550 兼容)。
///
/// 设备树属性 reg-shift = <2>、reg-io-width = <4>,寄存器按 4 字节步长
/// 32 位访问,与 QEMU virt 的字节步长 ns16550a 布局不同。
const UART_TRANSMIT_HOLDING: usize = 0x00;
const UART_LINE_STATUS: usize = 0x14;

const LINE_STATUS_TRANSMIT_EMPTY: u32 = 1 << 5;

pub(crate) fn write_console_byte(byte: u8) {
    while line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let transmit_register =
        (crate::early_console::virtual_base() + UART_TRANSMIT_HOLDING) as *mut u32;

    // SAFETY: the selected boot-phase UART mapping is supervisor-only and live.
    // THR 是 32 位寄存器,写入值低 8 位有效。
    unsafe {
        write_volatile(transmit_register, byte as u32);
    }
}

fn line_status() -> u32 {
    let status_register = (crate::early_console::virtual_base() + UART_LINE_STATUS) as *const u32;

    // SAFETY: the address is the JH7110 UART0 line-status register (LSR,
    // offset 0x14 after reg-shift) through either the bootstrap identity
    // mapping or the final high-half fixmap.
    unsafe { read_volatile(status_register) }
}
