use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

/// JH7110 UART0 是 Synopsys DW_apb_uart (16550 兼容)。
///
/// 设备树属性 reg-shift = <2>、reg-io-width = <4>,寄存器按 4 字节步长
/// 32 位访问,与 QEMU virt 的字节步长 ns16550a 布局不同。
const UART_RECEIVE_HOLDING: usize = 0x00;
const UART_TRANSMIT_HOLDING: usize = 0x00;
const UART_LINE_STATUS: usize = 0x14;

const LINE_STATUS_TRANSMIT_EMPTY: u32 = 1 << 5;
const LINE_STATUS_RECEIVE_READY: u32 = 1 << 0;

/// VisionFive 2 提供真正的 UART RX(1 ms workqueue 轮询消费 RBR)。
pub(crate) const HAS_CONSOLE_RX: bool = true;

pub(crate) fn write_console_byte(byte: u8) {
    while console_line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
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

/// 只有 LSR.DR(bit 0)置位才读 RBR,并只返回低 8 位。
pub(crate) fn try_read_console_byte() -> Option<u8> {
    if console_line_status() & LINE_STATUS_RECEIVE_READY == 0 {
        return None;
    }

    let receive_register =
        (crate::early_console::virtual_base() + UART_RECEIVE_HOLDING) as *const u32;

    // SAFETY: the address is the JH7110 UART0 receive-holding register through
    // either the bootstrap identity mapping or the final high-half fixmap.
    Some(unsafe { read_volatile(receive_register) } as u8)
}

/// 平台 UART 线路状态寄存器 (LSR),诊断用。
pub(crate) fn console_line_status() -> u32 {
    let status_register = (crate::early_console::virtual_base() + UART_LINE_STATUS) as *const u32;

    // SAFETY: the address is the JH7110 UART0 line-status register (LSR,
    // offset 0x14 after reg-shift) through either the bootstrap identity
    // mapping or the final high-half fixmap.
    unsafe { read_volatile(status_register) }
}
