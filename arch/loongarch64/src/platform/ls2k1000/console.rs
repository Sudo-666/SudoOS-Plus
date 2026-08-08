// arch/loongarch64/src/platform/ls2k1000/console.rs

use core::fmt::{self, Write};

// 龙芯 2K1000LA UART0 的强序非缓存虚拟地址
// 物理基址: 0x1fe2_0000 (LS2K1000-DP-FACTORY), XKPRANGE 非缓存映射前缀: 0x8000_0000_0000_0000
const UART0_BASE: usize = 0x8000_0000_1fe2_0000;

/// NS16550 UART 寄存器偏移 (8 位寄存器)
const UART_DAT: usize = UART0_BASE + 0x00; // 数据寄存器
const UART_LSR: usize = UART0_BASE + 0x05; // 线路状态寄存器

const LSR_TX_IDLE: u8 = 1 << 5; // 发送保持寄存器为空

pub struct Uart;

impl Uart {
    pub fn new() -> Self {
        Uart
    }

    /// 向 UART 写入单个字节 (轮询方式)
    pub fn putc(&mut self, c: u8) {
        unsafe {
            // 等待发送 FIFO 为空
            while (core::ptr::read_volatile(UART_LSR as *const u8) & LSR_TX_IDLE) == 0 {
                core::hint::spin_loop();
            }
            // 写入字符
            core::ptr::write_volatile(UART_DAT as *mut u8, c);
        }
    }
}

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            // 将换行符转换为 CRLF 以适配常规串口终端
            if b == b'\n' {
                self.putc(b'\r');
            }
            self.putc(b);
        }
        Ok(())
    }
}

/// 导出平台特定的控制台输出接口供宏使用
pub fn console_putchar(c: u8) {
    Uart::new().putc(c);
}

/// 内核早期控制台要求的接口名称
pub(crate) fn write_console_byte(byte: u8) {
    console_putchar(byte);
}

pub fn init() {
    // UART 在 U-Boot 阶段已经初始化过波特率（115200[cite: 1]）和时钟
    // 这里我们直接复用 U-Boot 的配置，不做重置
}