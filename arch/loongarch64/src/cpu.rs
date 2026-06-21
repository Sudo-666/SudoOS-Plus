use core::arch::asm;

/// 让当前处理器等待中断。
#[inline]
pub fn wait_for_interrupt() {
    // SAFETY:
    // IDLE 不访问 Rust 管理的内存，也不修改当前栈。
    unsafe {
        asm!("idle 0", options(nomem, nostack),);
    }
}

/// Enable local interrupts and enter the architectural wait state.
///
/// # Safety
///
/// The caller must have installed a valid exception entry and must enter with
/// local interrupts disabled after checking for pending work.
#[inline]
pub unsafe fn enable_and_wait_for_interrupt() {
    // SAFETY: upheld by the caller; enabling interrupts immediately before
    // IDLE lets a pending timer/IPI abort the idle sleep.
    unsafe {
        crate::interrupt::enable();
        asm!("idle 0", options(nomem, nostack),);
    }
}
