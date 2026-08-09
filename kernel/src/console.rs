use core::fmt;

use myos_runtime::console::ByteConsole;

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const CONSOLE_WRITE_CLASS: LockClass = LockClass::new("console.write", LockRank::Console, 3);
static CONSOLE_WRITE_LOCK: IrqSpinLock<()> = IrqSpinLock::new_with_class((), CONSOLE_WRITE_CLASS); // SUDOOS_FINAL_DIRECT_FIX_V1

/// 将当前架构的早期控制台适配到公共格式化设施。
struct EarlyConsole;

impl ByteConsole for EarlyConsole {
    #[inline]
    fn write_byte(byte: u8) {
        crate::arch::early_console::write_byte(byte);
    }
}

#[doc(hidden)]
pub fn print(arguments: fmt::Arguments<'_>) {
    let _guard = CONSOLE_WRITE_LOCK.lock();
    myos_runtime::console::write::<EarlyConsole>(arguments);
}

/// Serialize one real userspace write with kernel diagnostics on SMP.
pub fn write_bytes(bytes: &[u8]) {
    let _guard = CONSOLE_WRITE_LOCK.lock();
    for byte in bytes {
        crate::arch::early_console::write_byte(*byte);
    }
}

#[macro_export]
macro_rules! print {
    ($($argument:tt)*) => {
        $crate::console::print(format_args!($($argument)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };

    ($($argument:tt)*) => {
        $crate::print!("{}\n", format_args!($($argument)*))
    };
}

/*
 * LS2K1000 真机调试：无锁、无分配的裸串口输出。
 *
 * 直接写 UART（绕过 CONSOLE_WRITE_LOCK 与 println 的 fmt 路径），供
 * 全局分配器致命错误路径与 Scheduler 初始化分阶段检查点使用：这些场景
 * 发生在持有堆锁 / 控制台锁的内部，普通 println 可能死锁或互相干扰。
 * 仅 ls2k1000 平台参与编译；qemu_virt / riscv64 不受影响。
 */
#[cfg(feature = "platform-ls2k1000")]
pub mod raw {
    use core::fmt;

    /// 写单个字节到 UART（轮询 THRE，等待发送 FIFO 空闲）。
    #[inline]
    pub fn putc(byte: u8) {
        crate::arch::early_console::write_byte(byte);
    }

    /// 写字符串，将 \n 转换为 \r\n 以适配常规串口终端。
    #[inline(never)]
    pub fn puts(text: &str) {
        for byte in text.bytes() {
            if byte == b'\n' {
                putc(b'\r');
            }
            putc(byte);
        }
    }

    /// 写十六进制（带 0x 前缀，去掉前导零）。
    pub fn puthex(value: usize) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut buffer = [0u8; 16];
        let mut length = 0;
        let mut found = false;
        // 按 nibble 递减移位：shift 序列为 60,56,...,4,0（64 位）。此前用
        // `(0..64).rev().step_by(4)` 生成 63,59,...,3，step_by 取索引 0,4,...
        // 恒跳过 shift 0 —— 最低 4 bit 丢失（0x1234 被输出为 0x123）。
        for nibble in (0..core::mem::size_of::<usize>() * 2).rev() {
            let digit = ((value >> (nibble * 4)) & 0xf) as usize;
            if digit != 0 {
                found = true;
            }
            if found {
                buffer[length] = HEX[digit];
                length += 1;
            }
        }
        if length == 0 {
            putc(b'0');
            return;
        }
        puts("0x");
        for byte in &buffer[..length] {
            putc(*byte);
        }
    }

    /// 写十进制。
    pub fn putdec(value: usize) {
        let mut buffer = [0u8; 20];
        let mut length = 0;
        let mut value = value;
        if value == 0 {
            putc(b'0');
            return;
        }
        while value > 0 {
            buffer[19 - length] = b'0' + (value % 10) as u8;
            length += 1;
            value /= 10;
        }
        for byte in &buffer[20 - length..] {
            putc(*byte);
        }
    }

    /// fmt::Write 适配器：`write!(raw::Writer, "{:?}", ...)` 直接输出到裸串口，
    /// 复用 core::fmt 的格式化但全程不分配、不加锁。
    pub struct Writer;

    impl fmt::Write for Writer {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            puts(text);
            Ok(())
        }
    }
}
