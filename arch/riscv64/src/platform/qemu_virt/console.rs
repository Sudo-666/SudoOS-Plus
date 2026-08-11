use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

/// QEMU virt ns16550a @ 0x1000_0000,标准字节步长 16550 寄存器布局。
const UART_RECEIVE_HOLDING: usize = 0;
const UART_TRANSMIT_HOLDING: usize = 0;
const UART_LINE_STATUS: usize = 5;

const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;
const LINE_STATUS_RECEIVE_READY: u8 = 1 << 0;

/// QEMU virt ns16550 提供 RX(QEMU 串口支持字节步长接收)。
pub(crate) const HAS_CONSOLE_RX: bool = true;

pub(crate) fn write_console_byte(byte: u8) {
    while console_line_status() as u8 & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let transmit_register =
        (crate::early_console::virtual_base() + UART_TRANSMIT_HOLDING) as *mut u8;

    // SAFETY: the selected boot-phase UART mapping is supervisor-only and live.
    unsafe {
        write_volatile(transmit_register, byte);
    }
}

/// 只有 LSR.DR(bit 0)置位才读 RBR。
pub(crate) fn try_read_console_byte() -> Option<u8> {
    if console_line_status() as u8 & LINE_STATUS_RECEIVE_READY == 0 {
        return None;
    }

    let receive_register =
        (crate::early_console::virtual_base() + UART_RECEIVE_HOLDING) as *const u8;

    // SAFETY: the address is the QEMU virt UART receive-holding register through
    // either the bootstrap identity mapping or the final high-half fixmap.
    Some(unsafe { read_volatile(receive_register) })
}

/// 平台 UART 线路状态寄存器 (LSR),诊断用。
pub(crate) fn console_line_status() -> u32 {
    let status_register = (crate::early_console::virtual_base() + UART_LINE_STATUS) as *const u8;

    // SAFETY: the address is the QEMU virt UART line-status register through
    // either the bootstrap identity mapping or the final high-half fixmap.
    unsafe { read_volatile(status_register) as u32 }
}
