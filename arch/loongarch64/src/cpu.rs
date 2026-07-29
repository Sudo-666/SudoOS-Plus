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

/// Enable the LoongArch FPU by setting EUEN.FPE.
pub fn enable_fpu() {
    const CSR_EUEN: usize = 0x2;
    const EUEN_FPE: usize = 1 << 0;
    unsafe {
        let value: usize;
        core::arch::asm!("csrrd {}, {}", out(reg) value, const CSR_EUEN, options(nomem, nostack));
        core::arch::asm!("csrwr {}, {}", in(reg) (value | EUEN_FPE), const CSR_EUEN, options(nomem, nostack));
    }
}

/// Enable LSX (128-bit SIMD).
pub fn enable_lsx() {
    const CSR_EUEN: usize = 0x2;
    const EUEN_SXE: usize = 1 << 1;
    unsafe {
        let value: usize;
        core::arch::asm!("csrrd {}, {}", out(reg) value, const CSR_EUEN, options(nomem, nostack));
        core::arch::asm!("csrwr {}, {}", in(reg) (value | EUEN_SXE), const CSR_EUEN, options(nomem, nostack));
    }
}

/// Enable LASX (256-bit SIMD).
pub fn enable_lasx() {
    const CSR_EUEN: usize = 0x2;
    const EUEN_ASXE: usize = 1 << 2;
    unsafe {
        let value: usize;
        core::arch::asm!("csrrd {}, {}", out(reg) value, const CSR_EUEN, options(nomem, nostack));
        core::arch::asm!("csrwr {}, {}", in(reg) (value | EUEN_ASXE), const CSR_EUEN, options(nomem, nostack));
    }
}

/// Enable FPU + LSX + LASX eagerly at boot.  User-space never triggers
/// FPD / SXD / ASXD — no per-trap enable latency.
pub fn enable_all_user_extensions() {
    enable_fpu();
    enable_lsx();
    enable_lasx();
}
