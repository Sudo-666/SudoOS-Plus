use core::arch::asm;

/// 让 CPU 等待下一次中断。
///
/// 目前中断尚未初始化，因此它主要用于内核最终停机循环。
#[inline]
pub fn wait_for_interrupt() {
    // SAFETY:
    // `wfi` 不访问 Rust 管理的内存，也不修改栈。
    // 当前代码运行在 OpenSBI 提供的 RISC-V Supervisor 模式。
    unsafe {
        asm!("wfi", options(nomem, nostack),);
    }
}
