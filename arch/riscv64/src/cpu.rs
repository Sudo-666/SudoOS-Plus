use core::arch::asm;

/// 让 CPU 等待下一次中断。
///
/// 定时器和本地中断启用后，它用于 idle 循环以及唤醒自检。
#[inline]
pub fn wait_for_interrupt() {
    // SAFETY:
    // `wfi` 不访问 Rust 管理的内存，也不修改栈。
    // 当前代码运行在 OpenSBI 提供的 RISC-V Supervisor 模式。
    unsafe {
        asm!("wfi", options(nomem, nostack),);
    }
}
