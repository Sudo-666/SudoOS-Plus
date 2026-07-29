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

/// Extended-component enable bits in CSR.EUEN.
const CSR_EUEN: usize = 0x2;
const EUEN_FPE: usize = 1 << 0;
const EUEN_SXE: usize = 1 << 1;

/// Enable a set of architectural extended components on the current CPU.
///
/// The bits remain enabled on this CPU.  `task/switch.S` independently
/// enables FPE+SXE at every context-switch boundary and saves/restores the
/// complete 128-bit register state, so this helper is the exception fallback.
#[inline]
fn enable_extended(mask: usize) {
    let value: usize;
    // SAFETY: EUEN is a per-CPU architectural control register.  The caller
    // only requests architecturally defined enable bits.
    unsafe {
        asm!(
            "csrrd {value}, {csr}",
            value = out(reg) value,
            csr = const CSR_EUEN,
            options(nomem, nostack),
        );
        let mut new_value = value | mask;
        asm!(
            "csrwr {value}, {csr}",
            value = inout(reg) new_value => _,
            csr = const CSR_EUEN,
            options(nomem, nostack),
        );
    }
}

/// Enable scalar floating point on the current CPU.
pub fn enable_fpu() {
    enable_extended(EUEN_FPE);
}

/// Enable scalar floating point and the 128-bit LSX register file.
///
/// SXD (ECODE 0x10) is an extended-component-disabled exception, not a page
/// fault.  Enabling both FPE and SXE and retrying the faulting instruction is
/// the architectural recovery path.
pub fn enable_lsx() {
    enable_extended(EUEN_FPE | EUEN_SXE);
}
