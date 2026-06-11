use core::{
    hint::spin_loop,
    ptr::{read_volatile, write_volatile},
};

pub const MMIO_BASE: usize = 0x1000_0000;
pub const MMIO_SIZE: usize = 0x1000;

const UART_TRANSMIT_HOLDING: usize = 0;
const UART_LINE_STATUS: usize = 5;

const LINE_STATUS_TRANSMIT_EMPTY: u8 = 1 << 5;

pub fn write_byte(byte: u8) {
    while line_status() & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        spin_loop();
    }

    let transmit_register = (MMIO_BASE + UART_TRANSMIT_HOLDING) as *mut u8;

    // SAFETY:
    // 当前 early UART 页面已由启动地址空间直接映射。
    unsafe {
        write_volatile(transmit_register, byte);
    }
}

fn line_status() -> u8 {
    let status_register = (MMIO_BASE + UART_LINE_STATUS) as *const u8;

    // SAFETY:
    // 地址对应 QEMU virt UART line-status 寄存器。
    unsafe { read_volatile(status_register) }
}
