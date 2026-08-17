use core::fmt;

use myos_runtime::console::ByteConsole;

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const CONSOLE_WRITE_CLASS: LockClass = LockClass::new("console.write", LockRank::Console, 3);
static CONSOLE_WRITE_LOCK: IrqSpinLock<()> = IrqSpinLock::new_with_class((), CONSOLE_WRITE_CLASS); // SUDOOS_FINAL_DIRECT_FIX_V1

/// Stage-4 UART RX poller (任何 HAS_CONSOLE_RX 平台)。
///
/// A self-rescheduling delayed-work item drains the UART FIFO into the console
/// TTY at 1 ms cadence. Each tick reads the whole FIFO into a stack array first
/// and only then feeds the line discipline, so echo/termios processing can
/// never stall the hardware drain loop and widen the window in which the FIFO
/// fills and drops bytes. Workqueue callbacks run in a sleepable system-worker
/// context, so the TTY echo, waitqueue wakeups and signal delivery inside
/// `tty::input_byte` are legal. This is the phase-4 stand-in for the stage-5
/// UART IRQ path.
///
/// 平台是否真正启动本模块由 `HAS_CONSOLE_RX` 决定:无 RX 的平台仍编译本
/// 模块(保证公共代码可编译),但 `start()` 不会入队任何 work。
mod uart_input {
    use core::sync::atomic::{AtomicU64, Ordering};
    use core::time::Duration;

    // 115200-8N1 每秒约 11520 字节(≈11.5 字节/ms),1 ms 轮询配合硬件 FIFO
    // 足够跟上连续粘贴;10 ms 周期会让突发粘贴超出 16 字节 FIFO 而丢字。
    const POLL_INTERVAL: Duration = Duration::from_millis(1);
    // 单轮排空上限:回显/TTY 处理期间不得独占 worker。达到上限说明 FIFO
    // 仍有积压,立即重排队而不是等满一个周期。
    const MAX_DRAIN_PER_TICK: usize = 64;

    static POLL_COUNT: AtomicU64 = AtomicU64::new(0);

    fn queue_next_poll() {
        crate::workqueue::queue_delayed(POLL_INTERVAL, poll_uart_console, 0).unwrap_or_else(
            |error| {
                panic!("UART-RX: unable to queue poller: {error:?}");
            },
        );
    }

    fn poll_uart_console(_argument: usize) {
        // 一次性 first-poll 标志:仅首 tick 输出一次寄存器状态,证明 work 回调
        // 确实在 worker 线程执行过,并记录 UART 接收状态(空闲通常 0x60)。
        let poll = POLL_COUNT.fetch_add(1, Ordering::Relaxed);
        if poll == 0 {
            let lsr = crate::arch::early_console::console_line_status();
            crate::println!("UART-RX: first poll executed lsr={lsr:#04x}");
        }

        // 先把 FIFO 快速读入栈上数组,再统一交给行规程。读取循环里没有任何
        // 串口输出,回显/TTY 处理与硬件排空完全解耦,不会因打印拉长排空间隙。
        let mut pending = [0_u8; MAX_DRAIN_PER_TICK];
        let mut count = 0;
        while count < MAX_DRAIN_PER_TICK {
            match crate::arch::early_console::try_read_byte() {
                Some(byte) => {
                    pending[count] = byte;
                    count += 1;
                }
                None => break,
            }
        }
        let fifo_drained = count < MAX_DRAIN_PER_TICK;
        for byte in &pending[..count] {
            crate::tty::input_byte(*byte);
        }

        if fifo_drained {
            queue_next_poll();
        } else {
            // 单轮排满:可能还有积压,立即再次排队以追上串口线速。
            crate::workqueue::queue(poll_uart_console, 0).unwrap_or_else(|error| {
                panic!("UART-RX: unable to queue poller: {error:?}");
            });
        }
    }

    pub fn start() {
        // 首次立即入队而非延迟 10ms,确保启动日志能证明 worker 执行。
        crate::workqueue::queue(poll_uart_console, 0).unwrap_or_else(|error| {
            panic!("UART-RX: unable to start poller: {error:?}");
        });
        crate::println!("tty: uart rx poller queued");
    }
}

/// Start feeding the platform UART RX into the console TTY.
///
/// 按平台能力 `HAS_CONSOLE_RX` 决定是否真正入队 poller;无 RX 的平台
/// (如 loongarch64 qemu_virt)为 no-op,`rdinit=/init` 分支保持平台中立。
pub fn start_uart_input_poller() {
    if crate::arch::early_console::HAS_CONSOLE_RX {
        uart_input::start();
    }
}

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

/// Serialize a userspace write with inline `\n` -> `\r\n` translation (ONLCR)
/// while holding the console write lock once. Used by the console tty so user
/// output breaks lines correctly; kernel println does this conversion in the
/// runtime formatter instead.
pub fn write_bytes_translated(bytes: &[u8], from: u8, to: &[u8]) {
    let _guard = CONSOLE_WRITE_LOCK.lock();
    for byte in bytes {
        if *byte == from {
            for translated in to {
                crate::arch::early_console::write_byte(*translated);
            }
        } else {
            crate::arch::early_console::write_byte(*byte);
        }
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
