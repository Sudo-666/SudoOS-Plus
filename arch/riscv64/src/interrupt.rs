use core::arch::asm;

const SSTATUS_SIE: usize = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "saved interrupt state should be restored"]
pub struct InterruptState {
    enabled: bool,
}

impl InterruptState {
    pub const fn was_enabled(self) -> bool {
        self.enabled
    }
}

pub fn save_and_disable() -> InterruptState {
    let previous: usize;

    // SAFETY: 只原子清除当前 hart 的 SIE 位并读取旧 sstatus。
    unsafe {
        asm!(
            "csrrc {previous}, sstatus, {mask}",
            previous = out(reg) previous,
            mask = in(reg) SSTATUS_SIE,
            options(nostack),
        );
    }

    InterruptState {
        enabled: previous & SSTATUS_SIE != 0,
    }
}

pub fn restore(state: InterruptState) {
    if state.was_enabled() {
        // SAFETY: 恢复当前 hart 的 SIE 位，不访问内存或栈。
        unsafe {
            asm!(
                "csrs sstatus, {mask}",
                mask = in(reg) SSTATUS_SIE,
                options(nostack),
            );
        }
    } else {
        // SAFETY: 清除当前 hart 的 SIE 位，不访问内存或栈。
        unsafe {
            asm!(
                "csrc sstatus, {mask}",
                mask = in(reg) SSTATUS_SIE,
                options(nostack),
            );
        }
    }
}

pub fn disable() {
    // SAFETY: 清除当前 hart 的 SIE 位，不访问内存或栈。
    unsafe {
        asm!(
            "csrc sstatus, {mask}",
            mask = in(reg) SSTATUS_SIE,
            options(nostack),
        );
    }
}

/// # Safety
///
/// 调用者必须保证异常入口、栈以及所有可能的中断控制器均已配置。
pub unsafe fn enable() {
    // SAFETY: 调用者保证 trap entry、栈和中断控制器状态已准备好。
    unsafe {
        asm!(
            "csrs sstatus, {mask}",
            mask = in(reg) SSTATUS_SIE,
            options(nostack),
        );
    }
}

pub fn mask_all_sources() {
    // SAFETY: this masks every supervisor-local interrupt source on the
    // current hart without changing global interrupt state.
    unsafe {
        asm!("csrw sie, zero", options(nostack));
    }
}

pub fn are_enabled() -> bool {
    let status: usize;

    // SAFETY: 只读取当前 hart 的 sstatus CSR。
    unsafe {
        asm!(
            "csrr {status}, sstatus",
            status = out(reg) status,
            options(nostack),
        );
    }

    status & SSTATUS_SIE != 0
}

pub fn are_disabled() -> bool {
    !are_enabled()
}
