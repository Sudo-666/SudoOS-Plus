// arch/loongarch64/src/platform/ls2k1000/console.rs

use core::fmt::{self, Write};

// 龙芯 2K1000LA UART0 的强序非缓存虚拟地址
// 物理基址: 0x1fe2_0000 (LS2K1000-DP-FACTORY), XKPRANGE 非缓存映射前缀: 0x8000_0000_0000_0000
const UART0_BASE: usize = 0x8000_0000_1fe2_0000;

/// NS16550 UART 寄存器偏移 (8 位寄存器)
const UART_DAT: usize = UART0_BASE + 0x00; // 数据寄存器
const UART_LSR: usize = UART0_BASE + 0x05; // 线路状态寄存器

const LSR_TX_IDLE: u8 = 1 << 5; // 发送保持寄存器为空
const LSR_RX_READY: u8 = 1 << 0; // 接收数据寄存器有数据

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

    /// 从 UART 读取一个字节;若 RX FIFO 无数据则返回 None (轮询方式)。
    ///
    /// LSR bit 0 (DR) 为 1 表示接收数据寄存器已有数据,读 DAT 消费之。
    pub fn try_read_byte(&mut self) -> Option<u8> {
        unsafe {
            let lsr = core::ptr::read_volatile(UART_LSR as *const u8);
            if lsr & LSR_RX_READY == 0 {
                return None;
            }
            Some(core::ptr::read_volatile(UART_DAT as *const u8))
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

/// 平台控制台输入轮询接口:返回一个已就绪的 RX 字节,无数据则 None。
///
/// 供 `arch::early_console::try_read_byte` 转发;kernel 的 UART RX 轮询
/// (workqueue delayed work) 每 tick 调用本函数最多 64 次。
pub(crate) fn try_read_console_byte() -> Option<u8> {
    Uart::new().try_read_byte()
}

pub fn init() {
    // UART 在 U-Boot 阶段已经初始化过波特率（115200[cite: 1]）和时钟
    // 这里我们直接复用 U-Boot 的配置，不做重置
}